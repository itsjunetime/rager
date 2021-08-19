use crate::{
	*,
	config::Config,
	filter::Filter,
	errors::SyncErrors::*,
	entry::Entry
};
use std::{
	fs,
	sync::{
		Arc,
		Mutex,
		atomic::{
			AtomicBool,
			Ordering
		}
	},
};
use futures::StreamExt;

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
	filter: &Arc<Filter>, conf: &Arc<Config>, state: &Arc<Mutex<SyncTracker>>
) -> Result<(), errors::SyncErrors> {

	// to_check is vec of all the files that'll need to be downloaded this time. We iterate through
	// all the entries on the server, check if that file exists on the computer, and if it doesn't,
	// we add it to this array.
	//
	// Then, once we've checked all the files on the server and have a complete list of the ones we
	// need to download, we pass it into the futures::sream::iter func below and download all of
	// them through tokio.
	let to_check: Arc<Mutex<Vec<Download>>> = Arc::new(Mutex::new(Vec::new()));

	let log_dir = sync_dir();

	let mut first_time = !log_dir.exists();
	if !first_time  {
		first_time = match fs::read_dir(&log_dir) {
			Err(_) => true,
			Ok(entries) => entries.count() == 0,
		}
	}

	// just warn them if it's the first time they're syncing, since it'll probably take a while.
	if first_time {
		warn!("It appears you are syncing for the first time. This may take a while.\n");
	}

	if filter.oses.is_some() && !conf.beeper_hacks {
		warn!("You have a sync filter for specific OS(es). This means that sync may take significantly longer \
			than expected, since the server will have to check the OS of every entry from the server \
			before downloading any files.");
	}

	let list_url = format!("{}/api/listing/", conf.server);

	// get the list of days to check from the server
	let days = match req_with_auth(&list_url, &conf).await {
		Ok(d) => d,
		Err(err) => {
			err!("Couldn't get list of days to check from server: {}", err);
			return Err(ListingFailed)
		}
	};

	let days_text = match days.text().await {
		Ok(dt) => dt,
		Err(err) => {
			err!("Server's list of days contains unparseable text: {}", err);
			return Err(ListingFailed)
		}
	};

	let day_links = get_links(&days_text);
	let day_len = day_links.len();

	println!("Finding the files that need to be downloaded...");

	let failed_a_listing = Arc::new(AtomicBool::new(false));

	macro_rules! fail_listing{
		($mux:expr) => {
			let f = $mux.load(Ordering::Relaxed);
			$mux.store(!f, Ordering::Relaxed);
		}
	}

	// for each day...
	let day_joins = day_links.into_iter()
		.enumerate()
		.map(|(idx, d)| {

			let mut day_log_dir = log_dir.clone();
			let day = d.to_owned();
			day_log_dir.push(d);

			let day_state = state.clone();
			let day_conf = conf.clone();
			let day_to_check = to_check.clone();
			let day_fail = failed_a_listing.clone();
			let day_filter = filter.clone();

			let day_url = format!("{}{}", list_url, day);

			// spawn a new thread for each entry in each day, since we have to
			// check all the files in each entry
			tokio::spawn(async move {

				// before querying to get the list of entries for a specific day, just
				// make sure the day itself is allowed. Optimizations.
				if !day_filter.day_ok(&day) {
					if let Ok(mut state) = day_state.lock() {
						if idx == day_len {
							state.finalized_size = true;
						}
					}

					return;
				}

				let times = match req_with_auth(&day_url, &*day_conf).await {
					Ok(tm) => tm,
					Err(err) => {
						st_err!(day_state, "Could not get list of times of day {}: {}", day, err);
						fail_listing!(day_fail);
						return;
					}
				};

				let times_text = match times.text().await {
					Ok(tt) => tt,
					Err(err) => {
						st_err!(day_state, "Could not get text for list of times of day {}: {}", day, err);
						fail_listing!(day_fail);
						return;
					}
				};

				let time_lines = get_links(&times_text);

				if let Ok(mut state) = day_state.lock() {
					state.add_to_size(time_lines.len());

					// We check to set this finalized_size because we can't predict the order in
					// which these tokio tasks will execute. It may completely check all
					// directories but one, then start on the final directory.
					//
					// However, if this happens and we don't have a way of verifying that we've
					// added the total number of directories to the state, it will think it's done
					// when it hasn't even started on one directory
					//
					// So we need a way of manually telling it that we're not done yet, even if
					// we've downloaded the number of files that we've said we need to download.
					// Hence the flag.
					if idx == day_len {
						state.finalized_size = true;
					}
				}

				let time_joins = time_lines
					.into_iter()
					.map(|t| {

						let mut time_log_dir = day_log_dir.clone();
						let time = t.to_owned();
						time_log_dir.push(t);

						let day = day.to_owned();

						let time_state = day_state.clone();
						let time_conf = day_conf.clone();
						let time_to_check = day_to_check.clone();
						let time_fail = day_fail.clone();
						let time_filter = day_filter.clone();

						let time_url = format!("{}{}", day_url, time);

						if let Ok(mut state) = day_state.lock() {
							state.add_one_started();
						}

						// and then spawn a new thread for each entry...
						tokio::spawn(async move {
							macro_rules! finish{
								() => {
									if let Ok(mut state) = time_state.lock() {
										state.finished_one();
									}
									return;
								};
								($state:expr$(, $args:expr)*) => {
									{
										st_err!($state$(, $args)*);
										fail_listing!(time_fail);
										finish!();
									}
								}
							}

							let files = match req_with_auth(&time_url, &*time_conf).await {
								Ok(f) => f,
								Err(err) => finish!(time_state, "Could not retrieve list of files at {}: {}", time_url, err),
							};

							let files_text = match files.text().await {
								Ok(ft) => ft,
								Err(err) => finish!(time_state, "Could not get text for list of files at {}: {}", time_url, err),
							};

							let mut entry = Entry::new(day, time, time_conf.clone());

							let entry_ok = match time_filter.entry_ok(&mut entry, true).await {
								Ok(ok) => ok,
								_ => {
									st_log!(time_state, "entry {:?} is deemed bad", entry.date_time());
									return
								}
							};

							if entry_ok {
								if let Err(err) = fs::create_dir_all(&time_log_dir) {
									finish!(time_state, "Could not create directory {:?}: {}", time_log_dir, err);
								}

								// and iterate through the list of files (not the content of the files,
								// just the list of them) and check if they exist on the computer.
								for f in get_links(&files_text) {
									let mut file_log_dir = time_log_dir.clone();
									file_log_dir.push(f);

									// if they don't exist, append them to the list of files to
									// download.
									if !std::path::Path::new(&file_log_dir).exists() {
										if let Ok(mut check) = time_to_check.lock() {
											check.push(Download {
												subdir: format!("{}/{}", entry.date_time(), f),
												state: time_state.clone(),
												config: time_conf.clone()
											});
										}
									}
								}
							}

							finish!();
						})
					});

				futures::future::join_all(time_joins).await;

			})
		});

	futures::future::join_all(day_joins).await;

	if failed_a_listing.load(Ordering::Relaxed) {
		return Err(ListingFailed);
	}

	// change the progress bar title to reflect that we're downloading individual files now,
	// instead of looking through entries. Also reset the counts.
	// We don't need to reset the finalized_size flag because we set the total before actually
	// spawning any tasks, so we won't run into the same issue as above.
	if let Ok(mut state) = state.lock() {
		state.reset("Downloaded:".to_owned());
	}

	if let Ok(mut downloads) = to_check.lock() {
		if downloads.is_empty() {
			println!("\n✅ You're already all synced up!");
			return Ok(());
		}

		println!("\nDownloading files...");

		let mut empty = Vec::new();
		std::mem::swap(&mut *downloads, &mut empty);

		return download_files(empty, &state, &conf).await;
	};

	Ok(())
}

