pub enum SyncErrors {
	ListingFailed,
	FilesDownloadFailed(Vec<crate::sync::Download>)
}
