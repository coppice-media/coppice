//! Source-side page extraction for comic containers.
//!
//! Produces the `Iterator<Item = TransformResult<Vec<u8>>>` that [`super::transform_pages`]
//! consumes: ZIP/CBZ pages stream straight out of the archive in natural page
//! order; PDF pages are rendered through the `pdf` feature's processor; RAR
//! pages through the `rar` feature's processor.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::content_type::ContentType;
#[cfg(any(feature = "pdf", feature = "rar"))]
use crate::error::FileError;
#[cfg(any(feature = "pdf", feature = "rar"))]
use crate::FileProcessor;
use crate::{MediaConfig, PathUtils};

use super::error::{TransformError, TransformResult};

/// The extensions understood as transformable comic sources.
pub const COMIC_EXTENSIONS: [&str; 5] = ["cbz", "zip", "cbr", "rar", "pdf"];

/// Whether a media path looks like a transformable comic container.
pub fn is_comic_source(path: &str) -> bool {
	Path::new(path)
		.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| {
			COMIC_EXTENSIONS
				.iter()
				.any(|candidate| candidate.eq_ignore_ascii_case(extension))
		})
}

/// The ordered page bytes of a comic container.
#[derive(Debug)]
pub struct ComicPages {
	source: PathBuf,
	kind: ComicKind,
	page_count: usize,
}

#[derive(Debug)]
enum ComicKind {
	Zip(Vec<String>),
	#[cfg(feature = "pdf")]
	Pdf,
	#[cfg(feature = "rar")]
	Rar,
}

impl ComicPages {
	/// Open `path` and enumerate its pages.
	///
	/// `media_config` supplies the PDFium path and render settings for PDF
	/// sources.
	pub fn open(path: &Path, media_config: &MediaConfig) -> TransformResult<Self> {
		#[cfg(not(any(feature = "pdf", feature = "rar")))]
		let _ = media_config;
		let extension = path
			.extension()
			.and_then(|extension| extension.to_str())
			.map(str::to_ascii_lowercase)
			.ok_or_else(|| {
				TransformError::UnsupportedSource(path.display().to_string())
			})?;

		match extension.as_str() {
			"cbz" | "zip" => {
				let names = zip_page_names(path)?;
				let page_count = names.len();
				Ok(Self {
					source: path.to_path_buf(),
					kind: ComicKind::Zip(names),
					page_count,
				})
			},
			#[cfg(feature = "pdf")]
			"pdf" => {
				let page_count = crate::media::format::pdf::PdfProcessor::get_page_count(
					&path.to_string_lossy(),
					media_config,
				)
				.map_err(archive_error)?
				.max(0) as usize;
				Ok(Self {
					source: path.to_path_buf(),
					kind: ComicKind::Pdf,
					page_count,
				})
			},
			#[cfg(not(feature = "pdf"))]
			"pdf" => Err(TransformError::FeatureDisabled("PDF")),
			#[cfg(feature = "rar")]
			"cbr" | "rar" => {
				let page_count = crate::media::format::rar::RarProcessor::get_page_count(
					&path.to_string_lossy(),
					media_config,
				)
				.map_err(archive_error)?
				.max(0) as usize;
				Ok(Self {
					source: path.to_path_buf(),
					kind: ComicKind::Rar,
					page_count,
				})
			},
			#[cfg(not(feature = "rar"))]
			"cbr" | "rar" => Err(TransformError::FeatureDisabled("RAR")),
			other => Err(TransformError::UnsupportedSource(format!(
				".{other} is not a comic container"
			))),
		}
	}

	/// Number of source pages (before any tall-page splitting).
	pub fn page_count(&self) -> usize {
		self.page_count
	}

	/// Consume into the page iterator consumed by the pipeline.
	pub fn into_iter(
		self,
		media_config: &MediaConfig,
	) -> Box<dyn Iterator<Item = TransformResult<Vec<u8>>> + Send> {
		#[cfg(not(any(feature = "pdf", feature = "rar")))]
		let _ = media_config;
		match self.kind {
			ComicKind::Zip(names) => Box::new(ZipPageIter {
				archive: None,
				path: self.source,
				names,
				position: 0,
			}),
			#[cfg(feature = "pdf")]
			ComicKind::Pdf => {
				let path = self.source.to_string_lossy().into_owned();
				let media_config = media_config.clone();
				Box::new((0..self.page_count).map(move |index| {
					crate::media::format::pdf::PdfProcessor::get_page(
						&path,
						(index + 1) as i32,
						&media_config,
					)
					.map(|(_, bytes)| bytes)
					.map_err(archive_error)
				}))
			},
			#[cfg(feature = "rar")]
			ComicKind::Rar => {
				let path = self.source.to_string_lossy().into_owned();
				let media_config = media_config.clone();
				Box::new((0..self.page_count).map(move |index| {
					crate::media::format::rar::RarProcessor::get_page(
						&path,
						(index + 1) as i32,
						&media_config,
					)
					.map(|(_, bytes)| bytes)
					.map_err(archive_error)
				}))
			},
		}
	}
}

