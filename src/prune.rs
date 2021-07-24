use crate::{
	search::{
		SearchTerms,
		entries_with_terms,
		dbg_str_for_terms
	},
	err
};

pub async fn remove_with_terms(terms: SearchTerms) {
	let entries = match entries_with_terms(false, &terms).await {
		Some(e) => {
			if e.is_empty() {
				println!("Your conditions did not turn up any results :(");
				return
			}

			e
		},
		None => return
	};

	println!("Pruning logs{}...", dbg_str_for_terms(&terms));

	for e in entries.into_iter() {
		match std::fs::remove_dir_all(&e.path) {
			Err(err) => err!("Could not remove logs at {:?}: {}", e.path, err),
			_ => println!("Deleted entry at {:?}", e.path)
		}
	}
}
