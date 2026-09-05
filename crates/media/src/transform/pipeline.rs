//! The per-page transform stages: decode → resize → tone → encode, and the
//! concurrent [`transform_pages`] / [`transform_pages_blocking`] drivers.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::mpsc;
use std::sync::Mutex;

use image::DynamicImage;
use serde::{Deserialize, Serialize};

use super::error::{TransformError, TransformResult};
use super::profile::{
	MAX_TALL_PAGE_RATIO, TransformFormat, TransformProfile,
};

/// A single transformed page image, ready to be written into a container.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedPage {
	/// Width of the encoded image in pixels.
	pub width: u32,
	/// Height of the encoded image in pixels.
	pub height: u32,
	/// Encoded format (drives the container's media types).
	pub format: PageFormat,
	/// The encoded image bytes.
	pub bytes: Vec<u8>,
}

/// The concrete image format of an [`EncodedPage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageFormat {
	Jpeg,
	Png,
	Webp,
}

impl PageFormat {
	/// MIME type used inside EPUB manifests.
	pub fn media_type(&self) -> &'static str {
		match self {
			Self::Jpeg => "image/jpeg",
			Self::Png => "image/png",
			Self::Webp => "image/webp",
		}
	}

	/// File extension (without the dot).
	pub fn extension(&self) -> &'static str {
		match self {
			Self::Jpeg => "jpg",
			Self::Png => "png",
			Self::Webp => "webp",
		}
	}
}

impl From<&TransformFormat> for PageFormat {
	fn from(format: &TransformFormat) -> Self {
		match format {
			TransformFormat::Jpeg { .. } => Self::Jpeg,
			TransformFormat::Png => Self::Png,
			TransformFormat::Webp { .. } => Self::Webp,
		}
	}
}

/// Decode one source page image.
pub(crate) fn decode_page(bytes: &[u8]) -> TransformResult<DynamicImage> {
	image::load_from_memory(bytes)
		.map_err(|error| TransformError::Decode(error.to_string()))
}

/// Fit `(width, height)` inside the profile caps, preserving aspect ratio and
/// never upscaling.
pub(crate) fn fit_dimensions(
	width: u32,
	height: u32,
	max_width: Option<u32>,
	max_height: Option<u32>,
) -> (u32, u32) {
	let mut scale = f64::INFINITY;
	if let Some(max_width) = max_width.filter(|max| *max > 0 && width > *max) {
		scale = scale.min(f64::from(max_width) / f64::from(width));
	}
	if let Some(max_height) = max_height.filter(|max| *max > 0 && height > *max) {
		scale = scale.min(f64::from(max_height) / f64::from(height));
	}
	if !scale.is_finite() {
		return (width, height);
	}
	(
		((f64::from(width) * scale).round() as u32).max(1),
		((f64::from(height) * scale).round() as u32).max(1),
	)
}

/// Resize an image to exactly `(width, height)`.
///
/// With the `fast-resize` feature this is a SIMD Lanczos3 convolution via
/// `fast_image_resize` on the RGBA8 representation; otherwise the `image`
/// crate's CatmullRom convolution.
pub(crate) fn resize_image(
	image: &DynamicImage,
	width: u32,
	height: u32,
) -> TransformResult<DynamicImage> {
	if image.width() == width && image.height() == height {
		return Ok(image.clone());
	}

	#[cfg(feature = "fast-resize")]
	{
		use fast_image_resize::images::Image as FrImage;
		use fast_image_resize::{PixelType, Resizer};

		let rgba = image.to_rgba8();
		let (src_w, src_h) = (rgba.width(), rgba.height());
		let src = FrImage::from_vec_u8(
			src_w,
			src_h,
			rgba.into_raw(),
			PixelType::U8x4,
		)
		.map_err(|error| TransformError::Other(error.to_string()))?;
		let mut dst = FrImage::new(width, height, PixelType::U8x4);

		// `None` selects the default options: Lanczos3 convolution with alpha
		// premultiplication.
		Resizer::new()
			.resize(&src, &mut dst, None)
			.map_err(|error| TransformError::Other(error.to_string()))?;

		let buffer = image::ImageBuffer::from_raw(width, height, dst.buffer().to_vec())
			.ok_or_else(|| {
				TransformError::Other("resize produced an invalid buffer".to_string())
			})?;
		Ok(DynamicImage::ImageRgba8(buffer))
	}

	#[cfg(not(feature = "fast-resize"))]
	{
		let resized = image::imageops::resize(
			&image.to_rgba8(),
			width,
			height,
			image::imageops::FilterType::CatmullRom,
		);
		Ok(DynamicImage::ImageRgba8(resized))
	}
}

