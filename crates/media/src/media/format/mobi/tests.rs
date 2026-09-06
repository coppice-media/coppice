//! Byte-level fixtures for the Kindle reader.
//!
//! The `.azw3` under `integration-tests/data` was produced by `boko convert`
//! from a small EPUB and covers the real KF8 skeleton/fragment tables. Every
//! other case is built here out of bytes, because the header edge cases (a
//! `TEXtREAd` PalmDOC, a declared encryption type, a dual-format boundary,
//! HUFF/CDIC compression) cannot be produced by any converter that is
//! installable in this tree.

use std::path::PathBuf;

use tempfile::TempDir;

use super::*;

// ---------------------------------------------------------------------------
// Palm database fixtures
// ---------------------------------------------------------------------------

/// Assembles a Palm database record by record, laying out the header exactly as
/// [PDB](https://wiki.mobileread.com/wiki/PDB#Palm_Database_Format) describes
/// it so a header edge case can be expressed as bytes.
struct PalmBuilder {
	name: String,
	kind: [u8; 8],
	records: Vec<Vec<u8>>,
}

impl PalmBuilder {
	fn new(name: &str) -> Self {
		Self {
			name: name.to_string(),
			kind: *b"BOOKMOBI",
			records: Vec::new(),
		}
	}

	fn palmdoc_kind(mut self) -> Self {
		self.kind = *b"TEXtREAd";
		self
	}

	fn push(&mut self, record: Vec<u8>) -> usize {
		self.records.push(record);
		self.records.len() - 1
	}

	fn build(&self) -> Vec<u8> {
		let count = self.records.len();
		let list_len = PDB_HEADER_LEN + count * PDB_RECORD_ENTRY_LEN + 2;
		let mut header = vec![0u8; list_len];

		let name = self.name.as_bytes();
		let name_len = name.len().min(31);
		header[..name_len].copy_from_slice(&name[..name_len]);
		header[60..68].copy_from_slice(&self.kind);
		header[76..78].copy_from_slice(&(count as u16).to_be_bytes());

		let mut offset = list_len as u32;
		for (index, record) in self.records.iter().enumerate() {
			let entry = PDB_HEADER_LEN + index * PDB_RECORD_ENTRY_LEN;
			header[entry..entry + 4].copy_from_slice(&offset.to_be_bytes());
			header[entry + 7] = index as u8;
			offset += record.len() as u32;
		}

		let mut file = header;
		for record in &self.records {
			file.extend_from_slice(record);
		}
		file
	}

	fn write(&self, dir: &TempDir, name: &str) -> PathBuf {
		let path = dir.path().join(name);
		std::fs::write(&path, self.build()).expect("write fixture");
		path
	}
}

/// The MOBI header length every current producer writes, which is what makes
/// the KF8 index pointers at 0xF4..0xFC addressable.
const HEADER_LENGTH: u32 = 264;

/// Builds record 0: the PalmDOC header, the MOBI header, the EXTH block and
/// the full name, in that order
/// ([MOBI](https://wiki.mobileread.com/wiki/MOBI#MOBI_Header)).
#[derive(Debug, Clone)]
struct Record0 {
	compression: u16,
	text_length: u32,
	text_record_count: u16,
	encryption_type: u16,
	version: u32,
	encoding: u32,
	first_resource: u32,
	huff_record: u32,
	huff_count: u32,
	extra_flags: u32,
	fdst: Option<(u32, u32)>,
	ncx_index: Option<u32>,
	fragment_index: Option<u32>,
	skeleton_index: Option<u32>,
	exth: Vec<(u32, Vec<u8>)>,
	full_name: String,
}

impl Default for Record0 {
	fn default() -> Self {
		Self {
			compression: COMPRESSION_NONE,
			text_length: 0,
			text_record_count: 0,
			encryption_type: 0,
			version: 6,
			encoding: ENCODING_UTF8,
			first_resource: u32::MAX,
			huff_record: 0,
			huff_count: 0,
			extra_flags: 0,
			fdst: None,
			ncx_index: None,
			fragment_index: None,
			skeleton_index: None,
			exth: Vec::new(),
			full_name: String::new(),
		}
	}
}

impl Record0 {
	fn build(&self) -> Vec<u8> {
		let mut record = vec![0u8; 16 + HEADER_LENGTH as usize];
		record[0..2].copy_from_slice(&self.compression.to_be_bytes());
		record[4..8].copy_from_slice(&self.text_length.to_be_bytes());
		record[8..10].copy_from_slice(&self.text_record_count.to_be_bytes());
		record[10..12].copy_from_slice(&4096u16.to_be_bytes());
		record[12..14].copy_from_slice(&self.encryption_type.to_be_bytes());
		record[16..20].copy_from_slice(b"MOBI");
		record[20..24].copy_from_slice(&HEADER_LENGTH.to_be_bytes());
		record[24..28].copy_from_slice(&2u32.to_be_bytes());
		record[28..32].copy_from_slice(&self.encoding.to_be_bytes());
		record[36..40].copy_from_slice(&self.version.to_be_bytes());
		record[108..112].copy_from_slice(&self.first_resource.to_be_bytes());
		record[112..116].copy_from_slice(&self.huff_record.to_be_bytes());
		record[116..120].copy_from_slice(&self.huff_count.to_be_bytes());
		// No DRM key block: both words are the documented "absent" value.
		record[168..172].copy_from_slice(&u32::MAX.to_be_bytes());
		record[172..176].copy_from_slice(&u32::MAX.to_be_bytes());
		record[240..244].copy_from_slice(&self.extra_flags.to_be_bytes());
		for (offset, value) in [
			(192usize, self.fdst.map(|(record, _)| record)),
			(196, self.fdst.map(|(_, count)| count)),
			(244, self.ncx_index),
			(248, self.fragment_index),
			(252, self.skeleton_index),
		] {
			record[offset..offset + 4]
				.copy_from_slice(&value.unwrap_or(u32::MAX).to_be_bytes());
		}

		if !self.exth.is_empty() {
			record[128..132].copy_from_slice(&EXTH_PRESENT.to_be_bytes());
			let mut block = Vec::new();
			for (record_type, data) in &self.exth {
				block.extend_from_slice(&record_type.to_be_bytes());
				block.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
				block.extend_from_slice(data);
			}
			record.extend_from_slice(b"EXTH");
			record.extend_from_slice(&((block.len() + 12) as u32).to_be_bytes());
			record.extend_from_slice(&(self.exth.len() as u32).to_be_bytes());
			record.extend_from_slice(&block);
		}

		if !self.full_name.is_empty() {
			let offset = record.len() as u32;
			record[84..88].copy_from_slice(&offset.to_be_bytes());
			record[88..92].copy_from_slice(&(self.full_name.len() as u32).to_be_bytes());
			record.extend_from_slice(self.full_name.as_bytes());
			record.extend_from_slice(&[0, 0]);
		}

		record
	}
}

