//! DRM / content-encryption detection for staged files.
//!
//! Stump never removes DRM and never ships or invokes a DRM-removal tool. This
//! module only *identifies* protected files so ingest can reject them with a
//! precise, actionable reason instead of committing a book whose pages cannot
//! be decoded.
//!
//! Every rule below is derived from a published format description; the source
//! URL is cited on the rule it backs. No code is derived from any GPL project.
//!
//! Detection is deliberately container-sniffed rather than extension-driven: a
//! `.epub` that is really a MOBI, or a `.cbz` renamed from a PDF, is classified
//! by its bytes.

use std::{
	fs::File,
	io::{BufReader, Cursor, Read, Seek, SeekFrom},
	path::Path,
};

use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};

use crate::error::FileError;

/// `EncryptedData/EncryptionMethod@Algorithm` values that mean *font
/// obfuscation*, not DRM. An `encryption.xml` containing only these is a
/// perfectly readable EPUB and must never be reported as protected.
///
/// - `http://www.idpf.org/2008/embedding` is the EPUB font-obfuscation
///   algorithm ([EPUB 3.3 §4.4.5](https://www.w3.org/TR/epub-33/#sec-font-obfuscation)).
/// - `http://ns.adobe.com/pdf/enc#RC` is Adobe's older font-mangling scheme;
///   epubcheck classifies both as font mangling rather than encryption
///   ([`OCFEncryptionFileHandler.java:81-86`](https://github.com/w3c/epubcheck/blob/main/src/main/java/com/adobe/epubcheck/ocf/OCFEncryptionFileHandler.java)).
pub const FONT_OBFUSCATION_ALGORITHMS: [&str; 2] = [
	"http://www.idpf.org/2008/embedding",
	"http://ns.adobe.com/pdf/enc#RC",
];

/// Adobe's own `EncryptedData` algorithm URI. The generic
/// `http://www.w3.org/2001/04/xmlenc#aes128-cbc` is used by several vendors,
/// so only this Adobe-namespaced one attributes a scheme on its own
/// ([`ineptepub.py`](https://github.com/noDRM/DeDRM_tools/blob/v10.0.3/DeDRM_plugin/ineptepub.py)).
const ADEPT_CONTENT_ALGORITHM: &str =
	"http://ns.adobe.com/adept/xmlenc#aes128-cbc-uncompressed";

/// Shortest base64 ADEPT content key; RSA-1024 wrapping yields 172 and some
/// producers emit 192.
const ADEPT_KEY_MIN_LEN: usize = 172;
/// Base64 length of the Adobe PassHash (legacy Barnes & Noble) content key.
const BARNES_NOBLE_KEY_LEN: usize = 64;
/// `keyType` values above this mark Adobe's "hardened" DRM (RMSDK >= 10).
const ADEPT_HARDENED_KEY_TYPE: u32 = 2;

/// Magic of an encrypted Amazon KFX content blob, bare or as a `.kfx-zip`
/// member ([`kfxdedrm.py`](https://github.com/noDRM/DeDRM_tools/blob/v10.0.3/DeDRM_plugin/kfxdedrm.py)).
const DRMION_MAGIC: &[u8] = b"\xeaDRMION\xee";
/// Magic of the Topaz container, an Amazon store-only format
/// ([`k4mobidedrm.py`](https://github.com/noDRM/DeDRM_tools/blob/v10.0.3/DeDRM_plugin/k4mobidedrm.py)).
const TOPAZ_MAGIC: &[u8] = b"TPZ";

/// Bytes of a PDF scanned from the start; covers a linearized first-page
/// trailer.
const PDF_HEAD_WINDOW: usize = 64 * 1024;
/// Bytes of a PDF scanned from the end; covers the main trailer / xref stream.
const PDF_TAIL_WINDOW: usize = 1024 * 1024;
/// Upper bound on the PDB record we parse for MOBI headers.
const MOBI_RECORD_WINDOW: usize = 64 * 1024;
/// Upper bound on any single `META-INF` member we read into memory.
const META_INF_WINDOW: u64 = 1024 * 1024;

