use crate::{entry::Entry, errors::FilterErrors, sync_dir};
use lazy_static::lazy_static;
use regex::Regex;
use std::{fs, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}};
use requestty::{question::Question, PromptModule, OnEsc};

const NUM_REP_STR: &str = "$bfr\x1b[34;1m$num\x1b[0m$aft";
const NS_REP_STR: &str = "\x1b[32;1m$id\x1b[0m";
const FN_REP_STR: &str = " \x1b[35;1m$fn\x1b[0m$aft";
const NULL_REP_STR: &str = "\x1b[31;1m(null)\x1b[0m";
const HEX_REP_STR: &str = "\x1b[33;1m$hex\x1b[0m";
const URL_REP_STR: &str = "\x1b[31;3m$url\x1b[0m";
const ROOM_REP_STR: &str = "\x1b[33;3m$room\x1b[0m";
const USER_REP_STR: &str = "\x1b[36;1m$user\x1b[0m";

const CHUNK_SIZE: usize = 15;

const TERM_WIDTH: usize = 40;
const SECTIONS: usize = 4;
const LOADING_SECTIONS: usize = TERM_WIDTH * SECTIONS;
const LAST_CHARS: [&str; SECTIONS] = [" ", "▎", "▌", "▊"];

lazy_static! {
	static ref NULL_REGEX: Regex = Regex::new(r"\(null\)").unwrap();
	static ref NS_REGEX: Regex = Regex::new(r"(?P<id>\[[a-zA-Z]+\])").unwrap();
	static ref HEX_REGEX: Regex = Regex::new(r"(?P<hex>0x[a-fA-F0-9]+)").unwrap();
	static ref NUM_REGEX: Regex =
		Regex::new(r"(?P<bfr>([^\w]|^))(?P<num>#?\d+((\.|\-|:)\d+)*)(?P<aft>[^\w])").unwrap();
	static ref FN_REGEX: Regex =
		Regex::new(r" (?P<fn>[a-z]+[A-Z][a-zA-Z]*)(?P<aft>(:| ))").unwrap();
	static ref USER_REGEX: Regex = Regex::new(r"(?P<user>@[\w=]+:[^\.]+(\.[a-z]+)+)").unwrap();
	static ref ROOM_REGEX: Regex = Regex::new(r"(?P<room>![a-zA-Z]+:[a-z]+(\.[a-z]+)+)").unwrap();
	static ref URL_REGEX: Regex =
		Regex::new(r"(?P<url>(_matrix|.well-known)(/[\w%\-@:\.!]+)*)").unwrap();
}