/// Split text into 4096-byte records the way the format requires, appending the
/// trailing entries `extra_flags` announces.
fn text_records(text: &[u8], compression: u16, extra_flags: u32) -> Vec<Vec<u8>> {
	let huff = (compression == COMPRESSION_HUFF_CDIC).then(HuffCdicWriter::new);
	text.chunks(4096)
		.map(|chunk| {
			let mut record = match compression {
				COMPRESSION_NONE => chunk.to_vec(),
				COMPRESSION_PALMDOC => compress_palmdoc(chunk),
				COMPRESSION_HUFF_CDIC => huff.as_ref().expect("writer").encode(chunk),
				other => panic!("unsupported compression {other}"),
			};
			// Bit 0 is the multibyte overlap entry (here: none, so a single
			// count byte of zero) and bit 1 is the TBS entry, whose size is a
			// backward-encoded VWI covering itself.
			if extra_flags & 1 != 0 {
				record.push(0);
			}
			if extra_flags & 2 != 0 {
				record.extend_from_slice(&[0x2A, 0x81]);
			}
			record
		})
		.collect()
}

// ---------------------------------------------------------------------------
// PalmDOC writer
// ---------------------------------------------------------------------------

/// A greedy PalmDOC LZ77 encoder, so the decoder is proved against a stream it
/// did not produce itself: it emits back-references wherever a 3..10 byte match
/// exists within 2047 bytes, byte pairs for `space + letter`, and literal runs
/// otherwise ([PalmDOC](https://wiki.mobileread.com/wiki/PalmDOC#PalmDoc_byte_pair_compression)).
fn compress_palmdoc(input: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(input.len());
	let mut cursor = 0usize;
	let mut literals: Vec<u8> = Vec::new();

	let flush = |out: &mut Vec<u8>, literals: &mut Vec<u8>| {
		for run in literals.chunks(8) {
			// A byte in 0x01..=0x08 or >= 0x80 cannot stand alone, so it is
			// emitted through the "literals" escape.
			let escapes = run
				.iter()
				.any(|byte| (0x01..=0x08).contains(byte) || *byte >= 0x80);
			if escapes {
				out.push(run.len() as u8);
				out.extend_from_slice(run);
			} else {
				out.extend_from_slice(run);
			}
		}
		literals.clear();
	};

	while cursor < input.len() {
		let mut best: Option<(usize, usize)> = None;
		let window = cursor.saturating_sub(2047);
		for start in window..cursor {
			let mut length = 0usize;
			while length < 10
				&& cursor + length < input.len()
				&& input[start + length] == input[cursor + length]
			{
				length += 1;
			}
			if length >= 3 && best.map(|(best, _)| length > best).unwrap_or(true) {
				best = Some((length, cursor - start));
			}
		}

		if let Some((length, distance)) = best {
			flush(&mut out, &mut literals);
			let pair = ((distance as u32) << 3) | (length as u32 - 3);
			out.push(0x80 | ((pair >> 8) as u8 & 0x3F));
			out.push((pair & 0xFF) as u8);
			cursor += length;
			continue;
		}

		if input[cursor] == b' ' {
			if let Some(&next) = input.get(cursor + 1) {
				if next.is_ascii_alphabetic() {
					flush(&mut out, &mut literals);
					out.push(next ^ 0x80);
					cursor += 2;
					continue;
				}
			}
		}

		literals.push(input[cursor]);
		cursor += 1;
		if literals.len() == 8 {
			flush(&mut out, &mut literals);
		}
	}
	flush(&mut out, &mut literals);
	out
}

// ---------------------------------------------------------------------------
// HUFF/CDIC writer
// ---------------------------------------------------------------------------

/// Number of phrases reachable with each code length in the fixture code.
const SHORT_SLOTS: usize = 8;
const MEDIUM_SLOTS: usize = 8;
const LONG_SLOTS: usize = 256;

/// Writes a HUFF record, its CDIC records, and the bitstreams that decode back
/// to a given text.
///
/// The code deliberately uses three lengths so the decoder's three paths are
/// all exercised: 4-bit and 8-bit codes resolve straight out of `dict1` (its
/// terminal entries), and 12-bit codes only resolve through the per-length
/// `mincode`/`maxcode` bounds (its non-terminal entries). Within each length
/// the phrase index runs *backwards* through the code range, which is the
/// property `maxcode - code` encodes.
struct HuffCdicWriter {
	/// `(bytes, terminal)` per phrase index.
	phrases: Vec<(Vec<u8>, bool)>,
	/// `(code value, code length)` per phrase index.
	codes: Vec<(u32, u32)>,
}

/// The multi-byte phrase the writer prefers over single bytes.
const PHRASE_WORD: &[u8] = b" the ";
/// The phrase stored as a nested bitstream rather than as literal bytes.
const PHRASE_NESTED: &[u8] = b"<p>";
/// Single bytes that get a 4-bit code.
const SHORT_BYTES: &[u8] = b" etaon";
/// Single bytes that get an 8-bit code.
const MEDIUM_BYTES: &[u8] = b"isrhldcu";