/// The container family a [`DrmReport`] was produced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DrmContainer {
	/// OCF ZIP container (EPUB, KEPUB).
	Epub,
	/// Palm database container (MOBI, AZW, PRC).
	Mobipocket,
	/// Amazon Topaz container (`.azw1`, `.tpz`).
	Topaz,
	/// Amazon KFX container, bare or ZIP-wrapped (`.kfx`, `.kfx-zip`).
	Kfx,
	Pdf,
}

impl DrmContainer {
	pub fn as_str(self) -> &'static str {
		match self {
			DrmContainer::Epub => "epub",
			DrmContainer::Mobipocket => "mobipocket",
			DrmContainer::Topaz => "topaz",
			DrmContainer::Kfx => "kfx",
			DrmContainer::Pdf => "pdf",
		}
	}
}

/// The identified protection scheme. Naming a scheme is diagnostic only: no
/// variant enables any decryption path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DrmScheme {
	/// Adobe ADEPT (Adobe Digital Editions / Adobe Content Server).
	AdobeAdept,
	/// Barnes & Noble, which reuses the ADEPT `rights.xml` layout with a
	/// shorter content key.
	BarnesNoble,
	/// Kobo, identified by a `<kdrm>` block in `rights.xml`.
	Kobo,
	/// Apple FairPlay, identified by `META-INF/sinf.xml`.
	AppleFairPlay,
	/// Readium LCP, identified by `META-INF/license.lcpl`.
	ReadiumLcp,
	/// `encryption.xml` encrypts container resources with an algorithm that is
	/// not font obfuscation, but no vendor marker identifies the scheme.
	EpubUnknownEncryption,
	/// PalmDOC encryption type 1: single-PID Mobipocket encryption.
	MobipocketLegacy,
	/// PalmDOC encryption type 2: multi-PID Mobipocket/Kindle encryption.
	Mobipocket,
	/// Amazon Topaz: the container itself was only ever distributed with DRM.
	Topaz,
	/// Amazon KFX with an encrypted DRMION content blob.
	AmazonKfx,
	/// PDF standard security handler (`/Filter /Standard`): user or owner
	/// password, or permissions-only encryption.
	PdfStandardSecurity,
	/// Adobe ADEPT / Adobe Content Server PDF handler (`/EBX_HANDLER`).
	PdfAdobeEbx,
	/// Adobe APS PDF handler (`/Adobe.APS`).
	PdfAdobeAps,
	/// A `/Encrypt` dictionary whose security handler could not be named.
	PdfUnknownHandler,
}

impl DrmScheme {
	pub fn label(self) -> &'static str {
		match self {
			DrmScheme::AdobeAdept => "Adobe ADEPT",
			DrmScheme::BarnesNoble => "Barnes & Noble",
			DrmScheme::Kobo => "Kobo",
			DrmScheme::AppleFairPlay => "Apple FairPlay",
			DrmScheme::ReadiumLcp => "Readium LCP",
			DrmScheme::EpubUnknownEncryption => "unidentified EPUB encryption",
			DrmScheme::MobipocketLegacy => "Mobipocket (legacy, single PID)",
			DrmScheme::Mobipocket => "Mobipocket/Kindle",
			DrmScheme::Topaz => "Amazon Topaz",
			DrmScheme::AmazonKfx => "Amazon KFX (DRMION)",
			DrmScheme::PdfStandardSecurity => "PDF standard security handler",
			DrmScheme::PdfAdobeEbx => "Adobe ADEPT PDF (EBX_HANDLER)",
			DrmScheme::PdfAdobeAps => "Adobe APS PDF",
			DrmScheme::PdfUnknownHandler => "unidentified PDF security handler",
		}
	}
}

/// One positive DRM finding: which container, which scheme, the exact markers
/// that triggered it, and a sentence a user can act on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrmReport {
	pub container: DrmContainer,
	pub scheme: DrmScheme,
	/// Container-relative marker names, header field names, or byte tokens
	/// that produced the verdict, in detection order.
	pub markers: Vec<String>,
	pub reason: String,
}

impl DrmReport {
	fn new(
		container: DrmContainer,
		scheme: DrmScheme,
		markers: Vec<String>,
		reason: impl Into<String>,
	) -> Self {
		Self {
			container,
			scheme,
			markers,
			reason: reason.into(),
		}
	}
}

