//! Helpers shared by every tool: page detection, CBZ writing, atomic output,
//! `ComicInfo.xml` generation, and deterministic directory walks.
//!
//! These exist so the tools agree with the rest of Stump: page detection uses
//! the scanner's own predicates ([`stump_media::PathUtils`]), page order uses
//! the processors' natural sort (`alphanumeric_sort`), and CBZ output matches
//! the containers Stump already writes — `ComicInfo.xml` first, stored
//! (uncompressed) root-level pages named `0001.ext`
//! (`crates/provider/src/host.rs:601`,
//! `crates/media/src/transform/container.rs:58`).

use std::{
	fs::File,
	io::{Seek, Write},
	path::{Path, PathBuf},
};

use stump_media::{ContentType, PathUtils};
use zip::{write::SimpleFileOptions, CompressionMethod};

use crate::{ToolError, ToolResult};

/// The one entry name Stump's processors look for when importing metadata.
pub const COMIC_INFO_ENTRY: &str = "ComicInfo.xml";

/// True when `path` is a page image: an image by extension, not hidden, not a
/// `__MACOSX` resource fork. Mirrors the archive page rules in
/// `crates/media/src/transform/source.rs:237`.
pub fn is_page_file(path: &Path) -> bool {
	!path.is_hidden_file()
		&& path.is_img()
		&& path.naive_content_type() != ContentType::UNKNOWN
}

/// The archive entry name of page `index` (0-based) with `extension`.
///
/// 1-based and zero-padded to four digits so the entries sort in reading order
/// under every client's natural sort, matching `ProviderHost::build_archive`.
pub fn page_entry_name(index: usize, extension: &str) -> String {
	format!("{:04}.{extension}", index + 1)
}

/// The extension to use for a page of `content_type`, falling back to the
/// source name's extension and finally `jpg` (as `ProviderHost` does for
/// remote pages with an unusable content type).
pub fn page_extension(content_type: ContentType, name_hint: Option<&str>) -> String {
	let from_type = content_type.extension();
	if !from_type.is_empty() {
		return from_type.to_string();
	}

	let from_hint = name_hint
		.and_then(|name| Path::new(name).extension())
		.map(|ext| ext.to_string_lossy().to_lowercase())
		.filter(|ext| !ext.is_empty());

	from_hint.unwrap_or_else(|| "jpg".to_string())
}

/// Every file under `root`, depth-first, each directory's entries in natural
/// (alphanumeric) order. Directories are not returned.
pub fn sorted_files(root: &Path, recursive: bool) -> ToolResult<Vec<PathBuf>> {
	let mut files = Vec::new();
	collect_files(root, recursive, &mut files)?;
	Ok(files)
}

fn collect_files(
	dir: &Path,
	recursive: bool,
	files: &mut Vec<PathBuf>,
) -> ToolResult<()> {
	let mut entries = std::fs::read_dir(dir)?
		.collect::<Result<Vec<_>, _>>()?
		.into_iter()
		.map(|entry| {
			(
				entry.file_name().to_string_lossy().into_owned(),
				entry.path(),
			)
		})
		.collect::<Vec<_>>();
	entries.sort_by(|(a, _), (b, _)| alphanumeric_sort::compare_str(a, b));

	for (_, path) in entries {
		if path.is_dir() {
			if recursive {
				collect_files(&path, recursive, files)?;
			}
		} else {
			files.push(path);
		}
	}

	Ok(())
}

/// The immediate subdirectories of `dir` in natural order.
pub fn sorted_dirs(dir: &Path) -> ToolResult<Vec<PathBuf>> {
	let mut dirs = std::fs::read_dir(dir)?
		.collect::<Result<Vec<_>, _>>()?
		.into_iter()
		.map(|entry| {
			(
				entry.file_name().to_string_lossy().into_owned(),
				entry.path(),
			)
		})
		.filter(|(_, path)| path.is_dir())
		.collect::<Vec<_>>();
	dirs.sort_by(|(a, _), (b, _)| alphanumeric_sort::compare_str(a, b));

	Ok(dirs.into_iter().map(|(_, path)| path).collect())
}

