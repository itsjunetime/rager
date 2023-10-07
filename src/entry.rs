#![allow(non_camel_case_types)]

use crate::{
	config, err,
	errors::FilterErrors,
	get_links, req_with_auth,
	sync::{download_files, Download, SyncTracker},
	sync_dir,
};
use std::{
	convert::TryFrom,
	fs,
	sync::{Arc, Mutex},
};

pub struct Entry {
	pub day: String,  // e.g. `2021-07-21`
	pub time: String, // e.g. `022901`
	pub checked_details: bool,
	pub files: Option<Vec<String>>,
	pub reason: Option<String>,
	pub user_id: Option<String>,
	pub os: Option<EntryOS>,
	pub version: Option<String>,
	pub config: Arc<config::Config>,
}

impl Entry {
	pub fn new(day: &str, time: &str, config: Arc<config::Config>) -> Entry {
		// remove possible trailing directory separators
		let day = day.replace(['/', '\\'], "");
		let time = time.replace(['/', '\\'], "");

		Entry {
			day,
			time,
			config,
			checked_details: false,
			files: None,
			reason: None,
			user_id: None,
			os: None,
			version: None,
		}
	}

	pub fn date_time(&self) -> String {
		format!("{}/{}", self.day, self.time)
	}

	pub async fn retrieve_file_list(&mut self, force_sync: bool) -> Result<(), reqwest::Error> {
		let mut sync_dir = sync_dir();
		sync_dir.push(self.date_time());

		// if we're not forcing it to sync, just get the list of files that are
		// currently downloaded to the device
		if force_sync {
			let url = format!("{}/api/listing/{}", self.config.server, self.date_time());

			let response = req_with_auth(&url, &self.config).await?;
			let res_text = response.text().await?;

			let files = get_links(&res_text)
				.into_iter()
				// replace possible trailing slashes just in case
				.map(|l| l.replace('/', ""))
				.collect::<Vec<String>>();

			self.files = Some(files);
		} else if let Ok(contents) = fs::read_dir(&sync_dir) {
			// read through the currently downloaded files
			let files = contents
				.filter_map(std::result::Result::ok)
				.filter_map(|file|
					// get the path
					file.path()
						// get the file name
						.file_name()
						.and_then(|name|
							// map it to a string instead of osstr
							name.to_str()
								// and take ownership so we can store it
								.map(std::string::ToString::to_string)
						))
				.collect::<Vec<String>>();

			self.files = Some(files);
		}

		if let Some(ref mut f) = self.files {
			// sort the files
			f.sort();
		}

		Ok(())
	}

	pub fn details_file(&self) -> std::path::PathBuf {
		let mut dir = sync_dir();
		dir.push(&self.day);
		dir.push(&self.time);
		dir.push(crate::DETAILS);
		dir
	}

	pub async fn set_download_values(&mut self) -> Result<(), reqwest::Error> {
		// if we got the details file downloaded, just use it
		let contents = match fs::read_to_string(self.details_file()) {
			Ok(contents) => contents,
			_ => {
				// else, download it and use its contents
				let url = format!(
					"{}/api/listing/{}/{}",
					self.config.server,
					self.date_time(),
					crate::DETAILS
				);

				let response = req_with_auth(&url, &self.config).await?;
				response.text().await?
			}
		};

		let mut total_found = 0;
		let total = 5;

		for (idx, line) in contents.lines().enumerate() {
			if idx == 0 {
				self.reason = Some(line.to_owned());

				total_found += 1;
			} else if line.starts_with("Application") {
				let lower = line.to_lowercase();

				if lower.contains("android") {
					self.os = Some(EntryOS::Android);
				} else if lower.contains("web") || lower.contains("desktop") {
					self.os = Some(EntryOS::Desktop);
				} else if lower.contains("ios") {
					self.os = Some(EntryOS::iOS);
				}

				total_found += 1;
			} else if line.starts_with("user_id") {
				let mut components = line.split(' ');

				if let Some(user_id) = components.nth(1) {
					self.user_id = Some(user_id.to_owned());
				}

				total_found += 1;
			} else if line.starts_with("Version") || line.starts_with("app_hash") {
				let mut components = line.split(' ');

				if let Some(version) = components.nth(1) {
					self.version = Some(version.to_owned());
				}

				total_found += 1;
			} else if line.starts_with("build") {
				let components: Vec<&str> = line.split(' ').collect();

				if components.len() > 1 {
					let build = components[1..].join(" ");

					self.version = self
						.version
						.as_ref()
						.map(|vers| format!("{vers} ({build})"))
						.or(Some(build));
				}

				total_found += 1;
			}

			if total_found == total {
				break;
			}
		}

		self.checked_details = true;

		Ok(())
	}

