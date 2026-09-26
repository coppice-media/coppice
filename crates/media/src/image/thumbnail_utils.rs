use std::{
	path::{Path, PathBuf},
	sync::atomic::{AtomicU64, Ordering},
};

use models::shared::image_processor_options::{
	FitWithinResize, ImageProcessorOptions, ImageResizeMethod, SupportedImageFormat,
};
use tokio::{fs, task::spawn_blocking};
use tracing::{error, trace, warn};

use crate::{
	content_type::ContentType,
	image::{GenericImageProcessor, ImageProcessor, WebpProcessor},
	media::get_page_async,
	FileError, MediaConfig,
};

/// The widest a thumbnail generated on first request may be, in pixels.
pub const ON_DEMAND_THUMBNAIL_MAX_WIDTH: u32 = 400;
/// The tallest a thumbnail generated on first request may be, in pixels.
/// A 2:3 cover bounded to 400 px wide lands at exactly this height.
pub const ON_DEMAND_THUMBNAIL_MAX_HEIGHT: u32 = 600;
/// WebP quality for thumbnails generated on first request; quality 100 made a
/// 396×600 page ~145 KB.
pub const ON_DEMAND_THUMBNAIL_QUALITY: u16 = 80;

/// The options used for a book whose library has no `thumbnail_config`:
/// WebP at [`ON_DEMAND_THUMBNAIL_QUALITY`], shrunk to fit within
/// [`ON_DEMAND_THUMBNAIL_MAX_WIDTH`] × [`ON_DEMAND_THUMBNAIL_MAX_HEIGHT`]
/// without upscaling a cover that is already smaller.
pub fn on_demand_thumbnail_options() -> ImageProcessorOptions {
	ImageProcessorOptions {
		resize_method: Some(ImageResizeMethod::FitWithin(FitWithinResize {
			width: ON_DEMAND_THUMBNAIL_MAX_WIDTH,
			height: ON_DEMAND_THUMBNAIL_MAX_HEIGHT,
		})),
		format: SupportedImageFormat::Webp,
		quality: Some(ON_DEMAND_THUMBNAIL_QUALITY),
		page: None,
	}
}

static ON_DEMAND_TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Serve the thumbnail for a book nobody has generated one for yet, and keep it.
///
/// The page named by `options.page` (default 1) is pulled once, encoded with
/// `options` off the async runtime, written to `thumbnails/<id>.<ext>` — the
/// name [`crate::get_thumbnail`] resolves, so the next request reads the
/// file — and returned. The write goes through a temporary sibling and a
/// rename, so two first requests racing on the same book both produce a
/// complete file and the last rename wins.
///
/// Anything short of the page itself being unreadable degrades to the raw
/// page: a page that is not a raster the encoder understands, an encoder
/// error, or a thumbnails directory that cannot be written all return the
/// page bytes exactly as before, so a thumbnail that cannot be cached is
/// still a thumbnail.
pub async fn generate_thumbnail_on_demand(
	id: &str,
	book_path: &str,
	options: ImageProcessorOptions,
	config: &MediaConfig,
) -> Result<(ContentType, Vec<u8>), FileError> {
	let adjusted_config = MediaConfig {
		// Only the one page is wanted, so PDF prerendering would be wasted work
		pdf_prerender_range: 0,
		..config.clone()
	};
	let page = options.page.unwrap_or(1);
	let (content_type, page_data) =
		get_page_async(book_path, page, &adjusted_config).await?;
	if !content_type.is_decodable_image() {
		trace!(
			id,
			?content_type,
			"Page is not a decodable raster; serving it as is"
		);
		return Ok((content_type, page_data));
	}

	let format = options.format;
	let (page_data, encoded) = spawn_blocking(move || {
		let encoded = match format {
			SupportedImageFormat::Webp => WebpProcessor::generate(&page_data, options),
			_ => GenericImageProcessor::generate(&page_data, options),
		};
		(page_data, encoded)
	})
	.await
	.map_err(|error| FileError::UnknownError(error.to_string()))?;

	let thumbnail = match encoded {
		Ok(thumbnail) => thumbnail,
		Err(error) => {
			warn!(
				id,
				?error,
				"Could not encode the page into a thumbnail; serving the page"
			);
			return Ok((content_type, page_data));
		},
	};
	drop(page_data);

	let extension = format.extension();
	let thumbnails_dir = config.get_thumbnails_dir();
	let final_path = thumbnails_dir.join(format!("{id}.{extension}"));
	let temp_path = thumbnails_dir.join(format!(
		"{id}.{extension}.tmp-{}-{}",
		std::process::id(),
		ON_DEMAND_TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
	));
	if let Err(error) = persist_atomically(&temp_path, &final_path, &thumbnail).await {
		warn!(id, ?error, path = ?final_path, "Could not save the generated thumbnail; serving it uncached");
	}

	Ok((ContentType::from_extension(extension), thumbnail))
}