/// Replace characters that are illegal in a Windows/macOS/Linux file name with
/// `_`, collapse whitespace, and never return an empty stem.
///
/// Same character set as `crates/provider/src/host.rs:706`, so tool output and
/// materialised provider downloads are named by one rule.
pub fn sanitize_file_stem(stem: &str) -> String {
	let cleaned = stem
		.chars()
		.map(|c| match c {
			'/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
			c if c.is_control() => ' ',
			c => c,
		})
		.collect::<String>();

	let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
	let trimmed = collapsed.trim_end_matches('.').trim();

	if trimmed.is_empty() {
		"untitled".to_string()
	} else {
		trimmed.to_string()
	}
}

/// Write `target` through a sibling temp file that is renamed into place, so a
/// crash or error never leaves a half-written archive (and never touches the
/// original until the new bytes are complete).
///
/// The closure receives the temp file itself: it is `Write + Seek`, so a
/// `ZipWriter` can be built straight on top of it.
pub fn write_atomic<T, F>(target: &Path, write: F) -> ToolResult<T>
where
	F: FnOnce(&mut File) -> ToolResult<T>,
{
	let parent = target.parent().ok_or_else(|| {
		ToolError::Invalid(format!("target has no parent directory: {target:?}"))
	})?;
	if !parent.as_os_str().is_empty() {
		std::fs::create_dir_all(parent)?;
	}

	let mut temp = tempfile::Builder::new()
		.prefix(".stump-tools-")
		.suffix(".tmp")
		.tempfile_in(if parent.as_os_str().is_empty() {
			Path::new(".")
		} else {
			parent
		})?;

	let value = write(temp.as_file_mut())?;
	temp.as_file_mut().sync_all()?;
	temp.persist(target)
		.map_err(|error| ToolError::Io(error.error))?;

	Ok(value)
}

/// Write a stored CBZ into `sink`: `ComicInfo.xml` first when provided, then
/// every page in iteration order. Returns the number of pages written.
///
/// Pages are `(entry name, bytes)` results so callers can stream: only the
/// page being written is held in memory.
pub fn write_cbz<W, I>(sink: W, comic_info: Option<&[u8]>, pages: I) -> ToolResult<usize>
where
	W: Write + Seek,
	I: IntoIterator<Item = ToolResult<(String, Vec<u8>)>>,
{
	let mut zip = zip::ZipWriter::new(sink);
	let options =
		SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

	if let Some(comic_info) = comic_info {
		zip.start_file(COMIC_INFO_ENTRY, options)?;
		zip.write_all(comic_info)?;
	}

	let mut written = 0;
	for page in pages {
		let (name, bytes) = page?;
		zip.start_file(name, options)?;
		zip.write_all(&bytes)?;
		written += 1;
	}

	zip.finish()?;

	Ok(written)
}

/// [`write_cbz`] into `target` atomically (temp file + rename).
pub fn write_cbz_file<I>(
	target: &Path,
	comic_info: Option<&[u8]>,
	pages: I,
) -> ToolResult<usize>
where
	I: IntoIterator<Item = ToolResult<(String, Vec<u8>)>>,
{
	write_atomic(target, |file| write_cbz(file, comic_info, pages))
}

/// Where a tool writes a *derived* copy of `source`: `output_dir` when it is
/// given and is not the source's own directory, otherwise the source's
/// directory with `suffix` appended to the stem.
///
/// The suffix is what keeps a sibling output from being its own source. The
/// tools' default suffixes are bracketed or parenthesised, which
/// [`stump_scanner::clean_name`] strips, so a derived file still identifies as
/// the same series/volume/chapter during a scan.
pub fn derived_target(
	source: &Path,
	output_dir: Option<&Path>,
	suffix: &str,
	extension: &str,
) -> ToolResult<PathBuf> {
	let stem = source
		.file_stem()
		.map(|stem| stem.to_string_lossy().into_owned())
		.ok_or_else(|| {
			ToolError::Invalid(format!("{} has no file name", source.display()))
		})?;
	let parent = source.parent().unwrap_or_else(|| Path::new(""));
	let directory = output_dir.unwrap_or(parent);
	let stem = if same_directory(directory, parent) {
		format!("{stem}{suffix}")
	} else {
		stem
	};

	Ok(directory.join(format!("{stem}.{extension}")))
}

