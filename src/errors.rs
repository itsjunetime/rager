pub enum SyncErrors {
	ListingFailed,
	FilesDownloadFailed(Vec<crate::sync::Download>),
}

#[derive(Debug)]
pub enum FilterErrors {
	BadRegexTerm,
	TermFilterBeforeDownloading,
	ViewingBeforeDownloading,
	FileRetrievalFailed,
	FileReadingFailed,
	ViewPagingFailed,
}
