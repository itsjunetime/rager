use crate::{config::Config, entry::Entry, errors::SyncErrors::*, filter::Filter, *};
use futures::StreamExt;
use std::{
	fs,
	sync::{Arc, Mutex},
};

// a special macro so that we can remove the progress bar, print a line, and have the progress
// bar reappear underneat the line that was just printed
macro_rules! st_log{
	($state:expr, $msg:expr$(, $args:expr)*) => {
		{
			println!("\x1b[2K\r{}", format!($msg$(, $args)*));
			if let Ok(mut state) = $state.lock() {
				state.update(false);
			}
		}
	}
}

macro_rules! st_err{
	($state:expr, $msg:expr$(, $args:expr)*) => {
		st_log!($state, "{}", format!("{} {}", crate::ERR_PREFIX, format!($msg$(, $args)*)));
	}
}

// returns a vector of failed files, or none if all downloaded successfully.
// if it fails on something other than downloading a file, it will return an empty vector
pub async fn sync_logs(
	filter: &Arc<Filter>,
	conf: &Arc<Config>,
	state: &Arc<Mutex<SyncTracker>>,
) -> Result<(), errors::SyncErrors> {
	// a convenience struct to wrap a few simple things
	let helper = Arc::new(Mutex::new(SyncHelper {
		failed_listing: false,      // if we failed to get a listing of days or times
		to_download: Vec::new(),    // a list of files to download
		times_to_check: Vec::new(), // a list of times to check for files we need to download
	}));

	let log_dir = sync_dir();

	let mut first_time = !log_dir.exists();
	if !first_time {
		first_time = match fs::read_dir(&log_dir) {
			Err(_) => true,
			Ok(entries) => entries.count() == 0,
		}
	}

	// just warn them if it's the first time they're syncing, since it'll probably take a while.
	if first_time {
		warn!("It appears you are syncing for the first time. This may take a while.\n");
	}

	if filter.oses.is_some() && !(conf.beeper_hacks || conf.cache_details) {
		warn!(
			"You have a sync filter for specific OS(es). This means that sync may take significantly longer \
			than expected, since the server will have to check the OS of every entry from the server \
			before downloading any files."
		);
	}

	let list_url = format!("{}/api/listing/", conf.server);

	// get the list of days to check from the server
	let days_text = match req_with_auth(&list_url, conf).await {
		Ok(days) => match days.text().await {
			Ok(dt) => dt,
			Err(err) => {
				err!("Server's list of days contains unparseable text: {}", err);
				return Err(ListingFailed);
			}
		},
		Err(err) => {
			err!("Couldn't get list of days to check from server: {}", err);
			return Err(ListingFailed);
		}
	};

	let day_links = get_links(&days_text);

	// just filter out the ones that are not ok right off the bat
	let day_links = day_links
		.iter()
		.filter(|d| filter.day_ok(d))
		.collect::<Vec<_>>();

	println!("Finding the files that need to be downloaded...");

	if let Ok(mut state) = state.lock() {
		state.reset("Checking days:".to_owned());
		state.add_to_size(day_links.len());
	}

	// for each day...
	let day_joins = day_links.into_iter().map(|d| {
		let mut day_log_dir = log_dir.clone();
		let day = d.to_owned();
		day_log_dir.push(d);

		let day_state = state.clone();
		let day_conf = conf.clone();
		let day_helper = helper.clone();

		let day_url = format!("{}{}", list_url, day);

		macro_rules! finish{
				() => {
					if let Ok(mut state) = day_state.lock() {
						state.finished_one();
					}
					return;
				};
				($msg:expr$(, $args:expr)*) => {{
					st_err!(day_state, $msg$(, $args)*);
					if let Ok(mut helper) = day_helper.lock() {
						helper.failed_listing = true;
					}
					finish!();
				}}
			}

		// spawn a new thread for each entry in each day, since we have to
		// check all the files in each entry
		async move {
			if let Ok(mut state) = day_state.lock() {
				state.add_one_started();
			}

			let times_text = match req_with_auth(&day_url, &*day_conf).await {
				Ok(tm) => match tm.text().await {
					Ok(tt) => tt,
					Err(err) => finish!(
						"Could not get text for list of times of day {}: {}",
						day,
						err
					),
				},
				Err(err) => finish!("Could not get list of times of day {}: {}", day, err),
			};

			let time_lines = get_links(&times_text);

			let mut times = time_lines
				.into_iter()
				.map(|t| (day.replace("/", ""), t.replace("/", "")))
				.collect::<Vec<(String, String)>>();

			if let Ok(mut helper) = day_helper.lock() {
				helper.times_to_check.append(&mut times);
			}

			finish!();
		}
	});

	// first we buffer checking all the days, so that we don't overload TCP connections
	futures::stream::iter(day_joins)
		.buffer_unordered(conf.threads)
		.collect::<Vec<()>>()
		.await;

	// swap it out with the mutex-blocked struct so that we can use it outside
	let swap_array = if let Ok(mut helper) = helper.lock() {
		std::mem::take(&mut helper.times_to_check)
	} else {
		Vec::new()
	};

	if let Ok(mut state) = state.lock() {
		state.reset("Checking times:".to_owned());
		state.add_to_size(swap_array.len());
	}

	// then buffer through checking all the days, once again so that we don't overload
	futures::stream::iter(swap_array.into_iter().map(|(day, time)| {
		let mut time_log_dir = sync_dir();
		time_log_dir.push(&day);
		time_log_dir.push(&time);

		let time_state = state.clone();
		let time_conf = conf.clone();
		let time_filter = filter.clone();
		let time_helper = helper.clone();

		if let Ok(mut state) = state.lock() {
			state.add_one_started();
		}

		// get the url to check the files in this day
		let time_url = format!("{}/api/listing/{}/{}", conf.server, day, time);

		async move {
			// a convenience macro to show an error, clean up, and return
			macro_rules! finish {
					() => {
						if let Ok(mut state) = time_state.lock() {
							state.finished_one();
						}
						return;
					};
					($msg:expr$(, $args:expr)*) => {{
						st_err!(time_state, $msg$(, $args)*);
						if let Ok(mut helper) = time_helper.lock() {
							helper.failed_listing = !helper.failed_listing;
						}
						finish!();
					}}
				}

			let mut entry = Entry::new(day, time, time_conf.clone());

			// check the entry to make sure we should actually download its files
			let entry_ok = match time_filter.entry_ok(&mut entry, true).await {
				Ok(ok) => ok,
				_ => finish!("Failed to get details for entry at {}", time_url),
			};

			if entry_ok || time_conf.cache_details {
				if let Err(err) = fs::create_dir_all(&time_log_dir) {
					finish!("Could not create directory {:?}: {}", time_log_dir, err);
				}
			}

			if entry_ok {
				// Before now, the filter check just loaded in the list of files
				// from the device, but since this is an entry we want, we must force sync them
				if let Err(err) = entry.retrieve_file_list(true).await {
					finish!(
						"Could not retrieve file list for {:?}: {}",
						entry.date_time(),
						err
					);
				}

				// iterate over the files, which must be downloaded now
				if let Some(ref files) = entry.files {
					for f in files {
						let mut file_log_dir = time_log_dir.clone();
						file_log_dir.push(f);

						// ... and if they don't already exist, add them to the
						// list of files to be downloaded
						if !std::path::Path::new(&file_log_dir).exists() {
							if let Ok(mut helper) = time_helper.lock() {
								helper.to_download.push(Download {
									subdir: format!("{}/{}", entry.date_time(), f),
									is_cache: false,
									state: time_state.clone(),
									config: time_conf.clone(),
								});
							}
						}
					}
				}
			} else if time_conf.cache_details {
				// just grab the details file for this one
				time_log_dir.push(crate::DETAILS);

				if !std::path::Path::new(&time_log_dir).exists() {
					if let Ok(mut helper) = time_helper.lock() {
						helper.to_download.push(Download {
							subdir: format!("{}/{}", entry.date_time(), crate::DETAILS),
							is_cache: true,
							state: time_state.clone(),
							config: time_conf.clone(),
						});
					}
				}
			}

			finish!();
		}
	}))
	.buffer_unordered(conf.threads)
	.collect::<Vec<()>>()
	.await;

	// if we were unable to get the list of files in one of the day/times, just return an err
	if let Ok(helper) = helper.lock() {
		if helper.failed_listing {
			return Err(ListingFailed);
		}
	}

	// change the progress bar title to reflect that we're downloading individual files now,
	// instead of looking through entries. Also reset the counts.
	// We don't need to reset the finalized_size flag because we set the total before actually
	// spawning any tasks, so we won't run into the same issue as above.
	if let Ok(mut state) = state.lock() {
		state.reset("Downloaded:".to_owned());
	}

	// The Arc should only have one reference now, so we can try_unwrap it,
	// then move the value out of the inner mutex and pass it to the download_files
	let expect_err = "Helper was thrown onto unbuffered task";
	let downloads = match Arc::try_unwrap(helper)
		.unwrap_or_else(|_| panic!("{}", expect_err))
		.into_inner()
	{
		Ok(helper) if !helper.to_download.is_empty() => helper.to_download,
		_ => {
			println!("\n✅ You're already all synced up!");
			return Ok(());
		}
	};

	println!("\nDownloading files...");

	download_files(downloads, state, conf).await
}

