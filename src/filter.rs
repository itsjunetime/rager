use crate::{
	err,
	errors::FilterErrors,
	entry::{EntryOS, Entry},
	config::Config
};
use std::{
	cmp::Ordering,
	fs,
	convert::TryInto
};
use chrono::Datelike;

#[derive(Debug)]
pub struct Filter {
	pub oses: Option<Vec<EntryOS>>,
	pub before: Option<[u16; 3]>,
	pub after: Option<[u16; 3]>,
	pub when: Option<[u16; 3]>,
	pub user: Option<String>,
	pub term: Option<String>,
	pub any: bool,
	pub ok_unsure: bool,
}

impl Filter {
	pub fn from_config_file(file: &Option<String>) -> Filter {
		let conf = file.as_ref()
			.map(|f| f.to_owned())
			.unwrap_or_else(Config::default_file_url);

		let text = fs::read_to_string(&conf)
			.unwrap_or_else(|_| panic!("Cannot read contents of the config file at {}", conf));

		let val = text.parse::<toml::Value>().expect("Your config file does not have valid toml syntax");
		let table = val.as_table().expect("Your config file is not a valid toml table");

		macro_rules! some_or_none_str{
			($key:expr, $val:ident, $cl:tt) => {
				match table.get($key) {
					Some(higher) => match higher.as_str() {
						Some($val) => $cl,
						None => None,
					},
					None => None
				}
			}
		}

		let oses = some_or_none_str!("sync-os", o, (
			Some(o.split(',')
				.fold(Vec::new(), | mut sp, o | {
					if let Ok(eos) = o.try_into() {
						sp.push(eos);
					}

					sp
				}))
		));

		macro_rules! sync_str_to_arr{
			($key:expr) => {
				some_or_none_str!($key, v, (
					match Filter::date_array(v) {
						Some(arr) => Some(arr),
						_ => {
							err!("Your {} key does not match ISO-8601 format", $key);
							None
						}
					}
				));
			}
		}

		let before = sync_str_to_arr!("sync-before");
		let after = sync_str_to_arr!("sync-after");
		let when = sync_str_to_arr!("sync-when");

		let user = some_or_none_str!("sync-user", o, (Some(o.to_owned())));

		let any = some_or_none_str!("sync-any", o, (
			Some(o.parse::<bool>().unwrap_or(false))
		)).unwrap_or(false);

		let ok_unsure = some_or_none_str!("sync-unsure", o, (
			Some(o.parse::<bool>().unwrap_or(false))
		)).unwrap_or(false);

		Filter {
			oses,
			before,
			after,
			when,
			user,
			any,
			ok_unsure,
			term: None,
		}
	}

	pub async fn entry_ok(&self, entry: &mut Entry, syncing: bool) -> Result<bool, FilterErrors> {
		if (self.before.is_some() || self.after.is_some() || self.when.is_some()) &&
			self.day_ok(&entry.day) == self.any {
			return Ok(self.any);
		}

		if self.oses.is_some() {
			// now get the OS &&  check that as well
			if entry.get_and_set_os(syncing).await.is_err() {
				return Ok(self.ok_unsure)
			}

			let os = match &entry.os {
				Some(os) => os,
				None => return Ok(self.ok_unsure)
			};

			// if (os_ok && self.any) || (!os_ok && !self.any), basically
			if self.os_ok(os) == self.any {
				return Ok(self.any);
			}
		}

		// also check the user next
		if self.user.is_some() {
			if !entry.checked_details && entry.set_download_values().await.is_err() {
				return Ok(self.ok_unsure);
			}

			let user = match &entry.user_id {
				Some(user) => user,
				None => return Ok(self.ok_unsure)
			};

			if self.user_ok(user) == self.any {
				return Ok(self.any);
			}
		}

		if let Some(ref term) = self.term {
			if !entry.is_downloaded() {
				return Err(FilterErrors::TermFilterBeforeDownloading);
			}

			// since this is the last condition, we can just return it
			return Ok(!entry.files_containing_term(term).await?.is_empty());
		}

		Ok(true)
	}

	pub fn os_ok(&self, os: &EntryOS) -> bool {
		match &self.oses {
			Some(oses) => oses.contains(os),
			None => true,
		}
	}

	pub fn day_ok(&self, date: &str) -> bool {
		if self.before.is_none() && self.after.is_none() && self.when.is_none() {
			return true;
		}

		let date = match Self::date_array(date) {
			Some(arr) => arr,
			_ => return self.ok_unsure,
		};

		match self.any {
			true => self.before_ok(&date) || self.after_ok(&date) || self.when_ok(&date),
			_ => self.before_ok(&date) && self.after_ok(&date) && self.when_ok(&date)
		}
	}

	pub fn before_ok(&self, date: &[u16; 3]) -> bool {
		match self.before {
			Some(before) => {
				for (b, s) in before.iter().zip(date) {
					match b.cmp(s) {
						Ordering::Greater => break,
						Ordering::Less => return false,
						_ => ()
					}
				}

				true
			},
			None => true
		}
	}

	pub fn after_ok(&self, date: &[u16; 3]) -> bool {
		match self.after {
			Some(after) => {
				for (a, s) in after.iter().zip(date) {
					match a.cmp(s) {
						Ordering::Greater => return false,
						Ordering::Less => break,
						_ => ()
					}
				}

				true
			},
			None => true
		}
	}

	pub fn when_ok(&self, date: &[u16; 3]) -> bool {
		match self.when {
			Some(when) => *date == when,
			None => true
		}
	}

	pub fn user_ok(&self, user: &str) -> bool {
		match &self.user {
			Some(u) => u == user,
			None => true
		}
	}

	pub fn string_to_date_match(whens: &Option<String>) -> Option<Vec<[u16; 3]>> {
		whens.as_ref()
			.map(|days| days.split(',')
				.fold(Vec::new(), |mut mtc, day| {
					if let Some(day_arr) = Self::date_array(day) {
						mtc.push(day_arr)
					} else {
						let now = chrono::offset::Utc::now();
						let parse = day.parse::<chrono::Weekday>();

						let days_ago = if day.starts_with("today") {
							Some(0)
						} else if day.starts_with("yesterday") {
							Some(1)
						} else if let Ok(entry) = parse {
							let from_now = now.weekday().num_days_from_sunday();
							let from_then = entry.num_days_from_sunday();

							if from_now != from_then {
								Some((from_now + 7 - from_then) % 7)
							} else {
								Some(7)
							}
						} else {
							None
						};

						if let Some(da) = days_ago {
							if let Some(date_string) = now.with_day(now.day() - da) {
								let date = date_string.format("%Y-%m-%d").to_string();

								if let Some(day_arr) = Self::date_array(&date) {
									mtc.push(day_arr)
								}
							}
						}
					}

					mtc
				})
			)
	}

	pub fn date_array(input: &str) -> Option<[u16; 3]> {
		// remove the trailing slash in case we're passing in a directory
		let fixed = input.replace("/", "");
		let splits: Vec<&str> = fixed.split('-').collect();

		if splits.len() != 3 {
			return None;
		}

		macro_rules! get_split{
			($idx:expr) => {
				match splits[$idx].parse::<u16>() {
					Ok(val) => val,
					_ => return None,
				}
			}
		}

		let first = get_split!(0);
		let second = get_split!(1);
		let third = get_split!(2);

		Some([first, second, third])
	}
}
