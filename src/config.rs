use std::{
	fs::read_to_string,
	convert::TryInto,
	cmp::Ordering
};
use crate::{
	err,
	search,
	sync,
	search::EntryOS,
};

pub struct Config {
	pub server: String,
	pub username: String,
	pub password: String,
	pub threads: usize,
	pub filter: SyncFilter
}

impl Config {
	pub fn from_file(file: Option<String>) -> Option<Config> {
		let conf = file.unwrap_or_else(Config::default_file_url);

		match read_to_string(&conf) {
			Ok(text) => match text.parse::<toml::Value>() {
				Ok(val) => if let Some(table) = val.as_table() {

					macro_rules! get_val{
						($key:expr) => {
							match table.get($key).map(|v| v.as_str()) {
								Some(Some(val)) => val,
								_ => {
									err!("Your config file does not include the field '{}'", $key);
									return None;
								}
							};
						}
					}

					let server = get_val!("server");
					let password = get_val!("password");
					let username = get_val!("username");
					let threads = match table["threads"].as_integer() {
						Some(val) => val as usize,
						None => {
							err!("Your config file must include an integer with the key 'threads'");
							return None;
						}
					};

					macro_rules! some_or_none_str{
						($key:expr, $cl:tt) => {
							match table.get($key) {
								Some(higher) => match higher.as_str() {
									Some(val) => $cl(val),
									None => None,
								},
								None => None
							}
						}
					}

					let oses = some_or_none_str!("sync-os", (|o: &str|
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
							some_or_none_str!($key, (|v: &str|
								match SyncFilter::string_to_arr(v) {
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

					let ok_unsure = table["ok-unsure"].as_bool().unwrap_or(true);

					return Some(Config {
						filter: SyncFilter { oses, before, after, ok_unsure },
						server: server.to_string(),
						password: password.to_string(),
						username: username.to_string(),
						threads
					})
				},
				Err(err) => err!("Config file at {} is not proper TOML format: {}", conf, err),
			},
			_ => err!("Please place a config file at {}; \
				see github for details on what to include in it.", conf),
		}

		None
	}

	pub fn default_file_url() -> String {
		let mut config_dir = dirs::config_dir().unwrap();
		config_dir.push("rager");
		config_dir.set_extension("toml");

		config_dir.to_str()
			.unwrap_or_default()
			.to_string()
	}
}

pub struct SyncFilter {
	pub oses: Option<Vec<EntryOS>>,
	pub before: Option<[u16; 3]>,
	pub after: Option<[u16; 3]>,
	pub ok_unsure: bool
}

impl SyncFilter {
	pub fn string_to_arr(input: &str) -> Option<[u16; 3]> {
		let splits: Vec<&str> = input.split('-').collect();

		if splits.len() != 3 {
			return None;
		}

		macro_rules! get_split{
			($idx:expr) => {
				match splits[$idx].parse::<u16>() {
					Ok(val) => val,
					_ => { return None; }
				}
			}
		}

		let first = get_split!(0);
		let second = get_split!(1);
		let third = get_split!(2);

		Some([first, second, third])
	}

	pub async fn entry_allowed(&self, day_time: &str, conf: &std::sync::Arc<Config>) -> bool {
		if self.before.is_some() || self.after.is_some() {

			// If this is not safe to unwrap, something is massively wrong
			let name = day_time
				.split('/')
				.collect::<Vec<&str>>()[0];

			let splits = match Self::string_to_arr(name) {
				Some(sp) => sp,
				_ => return self.ok_unsure
			};

			if let Some(before) = self.before {
				for (b, s) in before.iter().zip(&splits) {
					// if the `before` year is after the date, it's bad (cause it has to be before
					// 'before'), and if it's before, then we're good.
					match b.cmp(s) {
						Ordering::Greater => break,
						Ordering::Less => return false,
						_ => ()
					}
				}
			}

			if let Some(after) = self.after {
				for (a, s) in after.iter().zip(&splits) {
					match a.cmp(s) {
						Ordering::Greater => return false,
						Ordering::Less => break,
						_ => ()
					}
				}
			}
		}

		// only actually look at the entry details themselves if we have an OS we want
		if let Some(ref oses) = self.oses {

			let mut on_device = crate::sync_dir();
			on_device.push(day_time);

			// check if we have it already downloaded
			if let Some(downloaded_entry) = search::get_details_of_entry(&on_device) {
				if !oses.contains(&downloaded_entry.os) {
					return false;
				}
			} else {
				// if we don't, get the details from the server and check there.
				match sync::get_online_details(day_time, conf).await {
					Some(entry) => if !oses.contains(&entry.os) {
						return false;
					},
					None => return self.ok_unsure
				}
			}
		}

		true
	}
}
