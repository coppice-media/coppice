//! Reader for the Kindle/Mobipocket Palm database formats: `.mobi`, `.prc`,
//! `.azw` (MOBI 6, "KF7") and `.azw3` (Kindle Format 8).
//!
//! Clean-room: every structure below is implemented from published format
//! descriptions, cited on the item it backs. No code is derived from calibre,
//! KindleUnpack, DeDRM, boko or any other GPL/LGPL project, and nothing here
//! decrypts anything — a protected file is refused through
//! [`crate::drm::detect_mobi_drm`]'s own verdict.
//!
//! Sources:
//! - Palm database envelope (name, type/creator, record info list):
//!   [MobileRead PDB](https://wiki.mobileread.com/wiki/PDB#Palm_Database_Format).
//! - PalmDOC header, MOBI header field table, EXTH record types, trailing
//!   entries, variable-width integers, INDX/TAGX:
//!   [MobileRead MOBI](https://wiki.mobileread.com/wiki/MOBI).
//! - PalmDOC LZ77 + byte-pair decode rules:
//!   [MobileRead PalmDOC](https://wiki.mobileread.com/wiki/PalmDOC#PalmDoc_byte_pair_compression).
//! - KF8 being a compiled EPUB carried in the same Palm database:
//!   [MobileRead KF8](https://wiki.mobileread.com/wiki/KF8).
//! - HUFF/CDIC table layout and the KF8 skeleton/fragment index semantics: the
//!   MOBI format notes of [Kindling](https://github.com/ciscoriordan/kindling)
//!   (MIT, linked from the MobileRead MOBI page as further format
//!   documentation) plus the record bytes themselves; see
//!   `crates/media/README.md` for the derivation.

use std::{
	collections::HashMap,
	fs::File,
	io::{Read, Seek, SeekFrom},
	path::Path,
};

use crate::{
	content_type::ContentType,
	drm::detect_mobi_drm,
	error::FileError,
	hash::{self, generate_koreader_hash},
	media::{
		process::{AnalyzedPage, FileProcessor, FileProcessorOptions, ProcessedFile},
		ProcessedFileHashes, ProcessedMediaMetadata,
	},
	MediaConfig,
};

/// The file extensions this processor claims. `.prc` and `.pdb` are the only
/// extensions PalmOS ever allowed, so a MOBI may legitimately wear either
/// ([MobileRead MOBI](https://wiki.mobileread.com/wiki/MOBI#Overview)).
pub const KINDLE_EXTENSIONS: [&str; 4] = ["mobi", "prc", "azw", "azw3"];

// ---------------------------------------------------------------------------
// Palm database envelope
// ---------------------------------------------------------------------------

/// Fixed part of the PDB header, before the record info list
/// ([PDB](https://wiki.mobileread.com/wiki/PDB#Palm_Database_Format)).
const PDB_HEADER_LEN: usize = 78;
/// One record info entry: 4-byte data offset, 1 attribute byte, 3-byte id.
const PDB_RECORD_ENTRY_LEN: usize = 8;
/// `type` + `creator` of a Mobipocket database.
const BOOKMOBI: &[u8] = b"BOOKMOBI";
/// `type` + `creator` of a plain PalmDOC database, which MOBI is a superset of.
const TEXTREAD: &[u8] = b"TEXtREAd";

/// Upper bound on the records a single database may declare. A 16-bit count
/// cannot exceed this, so it only guards a corrupt header.
const MAX_RECORDS: usize = u16::MAX as usize;
/// Upper bound on the decompressed text of one part. Amazon's own limit is far
/// below this; it exists so a corrupt `text length` cannot ask for a
/// terabyte-sized allocation.
const MAX_TEXT_BYTES: usize = 512 * 1024 * 1024;

/// The record table of a Palm database plus the open file it came from.
///
/// Records are read on demand: the text records and the indices are needed to
/// open a book, but the image records (which are most of its bytes) are only
/// read when a caller asks for one.
struct PalmDatabase {
	file: File,
	/// `(offset, length)` of every record, in record order.
	bounds: Vec<(u64, u64)>,
	/// The database name, NUL-trimmed. For an e-book this is usually the
	/// title, sometimes with part of the author.
	name: String,
}

impl PalmDatabase {
	fn open(path: &Path) -> Result<Self, FileError> {
		let mut file = File::open(path)?;
		let total = file.seek(SeekFrom::End(0))?;
		file.rewind()?;

		let mut header = [0u8; PDB_HEADER_LEN];
		file.read_exact(&mut header).map_err(|_| {
			FileError::MobiReadError("file is shorter than a PDB header".to_string())
		})?;
		let kind = &header[60..68];
		if kind != BOOKMOBI && kind != TEXTREAD {
			return Err(FileError::MobiReadError(format!(
				"not a Mobipocket database: type/creator is {:?}",
				String::from_utf8_lossy(kind)
			)));
		}

		let count = usize::from(be_u16(&header, 76).unwrap_or(0));
		if count == 0 || count > MAX_RECORDS {
			return Err(FileError::MobiReadError(format!(
				"database declares {count} records"
			)));
		}

		let mut list = vec![0u8; count * PDB_RECORD_ENTRY_LEN];
		file.read_exact(&mut list).map_err(|_| {
			FileError::MobiReadError("record info list is truncated".to_string())
		})?;
		let mut offsets = Vec::with_capacity(count);
		for index in 0..count {
			let offset = be_u32(&list, index * PDB_RECORD_ENTRY_LEN)
				.map(u64::from)
				.unwrap_or(0);
			offsets.push(offset.min(total));
		}
		// A record runs to the next record's offset; the last one to the end of
		// the file. Offsets are not required to be sorted, but every producer
		// writes them in order and a decreasing pair would only yield an empty
		// record, never an out-of-bounds read.
		let bounds = offsets
			.iter()
			.enumerate()
			.map(|(index, &start)| {
				let end = offsets.get(index + 1).copied().unwrap_or(total).min(total);
				(start, end.saturating_sub(start))
			})
			.collect::<Vec<_>>();

		let name_end = header[..32]
			.iter()
			.position(|byte| *byte == 0)
			.unwrap_or(32);
		let name = String::from_utf8_lossy(&header[..name_end]).into_owned();

		Ok(Self { file, bounds, name })
	}

	fn len(&self) -> usize {
		self.bounds.len()
	}

	/// The bytes of record `index`, or an empty vector when the index is past
	/// the table. A record that a header points at but that does not exist is
	/// a producer bug, not a reason to fail the whole book.
	fn record(&mut self, index: usize) -> Result<Vec<u8>, FileError> {
		let Some(&(offset, length)) = self.bounds.get(index) else {
			return Ok(Vec::new());
		};
		if length == 0 {
			return Ok(Vec::new());
		}
		self.file.seek(SeekFrom::Start(offset))?;
		let mut bytes = vec![0u8; length as usize];
		let mut read = 0;
		while read < bytes.len() {
			match self.file.read(&mut bytes[read..])? {
				0 => break,
				n => read += n,
			}
		}
		bytes.truncate(read);
		Ok(bytes)
	}
}

fn be_u16(bytes: &[u8], offset: usize) -> Option<u16> {
	let slice = bytes.get(offset..offset + 2)?;
	Some(u16::from_be_bytes([slice[0], slice[1]]))
}

fn be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
	let slice = bytes.get(offset..offset + 4)?;
	Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// A `0xFFFFFFFF` record pointer means "absent" throughout the MOBI header.
fn record_pointer(bytes: &[u8], offset: usize) -> Option<usize> {
	match be_u32(bytes, offset) {
		Some(u32::MAX) | None => None,
		Some(value) => Some(value as usize),
	}
}

// ---------------------------------------------------------------------------
// Record 0: PalmDOC header, MOBI header, EXTH
// ---------------------------------------------------------------------------

/// `1` = stored, `2` = PalmDOC LZ77, `17480` = HUFF/CDIC
/// ([MOBI PalmDOC header](https://wiki.mobileread.com/wiki/MOBI#PalmDOC_Header)).
const COMPRESSION_NONE: u16 = 1;
const COMPRESSION_PALMDOC: u16 = 2;
const COMPRESSION_HUFF_CDIC: u16 = 17480;

/// `1252` = CP1252, `65001` = UTF-8
/// ([MOBI header](https://wiki.mobileread.com/wiki/MOBI#MOBI_Header)).
const ENCODING_CP1252: u32 = 1252;
const ENCODING_UTF8: u32 = 65001;

/// MOBI header offsets, all relative to the start of record 0 (the identifier
/// itself sits at 16, after the 16-byte PalmDOC header).
mod mobi_offset {
	pub const COMPRESSION: usize = 0;
	pub const TEXT_LENGTH: usize = 4;
	pub const TEXT_RECORD_COUNT: usize = 8;
	pub const ENCRYPTION_TYPE: usize = 12;
	pub const IDENTIFIER: usize = 16;
	pub const HEADER_LENGTH: usize = 20;
	pub const TEXT_ENCODING: usize = 28;
	pub const VERSION: usize = 36;
	pub const FULL_NAME_OFFSET: usize = 84;
	pub const FULL_NAME_LENGTH: usize = 88;
	pub const FIRST_RESOURCE: usize = 108;
	pub const HUFF_RECORD_OFFSET: usize = 112;
	pub const HUFF_RECORD_COUNT: usize = 116;
	pub const EXTH_FLAGS: usize = 128;
	/// KF8 reuses the MOBI 6 "first/last content record" words for the flow
	/// table's record number and entry count.
	pub const FDST_RECORD: usize = 192;
	pub const EXTRA_DATA_FLAGS: usize = 240;
	pub const NCX_INDEX: usize = 244;
	pub const FRAGMENT_INDEX: usize = 248;
	pub const SKELETON_INDEX: usize = 252;
}

/// `EXTH flags` bit 6 announces an EXTH block after the MOBI header.
const EXTH_PRESENT: u32 = 0x40;