/// True when both paths name the same directory. Canonical paths are compared
/// when both resolve, so `.` and its absolute form are not read as two
/// different output directories.
fn same_directory(a: &Path, b: &Path) -> bool {
	match (a.canonicalize(), b.canonicalize()) {
		(Ok(a), Ok(b)) => a == b,
		_ => a == b,
	}
}

/// Replace — or insert — exactly one entry of a zip container in place,
/// atomically.
///
/// Every other member is written back with its original compressed bytes
/// (`raw_copy_file`), so a metadata edit provably cannot alter a page, a
/// content document, a font or a style sheet. A replaced entry keeps its
/// position and its unix mode; a new entry is written first, where Stump's own
/// containers put `ComicInfo.xml` and where an EPUB's `mimetype` already is.
pub fn replace_archive_entry(
	path: &Path,
	entry: &str,
	bytes: &[u8],
	compression: CompressionMethod,
) -> ToolResult<()> {
	let permissions = std::fs::metadata(path).map(|meta| meta.permissions()).ok();
	let mut archive = zip::ZipArchive::new(File::open(path)?)?;
	let existed = archive.index_for_name(entry).is_some();
	let options = SimpleFileOptions::default().compression_method(compression);

	write_atomic(path, |sink| {
		let mut writer = zip::ZipWriter::new(sink);
		if !existed {
			writer.start_file(entry, options.unix_permissions(0o644))?;
			writer.write_all(bytes)?;
		}

		for index in 0..archive.len() {
			let member = archive.by_index_raw(index)?;
			if member.name() == entry {
				let mode = member.unix_mode().unwrap_or(0o644);
				writer.start_file(entry, options.unix_permissions(mode))?;
				writer.write_all(bytes)?;
				continue;
			}
			writer.raw_copy_file(member)?;
		}

		writer.finish()?;
		Ok(())
	})?;

	// `write_atomic` stages through a 0600 temp file, so an in-place rewrite
	// has to put the library file's own mode back.
	if let Some(permissions) = permissions {
		let _ = std::fs::set_permissions(path, permissions);
	}

	Ok(())
}

/// The `ComicInfo.xml` fields Stump's tools generate.
///
/// Element names and order follow the ComicInfo v2.0 schema
/// (<https://anansi-project.github.io/docs/comicinfo/schemas/v2.0>); only the
/// fields a tool can know from its input are emitted, and empty fields are
/// omitted entirely rather than written as empty elements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComicInfo {
	pub title: Option<String>,
	pub series: Option<String>,
	pub number: Option<String>,
	pub volume: Option<i32>,
	pub summary: Option<String>,
	pub year: Option<i32>,
	pub month: Option<i32>,
	pub day: Option<i32>,
	pub writers: Vec<String>,
	pub publisher: Option<String>,
	/// Free-form tags, written as one comma-separated `<Tags>` element.
	pub tags: Vec<String>,
	pub page_count: Option<usize>,
	/// BCP 47 language tag, written as `<LanguageISO>`.
	pub language: Option<String>,
}

impl ComicInfo {
	/// True when nothing but a page count is known, i.e. the file would carry
	/// no metadata worth writing.
	pub fn is_empty(&self) -> bool {
		self.title.is_none()
			&& self.series.is_none()
			&& self.number.is_none()
			&& self.volume.is_none()
			&& self.summary.is_none()
			&& self.year.is_none()
			&& self.month.is_none()
			&& self.day.is_none()
			&& self.writers.is_empty()
			&& self.publisher.is_none()
			&& self.tags.is_empty()
			&& self.language.is_none()
	}

