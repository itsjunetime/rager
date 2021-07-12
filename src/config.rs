use std::fs::read_to_string;
use crate::err;

pub struct Config {
	pub server: String,
	pub username: String,
	pub password: String,
	pub threads: usize
}

impl Config {
	pub fn from_file(file: Option<String>) -> Option<Config> {
		let conf = file.unwrap_or_else(Config::default_file_url);

		match read_to_string(&conf) {
			Ok(text) => match text.parse::<toml::Value>() {
				Ok(val) => if let Some(table) = val.as_table() {

					macro_rules! get_val{
						($key:expr) => {
							match table[$key].as_str() {
								Some(val) => val,
								None => {
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

					return Some(Config {
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