/// The EXTH record types this reader consumes
/// ([MOBI EXTH header](https://wiki.mobileread.com/wiki/MOBI#EXTH_Header)).
mod exth_type {
	pub const AUTHOR: u32 = 100;
	pub const PUBLISHER: u32 = 101;
	pub const DESCRIPTION: u32 = 103;
	pub const ISBN: u32 = 104;
	pub const SUBJECT: u32 = 105;
	pub const PUBLISHING_DATE: u32 = 106;
	pub const ASIN: u32 = 113;
	/// Record number where the KF8 part of a dual-format file begins.
	pub const KF8_BOUNDARY: u32 = 121;
	pub const COVER_OFFSET: u32 = 201;
	pub const THUMB_OFFSET: u32 = 202;
	pub const UPDATED_TITLE: u32 = 503;
	pub const ASIN_ALT: u32 = 504;
	pub const LANGUAGE: u32 = 524;
}

/// The `MOBI type` value KindleGen 2 writes for a KF8 part, and the file
/// version that goes with it.
const KF8_VERSION: u32 = 8;

/// Everything this reader needs out of record 0 of one part.
#[derive(Debug, Clone, Default)]
struct PartHeader {
	compression: u16,
	text_length: usize,
	text_record_count: usize,
	encryption_type: u16,
	version: u32,
	text_encoding: u32,
	full_name: Option<String>,
	/// Record number (part-relative) of the first non-text resource record.
	first_resource: Option<usize>,
	huff_record: Option<usize>,
	huff_count: usize,
	/// Which trailing entries every text record carries.
	extra_flags: u32,
	fdst_record: Option<usize>,
	ncx_index: Option<usize>,
	fragment_index: Option<usize>,
	skeleton_index: Option<usize>,
	exth: Vec<(u32, Vec<u8>)>,
}

impl PartHeader {
	fn parse(record0: &[u8]) -> Result<Self, FileError> {
		if record0.len() < 16 {
			return Err(FileError::MobiReadError(
				"record 0 is shorter than a PalmDOC header".to_string(),
			));
		}
		let mut header = PartHeader {
			compression: be_u16(record0, mobi_offset::COMPRESSION).unwrap_or(0),
			text_length: be_u32(record0, mobi_offset::TEXT_LENGTH).unwrap_or(0) as usize,
			text_record_count: usize::from(
				be_u16(record0, mobi_offset::TEXT_RECORD_COUNT).unwrap_or(0),
			),
			encryption_type: be_u16(record0, mobi_offset::ENCRYPTION_TYPE).unwrap_or(0),
			text_encoding: ENCODING_CP1252,
			..Default::default()
		};

		// A `TEXtREAd` PalmDOC has no MOBI header at all: the 16-byte PalmDOC
		// header is the whole story, and the text is CP1252 with no resources.
		if record0.get(mobi_offset::IDENTIFIER..mobi_offset::IDENTIFIER + 4)
			!= Some(b"MOBI".as_slice())
		{
			return Ok(header);
		}

		let header_length =
			be_u32(record0, mobi_offset::HEADER_LENGTH).unwrap_or(0) as usize;
		// `header length` counts itself from the identifier, so the header ends
		// at record-0 offset `16 + header_length`.
		let header_end = mobi_offset::IDENTIFIER.saturating_add(header_length);
		let field = |offset: usize| (offset + 4 <= header_end).then_some(offset);

		header.text_encoding = field(mobi_offset::TEXT_ENCODING)
			.and_then(|offset| be_u32(record0, offset))
			.unwrap_or(ENCODING_CP1252);
		header.version = field(mobi_offset::VERSION)
			.and_then(|offset| be_u32(record0, offset))
			.unwrap_or(0);
		header.first_resource =
			field(mobi_offset::FIRST_RESOURCE).and_then(|o| record_pointer(record0, o));
		header.huff_record = field(mobi_offset::HUFF_RECORD_OFFSET)
			.and_then(|o| record_pointer(record0, o))
			.filter(|record| *record != 0);
		header.huff_count = field(mobi_offset::HUFF_RECORD_COUNT)
			.and_then(|offset| be_u32(record0, offset))
			.unwrap_or(0) as usize;
		header.extra_flags = field(mobi_offset::EXTRA_DATA_FLAGS)
			.and_then(|offset| be_u32(record0, offset))
			.unwrap_or(0);
		header.ncx_index =
			field(mobi_offset::NCX_INDEX).and_then(|o| record_pointer(record0, o));

		if header.version >= KF8_VERSION {
			header.fdst_record =
				field(mobi_offset::FDST_RECORD).and_then(|o| record_pointer(record0, o));
			header.fragment_index = field(mobi_offset::FRAGMENT_INDEX)
				.and_then(|o| record_pointer(record0, o));
			header.skeleton_index = field(mobi_offset::SKELETON_INDEX)
				.and_then(|o| record_pointer(record0, o));
		}

		let name_offset = field(mobi_offset::FULL_NAME_OFFSET)
			.and_then(|offset| be_u32(record0, offset))
			.unwrap_or(0) as usize;
		let name_length = field(mobi_offset::FULL_NAME_LENGTH)
			.and_then(|offset| be_u32(record0, offset))
			.unwrap_or(0) as usize;
		header.full_name = record0
			.get(name_offset..name_offset.saturating_add(name_length))
			.map(|bytes| decode_text(bytes, header.text_encoding))
			.filter(|name| !name.trim().is_empty());

		let exth_flags = field(mobi_offset::EXTH_FLAGS)
			.and_then(|offset| be_u32(record0, offset))
			.unwrap_or(0);
		if exth_flags & EXTH_PRESENT != 0 {
			header.exth = parse_exth(record0, header_end);
		}

		Ok(header)
	}

	fn exth_bytes(&self, record_type: u32) -> Option<&[u8]> {
		self.exth
			.iter()
			.find(|(kind, _)| *kind == record_type)
			.map(|(_, data)| data.as_slice())
	}

	fn exth_string(&self, record_type: u32) -> Option<String> {
		self.exth_bytes(record_type)
			.map(|bytes| decode_text(bytes, self.text_encoding))
			.map(|value| value.trim().to_owned())
			.filter(|value| !value.is_empty())
	}

	fn exth_strings(&self, record_type: u32) -> Vec<String> {
		self.exth
			.iter()
			.filter(|(kind, _)| *kind == record_type)
			.map(|(_, bytes)| decode_text(bytes, self.text_encoding).trim().to_owned())
			.filter(|value| !value.is_empty())
			.collect()
	}

	fn exth_u32(&self, record_type: u32) -> Option<u32> {
		self.exth_bytes(record_type)
			.and_then(|bytes| be_u32(bytes, 0))
			.filter(|value| *value != u32::MAX)
	}
}

/// The EXTH block starting at `start` in record 0: `EXTH`, header length,
/// record count, then `(type, length, data)` triples where the length counts
/// the 8-byte type/length pair
/// ([MOBI EXTH header](https://wiki.mobileread.com/wiki/MOBI#EXTH_Header)).
fn parse_exth(record0: &[u8], start: usize) -> Vec<(u32, Vec<u8>)> {
	if record0.get(start..start + 4) != Some(b"EXTH".as_slice()) {
		return Vec::new();
	}
	let block_length = be_u32(record0, start + 4).unwrap_or(0) as usize;
	let count = be_u32(record0, start + 8).unwrap_or(0) as usize;
	let end = record0
		.len()
		.min(start.saturating_add(block_length.max(12)));

	let mut records = Vec::new();
	let mut cursor = start + 12;
	for _ in 0..count {
		if cursor + 8 > end {
			break;
		}
		let record_type = be_u32(record0, cursor).unwrap_or(0);
		let length = be_u32(record0, cursor + 4).unwrap_or(0) as usize;
		if length < 8 || cursor + length > end {
			break;
		}
		let data = record0[cursor + 8..cursor + length].to_vec();
		records.push((record_type, data));
		cursor += length;
	}
	records
}

/// CP1252's 0x80..0x9F block, which is the only place it differs from
/// Latin-1. `U+FFFD` stands in for the five unassigned positions.
const CP1252_HIGH: [char; 32] = [
	'\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
	'\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}',
	'\u{017D}', '\u{FFFD}', '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
	'\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
	'\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
];

/// Decode `bytes` with the encoding the MOBI header declares. Anything that is
/// not UTF-8 is treated as CP1252, which is what every non-UTF-8 MOBI uses.
fn decode_text(bytes: &[u8], encoding: u32) -> String {
	if encoding == ENCODING_UTF8 {
		return String::from_utf8_lossy(bytes).into_owned();
	}
	bytes
		.iter()
		.map(|byte| match byte {
			0x80..=0x9F => CP1252_HIGH[usize::from(byte - 0x80)],
			other => char::from(*other),
		})
		.collect()
}

// ---------------------------------------------------------------------------
// Trailing entries and decompression
// ---------------------------------------------------------------------------

/// Drop the trailing entries the `Extra Record Data Flags` field announces.
///
/// Bits 1..15 each append `<data><size>`, where `size` is a *backward-encoded*
/// variable-width integer covering the whole entry, and the entries appear in
/// bit order — so entry 15 is at the very end and must come off first. Bit 0
/// is the multibyte-overlap entry: its last byte holds the byte count in its
/// low two bits, and those bytes reappear as normal content at the start of
/// the next record ([MOBI trailing
/// entries](https://wiki.mobileread.com/wiki/MOBI#Trailing_entries)).
fn strip_trailing_entries(record: &[u8], extra_flags: u32) -> &[u8] {
	let mut end = record.len();
	for bit in (1..16).rev() {
		if extra_flags & (1 << bit) == 0 {
			continue;
		}
		let size = backward_vwi(&record[..end]);
		if size == 0 || size > end {
			return &record[..end];
		}
		end -= size;
	}
	if extra_flags & 1 != 0 && end > 0 {
		let overlap = usize::from(record[end - 1] & 0x03) + 1;
		end = end.saturating_sub(overlap);
	}
	&record[..end]
}

