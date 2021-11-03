use clap::{App, Arg};
use std::sync::{Arc, Mutex};
use std::convert::TryInto;
use errors::FilterErrors::*;

mod sync;
mod search;
mod config;
mod view;
mod prune;
mod errors;
mod filter;
mod entry;

const ERR_PREFIX: &str = "\x1b[31;1mERROR:\x1b[0m";
const WARN_PREFIX: &str = "\x1b[33;1mWARNING:\x1b[0m";
const DETAILS: &str = "details.log.gz";

#[macro_export]
macro_rules! err{
	($msg:expr$(, $args:expr)*) => {
		eprintln!("{} {}", crate::ERR_PREFIX, format!($msg$(, $args)*))
	}
}

#[macro_export]
macro_rules! warn{
	($msg:expr$(, $args:expr)*) => {
		eprintln!("{} {}", crate::WARN_PREFIX, format!($msg$(, $args)*))
	}
}

#[tokio::main]
async fn main() {
	macro_rules! subcommand_search{
		($name:expr, $about:expr) => {
			App::new($name)
				.about($about)
				.arg(Arg::with_name("user")
					.short("u")
					.long("user")
					.help("Select logs from a specific user")
					.takes_value(true))
				.arg(Arg::with_name("when")
					.short("w")
					.long("when")
					.help("Select logs from a specific day (e.g. 'yesterday', 'friday', '2021-07-09')")
					.takes_value(true))
				.arg(Arg::with_name("term")
					.short("t")
					.long("term")
					.help("Select logs containing a specific term (rust-flavored regex supported)")
					.takes_value(true))
				.arg(Arg::with_name("os")
					.short("o")
					.long("os")
					.help("Select logs from a specific OS (either 'ios', 'android', or 'desktop')")
					.takes_value(true))
				.arg(Arg::with_name("before")
					.short("b")
					.long("before")
					.help("Select logs before a certain date")
					.takes_value(true))
				.arg(Arg::with_name("after")
					.short("a")
					.long("after")
					.help("Select logs from after a certain date")
					.takes_value(true))
		}
	}

	let matches = App::new("Rager")
		.version("1.0")
		.author("Ian Welker <@janshai:beeper.com>")
		.subcommand(subcommand_search!("sync", "Download all the logs from the server that you don't currently have on your device")
			.arg(Arg::with_name("config")
				.short("c")
				.help("The TOML config file to use when syncing. Located at ~/.config/rager.toml (on linux) by default")
				.takes_value(true))
			.arg(Arg::with_name("threads")
				.short("s")
				.help("How many threads to spawn while downloading. WARNING: this can cause panics when set too high. Recommended value is around 50.")
				.takes_value(true)))
		.subcommand(App::new("desync")
			.about("Clear all logs off of your device"))
		.subcommand(subcommand_search!("search", "Search through the logs currently on your device")
			.arg(Arg::with_name("preview")
				.short("p")
				.long("preview")
				.help("See only an overview of the selected issue, as opposed to viewing any of the logs")
				.takes_value(false)))
		.subcommand(App::new("view")
			.about("View a specific Entry")
			.arg(Arg::with_name("entry")
				.index(1)
				.required(true)
				.help("The entry (e.g. '2021-07-08/161300') to view the logs for")
				.takes_value(true)))
		.subcommand(subcommand_search!("prune", "Delete all entries that match the terms"))
		.get_matches();

	if let Some(args) = matches.subcommand_matches("sync") {
		// get the filter and the config file
		let (filter, mut config) = match filter_and_config(args, true) {
			Some((f, c)) => (f, c),
			None => {
				err!("Can't read configuration from given file");
				std::process::exit(1);
			}
		};

		if let Some(threads) = args.value_of("threads") {
			match threads.parse() {
				Ok(val) => config.threads = val,
				_ => {
					err!("The 'threads' argument must be passed in as an integer");
					return;
				}
			}
		}

		println!("Starting sync with server...");

		let lim = config.sync_retry_limit.map(|l| l as i8).unwrap_or(-1);
		let conf_arc = Arc::new(config);
		let filter_arc = Arc::new(filter);

		// normally I opt for a RwLock over a mutex but both this and to_check basically only ever
		// write, (state never reads, to_check only reads once and it's after everyone finishes writing
		// to it), so there's really no reason to choose RwLock over mutex here.
		let state = Arc::new(Mutex::new(sync::SyncTracker {
			prefix: "Checking Days:".to_owned(),
			started: 0,
			done: 0,
			total: 0,
		}));

		let mut retried: i8 = 0;

		let mut result = sync::sync_logs(&filter_arc, &conf_arc, &state).await;

		while retried < lim || lim == 0 {
			match result {
				Err(err) => {
					retried += 1;


					match err {
						errors::SyncErrors::ListingFailed => {
							if let Ok(mut state) = state.lock() {
								state.reset("Checking directories".to_owned());
							}

							println!("\nRager was unable to get a full list of directories; trying again...");
							result = sync::sync_logs(&filter_arc, &conf_arc, &state).await;
						},
						errors::SyncErrors::FilesDownloadFailed(files) => {
							if let Ok(mut state) = state.lock() {
								state.reset("Downloaded:".to_owned());
							}

							println!("\nSome files failed to download. Retrying them...");
							result = sync::download_files(files, &state, &conf_arc).await;
						},
					}
				},
				_ => break,
			}
		}

	} else if matches.subcommand_matches("desync").is_some() {

		sync::desync_all()

	} else if let Some(args) = matches.subcommand_matches("search") {
		let view = !args.is_present("preview");

		let (filter, config) = match filter_and_config(args, false) {
			Some((f, c)) => (f, c),
			None => {
				err!("Can't read configuration from given file");
				std::process::exit(1);
			}
		};

		search::search(filter, config, view).await;
	} else if let Some(args) = matches.subcommand_matches("view") {
		// safe to unwrap 'cause Clap would catch if it wasn't included
		let day_time = args.value_of("entry").unwrap();

		let regex_str = r"\d{4}-\d{2}-\d{2}/\d{6}";
		let date_regex = regex::Regex::new(regex_str).unwrap();

		// make sure it matches the regex so we can parse it correctly
		if !date_regex.is_match(day_time) {
			err!("Please enter a date that matches the regex {}", regex_str);
			return;
		}

		let splits = day_time.split('/').collect::<Vec<&str>>();
		let day = splits[0].to_owned();
		let time = splits[1].to_owned();

		let config_file = args.value_of("config").map(|c| c.to_owned());
		let config = match config::Config::from_file(&config_file) {
			Some(conf) => Arc::new(conf),
			None => {
				err!("Could not read or parse config file");
				std::process::exit(1);
			}
		};

		let entry = entry::Entry::new(day, time, config);

		if let Err(err) = view::view(entry, Vec::new()).await {
			match err {
				ViewingBeforeDownloading => err!("Cannot view a file before downloading the entry"),
				FileRetrievalFailed => err!("Failed to determine list of files in entry"),
				FileReadingFailed => err!("Failed to read specified file"),
				ViewPagingFailed => err!("Failed to display file on page"),
				_ => ()
			}
		}
	} else if let Some(args) = matches.subcommand_matches("prune") {
		// get the filter and the config file
		let (filter, config) = match filter_and_config(args, false) {
			Some((f, c)) => (f, c),
			None => {
				err!("Can't read configuration from given file");
				std::process::exit(1);
			}
		};

		prune::remove_with_terms(filter, config).await;
	}
}