/// Detect DRM in the file at `path`.
///
/// `Ok(None)` means "no protection found", which includes every container this
/// module does not classify (CBZ/CBR archives, plain images, unknown bytes).
/// Only I/O failures are errors: a malformed container is reported through the
/// verdict, never as a hard error, so one bad file cannot fail a whole scan.
pub fn detect_drm(path: &Path) -> Result<Option<DrmReport>, FileError> {
	let mut file = File::open(path)?;
	let mut header = [0u8; 68];
	let read = read_at_most(&mut file, &mut header)?;
	let header = &header[..read];

	if is_zip(header) {
		file.rewind()?;
		return detect_epub_drm(BufReader::new(file));
	}
	if is_pdf(header) {
		file.rewind()?;
		return detect_pdf_drm(file);
	}
	if is_palm_database(header) {
		file.rewind()?;
		return detect_mobi_drm(file);
	}
	if header.starts_with(DRMION_MAGIC) {
		return Ok(Some(DrmReport::new(
			DrmContainer::Kfx,
			DrmScheme::AmazonKfx,
			vec!["magic:DRMION".into()],
			"The file is an encrypted Amazon KFX content blob; its voucher \
			 lives outside the file, so it can never be read on its own.",
		)));
	}
	if header.starts_with(TOPAZ_MAGIC) {
		return Ok(Some(DrmReport::new(
			DrmContainer::Topaz,
			DrmScheme::Topaz,
			vec!["magic:TPZ".into()],
			"The file is an Amazon Topaz container, a store-only format whose \
			 records are encrypted to a Kindle account.",
		)));
	}
	Ok(None)
}

fn read_at_most<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<usize, FileError> {
	let mut filled = 0;
	while filled < buffer.len() {
		match reader.read(&mut buffer[filled..])? {
			0 => break,
			n => filled += n,
		}
	}
	Ok(filled)
}

fn is_zip(header: &[u8]) -> bool {
	// Local file header, central directory, or spanning marker signature.
	header.starts_with(b"PK\x03\x04")
		|| header.starts_with(b"PK\x05\x06")
		|| header.starts_with(b"PK\x07\x08")
}

fn is_pdf(header: &[u8]) -> bool {
	header.starts_with(b"%PDF-")
}

/// A Palm database stores `type` at offset 60 and `creator` at offset 64
/// ([MobileRead PDB](https://wiki.mobileread.com/wiki/PDB#Palm_Database_Format)).
/// MOBI/AZW files are `BOOKMOBI`; PalmDOC is `TEXtREAd`.
fn is_palm_database(header: &[u8]) -> bool {
	match header.get(60..68) {
		Some(id) => id == b"BOOKMOBI".as_slice() || id == b"TEXtREAd".as_slice(),
		None => false,
	}
}

// ---------------------------------------------------------------------------
// EPUB / OCF ZIP
// ---------------------------------------------------------------------------

