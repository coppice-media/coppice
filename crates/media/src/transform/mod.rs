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
	ComicContainer, JpegSubsampling, TransformFormat, TransformProfile,
	MAX_TALL_PAGE_RATIO,
};
pub use source::{is_comic_source, ComicPages, COMIC_EXTENSIONS};

#[cfg(test)]
mod benchmark_tests {
	//! Benchmark-style measurement: ms/page for the first 20 pages of the
	//! `science_comics_001.cbz` fixture at the Libra preset.
	//!
	//! Run with:
	//! `cargo test -p stump_media transform_bench -- --ignored --nocapture`

	use super::*;
	use crate::MediaConfig;
	use std::io::Cursor;
	use std::time::Instant;

	#[test]
	#[ignore = "benchmark; run with --ignored --nocapture"]
	fn transform_bench_ms_per_page_libra() {
		let fixture = crate::tests::get_test_cbz_path();
		let profile = TransformProfile::preset("libra").expect("libra preset");
		let config = MediaConfig::default();

		let source_bytes = std::fs::read(&fixture).expect("fixture readable");
		let source_size = source_bytes.len() as u64;

		let comic = source::ComicPages::open(std::path::Path::new(&fixture), &config)
			.expect("fixture is a comic");
		let total_pages = comic.page_count();
		let page_limit = total_pages.min(20);

		let started = Instant::now();
		let results: Vec<TransformResult<EncodedPage>> = pipeline::transform_pages_blocking(
			comic.into_iter(&config).take(page_limit),
			profile.clone(),
			config.cpu_concurrency_limit().max(1),
		)
		.collect();
		let elapsed = started.elapsed();

		let pages: Vec<_> = results
			.into_iter()
			.collect::<Result<Vec<_>, _>>()
			.expect("all pages transform");

		let transformed_bytes: u64 = pages.iter().map(|page| page.bytes.len() as u64).sum();
		let ms_per_page = elapsed.as_secs_f64() * 1000.0 / pages.len() as f64;

		// Package the transformed pages so the delivered size is comparable.
		let mut sink = Cursor::new(Vec::new());
		container::build_kepub(
			pages.into_iter().map(Ok),
			"Science Comics (Ace) no.1 (Libra)",
			&mut sink,
		)
		.expect("kepub builds");
		let kepub_size = sink.get_ref().len() as u64;

		println!("=== comic transform benchmark (Libra preset) ===");
		println!("pages transformed : {}/{} (source total)", pages.len(), total_pages);
		println!("wall time         : {:.1} ms", elapsed.as_secs_f64() * 1000.0);
		println!("ms/page           : {ms_per_page:.2}");
		println!("source size       : {source_size} bytes ({source_size:.2} KiB)");
		println!("sum of pages      : {transformed_bytes} bytes");
		println!("kepub container   : {kepub_size} bytes ({kepub_size:.2} KiB)");
	}
}
