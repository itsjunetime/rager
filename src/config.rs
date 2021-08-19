use std::fs::read_to_string;
use crate::err;

pub struct Config {
	pub server: String,
	pub username: String,
	pub password: String,
	pub threads: usize,
	pub beeper_hacks: bool,
	pub sync_retry_limit: Option<usize>
}

impl Config {
	pub fn from_file(file: &Option<String>) -> Option<Config> {
		//let conf = file.unwrap_or_else(Config::default_file_url);
		let conf = file.as_ref()
			.map(|f| f.to_owned())
			.unwrap_or_else(Self::default_file_url);

		match read_to_string(&conf) {
			Ok(text) => match text.parse::<toml::Value>() {
				Ok(val) => if let Some(table) = val.as_table() {

					macro_rules! get_val{
						($key:expr, $fn:ident) => {
							match table.get($key).map(|v| v.$fn()) {
								Some(Some(val)) => val,
								_ => {
									err!("Your config file does not include the field '{}'", $key);
									return None;
								}
							};
						}
					}

					let server = get_val!("server", as_str).to_string();
					let password = get_val!("password", as_str).to_string();
					let username = get_val!("username", as_str).to_string();
					let threads = get_val!("threads", as_integer) as usize;

					let sync_retry_limit = match table.get("sync-retry-limit").map(|s| s.as_integer()) {
						Some(Some(i)) => Some(i as usize),
						_ => None
					};

					let beeper_hacks = table.get("beeper-hacks")
						.map(|v| v.as_bool().unwrap_or(false)).unwrap_or(false);

					return Some(Config {
						server,
						password,
						username,
						threads,
						beeper_hacks,
						sync_retry_limit,
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