/// Sort-then-stream iterator over a ZIP/CBZ's image entries.
///
/// The archive is opened lazily on the first pull so construction stays cheap.
struct ZipPageIter {
	archive: Option<zip::ZipArchive<std::fs::File>>,
	path: PathBuf,
	names: Vec<String>,
	position: usize,
}

impl ZipPageIter {
	fn archive(&mut self) -> TransformResult<&mut zip::ZipArchive<std::fs::File>> {
		if self.archive.is_none() {
			let file = std::fs::File::open(&self.path)?;
			self.archive = Some(zip::ZipArchive::new(file)?);
		}
		Ok(self.archive.as_mut().expect("archive just initialized"))
	}
}

impl Iterator for ZipPageIter {
	type Item = TransformResult<Vec<u8>>;

	fn next(&mut self) -> Option<Self::Item> {
		let name = self.names.get(self.position)?.clone();
		self.position += 1;

		let result = self.archive().and_then(|archive| {
			let mut entry = archive.by_name(name.as_str())?;
			let mut contents = Vec::with_capacity(entry.size() as usize);
			entry.read_to_end(&mut contents)?;
			Ok(contents)
		});
		Some(result)
	}
}

/// Enumerate the archive's image entries in delivery order, skipping
/// directories, hidden files (including `__MACOSX`), and `ComicInfo.xml`.
fn zip_page_names(path: &Path) -> TransformResult<Vec<String>> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file)?;

	if archive.is_empty() {
		return Err(TransformError::Archive(
			"archive contains no entries".to_string(),
		));
	}

	let mut names: Vec<String> = archive.file_names().map(str::to_owned).collect();
	crate::media::utils::sort_file_names(&mut names);

	let mut pages = Vec::new();
	for name in names {
		let entry_path = match archive.by_name(&name) {
			Ok(entry) => {
				if entry.is_dir() {
					continue;
				}
				entry
					.enclosed_name()
					.map(|path| path.to_path_buf())
					.unwrap_or_else(|| Path::new(&name).to_path_buf())
			},
			Err(_) => continue,
		};

		if entry_path.is_hidden_file() {
			continue;
		}
		let content_type = entry_path.naive_content_type();
		if !content_type.is_image() || content_type == ContentType::UNKNOWN {
			continue;
		}
		pages.push(name);
	}

	Ok(pages)
}