pub async fn view(
	mut entry: Entry,
	file: Option<String>,
	matches: Option<Vec<String>>,
) -> Result<(), FilterErrors> {
	if !entry.is_downloaded() {
		entry.ensure_all_files_downloaded()
			.await
			.map_err(|_| FilterErrors::FileRetrievalFailed)?;
	}

	// Get the list of files if it's not yet loaded into memory
	if entry.files.is_none() {
		entry
			.retrieve_file_list(false)
			.await
			.map_err(|_| FilterErrors::FileRetrievalFailed)?;
	}

	// this may be the case if we are viewing this directly.
	// we only do this if these two are none since they are the two displayed on the prompt
	if entry.user_id.is_none() || entry.reason.is_none() {
		entry
			.set_download_values()
			.await
			.map_err(|_| FilterErrors::FileRetrievalFailed)?;
	}

	// grab the files, return if there are none
	let files = match &entry.files {
		Some(files) if !files.is_empty() => files,
		_ => {
			println!("Huh. Looks like there's no logs for this entry.");
			return Ok(());
		}
	};

	// If the user passed in a file, show that one.
	// Else prompt them to choose a file to show
	let to_show = file.or_else(|| {
		// the list of files, formatted to show a string if they match
		let string_paths = files
			.iter()
			.map(|log| {
				if matches.as_ref().is_some_and(|m| m.contains(log)) {
					format!("{log} (matches)")
				} else {
					log.clone()
				}
			});

		// And ask the user what file they'd like to view
		PromptModule::new(vec![
			Question::select("")
				.message("Files:")
				.choices(string_paths)
				.on_esc(OnEsc::Terminate)
				.default(0)
				.build()
			])
			.prompt_all()
			.ok()
			.and_then(|ans|
				ans[""].as_list_item().map(|l|
					files[l.index].clone()
				)
			)
	});

	if let Some(log) = to_show {
		let mut stored_loc = sync_dir();
		stored_loc.push(entry.date_time());
		stored_loc.push(&log);

		println!("Loading in log at {stored_loc:?}...\n");

		let lines_str = fs::read_to_string(stored_loc)
				.map_err(|_| FilterErrors::FileRetrievalFailed)?;

		let lines = lines_str
			.lines()
			.collect::<Vec<&str>>();

		let line_len = lines.len();

		// I tested a bunch of different values for this (ranging from 1 - 1000000) and
		// 15 seemed to fare the best for large files (I was testing one with 600mb, like 5M
		// lines of text iirc). With a smaller file (like 80mb, idk how many lines), a value
		// of 36-50 seemed to fare better but it was already loading in at like 800ms, so
		// I don't think we should be optimizing for that. And it's not worth my time to
		// figure out a solution that changes based on the number of lines since the difference
		// at this point is so small
		let chunks = lines.chunks(CHUNK_SIZE);
		let chunk_count = chunks.len();
		let done = Arc::new(AtomicUsize::new(0));

		let lines_vec = vec![None; line_len];
		let lines_mx: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(lines_vec));

		let chunk_joins = chunks
			.enumerate()
			.map(|(idx, lns)| {
				let line_clone = lines_mx.clone();
				let done_clone = done.clone();
				let joined = lns.join("\n");

				tokio::spawn(async move {
					let colored = colorize_line(&joined);

					if let Ok(mut lines_lock) = line_clone.lock() {
						lines_lock[idx] = Some(colored);
					}

					// we spawn another task here 'cause we don't want to block up the completion
					// of the colorization task with this computation (and the locking that comes
					// along with it and could slow it down a lot)
					tokio::spawn(async move {
						done_clone.fetch_add(1, Ordering::SeqCst);
						let done_now = done_clone.load(Ordering::SeqCst);

						// If we have less chunks to iterate through than we have sections to print
						// in the little loading indicator, we'll be printing something different
						// every time that we get here, so we should continue.
						if chunk_count >= LOADING_SECTIONS && done_now % (chunk_count / LOADING_SECTIONS) != 0 {
							return;
						}

						// calculate percentages to print out a nice little loading thing
						let perc = ((done_now as f64 / chunk_count as f64) * LOADING_SECTIONS as f64) as usize;

						// get the character in the middle that won't be completely empty or full
						let last_char = LAST_CHARS[perc % SECTIONS];
						let already = perc / SECTIONS;

						// print out the progress bar, resetting the cursor and clearing the line
						print!(
							"\x1b[2K\rLoading... [{}{}{}]",
							"█".repeat(already),
							if idx == chunk_count - 1 {
								""
							} else {
								last_char
							},
							" ".repeat((TERM_WIDTH - already).saturating_sub(1))
						);

						// flush stdout so that it actually goes to the screen
						use std::io::Write;
						let _ = std::io::stdout().flush();
					});
				})
			})
			.collect::<Vec<_>>();

		futures::future::join_all(chunk_joins).await;

		let file_contents = Arc::try_unwrap(lines_mx)
			.expect("lines_mx was passed to a buffer that never completed")
			.into_inner()
			.expect("Could not get inner value from Mutex lines_mx")
			.into_iter()
			.flatten()
			.collect::<Vec<String>>()
			.join("\n");

		// so that we can get a pretty newline after printing the colorize loading bar
		println!();

		let pager = minus::Pager::new();

		pager.set_text(file_contents).map_err(|_| FilterErrors::ViewPagingFailed)?;
		pager.set_line_numbers(minus::LineNumbers::Disabled).map_err(|_| FilterErrors::ViewPagingFailed)?;

		// set a nice prompt with all the details that we want them to see
		let prompt_str = format!(
			"{}/{} ({}; {})",
			entry.date_time(),
			log,
			entry.user_id.unwrap_or_else(|| "unknown".to_owned()),
			entry.reason.unwrap_or_else(|| "unknown".to_owned())
		);
		pager.set_prompt(prompt_str).map_err(|_| FilterErrors::ViewPagingFailed)?;

		minus::page_all(pager).map_err(|_| FilterErrors::ViewPagingFailed)?;
	}

	Ok(())
}

fn colorize_line(line: &str) -> String {
	// ya know, I wish there was a better/faster way of doing this. But I simply don't know what.
	let res = NUM_REGEX.replace_all(line, NUM_REP_STR);
	let res = NS_REGEX.replace_all(&res, NS_REP_STR);
	let res = FN_REGEX.replace_all(&res, FN_REP_STR);
	let res = NULL_REGEX.replace_all(&res, NULL_REP_STR);
	let res = HEX_REGEX.replace_all(&res, HEX_REP_STR);
	let res = URL_REGEX.replace_all(&res, URL_REP_STR);
	let res = ROOM_REGEX.replace_all(&res, ROOM_REP_STR);
	let res = USER_REGEX.replace_all(&res, USER_REP_STR);

	res.to_string()
}