	/// Serialize to `ComicInfo.xml`.
	pub fn to_xml(&self) -> String {
		let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
		xml.push_str(
			"<ComicInfo xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
		);

		push_text(&mut xml, "Title", self.title.as_deref());
		push_text(&mut xml, "Series", self.series.as_deref());
		push_text(&mut xml, "Number", self.number.as_deref());
		push_number(&mut xml, "Volume", self.volume);
		push_text(&mut xml, "Summary", self.summary.as_deref());
		push_number(&mut xml, "Year", self.year);
		push_number(&mut xml, "Month", self.month);
		push_number(&mut xml, "Day", self.day);
		if !self.writers.is_empty() {
			push_text(&mut xml, "Writer", Some(&self.writers.join(", ")));
		}
		push_text(&mut xml, "Publisher", self.publisher.as_deref());
		if !self.tags.is_empty() {
			push_text(&mut xml, "Tags", Some(&self.tags.join(", ")));
		}
		push_number(&mut xml, "PageCount", self.page_count.map(|c| c as i32));
		push_text(&mut xml, "LanguageISO", self.language.as_deref());

		xml.push_str("</ComicInfo>");
		xml
	}
}

fn push_text(xml: &mut String, element: &str, value: Option<&str>) {
	let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
		return;
	};
	xml.push_str("  <");
	xml.push_str(element);
	xml.push('>');
	xml.push_str(&quick_xml::escape::escape(value));
	xml.push_str("</");
	xml.push_str(element);
	xml.push_str(">\n");
}

