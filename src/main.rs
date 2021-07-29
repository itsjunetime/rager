use clap::{App, Arg};
use std::sync::Arc;

mod sync;
mod search;
mod config;
mod view;
mod prune;

const ERR_PREFIX: &str = "\x1b[31;1mERROR:\x1b[0m";
const WARN_PREFIX: &str = "\x1b[33;1mWARNING:\x1b[0m";

#[macro_export]
macro_rules! err{
	($msg:expr$(, $args:expr)*) => {
		eprintln!("{} {}", crate::ERR_PREFIX, format!($msg$(, $args)*));
	}
}

#[macro_export]
macro_rules! warn{
	($msg:expr$(, $args:expr)*) => {
		eprintln!("{} {}", crate::WARN_PREFIX, format!($msg$(, $args)*));
	}
}

#[tokio::main(flavor = "multi_thread", worker_threads = 20)]
async fn main() {
	let matches = App::new("Rager")
		.version("1.0")
		.author("Ian Welker <@janshai:beeper.com>")
		.subcommand(App::new("sync")
			.about("Download all the logs from the server that you don't currently have on your device")
			.arg(Arg::with_name("config")
				.short("c")
				.help("The TOML config file to use when syncing. Located at ~/.config/rager.toml (on linux) by default")
				.takes_value(true))
			.arg(Arg::with_name("threads")
				.short("t")
				.help("How many threads to spawn while downloading. WARNING: this can cause panics when set too high. Recommended value is around 50.")
				.takes_value(true)))
		.subcommand(App::new("desync")
			.about("Clear all logs off of your device"))
		.subcommand(App::new("search")
			.about("Search through the logs currently on your device")
			.arg(Arg::with_name("user")
				.short("u")
				.long("user")
				.help("Search for logs from a specific user")
				.takes_value(true))
			.arg(Arg::with_name("when")
				.short("w")
				.long("when")
				.help("Search for logs from a specific day (e.g. 'yesterday', 'friday', '2021-07-09')")
				.takes_value(true))
			.arg(Arg::with_name("term")
				.short("t")
				.long("term")
				.help("Search for logs containing a specific term (rust-flavored regex supported)")
				.takes_value(true))
			.arg(Arg::with_name("os")
				.short("o")
				.long("os")
				.help("Search for logs from a specific OS (either 'ios', 'android', or 'desktop')")
				.takes_value(true))
			.arg(Arg::with_name("any")
				.short("a")
				.long("any")
				.help("Match an entry if any of the terms are true, not just if all are")
				.takes_value(false))
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
		.subcommand(App::new("prune")
			.about("Delete all entries that match the terms")
			.arg(Arg::with_name("user")
				.short("u")
				.long("user")
				.help("Delete logs from a specific user")
				.takes_value(true))
			.arg(Arg::with_name("when")
				.short("w")
				.long("when")
				.help("Delete logs from a specific day (e.g. 'yesterday', 'friday', '2021-07-09')")
				.takes_value(true))
			.arg(Arg::with_name("term")
				.short("t")
				.long("term")
				.help("Delete logs containing a specific term (rust-flavored regex supported)")
				.takes_value(true))
			.arg(Arg::with_name("os")
				.short("o")
				.long("os")
				.help("Delete logs from a specific OS (either 'ios', 'android', or 'desktop')")
				.takes_value(true)))
		.get_matches();

	if let Some(args) = matches.subcommand_matches("sync") {
		let config_file = args.value_of("config")
			.map(|a| a.to_owned());

		let mut config = match config::Config::from_file(config_file) {
			Some(conf) => conf,
			None => return
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

		let limit = config.sync_retry_limit;
		let conf_arc = Arc::new(config);

		let mut retried: usize = 0;
		let mut success = sync::sync_logs(conf_arc.clone()).await;

		if let Some(lim) = limit {
			while (!success && retried < lim) || lim == 0 {
				println!("\nIt looks like some files failed to download. Syncing again...");

				retried += 1;
				success = sync::sync_logs(conf_arc.clone()).await;
			}
		}

	} else if matches.subcommand_matches("desync").is_some() {

		sync::desync_all()

	} else if let Some(args) = matches.subcommand_matches("search") {
		let any = args.is_present("any");
		let view = !args.is_present("preview");

		let terms = get_terms_from_matches(args);

		if terms.when.is_none() && terms.user.is_none() &&
			terms.term.is_none() && terms.os.is_none() {
			err!("You must enter some terms to search entries.");
		}

		search::search(any, terms, view).await;
	} else if let Some(args) = matches.subcommand_matches("view") {
		// safe to unwrap 'cause Clap would catch if it wasn't included
		let entry = args.value_of("entry").unwrap();

		let mut dir = sync_dir();
		dir.push(entry);

		match search::get_details_of_entry(&dir) {
			Some(ent) => view::view(&ent, Vec::new()),
			None => err!("There appears to be no entry at {:?}", dir),
		}
	} else if let Some(args) = matches.subcommand_matches("prune") {
		let terms = get_terms_from_matches(args);

		if terms.when.is_none() && terms.user.is_none() &&
			terms.term.is_none() && terms.os.is_none() {
			err!("You must enter some terms to prune entries. If you would like to delete all, use the \x1b[1mdesync\x1b[0m command");
		}

		prune::remove_with_terms(terms).await;
	}
}

pub fn get_terms_from_matches(terms: &clap::ArgMatches) -> search::SearchTerms {
	let when = terms.value_of("when").map(|w| w.to_owned());
	let user = terms.value_of("user").map(|u| u.to_owned());
	let term = terms.value_of("term").map(|t| t.to_owned());
	let os = terms.value_of("os").map(|o|
		match o {
			"ios" => search::EntryOS::iOS,
			"android" => search::EntryOS::Android,
			"desktop" => search::EntryOS::Desktop,
			x => {
				err!("Did not recognize os '{}' (must be 'ios, 'android', or 'desktop')", x);
				std::process::exit(1);
			}
		}
	);

	search::SearchTerms { user, term, when, os }
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
		.fold(Vec::new(), | mut lines, link | {
			let splits: Vec<&str> = link.split(&['<', '>'][..]).collect();
			if splits.len() > 3 {
				lines.push(splits[2]);
			}
			lines
		})
}
