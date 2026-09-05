use data_encoding::HEXLOWER;
use ring::digest::{Context, SHA256};
use std::io::{self, Read, Seek};
use tracing::debug;

// use std::fs::File;
#[cfg(target_family = "unix")]
use std::os::unix::prelude::FileExt;

#[cfg(target_family = "windows")]
use std::os::windows::prelude::*;

pub const HASH_SAMPLE_SIZE: u64 = 10000;
pub const HASH_SAMPLE_COUNT: u64 = 4;

fn read(file: &std::fs::File, offset: u64, size: u64) -> Result<Vec<u8>, io::Error> {
	let mut buffer = vec![0u8; size as usize];

	#[cfg(target_family = "unix")]
	{
		file.read_at(&mut buffer, offset)?;
	}

	#[cfg(target_family = "windows")]
	{
		file.seek_read(&mut buffer, offset)?;
	}

	Ok(buffer)
}

pub fn generate(path: &str, bytes: u64) -> Result<String, io::Error> {
	let file = std::fs::File::open(path)?;

	let mut ring_context = Context::new(&SHA256);

	if bytes <= HASH_SAMPLE_SIZE * HASH_SAMPLE_COUNT {
		let buffer = read(&file, 0, bytes)?;
		ring_context.update(&buffer);
	} else {
		for i in 0..HASH_SAMPLE_COUNT {
			let offset = (bytes / HASH_SAMPLE_COUNT) * i;
			let buffer = read(&file, offset, HASH_SAMPLE_SIZE)?;
			ring_context.update(&buffer);
		}

		let offset = bytes - HASH_SAMPLE_SIZE;
		let buffer = read(&file, offset, HASH_SAMPLE_SIZE)?;

		ring_context.update(&buffer);
	}

	let digest = ring_context.finish();

	let encoded_digest = HEXLOWER.encode(digest.as_ref());

	debug!("Generated checksum: {:?}", encoded_digest);

	Ok(encoded_digest)
}

/// Generate a hash for a file using a port of the Koreader hash algorithm, which is
/// originally written in Lua. The algorithm reads the file in 1KB chunks, starting
/// from the beginning, until it reaches the end of the file or 10 iterations. It isn't
/// overly complex.
///
/// See https://github.com/koreader/koreader/blob/master/frontend/util.lua#L1046-L1072
#[tracing::instrument(fields(path = %path.as_ref().display()))]
pub fn generate_koreader_hash<P: AsRef<std::path::Path>>(
	path: P,
) -> Result<String, io::Error> {
	let mut file = std::fs::File::open(path)?;

	let mut md5_context = md5::Context::new();

	let step = 1024i64;
	let size = 1024i64;

	let mut buffer = vec![0u8; size as usize];
	for i in -1..=10 {
		let offset = if i == -1 { 0 } else { step << (2 * i) };
		file.seek(std::io::SeekFrom::Start(offset as u64))?;

		let bytes_read = file.read(&mut buffer)?;
		if bytes_read == 0 {
			tracing::trace!(?offset, "Reached end of file");
			break;
		}

		// KOReader's util.partialMD5 hashes only the bytes actually read
		// (`frontend/util.lua`): a short final sample must not be zero-padded,
		// otherwise files whose size falls inside a sample window get a hash
		// KOReader never presents.
		md5_context.consume(&buffer[..bytes_read]);
	}

	let hash = format!("{:x}", md5_context.finalize());
	tracing::debug!(hash = %hash, "Generated hash");

	Ok(hash)
}

/// Width of the difference grid: 9 luma columns yield 8 horizontal gradients.
const DHASH_COLUMNS: u32 = 9;
/// Height of the difference grid.
const DHASH_ROWS: u32 = 8;

/// Difference hash (dHash, 8×8) of an already decoded image.
///
/// The image is reduced to a 9×8 grid of box-averaged luma values; each of the
/// 64 output bits is set when a cell is brighter than its right-hand
/// neighbour. Box averaging over the full cell (rather than point sampling)
/// keeps the hash stable across re-encoding, resizing, and mild JPEG noise.
pub fn dhash_image(image: &image::DynamicImage) -> u64 {
	let luma = image.to_luma8();
	let (width, height) = luma.dimensions();
	if width == 0 || height == 0 {
		return 0;
	}
	let pixels = luma.as_raw();
	let mut grid = [[0u32; DHASH_COLUMNS as usize]; DHASH_ROWS as usize];
	for (row, cells) in grid.iter_mut().enumerate() {
		let row = row as u32;
		let y0 = (row * height / DHASH_ROWS) as usize;
		let y1 = (((row + 1) * height / DHASH_ROWS).max(row * height / DHASH_ROWS + 1))
			.min(height) as usize;
		for (column, cell) in cells.iter_mut().enumerate() {
			let column = column as u32;
			let x0 = (column * width / DHASH_COLUMNS) as usize;
			let x1 = (((column + 1) * width / DHASH_COLUMNS)
				.max(column * width / DHASH_COLUMNS + 1))
			.min(width) as usize;
			let mut sum = 0u64;
			for y in y0..y1 {
				let line = &pixels[y * width as usize..(y + 1) * width as usize];
				sum += line[x0..x1].iter().map(|&p| u64::from(p)).sum::<u64>();
			}
			let count = ((y1 - y0) * (x1 - x0)) as u64;
			*cell = (sum / count) as u32;
		}
	}
	let mut hash = 0u64;
	for cells in &grid {
		for column in 0..(DHASH_COLUMNS as usize - 1) {
			hash = (hash << 1) | u64::from(cells[column] > cells[column + 1]);
		}
	}
	hash
}