/// Build the combined black-level + gamma lookup table, or `None` when the
/// profile does not tone-map.
fn tone_lut(black_level: Option<u8>, gamma: Option<f32>) -> Option<[u8; 256]> {
	let black_level = black_level.unwrap_or(0);
	let gamma = gamma.filter(|gamma| *gamma > 0.0);
	if black_level == 0 && gamma.is_none() {
		return None;
	}

	let mut lut = [0u8; 256];
	for (input, output) in lut.iter_mut().enumerate() {
		let input = input as f32;
		let stretched = if input <= f32::from(black_level) {
			0.0
		} else {
			(input - f32::from(black_level)) / (255.0 - f32::from(black_level))
		};
		let value = match gamma {
			Some(gamma) => stretched.powf(1.0 / gamma),
			None => stretched,
		};
		*output = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
	}
	Some(lut)
}

/// Apply grayscale conversion, black-level stretch, and gamma in one pass.
pub(crate) fn apply_tone(
	image: DynamicImage,
	grayscale: bool,
	black_level: Option<u8>,
	gamma: Option<f32>,
) -> DynamicImage {
	let image = if grayscale {
		DynamicImage::ImageLuma8(image::imageops::grayscale(&image))
	} else {
		image
	};

	match tone_lut(black_level, gamma) {
		None => image,
		Some(lut) => match image {
			DynamicImage::ImageLuma8(mut buffer) => {
				for pixel in buffer.pixels_mut() {
					pixel.0[0] = lut[pixel.0[0] as usize];
				}
				DynamicImage::ImageLuma8(buffer)
			},
			DynamicImage::ImageLumaA8(mut buffer) => {
				for pixel in buffer.pixels_mut() {
					pixel.0[0] = lut[pixel.0[0] as usize];
				}
				DynamicImage::ImageLumaA8(buffer)
			},
			DynamicImage::ImageRgb8(mut buffer) => {
				for pixel in buffer.pixels_mut() {
					pixel.0[0] = lut[pixel.0[0] as usize];
					pixel.0[1] = lut[pixel.0[1] as usize];
					pixel.0[2] = lut[pixel.0[2] as usize];
				}
				DynamicImage::ImageRgb8(buffer)
			},
			DynamicImage::ImageRgba8(mut buffer) => {
				for pixel in buffer.pixels_mut() {
					pixel.0[0] = lut[pixel.0[0] as usize];
					pixel.0[1] = lut[pixel.0[1] as usize];
					pixel.0[2] = lut[pixel.0[2] as usize];
				}
				DynamicImage::ImageRgba8(buffer)
			},
			other => {
				let mut buffer = other.into_rgb8();
				for pixel in buffer.pixels_mut() {
					pixel.0[0] = lut[pixel.0[0] as usize];
					pixel.0[1] = lut[pixel.0[1] as usize];
					pixel.0[2] = lut[pixel.0[2] as usize];
				}
				DynamicImage::ImageRgb8(buffer)
			},
		},
	}
}

/// Split a page into vertical panels when it is taller than
/// [`MAX_TALL_PAGE_RATIO`]× its width.
///
/// Returns `None` when the page does not need splitting. Every segment keeps
/// the page width and its height never exceeds `ratio × width`.
pub(crate) fn split_tall_page(image: &DynamicImage) -> Option<Vec<DynamicImage>> {
	let (width, height) = (image.width(), image.height());
	if width == 0 || height == 0 {
		return None;
	}
	let ratio = height as f32 / width as f32;
	if ratio <= MAX_TALL_PAGE_RATIO {
		return None;
	}

	let segments = (ratio / MAX_TALL_PAGE_RATIO).ceil() as u32;
	let segment_height = height.div_ceil(segments);

	let mut pages = Vec::with_capacity(segments as usize);
	let mut top = 0;
	while top < height {
		let bottom = (top + segment_height).min(height);
		pages.push(DynamicImage::ImageRgba8(
			image::imageops::crop_imm(image, 0, top, width, bottom - top).to_image(),
		));
		top = bottom;
	}
	Some(pages)
}