async fn persist_atomically(
	temp_path: &Path,
	final_path: &Path,
	bytes: &[u8],
) -> std::io::Result<()> {
	fs::write(temp_path, bytes).await?;
	if let Err(error) = fs::rename(temp_path, final_path).await {
		let _ = fs::remove_file(temp_path).await;
		return Err(error);
	}
	Ok(())
}

pub async fn place_thumbnail(
	id: &str,
	ext: &str,
	bytes: &[u8],
	config: &MediaConfig,
) -> Result<PathBuf, FileError> {
	let thumbnail_path = config.get_thumbnails_dir().join(format!("{id}.{ext}"));
	fs::write(&thumbnail_path, bytes).await?;
	Ok(thumbnail_path)
}

/// Replaces all thumbnail files for an entity and writes its new thumbnail.
///
/// Stump stores one selected thumbnail per entity. Existing generated and
/// uploaded variants share the entity filename prefix so replacing an upload
/// cannot leave stale files behind.
pub async fn replace_thumbnail(
	id: &str,
	ext: &str,
	bytes: &[u8],
	config: &MediaConfig,
) -> Result<PathBuf, FileError> {
	let ids = [id.to_owned()];
	if let Err(error) = remove_thumbnails(&ids, config.get_thumbnails_dir()).await {
		tracing::warn!(
			?error,
			id,
			"Could not remove previous thumbnails before replacement"
		);
	}
	place_thumbnail(id, ext, bytes, config).await
}

pub const THUMBNAIL_LOG_FREQUENCY: usize = 500;

/// Deletes thumbnails and returns the number deleted if successful, returns
/// [`FileError`] otherwise.
pub async fn remove_thumbnails(
	id_list: &[String],
	thumbnails_dir: &Path,
) -> Result<u64, FileError> {
	let mut read_dir = tokio::fs::read_dir(thumbnails_dir).await?;

	// Asynchronously collect thumbnails
	let mut found_thumbnails = Vec::with_capacity(id_list.len());
	while let Some(entry) = read_dir.next_entry().await? {
		let path = entry.path();
		if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
			if id_list.iter().any(|id| filename.starts_with(id)) {
				found_thumbnails.push(path);
			}
		}
	}

	let found_thumbnails_count = found_thumbnails.len();
	tracing::debug!(found_thumbnails_count, "Found thumbnails to remove");

	let mut deleted_thumbnails_count = 0;

	for (idx, path) in found_thumbnails.iter().enumerate() {
		if idx % THUMBNAIL_LOG_FREQUENCY == 0 {
			trace!("Processed {} thumbnails for removal.", idx + 1);
		}

		match tokio::fs::remove_file(path).await {
			Ok(_) => deleted_thumbnails_count += 1,
			Err(e) => {
				error!(error = ?e, ?path, "Error deleting thumbnail!");
				return Err(e.into());
			},
		};
	}

	Ok(deleted_thumbnails_count)
}

pub fn scale_width_dimension(w: f32, h: f32, target_height: f32) -> (u32, u32) {
	let scale = target_height / h;
	((w * scale).round() as u32, (h * scale).round() as u32)
}