/// Decode an encoded page image and compute its [`dhash_image`].
pub fn page_dhash(bytes: &[u8]) -> Result<u64, image::ImageError> {
	Ok(dhash_image(&image::load_from_memory(bytes)?))
}

/// Number of differing bits between two page hashes.
#[inline]
pub fn hamming(left: u64, right: u64) -> u32 {
	(left ^ right).count_ones()
}

#[cfg(test)]
mod dhash_tests {
	use super::{dhash_image, hamming, page_dhash};
	use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb};
	use std::io::Cursor;

	/// A page-like gradient with a dark band, so neighbouring cells differ.
	fn page(width: u32, height: u32, band: u32) -> DynamicImage {
		DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| {
			if (band..band + height / 10).contains(&y) {
				Rgb([20, 20, 20])
			} else {
				let v = ((x * 255) / width) as u8;
				Rgb([v, 255 - v, (y % 255) as u8])
			}
		}))
	}

	fn encode(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
		let mut out = Cursor::new(Vec::new());
		image.write_to(&mut out, format).unwrap();
		out.into_inner()
	}

	#[test]
	fn reencoded_and_resized_pages_stay_close() {
		let original = page(600, 900, 300);
		let base = dhash_image(&original);

		let jpeg = page_dhash(&encode(&original, ImageFormat::Jpeg)).unwrap();
		assert!(hamming(base, jpeg) <= 2, "jpeg drift {}", hamming(base, jpeg));

		let resized = original.resize_exact(300, 450, image::imageops::FilterType::Triangle);
		let png = page_dhash(&encode(&resized, ImageFormat::Png)).unwrap();
		assert!(hamming(base, png) <= 2, "resize drift {}", hamming(base, png));
	}

	#[test]
	fn different_pages_are_far_apart() {
		let a = dhash_image(&page(600, 900, 300));
		let b = dhash_image(&page(600, 900, 700).fliph());
		assert!(hamming(a, b) > 10, "distance {}", hamming(a, b));
		let blank = dhash_image(&DynamicImage::new_rgb8(64, 64));
		assert_eq!(blank, 0);
		assert!(hamming(a, blank) > 10);
	}

	#[test]
	fn tiny_images_do_not_panic() {
		let one = dhash_image(&DynamicImage::new_luma8(1, 1));
		assert_eq!(one, 0);
		let _ = dhash_image(&DynamicImage::new_luma8(3, 2));
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

	fn epub_path() -> PathBuf {
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("integration-tests/data/leaves.epub")
	}

	fn pdf_path() -> PathBuf {
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("integration-tests/data/tall.pdf")
	}

	// https://github.com/koreader/koreader/blob/master/spec/unit/util_spec.lua#L339-L341
	#[test]
	fn test_koreader_hash_epub() {
		assert_eq!(
			generate_koreader_hash(epub_path()).unwrap(),
			"59d481d168cca6267322f150c5f6a2a3".to_string()
		)
	}

	// https://github.com/koreader/koreader/blob/master/spec/unit/util_spec.lua#L342-L344
	#[test]
	fn test_koreader_hash_pdf() {
		assert_eq!(
			generate_koreader_hash(pdf_path()).unwrap(),
			"41cce710f34e5ec21315e19c99821415".to_string()
		)
	}
}

#[cfg(test)]
mod koreader_partial_md5 {
	use super::generate_koreader_hash;
	use std::io::Write;

	/// A 5000-byte file is sampled at offsets 0, 1024 and 4096; the last read
	/// returns 904 bytes. KOReader hashes exactly those bytes. The expected
	/// value is what `md5` produces over the concatenation of the sampled
	/// ranges, i.e. what KOReader's `util.partialMD5` reports for this file.
	#[test]
	fn short_final_sample_is_not_zero_padded() {
		let mut file = tempfile::NamedTempFile::new().unwrap();
		let bytes: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
		file.write_all(&bytes).unwrap();
		file.flush().unwrap();

		let mut expected = md5::Context::new();
		expected.consume(&bytes[0..1024]);
		expected.consume(&bytes[1024..2048]);
		expected.consume(&bytes[4096..5000]);
		let expected = format!("{:x}", expected.finalize());

		assert_eq!(generate_koreader_hash(file.path()).unwrap(), expected);
	}
}
