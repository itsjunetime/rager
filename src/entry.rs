#![allow(non_camel_case_types)]

use std::{
	convert::TryFrom,
	fs,
	sync::Arc
};
use crate::{
	err,
	sync_dir,
	get_links,
	req_with_auth,
	config,
	errors::FilterErrors
};

pub struct Entry {
	pub day: String, // e.g. `20210721`
	pub time: String, // e.g. `022901`
	pub checked_details: bool,
	pub files: Option<Vec<String>>,
	pub reason: Option<String>,
	pub user_id: Option<String>,
	pub os: Option<EntryOS>,
	pub version: Option<String>,
	pub config: Arc<config::Config>
}

impl Entry {
	pub fn new(day: String, time: String, config: Arc<config::Config>) -> Entry {
		// remove possible trailing directory separators
		let day = day.replace("/", "").replace("\\", "");
		let time = time.replace("/", "").replace("\\", "");

		Entry {
			day,
			time,
			config,
			checked_details: false,
			files: None,
			reason: None,
			user_id: None,
			os: None,
			version: None
		}
	}

	pub fn with_files(day: String, time: String, config: Arc<config::Config>, files: Vec<String>) -> Entry {
		// remove possible trailing directory separators
		let day = day.replace("/", "").replace("\\", "");
		let time = time.replace("/", "").replace("\\", "");

		Entry {
			day,
			time,
			config,
			files: Some(files),
			checked_details: false,
			reason: None,
			user_id: None,
			os: None,
			version: None
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
		if !force_sync {
			if let Ok(contents) = fs::read_dir(&sync_dir) {
                // read through the currently downloaded files
				let files = contents.filter_map(|file| {
					// if it's ok...
					file.ok()
						.and_then(|f|
							// get the path
							f.path()
								// get the file name
								.file_name()
								.and_then(|name| 
									// map it to a string instead of osstr
									name.to_str()
										// and take ownership so we can store it
										.map(|s| s.to_string())
								)
						)
				}).collect::<Vec<String>>();

				self.files = Some(files);

				return Ok(());
			}
		}

		let url = format!("{}/api/listing/{}", self.config.server, self.date_time());

		let response = req_with_auth(&url, &self.config).await?;
		let res_text = response.text().await?;

		let files = get_links(&res_text)
			.into_iter()
			// replace possible trailing slashes just in case
			.map(|l| l.replace("/", ""))
			.collect::<Vec<String>>();

		self.files = Some(files);

		Ok(())
	}

	pub async fn set_download_values(&mut self) -> Result<(), reqwest::Error> {
		let mut dir = sync_dir();
		dir.push(&self.day);
		dir.push(&self.time);
		dir.push("details.log.gz");

		// if we got the details file downloaded, just use it
		let contents = match fs::read_to_string(&dir) {
			Ok(contents) => contents,
			_ => {
				// else, download it and use its contents
				let url = format!("{}/api/listing/{}/details.log.gz", self.config.server, self.date_time());

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

				if line.contains("android") {
					self.os = Some(EntryOS::Android);
				} else if line.contains("web") || line.contains("desktop") {
					self.os = Some(EntryOS::Desktop);
				}

				total_found += 1;
			} else if line.starts_with("user_id") {
				let components: Vec<&str> = line.split(' ').collect();

				if components.len() > 1 {
					self.user_id = Some(components[1].to_owned());
				}

				total_found += 1;
			} else if line.starts_with("Version") || line.starts_with("app_hash") {
				let components: Vec<&str> = line.split(' ').collect();

				if components.len() > 1 {
					self.version = Some(components[1].to_owned());
				}

				total_found += 1;
			} else if line.starts_with("build") {
				let components: Vec<&str> = line.split(' ').collect();

				if components.len() > 1 {
					let build = components[1..].join(" ");

					self.version = match self.version {
						Some(ref vers) => Some(format!("{} ({})", vers, build)),
						None => Some(build)
					};
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
		// if we're using the hacky methods...
		if self.config.beeper_hacks {
			// download the list of files first
			if self.files.is_none() {
				self.retrieve_file_list(force_sync).await?;
			}

			// and then iterate through the files and see if we can detect it that way
			if let Some(ref links) = self.files {
				self.os = if links.iter().any(|l| l.starts_with("console")) {
					Some(EntryOS::iOS)
				} else {
					None
				}
			}
		}

		if self.os.is_none() {
			if let Err(err) = self.set_download_values().await {
				err!("Failed to determine details of entry {}: {}", self.date_time(), err);
			}
		}

		Ok(())
	}

	pub fn description(&self) -> String {
		let unknown = "unknown".to_owned();

		format!("\x1b[1m{}\x1b[0m: {}\n\
			\tOS:       \x1b[32;1m{}\x1b[0m\n\
			\tVersion:  \x1b[32;1m{}\x1b[0m\n\
			\tLocation: {:?}\n",
				self.user_id.as_ref().unwrap_or(&unknown),
				self.reason.as_ref().unwrap_or(&unknown),
				match self.os {
					Some(EntryOS::iOS) => "iOS",
					Some(EntryOS::Android) => "Android",
					Some(EntryOS::Desktop) => "Desktop",
					None => "unknown",
				},
				self.version.as_ref().unwrap_or(&unknown),
				self.date_time()
			)
	}

	pub fn selectable_description(&self) -> String {
		let unknown = "unknown".to_owned();

		format!("{} ({}): {}",
			self.user_id.as_ref().unwrap_or(&unknown),
			match self.os {
				Some(EntryOS::iOS) => "iOS",
				Some(EntryOS::Android) => "Android",
				Some(EntryOS::Desktop) => "Desktop",
				None => "unknown",
			},
			self.reason.as_ref().unwrap_or(&unknown)
		)
	}

	pub fn is_downloaded(&self) -> bool {
		let mut dir = sync_dir();
		dir.push(self.date_time());

		std::path::Path::new(&dir).exists()
	}

	pub async fn files_containing_term(&mut self, term: &str) -> Result<Vec<String>, FilterErrors> {
		let regex = match regex::Regex::new(term) {
			Ok(rg) => rg,
			_ => return Err(FilterErrors::BadRegexTerm)
		};

		let mut dir = sync_dir();
		dir.push(self.date_time());

		if self.files.is_none() {
			let _ = self.retrieve_file_list(false).await;
		}

		// iterate through the current list of files, fold them
		if let Some(files) = &self.files {
			Ok(files.iter()
				.fold(Vec::new(), | mut files, file | {
					let mut file_dir = dir.clone();
					file_dir.push(file);

					// if we can read it to string and it matches the regex, push it
					match fs::read_to_string(&file_dir) {
						Ok(text) if regex.is_match(&text) => {
							files.push(file.to_owned());
							files
						},
						_ => files
					}
				}))
		} else {
			Ok(Vec::new())
		}
	}
}

#[derive(PartialEq, Clone, Debug)]
pub enum EntryOS {
	iOS,
	Android,
	Desktop
}

impl ToString for EntryOS {
	fn to_string(&self) -> String {
		match self {
			EntryOS::iOS => "iOS".to_owned(),
			EntryOS::Android => "Android".to_owned(),
			EntryOS::Desktop => "Desktop".to_owned()
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