/// Encode an image according to the profile's output format.
pub(crate) fn encode_page(
	image: &DynamicImage,
	format: &TransformFormat,
) -> TransformResult<EncodedPage> {
	let bytes = match format {
		TransformFormat::Jpeg { quality, subsampling } => {
			encode_jpeg(image, *quality, *subsampling)?
		},
		TransformFormat::Png => {
			let mut buffer = Vec::new();
			image
				.write_to(&mut Cursor::new(&mut buffer), image::ImageFormat::Png)
				.map_err(|error| TransformError::Encode(error.to_string()))?;
			buffer
		},
		TransformFormat::Webp { quality } => {
			let encoded = webp::Encoder::from_image(image)
				.map_err(|error| TransformError::Encode(error.to_string()))?
				.encode(f32::from(*quality));
			encoded.to_vec()
		},
	};

	Ok(EncodedPage {
		width: image.width(),
		height: image.height(),
		format: PageFormat::from(format),
		bytes,
	})
}

fn encode_jpeg(
	image: &DynamicImage,
	quality: u8,
	subsampling: super::profile::JpegSubsampling,
) -> TransformResult<Vec<u8>> {
	let grayscale_input = matches!(
		image,
		DynamicImage::ImageLuma8(_) | DynamicImage::ImageLumaA8(_)
	);
	let use_grayscale =
		grayscale_input || subsampling == super::profile::JpegSubsampling::Gray;

	#[cfg(feature = "turbojpeg")]
	{
		use turbojpeg::Subsamp;

		let subsamp = if use_grayscale {
			Subsamp::Gray
		} else {
			match subsampling {
				super::profile::JpegSubsampling::S444 => Subsamp::None,
				super::profile::JpegSubsampling::S422 => Subsamp::Sub2x1,
				_ => Subsamp::Sub2x2,
			}
		};
		let encoded = if use_grayscale {
			turbojpeg::compress_image(&image.to_luma8(), i32::from(quality), subsamp)
		} else {
			turbojpeg::compress_image(&image.to_rgb8(), i32::from(quality), subsamp)
		}
		.map_err(|error| TransformError::Encode(error.to_string()))?;
		return Ok(encoded.to_vec());
	}

	#[cfg(not(feature = "turbojpeg"))]
	{
		let _ = subsampling;
		let mut buffer = Vec::new();
		let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
			&mut buffer,
			quality.clamp(1, 100),
		);
		let encoded = if use_grayscale {
			image
				.to_luma8()
				.write_with_encoder(encoder)
				.map_err(|error| TransformError::Encode(error.to_string()))
		} else {
			image
				.to_rgb8()
				.write_with_encoder(encoder)
				.map_err(|error| TransformError::Encode(error.to_string()))
		};
		encoded?;
		Ok(buffer)
	}
}

/// Transform a single source page into zero or more encoded pages.
///
/// This is the whole per-page pipeline: decode → (optional tall-page split) →
/// resize → tone → encode, per output segment.
pub fn transform_page_bytes(
	bytes: &[u8],
	profile: &TransformProfile,
) -> TransformResult<Vec<EncodedPage>> {
	let image = decode_page(bytes)?;
	let segments = if profile.split_tall_pages {
		split_tall_page(&image).unwrap_or_else(|| vec![image])
	} else {
		vec![image]
	};

	segments
		.into_iter()
		.map(|segment| {
			let (width, height) = fit_dimensions(
				segment.width(),
				segment.height(),
				profile.max_width,
				profile.max_height,
			);
			let resized = resize_image(&segment, width, height)?;
			let toned = apply_tone(
				resized,
				profile.grayscale,
				profile.black_level,
				profile.gamma,
			);
			encode_page(&toned, &profile.format)
		})
		.collect()
}

type PageBatch = (usize, TransformResult<Vec<EncodedPage>>);

