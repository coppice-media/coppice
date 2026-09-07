//! File and image processing primitives used by Stump.
//!
//! This crate deliberately has no dependency on `stump_core`; callers provide a
//! [`MediaConfig`] snapshot when processing media.  The default feature set keeps
//! PDFium and RAR support enabled, while `pdf` and `rar` can be disabled for
//! smaller deployments.
//!
//! Upstream provenance, feature/dispatch decisions, hash compatibility rules,
//! and verification commands are in `crates/media/README.md`.

use std::path::{Path, PathBuf};

pub mod archive;
pub mod audio;
pub mod common;
pub mod content_type;
pub mod directory_listing;
pub mod drm;
pub mod error;
pub mod hash;
pub mod transform;
pub mod image {
	mod error;
	mod generic;
	mod placeholder;
	mod process;
	mod thumbnail_utils;
	mod webp;

	pub use self::error::ProcessorError;
	pub use self::generic::GenericImageProcessor;
	pub use self::placeholder::{
		generate_image_metadata, generate_image_metadata_from_bytes,
		process_image_colors, process_image_colors_from_bytes, process_image_thumbhash,
		process_image_thumbhash_from_bytes,
	};
	pub use self::process::{ImageProcessor, ImageProcessorOptionsExt};
	pub use self::thumbnail_utils::{
		place_thumbnail, remove_thumbnails, replace_thumbnail, scale_height_dimension,
		scale_width_dimension, THUMBNAIL_LOG_FREQUENCY,
	};
	pub use self::webp::WebpProcessor;

	use image::ImageFormat;
	use models::shared::image_processor_options::{
		ScaledDimensionResize, SupportedImageFormat,
	};
	use tokio::{sync::oneshot, task::spawn_blocking};

	pub fn into_image_format(format: SupportedImageFormat) -> ImageFormat {
		match format {
			SupportedImageFormat::Jpeg => ImageFormat::Jpeg,
			SupportedImageFormat::Png => ImageFormat::Png,
			SupportedImageFormat::Webp => ImageFormat::WebP,
		}
	}

	fn _resize_image(
		buf: &[u8],
		dimension: ScaledDimensionResize,
	) -> Result<Vec<u8>, ProcessorError> {
		match image::guess_format(buf)? {
			ImageFormat::WebP => Ok(WebpProcessor::resize_scaled(buf, dimension)?),
			ImageFormat::Jpeg | ImageFormat::Png => {
				Ok(GenericImageProcessor::resize_scaled(buf, dimension)?)
			},
			_ => Err(ProcessorError::UnsupportedImageFormat),
		}
	}

	pub async fn resize_image(
		buf: Vec<u8>,
		dimension: ScaledDimensionResize,
	) -> Result<Vec<u8>, ProcessorError> {
		let (tx, rx) = oneshot::channel();

		let handle = spawn_blocking(move || {
			let result = _resize_image(&buf, dimension);
			let _ = tx.send(result);
		});

		let resized_image = if let Ok(recv) = rx.await {
			recv?
		} else {
			handle
				.await
				.map_err(|e| ProcessorError::UnknownError(e.to_string()))?;
			return Err(ProcessorError::UnknownError(
				"Failed to receive resized image".to_string(),
			));
		};

		Ok(resized_image)
	}
}
pub mod media {
	mod epub_search;
	pub mod format;
	mod metadata;
	mod process;
	pub mod readium;
	pub(crate) mod utils;

	pub use epub_search::{
		search_epub, EpubSearchCursor, EpubSearchError, EpubSearchOptions,
		EpubSearchResponse, EPUB_SEARCH_DEFAULT_LIMIT, EPUB_SEARCH_MAX_LIMIT,
	};
	pub use metadata::*;
	pub use process::*;
	pub use readium::ReadiumManifestGenerator;
	pub use utils::is_accepted_cover_name;
}
pub mod serde;
pub mod series_metadata;
pub mod virtual_media;

pub use common::*;
pub use content_type::ContentType;
pub use directory_listing::{
	DirectoryListing, DirectoryListingFile, DirectoryListingIgnoreParams,
	DirectoryListingInput,
};
pub use drm::{detect_drm, DrmContainer, DrmReport, DrmScheme};
pub use error::FileError;
pub use hash::{
	dhash_image, generate, generate_koreader_hash, hamming, page_dhash,
	DUPLICATE_PAGE_TOLERANCE, HASH_SAMPLE_COUNT, HASH_SAMPLE_SIZE,
};
pub use image::{
	generate_image_metadata, generate_image_metadata_from_bytes, into_image_format,
	process_image_colors, process_image_colors_from_bytes, process_image_thumbhash,
	process_image_thumbhash_from_bytes, resize_image, GenericImageProcessor,
	ImageProcessor, ImageProcessorOptionsExt, ProcessorError, WebpProcessor,
};
#[cfg(feature = "pdf")]
pub use media::format::pdf::PdfProcessor;
#[cfg(feature = "rar")]
pub use media::format::rar::{RarEntry, RarProcessor};
pub use media::format::{
	audio::{AudioProcessor, AUDIO_PAGES},
	epub::{EpubNavEntry, EpubProcessor, EpubStructure},
	mobi::{
		MobiBook, MobiFlow, MobiNavEntry, MobiProcessor, MobiResource, MobiSection,
		KINDLE_EXTENSIONS,
	},
	zip::ZipProcessor,
};
pub use media::{
	search_epub, EpubSearchCursor, EpubSearchError, EpubSearchOptions,
	EpubSearchResponse, FileProcessor, FileProcessorOptions, ProcessedFile,
	ProcessedFileHashes, ProcessedMediaMetadata, ReadiumManifestGenerator,
	EPUB_SEARCH_DEFAULT_LIMIT, EPUB_SEARCH_MAX_LIMIT,
};
pub use series_metadata::{ProcessedSeriesMetadata, SeriesJson};
pub use transform::{
	transform_pages, transform_pages_blocking, ComicContainer, TransformFormat,
	TransformProfile,
};

