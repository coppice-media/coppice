use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use thiserror::Error;

pub type ScanResult<T> = Result<T, ScanError>;

#[derive(Debug, Error)]
pub enum ScanError {
	#[error("scan source error: {0}")]
	Source(String),
	#[error("scan walk error: {0}")]
	Internal(String),
}

impl ScanError {
	/// Converts an adapter error into the scanner's transport-independent error type.
	pub fn source(error: impl std::fmt::Display) -> Self {
		Self::Source(error.to_string())
	}
}

/// The status values relevant to scan reconciliation.
///
/// This deliberately mirrors the small portion of the persistence status contract needed by the
/// walker without making the scanner depend on SeaORM model entities.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScanStatus {
	#[default]
	Unknown,
	Ready,
	Unsupported,
	Error,
	Missing,
}

impl ScanStatus {
	pub fn is_recovered_if_present(self) -> bool {
		matches!(self, Self::Missing | Self::Unknown)
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeriesIdentity {
	pub id: String,
	pub path: String,
	pub status: ScanStatus,
}

impl SeriesIdentity {
	pub fn new(
		id: impl Into<String>,
		path: impl Into<String>,
		status: ScanStatus,
	) -> Self {
		Self {
			id: id.into(),
			path: path.into(),
			status,
		}
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaIdentity {
	pub id: String,
	pub path: String,
	pub modified_at: Option<DateTime<FixedOffset>>,
	pub hash: Option<String>,
	pub status: ScanStatus,
}

impl MediaIdentity {
	pub fn new(
		id: impl Into<String>,
		path: impl Into<String>,
		modified_at: Option<DateTime<FixedOffset>>,
		status: ScanStatus,
	) -> Self {
		Self {
			id: id.into(),
			path: path.into(),
			modified_at,
			hash: None,
			status,
		}
	}
}

/// Read-only persistence boundary used by the filesystem walker.
///
/// Implementations provide only the identities and directory mtimes needed to plan a scan. The
/// SeaORM implementation lives in `stump_core`; this crate never receives a database connection,
/// application context, job runtime, or event sender.
#[async_trait]
pub trait ScanSource: Send + Sync {
	async fn existing_series(&self, library_id: &str) -> ScanResult<Vec<SeriesIdentity>>;
	async fn existing_media(&self, series_id: &str) -> ScanResult<Vec<MediaIdentity>>;
	async fn stored_dir_mtimes(
		&self,
		library_id: &str,
	) -> ScanResult<HashMap<String, u64>>;
}
