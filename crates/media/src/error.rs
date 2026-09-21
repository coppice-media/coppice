use std::io;

use thiserror::Error;
use zip::result::ZipError;

#[derive(Error, Debug)]
pub enum FileError {
	#[error("Error occurred while opening file: {0}")]
	FileIoError(#[from] io::Error),
	#[error("A zip error occurred: {0}")]
	ZipFileError(#[from] ZipError),
	#[error("Archive contains no files")]
	ArchiveEmptyError,
	#[error("Failed to deserialize file: {0}")]
	DeserializeError(#[from] serde_json::Error),
	#[error("Unable to open .epub file: {0}")]
	EpubOpenError(String),
	#[error("Error while attempting to read .epub file: {0}")]
	EpubReadError(String),
	#[error("Unable to read .mobi/.azw file: {0}")]
	MobiReadError(String),
	/// The file is protected: `crate::drm::detect_drm` produced a verdict, and
	/// the string is that verdict's user-facing reason. Processors return this
	/// instead of decoding ciphertext into garbage.
	#[error("{0}")]
	DrmProtected(String),
	#[error("Page {page} does not exist; the file has {available} pages")]
	PageNotFound { page: usize, available: usize },
	#[error("Resource {0} does not exist in the file")]
	ResourceNotFound(String),
	#[error("Could not find an image")]
	NoImageError,
	#[cfg(feature = "pdf")]
	#[error("{0}")]
	PdfRendererError(#[from] pdfium_render::prelude::PdfiumError),
	#[error("Coppice is not properly configured to render PDFs")]
	PdfConfigurationError,
	#[error("Failed to process PDF file: {0}")]
	PdfProcessingError(String),
	#[cfg(feature = "rar")]
	#[error("{0}")]
	RarError(#[from] unrar::error::UnrarError),
	#[cfg(feature = "rar")]
	#[error("Failed to open rar archive: {0}")]
	RarNulError(#[from] unrar::error::NulError),
	#[cfg(feature = "rar")]
	#[error("Could not open rar file")]
	RarOpenError,
	#[cfg(feature = "rar")]
	#[error("Error extracting RAR file: {0}")]
	RarExtractError(String),
	#[cfg(feature = "rar")]
	#[error("Error reading RAR file")]
	RarReadError,
	#[cfg(feature = "rar")]
	#[error("Error reading RAR byte content")]
	RarByteReadError(#[from] std::str::Utf8Error),
	#[cfg(feature = "rar")]
	#[error("RAR archive is empty")]
	RarEmpty,
	#[error("Unsupported file type: {0}")]
	UnsupportedFileType(String),
	#[error("{0}")]
	ImageIoError(#[from] image::ImageError),
	#[error("Failed to encode image to webp: {0}")]
	WebpEncodeError(String),
	#[error("Failed to read directory")]
	DirectoryReadError,
	#[error("Incorrect image processor for requested format")]
	IncorrectProcessorError,
	#[error("File not found on disk")]
	NotFound,
	/// The content exists but its bytes cannot be served, and never will be
	/// through this path: a provider chapter the remote source has stopped
	/// serving. The string is the user-facing reason, so callers map this to
	/// `404 Not Found` rather than an internal error.
	#[error("{0}")]
	Unavailable(String),
	#[error("An unknown error occurred: {0}")]
	UnknownError(String),
}
