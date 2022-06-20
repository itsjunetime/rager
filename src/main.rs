#![warn(clippy::all)]

use clap::{Command, Arg, ArgAction};
use errors::FilterErrors::*;
use std::convert::TryInto;
use std::sync::{Arc, Mutex};

mod completion;
mod config;
mod entry;
mod errors;
mod filter;
mod prune;
mod search;
mod sync;
mod view;

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
	macro_rules! subcommand_search {
		($name:expr, $about:expr) => {
			Command::new($name)
				.about($about)
				.arg(
					Arg::new("user")
						.short('u')
						.long("user")
						.help("Select logs from a specific user")
						.takes_value(true),
				)
				.arg(
					Arg::new("when")
						.short('w')
						.long("when")
						.help("Select logs from a specific day (e.g. 'yesterday', 'friday', '2021-07-09')")
						.takes_value(true),
				)
				.arg(
					Arg::new("term")
						.short('t')
						.long("term")
						.help("Select logs containing a specific term (rust-flavored regex supported)")
						.takes_value(true),
				)
				.arg(
					Arg::new("os")
						.short('o')
						.long("os")
						.help("Select logs from a specific OS (either 'ios', 'android', or 'desktop')")
						.takes_value(true),
				)
				.arg(
					Arg::new("before")
						.short('b')
						.long("before")
						.help("Select logs before a certain date")
						.takes_value(true),
				)
				.arg(
					Arg::new("after")
						.short('a')
						.long("after")
						.help("Select logs from after a certain date")
						.takes_value(true),
				)
				.arg(
					Arg::new("any")
						.short('y')
						.long("any")
						.help("Match on any true conditions, instead of all")
						.action(ArgAction::SetTrue)
				)
				.arg(
					Arg::new("reject-unsure")
						.short('r')
						.long("reject-unsure")
						.help("Reject an entry when searching or syncing if we cannot determine whether it fits the search parameters")
						.action(ArgAction::SetTrue)
				)
		};
	}

	let sep_char = if cfg!(windows) {
		'\\'
	} else {
		'/'
	};

	let matches = Command::new("Rager")
		.version("0.4.0")
		.author("Ian Welker <@janshai:beeper.com>")
		.subcommand(
			subcommand_search!("sync", "Download all the logs from the server that you don't currently have on your device")
				.arg(
					Arg::new("config")
						.short('c')
						.help("The TOML config file to use when syncing. Located at ~/.config/rager.toml (on linux) by default")
						.takes_value(true),
				)
				.arg(
					Arg::new("threads")
						.short('s')
						.help("How many threads to spawn while downloading. WARNING: this can cause panics when set too high. Recommended value is around 50.")
						.takes_value(true),
				)
				.arg(
					Arg::new("sync-since-last-day")
						.short('d')
						.long("sync-since-last-day")
						.help("Sync entries only since the last day you synced (inclusive)")
						.action(ArgAction::SetTrue)
				),
		)
		.subcommand(Command::new("desync").about("Clear all logs off of your device"))
		.subcommand(
			subcommand_search!("search", "Search through the logs currently on your device").arg(
				Arg::new("preview")
					.short('p')
					.long("preview")
					.help("See only an overview of the selected issue, as opposed to viewing any of the logs")
					.takes_value(false),
			),
		)
		.subcommand(
			Command::new("view").about("View a specific Entry").arg(
				Arg::new("entry")
					.index(1)
					.required(true)
					.help(format!("The entry (e.g. '2021-07-08{c}161300') or file (e.g. '2021-07-08{c}161300{c}details.log.gz') to view the logs for", c = sep_char).as_str())
					.takes_value(true),
			),
		)
		.subcommand(subcommand_search!("prune", "Delete all entries that match the terms"))
		.subcommand(
			Command::new("complete")
				.about("List completions for view command")
				.arg(
					Arg::new("input")
						.index(1)
						.help("The input to get completions for")
						.takes_value(true)
				)
				.arg(
					Arg::new("install")
						.help("Install completion to your $SHELL")
						.short('i')
						.long("install")
				)
		)
		.get_matches();

	if let Some(args) = matches.subcommand_matches("sync") {
		// get the filter and the config file
		let (filter, mut config) = filter_and_config(args, true)
			.expect("Can't read configuration from given file");

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

		while let Err(err) = result {
			if lim != 0 && retried >= lim {
				break;
			}

			retried += 1;

			match err {
				errors::SyncErrors::ListingFailed => {
					if let Ok(mut state) = state.lock() {
						state.reset("Checking directories".to_owned());
					}

					println!("\nRager was unable to get a full list of directories; trying again...");
					result = sync::sync_logs(&filter_arc, &conf_arc, &state).await;
				}
				errors::SyncErrors::FilesDownloadFailed(files) => {
					if let Ok(mut state) = state.lock() {
						state.reset("Downloaded:".to_owned());
					}

					println!("\nSome files failed to download. Retrying them...");
					result = sync::download_files(files, &state, &conf_arc).await;
				}
			}
		}
	} else if matches.subcommand_matches("desync").is_some() {
		sync::desync_all()
	} else if let Some(args) = matches.subcommand_matches("search") {
		let view = !args.is_present("preview");

		let (filter, config) = filter_and_config(args, false)
			.expect("Can't read configuration from given file");

		search::search(filter, config, view).await;
	} else if let Some(args) = matches.subcommand_matches("view") {
		// safe to unwrap 'cause Clap would catch if it wasn't included
		let day_time = args.value_of("entry").unwrap();

		let mut dir = sync_dir();
		dir.push(day_time);

		// make sure it matches the regex so we can parse it correctly
		if !dir.as_path().exists() {
			err!(
				"Entry/file '{}' does not exist or is not downloaded",
				day_time
			);
			return;
		}

		let mut splits = day_time.split('/');
		let day = splits
			.next()
			.expect("Splits somehow doesn't even have a 0th index")
			.to_owned();

		let time = match splits.next() {
			Some(t) => t.to_owned(),
			_ => {
				err!("You must enter at least a day and time to view");
				return;
			}
		};

		let file = splits.next().map(ToOwned::to_owned);

		let config_file = args.value_of("config").map(|c| c.to_owned());

		let config = config::Config::from_file(&config_file)
			.map(Arc::new)
			.expect("Could not read or parse config file");

		let entry = entry::Entry::new(day, time, config);

		if let Err(err) = view::view(entry, file, None).await {
			match err {
				ViewingBeforeDownloading => err!("Cannot view a file before downloading the entry"),
				FileRetrievalFailed => err!("Failed to determine list of files in entry"),
				FileReadingFailed => err!("Failed to read specified file"),
				ViewPagingFailed => err!("Failed to display file on page"),
				_ => (),
			}
		}
	} else if let Some(args) = matches.subcommand_matches("prune") {
		// get the filter and the config file
		let (filter, config) = filter_and_config(args, false)
			.expect("Can't read configuration from given file");

		prune::remove_with_terms(filter, config).await;
	} else if let Some(args) = matches.subcommand_matches("complete") {
		if args.is_present("install") {
			completion::install_completion();
		} else if let Some(input) = args.value_of("input") {
			completion::list_completions(input);
		}
	}
}