impl HuffCdicWriter {
	fn new() -> Self {
		let mut writer = Self {
			phrases: Vec::new(),
			codes: Vec::new(),
		};

		// Codes run downwards from the top of each length's range, and the
		// phrase index runs upwards, so index 0 gets the largest 4-bit code.
		for slot in 0..SHORT_SLOTS {
			writer.codes.push((15 - slot as u32, 4));
		}
		for slot in 0..MEDIUM_SLOTS {
			writer.codes.push((127 - slot as u32, 8));
		}
		for slot in 0..LONG_SLOTS {
			writer.codes.push((1919 - slot as u32, 12));
		}

		writer.phrases.push((PHRASE_WORD.to_vec(), true));
		// Filled in once the single-byte codes exist.
		writer.phrases.push((Vec::new(), false));
		for byte in SHORT_BYTES {
			writer.phrases.push((vec![*byte], true));
		}
		for byte in MEDIUM_BYTES {
			writer.phrases.push((vec![*byte], true));
		}
		for byte in 0..=u8::MAX {
			writer.phrases.push((vec![byte], true));
		}
		assert_eq!(writer.phrases.len(), writer.codes.len());

		let nested = writer.encode_bytes_only(PHRASE_NESTED);
		writer.phrases[1] = (nested, false);
		writer
	}

	/// Make the nested phrase reference itself, so the decoder's recursion
	/// guard can be proved.
	fn self_referential(mut self) -> Self {
		let mut bits = BitWriter::default();
		let (code, length) = self.codes[1];
		bits.push(code, length);
		self.phrases[1] = (bits.finish(), false);
		self
	}

	fn byte_index(&self, byte: u8) -> usize {
		if let Some(offset) = SHORT_BYTES.iter().position(|candidate| *candidate == byte)
		{
			return 2 + offset;
		}
		if let Some(offset) = MEDIUM_BYTES.iter().position(|candidate| *candidate == byte)
		{
			return 2 + SHORT_BYTES.len() + offset;
		}
		2 + SHORT_BYTES.len() + MEDIUM_BYTES.len() + usize::from(byte)
	}

	/// Encode using single-byte phrases only, for the nested phrase's own
	/// bitstream.
	fn encode_bytes_only(&self, text: &[u8]) -> Vec<u8> {
		let mut bits = BitWriter::default();
		for byte in text {
			let (code, length) = self.codes[self.byte_index(*byte)];
			bits.push(code, length);
		}
		bits.finish()
	}

	fn encode(&self, text: &[u8]) -> Vec<u8> {
		let mut bits = BitWriter::default();
		let mut cursor = 0usize;
		while cursor < text.len() {
			let rest = &text[cursor..];
			let index = if rest.starts_with(PHRASE_WORD) {
				cursor += PHRASE_WORD.len();
				0
			} else if rest.starts_with(PHRASE_NESTED) {
				cursor += PHRASE_NESTED.len();
				1
			} else {
				cursor += 1;
				self.byte_index(rest[0])
			};
			let (code, length) = self.codes[index];
			bits.push(code, length);
		}
		bits.finish()
	}

	fn huff_record(&self) -> Vec<u8> {
		let mut record = vec![0u8; 24];
		record[0..4].copy_from_slice(b"HUFF");
		record[4..8].copy_from_slice(&24u32.to_be_bytes());
		record[8..12].copy_from_slice(&24u32.to_be_bytes());
		record[12..16].copy_from_slice(&(24u32 + 1024).to_be_bytes());

		for slot in 0..256u32 {
			// The top 8 bits of the window decide the length: 0b1xxx_xxxx is a
			// 4-bit code, 120..=127 an 8-bit code, and anything lower needs the
			// bounds table.
			let entry = if slot >= 128 {
				(15 << 8) | 0x80 | 4
			} else if slot >= 120 {
				(135 << 8) | 0x80 | 8
			} else {
				9
			};
			record.extend_from_slice(&(entry as u32).to_be_bytes());
		}

		for length in 1..=32u32 {
			// Unused lengths carry the next code that *would* be available,
			// which is what makes the decoder's length search terminate; past
			// the longest code in use the bounds are zero.
			let (min, max) = match length {
				1 => (1, 0),
				2 => (2, 0),
				3 => (4, 0),
				4 => (8, 15),
				5 => (16, 0),
				6 => (32, 0),
				7 => (64, 0),
				8 => (120, 135),
				9 => (240, 0),
				10 => (480, 0),
				11 => (960, 0),
				12 => (1664, 1935),
				_ => (0, 0),
			};
			record.extend_from_slice(&(min as u32).to_be_bytes());
			record.extend_from_slice(&(max as u32).to_be_bytes());
		}
		record
	}

	/// One CDIC per 256 phrases. Every record declares the *total* phrase
	/// count, so the reader has to work out how many of its slots are real.
	fn cdic_records(&self) -> Vec<Vec<u8>> {
		const BITS: u32 = 8;
		let per_record = 1usize << BITS;
		let total = self.phrases.len();

		self.phrases
			.chunks(per_record)
			.map(|chunk| {
				let mut record = vec![0u8; 16];
				record[0..4].copy_from_slice(b"CDIC");
				record[4..8].copy_from_slice(&16u32.to_be_bytes());
				record[8..12].copy_from_slice(&(total as u32).to_be_bytes());
				record[12..16].copy_from_slice(&BITS.to_be_bytes());

				let table = chunk.len() * 2;
				let mut offsets = Vec::with_capacity(chunk.len());
				let mut data = Vec::new();
				for (bytes, terminal) in chunk {
					offsets.push((table + data.len()) as u16);
					let word = bytes.len() as u16 | if *terminal { 0x8000 } else { 0 };
					data.extend_from_slice(&word.to_be_bytes());
					data.extend_from_slice(bytes);
				}
				for offset in offsets {
					record.extend_from_slice(&offset.to_be_bytes());
				}
				record.extend_from_slice(&data);
				record
			})
			.collect()
	}
}

/// Packs codes most-significant-bit first and zero-pads to a byte boundary,
/// which is how a text record's bitstream ends.
#[derive(Default)]
struct BitWriter {
	bytes: Vec<u8>,
	current: u8,
	filled: u32,
}

impl BitWriter {
	fn push(&mut self, code: u32, length: u32) {
		for bit in (0..length).rev() {
			let value = (code >> bit) & 1;
			self.current = (self.current << 1) | value as u8;
			self.filled += 1;
			if self.filled == 8 {
				self.bytes.push(self.current);
				self.current = 0;
				self.filled = 0;
			}
		}
	}

