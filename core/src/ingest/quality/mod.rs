//! Deterministic quality checks for staged ingest items.
//!
//! Every check is deliberately limited to the immutable [`BookSnapshot`] and
//! persisted settings.  The one database-backed check (`duplicate_existing`)
//! only reads visible media rows and never mutates the database.

use std::{
	io::{self, ErrorKind, Read},
	path::{Component, Path},
	sync::LazyLock,
};

use data_encoding::HEXLOWER;
use image::DynamicImage;
use ring::digest::{Context, SHA256};
use serde_json::{json, Value};

use crate::{
	config::StumpConfig,
	ingest::contract::{
		BookSnapshot, IngestMediaKind, QualityCheckOutcome, QualityStatus,
		SettingDefinition, SettingKind, SettingValues,
	},
};
use stump_media::{media::get_page, EpubProcessor, FileError, PathUtils};

pub mod cover_not_page_two;
pub mod cover_present;
pub mod duplicate_existing;
pub mod epub_toc_chapters;
pub mod filename;
pub mod image_dimensions_consistent;
pub mod page_count_matches_archive_entries;
pub mod registry;

pub use cover_not_page_two::CoverNotPageTwoCheck;
pub use cover_present::CoverPresentCheck;
pub use duplicate_existing::DuplicateExistingCheck;
pub use epub_toc_chapters::EpubTocChaptersCheck;
pub use filename::{
	parse_filename, FilenameParseStatus, FilenameSeriesParseCheck, FilenameTuple,
	ParsedFilename,
};
pub use image_dimensions_consistent::ImageDimensionsConsistentCheck;
pub use page_count_matches_archive_entries::PageCountMatchesArchiveEntriesCheck;
pub use registry::{CheckDescriptor, QualityRegistry};

/// Version of the built-in check implementations.  Reports use the contract's
/// algorithm version; this implementation version is exposed in the catalog.
pub(crate) const QUALITY_VERSION: &str = "1";

static ENABLED_SETTINGS: LazyLock<Vec<SettingDefinition>> = LazyLock::new(|| {
	vec![SettingDefinition {
		key: "enabled",
		label: "Enabled",
		description: "Run this quality check during staged analysis",
		kind: SettingKind::Bool,
		default: Value::Bool(true),
		required: false,
		secret: false,
		help_url: None,
	}]
});

pub(crate) fn enabled_settings() -> &'static [SettingDefinition] {
	ENABLED_SETTINGS.as_slice()
}

pub(crate) fn enabled_setting(settings: &SettingValues) -> bool {
	settings
		.get("enabled")
		.and_then(Value::as_bool)
		.unwrap_or(true)
}

pub(crate) fn outcome(
	check_id: &str,
	label: &str,
	status: QualityStatus,
	normalized_score: f64,
	evidence: Value,
) -> QualityCheckOutcome {
	QualityCheckOutcome {
		check_id: check_id.to_string(),
		label: label.to_string(),
		status,
		normalized_score,
		evidence,
	}
}

pub(crate) fn disabled_outcome(check_id: &str, label: &str) -> QualityCheckOutcome {
	outcome(
		check_id,
		label,
		QualityStatus::NotApplicable,
		0.0,
		json!({"disabled": true}),
	)
}

#[derive(Debug)]
pub(crate) struct ArchiveEntry {
	pub path: String,
	pub is_image: bool,
	pub bytes: Option<Vec<u8>>,
}

/// Read a ZIP archive in the same natural path order as the existing ZIP
/// processor.  Only image entry bytes are retained; metadata and other entries
/// are represented by their path alone.
pub(crate) fn read_archive_entries(path: &Path) -> io::Result<Vec<ArchiveEntry>> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
	let mut names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
	alphanumeric_sort::sort_str_slice(&mut names);

	let mut entries = Vec::with_capacity(names.len());
	for name in names {
		let mut file = archive.by_name(&name).map_err(zip_error)?;
		if file.is_dir() {
			continue;
		}
		let entry_path = file
			.enclosed_name()
			.unwrap_or_else(|| std::path::PathBuf::from(&name));
		let path_string = entry_path.to_string_lossy().replace('\\', "/");
		let hidden = is_hidden_archive_path(&entry_path);
		let metadata = is_known_metadata(&entry_path);
		let is_image = !hidden && !metadata && entry_path.is_img();
		let bytes = if is_image {
			let mut contents = Vec::new();
			file.read_to_end(&mut contents)?;
			Some(contents)
		} else {
			None
		};
		entries.push(ArchiveEntry {
			path: path_string,
			is_image,
			bytes,
		});
	}
	Ok(entries)
}