pub async fn download_files(
	files: Vec<Download>,
	state: &Arc<Mutex<SyncTracker>>,
	conf: &Arc<config::Config>,
) -> Result<(), errors::SyncErrors> {
	let log_dir = sync_dir();
	let list_url = format!("{}/api/listing/", conf.server);

	if let Ok(mut state) = state.lock() {
		state.total = files.len();
	}

	let failed_files: Arc<Mutex<Vec<Download>>> = Arc::new(Mutex::new(Vec::new()));

	// iterate through all the files that we need to download and download them.
	futures::stream::iter(files.into_iter().map(|down| {
		let state_clone = state.clone();

		let fail_clone = failed_files.clone();

		macro_rules! finish{
				() => {
					if let Ok(mut stt) = state_clone.lock() {
						stt.finished_one();
					}
					return;
				};
				($msg:expr$(, $args:expr)*) => {{
					st_err!(down.state, $msg$(, $args)*);
					if let Ok(mut files) = fail_clone.lock() {
						files.push(down);
					}
					finish!();
				}}
			}

		// get the url to request and the directory which the file will be written to.
		let down_url = format!("{}{}", list_url, down.subdir);
		let mut down_dir = log_dir.clone();
		down_dir.push(&down.subdir);

		// create an async block, which will be what is executed on the `await`
		async move {
			if let Ok(mut state) = down.state.lock() {
				state.add_one_started();
			}

			let (action, fail_action, finish_action) = if down.is_cache {
				("Caching", "cache", "Cached")
			} else {
				("Downloading", "download", "Saved")
			};

			// inform that we're downloading the file
			st_log!(
				down.state,
				"{} file \x1b[32;1m{}\x1b[0m",
				action,
				down.subdir
			);

			// actualy download the file
			let request = match req_with_auth(&down_url, &*down.config).await {
				Ok(req) => req,
				Err(err) => finish!("Failed to {} file {}: {}", fail_action, down.subdir, err),
			};

			// if we can get the text, write it to the file since they're all text files
			match request.text().await {
				Ok(text) => match fs::write(&down_dir, text.as_bytes()) {
					Err(err) => finish!("Couldn't write file to {:?}: {}", down_dir, err),
					Ok(_) => st_log!(
						down.state,
						"✅ {} file \x1b[32;1m{}\x1b[0m",
						finish_action,
						down.subdir
					),
				},
				Err(err) => finish!(
					"Failed to get text from requested file {}: {}",
					down.subdir,
					err
				),
			}

			finish!();
		}
	}))
	.buffer_unordered(conf.threads)
	.collect::<Vec<()>>()
	.await;

	// if we did fail to download some files, pull the inner value out of the Arc<Mutex<_>>
	// and return that with the error
	let expect_err = "failed_files was passed to a buffer that did not finish";
	match Arc::try_unwrap(failed_files)
		.unwrap_or_else(|_| panic!("{}", expect_err))
		.into_inner()
	{
		Ok(files) if !files.is_empty() => Err(FilesDownloadFailed(files)),
		_ => Ok(()),
	}
}