pub async fn download_files(files: Vec<Download>, state: &Arc<Mutex<SyncTracker>>, conf: &Arc<config::Config>) -> Result<(), errors::SyncErrors> {
	let log_dir = sync_dir();
	let list_url = format!("{}/api/listing/", conf.server);

	if let Ok(mut state) = state.lock() {
		state.total = files.len();
	}

	let failed_files: Arc<Mutex<Vec<Download>>> = Arc::new(Mutex::new(Vec::new()));

	// iterate through all the files that we need to download and download them.
	futures::stream::iter(
		files.into_iter().map(|down| {
			let state_clone = state.clone();

			macro_rules! finish{ () => {
				if let Ok(mut stt) = state_clone.lock() {
					stt.finished_one();
				}
				return;
			}}

			// get the url to request and the directory which the file will be written to.
			let down_url = format!("{}{}", list_url, down.subdir);
			let mut down_dir = log_dir.clone();
			down_dir.push(&down.subdir);

			let fail_clone = failed_files.clone();

			macro_rules! fail_file{
				($file:expr) => {
					if let Ok(mut files) = fail_clone.lock() {
						files.push($file);
					}
				}
			}

			// create an async block, which will be what is executed on the `await`
			async move {
				if let Ok(mut state) = down.state.lock() {
					state.add_one_started();
				}

				st_log!(down.state, "Downloading file \x1b[32;1m{}\x1b[0m", down.subdir);
				let request = match req_with_auth(&down_url, &*down.config).await {
					Ok(req) => req,
					Err(err) => {
						st_err!(down.state, "Failed to download file {}: {}", down.subdir, err);
						fail_file!(down);
						finish!();
					}
				};

				match request.text().await {
					Ok(text) => match fs::write(&down_dir, text.as_bytes()) {
						Err(err) => {
							st_err!(down.state, "Couldn't write file to {:?}: {}", down_dir, err);
							fail_file!(down);
						},
						Ok(_) => st_log!(down.state, "✅ Saved file \x1b[32;1m{}\x1b[0m", down.subdir),
					}
					Err(err) => {
						st_err!(down.state, "Failed to get text from downloaded file {}: {}", down.subdir, err);
						fail_file!(down);
					}
				}

				finish!();
			}
		})
	).buffer_unordered(conf.threads)
		.collect::<Vec<()>>()
		.await;

	return match failed_files.lock() {
		Ok(mut files) => match files.is_empty() {
			true => Ok(()),
			_ => {
				let mut replace = Vec::new();
				std::mem::swap(&mut *files, &mut replace);
				Err(FilesDownloadFailed(replace))
			}
		}
		_ => Ok(())
	};
}