	fn finish(mut self) -> Vec<u8> {
		if self.filled > 0 {
			self.bytes.push(self.current << (8 - self.filled));
		}
		self.bytes
	}
}

// ---------------------------------------------------------------------------
// Whole-book fixtures
// ---------------------------------------------------------------------------

/// A MOBI 6 book with `text`, built with the given compression.
fn mobi6(dir: &TempDir, name: &str, text: &str, compression: u16) -> PathBuf {
	mobi6_with(dir, name, text, compression, |_| {})
}

fn mobi6_with(
	dir: &TempDir,
	name: &str,
	text: &str,
	compression: u16,
	edit: impl FnOnce(&mut Record0),
) -> PathBuf {
	// Bit 0 + bit 1: the pair every current producer writes.
	let extra_flags = 0b11;
	let records = text_records(text.as_bytes(), compression, extra_flags);

	let mut header = Record0 {
		compression,
		text_length: text.len() as u32,
		text_record_count: records.len() as u16,
		extra_flags,
		full_name: "Fixture Book".to_string(),
		..Default::default()
	};
	edit(&mut header);

	let mut builder = PalmBuilder::new("Fixture Book");
	if compression == COMPRESSION_HUFF_CDIC {
		let writer = HuffCdicWriter::new();
		// The HUFF block sits after the text records; the header's record
		// numbers are what find it.
		header.huff_record = (1 + records.len()) as u32;
		header.huff_count = 1 + writer.cdic_records().len() as u32;
		builder.push(header.build());
		for record in records {
			builder.push(record);
		}
		builder.push(writer.huff_record());
		for record in writer.cdic_records() {
			builder.push(record);
		}
	} else {
		builder.push(header.build());
		for record in records {
			builder.push(record);
		}
	}
	builder.write(dir, name)
}

fn azw3_fixture() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("integration-tests/data/kindle-formats.azw3")
}

fn section_text(book: &MobiBook, index: usize) -> String {
	String::from_utf8(book.sections()[index].html.clone()).expect("utf-8 section")
}

// ---------------------------------------------------------------------------
// Palm database and header parsing
// ---------------------------------------------------------------------------

#[test]
fn a_file_that_is_not_a_mobipocket_database_is_refused() {
	let dir = TempDir::new().expect("temp dir");

	let short = dir.path().join("short.mobi");
	std::fs::write(&short, b"tiny").expect("write");
	let error = MobiBook::open(&short).expect_err("too short");
	assert!(matches!(error, FileError::MobiReadError(_)), "{error}");

	// Right size, wrong type/creator: a Plucker database is a Palm database
	// but not a book this reader can decode.
	let mut plucker = PalmBuilder::new("Plucker");
	plucker.kind = *b"DataPlkr";
	plucker.push(vec![0u8; 32]);
	let path = plucker.write(&dir, "other.pdb");
	let error = MobiBook::open(&path).expect_err("wrong creator");
	assert!(
		matches!(&error, FileError::MobiReadError(message) if message.contains("DataPlkr")),
		"{error}"
	);
}

#[test]
fn a_textread_palmdoc_without_a_mobi_header_still_reads() {
	let dir = TempDir::new().expect("temp dir");
	let text = "<html><body>Plain PalmDOC text with no MOBI header.</body></html>";

	let mut builder = PalmBuilder::new("Old Doc").palmdoc_kind();
	// A PalmDOC record 0 is only the 16-byte header.
	let mut header = vec![0u8; 16];
	header[0..2].copy_from_slice(&COMPRESSION_PALMDOC.to_be_bytes());
	header[4..8].copy_from_slice(&(text.len() as u32).to_be_bytes());
	header[8..10].copy_from_slice(&1u16.to_be_bytes());
	header[10..12].copy_from_slice(&4096u16.to_be_bytes());
	builder.push(header);
	builder.push(compress_palmdoc(text.as_bytes()));
	let path = builder.write(&dir, "old.prc");

	let book = MobiBook::open(&path).expect("open palmdoc");
	assert!(!book.is_kf8());
	assert_eq!(book.sections().len(), 1);
	assert!(section_text(&book, 0).contains("Plain PalmDOC text"));
	// No MOBI header means no EXTH and no full name: the database name is the
	// only title there is.
	assert_eq!(book.title(), "Old Doc");
}

#[test]
fn cp1252_text_is_decoded_through_the_windows_high_block() {
	let dir = TempDir::new().expect("temp dir");
	let mut builder = PalmBuilder::new("Latin");
	let header = Record0 {
		compression: COMPRESSION_NONE,
		text_record_count: 1,
		encoding: ENCODING_CP1252,
		full_name: "Latin".to_string(),
		..Default::default()
	};
	builder.push(header.build());
	// 0x92 is a right single quote in CP1252 and a continuation byte in UTF-8.
	let mut text = b"<html><body>Alice\x92s ".to_vec();
	text.extend_from_slice(b"caf\xE9\x85</body></html>");
	let text_len = text.len();
	builder.push(text);
	let path = builder.write(&dir, "latin.mobi");

	let book = MobiBook::open(&path).expect("open cp1252");
	let section = section_text(&book, 0);
	assert!(section.contains("Alice\u{2019}s"), "{section}");
	assert!(section.contains("caf\u{E9}\u{2026}"), "{section}");
	assert!(text_len > 0);
}

// ---------------------------------------------------------------------------
// Decompression
// ---------------------------------------------------------------------------

/// Text long enough to force back-references, byte pairs and several records.
fn sample_text() -> String {
	let mut text = String::from("<html><head><title>Sample</title></head><body>");
	for chapter in 0..3 {
		text.push_str(&format!(
			"<h1>Chapter {chapter}</h1><p>the quick brown fox jumps over the lazy \
			 dog, and the quick brown fox does it again.</p>"
		));
		if chapter < 2 {
			text.push_str("<mbp:pagebreak/>");
		}
	}
	text.push_str("</body></html>");
	text
}