/// A backward-encoded variable-width integer read from the end of `bytes`:
/// 7 bits per byte, big-endian, with only the most significant byte carrying
/// bit 8 ([MOBI variable-width
/// integers](https://wiki.mobileread.com/wiki/MOBI#Variable-width_integers)).
fn backward_vwi(bytes: &[u8]) -> usize {
	let mut value = 0usize;
	for (shift, byte) in bytes.iter().rev().take(4).enumerate() {
		value |= usize::from(byte & 0x7F) << (7 * shift);
		if byte & 0x80 != 0 {
			break;
		}
	}
	value
}

/// A forward-encoded variable-width integer at `cursor`: 7 bits per byte, and
/// the *last* byte carries bit 8. Returns the value and the new cursor.
fn forward_vwi(bytes: &[u8], mut cursor: usize) -> (u32, usize) {
	let mut value = 0u32;
	while let Some(byte) = bytes.get(cursor) {
		cursor += 1;
		value = (value << 7) | u32::from(byte & 0x7F);
		if byte & 0x80 != 0 {
			break;
		}
	}
	(value, cursor)
}

/// PalmDOC LZ77 with byte-pair escapes
/// ([PalmDOC](https://wiki.mobileread.com/wiki/PalmDOC#PalmDoc_byte_pair_compression)).
fn decompress_palmdoc(input: &[u8], out: &mut Vec<u8>) -> Result<(), FileError> {
	let mut cursor = 0;
	while let Some(&byte) = input.get(cursor) {
		cursor += 1;
		match byte {
			0x00 | 0x09..=0x7F => out.push(byte),
			0x01..=0x08 => {
				let count = usize::from(byte);
				let end = input.len().min(cursor + count);
				out.extend_from_slice(&input[cursor..end]);
				cursor = end;
			},
			0x80..=0xBF => {
				let Some(&next) = input.get(cursor) else {
					// A dangling first byte at the end of a record is how some
					// producers pad; there is nothing left to copy.
					break;
				};
				cursor += 1;
				let pair = (u32::from(byte) << 8) | u32::from(next);
				let distance = ((pair >> 3) & 0x07FF) as usize;
				let length = (pair & 0x07) as usize + 3;
				if distance == 0 || distance > out.len() {
					return Err(FileError::MobiReadError(format!(
						"PalmDOC back-reference of {distance} bytes at output offset {}",
						out.len()
					)));
				}
				// Byte-by-byte on purpose: an overlapping copy (distance <
				// length) is legal and repeats the pattern.
				for _ in 0..length {
					let byte = out[out.len() - distance];
					out.push(byte);
				}
			},
			0xC0..=0xFF => {
				out.push(b' ');
				out.push(byte ^ 0x80);
			},
		}
	}
	Ok(())
}

/// One HUFF `dict1` slot: the code length the top 8 bits of the window imply,
/// whether that length is already decisive, and the length's upper bound.
#[derive(Debug, Clone, Copy, Default)]
struct HuffSlot {
	code_length: u32,
	terminal: bool,
	max_code: u64,
}

/// A HUFF/CDIC ("huffdic") dictionary: PalmDOC compression type 17480.
///
/// One `HUFF` record carries the code tables and `huff_count - 1` `CDIC`
/// records carry the phrases. `dict1` is a 256-entry table keyed by the top 8
/// bits of a 32-bit code window; codes those 8 bits cannot resolve fall back to
/// the per-length `mincode`/`maxcode` bounds. A symbol's phrase index is
/// `maxcode - code`, so within one code length the phrases run backwards
/// through the code range, and the bounds have to be computed in 64 bits
/// because `(max + 1) << (32 - length)` overruns 32 bits at short lengths.
///
/// Every `CDIC` declares the *total* phrase count rather than its own, so a
/// record contributes `min(1 << bits, total - loaded)` phrases and the last
/// record's offset table has stale slots that must not be read. A phrase whose
/// length word lacks the `0x8000` flag is itself a compressed bitstream and
/// expands through the same decoder.
///
/// Layout derived from the record bytes and the MOBI format notes of
/// [Kindling](https://github.com/ciscoriordan/kindling) (MIT); the compression
/// type itself is on
/// [MobileRead MOBI](https://wiki.mobileread.com/wiki/MOBI#PalmDOC_Header).
struct HuffCdic {
	dict1: Vec<HuffSlot>,
	/// Indexed by code length 1..=32.
	min_code: [u64; 33],
	max_code: [u64; 33],
	/// `(bytes, terminal)` per phrase.
	phrases: Vec<(Vec<u8>, bool)>,
}

/// A non-terminal phrase expands through the decoder again; real files nest one
/// level, so this only stops a self-referential phrase from recursing forever.
const HUFF_MAX_DEPTH: u32 = 16;

impl HuffCdic {
	fn parse(huff: &[u8], cdics: &[Vec<u8>]) -> Result<Self, FileError> {
		if huff.get(..4) != Some(b"HUFF".as_slice()) {
			return Err(FileError::MobiReadError(
				"compression type 17480 without a HUFF record".to_string(),
			));
		}
		let dict1_offset = be_u32(huff, 8).unwrap_or(0) as usize;
		let dict2_offset = be_u32(huff, 12).unwrap_or(0) as usize;

		let mut dict1 = Vec::with_capacity(256);
		for slot in 0..256usize {
			let raw = be_u32(huff, dict1_offset + slot * 4).ok_or_else(|| {
				FileError::MobiReadError("HUFF code table is truncated".to_string())
			})?;
			let code_length = raw & 0x1F;
			dict1.push(HuffSlot {
				code_length,
				terminal: raw & 0x80 != 0,
				max_code: shifted_max(u64::from(raw >> 8), code_length),
			});
		}

		let mut min_code = [0u64; 33];
		let mut max_code = [0u64; 33];
		for index in 0..32usize {
			let length = index as u32 + 1;
			let low = be_u32(huff, dict2_offset + index * 8).ok_or_else(|| {
				FileError::MobiReadError("HUFF bounds table is truncated".to_string())
			})?;
			let high = be_u32(huff, dict2_offset + index * 8 + 4).unwrap_or(0);
			min_code[length as usize] = u64::from(low) << (32 - length);
			max_code[length as usize] = shifted_max(u64::from(high), length);
		}

		let mut phrases = Vec::new();
		for cdic in cdics {
			if cdic.get(..4) != Some(b"CDIC".as_slice()) {
				return Err(FileError::MobiReadError(
					"HUFF record count covers a record that is not a CDIC".to_string(),
				));
			}
			let total = be_u32(cdic, 8).unwrap_or(0) as usize;
			let bits = be_u32(cdic, 12).unwrap_or(0);
			if bits > 16 {
				return Err(FileError::MobiReadError(format!(
					"CDIC declares {bits} index bits"
				)));
			}
			let slots = (1usize << bits).min(total.saturating_sub(phrases.len()));
			for slot in 0..slots {
				let offset = 16
					+ usize::from(be_u16(cdic, 16 + slot * 2).ok_or_else(|| {
						FileError::MobiReadError(
							"CDIC offset table is truncated".to_string(),
						)
					})?);
				let word = be_u16(cdic, offset).ok_or_else(|| {
					FileError::MobiReadError("CDIC phrase is truncated".to_string())
				})?;
				let length = usize::from(word & 0x7FFF);
				let bytes = cdic
					.get(offset + 2..offset + 2 + length)
					.ok_or_else(|| {
						FileError::MobiReadError(
							"CDIC phrase runs past the record".to_string(),
						)
					})?
					.to_vec();
				phrases.push((bytes, word & 0x8000 != 0));
			}
		}

		Ok(Self {
			dict1,
			min_code,
			max_code,
			phrases,
		})
	}

	fn decompress(&self, input: &[u8], out: &mut Vec<u8>) -> Result<(), FileError> {
		self.expand(input, out, 0)
	}

	fn expand(
		&self,
		input: &[u8],
		out: &mut Vec<u8>,
		depth: u32,
	) -> Result<(), FileError> {
		if depth >= HUFF_MAX_DEPTH {
			return Err(FileError::MobiReadError(
				"HUFF/CDIC phrase expands into itself".to_string(),
			));
		}

		let mut bits_left = (input.len() as i64) * 8;
		let mut bit = 0usize;
		while bits_left > 0 {
			let window = window_at(input, bit);
			let slot = self.dict1[(window >> 24) as usize];
			let mut code_length = slot.code_length;
			if code_length == 0 {
				return Err(FileError::MobiReadError(
					"HUFF code table has a zero-length code".to_string(),
				));
			}
			let max_code = if slot.terminal {
				slot.max_code
			} else {
				while code_length <= 32 && window < self.min_code[code_length as usize] {
					code_length += 1;
				}
				if code_length > 32 {
					return Err(FileError::MobiReadError(
						"HUFF bitstream has no code for the current window".to_string(),
					));
				}
				self.max_code[code_length as usize]
			};

			bits_left -= i64::from(code_length);
			if bits_left < 0 {
				// The tail of a record is zero padding, not a symbol.
				break;
			}
			bit += code_length as usize;

			let index = ((max_code.wrapping_sub(window)) >> (32 - code_length)) as usize;
			let (phrase, terminal) = self.phrases.get(index).ok_or_else(|| {
				FileError::MobiReadError(format!(
					"HUFF/CDIC phrase {index} is not in the dictionary"
				))
			})?;
			if *terminal {
				out.extend_from_slice(phrase);
			} else {
				self.expand(phrase, out, depth + 1)?;
			}
			if out.len() > MAX_TEXT_BYTES {
				return Err(FileError::MobiReadError(
					"HUFF/CDIC output exceeds the size limit".to_string(),
				));
			}
		}
		Ok(())
	}
}

/// The upper bound of a code range, left-aligned in the 32-bit code window.
/// Computed in 64 bits because the shift overruns 32 at short code lengths.
fn shifted_max(raw_max: u64, code_length: u32) -> u64 {
	if code_length == 0 || code_length > 32 {
		return 0;
	}
	((raw_max + 1) << (32 - code_length)) - 1
}

