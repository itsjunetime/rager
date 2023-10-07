use crate::{
	err,
	entry::Entry, 
	config::Config,
	view::view
};
use std::sync::Arc;

pub async fn find_issue(
	team: &str, issue: u16, config: Config
) -> Result<(), Box<dyn std::error::Error>> {
	let Some(ref linear_token) = config.linear_token else {
		err!("Looks like you're missing a token to interact with the linear API.\nGet one from \x1b[1mhttps://linear.app/settings/api\x1b[0m and then add it to the config file under the \x1b[1mlinear-token\x1b[0m key");
		return Ok(());
	};

	let mut query = std::collections::HashMap::new();
	query.insert("query", format!("{{ issues(filter: {{ number: {{ eq: {issue} }} team: {{ key: {{ eq: \"{team}\" }} }} }}) {{ nodes {{ description }} }} }}"));

	let text = reqwest::Client::new()
		.post("https://api.linear.app/graphql")
		.header("Content-Type", "application/json")
		.json(&query)
		.header("Authorization", linear_token)
		.send()
		.await?
		.text()
		.await?;

	let filtered_url = config.server.replace('.', "\\.");
	let rageshake_url_regex = regex::Regex::new(&(filtered_url + "/api/listing/\\d{4,}\\-\\d{2,}\\-\\d{2,}/\\d{6,}"))?;

	if let Some(entry) = rageshake_url_regex
		.captures(&text)
		.and_then(|c|
			 c.get(0).map(|f| {
				let url = f.as_str();
				let len = url.len();
				// get the day
				let day = &url[len - 17..len - 7];
				let time = &url[len - 6..];
				Entry::new(day, time, Arc::new(config))
			})
		) {
			println!("✨ Found logs! (\x1b[1m{}\x1b[0m)", entry.date_time());

			view(entry, None, None).await?;
	} else {
		err!("It appears that the description of this issue contains no links to rageshake logs");
	}

	Ok(())
}
