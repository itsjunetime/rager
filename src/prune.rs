use crate::{
	err,
	sync_dir,
	search::entries_with_filter,
	filter::Filter,
	config::Config
};
use std::sync::Arc;

pub async fn remove_with_terms(filter: Filter, config: Config) {
	let filter_arc = Arc::new(filter);
	let config_arc = Arc::new(config);

	let entries = match entries_with_filter(&filter_arc, &config_arc).await {
		Some(e) => {
			if e.is_empty() {
				println!("Your conditions did not turn up any results :(");
				return
			}

			e
		},
		None => return
	};

	let log_dir = sync_dir();

	for e in entries.into_iter() {
		let mut entry_dir = log_dir.clone();
		entry_dir.push(e.date_time());

		match std::fs::remove_dir_all(&entry_dir) {
			Err(err) => err!("Could not remove logs at {:?}: {}", entry_dir, err),
			_ => println!("Deleted entry at {:?}", entry_dir)
		}
	}
}