#[test]
fn palmdoc_compressed_text_round_trips() {
	let dir = TempDir::new().expect("temp dir");
	let text = sample_text();
	let path = mobi6(&dir, "palmdoc.mobi", &text, COMPRESSION_PALMDOC);

	let book = MobiBook::open(&path).expect("open palmdoc");
	assert_eq!(book.sections().len(), 3, "one section per pagebreak span");
	for chapter in 0..3 {
		let section = section_text(&book, chapter);
		assert!(
			section.contains(&format!("Chapter {chapter}")),
			"section {chapter}: {section}"
		);
		assert!(section.contains("quick brown fox"), "{section}");
	}
}

#[test]
fn uncompressed_and_palmdoc_text_agree() {
	let dir = TempDir::new().expect("temp dir");
	let text = sample_text();
	let stored = mobi6(&dir, "stored.mobi", &text, COMPRESSION_NONE);
	let packed = mobi6(&dir, "packed.mobi", &text, COMPRESSION_PALMDOC);

	let stored = MobiBook::open(&stored).expect("open stored");
	let packed = MobiBook::open(&packed).expect("open packed");
	assert_eq!(
		stored
			.sections()
			.iter()
			.map(|s| s.html.clone())
			.collect::<Vec<_>>(),
		packed
			.sections()
			.iter()
			.map(|s| s.html.clone())
			.collect::<Vec<_>>(),
	);
}

#[test]
fn huff_cdic_compressed_text_round_trips() {
	let dir = TempDir::new().expect("temp dir");
	let text = sample_text();
	let path = mobi6(&dir, "huffdic.mobi", &text, COMPRESSION_HUFF_CDIC);

	let book = MobiBook::open(&path).expect("open huffdic");
	let stored = mobi6(&dir, "stored.mobi", &text, COMPRESSION_NONE);
	let stored = MobiBook::open(&stored).expect("open stored");
	assert_eq!(
		book.sections()
			.iter()
			.map(|s| s.html.clone())
			.collect::<Vec<_>>(),
		stored
			.sections()
			.iter()
			.map(|s| s.html.clone())
			.collect::<Vec<_>>(),
		"HUFF/CDIC must decode to the same text as the uncompressed twin"
	);
	// The nested (non-terminal) phrase is what `<p>` encodes to, so its
	// presence proves the recursive expansion path ran.
	assert!(section_text(&book, 0).contains("<p>"));
}

#[test]
fn a_self_referential_huff_phrase_is_refused_instead_of_recursing() {
	let dir = TempDir::new().expect("temp dir");
	let writer = HuffCdicWriter::new().self_referential();
	// One `<p>` is enough: its code resolves to the phrase that references it.
	let text = "<p>";

	let mut builder = PalmBuilder::new("Looping");
	let mut header = Record0 {
		compression: COMPRESSION_HUFF_CDIC,
		text_length: text.len() as u32,
		text_record_count: 1,
		huff_record: 2,
		huff_count: 1 + writer.cdic_records().len() as u32,
		full_name: "Looping".to_string(),
		..Default::default()
	};
	header.extra_flags = 0;
	builder.push(header.build());
	builder.push(writer.encode(text.as_bytes()));
	builder.push(writer.huff_record());
	for record in writer.cdic_records() {
		builder.push(record);
	}
	let path = builder.write(&dir, "loop.mobi");

	let error = MobiBook::open(&path).expect_err("self-referential phrase");
	assert!(
		matches!(&error, FileError::MobiReadError(message) if message.contains("itself")),
		"{error}"
	);
}

#[test]
fn trailing_entries_are_stripped_before_decompression() {
	let dir = TempDir::new().expect("temp dir");
	let text = "<html><body>No trailing junk here.</body></html>";

	// Without the flags the same bytes would leave the TBS entry in the text.
	let flagged = mobi6_with(&dir, "flagged.mobi", text, COMPRESSION_NONE, |header| {
		header.extra_flags = 0b11
	});
	let book = MobiBook::open(&flagged).expect("open flagged");
	let section = section_text(&book, 0);
	assert!(section.contains("No trailing junk here."), "{section}");
	assert!(!section.contains('\u{2a}'.to_string().repeat(2).as_str()));
	assert!(
		!section.contains("\u{0}"),
		"the multibyte count byte must not survive: {section}"
	);
}

#[test]
fn a_palmdoc_back_reference_past_the_output_is_refused() {
	let dir = TempDir::new().expect("temp dir");
	let mut builder = PalmBuilder::new("Corrupt");
	let header = Record0 {
		compression: COMPRESSION_PALMDOC,
		text_record_count: 1,
		full_name: "Corrupt".to_string(),
		..Default::default()
	};
	builder.push(header.build());
	// A length/distance pair as the very first byte pair has nothing behind it.
	builder.push(vec![0x80 | 0x3F, 0xFF]);
	let path = builder.write(&dir, "corrupt.mobi");

	let error = MobiBook::open(&path).expect_err("corrupt back-reference");
	assert!(
		matches!(&error, FileError::MobiReadError(message) if message.contains("back-reference")),
		"{error}"
	);
}

// ---------------------------------------------------------------------------
// EXTH metadata and DRM
// ---------------------------------------------------------------------------

#[test]
fn exth_records_become_stump_metadata() {
	let dir = TempDir::new().expect("temp dir");
	let path = mobi6_with(
		&dir,
		"meta.mobi",
		"<html><body>Body</body></html>",
		COMPRESSION_NONE,
		|header| {
			header.exth = vec![
				(exth_type::AUTHOR, b"Ada Palmer".to_vec()),
				(exth_type::AUTHOR, b"Bo Reader".to_vec()),
				(exth_type::PUBLISHER, b"Stump Press".to_vec()),
				(exth_type::DESCRIPTION, b"A short walk.".to_vec()),
				(exth_type::ISBN, b"9781234567897".to_vec()),
				(exth_type::SUBJECT, b"Reference".to_vec()),
				(exth_type::SUBJECT, b"Formats".to_vec()),
				(exth_type::PUBLISHING_DATE, b"2011-01-16".to_vec()),
				(exth_type::ASIN, b"B00M0DRZ56".to_vec()),
				(exth_type::LANGUAGE, b"en".to_vec()),
				(exth_type::UPDATED_TITLE, b"Kindle Formats".to_vec()),
			];
		},
	);

	let book = MobiBook::open(&path).expect("open");
	// EXTH 503 wins over the MOBI header's full name.
	assert_eq!(book.title(), "Kindle Formats");

	let metadata = book.metadata();
	assert_eq!(metadata.title.as_deref(), Some("Kindle Formats"));
	assert_eq!(
		metadata.writers,
		Some(vec!["Ada Palmer".to_string(), "Bo Reader".to_string()])
	);
	assert_eq!(metadata.publisher.as_deref(), Some("Stump Press"));
	assert_eq!(metadata.summary.as_deref(), Some("A short walk."));
	assert_eq!(metadata.identifier_isbn.as_deref(), Some("9781234567897"));
	assert_eq!(
		metadata.genres,
		Some(vec!["Reference".to_string(), "Formats".to_string()])
	);
	assert_eq!(metadata.year, Some(2011));
	assert_eq!(metadata.month, Some(1));
	assert_eq!(metadata.day, Some(16));
	assert_eq!(metadata.language.as_deref(), Some("en"));
	assert_eq!(metadata.identifier_mobi_asin.as_deref(), Some("B00M0DRZ56"));
}