/// The 32-bit code window starting at bit `bit`, zero-padded past the end.
fn window_at(input: &[u8], bit: usize) -> u64 {
	let byte = bit / 8;
	let mut buffer = [0u8; 8];
	if byte < input.len() {
		let available = (input.len() - byte).min(8);
		buffer[..available].copy_from_slice(&input[byte..byte + available]);
	}
	let block = u64::from_be_bytes(buffer);
	(block >> (32 - (bit % 8))) & 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// INDX / TAGX
// ---------------------------------------------------------------------------

/// One index entry: its label plus the tag values the TAGX table describes.
#[derive(Debug, Clone, Default)]
struct IndexEntry {
	label: Vec<u8>,
	tags: HashMap<u8, Vec<u32>>,
}

impl IndexEntry {
	fn tag(&self, tag: u8) -> Option<u32> {
		self.tags
			.get(&tag)
			.and_then(|values| values.first())
			.copied()
	}

	/// The `(offset, length)` pair of a geometry tag. Some producers repeat the
	/// pair; only the first one is meaningful.
	fn pair(&self, tag: u8) -> Option<(usize, usize)> {
		let values = self.tags.get(&tag)?;
		Some((*values.first()? as usize, *values.get(1)? as usize))
	}
}

/// One TAGX table row: tag id, values per entry, control-byte mask, and the
/// end-of-control-byte marker
/// ([MOBI TAGX section](https://wiki.mobileread.com/wiki/MOBI#TAGX_section)).
#[derive(Debug, Clone, Copy)]
struct TagxRow {
	tag: u8,
	values_per_entry: u8,
	mask: u8,
	end_of_control_byte: bool,
}

/// A parsed index: its entries in order, plus the CNCX blob its `name` tags
/// point into.
#[derive(Debug, Clone, Default)]
struct Index {
	entries: Vec<IndexEntry>,
	cncx: Vec<u8>,
}

impl Index {
	/// The CNCX string at `offset`: a forward-encoded length followed by that
	/// many bytes.
	fn cncx_string(&self, offset: usize, encoding: u32) -> Option<String> {
		if offset >= self.cncx.len() {
			return None;
		}
		let (length, cursor) = forward_vwi(&self.cncx, offset);
		let bytes = self.cncx.get(cursor..cursor + length as usize)?;
		Some(decode_text(bytes, encoding))
	}
}

/// Read the index whose primary record is at `base`: the INDX header names the
/// number of data records that follow it, and the CNCX records come after those
/// ([MOBI index meta record](https://wiki.mobileread.com/wiki/MOBI#Index_meta_record)).
fn read_index(db: &mut PalmDatabase, base: usize) -> Result<Index, FileError> {
	let primary = db.record(base)?;
	if primary.get(..4) != Some(b"INDX".as_slice()) {
		return Ok(Index::default());
	}
	let header_length = be_u32(&primary, 4).unwrap_or(0) as usize;
	let data_records = be_u32(&primary, 24).unwrap_or(0) as usize;
	let (control_bytes, rows) = parse_tagx(&primary, header_length);
	if rows.is_empty() {
		return Ok(Index::default());
	}

	let mut entries = Vec::new();
	for offset in 1..=data_records {
		let record = db.record(base + offset)?;
		if record.get(..4) != Some(b"INDX".as_slice()) {
			continue;
		}
		entries.extend(parse_index_entries(&record, control_bytes, &rows));
	}

	// The CNCX records follow the data records; there is rarely more than one,
	// and concatenating them is how their offsets are addressed.
	let mut cncx = Vec::new();
	let mut cursor = base + data_records + 1;
	while cursor < db.len() {
		let record = db.record(cursor)?;
		if record.len() < 2
			|| record.get(..4) == Some(b"INDX".as_slice())
			|| is_container_record(&record)
		{
			break;
		}
		cncx.extend_from_slice(&record);
		cursor += 1;
	}

	Ok(Index { entries, cncx })
}

/// Records that mark the end of an index block rather than more CNCX data: the
/// format records, the high-DPI container records, and the first resource.
fn is_container_record(record: &[u8]) -> bool {
	const CONTAINERS: [&[u8]; 3] = [b"RESC", b"CONT", b"CRES"];
	ends_resource_range(record)
		|| CONTAINERS
			.iter()
			.any(|container| record.starts_with(container))
		|| ContentType::from_bytes(record).is_image()
}

/// The TAGX table that follows the INDX header of a primary index record.
fn parse_tagx(primary: &[u8], header_length: usize) -> (usize, Vec<TagxRow>) {
	if primary.get(header_length..header_length + 4) != Some(b"TAGX".as_slice()) {
		return (0, Vec::new());
	}
	let table_length = be_u32(primary, header_length + 4).unwrap_or(0) as usize;
	let control_bytes = be_u32(primary, header_length + 8).unwrap_or(0) as usize;
	let mut rows = Vec::new();
	let mut cursor = header_length + 12;
	while cursor + 4 <= header_length + table_length {
		let row = &primary[cursor..cursor + 4];
		rows.push(TagxRow {
			tag: row[0],
			values_per_entry: row[1],
			mask: row[2],
			end_of_control_byte: row[3] & 0x01 != 0,
		});
		cursor += 4;
	}
	(control_bytes, rows)
}

/// The entries of one index data record. `idxt start` names an `IDXT` block of
/// big-endian 16-bit entry offsets, and each entry is
/// `[label length][label][control bytes][values]`.
fn parse_index_entries(
	record: &[u8],
	control_bytes: usize,
	rows: &[TagxRow],
) -> Vec<IndexEntry> {
	let idxt = be_u32(record, 20).unwrap_or(0) as usize;
	let count = be_u32(record, 24).unwrap_or(0) as usize;

	let offsets = (0..count)
		.map(|index| be_u16(record, idxt + 4 + index * 2).map(usize::from))
		.collect::<Option<Vec<_>>>()
		.unwrap_or_default();

	let mut entries = Vec::with_capacity(offsets.len());
	for (index, &start) in offsets.iter().enumerate() {
		let end = offsets
			.get(index + 1)
			.copied()
			.unwrap_or(idxt)
			.min(record.len());
		if start >= end {
			continue;
		}
		if let Some(entry) = parse_index_entry(&record[start..end], control_bytes, rows) {
			entries.push(entry);
		}
	}
	entries
}

fn parse_index_entry(
	blob: &[u8],
	control_bytes: usize,
	rows: &[TagxRow],
) -> Option<IndexEntry> {
	let label_length = usize::from(*blob.first()?);
	let label = blob.get(1..1 + label_length)?.to_vec();
	let controls = blob.get(1 + label_length..1 + label_length + control_bytes)?;
	let mut cursor = 1 + label_length + control_bytes;

	let mut tags: HashMap<u8, Vec<u32>> = HashMap::new();
	let mut control = 0usize;
	for row in rows {
		if row.end_of_control_byte {
			control += 1;
			continue;
		}
		let Some(&byte) = controls.get(control) else {
			continue;
		};
		let masked = byte & row.mask;
		if masked == 0 {
			continue;
		}
		let mut values = Vec::new();
		if masked == row.mask && row.mask.count_ones() > 1 {
			// An all-ones masked field means the value count is a byte budget
			// stored as a variable-width integer ahead of the values.
			let (budget, next) = forward_vwi(blob, cursor);
			cursor = next;
			let stop = cursor + budget as usize;
			while cursor < stop {
				let (value, next) = forward_vwi(blob, cursor);
				if next == cursor {
					break;
				}
				cursor = next;
				values.push(value);
			}
		} else {
			let groups = if masked == row.mask {
				1
			} else {
				u32::from(masked >> row.mask.trailing_zeros())
			};
			for _ in 0..groups * u32::from(row.values_per_entry) {
				let (value, next) = forward_vwi(blob, cursor);
				if next == cursor {
					break;
				}
				cursor = next;
				values.push(value);
			}
		}
		tags.insert(row.tag, values);
	}

	Some(IndexEntry { label, tags })
}

// ---------------------------------------------------------------------------
// Public model
// ---------------------------------------------------------------------------

/// One reassembled content document, in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobiSection {
	/// Archive-style name a converter can write it under, e.g. `part0000.html`.
	pub name: String,
	/// The document's `<title>`, when it has one.
	pub title: Option<String>,
	/// The markup, with every Kindle-internal link rewritten to point at the
	/// names in [`MobiBook::sections`], [`MobiBook::resources`] and
	/// [`MobiBook::flows`].
	pub html: Vec<u8>,
	/// Where the section starts in the coordinate space this part's links use:
	/// the reassembled text for KF8, the raw text for MOBI 6.
	pub text_start: usize,
	/// Length of the section in that same coordinate space.
	pub text_length: usize,
}

/// A non-markup resource record: an image, a font, or anything else the
/// resource range holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobiResource {
	/// Archive-style name, e.g. `images/image0000.png`.
	pub name: String,
	pub content_type: ContentType,
	/// Absolute record number, so [`MobiBook::resource_bytes`] can fetch it.
	record: usize,
}

/// A KF8 flow other than the markup flow: a style sheet, or an SVG the
/// producer kept out of the main flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobiFlow {
	/// Archive-style name, e.g. `styles/style0001.css`.
	pub name: String,
	pub content_type: ContentType,
	pub bytes: Vec<u8>,
}

/// One navigation entry out of the NCX index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobiNavEntry {
	pub title: String,
	/// Index into [`MobiBook::sections`].
	pub section: usize,
	/// Anchor inside that section, without the `#`. Empty for a whole-section
	/// entry.
	pub fragment: String,
	pub children: Vec<MobiNavEntry>,
}

/// A parsed Kindle book: the active part's metadata, its reassembled sections,
/// its flows, and the record numbers of its resources.
pub struct MobiBook {
	db: PalmDatabase,
	header: PartHeader,
	/// Record number the active part's record 0 sits at: `0` for a plain MOBI
	/// or a KF8-only `.azw3`, the EXTH 121 boundary for the KF8 half of a
	/// dual-format file.
	part_base: usize,
	kf8: bool,
	sections: Vec<MobiSection>,
	resources: Vec<MobiResource>,
	flows: Vec<MobiFlow>,
	navigation: Vec<MobiNavEntry>,
}

