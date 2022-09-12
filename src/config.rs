use crate::err;
use std::fs::read_to_string;

#[derive(Debug)]
pub struct Config {
	// the server to connect to
	pub server: String,
	// the username to use to connect
	pub username: String,
	// the password to connect with
	pub password: String,
	// how many tasks to spawn when syncing
	pub threads: usize,
	// whether or not to use hacky methods to determine os
	pub beeper_hacks: bool,
	// whether or not to cache detail files of undesireable entries
	pub cache_details: bool,
	// how many times to retry
	pub sync_retry_limit: Option<usize>,
	// the token to interact with the linear api
	pub linear_token: Option<String>
}

impl Config {
	pub fn from_file(file: &Option<String>) -> Option<Config> {
		// get the file, default if they passed in none
		let conf = file
			.as_ref()
			.map(|f| f.to_owned())
			.unwrap_or_else(Self::default_file_url);

		let text = read_to_string(&conf)
			.ok()
			.or_else(|| {
				err!(
					"Please place a config file at {}; \
					see github for details on what to include in it.",
					conf
				);
				None
			})?;

		let val = text.parse::<toml::Value>()
			.map_err(|err| {
				err!("Config file at {} is not proper TOML format: {}", conf, err);
				err
			})
			.ok()?;

		let table = val.as_table()?;

		// a nice macro to get a value from a toml table
		// and error out if that value doesn't exist
		macro_rules! get_val {
			($key:expr, $fn:ident) => {
				table.get($key).map(|v| v.$fn())
					.flatten()
					.or_else(|| {
						err!(
							"Your config file does not include the field '{}'",
							$key
						);
						None
					})?
			};
		}

		let server = get_val!("server", as_str)
			// Need to make sure it has no trailing slashes
			.trim_matches('/')
			.to_string();
		let password = get_val!("password", as_str).to_string();
		let username = get_val!("username", as_str).to_string();
		let threads = get_val!("threads", as_integer) as usize;

		// don't error out on this one tho
		let sync_retry_limit = table
			.get("sync-retry-limit")
			.and_then(|s| s.as_integer())
			.map(|i| i as usize);

		let beeper_hacks = table
			.get("beeper-hacks")
			.and_then(|v| v.as_bool())
			.unwrap_or(false);

		let cache_details = table
			.get("cache-details")
			.and_then(|v| v.as_bool())
			.unwrap_or(false);

		let linear_token = table
			.get("linear-token")
			.and_then(|t| t.as_str().map(|s| s.to_string()));

		Some(Config {
			server,
			password,
			username,
			threads,
			beeper_hacks,
			cache_details,
			sync_retry_limit,
			linear_token
		})
	}

	pub fn default_file_url() -> String {
		// safe to unwrap 'cause the documentation says it always returns `Some`
		let mut config_dir = dirs::config_dir().unwrap();
		config_dir.push("rager");
		config_dir.set_extension("toml");

		config_dir.to_str().unwrap_or_default().to_string()
	}
}