/// Classify an OCF ZIP container.
///
/// The vendor markers mirror the behaviour of the public-domain
/// [`epubtest.py`](https://github.com/noDRM/DeDRM_tools/blob/v10.0.3/DeDRM_plugin/epubtest.py)
/// (released under the Unlicense), reimplemented here from its documented
/// rules; the font-obfuscation exemption follows
/// [EPUB 3.3 §4.2.6.3.2](https://www.w3.org/TR/epub-33/#sec-container-metainf-encryption.xml).
pub fn detect_epub_drm<R: Read + Seek>(
	reader: R,
) -> Result<Option<DrmReport>, FileError> {
	let mut archive = match zip::ZipArchive::new(reader) {
		Ok(archive) => archive,
		// Not a readable ZIP: nothing to classify, and the container checks
		// will report the real problem.
		Err(zip::result::ZipError::InvalidArchive(_))
		| Err(zip::result::ZipError::UnsupportedArchive(_)) => return Ok(None),
		Err(error) => return Err(error.into()),
	};

	let encryption = read_member(&mut archive, "META-INF/encryption.xml")?;
	let rights = read_member(&mut archive, "META-INF/rights.xml")?;
	let sinf = read_member(&mut archive, "META-INF/sinf.xml")?;
	let has_lcpl = read_member(&mut archive, "META-INF/license.lcpl")?.is_some();

	if let Some(encryption) = encryption.as_deref() {
		if has_lcpl && contains_ascii(encryption, b"EncryptedContentKey") {
			return Ok(Some(DrmReport::new(
				DrmContainer::Epub,
				DrmScheme::ReadiumLcp,
				vec![
					"META-INF/license.lcpl".into(),
					"META-INF/encryption.xml:EncryptedContentKey".into(),
				],
				"The EPUB carries a Readium LCP licence and an encrypted \
				 content key, so its resources cannot be read.",
			)));
		}
	}

	if let Some(sinf) = sinf.as_deref() {
		if contains_ascii(sinf, b"fairplay") {
			return Ok(Some(DrmReport::new(
				DrmContainer::Epub,
				DrmScheme::AppleFairPlay,
				vec!["META-INF/sinf.xml:fairplay".into()],
				"The EPUB is an Apple FairPlay purchase; its resources are \
				 encrypted to an Apple account.",
			)));
		}
	}

	if let Some(rights) = rights.as_deref() {
		if contains_ascii(rights, b"<kdrm>") {
			return Ok(Some(DrmReport::new(
				DrmContainer::Epub,
				DrmScheme::Kobo,
				vec!["META-INF/rights.xml:kdrm".into()],
				"The EPUB carries a Kobo <kdrm> rights block; its resources \
				 are encrypted to a Kobo account.",
			)));
		}
		if contains_ascii(rights, b"ns.adobe.com/adept") {
			if let Some(key) = adept_encrypted_key(rights) {
				let (scheme, note) = if key.value.len() >= ADEPT_KEY_MIN_LEN {
					(DrmScheme::AdobeAdept, "an Adobe ADEPT")
				} else if key.value.len() == BARNES_NOBLE_KEY_LEN {
					(DrmScheme::BarnesNoble, "an Adobe PassHash (Barnes & Noble)")
				} else {
					(
						DrmScheme::EpubUnknownEncryption,
						"an unrecognised ADEPT-style",
					)
				};
				let mut markers = vec![format!(
					"META-INF/rights.xml:encryptedKey[{}]",
					key.value.len()
				)];
				let hardened = scheme == DrmScheme::AdobeAdept
					&& key.key_type > ADEPT_HARDENED_KEY_TYPE;
				if hardened {
					markers.push(format!("META-INF/rights.xml:keyType={}", key.key_type));
				}
				return Ok(Some(DrmReport::new(
					DrmContainer::Epub,
					scheme,
					markers,
					format!(
						"The EPUB carries {note} content key{}, so its \
						 resources are encrypted.",
						if hardened { " (hardened)" } else { "" }
					),
				)));
			}
		}
	}

	if let Some(encryption) = encryption.as_deref() {
		match content_encryption_algorithms(encryption) {
			Ok(algorithms) if !algorithms.is_empty() => {
				let markers = algorithms
					.iter()
					.map(|algorithm| format!("META-INF/encryption.xml:{algorithm}"))
					.collect();
				let adobe = algorithms
					.iter()
					.any(|algorithm| algorithm == ADEPT_CONTENT_ALGORITHM);
				let scheme = if adobe {
					DrmScheme::AdobeAdept
				} else {
					DrmScheme::EpubUnknownEncryption
				};
				return Ok(Some(DrmReport::new(
					DrmContainer::Epub,
					scheme,
					markers,
					format!(
						"META-INF/encryption.xml encrypts container \
						 resources with {}, which is not EPUB font \
						 obfuscation, so the content cannot be read.",
						algorithms.join(", ")
					),
				)));
			},
			Ok(_) => {},
			Err(error) => {
				return Ok(Some(DrmReport::new(
					DrmContainer::Epub,
					DrmScheme::EpubUnknownEncryption,
					vec!["META-INF/encryption.xml:unparseable".into()],
					format!(
						"META-INF/encryption.xml declares encrypted \
						 resources but could not be parsed ({error}), so the \
						 container cannot be shown to be readable."
					),
				)));
			},
		}
	}

	if let Some(member) = kfx_zip_member(&mut archive)? {
		return Ok(Some(DrmReport::new(
			DrmContainer::Kfx,
			DrmScheme::AmazonKfx,
			vec![format!("{member}:DRMION")],
			"The ZIP is an Amazon KFX package holding an encrypted DRMION \
			 content blob, so its pages cannot be decoded.",
		)));
	}

	Ok(None)
}

