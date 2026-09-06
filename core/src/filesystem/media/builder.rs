use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Utc};
use models::{
	entity::{library_config, media, media_metadata},
	services::audio::AudioFacts,
	shared::enums::FileStatus,
};
use sea_orm::Set;
use uuid::Uuid;

use crate::{config::StumpConfig, CoreResult};
use stump_media::{
	media::{
		generate_hashes, process, process_metadata, ProcessedFileHashes,
		ProcessedMediaMetadata,
	},
	FileParts, PathUtils,
};
use stump_scanner::{CustomVisit, CustomVisitResult};

pub struct MediaBuilder {
	path: PathBuf,
	series_id: String,
	library_config: library_config::Model,
	config: StumpConfig,
}

#[derive(Debug, Clone)]
pub struct BuiltMedia {
	pub media: media::ActiveModel,
	pub metadata: Option<media_metadata::ActiveModel>,
	/// Tag names extracted from the file's metadata (e.g. ComicInfo.xml `<Tags>`).
	/// Applied additively to `media_tags` during create/update so user-assigned tags
	/// are never removed by a rescan.
	pub tags: Vec<String>,
	/// The probed audio facts of an audiobook, which the scanner persists
	/// into `media_audio` and its track/chapter tables through
	/// [`models::services::audio::replace_in`]. `None` for every other book.
	pub audio: Option<AudioFacts>,
}

/// The extension of `path`, lowercased and without the dot.
///
/// `media.extension` is what OPDS turns into an acquisition mime type, so it
/// is compared case-insensitively downstream; an audiobook's tracks are the
/// only place Stump reads an extension off a path it did not build itself.
fn lowercase_extension(path: &str) -> String {
	Path::new(path)
		.extension()
		.and_then(|extension| extension.to_str())
		.unwrap_or_default()
		.to_lowercase()
}

impl MediaBuilder {
	pub fn new(
		path: &Path,
		series_id: &str,
		library_config: library_config::Model,
		config: &StumpConfig,
	) -> Self {
		Self {
			path: path.to_path_buf(),
			series_id: series_id.to_string(),
			library_config,
			config: config.clone(),
		}
	}

	pub fn rebuild(self, media: &media::ModelWithMetadata) -> CoreResult<BuiltMedia> {
		let generated = self.build()?;
		Ok(BuiltMedia {
			media: media::ActiveModel {
				id: Set(media.media.id.clone()),
				..generated.media
			},
			metadata: generated.metadata.map(|meta| media_metadata::ActiveModel {
				media_id: Set(Some(media.media.id.clone())),
				..meta
			}),
			tags: generated.tags,
			audio: generated.audio,
		})
	}