pub(crate) fn archive_images(path: &Path) -> io::Result<Vec<(String, Vec<u8>)>> {
	Ok(read_archive_entries(path)?
		.into_iter()
		.filter_map(|entry| entry.bytes.map(|bytes| (entry.path, bytes)))
		.collect())
}

pub(crate) fn is_hidden_archive_path(path: &Path) -> bool {
	if path.is_hidden_file() {
		return true;
	}
	path.components().any(|component| match component {
		Component::Normal(name) => {
			let name = name.to_string_lossy();
			name.starts_with('.') || name.eq_ignore_ascii_case("__MACOSX")
		},
		_ => false,
	})
}

fn is_known_metadata(path: &Path) -> bool {
	path.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| {
			name.eq_ignore_ascii_case("ComicInfo.xml")
				|| name.eq_ignore_ascii_case("Thumbs.db")
		})
}

fn zip_error(error: zip::result::ZipError) -> io::Error {
	io::Error::new(ErrorKind::InvalidData, error.to_string())
}

pub(crate) fn image_from_bytes(bytes: &[u8]) -> Result<DynamicImage, image::ImageError> {
	image::load_from_memory(bytes)
}

pub(crate) fn image_digest(bytes: &[u8]) -> Result<String, image::ImageError> {
	let image = image_from_bytes(bytes)?;
	let rgba = image.to_rgba8();
	let mut context = Context::new(&SHA256);
	context.update(rgba.as_raw());
	Ok(HEXLOWER.encode(context.finish().as_ref()))
}

/// Use the same bounded header read as the existing media analyzer.
pub(crate) fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
	const MAX_HEADER_BYTES: usize = 64 * 1024;
	let header = &bytes[..bytes.len().min(MAX_HEADER_BYTES)];
	imagesize::blob_size(header)
		.ok()
		.map(|size| (size.width as u32, size.height as u32))
}

pub(crate) fn first_decodable_archive_image(
	path: &Path,
) -> io::Result<Option<(String, Vec<u8>)>> {
	for (entry_path, bytes) in archive_images(path)? {
		if image_from_bytes(&bytes).is_ok() {
			return Ok(Some((entry_path, bytes)));
		}
	}
	Ok(None)
}

pub(crate) fn archive_image_at(
	path: &Path,
	index: usize,
) -> io::Result<Option<(String, Vec<u8>)>> {
	Ok(archive_images(path)?.into_iter().nth(index))
}

pub(crate) fn first_decodable_page(
	book: &BookSnapshot,
) -> io::Result<Option<(String, Vec<u8>)>> {
	match book.media_kind {
		IngestMediaKind::ComicArchive => first_decodable_archive_image(&book.staged_path),
		IngestMediaKind::Epub => {
			let (content_type, bytes) =
				EpubProcessor::get_cover(book.staged_path.to_string_lossy().as_ref())
					.map_err(file_error)?;
			if content_type.is_image() && image_from_bytes(&bytes).is_ok() {
				Ok(Some(("cover".to_string(), bytes)))
			} else {
				Ok(None)
			}
		},
		IngestMediaKind::ComicRarArchive | IngestMediaKind::Pdf => {
			let config = StumpConfig::debug();
			for index in 0..book.pages.len() {
				let result = get_page(
					book.staged_path.to_string_lossy().as_ref(),
					(index + 1) as i32,
					&config.media,
				);
				if let Ok((content_type, bytes)) = result {
					if content_type.is_image() && image_from_bytes(&bytes).is_ok() {
						return Ok(Some((format!("page-{}", index + 1), bytes)));
					}
				}
			}
			Ok(None)
		},
		IngestMediaKind::Unknown => Ok(None),
	}
}

