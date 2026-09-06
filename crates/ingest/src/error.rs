//! The ingest pipeline's own error type.
//!
//! The crate is host-agnostic (it never sees `stump_core`), so it cannot
//! return `CoreError`. Hosts map [`IngestError`] into whatever their transport
//! needs; `stump_core` converts it into `CoreError` (see
//! `core/src/ingest_host.rs`) and GraphQL renders it through a
//! `Display`-generic mapper.

use std::io;

use thiserror::Error;

pub type IngestResult<T> = Result<T, IngestError>;

#[derive(Error, Debug)]
pub enum IngestError {
	#[error("Requested resource could not be found: {0}")]
	NotFound(String),
	#[error("{0}")]
	BadRequest(String),
	#[error("Requested file could not be found: {0}")]
	FileNotFound(String),
	#[error("Failed to read file: {0}")]
	IoError(#[from] io::Error),
	#[error("Failed to initialize the ingest pipeline: {0}")]
	InitializationError(String),
	#[error("{0}")]
	InternalError(String),
	#[error("Query error: {0}")]
	DBError(#[from] sea_orm::error::DbErr),
	#[error("An object failed to (de)serialize: {0}")]
	SerdeFailure(#[from] serde_json::Error),
	#[error("An unknown error occurred: {0}")]
	Unknown(String),
}

impl From<stump_media::FileError> for IngestError {
	fn from(error: stump_media::FileError) -> Self {
		match error {
			stump_media::FileError::FileIoError(err) => Self::IoError(err),
			stump_media::FileError::UnknownError(err) => Self::Unknown(err),
			error => Self::InternalError(error.to_string()),
		}
	}
}
