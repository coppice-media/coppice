//! Minimal ZIP writer for KEPUB output.
//!
//! The `zip` crate has no API for adding an entry whose bytes are already
//! compressed, which the parallel conversion pipeline needs (content
//! documents are deflated off-thread, then written in source order).  The
//! container format is small enough to write directly: local headers, entry
//! data, a central directory, and the end-of-central-directory record.
//! Reading still goes through `zip`; entries that are copied verbatim are
//! sliced from the source archive by offset without inflating them.

use std::io::Write;

use libdeflater::{CompressionLvl, Compressor, Decompressor};
use zip::{read::ZipFile, CompressionMethod, DateTime};

use crate::KepubError;

const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;
const VERSION_NEEDED: u16 = 20;
/// "Made by" version: Unix host (3) so the external attributes carry a mode.
const VERSION_MADE_BY: u16 = (3 << 8) | 20;
/// General-purpose flag bit 11: the file name is UTF-8.
const UTF8_FLAG: u16 = 1 << 11;
const MAX_ENTRY_SIZE: u64 = u32::MAX as u64;

/// Per-entry metadata carried from the source archive (or defaulted).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EntryMeta {
	pub(crate) modified: DateTime,
	pub(crate) unix_mode: Option<u32>,
}

impl EntryMeta {
	pub(crate) fn from_source(file: &ZipFile<'_>) -> Self {
		Self {
			modified: file.last_modified(),
			unix_mode: file.unix_mode(),
		}
	}
}

/// A deflated payload produced away from the writer (e.g. on a rayon worker).
#[derive(Debug)]
pub(crate) struct Deflated {
	pub(crate) name: String,
	pub(crate) meta: EntryMeta,
	pub(crate) crc32: u32,
	pub(crate) uncompressed_size: u64,
	pub(crate) data: Vec<u8>,
}

impl Deflated {
	/// Compress `data` with raw deflate at `level` (libdeflate, whole-buffer).
	pub(crate) fn compress(
		name: &str,
		meta: EntryMeta,
		data: &[u8],
		level: u32,
	) -> Result<Self, KepubError> {
		let level = CompressionLvl::new(level as i32).map_err(|_| {
			KepubError::InvalidEpub(format!("invalid deflate level {level}"))
		})?;
		let mut compressor = Compressor::new(level);
		let mut out = vec![0; compressor.deflate_compress_bound(data.len())];
		let written = compressor
			.deflate_compress(data, &mut out)
			.map_err(|error| {
				KepubError::InvalidEpub(format!("deflate failed: {error:?}"))
			})?;
		out.truncate(written);
		out.shrink_to_fit();
		Ok(Self {
			name: name.to_owned(),
			meta,
			crc32: libdeflater::crc32(data),
			uncompressed_size: data.len() as u64,
			data: out,
		})
	}
}

/// Where an entry's compressed bytes live in the source archive, so it can be
/// inflated later (on any thread) without holding a `ZipFile` borrow.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EntryLocation {
	data_start: u64,
	compressed_size: u64,
	size: u64,
	method: CompressionMethod,
}

impl EntryLocation {
	pub(crate) fn from_source(file: &ZipFile<'_>) -> Self {
		Self {
			data_start: file.data_start(),
			compressed_size: file.compressed_size(),
			size: file.size(),
			method: file.compression(),
		}
	}

	/// Inflate the entry in one libdeflate call (`Stored` is copied; other
	/// methods fall back to `zip`'s streaming reader).
	pub(crate) fn read(&self, source: &[u8], name: &str) -> Result<Vec<u8>, KepubError> {
		let start = self.data_start as usize;
		let end = start + self.compressed_size as usize;
		let raw = source.get(start..end).ok_or_else(|| {
			KepubError::InvalidEpub(format!("entry {name} points outside the archive"))
		})?;
		match self.method {
			CompressionMethod::Stored => Ok(raw.to_vec()),
			CompressionMethod::Deflated => {
				let mut out = vec![0; self.size as usize];
				let written = Decompressor::new()
					.deflate_decompress(raw, &mut out)
					.map_err(|error| {
						KepubError::InvalidEpub(format!(
							"inflate {name} failed: {error:?}"
						))
					})?;
				out.truncate(written);
				Ok(out)
			},
			_ => {
				let mut archive = zip::ZipArchive::new(std::io::Cursor::new(source))?;
				let mut file = archive.by_name(name)?;
				let mut out = Vec::with_capacity(self.size as usize);
				std::io::Read::read_to_end(&mut file, &mut out)?;
				Ok(out)
			},
		}
	}
}

struct CentralEntry {
	name: String,
	meta: EntryMeta,
	method: u16,
	crc32: u32,
	compressed_size: u32,
	uncompressed_size: u32,
	local_header_offset: u32,
}