/// The immutable configuration snapshot consumed by media processors.
///
/// Directory values are resolved when the parent `StumpConfig` is finalized so
/// processor calls do not repeatedly clone or rebuild paths.
#[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
pub struct MediaConfig {
	pub pdfium_path: Option<PathBuf>,
	pub pdf_cache_pages: bool,
	pub pdf_prerender_range: u32,
	pub pdf_render_dpi: u32,
	pub pdf_max_dimension: u32,
	pub pdf_high_quality: bool,
	pub pdf_render_format: String,
	pub config_dir: PathBuf,
	pub cache_dir: PathBuf,
	pub thumbnails_dir: PathBuf,
	pub pdf_cache_dir: PathBuf,
	pub cpu_concurrency_limit: usize,
}

impl Default for MediaConfig {
	fn default() -> Self {
		Self::from_values(
			PathBuf::new(),
			None,
			false,
			0,
			150,
			0,
			false,
			"webp".to_string(),
			1,
		)
	}
}

impl MediaConfig {
	#[allow(clippy::too_many_arguments)]
	pub fn from_values(
		config_dir: PathBuf,
		pdfium_path: Option<PathBuf>,
		pdf_cache_pages: bool,
		pdf_prerender_range: u32,
		pdf_render_dpi: u32,
		pdf_max_dimension: u32,
		pdf_high_quality: bool,
		pdf_render_format: String,
		cpu_concurrency_limit: usize,
	) -> Self {
		let cache_dir = config_dir.join("cache");
		let thumbnails_dir = config_dir.join("thumbnails");
		let pdf_cache_dir = cache_dir.join("pdf_pages");
		Self {
			pdfium_path,
			pdf_cache_pages,
			pdf_prerender_range,
			pdf_render_dpi,
			pdf_max_dimension,
			pdf_high_quality,
			pdf_render_format,
			config_dir,
			cache_dir,
			thumbnails_dir,
			pdf_cache_dir,
			cpu_concurrency_limit,
		}
	}

	pub fn get_config_dir(&self) -> &Path {
		&self.config_dir
	}

	pub fn get_cache_dir(&self) -> &Path {
		&self.cache_dir
	}

	pub fn get_thumbnails_dir(&self) -> &Path {
		&self.thumbnails_dir
	}

	pub fn get_pdf_cache_dir(&self) -> &Path {
		&self.pdf_cache_dir
	}

	pub fn cpu_concurrency_limit(&self) -> usize {
		self.cpu_concurrency_limit
	}

	/// Parse the configured PDF render format, falling back to WebP as before.
	pub fn get_pdf_render_format(
		&self,
	) -> models::shared::image_processor_options::SupportedImageFormat {
		use models::shared::image_processor_options::SupportedImageFormat;

		match self.pdf_render_format.to_lowercase().as_str() {
			"webp" => SupportedImageFormat::Webp,
			"jpeg" | "jpg" => SupportedImageFormat::Jpeg,
			"png" => SupportedImageFormat::Png,
			_ => {
				tracing::warn!(
					format = self.pdf_render_format,
					"Invalid PDF render format, falling back to WebP"
				);
				SupportedImageFormat::Webp
			},
		}
	}
}

#[cfg(test)]
pub(crate) mod tests {
	use std::{fs, path::PathBuf};

	pub fn get_test_zip_path() -> String {
		fixture("book.zip")
	}

	pub fn get_test_complex_zip_path() -> String {
		fixture("book-complex-tree.zip")
	}

	pub fn get_test_rar_path() -> String {
		fixture("book.rar")
	}

	pub fn get_test_rar_file_data() -> Vec<u8> {
		fs::read(get_test_rar_path()).expect("Failed to fetch test rar file")
	}

	pub fn get_test_complex_rar_path() -> String {
		fixture("book-complex-tree.rar")
	}

	pub fn get_test_epub_path() -> String {
		fixture("book.epub")
	}

	pub fn get_test_pdf_path() -> String {
		fixture("rust_book.pdf")
	}

	pub fn get_test_cbz_path() -> String {
		fixture("science_comics_001.cbz")
	}

	pub fn get_nested_macos_compressed_cbz_path() -> String {
		fixture("nested-macos-compressed.cbz")
	}

	pub fn get_test_webp_path() -> String {
		fixture("example.webp")
	}

	pub fn get_test_jpg_path() -> String {
		fixture("example.jpeg")
	}

	pub fn get_test_png_path() -> String {
		fixture("example.png")
	}

	fn fixture(name: &str) -> String {
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("integration-tests/data")
			.join(name)
			.to_string_lossy()
			.to_string()
	}
}
