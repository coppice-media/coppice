//! Comic page transform pipeline: resize, tone, and re-encode comic pages for
//! a target device, then package them as a stored CBZ or a fixed-layout
//! KEPUB.
//!
//! The pipeline never upscales, never mutates the source, and is driven by a
//! [`TransformProfile`] — either a named device preset (`clara`, `libra`,
//! …) or a fully custom profile deserialized from a device's stored
//! `transform_profile` JSON.
//!
//! Feature switches (default on, like `pdf`/`rar`):
//!
//! - `fast-resize` — SIMD Lanczos3 downscaling via `fast_image_resize`;
//!   otherwise the `image` crate's CatmullRom convolution.
//! - `turbojpeg` — JPEG encoding via libjpeg-turbo; otherwise the `image`
//!   crate's JPEG encoder.
//! - `kepub` — the fixed-layout KEPUB container via `stump_kepub`; otherwise
//!   KEPUB output is a typed [`TransformError::FeatureDisabled`].

pub mod cache;
pub mod container;
pub mod error;
pub mod pipeline;
pub mod profile;
pub mod source;

pub use cache::{CacheSweep, TransformCache};
pub use container::comic_info_xml;
pub use error::{TransformError, TransformResult};
pub use pipeline::{
	transform_page_bytes, transform_pages, transform_pages_blocking, EncodedPage,
	PageFormat,
};
pub use profile::{
	parse_bitrate, AudioOutput, AudioProfile, ComicContainer, JpegSubsampling,
	TransformFormat, TransformProfile, AUDIO_OGG, DEFAULT_OPUS_BITRATE,
	MAX_TALL_PAGE_RATIO,
};
pub use source::{is_comic_source, ComicPages, COMIC_EXTENSIONS};

#[cfg(test)]
mod benchmark_tests {
	//! Benchmark-style measurement: ms/page for the first 20 pages of the
	//! `science_comics_001.cbz` fixture at the Libra preset, single-threaded
	//! and at the machine's parallelism.
	//!
	//! Run with:
	//! `cargo test -p stump_media --release transform_bench -- --ignored --nocapture`
	//!
	//! `TRANSFORM_BENCH_CBZ=/path/to/other.cbz` measures another comic (the
	//! fixture's 480 px pages never hit the resize stage).

	use super::*;
	use crate::MediaConfig;
	use std::io::Cursor;
	use std::time::Instant;

	#[test]
	#[ignore = "benchmark; run with --ignored --nocapture"]
	fn transform_bench_ms_per_page_libra() {
		let fixture = std::env::var("TRANSFORM_BENCH_CBZ")
			.unwrap_or_else(|_| crate::tests::get_test_cbz_path());
		let profile = TransformProfile::preset("libra").expect("libra preset");
		let config = MediaConfig::default();
		let parallelism = std::thread::available_parallelism().map_or(1, |n| n.get());

		let source_size = std::fs::metadata(&fixture).expect("fixture readable").len();
		let comic = source::ComicPages::open(std::path::Path::new(&fixture), &config)
			.expect("fixture is a comic");
		let total_pages = comic.page_count();

		// Read the source pages up front so the timing covers the transform
		// alone, not archive I/O.
		let source_pages: Vec<Vec<u8>> = comic
			.into_iter(&config)
			.take(total_pages.min(20))
			.collect::<TransformResult<_>>()
			.expect("fixture pages read");
		let page_count = source_pages.len();
		let source_page_bytes: u64 =
			source_pages.iter().map(|page| page.len() as u64).sum();

		let run = |concurrency: usize| {
			let started = Instant::now();
			let pages: Vec<EncodedPage> = pipeline::transform_pages_blocking(
				source_pages.clone().into_iter().map(Ok),
				profile.clone(),
				concurrency,
			)
			.collect::<TransformResult<_>>()
			.expect("all pages transform");
			(started.elapsed(), pages)
		};
		let (serial_elapsed, _) = run(1);
		let (parallel_elapsed, pages) = run(parallelism);

		let transformed_bytes: u64 =
			pages.iter().map(|page| page.bytes.len() as u64).sum();
		let (first_width, first_height) = pages
			.first()
			.map_or((0, 0), |page| (page.width, page.height));

		// Package the transformed pages so the delivered size is comparable.
		let started = Instant::now();
		let mut sink = Cursor::new(Vec::new());
		container::build_kepub(pages.into_iter().map(Ok), "benchmark", &mut sink)
			.expect("kepub builds");
		let package_elapsed = started.elapsed();
		let kepub_size = sink.get_ref().len() as u64;

		let kib = |bytes: u64| bytes as f64 / 1024.0;
		let ms = |elapsed: std::time::Duration| elapsed.as_secs_f64() * 1000.0;
		println!("=== comic transform benchmark (Libra preset) ===");
		println!("source            : {fixture}");
		println!("pages transformed : {page_count}/{total_pages} (source total)");
		println!("first page        : {first_width}x{first_height} px after transform");
		println!(
			"1 worker          : {:.1} ms total, {:.2} ms/page",
			ms(serial_elapsed),
			ms(serial_elapsed) / page_count as f64
		);
		println!(
			"{parallelism} workers         : {:.1} ms total, {:.2} ms/page",
			ms(parallel_elapsed),
			ms(parallel_elapsed) / page_count as f64
		);
		println!("package time      : {:.1} ms", ms(package_elapsed));
		println!(
			"source file       : {source_size} bytes ({:.1} KiB)",
			kib(source_size)
		);
		println!(
			"source pages      : {source_page_bytes} bytes ({:.1} KiB)",
			kib(source_page_bytes)
		);
		println!(
			"transformed pages : {transformed_bytes} bytes ({:.1} KiB)",
			kib(transformed_bytes)
		);
		println!(
			"kepub container   : {kepub_size} bytes ({:.1} KiB)",
			kib(kepub_size)
		);
	}
}
