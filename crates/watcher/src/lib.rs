//! Library filesystem watching.
//!
//! `stump_watcher` owns `notify` setup, change debouncing, path accumulation, and the watcher
//! lifecycle. It deliberately contains no database, job queue, application context, or event
//! type: the owning crate implements [`WatchedLibraries`] to say which roots are watched and
//! [`ScanSubmitter`] to turn a debounced change into a scan.

mod watcher;

use async_trait::async_trait;

pub use watcher::{Watcher, DEFAULT_DEBOUNCE};

/// A boxed error returned by the owner-supplied traits.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A library whose root directory is watched for changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
	pub id: String,
	pub path: String,
}

/// A request to scan one library after its root changed on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRequest {
	pub library_id: String,
	pub path: String,
}

/// Supplies the libraries whose roots should currently be watched.
#[async_trait]
pub trait WatchedLibraries: Send + Sync {
	async fn enabled_libraries(&self) -> Result<Vec<LibraryRoot>, BoxError>;
}

/// Receives a scan request for a library whose root changed.
#[async_trait]
pub trait ScanSubmitter: Send + Sync {
	async fn submit(&self, request: ScanRequest) -> Result<(), BoxError>;
}

#[derive(Debug, thiserror::Error)]
pub enum WatcherError {
	/// The listener task is no longer running, so no command can be delivered.
	#[error("{0}")]
	Stopped(String),
	/// The filesystem backend rejected a watch or could not be created.
	#[error("{0}")]
	Notify(#[from] notify::Error),
	#[error("Failed to load watched libraries: {0}")]
	Libraries(#[source] BoxError),
	#[error("Failed to submit scan for library {library_id}: {source}")]
	Submit {
		library_id: String,
		#[source]
		source: BoxError,
	},
}

impl WatcherError {
	/// Whether the error is `notify` reporting that the watched path does not exist.
	pub fn is_path_not_found(&self) -> bool {
		matches!(
			self,
			WatcherError::Notify(notify::Error {
				kind: notify::ErrorKind::PathNotFound,
				..
			})
		)
	}
}
