#![allow(non_camel_case_types)]

use crate::{
	*,
	config::Config,
	filter::Filter,
	entry::Entry
};
use std::{
	fs,
	sync::{Arc, Mutex},
};

pub async fn search(filter: Filter, config: Config, view: bool) {
	let conf_arc = Arc::new(config);
	let filter_arc = Arc::new(filter);

	let mut finds = match entries_with_filter(&filter_arc, &conf_arc).await {
		Some(mut fs) => {
			if fs.is_empty() {
				println!(":( It looks like your search terms didn't turn up any results");
				return
			}

			for entry in fs.iter_mut() {
				if let Err(err) = entry.set_download_values().await {
					err!("Unable to get downloaded values for {}: {:?}", entry.date_time(), err);
				}
			}

			fs
		},
		None => return,
	};

	let descriptions = finds.iter_mut().map(|e| e.selectable_description());

	let mut menu = youchoose::Menu::new(descriptions);
	let choice = menu.show();

	if !choice.is_empty() {
		let mut entry = finds.remove(choice[0]);

		if view {
			// get the entries that contain the specified term so we can pass it to the view fn
			let entries = match filter_arc.term.as_ref() {
				Some(term) => match entry.files_containing_term(term).await {
					Ok(fil) => Some(fil),
					_ => None
				},
				_ => None
			};

			if let Err(err) = view::view(&mut entry, entries.unwrap_or_default()).await {
				match err {
					ViewingBeforeDownloading => err!("Cannot view a file before downloading the entry"),
					FileRetrievalFailed => err!("Failed to determine list of files in entry"),
					FileReadingFailed => err!("Failed to read specified file"),
					ViewPagingFailed => err!("Failed to display file on page"),
					_ => ()
				}
			}
		} else {
			println!("{}", entry.description());
		}
	}
}

pub async fn entries_with_filter(filter: &Arc<Filter>, config: &Arc<Config>) -> Option<Vec<Entry>> {
	let sync_dir = sync_dir();

	let matches: Arc<Mutex<Vec<Entry>>> = Arc::new(Mutex::new(Vec::new()));

	// go through the top level directory and get all the days
	if let Ok(contents) = fs::read_dir(&sync_dir) {
		let day_joins = contents.map(|day| {
			// for each of the days ...

			let day_filter = filter.clone();
			let day_conf = config.clone();
			let day_match = matches.clone();

			tokio::spawn(async move {
				// ensure that it's Ok()
				let day = match day {
					Ok(day) => day,
					_ => return
				};

				// iterate over the times
				if let Ok(times) = fs::read_dir(&day.path()) {
					let time_joins = times.map(|time| {
						let time_filter = day_filter.clone();
						let time_conf = day_conf.clone();
						let time_match = day_match.clone();

						let day_path = day.path();

						tokio::spawn(async move {
							// make sure each time is Ok()
							let time_path = match time {
								Ok(time) => time.path(),
								_ => return,
							};

							macro_rules! final_component{
								($path:ident) => {
									match $path.file_name() {
										Some(file_name) => match file_name.to_str() {
											Some(name) => name.to_owned(),
											_ => return,
										},
										_ => return,
									}
								}
							}

							// get the string for the day
							let day_str = final_component!(day_path);
							let time_str = final_component!(time_path);

							let mut entry = Entry::new(day_str, time_str, time_conf);

							match time_filter.entry_ok(&mut entry, false).await {
								Err(err) => err!("Error when checking entry: {:?}", err),
								Ok(true) => if let Ok(mut matches) = time_match.lock() {
									matches.push(entry);
								},
								_ => ()
							}
						})
					});

					futures::future::join_all(time_joins).await;
				}
			})
		});

		futures::future::join_all(day_joins).await;
	} else {
		return None;
	};

	// clippy I know you think that it would be possible to omit the return statements but it
	// actually wouldn't cause of compiler errors that occur when I remove the semicolon at the end
	match matches.lock() {
		Ok(mut entries) => {
			let mut new_entries = Vec::new();
			std::mem::swap(&mut new_entries, &mut *entries);
			return Some(new_entries)
		},
		_ => return None
	};
}