// just get rid of all the logs
pub fn desync_all() {
	if let Ok(contents) = std::fs::read_dir(&sync_dir()) {
		for path in contents
			.filter_map(|c| c.ok().map(|p| p.path()))
			.filter(|p| p.is_dir())
		{
			match std::fs::remove_dir_all(&path) {
				Ok(_) => println!("Removed logs at {:?}", path),
				Err(err) => err!("Unable to remove logs at {:?}: {}", path, err),
			}
		}
	}
}

// just some nice structs that I don't want to throw elsewhere
pub struct Download {
	pub subdir: String,
	pub is_cache: bool,
	pub state: Arc<Mutex<SyncTracker>>,
	pub config: Arc<config::Config>,
}

pub struct SyncTracker {
	pub started: usize,
	pub done: usize,
	pub total: usize,
	pub prefix: String,
}

impl SyncTracker {
	pub fn add_one_started(&mut self) {
		self.started += 1;
		self.update(true);
	}

	pub fn add_to_size(&mut self, add: usize) {
		self.total += add;
		self.update(true);
	}

	pub fn finished_one(&mut self) {
		self.done += 1;
		self.started -= 1;
		self.update(true);
	}

	pub fn update(&mut self, clear: bool) {
		use std::io::Write;

		if self.done < self.total {
			let clear = if clear {
				"\x1b[2K\r"
			} else {
				""
			};

			print!(
				"{}{} \x1b[32;1m{}\x1b[1m/\x1b[32m{}\x1b[0m ({} in progress)",
				clear, self.prefix, self.done, self.total, self.started
			);
			// have to flush stdout 'cause it's line-buffered and this print! doesn't have a newline
			let _ = std::io::stdout().flush();
		} else {
			println!("\x1b[2K\r✨ Finished with {} items", self.total);
		}
	}

	pub fn reset(&mut self, title: String) {
		self.prefix = title;
		self.total = 0;
		self.done = 0;
		self.started = 0;
	}
}

pub struct SyncHelper {
	pub failed_listing: bool,
	pub to_download: Vec<Download>,
	pub times_to_check: Vec<(String, String)>,
}
