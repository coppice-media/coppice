//! The file a Kindle gets: which formats are allowed, when the book is
//! converted, and what it is called.

use std::path::Path;

use stump_media::{ContentType, FileParts, PathUtils};

use crate::{
	convert::{Conversion, KindleConverter},
	error::{KindleError, KindleResult},
};

/// The attachment formats Amazon's Send to Kindle service accepts (its own
/// list: DOC, DOCX, HTML, HTM, RTF, TXT, PDF, EPUB) plus the Kindle's native
/// AZW3/MOBI, which need no conversion at all.
///
/// Comic archives are absent on purpose: a Kindle cannot open a CBZ/CBR and
/// Amazon discards it, so mailing one would look like a successful send.
pub const KINDLE_FORMATS: &[&str] = &[
	"azw3", "doc", "docx", "epub", "htm", "html", "mobi", "pdf", "rtf", "txt",
];

/// The one source format worth converting: everything else in
/// [`KINDLE_FORMATS`] is either already a Kindle format or something boko
/// cannot read.
const CONVERTIBLE_FORMAT: &str = "epub";

/// The extension the conversion produces.
pub const KINDLE_FORMAT: &str = "azw3";

/// Amazon's documented Send to Kindle ceiling: 50 MB per document, counted on
/// the attachment itself
/// (<https://www.amazon.com/sendtokindle/email>). A larger book is refused
/// here, before an SMTP connection is opened, because Amazon's gateway
/// discards the message silently and the operator would see a successful send.
pub const AMAZON_MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

/// The bytes that decide a MIME type: [`ContentType::from_bytes_with_fallback`]
/// sniffs the container's magic, and nothing shorter than this is a book.
const MIN_BOOK_BYTES: usize = 5;

/// How hard the lane insists on a Kindle format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatPolicy {
	/// The e-mail lane. Amazon's gateway converts an EPUB itself, so a
	/// conversion that cannot happen is a note on a successful delivery.
	PreferKindle,
	/// The USB lane. A Kindle mounted as a disk has no gateway: a file it
	/// cannot open is worse than no file, so an unconvertible EPUB is
	/// refused.
	RequireKindle,
}

/// Exactly what gets attached to the mail or written to the mounted Kindle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindleFile {
	/// `<book file stem>.<format>`, i.e. the name the Kindle shows.
	pub filename: String,
	/// Extension of the file: `azw3` when it was converted, otherwise the
	/// book's own.
	pub format: String,
	pub content: Vec<u8>,
	/// MIME type sniffed from the bytes, with the extension breaking the tie
	/// between the Kindle formats.
	pub content_type: String,
	pub converted: bool,
	/// Why the book was *not* converted. `None` when it was.
	pub note: Option<String>,
}

impl KindleFile {
	pub fn bytes(&self) -> u64 {
		self.content.len() as u64
	}
}

/// Reads `source`, converting an EPUB to AZW3 when the operator's boko can,
/// and returns the file the caller should hand on.
///
/// The conversion output is written into `work_dir`, which the caller owns and
/// removes: the library file is never touched.
pub async fn prepare_file<C>(
	source: &Path,
	policy: FormatPolicy,
	converter: &C,
	work_dir: &Path,
) -> KindleResult<KindleFile>
where
	C: KindleConverter,
{
	let FileParts {
		file_stem,
		extension,
		..
	} = source.file_parts();
	let extension = extension.to_lowercase();
	if !KINDLE_FORMATS.contains(&extension.as_str()) {
		return Err(KindleError::UnsupportedFormat {
			extension,
			accepted: KINDLE_FORMATS.join(", "),
		});
	}

	let conversion = if extension == CONVERTIBLE_FORMAT {
		converter.to_azw3(source, work_dir).await
	} else {
		Conversion::Skipped(format!(".{extension} needs no conversion"))
	};

	let (path, format, note) = match &conversion {
		Conversion::Converted(path) => (path.as_path(), KINDLE_FORMAT, None),
		Conversion::Skipped(reason) => {
			// The USB lane cannot fall back on Amazon: refuse rather than
			// hand out a file the device will not open.
			if policy == FormatPolicy::RequireKindle && extension == CONVERTIBLE_FORMAT {
				tracing::debug!(%reason, "Refusing the USB download: no conversion");
				return Err(KindleError::ConversionRequired(extension));
			}
			tracing::debug!(%reason, "Using the book unconverted");
			(source, extension.as_str(), Some(reason.clone()))
		},
	};

	let content = tokio::fs::read(path).await?;
	let filename = format!("{file_stem}.{format}");
	if content.len() < MIN_BOOK_BYTES {
		return Err(KindleError::Attachment(format!(
			"{filename} is too small to be a book"
		)));
	}

	// The bytes decide, except that the extension wins when it names a
	// subtype of what the bytes say: `infer` reports every `BOOKMOBI` Palm
	// database as MOBI, and only `.azw3` distinguishes KF8.
	let content_type =
		ContentType::from_bytes_with_fallback(&content[..MIN_BOOK_BYTES], format)
			.mime_type()
			.to_string();

	Ok(KindleFile {
		filename,
		format: format.to_string(),
		content,
		content_type,
		converted: matches!(conversion, Conversion::Converted(_)),
		note,
	})
}
