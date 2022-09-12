#[derive(Debug, thiserror::Error)]
pub enum SyncErrors {
	#[error("Retrieving the list of files failed")]
	ListingFailed,
	#[error("A number of files failed to download")]
	FilesDownloadFailed(Vec<crate::sync::Download>),
}

#[derive(Debug, thiserror::Error)]
pub enum FilterErrors {
	#[error("User provided a bad regex term")]
	BadRegexTerm,
	#[error("User tried to filter by a term before downloading")]
	TermFilterBeforeDownloading,
	#[error("User tried to view a file before it was downloaded")]
	ViewingBeforeDownloading,
	#[error("Retrieval of list of files failed")]
	FileRetrievalFailed,
	#[error("Reading file from disk failed")]
	FileReadingFailed,
	#[error("Paging the view to the screen failed")]
	ViewPagingFailed,
}
