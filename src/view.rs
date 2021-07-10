use std::fs;
use crate::{
	search::EntryDetails,
	err
};
use regex::Regex;
use lazy_static::lazy_static;

const NULL_COLOR: &'static str = "\x1b[31;1m";
const NS_COLOR: &'static str = "\x1b[32;1m";
const HEX_COLOR: &'static str = "\x1b[33;1m";
const NUM_COLOR: &'static str = "\x1b[34;1m";
const FN_COLOR: &'static str = "\x1b[35;1m";
const USER_COLOR: &'static str = "\x1b[36;1m";
const RESET: &'static str = "\x1b[0m";

lazy_static! {
	static ref NULL_REGEX: Regex = Regex::new(r"\(null\)").unwrap();
	static ref NS_REGEX: Regex = Regex::new(r"(?P<id>\[[a-zA-Z]+\])").unwrap();
	static ref HEX_REGEX: Regex = Regex::new(r"(?P<hex>0x[a-fA-F0-9]+)").unwrap();
	static ref NUM_REGEX: Regex = Regex::new(r"(?P<bfr>[^\w])(?P<num>#?\d+(\.\d+)*)(?P<aft>[^\w])").unwrap();
	static ref FN_REGEX: Regex = Regex::new(r"(?P<bfr>[^a-zA-Z])(?P<fn>[a-z]+[A-Z][a-zA-Z]*)").unwrap();
	static ref USER_REGEX: Regex = Regex::new(r"(?P<user>@\w+:[^\.]+\.(com|org|net))").unwrap();
}

pub fn view(entry: &EntryDetails) {
	let logs = if let Ok(contents) = fs::read_dir(&entry.path) {
		contents.fold(Vec::new(), |mut entries, log_res| {
			if let Ok(log) = log_res {
				entries.push(log.path());
			}

			entries
		})
	} else {
		println!("Huh. Looks like there's no logs for this entry");
		return;
	};

	if logs.is_empty() {
		println!("Huh. Looks like there's no logs for this entry.");
		return;
	}

	let string_paths = logs.iter()
		.fold(Vec::new(), | mut files, log | {
			if let Some(ref_str) = log.to_str() {
				files.push(ref_str.to_owned());
			}
			files
		});

	let mut menu = youchoose::Menu::new(string_paths.iter());
	let choice = menu.show();

	if !choice.is_empty() {
		let log = &logs[choice[0]];

		println!("Loading in log at {:?}...", log);

		let file_contents = match fs::read_to_string(log) {
			Ok(fc) => fc.lines()
				.map(|l| colorize_line(l))
				.collect::<Vec<String>>()
				.join("\n"),
			Err(err) => {
				err!("Couldn't load in contents of log: {}", err);
				return;
			}
		};

		let mut pager = minus::Pager::new().unwrap();

		pager.set_text(file_contents);
		pager.set_prompt(log.to_str().unwrap_or("rager"));

		if let Err(err) = minus::page_all(pager) {
			err!("Can't page output: {}", err);
			return;
		}
	}
}

fn colorize_line(line: &str) -> String {
	let res = NUM_REGEX.replace_all(&line, format!("$bfr{}$num{}$aft", NUM_COLOR, RESET));
	let res = NS_REGEX.replace_all(&res, format!("{}$id{}", NS_COLOR, RESET));
	let res = FN_REGEX.replace_all(&res, format!("$bfr{}$fn{}", FN_COLOR, RESET));
	let res = NULL_REGEX.replace_all(&res, format!("{}(null){}", NULL_COLOR, RESET));
	let res = HEX_REGEX.replace_all(&res, format!("{}$hex{}", HEX_COLOR, RESET));
	let res = USER_REGEX.replace_all(&res, format!("{}$user{}", USER_COLOR, RESET));

	res.to_string()
}
