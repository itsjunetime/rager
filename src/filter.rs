use crate::{
	config::Config,
	entry::{Entry, EntryOS},
	err,
	errors::FilterErrors,
	get_last_synced_day,
};
use chrono::Datelike;
use std::{cmp::Ordering, convert::TryInto, fs};

#[derive(Debug)]
pub struct Filter {
	pub oses: Option<Vec<EntryOS>>,
	pub before: Option<[u16; 3]>,
	pub after: Option<[u16; 3]>,
	pub when: Option<Vec<[u16; 3]>>,
	pub user: Option<String>,
	pub term: Option<String>,
	pub any: bool,
	pub reject_unsure: bool,
}

impl Filter {
	pub fn from_config_file(file: &Option<String>) -> Filter {
		let conf = file
			.as_ref()
			.map_or_else(Config::default_file_url, std::borrow::ToOwned::to_owned);

		// These are all safe to panic! or expect because if the config file was not readable
		// or invalid toml or whatever, Config::from_file would've caught it and exited the program
		// before it even reached this
		let text = fs::read_to_string(&conf)
			.unwrap_or_else(|_| panic!("Cannot read contents of the config file at {conf}"));

		let val = text
			.parse::<toml::Value>()
			.expect("Your config file does not have valid toml syntax");
		let table = val
			.as_table()
			.expect("Your config file is not a valid toml table");

		macro_rules! some_or_none_str {
			($key:expr, $val:ident, $cl:tt) => {
				table
					.get($key)
					.and_then(|higher| higher.as_str())
					.and_then(|$val| $cl)
			};
		}

		let oses = some_or_none_str!(
			"sync-os",
			o,
			(Some(
				o.split(',')
					.filter_map(|o| o.try_into().ok())
					.collect::<Vec<_>>()
			))
		);

		macro_rules! sync_str_to_arr {
			($key:expr) => {
				some_or_none_str!(
					$key,
					v,
					(Filter::date_array(v).or_else(|| {
						err!("Your {} key does not match ISO-8601 format", $key);
						None
					}))
				)
			};
		}

		let before = sync_str_to_arr!("sync-before");
		let after = sync_str_to_arr!("sync-after");
		let when = some_or_none_str!("sync-when", v, (Some(Filter::string_to_dates(v))));

		let user = some_or_none_str!("sync-user", o, (Some(o.to_owned())));

		macro_rules! sync_bool {
			($key:expr, $def:expr) => {
				table.get($key).and_then(|h| h.as_bool()).unwrap_or($def)
			};
		}

		let any = sync_bool!("sync-any", false);
		let reject_unsure = !sync_bool!("sync-unsure", false);
		let last_synced = sync_bool!("sync-since-last-day", false);

		if last_synced {
			if let Some(last_day) = get_last_synced_day() {
				return Filter {
					oses,
					user,
					any,
					reject_unsure,
					before: None,
					after: Some(last_day),
					when: None,
					term: None,
				};
			}
		}

		Filter {
			oses,
			before,
			after,
			when,
			user,
			any,
			reject_unsure,
			term: None,
		}
	}

	pub async fn entry_ok(&self, entry: &mut Entry, syncing: bool) -> Result<bool, FilterErrors> {
		// have to make sure they're some 'cause if we have no time specifiers, day_ok
		// will return true and all entries will get through
		if (self.before.is_some() || self.after.is_some() || self.when.is_some())
			&& self.day_ok(&entry.day) == self.any
		{
			return Ok(self.any);
		}

		if std::path::Path::new(&entry.details_file()).exists() {
			if let Err(err) = entry.set_download_values().await {
				err!("Failed to set download details: {}", err);
			}
		}

		if self.oses.is_some() {
			// now get the OS &&  check that as well
			if entry.get_and_set_os(syncing).await.is_err() {
				return Ok(self.reject_unsure);
			}

			let Some(ref os) = &entry.os else {
				return Ok(self.reject_unsure);
			};

			// if (os_ok && self.any) || (!os_ok && !self.any), basically
			if self.os_ok(os) == self.any {
				return Ok(self.any);
			}
		}

		// also check the user next
		if self.user.is_some() {
			if !entry.checked_details && entry.set_download_values().await.is_err() {
				return Ok(self.reject_unsure);
			}

			let Some(ref user) = &entry.user_id else {
				return Ok(self.reject_unsure);
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
		self.oses.as_ref().map_or(true, |oses| oses.contains(os))
	}

	pub fn day_ok(&self, date: &str) -> bool {
		if self.before.is_none() && self.after.is_none() && self.when.is_none() {
			return true;
		}

		let Some(date) = Self::date_array(date) else {
			return self.reject_unsure;
		};

		if self.any {
			self.before_ok(date) || self.after_ok(date) || self.when_ok(date)
		} else {
			self.before_ok(date) && self.after_ok(date) && self.when_ok(date)
		}
	}

	pub fn before_ok(&self, date: [u16; 3]) -> bool {
		self.before.map_or(true, |before| {
			for (b, s) in before.iter().zip(date) {
				match b.cmp(&s) {
					Ordering::Greater => break,
					Ordering::Less => return false,
					Ordering::Equal => (),
				}
			}

			date != before
		})
	}

	pub fn after_ok(&self, date: [u16; 3]) -> bool {
		self.after.map_or(true, |after| {
			for (a, s) in after.iter().zip(date) {
				match a.cmp(&s) {
					Ordering::Greater => return false,
					Ordering::Less => break,
					Ordering::Equal => (),
				}
			}

			date != after
		})
	}

	pub fn when_ok(&self, date: [u16; 3]) -> bool {
		self.when.as_ref().map_or(true, |when| when.contains(&date))
	}

	pub fn user_ok(&self, user: &str) -> bool {
		self.user.as_ref().map_or(true, |u| user.contains(u))
	}

	pub fn string_to_dates(whens: &str) -> Vec<[u16; 3]> {
		whens
			.split(',')
			.filter_map(Self::string_to_single_date)
			.collect()
	}

	pub fn string_to_single_date(day: &str) -> Option<[u16; 3]> {
		Self::date_array(day).or_else(|| {
			let now = chrono::offset::Utc::now();

			let days_ago = if day.starts_with("today") {
				Some(0)
			} else if day.starts_with("yesterday") {
				Some(1)
			} else if let Ok(entry) = day.parse::<chrono::Weekday>() {
				let from_now = now.weekday().num_days_from_sunday();
				let from_then = entry.num_days_from_sunday();

				if from_now == from_then {
					Some(7)
				} else {
					Some((from_now + 7 - from_then) % 7)
				}
			} else {
				return None;
			};

			if let Some(da) = days_ago {
				if let Some(date_string) = now.with_day(now.day() - da) {
					let date = date_string.format("%Y-%m-%d").to_string();

					return Self::date_array(&date);
				}
			}

			None
		})
	}

	pub fn date_array(input: &str) -> Option<[u16; 3]> {
		// remove the trailing slash in case we're passing in a directory
		let fixed = input.replace('/', "");
		let mut splits = fixed.split('-');

		let (Some(first), Some(second), Some(third)) =
			(splits.next(), splits.next(), splits.next())
		else {
			return None;
		};

		let first = first.parse::<u16>().ok()?;
		let second = second.parse::<u16>().ok()?;
		let third = third.parse::<u16>().ok()?;

		Some([first, second, third])
	}
}