pub fn desync_all() {
	if let Ok(mut contents) = std::fs::read_dir(&sync_dir()) {
		while let Some(Ok(path)) = contents.next() {
			if !path.path().is_dir() {
				continue;
			}

			match std::fs::remove_dir_all(&path.path()) {
				Ok(_) => println!("Removed logs at {:?}", path.path()),
				Err(err) => err!("Unable to remove logs at {:?}: {}", path.path(), err)
			}
		}
	}
}

pub struct Download {
	pub subdir: String,
	pub state: Arc<Mutex<SyncTracker>>,
	pub config: Arc<config::Config>
}

pub struct SyncTracker {
	pub started: usize,
	pub done: usize,
	pub total: usize,
	pub finalized_size: bool,
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

		if self.done < self.total && !self.finalized_size {
			let clear = if clear {
				"\x1b[2K\r"
			} else {
				""
			};

			print!("{}{} \x1b[32;1m{}\x1b[1m/\x1b[32m{}\x1b[0m ({} in progress)", clear, self.prefix, self.done, self.total, self.started);
			// have to flush stdout 'cause it's line-buffered and this print! doesn't have a newline
			let _ = std::io::stdout().flush();

		} else {
			println!("\x1b[2K\r✨ Finished");
		}
	}

	pub fn reset(&mut self, title: String) {
		self.prefix = title;
		self.total = 0;
		self.done = 0;
		self.started = 0;
	}
}