fn push_number(xml: &mut String, element: &str, value: Option<i32>) {
	if let Some(value) = value {
		xml.push_str(&format!("  <{element}>{value}</{element}>\n"));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Cursor;

	#[test]
	fn page_names_sort_in_reading_order() {
		let mut names = (0..12)
			.map(|index| page_entry_name(index, "jpg"))
			.collect::<Vec<_>>();
		let expected = names.clone();

		names.sort();
		assert_eq!(names, expected, "zero-padded names must sort naturally");
		assert_eq!(names[0], "0001.jpg");
		assert_eq!(names[9], "0010.jpg");
	}

	#[test]
	fn page_detection_skips_hidden_and_non_images() {
		assert!(is_page_file(Path::new("/books/series/001.jpg")));
		assert!(is_page_file(Path::new("/books/series/page.WEBP")));
		assert!(!is_page_file(Path::new("/books/series/.001.jpg")));
		assert!(!is_page_file(Path::new("/books/__MACOSX/._001.jpg")));
		assert!(!is_page_file(Path::new("/books/series/notes.txt")));
		assert!(!is_page_file(Path::new("/books/series/ComicInfo.xml")));
	}

	#[test]
	fn file_stems_lose_illegal_characters_but_keep_content() {
		assert_eq!(sanitize_file_stem("Berserk: v01"), "Berserk_ v01");
		assert_eq!(sanitize_file_stem("a/b\\c*d?e"), "a_b_c_d_e");
		assert_eq!(sanitize_file_stem("  spaced   out  "), "spaced out");
		assert_eq!(sanitize_file_stem("trailing dots..."), "trailing dots");
		assert_eq!(sanitize_file_stem("   "), "untitled");
	}

	#[test]
	fn comic_info_omits_unknown_fields_and_escapes_text() {
		let info = ComicInfo {
			title: Some("Tom & Jerry <1>".to_string()),
			series: Some("Tom & Jerry".to_string()),
			number: Some("1".to_string()),
			page_count: Some(3),
			language: Some("en".to_string()),
			..Default::default()
		};

		let xml = info.to_xml();
		assert!(
			xml.contains("<Title>Tom &amp; Jerry &lt;1&gt;</Title>"),
			"{xml}"
		);
		assert!(xml.contains("<PageCount>3</PageCount>"), "{xml}");
		assert!(xml.contains("<LanguageISO>en</LanguageISO>"), "{xml}");
		assert!(!xml.contains("<Summary"), "{xml}");
		assert!(!xml.contains("<Volume"), "{xml}");
		assert!(!info.is_empty());
		assert!(ComicInfo {
			page_count: Some(3),
			..Default::default()
		}
		.is_empty());
	}

	#[test]
	fn generated_comic_info_parses_as_stump_media_metadata() {
		let info = ComicInfo {
			title: Some("Chapter 1".to_string()),
			series: Some("Berserk".to_string()),
			number: Some("1.5".to_string()),
			volume: Some(3),
			summary: Some("A & B".to_string()),
			year: Some(1989),
			month: Some(8),
			day: Some(25),
			writers: vec!["Kentaro Miura".to_string()],
			publisher: Some("Hakusensha".to_string()),
			tags: vec!["dark fantasy".to_string(), "seinen".to_string()],
			page_count: Some(2),
			language: Some("ja".to_string()),
		};

		let parsed: stump_media::media::ProcessedMediaMetadata =
			quick_xml::de::from_str(&info.to_xml()).expect("Stump must parse our XML");
		assert_eq!(parsed.title.as_deref(), Some("Chapter 1"));
		assert_eq!(parsed.series.as_deref(), Some("Berserk"));
		assert_eq!(parsed.number, Some(1.5));
		assert_eq!(parsed.volume, Some(3));
		assert_eq!(parsed.summary.as_deref(), Some("A & B"));
		assert_eq!(
			(parsed.year, parsed.month, parsed.day),
			(Some(1989), Some(8), Some(25))
		);
		assert_eq!(parsed.writers, Some(vec!["Kentaro Miura".to_string()]));
		assert_eq!(parsed.page_count, Some(2));
		assert_eq!(parsed.publisher.as_deref(), Some("Hakusensha"));
		assert_eq!(
			parsed.tags,
			Some(vec!["dark fantasy".to_string(), "seinen".to_string()])
		);
	}

	#[test]
	fn cbz_is_stored_with_comic_info_first() {
		let mut buffer = Cursor::new(Vec::new());
		let pages = (0..3)
			.map(|index| Ok((page_entry_name(index, "png"), vec![index as u8; 4])))
			.collect::<Vec<_>>();

		let written = write_cbz(&mut buffer, Some(b"<ComicInfo/>"), pages).unwrap();
		assert_eq!(written, 3);

		// `ZipArchive::file_names` iterates a hash map, so entry ORDER can only
		// be read back by index.
		let mut archive = zip::ZipArchive::new(buffer).unwrap();
		assert_eq!(archive.len(), 4);
		let names = (0..archive.len())
			.map(|index| archive.by_index(index).unwrap().name().to_owned())
			.collect::<Vec<_>>();
		assert_eq!(
			names,
			vec![COMIC_INFO_ENTRY, "0001.png", "0002.png", "0003.png"]
		);
		assert_eq!(
			archive.by_index(1).unwrap().compression(),
			CompressionMethod::Stored
		);
	}

	#[test]
	fn write_atomic_leaves_no_temp_file_and_keeps_the_original_on_error() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("out.cbz");

		write_atomic(&target, |file| Ok(file.write_all(b"good")?)).unwrap();
		assert_eq!(std::fs::read(&target).unwrap(), b"good");

		let error = write_atomic::<(), _>(&target, |_| {
			Err(ToolError::Invalid("boom".to_string()))
		})
		.unwrap_err();
		assert!(matches!(error, ToolError::Invalid(_)), "{error}");
		assert_eq!(
			std::fs::read(&target).unwrap(),
			b"good",
			"a failed write must not damage the existing file"
		);
		assert_eq!(
			sorted_files(dir.path(), false).unwrap(),
			vec![target],
			"no temp file may survive"
		);
	}

	#[test]
	fn sorted_files_is_natural_and_depth_first() {
		let dir = tempfile::tempdir().unwrap();
		let nested = dir.path().join("ch2");
		std::fs::create_dir(&nested).unwrap();
		for name in ["10.jpg", "2.jpg", "1.jpg"] {
			std::fs::write(dir.path().join(name), b"x").unwrap();
			std::fs::write(nested.join(name), b"x").unwrap();
		}

		let shallow = sorted_files(dir.path(), false).unwrap();
		let names = shallow
			.iter()
			.map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
			.collect::<Vec<_>>();
		assert_eq!(names, vec!["1.jpg", "2.jpg", "10.jpg"]);

		// Depth-first in one natural order: a subdirectory's files appear where
		// its own name sorts, so digits come before the `ch2` directory.
		let deep = sorted_files(dir.path(), true).unwrap();
		let relative = deep
			.iter()
			.map(|path| {
				path.strip_prefix(dir.path())
					.unwrap()
					.to_string_lossy()
					.into_owned()
			})
			.collect::<Vec<_>>();
		assert_eq!(
			relative,
			vec![
				"1.jpg",
				"2.jpg",
				"10.jpg",
				"ch2/1.jpg",
				"ch2/2.jpg",
				"ch2/10.jpg",
			]
		);
		assert_eq!(sorted_dirs(dir.path()).unwrap(), vec![nested]);
	}

	#[test]
	fn a_derived_target_only_takes_the_suffix_beside_its_source() {
		let dir = tempfile::tempdir().unwrap();
		let source = dir.path().join("Berserk v01.cbz");
		std::fs::write(&source, b"cbz").unwrap();
		let elsewhere = dir.path().join("out");
		std::fs::create_dir(&elsewhere).unwrap();

		// Beside the source the suffix is what keeps the output from being its
		// own input.
		assert_eq!(
			derived_target(&source, None, " [webp]", "cbz").unwrap(),
			dir.path().join("Berserk v01 [webp].cbz")
		);
		// In another directory the name is kept as it is.
		assert_eq!(
			derived_target(&source, Some(&elsewhere), " [webp]", "cbz").unwrap(),
			elsewhere.join("Berserk v01.cbz")
		);
		// An output directory that only *spells* the source's differently is
		// still the source's directory.
		let same = dir.path().join(".");
		assert_eq!(
			derived_target(&source, Some(&same), " (polished)", "epub").unwrap(),
			same.join("Berserk v01 (polished).epub")
		);
	}

	#[test]
	fn replacing_one_entry_keeps_every_other_byte_and_its_position() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("book.cbz");
		let pages = [
			("0001.jpg", b"page-one".to_vec()),
			("0002.jpg", b"two".to_vec()),
		];
		write_atomic(&path, |file| {
			write_cbz(
				file,
				Some(b"<ComicInfo><Series>Old</Series></ComicInfo>"),
				pages
					.iter()
					.map(|(name, bytes)| Ok((name.to_string(), bytes.clone()))),
			)
			.map(|_| ())
		})
		.unwrap();

		replace_archive_entry(
			&path,
			COMIC_INFO_ENTRY,
			b"<ComicInfo><Series>New</Series></ComicInfo>",
			CompressionMethod::Stored,
		)
		.unwrap();

		let entries = read_all(&path);
		assert_eq!(
			entries
				.iter()
				.map(|(name, _)| name.as_str())
				.collect::<Vec<_>>(),
			vec![COMIC_INFO_ENTRY, "0001.jpg", "0002.jpg"],
			"a replaced entry keeps its position"
		);
		assert_eq!(entries[0].1, b"<ComicInfo><Series>New</Series></ComicInfo>");
		assert_eq!(entries[1].1, b"page-one");
		assert_eq!(entries[2].1, b"two");

		// A new entry is written first, where Stump's containers keep metadata.
		replace_archive_entry(&path, "extra.txt", b"note", CompressionMethod::Stored)
			.unwrap();
		let entries = read_all(&path);
		assert_eq!(
			entries
				.iter()
				.map(|(name, _)| name.as_str())
				.collect::<Vec<_>>(),
			vec!["extra.txt", COMIC_INFO_ENTRY, "0001.jpg", "0002.jpg"]
		);
		assert_eq!(entries[2].1, b"page-one");
	}

	fn read_all(path: &Path) -> Vec<(String, Vec<u8>)> {
		use std::io::Read;

		let mut archive =
			zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
		(0..archive.len())
			.map(|index| {
				let mut entry = archive.by_index(index).unwrap();
				let name = entry.name().to_string();
				let mut bytes = Vec::new();
				entry.read_to_end(&mut bytes).unwrap();
				(name, bytes)
			})
			.collect()
	}
}