	pub fn build(self) -> CoreResult<BuiltMedia> {
		let processed_entry =
			process(&self.path, self.library_config.into(), &self.config.media)?;

		tracing::trace!(?processed_entry, "Processed entry");

		let pathbuf = processed_entry.path;
		let path = pathbuf.as_path();

		let FileParts {
			file_name,
			extension,
			file_stem,
			..
		} = path.file_parts();
		let path_str = path.to_str().unwrap_or_default().to_string();

		let (raw_size, last_modified_at) = path.metadata().map(|m| {
			let datetime: Option<DateTime<Utc>> = m.modified().ok().map(|t| t.into());
			let last_modified_at: Option<DateTime<FixedOffset>> =
				datetime.map(|dt| dt.into());
			(m.len(), last_modified_at)
		})?;
		let size = raw_size.try_into().unwrap_or_else(|_| {
			tracing::error!(?raw_size, ?path, "Failed to convert file size to i64");
			0
		});

		let id = Uuid::new_v4().to_string();
		let pages = processed_entry.pages;
		let audio = processed_entry.audio;

		// An audiobook's name, extension and size come from its tracks rather
		// than from the path. A folder book has no extension of its own and a
		// directory's `metadata().len()` is the size of the directory entry,
		// not of the book; OPDS derives the acquisition mime type from
		// `extension`, so it has to name a real container either way. The
		// first track is the one playback starts with, and every track has an
		// audio extension by construction — `PathUtils::is_audio` is what let
		// it into the probe.
		let (name, extension, size) = match audio.as_ref() {
			Some(facts) => (
				if path.is_dir() { file_name } else { file_stem },
				facts
					.tracks
					.first()
					.map(|track| lowercase_extension(&track.path))
					.unwrap_or(extension),
				facts.tracks.iter().map(|track| track.byte_size).sum(),
			),
			None => (file_stem, extension, size),
		};
		let (resolved_metadata, resolved_tags) = processed_entry
			.metadata
			.map(|mut metadata| {
				let conflicting_page_counts =
					metadata.page_count.is_some_and(|count| count != pages);
				if conflicting_page_counts {
					tracing::warn!(
						?pages,
						?metadata.page_count,
						"Page count in metadata does not match actual page count!"
					);
					metadata.page_count = Some(pages);
				}

				let tags = metadata.tags.take().unwrap_or_default();

				let active = media_metadata::ActiveModel {
					media_id: Set(Some(id.clone())),
					..metadata.into_active_model()
				};
				(Some(active), tags)
			})
			.unwrap_or_default();

		let media = media::ActiveModel {
			id: Set(id),
			name: Set(name),
			size: Set(size),
			extension: Set(extension),
			pages: Set(pages),
			hash: Set(processed_entry.hash),
			koreader_hash: Set(processed_entry.koreader_hash),
			path: Set(path_str),
			series_id: Set(Some(self.series_id)),
			modified_at: Set(last_modified_at),
			status: Set(FileStatus::Ready),
			created_at: Set(chrono::Utc::now().into()),
			..Default::default()
		};

		Ok(BuiltMedia {
			media,
			metadata: resolved_metadata,
			tags: resolved_tags,
			audio,
		})
	}

	pub fn regen_hashes(&self) -> CoreResult<ProcessedFileHashes> {
		Ok(generate_hashes(
			self.path.clone(),
			self.library_config.clone().into(),
		)?)
	}

	pub fn regen_meta(&self) -> CoreResult<Option<ProcessedMediaMetadata>> {
		Ok(process_metadata(self.path.clone())?)
	}

	pub fn custom_visit(self, config: CustomVisit) -> CoreResult<CustomVisitResult> {
		let mut result = CustomVisitResult::default();
		if config.regen_hashes {
			result.hashes = Some(self.regen_hashes()?);
		}
		if config.regen_meta {
			result.meta = self.regen_meta()?.map(Box::new);
		}
		Ok(result)
	}
}

#[cfg(test)]
mod tests {
	use models::shared::enums::{
		LibraryPattern, LibraryType, LibraryViewMode, ReadingDirection,
		ReadingImageScaleFit, ReadingMode,
	};
	use sea_orm::ActiveValue;

	use super::*;
	use crate::filesystem::media::tests::{
		get_test_audiobook_folder_path, get_test_cbz_path, get_test_epub_path,
		get_test_pdf_path, get_test_rar_path, get_test_zip_path,
	};

	#[test]
	fn test_build_media_zip() {
		// Test with zip
		let media = build_media_test_helper(get_test_zip_path());
		assert!(media.is_ok());
		let media = media.unwrap().media;
		assert_eq!(media.extension, ActiveValue::Set("zip".to_string()));
	}

	#[test]
	fn test_build_media_cbz() {
		let media = build_media_test_helper(get_test_cbz_path());
		assert!(media.is_ok());
		let media = media.unwrap().media;
		assert_eq!(media.extension, ActiveValue::Set("cbz".to_string()));
	}

	#[test]
	fn test_build_media_rar() {
		let media = build_media_test_helper(get_test_rar_path());
		assert!(media.is_ok());
		let media = media.unwrap().media;
		assert_eq!(media.extension, ActiveValue::Set("rar".to_string()));
	}