/// A summary rather than the whole book: the markup of every section would
/// otherwise land in every assertion message.
impl std::fmt::Debug for MobiBook {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MobiBook")
			.field("kf8", &self.kf8)
			.field("part_base", &self.part_base)
			.field("compression", &self.header.compression)
			.field("encoding", &self.header.text_encoding)
			.field("sections", &self.sections.len())
			.field("resources", &self.resources.len())
			.field("flows", &self.flows.len())
			.field("navigation", &self.navigation.len())
			.finish()
	}
}

impl MobiBook {
	/// Open and fully parse the book at `path`.
	///
	/// A protected file is refused with [`FileError::DrmProtected`] carrying
	/// the DRM detector's own reason: its text records are ciphertext, and
	/// decompressing them would produce garbage rather than a book.
	pub fn open(path: &Path) -> Result<Self, FileError> {
		if let Some(report) = detect_mobi_drm(File::open(path)?)? {
			return Err(FileError::DrmProtected(report.reason));
		}

		let mut db = PalmDatabase::open(path)?;
		let mut part_base = 0usize;
		let mut header = PartHeader::parse(&db.record(0)?)?;

		// A dual-format file carries a whole second database at the KF8
		// boundary; the KF8 part is the one to read, and every record number in
		// its header is relative to that boundary.
		if let Some(boundary) = header.exth_u32(exth_type::KF8_BOUNDARY) {
			let boundary = boundary as usize;
			if boundary > 0 && boundary < db.len() {
				let candidate = PartHeader::parse(&db.record(boundary)?)?;
				if candidate.version >= KF8_VERSION {
					part_base = boundary;
					header = candidate;
				}
			}
		}

		// The DRM detector only looks at record 0; the KF8 half declares its
		// own encryption type.
		if header.encryption_type != 0 {
			return Err(FileError::DrmProtected(format!(
				"The MOBI/AZW PalmDOC header declares encryption type {}, so its \
				 text records are ciphertext.",
				header.encryption_type
			)));
		}

		let kf8 = header.version >= KF8_VERSION;
		let text = read_text(&mut db, part_base, &header)?;
		let resources = read_resources(&mut db, part_base, &header)?;

		let mut book = Self {
			db,
			header,
			part_base,
			kf8,
			sections: Vec::new(),
			resources,
			flows: Vec::new(),
			navigation: Vec::new(),
		};

		if kf8 {
			book.build_kf8_sections(text)?;
		} else {
			book.build_mobi6_sections(text);
		}
		book.navigation = book.read_navigation()?;

		Ok(book)
	}

	/// `true` when the part being read is Kindle Format 8 (`.azw3`, or the KF8
	/// half of a dual-format `.mobi`).
	pub fn is_kf8(&self) -> bool {
		self.kf8
	}

	/// The book's title: EXTH 503 (`updatedtitle`), else the MOBI header's full
	/// name, else the Palm database name.
	pub fn title(&self) -> String {
		self.header
			.exth_string(exth_type::UPDATED_TITLE)
			.or_else(|| self.header.full_name.clone())
			.unwrap_or_else(|| self.db.name.clone())
	}

	pub fn authors(&self) -> Vec<String> {
		self.header.exth_strings(exth_type::AUTHOR)
	}

	pub fn language(&self) -> Option<String> {
		self.header.exth_string(exth_type::LANGUAGE)
	}

	pub fn sections(&self) -> &[MobiSection] {
		&self.sections
	}

	pub fn resources(&self) -> &[MobiResource] {
		&self.resources
	}

	pub fn flows(&self) -> &[MobiFlow] {
		&self.flows
	}

	pub fn navigation(&self) -> &[MobiNavEntry] {
		&self.navigation
	}

	/// The bytes of a resource listed in [`MobiBook::resources`].
	pub fn resource_bytes(&mut self, index: usize) -> Result<Vec<u8>, FileError> {
		let record = self
			.resources
			.get(index)
			.ok_or_else(|| FileError::ResourceNotFound(format!("resource {index}")))?
			.record;
		self.db.record(record)
	}

	/// The cover image: EXTH 201 (`coveroffset`) added to the first resource
	/// record, then EXTH 202 (`thumboffset`), then the first resource that is
	/// an image ([MOBI EXTH
	/// header](https://wiki.mobileread.com/wiki/MOBI#EXTH_Header)).
	pub fn cover(&mut self) -> Result<(ContentType, Vec<u8>), FileError> {
		let candidates = [
			self.header.exth_u32(exth_type::COVER_OFFSET),
			self.header.exth_u32(exth_type::THUMB_OFFSET),
		];
		for offset in candidates.into_iter().flatten() {
			let index = offset as usize;
			let Some(resource) = self.resources.get(index) else {
				continue;
			};
			if !resource.content_type.is_image() {
				continue;
			}
			let bytes = self.db.record(resource.record)?;
			if !bytes.is_empty() {
				return Ok((resource.content_type, bytes));
			}
		}

		let first_image = self
			.resources
			.iter()
			.position(|resource| resource.content_type.is_image());
		if let Some(index) = first_image {
			let resource = &self.resources[index];
			let content_type = resource.content_type;
			let bytes = self.db.record(resource.record)?;
			if !bytes.is_empty() {
				return Ok((content_type, bytes));
			}
		}

		Err(FileError::NoImageError)
	}

	/// The metadata EXTH carries, mapped onto Stump's shape through the same
	/// `HashMap` conversion the EPUB processor uses, so one key vocabulary
	/// covers both formats.
	pub fn metadata(&self) -> ProcessedMediaMetadata {
		let mut map: HashMap<String, Vec<String>> = HashMap::new();
		let mut insert = |key: &str, values: Vec<String>| {
			if !values.is_empty() {
				map.insert(key.to_string(), values);
			}
		};

		insert("title", vec![self.title()]);
		insert("author", self.authors());
		insert(
			"publisher",
			self.header
				.exth_string(exth_type::PUBLISHER)
				.into_iter()
				.collect(),
		);
		insert(
			"description",
			self.header
				.exth_string(exth_type::DESCRIPTION)
				.into_iter()
				.collect(),
		);
		insert(
			"identifier_isbn",
			self.header
				.exth_string(exth_type::ISBN)
				.into_iter()
				.collect(),
		);
		insert("subject", self.header.exth_strings(exth_type::SUBJECT));
		insert(
			"date",
			self.header
				.exth_string(exth_type::PUBLISHING_DATE)
				.into_iter()
				.collect(),
		);
		insert("language", self.language().into_iter().collect());
		insert(
			"identifier_mobi_asin",
			self.header
				.exth_string(exth_type::ASIN)
				.or_else(|| self.header.exth_string(exth_type::ASIN_ALT))
				.into_iter()
				.collect(),
		);

		ProcessedMediaMetadata::from(map)
	}

	// -- section building ---------------------------------------------------

	/// Split a MOBI 6 book on `<mbp:pagebreak>`, the tag the format's own
	/// authoring guidelines prescribe for a page change
	/// ([MOBI guidelines](https://wiki.mobileread.com/wiki/MOBI#Guidelines)).
	///
	/// Everything here works on the raw bytes rather than on decoded text,
	/// because a `filepos` link is a *byte* offset into the text: decoding CP1252
	/// first would move every offset past the first non-ASCII character.
	fn build_mobi6_sections(&mut self, text: Vec<u8>) {
		let lower = text.to_ascii_lowercase();
		let body = body_range(&lower);
		let anchors = collect_filepos_targets(&lower);

		let mut ranges = Vec::new();
		let mut start = body.start;
		let mut cursor = body.start;
		while let Some(found) = find_bytes(&lower[cursor..body.end], b"<mbp:pagebreak") {
			let tag_start = cursor + found;
			let tag_end = find_bytes(&lower[tag_start..body.end], b">")
				.map(|offset| tag_start + offset + 1)
				.unwrap_or(body.end);
			if tag_start > start {
				ranges.push(start..tag_start);
			}
			start = tag_end;
			cursor = tag_end;
			if cursor >= body.end {
				break;
			}
		}
		if start < body.end {
			ranges.push(start..body.end);
		}
		if ranges.is_empty() {
			ranges.push(body.clone());
		}

		let language = self.header.exth_string(exth_type::LANGUAGE);
		let title = self.title();
		let encoding = self.header.text_encoding;
		for (index, range) in ranges.iter().enumerate() {
			let cleaned = rewrite_mobi6_markup(
				&text[range.clone()],
				&lower[range.clone()],
				range.start,
				&anchors,
				&ranges,
			);
			// Decoding after rewriting is safe: every byte this adds is ASCII.
			let cleaned = decode_text(&cleaned, encoding);
			let heading =
				first_title(&cleaned).or_else(|| (index == 0).then(|| title.clone()));
			self.sections.push(MobiSection {
				name: section_name(index),
				title: heading,
				html: wrap_document(language.as_deref(), &cleaned).into_bytes(),
				text_start: range.start,
				text_length: range.len(),
			});
		}
	}

