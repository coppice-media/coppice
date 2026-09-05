//! Errors surfaced by the export model and sinks.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnnotationSyncError {
	#[error("database error: {0}")]
	Db(#[from] sea_orm::DbErr),
	#[error("io error: {0}")]
	Io(#[from] std::io::Error),
	#[error("serialization error: {0}")]
	Json(#[from] serde_json::Error),
	#[error("sink error: {0}")]
	Sink(String),
	/// A non-fast-forward push survived its single rebase retry as a conflict;
	/// the local branch is left untouched and the error is surfaced to the job.
	#[error("git conflict while publishing; the branch needs manual attention: {0}")]
	GitConflict(String),
	#[error("git error: {0}")]
	Git(String),
}

impl AnnotationSyncError {
	pub fn sink(message: impl Into<String>) -> Self {
		Self::Sink(message.into())
	}
}

#[cfg(feature = "git")]
impl From<git2::Error> for AnnotationSyncError {
	fn from(error: git2::Error) -> Self {
		if error.code() == git2::ErrorCode::NotFastForward
			|| error.code() == git2::ErrorCode::MergeConflict
		{
			Self::GitConflict(error.message().to_string())
		} else {
			Self::Git(error.message().to_string())
		}
	}
}