fn read_member<R: Read + Seek>(
	archive: &mut zip::ZipArchive<R>,
	name: &str,
) -> Result<Option<Vec<u8>>, FileError> {
	let mut entry = match archive.by_name(name) {
		Ok(entry) => entry,
		Err(zip::result::ZipError::FileNotFound) => return Ok(None),
		Err(error) => return Err(error.into()),
	};
	let mut bytes = Vec::new();
	entry
		.by_ref()
		.take(META_INF_WINDOW)
		.read_to_end(&mut bytes)?;
	Ok(Some(bytes))
}

fn contains_ascii(haystack: &[u8], needle: &[u8]) -> bool {
	if needle.is_empty() || haystack.len() < needle.len() {
		return false;
	}
	haystack
		.windows(needle.len())
		.any(|window| window.eq_ignore_ascii_case(needle))
}

/// The `<adept:encryptedKey>` element's base64 text and its `keyType`.
struct AdeptKey {
	value: String,
	/// `keyType` attribute, base-10; `0` when absent. Values above
	/// [`ADEPT_HARDENED_KEY_TYPE`] mark Adobe's hardened DRM
	/// ([`ineptepub.py`](https://github.com/noDRM/DeDRM_tools/blob/v10.0.3/DeDRM_plugin/ineptepub.py)).
	key_type: u32,
}

fn adept_encrypted_key(rights: &[u8]) -> Option<AdeptKey> {
	let mut reader = Reader::from_reader(Cursor::new(rights));
	let mut buffer = Vec::new();
	let mut key_type = None;
	loop {
		match reader.read_event_into(&mut buffer) {
			Ok(Event::Start(event)) => {
				key_type = (event.local_name().as_ref() == b"encryptedKey").then(|| {
					event
						.try_get_attribute("keyType")
						.ok()
						.flatten()
						.and_then(|attribute| attribute.unescape_value().ok())
						.and_then(|value| value.trim().parse::<u32>().ok())
						.unwrap_or(0)
				});
			},
			Ok(Event::Text(text)) => {
				if let Some(key_type) = key_type {
					// Base64 content: no XML entities to unescape.
					let value = text.decode().ok()?;
					let value = value
						.chars()
						.filter(|character| !character.is_whitespace())
						.collect::<String>();
					if !value.is_empty() {
						return Some(AdeptKey { value, key_type });
					}
				}
			},
			Ok(Event::End(_)) => key_type = None,
			Ok(Event::Eof) | Err(_) => return None,
			_ => {},
		}
		buffer.clear();
	}
}

/// Name of the first `.kfx` archive member whose bytes start with the DRMION
/// magic.
///
/// A `.kfx-zip` / `.azw8` package stores its encrypted content in `.kfx`
/// members, so only those are opened and only their first eight bytes are read
/// ([`kfxdedrm.py`](https://github.com/noDRM/DeDRM_tools/blob/v10.0.3/DeDRM_plugin/kfxdedrm.py)).
/// Restricting the scan by name keeps a 500-page CBZ from being partially
/// decompressed on every ingest.
fn kfx_zip_member<R: Read + Seek>(
	archive: &mut zip::ZipArchive<R>,
) -> Result<Option<String>, FileError> {
	let candidates = archive
		.file_names()
		.filter(|name| {
			std::path::Path::new(name)
				.extension()
				.is_some_and(|extension| extension.eq_ignore_ascii_case("kfx"))
		})
		.map(str::to_string)
		.collect::<Vec<_>>();
	for name in candidates {
		let mut entry = archive.by_name(&name)?;
		let mut magic = [0u8; 8];
		if read_at_most(&mut entry, &mut magic)? == magic.len() && magic == DRMION_MAGIC {
			return Ok(Some(name));
		}
	}
	Ok(None)
}