pub fn filter_and_config(terms: &clap::ArgMatches, syncing: bool) -> Option<(filter::Filter, config::Config)> {
	let config_file = terms.value_of("config").map(|c| c.to_owned());
	let config = match config::Config::from_file(&config_file) {
		Some(conf) => conf,
		None => return None,
	};

	let user = terms.value_of("user").map(|u| u.to_owned());
	let term = terms.value_of("term").map(|t| t.to_owned());

	let any = terms.is_present("any");
	let ok_unsure = terms.is_present("ok_unsure");

	let when = terms.value_of("when")
		.map(filter::Filter::string_to_dates);

	let before = terms.value_of("before")
		.and_then(filter::Filter::string_to_single_date);

	let after = terms.value_of("after")
		.and_then(filter::Filter::string_to_single_date);

	let oses = terms.value_of("os").map(|o|
		match o.try_into() {
			Ok(entry) => vec![entry],
			Err(err) => {
				err!("{}", err);
				std::process::exit(1);
			}
		}
	);

	let ret_filter = if syncing {
		let mut ret_filter = filter::Filter::from_config_file(&config_file);

		macro_rules! set_new {
			($($items:ident, )*) => {
				$(if let Some(val) = $items {
					ret_filter.$items = Some(val);
				})*
			}
		}

		set_new!(user, term, when, before, after, oses,);

		if any {
			ret_filter.any = true;
		}

		if ok_unsure {
			ret_filter.ok_unsure = true;
		}

		ret_filter
	} else {
		filter::Filter {
			user,
			term,
			when,
			before,
			after,
			oses,
			any,
			ok_unsure
		}
	};

	Some((ret_filter, config))
}

async fn req_with_auth<U: reqwest::IntoUrl>(url: U, conf: &config::Config) -> reqwest::Result<reqwest::Response> {
	let client = reqwest::Client::new();

	let req = client.get(url)
		.basic_auth(&conf.username, Some(&conf.password))
		.build()?;

	Ok(client.execute(req).await?)
}

fn sync_dir() -> std::path::PathBuf {
	// documentation says this always returns some so we can safely unwrap
	let mut sync_dir = dirs::data_dir().unwrap();
	sync_dir.push("rageshake");
	sync_dir
}

fn get_links(output: &str) -> Vec<&str> {
	output.split('\n')
		.collect::<Vec<&str>>()
		.iter()
		.filter_map(|link| {
			let splits: Vec<&str> = link.split(&['<', '>'][..]).collect();
			if splits.len() > 3 {
				return Some(splits[2])
			}
			None
		})
		.collect::<Vec<&str>>()
}