	#[test]
	fn test_build_media_epub() {
		let media = build_media_test_helper(get_test_epub_path());
		assert!(media.is_ok());
		let media = media.unwrap().media;
		assert_eq!(media.extension, ActiveValue::Set("epub".to_string()));
	}

	/// A folder audiobook is ONE publication whose shape comes from its
	/// tracks, not from its path: a directory has no extension of its own and
	/// its `metadata().len()` is the size of a directory entry, not of the
	/// book. OPDS derives the acquisition mime type from `extension`, so
	/// `" 1"` — what `file_parts()` reads off `Book Vol. 1` — would make the
	/// book unplayable.
	#[test]
	fn test_build_media_audiobook_folder() {
		let path = get_test_audiobook_folder_path();
		let built = build_media_test_helper(path.clone()).expect("folder book builds");

		let summed_size = std::fs::read_dir(&path)
			.unwrap()
			.filter_map(Result::ok)
			.map(|entry| entry.metadata().unwrap().len() as i64)
			.sum::<i64>();

		assert_eq!(built.media.pages, ActiveValue::Set(-1));
		assert_eq!(built.media.extension, ActiveValue::Set("mp3".to_string()));
		assert_eq!(built.media.size, ActiveValue::Set(summed_size));
		assert_eq!(
			built.media.name,
			ActiveValue::Set("Book Vol. 1".to_string())
		);

		let facts = built.audio.expect("a folder book carries audio facts");
		assert_eq!(facts.tracks.len(), 3);
		assert_eq!(
			facts.duration_ms,
			facts
				.tracks
				.iter()
				.map(|track| track.duration_ms)
				.sum::<i64>()
		);
	}

	#[cfg(feature = "pdf")]
	#[test]
	fn test_build_media_pdf() {
		if stump_media::PdfProcessor::renderer(&None).is_err() {
			eprintln!("Skipping test: PDFium is not configured or available.");
			return;
		}
		let media = build_media_test_helper(get_test_pdf_path());
		assert!(media.is_ok());
		let media = media.unwrap().media;
		assert_eq!(media.extension, ActiveValue::Set("pdf".to_string()));
	}

	fn build_media_test_helper(path: String) -> CoreResult<BuiltMedia> {
		let path = Path::new(&path);
		let library_config = library_config::Model {
			convert_rar_to_zip: false,
			hard_delete_conversions: false,
			..library_config()
		};
		let series_id = "series_id";

		MediaBuilder::new(path, series_id, library_config, &StumpConfig::debug()).build()
	}

	#[test]
	fn test_regen_hashes() {
		let path = get_test_zip_path();
		let path = Path::new(&path);
		let library_config = library_config::Model {
			generate_file_hashes: true,
			..library_config()
		};
		let series_id = "series_id";

		let builder =
			MediaBuilder::new(path, series_id, library_config, &StumpConfig::debug());
		let hashes = builder.regen_hashes();
		assert!(hashes.is_ok());
		assert!(hashes.unwrap().hash.is_some());
	}

	fn library_config() -> library_config::Model {
		library_config::Model {
			id: 1,
			convert_rar_to_zip: false,
			generate_file_hashes: false,
			default_reading_dir: ReadingDirection::Ltr,
			default_reading_image_scale_fit: ReadingImageScaleFit::Auto,
			default_reading_mode: ReadingMode::Paged,
			generate_koreader_hashes: false,
			hard_delete_conversions: false,
			ignore_rules: None,
			library_id: Some("library_id".to_string()),
			library_pattern: LibraryPattern::SeriesBased,
			process_metadata: true,
			thumbnail_config: None,
			process_thumbnail_colors_even_without_config: false,
			watch: false,
			default_library_view_mode: LibraryViewMode::Series,
			hide_series_view: false,
			library_type: LibraryType::Mixed,
			skip_book_overview: false,
			metadata_policy: None,
		}
	}
}