/// `Algorithm` values of every `EncryptedData` in `encryption.xml` that is not
/// font obfuscation, deduplicated in document order.
///
/// `EncryptedKey` also carries an `EncryptionMethod`; only the algorithms of
/// `EncryptedData` elements describe an encrypted container resource, so the
/// key-wrapping algorithm is deliberately ignored.
fn content_encryption_algorithms(
	encryption: &[u8],
) -> Result<Vec<String>, quick_xml::Error> {
	let mut reader = Reader::from_reader(Cursor::new(encryption));
	let mut buffer = Vec::new();
	let mut depth = 0usize;
	let mut data_depth: Option<usize> = None;
	let mut algorithms = Vec::new();
	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(event) => {
				match event.local_name().as_ref() {
					b"EncryptedData" if data_depth.is_none() => {
						data_depth = Some(depth);
					},
					b"EncryptionMethod" if data_depth.is_some() => {
						push_algorithm(&event, &mut algorithms)?;
					},
					_ => {},
				}
				depth += 1;
			},
			Event::Empty(event) => {
				if data_depth.is_some()
					&& event.local_name().as_ref() == b"EncryptionMethod"
				{
					push_algorithm(&event, &mut algorithms)?;
				}
			},
			Event::End(_) => {
				depth = depth.saturating_sub(1);
				if data_depth == Some(depth) {
					data_depth = None;
				}
			},
			Event::Eof => break,
			_ => {},
		}
		buffer.clear();
	}
	Ok(algorithms)
}

/// Record an `EncryptionMethod@Algorithm` value unless it is font obfuscation
/// or already recorded.
fn push_algorithm(
	event: &quick_xml::events::BytesStart<'_>,
	algorithms: &mut Vec<String>,
) -> Result<(), quick_xml::Error> {
	let Some(attribute) = event.try_get_attribute("Algorithm")? else {
		return Ok(());
	};
	let value = attribute.unescape_value()?.trim().to_string();
	let obfuscation = FONT_OBFUSCATION_ALGORITHMS
		.iter()
		.any(|known| known.eq_ignore_ascii_case(value.as_str()));
	if !obfuscation && !value.is_empty() && !algorithms.contains(&value) {
		algorithms.push(value);
	}
	Ok(())
}

// ---------------------------------------------------------------------------
// MOBI / AZW (Palm database)
// ---------------------------------------------------------------------------

/// Classify a Palm-database e-book (MOBI, AZW, PRC).
///
/// The authoritative signal is the PalmDOC header's encryption type at
/// record-0 offset 12: `0` = none, `1` = old (single-PID) Mobipocket
/// encryption, `2` = current Mobipocket encryption
/// ([MobileRead MOBI](https://wiki.mobileread.com/wiki/MOBI#PalmDOC_Header)).
/// A non-zero value means the text records themselves are ciphertext.
///
/// The MOBI header's DRM key block (`DRM Offset` 0xA8 / `DRM Count` 0xAC,
/// `0xFFFFFFFF` when absent) is treated as a second positive signal.
///
/// EXTH `209` (tamper-proof keys) and `1`/`2`/`3` (`drm_server_id`,
/// `drm_commerce_id`, `drm_ebookbase_book_id`) are recorded as markers but
/// never flip the verdict on their own: those records also survive in
/// DRM-free Amazon files, while encryption type `0` proves the text is
/// plaintext.
pub fn detect_mobi_drm<R: Read + Seek>(
	mut reader: R,
) -> Result<Option<DrmReport>, FileError> {
	let mut pdb = [0u8; 82];
	let read = read_at_most(&mut reader, &mut pdb)?;
	if read < pdb.len() || !is_palm_database(&pdb) {
		return Ok(None);
	}
	if be_u16(&pdb, 76) == 0 {
		return Ok(None);
	}
	let record0_offset = u64::from(be_u32(&pdb, 78));

	reader.seek(SeekFrom::Start(record0_offset))?;
	let mut record0 = vec![0u8; MOBI_RECORD_WINDOW];
	let read = read_at_most(&mut reader, &mut record0)?;
	record0.truncate(read);
	if record0.len() < 16 {
		return Ok(None);
	}

	let mut markers = Vec::new();
	let encryption_type = be_u16(&record0, 12);
	let exth = mobi_exth_drm_records(&record0);
	for record_type in &exth {
		markers.push(format!("EXTH:{record_type}"));
	}

	let drm_key_block = mobi_drm_key_block(&record0);
	if let Some((offset, count)) = drm_key_block {
		markers.push(format!("MOBI:drm_offset={offset:#x},drm_count={count}"));
	}

	if encryption_type == 0 && drm_key_block.is_none() {
		return Ok(None);
	}

	let scheme = match encryption_type {
		1 => DrmScheme::MobipocketLegacy,
		_ => DrmScheme::Mobipocket,
	};
	markers.insert(0, format!("PalmDOC:encryption_type={encryption_type}"));
	let reason = if encryption_type == 0 {
		"The MOBI/AZW header declares a DRM key block, so the file is tied to \
		 a reader account."
			.to_string()
	} else {
		format!(
			"The MOBI/AZW PalmDOC header declares encryption type \
			 {encryption_type} ({}), so its text records are ciphertext.",
			scheme.label()
		)
	};
	Ok(Some(DrmReport::new(
		DrmContainer::Mobipocket,
		scheme,
		markers,
		reason,
	)))
}