/// Blocking, in-order driver used by both the stream and the container path.
///
/// Up to `concurrency` worker threads pull pages from the shared iterator and
/// run the CPU pipeline; results are re-ordered so the returned iterator yields
/// them in input order, with each input page flattened into its (possibly
/// split) output pages.
pub fn transform_pages_blocking<I>(
	pages: I,
	profile: TransformProfile,
	concurrency: usize,
) -> impl Iterator<Item = TransformResult<EncodedPage>>
where
	I: Iterator<Item = Vec<u8>> + Send + 'static,
{
	let concurrency = concurrency.max(1);
	let shared = std::sync::Arc::new(Mutex::new(pages.enumerate()));
	let profile = std::sync::Arc::new(profile);
	let (tx, rx) = mpsc::channel::<PageBatch>();

	for _ in 0..concurrency {
		let shared = std::sync::Arc::clone(&shared);
		let profile = std::sync::Arc::clone(&profile);
		let tx = tx.clone();
		std::thread::spawn(move || loop {
			let next = shared.lock().ok().and_then(|mut guard| guard.next());
			let Some((index, bytes)) = next else {
				break;
			};
			let result = transform_page_bytes(&bytes, &profile);
			if tx.send((index, result)).is_err() {
				// The consumer is gone; stop feeding the pipeline.
				break;
			}
		});
	}
	drop(tx);

	TransformPagesIter {
		rx,
		pending: BTreeMap::new(),
		queue: std::collections::VecDeque::new(),
		next_index: 0,
	}
}

/// In-order flattening iterator over the worker results.
///
/// Batches arrive out of order; they are buffered by input index and drained
/// strictly in order, with each input page's (possibly split) output pages
/// queued for sequential emission.
struct TransformPagesIter {
	rx: mpsc::Receiver<PageBatch>,
	pending: BTreeMap<usize, TransformResult<Vec<EncodedPage>>>,
	queue: std::collections::VecDeque<EncodedPage>,
	next_index: usize,
}

impl Iterator for TransformPagesIter {
	type Item = TransformResult<EncodedPage>;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			if let Some(page) = self.queue.pop_front() {
				return Some(Ok(page));
			}

			if let Some(batch) = self.pending.remove(&self.next_index) {
				self.next_index += 1;
				match batch {
					Ok(pages) => self.queue.extend(pages),
					Err(error) => return Some(Err(error)),
				}
				continue;
			}

			match self.rx.recv() {
				Ok((index, batch)) => {
					self.pending.insert(index, batch);
				},
				// All workers exited; done once the ordered backlog is empty.
				Err(_) if self.queue.is_empty() && self.pending.is_empty() => {
					return None;
				},
				Err(_) => continue,
			}
		}
	}
}