pub fn filter_and_config(
	terms: &clap::ArgMatches,
	syncing: bool,
) -> Option<(filter::Filter, config::Config)> {
	let config_file = terms.value_of("config").map(|c| c.to_owned());
	let config = config::Config::from_file(&config_file)?;

	let user = terms.value_of("user").map(|u| u.to_owned());
	let term = terms.value_of("term").map(|t| t.to_owned());

	let any = *terms.get_one::<bool>("any").unwrap_or(&false);
	let reject_unsure = *terms.get_one::<bool>("reject-unsure").unwrap_or(&false);

	let when = terms.value_of("when").map(filter::Filter::string_to_dates);

	let before = terms
		.value_of("before")
		.and_then(filter::Filter::string_to_single_date);

	let after = terms
		.value_of("after")
		.and_then(filter::Filter::string_to_single_date);

	let oses = terms.value_of("os")
		.map(|o| o.try_into()
			.map(|os| vec![os])
			.expect("OS specified in config file is not valid")
		);

	let sync_since_last: bool = *terms
		.get_one::<bool>("sync-since-last-day")
		.unwrap_or(&true);

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

		if sync_since_last {
			if let Some(last) = get_last_synced_day() {
				// If we get one, then override the before, after, and when
				// in the filter so that this takes precedence
				// println!("last is {last}");
				ret_filter.before = None;
				ret_filter.when = None;
				ret_filter.after = Some(last);
			}
		}

		if any {
			ret_filter.any = true;
		}

		if reject_unsure {
			ret_filter.reject_unsure = false;
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
			reject_unsure
		}
	};

	Some((ret_filter, config))
}

async fn req_with_auth<U: reqwest::IntoUrl>(
	url: U,
	conf: &config::Config,
) -> reqwest::Result<reqwest::Response> {
	let client = reqwest::Client::new();

	let req = client
		.get(url)
		.basic_auth(&conf.username, Some(&conf.password))
		.build()?;

	client.execute(req).await
}

fn sync_dir() -> std::path::PathBuf {
	// documentation says this always returns some so we can safely unwrap
	let mut sync_dir = dirs::data_dir().unwrap();
	sync_dir.push("rageshake");
	sync_dir
}

fn get_links(output: &str) -> Vec<&str> {
	output
		.split('\n')
		.filter_map(|link| link.split(&['<', '>'][..]).nth(2))
		.filter(|s| !s.is_empty())
		.collect::<Vec<&str>>()
}

// Gets the most recent day that we actually synced during
fn get_last_synced_day() -> Option<[u16; 3]> {
	// iterate over all the entries we've downloaded
	std::fs::read_dir(&sync_dir())
		.ok()
		.and_then(|contents| {
			// Get their paths and filter out the bad ones
			 let mut sorted = contents.filter_map(|day|
				day.ok().map(|d| d.path())
			).collect::<Vec<std::path::PathBuf>>();
			// Sort them so that the most recent is last
			sorted.sort();
			// Then get the second-to-last one, which is what
			// we'll be telling it to sync after (so it still
			// tries to sync the most recent day)
			let second_to_last = sorted.len().saturating_sub(2);
			let s: Option<[u16; 3]> = sorted.into_iter()
				.nth(second_to_last)
				.and_then(|l| {
					// And get the file name, which will be
					// the date of the most recent successful sync
					l.file_name().and_then(|f|
						f.to_str().and_then(|s|
							// and parse it into a [u16; 3],
							// which makes it easier for us to use
							filter::Filter::string_to_dates(s)
								.into_iter()
								.next()
						)
					)
				});

			s
		})
}