/// `(drm_offset, drm_count)` when the MOBI header declares a DRM key block.
fn mobi_drm_key_block(record0: &[u8]) -> Option<(u32, u32)> {
	if record0.get(16..20)? != b"MOBI".as_slice() {
		return None;
	}
	// `header length` includes the four identifier bytes at offset 16, so the
	// DRM words at 0xA8..0xB8 only exist when the header reaches record-0
	// offset 184. Short and `TEXtREAd` headers have no such fields and
	// reading them would yield garbage.
	let header_len = be_u32(record0, 20) as usize;
	if 16 + header_len < 184 || record0.len() < 176 {
		return None;
	}
	let offset = be_u32(record0, 168);
	let count = be_u32(record0, 172);
	if offset == u32::MAX || count == u32::MAX || count == 0 {
		return None;
	}
	Some((offset, count))
}

/// DRM-related EXTH record types present with non-empty payloads.
fn mobi_exth_drm_records(record0: &[u8]) -> Vec<u32> {
	const DRM_RECORD_TYPES: [u32; 4] = [1, 2, 3, 209];

	if record0.get(16..20) != Some(b"MOBI".as_slice()) {
		return Vec::new();
	}
	let header_len = be_u32(record0, 20) as usize;
	if record0.len() < 132 || be_u32(record0, 128) & 0x40 == 0 {
		return Vec::new();
	}
	let Some(start) = 16usize.checked_add(header_len) else {
		return Vec::new();
	};
	if record0.len() < start + 12
		|| record0.get(start..start + 4) != Some(b"EXTH".as_slice())
	{
		return Vec::new();
	}
	let exth_len = be_u32(record0, start + 4) as usize;
	let end = record0.len().min(start + exth_len.max(12));
	let count = be_u32(record0, start + 8) as usize;

	let mut found = Vec::new();
	let mut cursor = start + 12;
	for _ in 0..count {
		if cursor + 8 > end {
			break;
		}
		let record_type = be_u32(record0, cursor);
		let record_len = be_u32(record0, cursor + 4) as usize;
		if record_len < 8 || cursor + record_len > end {
			break;
		}
		if DRM_RECORD_TYPES.contains(&record_type)
			&& record_len > 8
			&& !found.contains(&record_type)
		{
			found.push(record_type);
		}
		cursor += record_len;
	}
	found
}

fn be_u16(bytes: &[u8], offset: usize) -> u16 {
	match bytes.get(offset..offset + 2) {
		Some(slice) => u16::from_be_bytes([slice[0], slice[1]]),
		None => 0,
	}
}

fn be_u32(bytes: &[u8], offset: usize) -> u32 {
	match bytes.get(offset..offset + 4) {
		Some(slice) => u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]),
		None => 0,
	}
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