/// Sequential ZIP writer over any `Write` sink; only the central directory
/// (a few dozen bytes per entry) is retained until `finish`.
pub(crate) struct ArchiveWriter<W: Write> {
	output: W,
	offset: u64,
	entries: Vec<CentralEntry>,
}

impl<W: Write> ArchiveWriter<W> {
	pub(crate) fn new(output: W) -> Self {
		Self {
			output,
			offset: 0,
			entries: Vec::new(),
		}
	}

	/// Store `data` uncompressed (the EPUB `mimetype` entry).
	pub(crate) fn add_stored(
		&mut self,
		name: &str,
		meta: EntryMeta,
		data: &[u8],
	) -> Result<(), KepubError> {
		self.add_raw(
			name,
			meta,
			method_id(CompressionMethod::Stored),
			libdeflater::crc32(data),
			data.len() as u64,
			data,
		)
	}

	/// Deflate and add `data` on the calling thread.
	pub(crate) fn add_deflated(
		&mut self,
		name: &str,
		meta: EntryMeta,
		data: &[u8],
		level: u32,
	) -> Result<(), KepubError> {
		let deflated = Deflated::compress(name, meta, data, level)?;
		self.add_precompressed(&deflated)
	}

	/// Add an entry that was deflated elsewhere.
	pub(crate) fn add_precompressed(
		&mut self,
		entry: &Deflated,
	) -> Result<(), KepubError> {
		self.add_raw(
			&entry.name,
			entry.meta,
			method_id(CompressionMethod::Deflated),
			entry.crc32,
			entry.uncompressed_size,
			&entry.data,
		)
	}

	/// Copy a source entry verbatim: its compressed bytes are sliced from the
	/// source archive by offset, so the entry is neither inflated nor
	/// re-deflated.
	pub(crate) fn copy_entry(
		&mut self,
		source: &[u8],
		file: &ZipFile<'_>,
	) -> Result<(), KepubError> {
		let start = file.data_start() as usize;
		let end = start + file.compressed_size() as usize;
		let compressed = source.get(start..end).ok_or_else(|| {
			KepubError::InvalidEpub(format!(
				"entry {} points outside the archive",
				file.name()
			))
		})?;
		self.add_raw(
			file.name(),
			EntryMeta::from_source(file),
			method_id(file.compression()),
			file.crc32(),
			file.size(),
			compressed,
		)
	}

	fn add_raw(
		&mut self,
		name: &str,
		meta: EntryMeta,
		method: u16,
		crc32: u32,
		uncompressed_size: u64,
		compressed: &[u8],
	) -> Result<(), KepubError> {
		let compressed_size = compressed.len() as u64;
		let local_header_offset = self.offset;
		if uncompressed_size > MAX_ENTRY_SIZE
			|| compressed_size > MAX_ENTRY_SIZE
			|| local_header_offset > MAX_ENTRY_SIZE
			|| name.len() > u16::MAX as usize
		{
			return Err(KepubError::InvalidEpub(format!(
				"entry {name} exceeds the ZIP32 limits"
			)));
		}
		let flags = if name.is_ascii() { 0 } else { UTF8_FLAG };
		let mut header = [0u8; 30];
		put_u32(&mut header[0..], LOCAL_HEADER_SIGNATURE);
		put_u16(&mut header[4..], VERSION_NEEDED);
		put_u16(&mut header[6..], flags);
		put_u16(&mut header[8..], method);
		put_u16(&mut header[10..], meta.modified.timepart());
		put_u16(&mut header[12..], meta.modified.datepart());
		put_u32(&mut header[14..], crc32);
		put_u32(&mut header[18..], compressed_size as u32);
		put_u32(&mut header[22..], uncompressed_size as u32);
		put_u16(&mut header[26..], name.len() as u16);
		put_u16(&mut header[28..], 0); // extra field length
		self.output.write_all(&header)?;
		self.output.write_all(name.as_bytes())?;
		self.output.write_all(compressed)?;
		self.offset += header.len() as u64 + name.len() as u64 + compressed_size;
		self.entries.push(CentralEntry {
			name: name.to_owned(),
			meta,
			method,
			crc32,
			compressed_size: compressed_size as u32,
			uncompressed_size: uncompressed_size as u32,
			local_header_offset: local_header_offset as u32,
		});
		Ok(())
	}

