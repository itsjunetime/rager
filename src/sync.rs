use crate::*;
use std::{
	fs,
	sync::{Arc, Mutex},
};
use futures::StreamExt;

pub async fn sync_logs(config: config::Config) {

	// normally I opt for a RwLock over a mutex but both this and to_check basically only ever
	// write, (state never reads, to_check only reads once and it's after everyone finishes writing
	// to it), so there's really no reason to choose RwLock over mutex here.
	let state = Arc::new(Mutex::new(SyncTracker {
		prefix: "Checking Directories:".to_owned(),
		started: 0,
		done: 0,
		total: 0
	}));

	// to_check is vec of all the files that'll need to be downloaded this time. We iterate through
	// all the entries on the server, check if that file exists on the computer, and if it doesn't,
	// we add it to this array.
	//
	// Then, once we've checked all the files on the server and have a complete list of the ones we
	// need to download, we pass it into the futures::sream::iter func below and download all of
	// them through tokio.
	let to_check: Arc<Mutex<Vec<Download>>> = Arc::new(Mutex::new(Vec::new()));
	let conf = Arc::new(config);

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

	println!("Starting sync with server...");

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

	let list_url = format!("{}/api/listing/", conf.server);

	// get the list of days to check from the server
	let days = match req_with_auth(&list_url, &conf).await {
		Ok(d) => d,
		Err(err) => {
			err!("Couldn't get list of days to check from server: {}", err);
			return;
		}
	};

	let days_text = match days.text().await {
		Ok(dt) => dt,
		Err(err) => {
			err!("Server's list of days contains unparseable text: {}", err);
			return;
		}
	};

	let day_links = get_links(&days_text);
	
	println!("Finding the files that need to be downloaded...");

	// for each day...
	let day_joins = day_links.into_iter()
		.fold(Vec::new(), | mut joins, d | {

			let mut day_log_dir = log_dir.clone();
			let day = d.to_owned();
			day_log_dir.push(d);

			let day_state = state.clone();
			let day_conf = conf.clone();
			let day_to_check = to_check.clone();

			let day_url = format!("{}{}", list_url, day);

			// spawn a new thread for each entry in each day, since we have to check all the files
			// in each entry
			let join = tokio::spawn(async move {
				let times = match req_with_auth(&day_url, &*day_conf).await {
					Ok(tm) => tm,
					Err(err) => {
						st_err!(day_state, "Could not get list of times of day {}: {}", day, err);
						return;
					}
				};

				let times_text = match times.text().await {
					Ok(tt) => tt,
					Err(err) => {
						st_err!(day_state, "Could not get text for list of times of day {}: {}", day, err);
						return;
					}
				};

				let time_lines = get_links(&times_text);

				if let Ok(mut state) = day_state.lock() {
					state.add_to_size(time_lines.len());
				}

				let time_joins = time_lines
					.into_iter()
					.fold(Vec::new(), | mut joins, t | {

						let mut time_log_dir = day_log_dir.clone();
						let time = t.to_owned();
						time_log_dir.push(t);

						let time_state = day_state.clone();
						let time_conf = day_conf.clone();
						let time_to_check = day_to_check.clone();

						let time_url = format!("{}{}", day_url, time);
						let day_time = format!("{}{}", day, time);

						if let Ok(mut state) = day_state.lock() {
							state.add_one_started();
						}

						// and then spawn a new thread for each entry...
						let join = tokio::spawn(async move {
							macro_rules! finish{
								() => {
									if let Ok(mut state) = time_state.lock() {
										state.finished_one();
									}
									return;
								}
							}

							if let Err(err) = fs::create_dir_all(&time_log_dir) {
								st_err!(time_state, "Could not create directory {:?}: {}", time_log_dir, err);
								finish!();
							}

							let files = match req_with_auth(&time_url, &*time_conf).await {
								Ok(f) => f,
								Err(err) => {
									st_err!(time_state, "Could not retrieve list of files at {}: {}", time_url, err);
									finish!();
								}
							};

							let files_text = match files.text().await {
								Ok(ft) => ft,
								Err(err) => {
									st_err!(time_state, "Could not get text for list of files at {}: {}", time_url, err);
									finish!();
								}
							};

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
											subdir: format!("{}{}", day_time, f),
											state: time_state.clone(),
											config: time_conf.clone()
										});
									}
								}
							}

							finish!();
						});

						joins.push(join);

						joins
					});

				futures::future::join_all(time_joins).await;
				
			});

			joins.push(join);
			joins

		});

	futures::future::join_all(day_joins).await;

	// change the progress bar title to reflect that we're downloading individual files now,
	// instead of looking through entries. Also reset the counts.
	if let Ok(mut state) = state.lock() {
		state.prefix = "Downloaded:".to_owned();
		state.total = 0;
		state.done = 0;
		state.started = 0;
	}

	if let Ok(downloads) = to_check.lock() {
		if downloads.is_empty() {
			println!("\n✅ You're already all synced up!");
			return;
		}

		println!("\nStarting file downloads...");

		if let Ok(mut state) = state.lock() {
			state.total = downloads.len();
		}

		// iterate through all the files that we need to download and download them.
		futures::stream::iter(
			downloads.iter().map(|down| {
				macro_rules! finish{
					() => {
						if let Ok(mut state) = down.state.lock() {
							state.finished_one();
						}
						return;
					}
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

					st_log!(down.state, "Downloading file \x1b[32;1m{}\x1b[0m", down.subdir);
					let request = match req_with_auth(&down_url, &*down.config).await {
						Ok(req) => req,
						Err(err) => {
							st_err!(down.state, "Failed to download file {}: {}", down.subdir, err);
							finish!();
						}
					};

					match request.text().await {
						Ok(text) => match fs::write(&down_dir, text.as_bytes()) {
							Err(err) => st_err!(down.state, "Couldn't write file to {:?}: {}", down_dir, err),
							Ok(_) => st_log!(down.state, "✅ Saved file \x1b[32;1m{}\x1b[0m", down.subdir),
						}
						Err(err) => st_err!(down.state, "Failed to get text from downloaded file {}: {}", down.subdir, err),
					}

					finish!();
				}
			})
		).buffer_unordered(conf.threads)
			.collect::<Vec<()>>()
			.await;
	};
}

pub fn desync_all() {
	// ugh. nestedness.
	if let Ok(mut contents) = std::fs::read_dir(&sync_dir()) {
		while let Some(dir) = contents.next() {
			if let Ok(path) = dir {
				if path.path().is_dir() {
					match std::fs::remove_dir_all(&path.path()) {
						Ok(_) => println!("Removed logs at {:?}", path.path()),
						Err(err) => err!("Unable to remove logs at {:?}: {}", path.path(), err)
					}
				}
			}
		}
	}
}

struct Download {
	pub subdir: String,
	pub state: Arc<Mutex<SyncTracker>>,
	pub config: Arc<config::Config>
}

struct SyncTracker {
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

			print!("{}{} \x1b[32;1m{}\x1b[1m/\x1b[32m{}\x1b[0m ({} in progress)", clear, self.prefix, self.done, self.total, self.started);
			let _ = std::io::stdout().flush();

		} else {
			println!("\x1b[2K\r✨ Finished");
		}
	}
}