pub(crate) fn canonical_page(
	book: &BookSnapshot,
	index: usize,
) -> io::Result<Option<(String, Vec<u8>)>> {
	match book.media_kind {
		IngestMediaKind::ComicArchive => archive_image_at(&book.staged_path, index),
		IngestMediaKind::Epub => epub_spine_resource(&book.staged_path, index),
		IngestMediaKind::ComicRarArchive | IngestMediaKind::Pdf => {
			let config = StumpConfig::debug();
			match get_page(
				book.staged_path.to_string_lossy().as_ref(),
				(index + 1) as i32,
				&config.media,
			) {
				Ok((content_type, bytes)) if content_type.is_image() => {
					Ok(Some((format!("page-{}", index + 1), bytes)))
				},
				Ok(_) => Ok(None),
				Err(_) => Ok(None),
			}
		},
		IngestMediaKind::Unknown => Ok(None),
	}
}

pub(crate) fn epub_spine_len(path: &Path) -> io::Result<usize> {
	let doc = EpubProcessor::open(path.to_string_lossy().as_ref()).map_err(file_error)?;
	Ok(doc.spine.len())
}

pub(crate) fn epub_spine_resource(
	path: &Path,
	index: usize,
) -> io::Result<Option<(String, Vec<u8>)>> {
	let doc = EpubProcessor::open(path.to_string_lossy().as_ref()).map_err(file_error)?;
	let Some(spine_item) = doc.spine.get(index) else {
		return Ok(None);
	};
	let Some(resource) = doc.resources.get(&spine_item.idref) else {
		return Ok(None);
	};
	let (_, bytes) = EpubProcessor::get_resource_by_id(
		path.to_string_lossy().as_ref(),
		&spine_item.idref,
	)
	.map_err(file_error)?;
	Ok(Some((resource.path.to_string_lossy().to_string(), bytes)))
}

pub(crate) fn epub_spine_image_resource(
	path: &Path,
	index: usize,
) -> io::Result<Option<(String, Vec<u8>)>> {
	let doc = EpubProcessor::open(path.to_string_lossy().as_ref()).map_err(file_error)?;
	let Some(spine_item) = doc.spine.get(index) else {
		return Ok(None);
	};
	let Some(resource) = doc.resources.get(&spine_item.idref) else {
		return Ok(None);
	};
	if !resource.mime.starts_with("image/") {
		return Ok(None);
	}
	let (_, bytes) = EpubProcessor::get_resource_by_id(
		path.to_string_lossy().as_ref(),
		&spine_item.idref,
	)
	.map_err(file_error)?;
	Ok(Some((resource.path.to_string_lossy().to_string(), bytes)))
}

fn file_error(error: FileError) -> io::Error {
	io::Error::new(ErrorKind::InvalidData, error.to_string())
}

pub(crate) fn page_count(book: &BookSnapshot) -> i64 {
	let pages = book.pages.iter().filter(|page| page.is_image).count();
	if pages > 0 {
		pages as i64
	} else {
		book.embedded_metadata
			.as_ref()
			.and_then(|metadata| metadata.page_count)
			.map(i64::from)
			.unwrap_or(0)
	}
}

type AnalysisDimensions = Option<(usize, Vec<Option<(u32, u32)>>)>;

pub(crate) fn dimensions_from_analysis(book: &BookSnapshot) -> AnalysisDimensions {
	let analysis = book.analysis.as_ref()?;
	if analysis.dimensions.is_empty() {
		return None;
	}
	let dimensions = if analysis.content_types.len() == analysis.dimensions.len() {
		analysis
			.dimensions
			.iter()
			.zip(&analysis.content_types)
			.filter(|(_, content_type)| content_type.starts_with("image/"))
			.map(|(dimension, _)| Some((dimension.width, dimension.height)))
			.collect::<Vec<_>>()
	} else {
		analysis
			.dimensions
			.iter()
			.map(|dimension| Some((dimension.width, dimension.height)))
			.collect::<Vec<_>>()
	};
	let image_count = dimensions.len();
	if image_count == 0 {
		return None;
	}
	Some((image_count, dimensions))
}

#[cfg(test)]
mod tests;
