use std::fs;
use crate::{
	sync_dir,
	entry::Entry,
	errors::FilterErrors
};
use regex::Regex;
use lazy_static::lazy_static;

const NULL_COLOR: &str = "\x1b[31;1m";
const NS_COLOR: &str = "\x1b[32;1m";
const HEX_COLOR: &str = "\x1b[33;1m";
const NUM_COLOR: &str = "\x1b[34;1m";
const FN_COLOR: &str = "\x1b[35;1m";
const USER_COLOR: &str = "\x1b[36;1m";
const ROOM_COLOR: &str = "\x1b[33;3m";
const URL_COLOR: &str = "\x1b[31;3m";
const RESET: &str = "\x1b[0m";

lazy_static! {
	static ref NULL_REGEX: Regex = Regex::new(r"\(null\)").unwrap();
	static ref NS_REGEX: Regex = Regex::new(r"(?P<id>\[[a-zA-Z]+\])").unwrap();
	static ref HEX_REGEX: Regex = Regex::new(r"(?P<hex>0x[a-fA-F0-9]+)").unwrap();
	static ref NUM_REGEX: Regex = Regex::new(r"(?P<bfr>([^\w]|^))(?P<num>#?\d+((\.|\-|:)\d+)*)(?P<aft>[^\w])").unwrap();
	static ref FN_REGEX: Regex = Regex::new(r" (?P<fn>[a-z]+[A-Z][a-zA-Z]*)(?P<aft>(:| ))").unwrap();
	static ref USER_REGEX: Regex = Regex::new(r"(?P<user>@[\w=]+:[^\.]+(\.[a-z]+)+)").unwrap();
	static ref ROOM_REGEX: Regex = Regex::new(r"(?P<room>![a-zA-Z]+:[a-z]+(\.[a-z]+)+)").unwrap();
	static ref URL_REGEX: Regex = Regex::new(r"(?P<url>(_matrix|.well-known)(/[\w%\-@:\.!]+)*)").unwrap();
}

pub async fn view(entry: &mut Entry, matches: Vec<String>) -> Result<(), FilterErrors> {
	if !entry.is_downloaded() {
		return Err(FilterErrors::ViewingBeforeDownloading);
	}

	if entry.files.is_none() {
		entry.retrieve_file_list(false).await
			.map_err(|_| FilterErrors::FileRetrievalFailed)?;
	}

	macro_rules! p_and_ok { () => {{
		println!("Huh. Looks like there's no logs for this entry.");
		return Ok(());
	}}}

	let files = match &entry.files {
		None => p_and_ok!(),
		Some(files) if files.is_empty() => p_and_ok!(),
		Some(files) => files
	};

	let string_paths = files.iter()
		.fold(Vec::new(), | mut files, log | {
			let log_str = if matches.contains(&log) {
				format!("{} (matches)", log)
			} else {
				log.to_owned()
			};

			files.push(log_str);
			files
		});

	let mut menu = youchoose::Menu::new(string_paths.iter());
	let choice = menu.show();

	if !choice.is_empty() {
		let log = &files[choice[0]];

		let mut stored_loc = sync_dir();
		stored_loc.push(entry.date_time());
		stored_loc.push(log);

		println!("Loading in log at {:?}...", stored_loc);

		let file_contents = match fs::read_to_string(stored_loc) {
			Ok(fc) => fc.lines()
				.map(|l| colorize_line(l))
				.collect::<Vec<String>>()
				.join("\n"),
			_ => return Err(FilterErrors::FileReadingFailed),
		};

		let mut pager = minus::Pager::new().unwrap();

		pager.set_text(file_contents);

		let prompt_str = format!("{}/{} ({}; {})", 
			entry.date_time(),
			log,
			entry.user_id.as_ref().unwrap_or(&"unknown".to_owned()),
			entry.reason.as_ref().unwrap_or(&"unknown".to_owned())
		);
		pager.set_prompt(prompt_str);

		minus::page_all(pager).map_err(|_| FilterErrors::ViewPagingFailed)?;
	}

	Ok(())
}

fn colorize_line(line: &str) -> String {
	let res = NUM_REGEX.replace_all(line, format!("$bfr{}$num{}$aft", NUM_COLOR, RESET));
	let res = NS_REGEX.replace_all(&res, format!("{}$id{}", NS_COLOR, RESET));
	let res = FN_REGEX.replace_all(&res, format!(" {}$fn{}$aft", FN_COLOR, RESET));
	let res = NULL_REGEX.replace_all(&res, format!("{}(null){}", NULL_COLOR, RESET));
	let res = HEX_REGEX.replace_all(&res, format!("{}$hex{}", HEX_COLOR, RESET));
	let res = URL_REGEX.replace_all(&res, format!("{}$url{}", URL_COLOR, RESET));
	let res = ROOM_REGEX.replace_all(&res, format!("{}$room{}", ROOM_COLOR, RESET));
	let res = USER_REGEX.replace_all(&res, format!("{}$user{}", USER_COLOR, RESET));

	res.to_string()
}