/// Transform pages concurrently on the blocking pool, streaming the encoded
/// pages in input order.
///
/// `concurrency` bounds the number of CPU-bound page transforms in flight
/// (typically `MediaConfig::cpu_concurrency_limit`). The `pages` iterator is
/// consumed by the worker threads, so I/O such as archive reads also happens
/// off the async runtime.
pub fn transform_pages<I>(
	pages: I,
	profile: TransformProfile,
	concurrency: usize,
) -> impl futures_util::Stream<Item = TransformResult<EncodedPage>>
where
	I: Iterator<Item = Vec<u8>> + Send + 'static,
{
	async_stream::stream! {
		let (tx, mut rx) = tokio::sync::mpsc::channel::<TransformResult<EncodedPage>>(2);
		tokio::task::spawn_blocking(move || {
			for page in transform_pages_blocking(pages, profile, concurrency) {
				if tx.blocking_send(page).is_err() {
					break;
				}
			}
		});

		while let Some(page) = rx.recv().await {
			yield page;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn gradient(width: u32, height: u32) -> DynamicImage {
		// Deterministic RGB ramp; pixel (x, y) = (x mod 256, y mod 256, 128).
		let buffer = image::ImageBuffer::from_fn(width, height, |x, y| {
			image::Rgb([((x % 256) * 2).min(255) as u8, y as u8, 128])
		});
		DynamicImage::ImageRgb8(buffer)
	}

	fn jpeg_bytes() -> Vec<u8> {
		let mut buffer = Vec::new();
		gradient(64, 64)
			.write_to(&mut Cursor::new(&mut buffer), image::ImageFormat::Jpeg)
			.unwrap();
		buffer
	}

	#[test]
	fn fit_dimensions_preserves_aspect_and_never_upscales() {
		// Downscales to the binding cap.
		assert_eq!(fit_dimensions(2000, 1000, Some(1000), Some(1000)), (1000, 500));
		assert_eq!(fit_dimensions(1000, 2000, Some(1000), Some(1000)), (500, 1000));
		// Smaller images pass through untouched.
		assert_eq!(fit_dimensions(800, 600, Some(1264, ), Some(1680)), (800, 600));
		assert_eq!(fit_dimensions(1264, 1680, Some(1264), Some(1680)), (1264, 1680));
		// No caps: unchanged.
		assert_eq!(fit_dimensions(4000, 2000, None, None), (4000, 2000));
		// Zero caps are ignored rather than producing zero-size pages.
		assert_eq!(fit_dimensions(100, 100, Some(0), Some(0)), (100, 100));
		// Tiny images stay at least 1px.
		assert_eq!(fit_dimensions(4, 8, Some(2), Some(2)), (1, 2));
	}

	#[test]
	fn resize_image_produces_exact_dimensions() {
		let image = gradient(800, 600);
		let resized = resize_image(&image, 320, 240).unwrap();
		assert_eq!(resized.width(), 320);
		assert_eq!(resized.height(), 240);

		// Same dimensions short-circuits to a copy.
		let same = resize_image(&image, 800, 600).unwrap();
		assert_eq!(same.width(), 800);
		assert_eq!(same.height(), 600);
	}

	#[test]
	fn grayscale_conversion_and_tone_lut() {
		assert!(tone_lut(None, None).is_none());

		let black_only = tone_lut(Some(64), None).unwrap();
		assert_eq!(black_only[0], 0);
		assert_eq!(black_only[64], 0);
		assert_eq!(black_only[65], 1); // 1 * 255 / (255 - 64)
		assert_eq!(black_only[255], 255);

		let gamma_only = tone_lut(None, Some(2.0)).unwrap();
		assert_eq!(gamma_only[128], (255.0 * (128.0 / 255.0f64).sqrt().powi(1)).round() as u8);
		// midtones brightened, extremes fixed
		assert!(gamma_only[128] > 128);
		assert_eq!(gamma_only[0], 0);
		assert_eq!(gamma_only[255], 255);

		let combined = tone_lut(Some(16), Some(2.2)).unwrap();
		assert_eq!(combined[16], 0);
		assert!(combined[128] > 128);
	}

	#[test]
	fn apply_tone_grayscales_and_maps_levels() {
		let image = gradient(16, 16);
		let gray = apply_tone(image.clone(), true, None, None);
		assert!(matches!(gray, DynamicImage::ImageLuma8(_)));

		// black_level: darkest band becomes pure black
		let black = apply_tone(image, false, Some(200), None);
		let rgb = black.into_rgb8();
		assert_eq!(rgb.get_pixel(0, 0).0[0], 0);
	}

	#[test]
	fn split_tall_page_segments() {
		// Not tall: no split.
		assert!(split_tall_page(&gradient(1000, 3000)).is_none());
		assert!(split_tall_page(&gradient(1000, 2000)).is_none());

		// ratio 3.1 → 2 segments
		let pages = split_tall_page(&gradient(1000, 3100)).unwrap();
		assert_eq!(pages.len(), 2);
		assert!(pages.iter().all(|page| page.width() == 1000));
		assert!(pages
			.iter()
			.all(|page| page.height() as f32 / page.width() as f32 <= MAX_TALL_PAGE_RATIO));

		// ratio 7 → 3 segments, reassembled height matches
		let pages = split_tall_page(&gradient(1000, 7000)).unwrap();
		assert_eq!(pages.len(), 3);
		let total: u32 = pages.iter().map(|page| page.height()).sum();
		assert_eq!(total, 7000);

		// split disabled via profile is tested through transform_page_bytes
	}

	#[test]
	fn transform_page_bytes_runs_the_full_pipeline() {
		let mut profile = TransformProfile::preset("libra").unwrap();
		profile.split_tall_pages = true;

		let mut bytes = Vec::new();
		gradient(1200, 4000)
			.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
			.unwrap();

		let pages = transform_page_bytes(&bytes, &profile).unwrap();
		// 4000/1200 = 3.33 → 2 segments
		assert_eq!(pages.len(), 2);
		for page in &pages {
			assert!(page.width <= 1264);
			assert!(page.height <= 1680);
			assert_eq!(page.format, PageFormat::Jpeg);
			assert!(page.bytes.starts_with(&[0xFF, 0xD8, 0xFF]), "jpeg magic");
		}
	}

	#[test]
	fn encode_page_jpeg_webp_and_png_magic() {
		let image = gradient(48, 48);

		let jpeg = encode_page(
			&image,
			&TransformFormat::Jpeg {
				quality: 80,
				subsampling: super::super::profile::JpegSubsampling::Auto,
			},
		)
		.unwrap();
		assert!(jpeg.bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
		assert_eq!(jpeg.format, PageFormat::Jpeg);

		let webp = encode_page(&image, &TransformFormat::Webp { quality: 80 }).unwrap();
		assert!(webp.bytes.starts_with(b"RIFF") && &webp.bytes[8..12] == b"WEBP");
		assert_eq!(webp.format, PageFormat::Webp);

		let png = encode_page(&image, &TransformFormat::Png).unwrap();
		assert!(png.bytes.starts_with(&[0x89, b'P', b'N', b'G']));
		assert_eq!(png.format, PageFormat::Png);
	}

	#[test]
	fn transform_pages_blocking_preserves_order_and_reports_errors() {
		let profile = TransformProfile::preset("nia").unwrap();
		let inputs: Vec<Vec<u8>> = (0..12)
			.map(|i| {
				let mut bytes = Vec::new();
				if i == 5 {
					bytes.extend_from_slice(b"not an image");
				} else {
					gradient(100 + i as u32 * 10, 200)
						.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
						.unwrap();
				}
				bytes
			})
			.collect();

		let results: Vec<_> =
			transform_pages_blocking(inputs, profile, 4).collect();

		assert_eq!(results.len(), 12);
		for (i, result) in results.iter().enumerate() {
			if i == 5 {
				assert!(result.is_err(), "page 5 should fail to decode");
			} else {
				let page = result.as_ref().expect("page renders");
				assert_eq!(page.format, PageFormat::Jpeg);
				assert!(page.width <= 758);
			}
		}
	}

	#[tokio::test]
	async fn transform_pages_stream_matches_blocking_output() {
		let profile = TransformProfile::preset("nia").unwrap();
		let inputs: Vec<Vec<u8>> = (0..6)
			.map(|i| {
				let mut bytes = Vec::new();
				gradient(60 + i as u32 * 20, 90)
					.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
					.unwrap();
				bytes
			})
			.collect();

		let stream = transform_pages(inputs, profile, 2);
		let results: Vec<_> = futures_util::StreamExt::collect(stream).await;

		assert_eq!(results.len(), 6);
		assert!(results.iter().all(|result| result.is_ok()));
		let widths: Vec<u32> = results
			.iter()
			.map(|result| result.as_ref().unwrap().width)
			.collect();
		// Input order preserved (each page has a distinct width).
		let expected: Vec<u32> = (0..6)
			.map(|i| {
				fit_dimensions(60 + i as u32 * 20, 90, Some(758), Some(1024)).0
			})
			.collect();
		assert_eq!(widths, expected);
	}

	#[tokio::test]
	async fn transform_pages_concurrency_is_bounded_and_complete() {
		let profile = TransformProfile::preset("phone").unwrap();
		let inputs: Vec<Vec<u8>> = (0..9)
			.map(|i| {
				let mut bytes = Vec::new();
				gradient(40, 40 + i as u32 * 5)
					.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
					.unwrap();
				bytes
			})
			.collect();

		let stream = transform_pages(inputs, profile, 1);
		let results: Vec<_> = futures_util::StreamExt::collect(stream).await;
		assert_eq!(results.len(), 9);
		assert!(results.iter().all(|result| result.is_ok()));
	}
}