/// Classify a PDF by its trailer / cross-reference-stream `/Encrypt` entry.
///
/// `/Encrypt` is required in the trailer dictionary of every encrypted PDF
/// ([ISO 32000-1 §7.5.5, table 15](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf)),
/// so its presence is the detection rule. The handler is named from the
/// encryption dictionary's `/Filter`: `/Standard` is the built-in security
/// handler (§7.6.3), while `/EBX_HANDLER` and `/Adobe.APS` are Adobe DRM
/// handlers ([`ineptpdf.py:1247-1256`](https://github.com/noDRM/DeDRM_tools/blob/v10.0.3/DeDRM_plugin/ineptpdf.py)).
///
/// Permissions-only encryption (an empty user password) also trips this rule:
/// the ingest pipeline has no password to supply and cannot prove the page
/// content is decodable.
pub fn detect_pdf_drm<R: Read + Seek>(
	mut reader: R,
) -> Result<Option<DrmReport>, FileError> {
	let len = reader.seek(SeekFrom::End(0))?;
	let mut windows = Vec::with_capacity(2);

	let tail_len = len.min(PDF_TAIL_WINDOW as u64);
	reader.seek(SeekFrom::Start(len - tail_len))?;
	let mut tail = vec![0u8; tail_len as usize];
	let read = read_at_most(&mut reader, &mut tail)?;
	tail.truncate(read);
	windows.push(tail);

	if len > PDF_TAIL_WINDOW as u64 {
		reader.rewind()?;
		let mut head = vec![0u8; PDF_HEAD_WINDOW];
		let read = read_at_most(&mut reader, &mut head)?;
		head.truncate(read);
		windows.push(head);
	}

	for window in &windows {
		if !has_encrypt_entry(window) {
			continue;
		}
		let (scheme, marker) = pdf_handler(window);
		let mut markers = vec!["trailer:/Encrypt".to_string()];
		if let Some(marker) = marker {
			markers.push(format!("encrypt_dict:/Filter/{marker}"));
		}
		let reason = format!(
			"The PDF declares an /Encrypt dictionary ({}), so page content \
			 cannot be decoded without the document's credentials.",
			scheme.label()
		);
		return Ok(Some(DrmReport::new(
			DrmContainer::Pdf,
			scheme,
			markers,
			reason,
		)));
	}
	Ok(None)
}

/// `/Encrypt` followed by an indirect reference or an inline dictionary.
///
/// The value shape is validated so the literal text `/Encrypt` inside a
/// content stream or an annotation cannot produce a false positive.
fn has_encrypt_entry(window: &[u8]) -> bool {
	const KEY: &[u8] = b"/Encrypt";
	let mut from = 0;
	while let Some(found) = find(&window[from..], KEY) {
		let after = from + found + KEY.len();
		let rest = &window[after..];
		let value = rest
			.iter()
			.position(|byte| !byte.is_ascii_whitespace())
			.map(|skip| &rest[skip..]);
		match value {
			Some(value)
				if value.starts_with(b"<<")
					|| value.first().is_some_and(u8::is_ascii_digit) =>
			{
				return true
			},
			_ => {},
		}
		from = after;
	}
	false
}

fn pdf_handler(window: &[u8]) -> (DrmScheme, Option<&'static str>) {
	for (needle, scheme, label) in [
		(&b"/EBX_HANDLER"[..], DrmScheme::PdfAdobeEbx, "EBX_HANDLER"),
		// The ADEPT licence blob lives in the same encryption dictionary and
		// is the strongest EBX signal when /Filter sits in another window.
		(
			&b"/ADEPT_LICENSE"[..],
			DrmScheme::PdfAdobeEbx,
			"ADEPT_LICENSE",
		),
		(&b"/Adobe.APS"[..], DrmScheme::PdfAdobeAps, "Adobe.APS"),
		(
			&b"/Standard"[..],
			DrmScheme::PdfStandardSecurity,
			"Standard",
		),
	] {
		if find(window, needle).is_some() {
			return (scheme, Some(label));
		}
	}
	(DrmScheme::PdfUnknownHandler, None)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	if needle.is_empty() || haystack.len() < needle.len() {
		return None;
	}
	haystack
		.windows(needle.len())
		.position(|window| window == needle)
}

#[cfg(test)]
mod tests;
