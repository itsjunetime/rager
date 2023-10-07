use crate::{config::Config, entry::Entry, filter::Filter, *};
use std::{
	fs,
	sync::{Arc, Mutex},
};
use requestty::{question::*, PromptModule, OnEsc};

pub async fn search(filter: Filter, config: Config, view: bool) {
	let conf_arc = Arc::new(config);
	let filter_arc = Arc::new(filter);

	let Some(mut finds) = entries_with_filter(&filter_arc, &conf_arc).await else {
		return;
	};

	if finds.is_empty() {
		println!(":( It looks like your search terms didn't turn up any results");
		return;
	}

	for entry in &mut finds {
		if let Err(err) = entry.set_download_values().await {
			err!(
				"Unable to get downloaded values for {}: {:?}",
				entry.date_time(),
				err
			);
		}
	}

	let descriptions = finds
		.iter_mut()
		.map(|e| e.selectable_description())
		.collect::<Vec<String>>();

	let question = Question::select("")
		.message("Matching Entries:")
		.choices(descriptions)
		.on_esc(OnEsc::Terminate)
		.default(0)
		.build();

	let choice = PromptModule::new(vec![question])
		.prompt_all()
		.ok()
		.and_then(|p| p[""].as_list_item().map(|l| l.index));

	if let Some(ch) = choice {
		let mut entry = finds.remove(ch);

		if view {
			// get the entries that contain the specified term so we can pass it to the view fn
			let entries = match filter_arc.term.as_ref() {
				Some(term) => entry.files_containing_term(term).await.ok(),
				_ => None,
			};

			if let Err(err) = view::view(entry, None, entries).await {
				match err {
					ViewingBeforeDownloading => {
						err!("Cannot view a file before downloading the entry");
					}
					FileRetrievalFailed => err!("Failed to determine list of files in entry"),
					FileReadingFailed => err!("Failed to read specified file"),
					ViewPagingFailed => err!("Failed to display file on page"),
					_ => (),
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

	let day_joins = fs::read_dir(&sync_dir)
		.ok()
		.map(|contents| contents
			.filter_map(|day_dir| {
				let day = day_dir.ok().map(|d| d.path())?;

				if let Some(day_str) = day.file_name().and_then(|d| d.to_str()) {
					if !filter.day_ok(day_str) {
						return None;
					}
				}

				// for each of the days ...
				let day_filter = filter.clone();
				let day_conf = config.clone();
				let day_match = matches.clone();

				Some(tokio::spawn(async move {
					// iterate over the times
					if let Ok(times) = fs::read_dir(&day) {
						let time_joins = times
							.filter_map(|time| {
								let time = time.ok().map(|t| t.path())?;

								let time_filter = day_filter.clone();
								let time_conf = day_conf.clone();
								let time_match = day_match.clone();

								macro_rules! final_component {
									($path:ident) => {
										$path
											.file_name()
											.and_then(|nm| nm.to_str())
											.map(|name| name.to_owned())?
									};
								}

								// get the string for the day
								let day_str = final_component!(day);
								let time_str = final_component!(time);

								Some(tokio::spawn(async move {
									let mut entry = Entry::new(&day_str, &time_str, time_conf);

									match time_filter.entry_ok(&mut entry, false).await {
										Err(err) => err!("Error when checking entry: {err:?}"),
										Ok(true) => {
											if let Ok(mut matches) = time_match.lock() {
												matches.push(entry);
											}
										}
										_ => (),
									}
								}))
							});

						futures::future::join_all(time_joins).await;
					}
				}))
			}
		))?;

	futures::future::join_all(day_joins).await;

	Arc::try_unwrap(matches)
		.unwrap_or_else(|_| panic!("matches was thrown onto task that was never completed"))
		.into_inner()
		.ok()
}