	/// Reassemble a KF8 part: the flow table splits the text, then the skeleton
	/// and fragment indices put each source file back together
	/// ([KF8](https://wiki.mobileread.com/wiki/KF8#The_Format)).
	fn build_kf8_sections(&mut self, text: Vec<u8>) -> Result<(), FileError> {
		let flows = self.read_flow_table(&text)?;
		let markup_end = flows.first().map(|range| range.end).unwrap_or(text.len());
		let markup = &text[..markup_end.min(text.len())];

		for (index, range) in flows.iter().enumerate().skip(1) {
			let bytes = text
				.get(range.start.min(text.len())..range.end.min(text.len()))
				.unwrap_or_default()
				.to_vec();
			let content_type = flow_content_type(&bytes);
			self.flows.push(MobiFlow {
				name: flow_name(index, content_type),
				content_type,
				bytes,
			});
		}

		let skeletons = match self.header.skeleton_index {
			Some(index) => read_index(&mut self.db, self.part_base + index)?,
			None => Index::default(),
		};
		let fragments = match self.header.fragment_index {
			Some(index) => read_index(&mut self.db, self.part_base + index)?,
			None => Index::default(),
		};

		if skeletons.entries.is_empty() {
			// No skeleton table: the markup flow is one document. Rare, but a
			// single-file KF8 is legal and must still be readable.
			let html = rewrite_kf8_markup(
				&decode_text(markup, self.header.text_encoding),
				0,
				&Kf8LinkTargets::default(),
			);
			self.sections.push(MobiSection {
				name: section_name(0),
				title: first_title(&html),
				text_start: 0,
				text_length: markup.len(),
				html: html.into_bytes(),
			});
			return Ok(());
		}

		// Assemble first, so link rewriting can resolve a fragment id to the
		// section it lands in.
		let mut assembled: Vec<(Vec<u8>, usize)> = Vec::new();
		let mut fragment_cursor = 0usize;
		let mut file_start = 0usize;
		for skeleton in &skeletons.entries {
			let Some((offset, length)) = skeleton.pair(6) else {
				continue;
			};
			let chunk_count = skeleton.tag(1).unwrap_or(0) as usize;
			let mut document = markup
				.get(offset..offset.saturating_add(length))
				.unwrap_or_default()
				.to_vec();

			// Chunks sit in the raw text immediately after their skeleton, in
			// order; the fragment's label is where it belongs in the finished
			// text, so subtracting everything emitted so far gives the offset
			// inside this document.
			let mut chunk_cursor = offset.saturating_add(length);
			for _ in 0..chunk_count {
				let Some(fragment) = fragments.entries.get(fragment_cursor) else {
					break;
				};
				fragment_cursor += 1;
				let chunk_length =
					fragment.pair(6).map(|(_, length)| length).unwrap_or(0);
				let chunk = markup
					.get(chunk_cursor..chunk_cursor.saturating_add(chunk_length))
					.unwrap_or_default()
					.to_vec();
				chunk_cursor = chunk_cursor.saturating_add(chunk_length);

				let insert = decimal_label(&fragment.label)
					.saturating_sub(file_start)
					.min(document.len());
				document.splice(insert..insert, chunk);
			}

			assembled.push((document, file_start));
			file_start += assembled.last().map(|(bytes, _)| bytes.len()).unwrap_or(0);
		}

		let targets = kf8_link_targets(&fragments, self.header.text_encoding);
		for (index, (document, start)) in assembled.into_iter().enumerate() {
			let markup = decode_text(&document, self.header.text_encoding);
			let html = rewrite_kf8_markup(&markup, index, &targets);
			self.sections.push(MobiSection {
				name: section_name(index),
				title: first_title(&html),
				text_start: start,
				text_length: document.len(),
				html: html.into_bytes(),
			});
		}

		Ok(())
	}

	/// The KF8 flow table: `FDST`, the offset of the table, the entry count,
	/// then `(start, end)` byte offsets into the part's text.
	fn read_flow_table(
		&mut self,
		text: &[u8],
	) -> Result<Vec<std::ops::Range<usize>>, FileError> {
		let Some(record) = self.header.fdst_record else {
			return Ok(vec![0..text.len()]);
		};
		let fdst = self.db.record(self.part_base + record)?;
		if fdst.get(..4) != Some(b"FDST".as_slice()) {
			return Ok(vec![0..text.len()]);
		}
		let table = be_u32(&fdst, 4).unwrap_or(12) as usize;
		let count = be_u32(&fdst, 8).unwrap_or(0) as usize;
		let mut flows = Vec::with_capacity(count);
		for index in 0..count {
			let start = be_u32(&fdst, table + index * 8).unwrap_or(0) as usize;
			let end = be_u32(&fdst, table + index * 8 + 4).unwrap_or(0) as usize;
			if end < start || start > text.len() {
				continue;
			}
			flows.push(start..end.min(text.len()));
		}
		if flows.is_empty() {
			flows.push(0..text.len());
		}
		Ok(flows)
	}

	/// The navigation tree out of the NCX index: tag 1 is the text position,
	/// tag 3 the CNCX offset of the label, and tag 4 the nesting depth.
	fn read_navigation(&mut self) -> Result<Vec<MobiNavEntry>, FileError> {
		let Some(index) = self.header.ncx_index else {
			return Ok(Vec::new());
		};
		let ncx = read_index(&mut self.db, self.part_base + index)?;
		let encoding = self.header.text_encoding;

		let mut flat = Vec::new();
		for entry in &ncx.entries {
			let Some(position) = entry.tag(1) else {
				continue;
			};
			let title = entry
				.tag(3)
				.and_then(|offset| ncx.cncx_string(offset as usize, encoding))
				.map(|title| title.trim().to_owned())
				.filter(|title| !title.is_empty());
			let Some(title) = title else {
				continue;
			};
			let section = self.section_at(position as usize);
			flat.push((
				entry.tag(4).unwrap_or(0),
				MobiNavEntry {
					title,
					section,
					fragment: String::new(),
					children: Vec::new(),
				},
			));
		}

		Ok(nest_nav_entries(flat))
	}

	/// The section holding a text position, in that part's own coordinate
	/// space.
	fn section_at(&self, position: usize) -> usize {
		self.sections
			.iter()
			.position(|section| {
				position >= section.text_start
					&& position < section.text_start + section.text_length
			})
			.unwrap_or(0)
	}
}

/// Decompress every text record of the part into one buffer.
fn read_text(
	db: &mut PalmDatabase,
	part_base: usize,
	header: &PartHeader,
) -> Result<Vec<u8>, FileError> {
	let huff = match header.compression {
		COMPRESSION_HUFF_CDIC => {
			let record = header.huff_record.ok_or_else(|| {
				FileError::MobiReadError(
					"compression type 17480 without a HUFF record number".to_string(),
				)
			})?;
			let huff_record = db.record(part_base + record)?;
			let mut cdics = Vec::new();
			for offset in 1..header.huff_count {
				cdics.push(db.record(part_base + record + offset)?);
			}
			Some(HuffCdic::parse(&huff_record, &cdics)?)
		},
		_ => None,
	};

	let mut text = Vec::with_capacity(header.text_length.min(MAX_TEXT_BYTES));
	for offset in 1..=header.text_record_count {
		let record = db.record(part_base + offset)?;
		if record.is_empty() {
			continue;
		}
		let payload = strip_trailing_entries(&record, header.extra_flags);
		match header.compression {
			COMPRESSION_NONE => text.extend_from_slice(payload),
			COMPRESSION_PALMDOC => decompress_palmdoc(payload, &mut text)?,
			COMPRESSION_HUFF_CDIC => {
				huff.as_ref()
					.expect("HUFF dictionary is built for this compression type")
					.decompress(payload, &mut text)?;
			},
			other => {
				return Err(FileError::MobiReadError(format!(
					"unsupported PalmDOC compression type {other}"
				)))
			},
		}
		if text.len() > MAX_TEXT_BYTES {
			return Err(FileError::MobiReadError(
				"decompressed text exceeds the size limit".to_string(),
			));
		}
	}

	// The PalmDOC header's `text length` is *not* the length of the whole
	// stream: a KF8 producer may count only the markup flow, leaving the style
	// sheet flow past the declared end. The FDST table delimits KF8 flows and
	// `</body>` delimits MOBI 6 content, so the decompressed length is kept as
	// it is rather than trusting the declaration.
	if header.text_length != text.len() {
		tracing::trace!(
			declared = header.text_length,
			decompressed = text.len(),
			"MOBI text length differs from the declared length"
		);
	}
	Ok(text)
}

/// The resource records of the part: everything from `First Image index` up to
/// the first container record that is not book content.
fn read_resources(
	db: &mut PalmDatabase,
	part_base: usize,
	header: &PartHeader,
) -> Result<Vec<MobiResource>, FileError> {
	let Some(first) = header.first_resource.filter(|first| *first > 0) else {
		return Ok(Vec::new());
	};
	let mut resources = Vec::new();
	let mut record = part_base + first;
	while record < db.len() {
		let bytes = db.record(record)?;
		if ends_resource_range(&bytes) {
			break;
		}
		let content_type = ContentType::from_bytes(&bytes);
		let name = if content_type.is_image() {
			format!(
				"images/image{:04}.{}",
				resources.len(),
				content_type.extension()
			)
		} else {
			// A `RESC` layout record or a `CRES`/`CONT` high-DPI placeholder
			// sits *inside* the range and takes a slot, so keeping it preserves
			// the 1-based `kindle:embed` numbering of everything after it.
			format!("resources/resource{:04}", resources.len())
		};
		resources.push(MobiResource {
			name,
			content_type: if content_type.is_image() {
				content_type
			} else {
				ContentType::UNKNOWN
			},
			record,
		});
		record += 1;
	}
	Ok(resources)
}

/// The records that follow the resources rather than belonging to them: the
/// flow table, the two bookkeeping records, the compilation records, the
/// boundary marker of a dual-format file and the end-of-file record
/// ([MOBI magic records](https://wiki.mobileread.com/wiki/MOBI#Magic_Records)).
fn ends_resource_range(bytes: &[u8]) -> bool {
	const TERMINATORS: [&[u8]; 8] = [
		b"FDST",
		b"FLIS",
		b"FCIS",
		b"SRCS",
		b"CMET",
		b"HUFF",
		b"CDIC",
		b"BOUNDARY",
	];
	bytes == [0xE9, 0x8E, 0x0D, 0x0A]
		|| TERMINATORS
			.iter()
			.any(|terminator| bytes.starts_with(terminator))
}

// ---------------------------------------------------------------------------
// Markup rewriting
// ---------------------------------------------------------------------------

/// KF8 uses base32 with the digits `0-9A-V` for every internal id.
fn base32(value: &str) -> Option<usize> {
	if value.is_empty() {
		return None;
	}
	let mut total = 0usize;
	for character in value.chars() {
		let digit = match character.to_ascii_uppercase() {
			'0'..='9' => u32::from(character as u8 - b'0'),
			'A'..='V' => u32::from(character.to_ascii_uppercase() as u8 - b'A') + 10,
			_ => return None,
		};
		total = total.checked_mul(32)?.checked_add(digit as usize)?;
	}
	Some(total)
}

fn section_name(index: usize) -> String {
	format!("part{index:04}.html")
}

