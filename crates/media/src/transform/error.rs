//! Errors surfaced by the comic transform pipeline.

use thiserror::Error;

/// Everything that can go wrong while transforming comic pages or writing a
/// transformed container.
#[derive(Debug, Error)]
pub enum TransformError {
	/// A source page could not be decoded into an image.
	#[error("failed to decode page image: {0}")]
	Decode(String),

	/// A transformed page could not be encoded into its target format.
	#[error("failed to encode page image: {0}")]
	Encode(String),

	/// The container writer or a source iterator failed on I/O.
	#[error(transparent)]
	Io(#[from] std::io::Error),

	/// A source or output archive could not be read or written.
	#[error("archive error: {0}")]
	Archive(String),

	/// The media path is not a transformable comic container.
	#[error("unsupported comic source: {0}")]
	UnsupportedSource(String),

	/// A stage is compiled out (e.g. `pdf`, `rar`, or `kepub` features off).
	#[error("{0} support is disabled in this build")]
	FeatureDisabled(&'static str),

	/// Any other pipeline failure.
	#[error("transform failed: {0}")]
	Other(String),
}

/// Convenience alias used across the transform module.
pub type TransformResult<T> = Result<T, TransformError>;

impl From<zip::result::ZipError> for TransformError {
	fn from(error: zip::result::ZipError) -> Self {
		Self::Archive(error.to_string())
	}
}