pub fn scale_height_dimension(w: f32, h: f32, target_width: f32) -> (u32, u32) {
	let scale = target_width / w;
	((w * scale).round() as u32, (h * scale).round() as u32)
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use image::GenericImageView;
	use models::shared::image_processor_options::ExactDimensionResize;
	use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

	use super::*;
	use crate::tests::get_test_png_path;

	fn media_config(root: &Path) -> MediaConfig {
		let config = MediaConfig::from_values(
			root.to_path_buf(),
			None,
			false,
			0,
			150,
			0,
			false,
			"webp".to_owned(),
			1,
		);
		std::fs::create_dir_all(config.get_thumbnails_dir()).expect("thumbnails dir");
		config
	}

	fn cbz_with_first_page(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
		let path = dir.join("book.cbz");
		let mut zip = ZipWriter::new(std::fs::File::create(&path).expect("create cbz"));
		zip.start_file(
			name,
			SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
		)
		.expect("start entry");
		zip.write_all(bytes).expect("write entry");
		zip.finish().expect("finish cbz");
		path
	}

	/// The fixture photo blown up to print resolution, as a PNG: the shape of
	/// a scanned comic page.
	fn print_resolution_png() -> Vec<u8> {
		GenericImageProcessor::generate_from_path(
			&get_test_png_path(),
			ImageProcessorOptions {
				resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
					width: 1500,
					height: 2100,
				})),
				format: SupportedImageFormat::Png,
				quality: None,
				page: None,
			},
		)
		.expect("upscaled fixture")
	}

	fn temp_files(dir: &Path) -> Vec<PathBuf> {
		std::fs::read_dir(dir)
			.expect("read thumbnails dir")
			.map(|entry| entry.expect("entry").path())
			.filter(|path| path.to_string_lossy().contains(".tmp-"))
			.collect()
	}

	#[tokio::test]
	async fn first_request_encodes_a_bounded_thumbnail_and_keeps_it() {
		let tempdir = tempfile::tempdir().expect("tempdir");
		let config = media_config(tempdir.path());
		let page = print_resolution_png();
		let book = cbz_with_first_page(tempdir.path(), "001.png", &page);

		let (content_type, thumbnail) = generate_thumbnail_on_demand(
			"book-1",
			book.to_str().expect("utf-8 path"),
			on_demand_thumbnail_options(),
			&config,
		)
		.await
		.expect("thumbnail");

		assert_eq!(content_type, ContentType::WEBP);
		let (width, height) = image::load_from_memory(&thumbnail)
			.expect("decodable thumbnail")
			.dimensions();
		assert_eq!((width, height), (400, 560), "1500×2100 fit within 400×600");
		assert!(
			thumbnail.len() * 8 < page.len(),
			"thumbnail {} bytes vs page {} bytes",
			thumbnail.len(),
			page.len()
		);

		let saved = config.get_thumbnails_dir().join("book-1.webp");
		assert_eq!(std::fs::read(&saved).expect("saved thumbnail"), thumbnail);
		assert!(temp_files(config.get_thumbnails_dir()).is_empty());

		// The kept file is what the lookup every profile runs first resolves,
		// so the next request never opens the book again.
		std::fs::remove_file(&book).expect("remove book");
		let found = crate::get_thumbnail(config.get_thumbnails_dir(), "book-1", None)
			.await
			.expect("lookup")
			.expect("found");
		assert_eq!(found, (ContentType::WEBP, thumbnail));
	}

	#[tokio::test]
	async fn configured_options_decide_format_and_size() {
		let tempdir = tempfile::tempdir().expect("tempdir");
		let config = media_config(tempdir.path());
		let book =
			cbz_with_first_page(tempdir.path(), "001.png", &print_resolution_png());

		let (content_type, thumbnail) = generate_thumbnail_on_demand(
			"book-2",
			book.to_str().expect("utf-8 path"),
			ImageProcessorOptions {
				resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
					width: 120,
					height: 180,
				})),
				format: SupportedImageFormat::Jpeg,
				quality: None,
				page: None,
			},
			&config,
		)
		.await
		.expect("thumbnail");

		assert_eq!(content_type, ContentType::JPEG);
		let (width, height) = image::load_from_memory(&thumbnail)
			.expect("decodable thumbnail")
			.dimensions();
		assert_eq!((width, height), (120, 180));
		assert!(config.get_thumbnails_dir().join("book-2.jpeg").is_file());
		assert!(!config.get_thumbnails_dir().join("book-2.webp").exists());
	}

	#[tokio::test]
	async fn undecodable_page_is_served_raw_and_not_kept() {
		let tempdir = tempfile::tempdir().expect("tempdir");
		let config = media_config(tempdir.path());
		let not_a_png = b"definitely not a png".to_vec();
		let book = cbz_with_first_page(tempdir.path(), "001.png", &not_a_png);

		let (content_type, bytes) = generate_thumbnail_on_demand(
			"book-3",
			book.to_str().expect("utf-8 path"),
			on_demand_thumbnail_options(),
			&config,
		)
		.await
		.expect("falls back to the page");

		assert_eq!(content_type, ContentType::PNG);
		assert_eq!(bytes, not_a_png);
		assert!(std::fs::read_dir(config.get_thumbnails_dir())
			.expect("read thumbnails dir")
			.next()
			.is_none());
	}

	#[tokio::test]
	async fn racing_first_requests_both_leave_a_complete_file() {
		let tempdir = tempfile::tempdir().expect("tempdir");
		let config = media_config(tempdir.path());
		let book =
			cbz_with_first_page(tempdir.path(), "001.png", &print_resolution_png());
		let path = book.to_str().expect("utf-8 path").to_owned();

		let (first, second) = tokio::join!(
			generate_thumbnail_on_demand(
				"book-4",
				&path,
				on_demand_thumbnail_options(),
				&config
			),
			generate_thumbnail_on_demand(
				"book-4",
				&path,
				on_demand_thumbnail_options(),
				&config
			),
		);
		let first = first.expect("first").1;
		let second = second.expect("second").1;
		assert_eq!(first, second);

		let saved = std::fs::read(config.get_thumbnails_dir().join("book-4.webp"))
			.expect("saved thumbnail");
		assert_eq!(saved, first);
		assert!(temp_files(config.get_thumbnails_dir()).is_empty());
	}
}