/// Map a [`FileError`] from the format processors onto the transform error.
#[cfg(any(feature = "pdf", feature = "rar"))]
fn archive_error(error: FileError) -> TransformError {
	match error {
		FileError::FileIoError(io) => TransformError::Io(io),
		FileError::ZipFileError(zip) => TransformError::Archive(zip.to_string()),
		other => TransformError::Other(other.to_string()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	#[test]
	fn comic_source_detection_by_extension() {
		assert!(is_comic_source("/books/a.cbz"));
		assert!(is_comic_source("/books/a.CBZ"));
		assert!(is_comic_source("/books/a.zip"));
		assert!(is_comic_source("/books/a.cbr"));
		assert!(is_comic_source("/books/a.rar"));
		assert!(is_comic_source("/books/a.pdf"));
		assert!(!is_comic_source("/books/a.epub"));
		assert!(!is_comic_source("/books/a.kepub.epub"));
		assert!(!is_comic_source("/books/a"));
	}

	/// Write a CBZ whose entries are deliberately out of order and padded with
	/// everything the page enumeration must skip.
	fn scrambled_cbz() -> tempfile::NamedTempFile {
		let file = tempfile::NamedTempFile::with_suffix(".cbz").unwrap();
		{
			let mut zip = zip::ZipWriter::new(file.as_file());
			let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
			for (name, contents) in [
				("book/10.png", &b"page-10"[..]),
				("book/2.png", b"page-2"),
				("ComicInfo.xml", b"<ComicInfo/>"),
				("__MACOSX/book/._1.png", b"resource-fork"),
				("book/.hidden.png", b"hidden"),
				("book/notes.txt", b"not a page"),
				("book/1.png", b"page-1"),
			] {
				zip.start_file(name, options).unwrap();
				zip.write_all(contents).unwrap();
			}
			zip.add_directory("book/empty", options).unwrap();
			zip.finish().unwrap();
		}
		file
	}

	#[test]
	fn zip_source_yields_only_image_entries_in_natural_order() {
		let cbz = scrambled_cbz();
		let config = MediaConfig::default();
		let comic = ComicPages::open(cbz.path(), &config).expect("cbz opens");
		assert_eq!(comic.page_count(), 3);

		let pages: Vec<Vec<u8>> = comic
			.into_iter(&config)
			.collect::<TransformResult<_>>()
			.expect("every page reads");
		assert_eq!(
			pages,
			[b"page-1".to_vec(), b"page-2".to_vec(), b"page-10".to_vec()]
		);
	}

	#[test]
	fn zip_source_skips_macos_shadow_entries() {
		// Three real pages plus `__MACOSX` shadows, `.DS_Store`s, and directories.
		let path = crate::tests::get_nested_macos_compressed_cbz_path();
		let config = MediaConfig::default();
		let comic = ComicPages::open(Path::new(&path), &config).expect("cbz opens");
		assert_eq!(comic.page_count(), 3);

		let pages: Vec<Vec<u8>> = comic
			.into_iter(&config)
			.collect::<TransformResult<_>>()
			.expect("every page reads");
		assert_eq!(pages.len(), 3);
		for page in &pages {
			assert!(page.starts_with(&[0xFF, 0xD8, 0xFF]), "pages are the JPEGs");
		}
	}

	#[test]
	fn zip_source_without_images_has_no_pages() {
		// Only `.ico` files and metadata: nothing a reader could show.
		let path = crate::tests::get_test_complex_zip_path();
		let config = MediaConfig::default();
		let comic = ComicPages::open(Path::new(&path), &config).expect("zip opens");
		assert_eq!(comic.page_count(), 0);
		assert_eq!(comic.into_iter(&config).count(), 0);
	}

	#[test]
	fn unknown_extension_is_unsupported() {
		let config = MediaConfig::default();
		let error = ComicPages::open(Path::new("/books/a.epub"), &config).unwrap_err();
		assert!(matches!(error, TransformError::UnsupportedSource(_)));
	}

	#[cfg(not(feature = "pdf"))]
	#[test]
	fn pdf_source_is_disabled_without_the_feature() {
		let config = MediaConfig::default();
		let error = ComicPages::open(Path::new("/books/a.pdf"), &config).unwrap_err();
		assert!(matches!(error, TransformError::FeatureDisabled("PDF")));
	}

	#[cfg(feature = "pdf")]
	#[test]
	fn pdf_source_renders_every_page() {
		if crate::media::format::pdf::PdfProcessor::renderer(&None).is_err() {
			eprintln!("Skipping test: PDFium is not configured or available.");
			return;
		}
		let config = MediaConfig::default();
		let pdf_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("integration-tests/data/tall.pdf");
		let comic = ComicPages::open(&pdf_path, &config).expect("pdf opens with pdfium");
		let page_count = comic.page_count();
		assert!(page_count > 0);

		let pages: Vec<Vec<u8>> = comic
			.into_iter(&config)
			.collect::<TransformResult<_>>()
			.expect("every page renders");
		assert_eq!(pages.len(), page_count);
		for page in &pages {
			image::load_from_memory(page).expect("rendered page decodes");
		}
	}

	#[cfg(not(feature = "rar"))]
	#[test]
	fn rar_source_is_disabled_without_the_feature() {
		let config = MediaConfig::default();
		let error = ComicPages::open(Path::new("/books/a.cbr"), &config).unwrap_err();
		assert!(matches!(error, TransformError::FeatureDisabled("RAR")));
	}

	#[cfg(feature = "rar")]
	#[test]
	fn rar_source_yields_only_image_entries_in_natural_order() {
		// `scrambled.cbr`: 10.jpg, 2.jpg, ComicInfo.xml, __MACOSX/._1.jpg, 1.jpg
		// in that archive order; the pages are 10, 20, and 30 px wide.
		let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("integration-tests/data/scrambled.cbr");
		let config = MediaConfig::default();
		let comic = ComicPages::open(&path, &config).expect("cbr opens");
		assert_eq!(comic.page_count(), 3);

		let widths: Vec<u32> = comic
			.into_iter(&config)
			.map(|page| {
				image::load_from_memory(&page.expect("page extracts"))
					.expect("extracted page decodes")
					.width()
			})
			.collect();
		assert_eq!(widths, [10, 20, 30]);
	}
}
