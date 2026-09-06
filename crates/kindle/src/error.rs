use sea_orm::DbErr;
use stump_devices::DeviceError;

pub type KindleResult<T> = Result<T, KindleError>;

/// Everything a send-to-Kindle lane refuses, in the operator's words.
///
/// Every variant is a sentence an operator can act on: the refusals name the
/// device, the extension, or the limit that stopped the delivery, because the
/// alternative is Amazon's gateway dropping the attachment minutes later with
/// no explanation at all.
#[derive(Debug, thiserror::Error)]
pub enum KindleError {
	#[error(transparent)]
	Device(#[from] DeviceError),
	#[error("Book not found")]
	BookNotFound,
	#[error("{0} has been revoked")]
	Revoked(String),
	#[error("{0} has no Kindle address; set one before sending to it")]
	NoAddress(String),
	#[error("a Kindle cannot read .{extension}; Amazon accepts {accepted}")]
	UnsupportedFormat { extension: String, accepted: String },
	/// A Kindle mounted over USB has no gateway behind it: the file has to be
	/// a Kindle format before it is copied.
	#[error(
		"a Kindle opens no .{0} over USB; install boko so Stump can convert it \
		 (or mail the book instead, which lets Amazon convert it)"
	)]
	ConversionRequired(String),
	#[error("{0}")]
	TooLarge(String),
	#[error("{0}")]
	Attachment(String),
	/// The transport refused the message. Recorded on the delivery row.
	#[error("{0}")]
	Refused(String),
	#[error("failed to read the book: {0}")]
	Io(#[from] std::io::Error),
	#[error(transparent)]
	Database(#[from] DbErr),
}