	pub async fn get_and_set_os(&mut self, force_sync: bool) -> Result<(), reqwest::Error> {
		let dir = self.details_file();

		// if the details file exists, just load it from that
		if std::path::Path::new(&dir).exists() {
			match self.set_download_values().await {
				Err(err) => err!(
					"Failed to determine details of entry {}: {}",
					self.date_time(),
					err
				),
				_ => return Ok(()),
			}
		}

		// else, if we're using the hacky methods...
		if self.config.beeper_hacks {
			// download the list of files first
			if self.files.is_none() {
				self.retrieve_file_list(force_sync).await?;
			}

			// and then iterate through the files and see if we can detect it that way
			if let Some(ref links) = self.files {
				self.os = links
					.iter()
					.any(|l| l.starts_with("console"))
					.then_some(EntryOS::iOS);
			}
		}

		// if neither of the above works, we'll have to download the details file, so do that
		if self.os.is_none() {
			if let Err(err) = self.set_download_values().await {
				err!(
					"Failed to determine details of entry {}: {}",
					self.date_time(),
					err
				);
			}
		}

		Ok(())
	}

	pub fn description(&self) -> String {
		let unknown = "unknown".to_owned();

		format!(
			"\x1b[1m{}\x1b[0m: {}\n\
			\tOS:       \x1b[32;1m{}\x1b[0m\n\
			\tVersion:  \x1b[32;1m{}\x1b[0m\n\
			\tLocation: {:?}\n",
			self.user_id.as_ref().unwrap_or(&unknown),
			self.reason.as_ref().unwrap_or(&unknown),
			self.os
				.as_ref()
				.map_or_else(|| "unknown".to_string(), std::string::ToString::to_string),
			self.version.as_ref().unwrap_or(&unknown),
			self.date_time()
		)
	}

	pub fn selectable_description(&self) -> String {
		// Yeah I know these are kinda sketchy, just doing indices,
		// but it should be fine, due to how these times are always
		// sent from the api server.
		let time_display =
			String::from(&self.time[..2]) + ":" + &self.time[2..4] + ":" + &self.time[4..];

		format!(
			"{} ({}, on {} at {}): {}",
			self.user_id
				.as_ref()
				.map_or_else(|| "unknown", std::string::String::as_str),
			self.os
				.as_ref()
				.map_or_else(|| "unknown".to_string(), std::string::ToString::to_string),
			self.day,
			time_display,
			self.reason
				.as_ref()
				.map_or_else(|| "unknown", std::string::String::as_str)
		)
	}

	pub fn is_downloaded(&self) -> bool {
		let mut dir = sync_dir();
		dir.push(self.date_time());

		std::fs::read_dir(dir).is_ok_and(|r| r.count() != 0)
	}

	pub async fn files_containing_term(&mut self, term: &str) -> Result<Vec<String>, FilterErrors> {
		let regex = regex::Regex::new(term).map_err(|_| FilterErrors::BadRegexTerm)?;

		let mut dir = sync_dir();
		dir.push(self.date_time());

		if self.files.is_none() {
			let _ = self.retrieve_file_list(false).await;
		}

		// iterate through the current list of files, fold them
		if let Some(files) = &self.files {
			Ok(files
				.iter()
				.filter_map(|file| {
					let mut file_dir = dir.clone();
					file_dir.push(file);

					// if we can read it to string and it matches the regex, push it
					match fs::read_to_string(&file_dir) {
						Ok(text) if regex.is_match(&text) => Some(file.clone()),
						_ => None,
					}
				})
				.collect::<Vec<String>>())
		} else {
			Ok(Vec::new())
		}
	}

	pub async fn ensure_all_files_downloaded(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		if self.is_downloaded() {
			// If files is Some and not empty...
			return Ok(());
		}

		println!(
			"🟡 It appears not all files are downloaded for this entry; downloading all files now"
		);

		self.retrieve_file_list(true).await?;

		let state = Arc::new(Mutex::new(SyncTracker {
			prefix: "Downloading files:".to_owned(),
			started: 0,
			done: 0,
			total: self.files.as_ref().map_or(0, std::vec::Vec::len),
		}));

		if let Some(downloads) = self.files.as_ref().map(|files| {
			files
				.iter()
				.map(|f| Download {
					subdir: self.date_time() + "/" + f,
					is_cache: false,
					state: state.clone(),
					config: self.config.clone(),
				})
				.collect::<Vec<Download>>()
		}) {
			let mut parent_dir = sync_dir();
			parent_dir.push(self.date_time());

			std::fs::create_dir_all(parent_dir)?;

			download_files(downloads, &state, &self.config).await?;
		}

		self.set_download_values().await?;

		Ok(())
	}
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum EntryOS {
	iOS,
	Android,
	Desktop,
}

impl ToString for EntryOS {
	fn to_string(&self) -> String {
		match self {
			EntryOS::iOS => "iOS".to_owned(),
			EntryOS::Android => "Android".to_owned(),
			EntryOS::Desktop => "Desktop".to_owned(),
		}
	}
}

impl TryFrom<&str> for EntryOS {
	type Error = String;

	fn try_from(val: &str) -> Result<Self, Self::Error> {
		let lower = val.to_lowercase();

		if lower.contains("ios") {
			Ok(EntryOS::iOS)
		} else if lower.contains("android") {
			Ok(EntryOS::Android)
		} else if lower.contains("web") || lower.contains("desktop") {
			Ok(EntryOS::Desktop)
		} else {
			Err("EntryOS string must contain 'ios', 'android', 'web', or 'desktop'".to_owned())
		}
	}
}