fn flow_name(index: usize, content_type: ContentType) -> String {
	match content_type {
		ContentType::XHTML | ContentType::XML | ContentType::HTML => {
			format!("flows/flow{index:04}.xhtml")
		},
		_ => format!("styles/style{index:04}.css"),
	}
}

/// A KF8 flow is a style sheet unless it opens like markup.
fn flow_content_type(bytes: &[u8]) -> ContentType {
	let head = String::from_utf8_lossy(&bytes[..bytes.len().min(64)]);
	let head = head.trim_start();
	if head.starts_with("<?xml") || head.starts_with('<') {
		ContentType::XML
	} else {
		// `text/css` has no `ContentType` variant; `TXT` is the closest honest
		// answer and the name carries the `.css` extension.
		ContentType::TXT
	}
}

/// Where a fragment id points: the section it lands in and the `aid` anchor.
#[derive(Debug, Clone, Default)]
struct Kf8LinkTargets {
	/// Fragment index -> `(section index, aid)`.
	fragments: HashMap<usize, (usize, String)>,
}

/// Where every fragment id points: the section its file number names and the
/// `aid` its CNCX entry carries.
fn kf8_link_targets(fragments: &Index, encoding: u32) -> Kf8LinkTargets {
	let mut targets = Kf8LinkTargets::default();
	for (index, fragment) in fragments.entries.iter().enumerate() {
		let section = fragment.tag(3).unwrap_or(0) as usize;
		let aid = fragment
			.tag(2)
			.and_then(|offset| fragments.cncx_string(offset as usize, encoding))
			.and_then(|text| extract_aid(&text))
			.unwrap_or_default();
		targets.fragments.insert(index, (section, aid));
	}
	targets
}

/// The `aid` out of a fragment's CNCX text, which KindleGen writes as an XPath
/// predicate: `P-//*[@aid='0003']`.
fn extract_aid(text: &str) -> Option<String> {
	let start = text.find("@aid=")? + "@aid=".len();
	let rest = &text[start..];
	let quote = rest.chars().next()?;
	let rest = &rest[quote.len_utf8()..];
	let end = rest.find(quote)?;
	Some(rest[..end].to_string())
}

/// Rewrite the `kindle:` URI scheme KF8 uses for every internal reference.
///
/// - `kindle:pos:fid:FFFF:off:OOOOOOOOOO` addresses a fragment, so it becomes
///   the section that fragment lands in plus its `aid` anchor.
/// - `kindle:flow:FFFF` addresses a non-markup flow.
/// - `kindle:embed:EEEE` addresses a resource record, 1-based.
fn rewrite_kf8_markup(markup: &str, section: usize, targets: &Kf8LinkTargets) -> String {
	let mut out = String::with_capacity(markup.len());
	let mut rest = markup;
	while let Some(found) = rest.find("kindle:") {
		out.push_str(&rest[..found]);
		let uri = &rest[found..];
		let end = uri
			.find(|c: char| c == '"' || c == '\'' || c == ' ' || c == '>')
			.unwrap_or(uri.len());
		let (link, tail) = uri.split_at(end);
		out.push_str(&rewrite_kindle_uri(link, section, targets));
		rest = tail;
	}
	out.push_str(rest);
	out
}

fn rewrite_kindle_uri(link: &str, section: usize, targets: &Kf8LinkTargets) -> String {
	// The query only ever carries `?mime=`, which the rewritten name encodes in
	// its extension.
	let body = link.split('?').next().unwrap_or(link);
	if let Some(rest) = body.strip_prefix("kindle:pos:fid:") {
		let mut parts = rest.split(":off:");
		let fid = parts.next().unwrap_or_default();
		if let Some(index) = base32(fid) {
			if let Some((target, aid)) = targets.fragments.get(&index) {
				let anchor = if aid.is_empty() {
					String::new()
				} else {
					format!("#{aid}")
				};
				return if *target == section {
					if anchor.is_empty() {
						section_name(*target)
					} else {
						anchor
					}
				} else {
					format!("{}{anchor}", section_name(*target))
				};
			}
		}
		return section_name(section);
	}
	if let Some(rest) = body.strip_prefix("kindle:flow:") {
		if let Some(index) = base32(rest) {
			let content_type = link
				.split("mime=")
				.nth(1)
				.map(ContentType::from)
				.unwrap_or(ContentType::TXT);
			return flow_name(index, content_type);
		}
	}
	if let Some(rest) = body.strip_prefix("kindle:embed:") {
		if let Some(index) = base32(rest) {
			let content_type = link
				.split("mime=")
				.nth(1)
				.map(ContentType::from)
				.unwrap_or(ContentType::UNKNOWN);
			// `kindle:embed` is 1-based over the resource records.
			let extension = if content_type == ContentType::UNKNOWN {
				"jpg".to_string()
			} else {
				content_type.extension().to_string()
			};
			return format!("images/image{:04}.{extension}", index.saturating_sub(1));
		}
	}
	body.to_string()
}

/// The first occurrence of `needle` in `haystack`.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	if needle.is_empty() || needle.len() > haystack.len() {
		return None;
	}
	haystack
		.windows(needle.len())
		.position(|window| window == needle)
}

/// Every `filepos` target in a MOBI 6 book, so anchors can be planted before
/// the links that point at them are rewritten. `lower` is the ASCII-lowercased
/// text, which has the same length and therefore the same offsets.
fn collect_filepos_targets(lower: &[u8]) -> Vec<usize> {
	const NAME: &[u8] = b"filepos=";
	let mut targets = Vec::new();
	let mut cursor = 0usize;
	while let Some(found) = find_bytes(&lower[cursor..], NAME) {
		cursor += found + NAME.len();
		let digits = lower[cursor..]
			.iter()
			.skip_while(|byte| **byte == b'"' || **byte == b'\'')
			.take_while(|byte| byte.is_ascii_digit())
			.map(|byte| char::from(*byte))
			.collect::<String>();
		if let Ok(position) = digits.parse::<usize>() {
			targets.push(position);
		}
	}
	targets.sort_unstable();
	targets.dedup();
	targets
}

/// Rewrite one MOBI 6 fragment: drop the `mbp:` layout elements, turn
/// `recindex` image references into resource names, plant `filepos` anchors,
/// and rewrite `filepos` links to them.
fn rewrite_mobi6_markup(
	fragment: &[u8],
	lower: &[u8],
	fragment_start: usize,
	anchors: &[usize],
	sections: &[std::ops::Range<usize>],
) -> Vec<u8> {
	let mut out = Vec::with_capacity(fragment.len() + 32);
	let mut anchor_cursor = anchors.partition_point(|target| *target < fragment_start);
	let mut cursor = 0usize;

	let plant = |out: &mut Vec<u8>, cursor: &mut usize, limit: usize| {
		while let Some(&target) = anchors.get(*cursor) {
			if target > limit {
				break;
			}
			out.extend_from_slice(
				format!("<a id=\"filepos{target:010}\"></a>").as_bytes(),
			);
			*cursor += 1;
		}
	};

	while cursor < fragment.len() {
		// Plant every anchor that falls at or before this byte, so a link into
		// the middle of a paragraph still lands in the right place.
		plant(&mut out, &mut anchor_cursor, fragment_start + cursor);

		if lower[cursor] != b'<' {
			let next = find_bytes(&lower[cursor + 1..], b"<")
				.map(|offset| cursor + 1 + offset)
				.unwrap_or(fragment.len());
			out.extend_from_slice(&fragment[cursor..next]);
			cursor = next;
			continue;
		}
		let tag_end = find_bytes(&lower[cursor..], b">")
			.map(|offset| cursor + offset + 1)
			.unwrap_or(fragment.len());
		let tag = &fragment[cursor..tag_end];
		let tag_lower = &lower[cursor..tag_end];
		cursor = tag_end;

		if tag_lower.starts_with(b"<mbp:") || tag_lower.starts_with(b"</mbp:") {
			continue;
		}
		if tag_lower.starts_with(b"<img") || tag_lower.starts_with(b"<image") {
			out.extend_from_slice(&rewrite_recindex(tag, tag_lower));
			continue;
		}
		if find_bytes(tag_lower, b"filepos=").is_some() {
			out.extend_from_slice(&rewrite_filepos(tag_lower, fragment_start, sections));
			continue;
		}
		out.extend_from_slice(tag);
	}

	// Anchors that land exactly on the fragment's end still belong to it.
	plant(
		&mut out,
		&mut anchor_cursor,
		fragment_start + fragment.len(),
	);
	out
}

/// `<img recindex="0001">` names a resource record, 1-based
/// ([MOBI image records](https://wiki.mobileread.com/wiki/MOBI#Image_Records)).
fn rewrite_recindex(tag: &[u8], tag_lower: &[u8]) -> Vec<u8> {
	let index = attribute_value(tag_lower, b"recindex")
		.and_then(|value| value.trim().parse::<usize>().ok());
	let Some(index) = index else {
		return tag.to_vec();
	};
	format!(
		"<img src=\"images/image{:04}.jpg\" alt=\"\"/>",
		index.saturating_sub(1)
	)
	.into_bytes()
}

/// A `filepos` link addresses a byte offset in the text; the anchors planted by
/// [`rewrite_mobi6_markup`] give it a destination, and the section ranges say
/// which document that anchor ended up in.
fn rewrite_filepos(
	tag_lower: &[u8],
	fragment_start: usize,
	sections: &[std::ops::Range<usize>],
) -> Vec<u8> {
	let position = attribute_value(tag_lower, b"filepos")
		.and_then(|value| value.trim().parse::<usize>().ok());
	let Some(position) = position else {
		return tag_lower.to_vec();
	};
	if !tag_lower.starts_with(b"<a") {
		// `<reference filepos=...>` inside `<guide>` is metadata, not content.
		return Vec::new();
	}
	let target = sections
		.iter()
		.position(|range| range.contains(&position))
		.unwrap_or(0);
	let document = if sections
		.get(target)
		.is_some_and(|range| range.start == fragment_start)
	{
		String::new()
	} else {
		section_name(target)
	};
	format!("<a href=\"{document}#filepos{position:010}\">").into_bytes()
}