#[test]
fn a_book_with_no_exth_falls_back_to_the_header_and_database_names() {
	let dir = TempDir::new().expect("temp dir");
	let path = mobi6(
		&dir,
		"plain.mobi",
		"<html><body>Body</body></html>",
		COMPRESSION_NONE,
	);
	let book = MobiBook::open(&path).expect("open");
	assert_eq!(book.title(), "Fixture Book");
	assert!(book.authors().is_empty());
}

#[test]
fn a_drm_protected_book_is_refused_with_the_detectors_reason() {
	let dir = TempDir::new().expect("temp dir");
	for encryption_type in [1u16, 2] {
		let path = mobi6_with(
			&dir,
			&format!("drm{encryption_type}.mobi"),
			"<html><body>ciphertext</body></html>",
			COMPRESSION_NONE,
			|header| header.encryption_type = encryption_type,
		);
		let error = MobiBook::open(&path).expect_err("protected book");
		let FileError::DrmProtected(reason) = &error else {
			panic!("expected a DRM verdict, got {error}");
		};
		assert!(
			reason.contains(&format!("encryption type {encryption_type}")),
			"{reason}"
		);
		// The processor entry points must refuse it too, not just the reader.
		let path = path.to_string_lossy().to_string();
		assert!(matches!(
			MobiProcessor::process_metadata(&path),
			Err(FileError::DrmProtected(_))
		));
		assert!(matches!(
			MobiProcessor::get_page(&path, 1, &MediaConfig::default()),
			Err(FileError::DrmProtected(_))
		));
	}
}

// ---------------------------------------------------------------------------
// MOBI 6 sections, images and links
// ---------------------------------------------------------------------------