	/// Write the central directory and return the sink.
	pub(crate) fn finish(mut self) -> Result<W, KepubError> {
		let central_start = self.offset;
		if self.entries.len() > u16::MAX as usize {
			return Err(KepubError::InvalidEpub(
				"archive has more than 65535 entries".to_string(),
			));
		}
		let mut central = Vec::with_capacity(self.entries.len() * 64);
		for entry in &self.entries {
			let flags = if entry.name.is_ascii() { 0 } else { UTF8_FLAG };
			let external_attributes = entry.meta.unix_mode.map_or(0, |mode| mode << 16);
			let mut header = [0u8; 46];
			put_u32(&mut header[0..], CENTRAL_HEADER_SIGNATURE);
			put_u16(&mut header[4..], VERSION_MADE_BY);
			put_u16(&mut header[6..], VERSION_NEEDED);
			put_u16(&mut header[8..], flags);
			put_u16(&mut header[10..], entry.method);
			put_u16(&mut header[12..], entry.meta.modified.timepart());
			put_u16(&mut header[14..], entry.meta.modified.datepart());
			put_u32(&mut header[16..], entry.crc32);
			put_u32(&mut header[20..], entry.compressed_size);
			put_u32(&mut header[24..], entry.uncompressed_size);
			put_u16(&mut header[28..], entry.name.len() as u16);
			// extra (30), comment (32), disk (34), internal attrs (36) stay 0
			put_u32(&mut header[38..], external_attributes);
			put_u32(&mut header[42..], entry.local_header_offset);
			central.extend_from_slice(&header);
			central.extend_from_slice(entry.name.as_bytes());
		}
		let central_size = central.len() as u64;
		if central_start > MAX_ENTRY_SIZE || central_size > MAX_ENTRY_SIZE {
			return Err(KepubError::InvalidEpub(
				"archive exceeds the ZIP32 limits".to_string(),
			));
		}
		let mut end = [0u8; 22];
		put_u32(&mut end[0..], END_OF_CENTRAL_DIRECTORY_SIGNATURE);
		// this disk (4) and central directory disk (6) stay 0
		put_u16(&mut end[8..], self.entries.len() as u16);
		put_u16(&mut end[10..], self.entries.len() as u16);
		put_u32(&mut end[12..], central_size as u32);
		put_u32(&mut end[16..], central_start as u32);
		// comment length (20) stays 0
		self.output.write_all(&central)?;
		self.output.write_all(&end)?;
		self.output.flush()?;
		Ok(self.output)
	}
}

/// Wire identifier of a compression method (needed to copy entries of any
/// method verbatim); `zip` deprecates the accessor in favour of matching on
/// known constants, which cannot express an arbitrary source method.
#[allow(deprecated)]
fn method_id(method: CompressionMethod) -> u16 {
	method.to_u16()
}

fn put_u16(out: &mut [u8], value: u16) {
	out[..2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut [u8], value: u32) {
	out[..4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
	use std::io::{Cursor, Read};

	use zip::ZipArchive;

	use super::*;

	#[test]
	fn round_trips_stored_deflated_and_copied_entries() {
		let mut writer = ArchiveWriter::new(Vec::new());
		writer
			.add_stored("mimetype", EntryMeta::default(), b"application/epub+zip")
			.unwrap();
		let body = "x".repeat(10_000);
		writer
			.add_deflated("OEBPS/a.xhtml", EntryMeta::default(), body.as_bytes(), 6)
			.unwrap();
		let first = writer.finish().unwrap();

		// Copy the deflated entry from the first archive into a second one.
		let mut source = ZipArchive::new(Cursor::new(first.as_slice())).unwrap();
		let mut writer = ArchiveWriter::new(Vec::new());
		{
			let file = source.by_name("OEBPS/a.xhtml").unwrap();
			writer.copy_entry(&first, &file).unwrap();
		}
		writer
			.add_stored("naïve.txt", EntryMeta::default(), b"utf8 name")
			.unwrap();
		let second = writer.finish().unwrap();

		let mut archive = ZipArchive::new(Cursor::new(second.as_slice())).unwrap();
		assert_eq!(archive.len(), 2);
		let mut copied = String::new();
		archive
			.by_name("OEBPS/a.xhtml")
			.unwrap()
			.read_to_string(&mut copied)
			.unwrap();
		assert_eq!(copied, body);
		let mut named = String::new();
		archive
			.by_name("naïve.txt")
			.unwrap()
			.read_to_string(&mut named)
			.unwrap();
		assert_eq!(named, "utf8 name");
		assert!(second.len() < body.len());
	}

	#[test]
	fn preserves_unix_mode_and_stored_method() {
		let meta = EntryMeta {
			modified: DateTime::default(),
			unix_mode: Some(0o644),
		};
		let mut writer = ArchiveWriter::new(Vec::new());
		writer
			.add_stored("mimetype", meta, b"application/epub+zip")
			.unwrap();
		let bytes = writer.finish().unwrap();
		let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
		let file = archive.by_index(0).unwrap();
		assert_eq!(file.compression(), CompressionMethod::Stored);
		assert_eq!(file.unix_mode(), Some(0o644));
		assert_eq!(file.data_start(), 30 + "mimetype".len() as u64);
	}
}