/// The value of `name=` in an ASCII-lowercased tag, with or without quotes.
/// MOBI 6 writes bare values (`filepos=0000000123`), which no XML parser would
/// accept.
fn attribute_value(tag_lower: &[u8], name: &[u8]) -> Option<String> {
	let found = find_bytes(tag_lower, name)?;
	let start = found + name.len();
	if tag_lower.get(start) != Some(&b'=') {
		return None;
	}
	let rest = tag_lower.get(start + 1..)?;
	let value = match rest.first()? {
		quote @ (b'"' | b'\'') => {
			let rest = &rest[1..];
			let end = find_bytes(rest, &[*quote])?;
			&rest[..end]
		},
		_ => {
			let end = rest
				.iter()
				.position(|byte| {
					byte.is_ascii_whitespace() || *byte == b'>' || *byte == b'/'
				})
				.unwrap_or(rest.len());
			&rest[..end]
		},
	};
	Some(String::from_utf8_lossy(value).into_owned())
}

/// The inner range of the first `<body>` element, or the whole document when it
/// has none. `lower` is the ASCII-lowercased text.
fn body_range(lower: &[u8]) -> std::ops::Range<usize> {
	let start = find_bytes(lower, b"<body")
		.and_then(|found| {
			find_bytes(&lower[found..], b">").map(|offset| found + offset + 1)
		})
		.unwrap_or(0);
	let end = (0..lower.len().saturating_sub(5))
		.rev()
		.find(|offset| lower[*offset..].starts_with(b"</body"))
		.unwrap_or(lower.len())
		.max(start);
	start..end
}

/// The text of the first heading or `<title>` in a fragment, used as a section
/// title when the navigation index has nothing to say.
fn first_title(markup: &str) -> Option<String> {
	let lower = markup.to_ascii_lowercase();
	for (open, close) in [
		("<title>", "</title>"),
		("<h1", "</h1>"),
		("<h2", "</h2>"),
		("<h3", "</h3>"),
	] {
		let Some(start) = lower.find(open) else {
			continue;
		};
		let start = match markup[start..].find('>') {
			Some(offset) => start + offset + 1,
			None => continue,
		};
		let Some(end) = lower[start..].find(close).map(|offset| start + offset) else {
			continue;
		};
		let text = strip_tags(&markup[start..end]);
		if !text.is_empty() {
			return Some(text);
		}
	}
	None
}

fn strip_tags(markup: &str) -> String {
	let mut out = String::with_capacity(markup.len());
	let mut depth = 0usize;
	for character in markup.chars() {
		match character {
			'<' => depth += 1,
			'>' => depth = depth.saturating_sub(1),
			other if depth == 0 => out.push(other),
			_ => (),
		}
	}
	out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wrap a MOBI 6 fragment in the XHTML skeleton an EPUB content document needs.
fn wrap_document(language: Option<&str>, body: &str) -> String {
	let language = language
		.map(|language| format!(" xml:lang=\"{language}\""))
		.unwrap_or_default();
	format!(
		"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html \
		 xmlns=\"http://www.w3.org/1999/xhtml\"{language}><head><meta \
		 charset=\"utf-8\"/><title></title></head><body>{body}</body></html>"
	)
}

/// A fragment's label is its insert position, written as decimal digits.
fn decimal_label(label: &[u8]) -> usize {
	let text = String::from_utf8_lossy(label);
	text.trim().trim_start_matches('0').parse().unwrap_or(0)
}

/// Nest a depth-tagged flat navigation list.
fn nest_nav_entries(flat: Vec<(u32, MobiNavEntry)>) -> Vec<MobiNavEntry> {
	let mut roots: Vec<MobiNavEntry> = Vec::new();
	// Path of indices into the tree, one per open depth level.
	let mut path: Vec<usize> = Vec::new();

	for (depth, entry) in flat {
		let depth = depth as usize;
		path.truncate(depth);
		if path.is_empty() {
			roots.push(entry);
			path.push(roots.len() - 1);
			continue;
		}
		let mut cursor = &mut roots[path[0]];
		for index in &path[1..] {
			cursor = &mut cursor.children[*index];
		}
		cursor.children.push(entry);
		let child = cursor.children.len() - 1;
		path.push(child);
	}
	roots
}

// ---------------------------------------------------------------------------
// FileProcessor
// ---------------------------------------------------------------------------

/// A file processor for the Kindle/Mobipocket formats.
pub struct MobiProcessor;

impl MobiProcessor {
	/// The cover image of the book at `path`.
	pub fn get_cover(path: &str) -> Result<(ContentType, Vec<u8>), FileError> {
		MobiBook::open(Path::new(path))?.cover()
	}

	/// The markup of one section, 0-based.
	pub fn get_section(
		path: &str,
		section: usize,
	) -> Result<(ContentType, Vec<u8>), FileError> {
		let book = MobiBook::open(Path::new(path))?;
		let available = book.sections.len();
		let content_type = book.section_content_type();
		let section = book.sections.get(section).ok_or(FileError::PageNotFound {
			page: section,
			available,
		})?;
		Ok((content_type, section.html.clone()))
	}
}

/// The section a page number addresses: page 1 is the cover, so section `0` is
/// page 2. `None` for a page that is not a section at all.
fn page_section(page: i32) -> Option<usize> {
	usize::try_from(page).ok()?.checked_sub(2)
}

impl MobiBook {
	fn section_content_type(&self) -> ContentType {
		if self.kf8 {
			ContentType::XHTML
		} else {
			ContentType::HTML
		}
	}
}

impl FileProcessor for MobiProcessor {
	/// Bytes of the first few sections, the same rule the EPUB processor uses,
	/// so a metadata edit that leaves the text alone keeps the hash stable.
	fn get_sample_size(path: &str) -> Result<u64, FileError> {
		let book = MobiBook::open(Path::new(path))?;
		Ok(book
			.sections
			.iter()
			.take(6)
			.map(|section| section.html.len() as u64)
			.sum())
	}

	fn generate_stump_hash(path: &str) -> Option<String> {
		let sample = MobiProcessor::get_sample_size(path).ok()?;
		match hash::generate(path, sample) {
			Ok(digest) => Some(digest),
			Err(error) => {
				tracing::debug!(error = ?error, path, "Failed to digest mobi file");
				None
			},
		}
	}

	fn generate_hashes(
		path: &str,
		FileProcessorOptions {
			generate_file_hashes,
			generate_koreader_hashes,
			..
		}: FileProcessorOptions,
	) -> Result<ProcessedFileHashes, FileError> {
		let hash = generate_file_hashes
			.then(|| MobiProcessor::generate_stump_hash(path))
			.flatten();
		let koreader_hash = generate_koreader_hashes
			.then(|| generate_koreader_hash(path))
			.transpose()?;

		Ok(ProcessedFileHashes {
			hash,
			koreader_hash,
		})
	}

	fn process_metadata(path: &str) -> Result<Option<ProcessedMediaMetadata>, FileError> {
		Ok(Some(MobiBook::open(Path::new(path))?.metadata()))
	}

	fn process(
		path: &str,
		options: FileProcessorOptions,
		_: &MediaConfig,
	) -> Result<ProcessedFile, FileError> {
		tracing::trace!(?path, "Processing mobi");

		let book = MobiBook::open(Path::new(path))?;
		// The cover plus one page per section; see `get_page_count`.
		let pages = book.sections.len() as i32 + 1;
		let metadata = book.metadata();
		drop(book);

		let ProcessedFileHashes {
			hash,
			koreader_hash,
		} = MobiProcessor::generate_hashes(path, options)?;

		Ok(ProcessedFile {
			path: std::path::PathBuf::from(path),
			hash,
			koreader_hash,
			metadata: Some(metadata),
			pages,
		})
	}

	/// Page 1 is the cover — the thumbnail job asks for page 1 and nothing
	/// else (`core/src/filesystem/image/thumbnail/generate.rs`) — and pages
	/// `2..=count` are the sections in order.
	///
	/// Unlike [`super::epub::EpubProcessor`], which indexes the spine with the
	/// raw page number and so never serves its first two spine items, the
	/// offset is applied here: every section a Kindle book has is reachable,
	/// because `get_page` is the *only* text route for one (the Readium and
	/// EPUB routes are gated on the `epub` extension).
	fn get_page(
		path: &str,
		page: i32,
		_: &MediaConfig,
	) -> Result<(ContentType, Vec<u8>), FileError> {
		if page == 1 {
			return MobiProcessor::get_cover(path);
		}
		let Some(section) = page_section(page) else {
			return Err(FileError::PageNotFound {
				page: page.max(0) as usize,
				available: MobiProcessor::get_page_count(path, &MediaConfig::default())?
					as usize,
			});
		};
		MobiProcessor::get_section(path, section)
	}

	/// The cover plus one page per reassembled section. Unlike an EPUB, a
	/// Kindle book is one compressed text stream, so there is no per-document
	/// compressed size to build a synthetic page budget from.
	fn get_page_count(path: &str, _: &MediaConfig) -> Result<i32, FileError> {
		let sections = MobiBook::open(Path::new(path))?.sections.len();
		Ok(sections as i32 + 1)
	}

	fn get_page_content_types(
		path: &str,
		pages: Vec<i32>,
	) -> Result<HashMap<i32, ContentType>, FileError> {
		let mut book = MobiBook::open(Path::new(path))?;
		let section_type = book.section_content_type();
		let available = book.sections.len();

		let mut content_types = HashMap::new();
		for page in pages {
			if page == 1 {
				let (content_type, _) = book.cover()?;
				content_types.insert(page, content_type);
				continue;
			}
			// Pages past the end are simply absent from the result, the same
			// contract the EPUB processor has.
			if page_section(page).is_some_and(|section| section < available) {
				content_types.insert(page, section_type);
			}
		}
		Ok(content_types)
	}

	fn analyze_page(
		_path: &str,
		_page: i32,
		_config: &MediaConfig,
	) -> Result<AnalyzedPage, FileError> {
		Err(FileError::UnsupportedFileType(
			"Kindle page analysis is not supported".to_string(),
		))
	}
}

#[cfg(test)]
mod tests;