#[test]
fn mobi6_sections_split_on_pagebreak_and_drop_the_mbp_elements() {
	let dir = TempDir::new().expect("temp dir");
	let text = "<html><head><title>T</title></head><body><mbp:frameset><h1>One</h1>\
	            <p>First</p><mbp:pagebreak/><h1>Two</h1><p>Second</p>\
	            <mbp:pagebreak/><h1>Three</h1><p>Third</p></mbp:frameset>\
	            </body></html>";
	let path = mobi6(&dir, "split.mobi", text, COMPRESSION_NONE);

	let book = MobiBook::open(&path).expect("open");
	assert_eq!(book.sections().len(), 3);
	assert_eq!(
		book.sections()
			.iter()
			.map(|section| section.name.clone())
			.collect::<Vec<_>>(),
		vec!["part0000.html", "part0001.html", "part0002.html"]
	);
	assert_eq!(
		book.sections()
			.iter()
			.map(|section| section.title.clone())
			.collect::<Vec<_>>(),
		vec![
			Some("One".to_string()),
			Some("Two".to_string()),
			Some("Three".to_string())
		]
	);

	let first = section_text(&book, 0);
	assert!(first.contains("First"), "{first}");
	assert!(!first.contains("Second"), "{first}");
	assert!(
		!first.contains("mbp:"),
		"mbp elements must be dropped: {first}"
	);
	assert!(first.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
	assert!(first.contains("xmlns=\"http://www.w3.org/1999/xhtml\""));
	// The wrapper closes what it opens, so each section is a document.
	assert!(first.ends_with("</body></html>"));
}

#[test]
fn mobi6_recindex_images_and_filepos_links_are_rewritten() {
	let dir = TempDir::new().expect("temp dir");
	// A `filepos` value is a fixed-width byte offset into the text, so the
	// document is laid out once with a placeholder to find the target's own
	// offset, then rebuilt with it: the digits keep their width, so nothing
	// moves.
	let layout = |target: usize| {
		format!(
			"<html><body><a filepos={target:010}>Go</a><img recindex=\"0002\"/>\
			 <p>First</p><mbp:pagebreak/><h1>Target</h1><p>Second</p></body></html>"
		)
	};
	let target = layout(0).find("<h1>Target").expect("target heading");
	let text = layout(target);
	let path = mobi6_with(&dir, "links.mobi", &text, COMPRESSION_NONE, |header| {
		header.first_resource = 2;
	});

	let book = MobiBook::open(&path).expect("open");
	assert_eq!(book.sections().len(), 2);
	let first = section_text(&book, 0);
	// `recindex` is 1-based over the resource records.
	assert!(first.contains("src=\"images/image0001.jpg\""), "{first}");
	// The link resolves to the section holding the target byte, plus the
	// anchor planted there.
	assert!(
		first.contains(&format!("href=\"part0001.html#filepos{target:010}\"")),
		"{first}"
	);
	let second = section_text(&book, 1);
	assert!(
		second.contains(&format!("id=\"filepos{target:010}\"")),
		"the anchor must exist in the target section: {second}"
	);
	// The anchor is planted before the heading it addresses, not after it.
	assert!(
		second.find("filepos").expect("anchor") < second.find("<h1").expect("heading"),
		"{second}"
	);
}

// ---------------------------------------------------------------------------
// KF8
// ---------------------------------------------------------------------------

#[test]
fn a_dual_format_file_reads_its_kf8_half() {
	let dir = TempDir::new().expect("temp dir");
	let mobi6_text = "<html><body>The MOBI 6 half.</body></html>";
	let kf8_text = "<html><body>The KF8 half.</body></html>";

	let mut builder = PalmBuilder::new("Dual");
	let boundary_record = 3u32;
	let first = Record0 {
		compression: COMPRESSION_NONE,
		text_length: mobi6_text.len() as u32,
		text_record_count: 1,
		full_name: "Dual".to_string(),
		exth: vec![(
			exth_type::KF8_BOUNDARY,
			boundary_record.to_be_bytes().to_vec(),
		)],
		..Default::default()
	};
	builder.push(first.build());
	builder.push(mobi6_text.as_bytes().to_vec());
	builder.push(b"BOUNDARY".to_vec());
	assert_eq!(builder.records.len(), boundary_record as usize);

	let second = Record0 {
		compression: COMPRESSION_NONE,
		text_length: kf8_text.len() as u32,
		text_record_count: 1,
		version: 8,
		full_name: "Dual KF8".to_string(),
		exth: vec![(exth_type::UPDATED_TITLE, b"Dual Format Book".to_vec())],
		..Default::default()
	};
	builder.push(second.build());
	builder.push(kf8_text.as_bytes().to_vec());
	let path = builder.write(&dir, "dual.mobi");

	let book = MobiBook::open(&path).expect("open dual");
	assert!(book.is_kf8(), "the KF8 half is the one to read");
	assert_eq!(book.title(), "Dual Format Book");
	let section = section_text(&book, 0);
	assert!(section.contains("The KF8 half."), "{section}");
	assert!(!section.contains("MOBI 6"), "{section}");
}

#[test]
fn kf8_sections_are_reassembled_from_the_skeleton_and_fragment_indices() {
	let book = MobiBook::open(&azw3_fixture()).expect("open azw3");
	assert!(book.is_kf8());

	// The source EPUB had a cover page plus three chapters.
	assert_eq!(book.sections().len(), 4);
	assert_eq!(
		book.sections()
			.iter()
			.map(|section| section.title.clone())
			.collect::<Vec<_>>(),
		vec![
			Some("Cover".to_string()),
			Some("The Palm Database".to_string()),
			Some("Compression".to_string()),
			Some("Kindle Format 8".to_string()),
		]
	);

	// A reassembled section is the skeleton with its fragment spliced back in
	// between `<body>` and `</body>`, so both halves must be present and in
	// order.
	let chapter = section_text(&book, 1);
	assert!(chapter.starts_with("<?xml version=\"1.0\""), "{chapter}");
	assert!(
		chapter.contains("<title>The Palm Database</title>"),
		"{chapter}"
	);
	let body = chapter.find("<body").expect("body");
	let heading = chapter.find("<h1").expect("heading");
	let close = chapter.find("</body>").expect("body close");
	assert!(body < heading && heading < close, "{chapter}");
	assert!(
		chapter.contains("A Palm database stores its records behind an eight-byte"),
		"{chapter}"
	);
	assert!(chapter.ends_with("</html>"), "{chapter}");

	// Every byte of the markup flow ends up in exactly one section.
	let total: usize = book
		.sections()
		.iter()
		.map(|section| section.text_length)
		.sum();
	assert_eq!(total, 1642, "the declared markup flow length");
}

#[test]
fn kf8_internal_links_are_rewritten_to_flow_and_resource_names() {
	let book = MobiBook::open(&azw3_fixture()).expect("open azw3");

	for section in book.sections() {
		let html = String::from_utf8(section.html.clone()).expect("utf-8");
		assert!(
			!html.contains("kindle:"),
			"{} still carries a kindle: URI: {html}",
			section.name
		);
	}

	// The chapters link the style sheet flow; the cover page and chapter two
	// carry the two images.
	let chapter = section_text(&book, 1);
	assert!(
		chapter.contains("href=\"styles/style0001.css\""),
		"{chapter}"
	);
	assert!(
		section_text(&book, 0).contains("src=\"images/image0000.png\""),
		"{}",
		section_text(&book, 0)
	);
	assert!(
		section_text(&book, 2).contains("src=\"images/image0001.png\""),
		"{}",
		section_text(&book, 2)
	);
}

#[test]
fn kf8_flows_carry_the_style_sheet() {
	let book = MobiBook::open(&azw3_fixture()).expect("open azw3");
	assert_eq!(book.flows().len(), 1);
	let flow = &book.flows()[0];
	assert_eq!(flow.name, "styles/style0001.css");
	let css = String::from_utf8(flow.bytes.clone()).expect("utf-8 css");
	assert!(css.contains("text-indent"), "{css}");
}

#[test]
fn kf8_resources_are_the_image_records_in_embed_order() {
	let mut book = MobiBook::open(&azw3_fixture()).expect("open azw3");
	assert_eq!(
		book.resources()
			.iter()
			.map(|resource| resource.name.clone())
			.collect::<Vec<_>>(),
		vec!["images/image0000.png", "images/image0001.png"]
	);
	let bytes = book.resource_bytes(0).expect("first resource");
	assert_eq!(ContentType::from_bytes(&bytes), ContentType::PNG);
	assert!(bytes.len() > 100);
}

#[test]
fn the_cover_comes_from_the_exth_cover_offset() {
	let mut book = MobiBook::open(&azw3_fixture()).expect("open azw3");
	let (content_type, bytes) = book.cover().expect("cover");
	assert_eq!(content_type, ContentType::PNG);
	// EXTH 201 is 0 here, so the cover is the first resource record.
	let first = book.resource_bytes(0).expect("first resource");
	assert_eq!(bytes, first);
}

#[test]
fn navigation_comes_from_the_ncx_index() {
	let book = MobiBook::open(&azw3_fixture()).expect("open azw3");
	let nav = book.navigation();
	assert_eq!(
		nav.iter()
			.map(|entry| entry.title.clone())
			.collect::<Vec<_>>(),
		vec!["The Palm Database", "Compression", "Kindle Format 8"]
	);
	// Each entry resolves onto the section whose text range holds its
	// position; the cover page is section 0, so the chapters start at 1.
	assert_eq!(
		nav.iter().map(|entry| entry.section).collect::<Vec<_>>(),
		vec![1, 2, 3]
	);
}

// ---------------------------------------------------------------------------
// FileProcessor
// ---------------------------------------------------------------------------

#[test]
fn page_one_is_the_cover_and_the_rest_are_the_sections_in_order() {
	let path = azw3_fixture().to_string_lossy().to_string();
	let config = MediaConfig::default();

	// Four sections plus the cover.
	assert_eq!(MobiProcessor::get_page_count(&path, &config).unwrap(), 5);

	let (content_type, cover) = MobiProcessor::get_page(&path, 1, &config).unwrap();
	assert_eq!(content_type, ContentType::PNG);
	assert!(!cover.is_empty());

	// Every section is reachable, including the first: page 2 is section 0.
	let titles = [
		"Cover",
		"The Palm Database",
		"Compression",
		"Kindle Format 8",
	];
	for (offset, title) in titles.iter().enumerate() {
		let page = offset as i32 + 2;
		let (content_type, section) =
			MobiProcessor::get_page(&path, page, &config).unwrap();
		assert_eq!(content_type, ContentType::XHTML);
		assert!(
			String::from_utf8_lossy(&section).contains(title),
			"page {page} should hold {title}"
		);
	}

	let error = MobiProcessor::get_page(&path, 9, &config).expect_err("past the end");
	assert!(
		matches!(
			error,
			FileError::PageNotFound {
				page: 7,
				available: 4
			}
		),
		"{error}"
	);

	let types = MobiProcessor::get_page_content_types(&path, vec![1, 2, 5, 6]).unwrap();
	assert_eq!(types.get(&1), Some(&ContentType::PNG));
	assert_eq!(types.get(&2), Some(&ContentType::XHTML));
	assert_eq!(types.get(&5), Some(&ContentType::XHTML));
	assert_eq!(types.get(&6), None, "past the last section");
}

#[test]
fn the_processor_produces_metadata_and_a_stable_hash() {
	let path = azw3_fixture().to_string_lossy().to_string();
	let metadata = MobiProcessor::process_metadata(&path)
		.expect("metadata")
		.expect("some metadata");
	assert_eq!(metadata.title.as_deref(), Some("Kindle Formats Explained"));
	assert_eq!(metadata.writers, Some(vec!["Ada Palmer".to_string()]));
	assert_eq!(metadata.publisher.as_deref(), Some("Stump Press"));
	assert_eq!(metadata.language.as_deref(), Some("en"));
	assert_eq!(metadata.year, Some(2011));

	let processed = MobiProcessor::process(
		&path,
		FileProcessorOptions {
			generate_file_hashes: true,
			..Default::default()
		},
		&MediaConfig::default(),
	)
	.expect("process");
	assert_eq!(processed.pages, 5, "four sections plus the cover");
	let first = processed.hash.expect("hash");
	let second = MobiProcessor::generate_stump_hash(&path).expect("hash again");
	assert_eq!(first, second, "the sampled hash must be deterministic");
}

#[test]
fn the_dispatcher_routes_kindle_files_to_this_processor() {
	// Goes through `determine_processor`, so this is the scanner's own path.
	let path = azw3_fixture();
	let count = crate::media::process::get_page_count(
		&path.to_string_lossy(),
		&MediaConfig::default(),
	)
	.expect("dispatch");
	assert_eq!(count, 5);

	let metadata = crate::media::process::process_metadata(&path)
		.expect("dispatch metadata")
		.expect("some metadata");
	assert_eq!(metadata.title.as_deref(), Some("Kindle Formats Explained"));
}

// ---------------------------------------------------------------------------
// Unit-level rules
// ---------------------------------------------------------------------------

#[test]
fn base32_ids_decode_with_the_kindle_alphabet() {
	assert_eq!(base32("0000"), Some(0));
	assert_eq!(base32("0001"), Some(1));
	// `A` is 10, so `000A` is ten and `0010` is thirty-two.
	assert_eq!(base32("000A"), Some(10));
	assert_eq!(base32("0010"), Some(32));
	assert_eq!(base32("000V"), Some(31));
	assert_eq!(base32("000W"), None, "W is past the alphabet");
	assert_eq!(base32(""), None);
}

#[test]
fn a_backward_encoded_size_reads_from_the_end() {
	// Only the most significant byte carries bit 8.
	assert_eq!(backward_vwi(&[0x00, 0x81]), 1);
	assert_eq!(backward_vwi(&[0x84, 0x22, 0x11]), 0x11111);
	assert_eq!(backward_vwi(&[]), 0);
}

#[test]
fn a_forward_encoded_value_stops_on_the_high_bit() {
	assert_eq!(forward_vwi(&[0x81], 0), (1, 1));
	assert_eq!(forward_vwi(&[0x04, 0x22, 0x91], 0), (0x11111, 3));
	// A truncated value consumes what is there rather than looping.
	assert_eq!(forward_vwi(&[0x04], 0), (4, 1));
}

#[test]
fn an_aid_is_taken_out_of_the_fragments_cncx_text() {
	assert_eq!(extract_aid("P-//*[@aid='0003']").as_deref(), Some("0003"));
	assert_eq!(extract_aid("//*[@aid=\"000A\"]").as_deref(), Some("000A"));
	assert_eq!(extract_aid("no aid here"), None);
}

#[test]
fn a_fragment_label_is_its_decimal_insert_position() {
	assert_eq!(decimal_label(b"0000000132"), 132);
	assert_eq!(decimal_label(b"0"), 0);
	assert_eq!(decimal_label(b"SKEL0000000000"), 0);
}

#[test]
fn navigation_depth_becomes_nesting() {
	let entry = |title: &str| MobiNavEntry {
		title: title.to_string(),
		section: 0,
		fragment: String::new(),
		children: Vec::new(),
	};
	let nested = nest_nav_entries(vec![
		(0, entry("Part One")),
		(1, entry("Chapter One")),
		(2, entry("Section A")),
		(1, entry("Chapter Two")),
		(0, entry("Part Two")),
	]);

	assert_eq!(nested.len(), 2);
	assert_eq!(nested[0].title, "Part One");
	assert_eq!(nested[0].children.len(), 2);
	assert_eq!(nested[0].children[0].children[0].title, "Section A");
	assert_eq!(nested[1].title, "Part Two");
	assert!(nested[1].children.is_empty());
}
