use std::io::{Cursor, Read, Write};

use pretty_assertions::assert_eq;
use stump_media::ContentType;
use tempfile::TempDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use super::*;
use crate::NoopProgress;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A PNG page with per-pixel noise, so it is genuinely photographic: a flat
/// colour would compress to nothing in every format and make a size assertion
/// meaningless.
fn png_page(width: u32, height: u32, seed: u32) -> Vec<u8> {
	let image = image::RgbImage::from_fn(width, height, |x, y| {
		let noise = (x * 7 + y * 13 + seed * 31) % 256;
		image::Rgb([(x % 256) as u8, (y % 256) as u8, noise as u8])
	});
	let mut bytes = Vec::new();
	image::DynamicImage::ImageRgb8(image)
		.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
		.expect("encode png");

	bytes
}

/// A real WebP page, encoded by the same pipeline the tool uses.
fn webp_page(width: u32, height: u32, seed: u32) -> Vec<u8> {
	let profile = TransformProfile {
		format: TransformFormat::Webp { quality: 80 },
		container: ComicContainer::Cbz,
		..Default::default()
	};

	transform_page_bytes(&png_page(width, height, seed), &profile)
		.expect("encode webp")
		.remove(0)
		.bytes
}

/// Write a CBZ holding `entries` in the given order, stored.
fn write_archive(path: &Path, entries: &[(&str, Vec<u8>)]) {
	let file = std::fs::File::create(path).expect("create archive");
	let mut zip = ZipWriter::new(file);
	let options =
		SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
	for (name, bytes) in entries {
		zip.start_file(*name, options).expect("start entry");
		zip.write_all(bytes).expect("write entry");
	}
	zip.finish().expect("finish archive");
}

/// Every entry of an archive as `(name, bytes)`, in archive order.
fn read_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
	let mut archive =
		ZipArchive::new(std::fs::File::open(path).expect("open archive")).expect("zip");
	(0..archive.len())
		.map(|index| {
			let mut entry = archive.by_index(index).expect("entry");
			let name = entry.name().to_string();
			let mut bytes = Vec::new();
			entry.read_to_end(&mut bytes).expect("read entry");
			(name, bytes)
		})
		.collect()
}

fn entry_names(path: &Path) -> Vec<String> {
	read_entries(path)
		.into_iter()
		.map(|(name, _)| name)
		.collect()
}

const COMIC_INFO: &[u8] = br#"<?xml version="1.0"?>
<ComicInfo><Series>Berserk</Series><Number>1</Number></ComicInfo>"#;

/// A three-page comic with metadata and a non-page member.
fn comic(dir: &Path, name: &str) -> PathBuf {
	let path = dir.join(name);
	write_archive(
		&path,
		&[
			(util::COMIC_INFO_ENTRY, COMIC_INFO.to_vec()),
			("0001.png", png_page(400, 600, 1)),
			("0002.png", png_page(400, 600, 2)),
			("0003.png", png_page(400, 600, 3)),
			(".stump-covers.json", b"{\"version\":1}".to_vec()),
		],
	);

	path
}

fn options(quality: u8) -> serde_json::Value {
	serde_json::json!({ "quality": quality })
}

fn detail_of(action: &Action) -> serde_json::Value {
	action.detail.clone()
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[test]
fn plan_names_every_page_and_writes_nothing() {
	let dir = TempDir::new().expect("temp dir");
	let source = comic(dir.path(), "Berserk v01.cbz");
	let before = std::fs::read(&source).expect("read source");

	let input = ToolInput::new(vec![source.clone()]).with_options(options(80));
	let plan = WebpConvert.plan(&input).expect("plan");

	assert_eq!(plan.tool, ID);
	assert_eq!(plan.actions.len(), 1);
	let action = &plan.actions[0];
	assert_eq!(action.kind, ACTION_CONVERT);
	assert_eq!(action.source.as_deref(), Some(source.as_path()));
	// The suffix is bracketed so the scanner still reads volume 1 (see
	// `converted_name_still_parses_as_the_same_volume`).
	assert_eq!(
		action.target.as_deref(),
		Some(dir.path().join("Berserk v01 [webp].cbz").as_path())
	);

	let detail = detail_of(action);
	assert_eq!(
		detail["pages"]
			.as_array()
			.expect("pages")
			.iter()
			.map(|page| page["target"].as_str().expect("target"))
			.collect::<Vec<_>>(),
		vec!["0001.webp", "0002.webp", "0003.webp"]
	);
	assert_eq!(detail["kept_webp"], 0);
	assert_eq!(detail["quality"], 80);

	// A plan is a dry run.
	assert!(!dir.path().join("Berserk v01 [webp].cbz").exists());
	assert_eq!(std::fs::read(&source).expect("read source"), before);
}

#[test]
fn an_all_webp_archive_is_reported_and_not_planned() {
	let dir = TempDir::new().expect("temp dir");
	let source = dir.path().join("Done.cbz");
	write_archive(
		&source,
		&[
			("0001.webp", webp_page(80, 120, 1)),
			("0002.WEBP", webp_page(80, 120, 2)),
		],
	);

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![source]).with_options(options(80)))
		.expect("plan");

	assert!(plan.is_empty());
	assert_eq!(
		plan.warnings
			.iter()
			.map(|warning| warning.code.as_str())
			.collect::<Vec<_>>(),
		vec!["already-webp"]
	);
}

#[test]
fn an_extension_collision_leaves_the_archive_alone() {
	let dir = TempDir::new().expect("temp dir");
	let source = dir.path().join("Ambiguous.cbz");
	write_archive(
		&source,
		&[
			("0001.png", png_page(40, 60, 1)),
			("0001.jpg", png_page(40, 60, 2)),
		],
	);

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![source]).with_options(options(80)))
		.expect("plan");

	assert!(plan.is_empty());
	assert!(plan
		.warnings
		.iter()
		.any(|warning| warning.code == "page-name-collision"));
}

#[test]
fn a_folder_is_walked_only_when_recursive() {
	let dir = TempDir::new().expect("temp dir");
	comic(dir.path(), "Top.cbz");
	let nested = dir.path().join("v02");
	std::fs::create_dir(&nested).expect("mkdir");
	comic(&nested, "Deep.cbz");

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![dir.path().to_path_buf()]).with_options(options(80)))
		.expect("plan");
	assert_eq!(plan.actions.len(), 1);

	let plan = WebpConvert
		.plan(
			&ToolInput::new(vec![dir.path().to_path_buf()])
				.with_options(serde_json::json!({ "recursive": true })),
		)
		.expect("plan");
	assert_eq!(plan.actions.len(), 2);
}

#[test]
fn an_existing_target_needs_overwrite() {
	let dir = TempDir::new().expect("temp dir");
	let source = comic(dir.path(), "Berserk v01.cbz");
	std::fs::write(dir.path().join("Berserk v01 [webp].cbz"), b"old").expect("write");

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![source.clone()]).with_options(options(80)))
		.expect("plan");
	assert!(plan.is_empty());
	assert!(plan
		.warnings
		.iter()
		.any(|warning| warning.code == "target-exists"));

	let plan = WebpConvert
		.plan(
			&ToolInput::new(vec![source])
				.with_options(serde_json::json!({ "overwrite": true })),
		)
		.expect("plan");
	assert_eq!(plan.actions.len(), 1);
}

#[test]
fn bad_options_are_rejected_before_any_work() {
	let dir = TempDir::new().expect("temp dir");
	let source = comic(dir.path(), "Berserk v01.cbz");

	let error = WebpConvert
		.plan(&ToolInput::new(vec![source.clone()]).with_options(options(0)))
		.expect_err("quality 0");
	assert!(matches!(error, ToolError::Options(_)), "{error}");

	let error = WebpConvert
		.plan(
			&ToolInput::new(vec![source]).with_options(serde_json::json!({
				"in_place": true,
				"output_dir": dir.path(),
			})),
		)
		.expect_err("contradictory options");
	assert!(matches!(error, ToolError::Invalid(_)), "{error}");

	let error = WebpConvert
		.plan(&ToolInput::new(vec![]))
		.expect_err("no paths");
	assert!(matches!(error, ToolError::Invalid(_)), "{error}");
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

#[test]
fn pages_become_webp_and_the_archive_shrinks() {
	let dir = TempDir::new().expect("temp dir");
	let source = comic(dir.path(), "Berserk v01.cbz");
	let before = std::fs::read(&source).expect("read source");

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![source.clone()]).with_options(options(80)))
		.expect("plan");
	let report = WebpConvert.apply(&plan, &mut NoopProgress).expect("apply");

	assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);
	assert!(report.skipped.is_empty());

	let target = dir.path().join("Berserk v01 [webp].cbz");
	let entries = read_entries(&target);
	assert_eq!(
		entries
			.iter()
			.map(|(name, _)| name.as_str())
			.collect::<Vec<_>>(),
		vec![
			util::COMIC_INFO_ENTRY,
			"0001.webp",
			"0002.webp",
			"0003.webp",
			".stump-covers.json",
		],
		"page order and non-page members must survive"
	);

	for (name, bytes) in &entries {
		if !name.ends_with(".webp") {
			continue;
		}
		assert_eq!(
			ContentType::from_bytes(bytes),
			ContentType::WEBP,
			"{name} is not a WebP image"
		);
		assert_eq!(ContentType::from_bytes(bytes).mime_type(), "image/webp");
	}

	// Metadata and the covers manifest are copied byte for byte.
	assert_eq!(entries[0].1, COMIC_INFO.to_vec());
	assert_eq!(entries[4].1, b"{\"version\":1}".to_vec());

	let detail = detail_of(&report.applied[0]);
	assert_eq!(detail["pages"], 3);
	assert_eq!(detail["converted"], 3);
	assert_eq!(detail["kept_webp"], 0);
	let before_bytes = detail["before_bytes"].as_u64().expect("before_bytes");
	let after_bytes = detail["after_bytes"].as_u64().expect("after_bytes");
	assert_eq!(before_bytes, before.len() as u64);
	assert_eq!(after_bytes, std::fs::metadata(&target).unwrap().len());
	assert!(
		after_bytes < before_bytes,
		"WebP pages must shrink the archive: {after_bytes} >= {before_bytes}"
	);
	assert_eq!(
		detail["saved_bytes"].as_i64().expect("saved_bytes"),
		before_bytes as i64 - after_bytes as i64
	);

	// The source is never touched.
	assert_eq!(std::fs::read(&source).expect("read source"), before);
}

#[test]
fn an_already_webp_page_is_copied_not_re_encoded() {
	let dir = TempDir::new().expect("temp dir");
	let source = dir.path().join("Mixed.cbz");
	let webp = webp_page(200, 300, 7);
	write_archive(
		&source,
		&[
			("0001.webp", webp.clone()),
			("0002.png", png_page(200, 300, 8)),
		],
	);

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![source]).with_options(options(80)))
		.expect("plan");
	assert_eq!(detail_of(&plan.actions[0])["kept_webp"], 1);

	let report = WebpConvert.apply(&plan, &mut NoopProgress).expect("apply");
	assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);

	let entries = read_entries(&dir.path().join("Mixed [webp].cbz"));
	assert_eq!(
		entries
			.iter()
			.map(|(name, _)| name.as_str())
			.collect::<Vec<_>>(),
		vec!["0001.webp", "0002.webp"]
	);
	assert_eq!(
		entries[0].1, webp,
		"an already-WebP page must be copied verbatim, never re-encoded"
	);
	assert_eq!(detail_of(&report.applied[0])["kept_webp"], 1);
}

#[test]
fn max_dimensions_downscale_and_never_upscale() {
	let dir = TempDir::new().expect("temp dir");
	let source = dir.path().join("Big.cbz");
	write_archive(
		&source,
		&[
			("0001.png", png_page(400, 600, 1)),
			("0002.png", png_page(50, 75, 2)),
		],
	);

	let plan = WebpConvert
		.plan(
			&ToolInput::new(vec![source])
				.with_options(serde_json::json!({ "max_width": 100 })),
		)
		.expect("plan");
	WebpConvert.apply(&plan, &mut NoopProgress).expect("apply");

	let entries = read_entries(&dir.path().join("Big [webp].cbz"));
	let sizes = entries
		.iter()
		.map(|(_, bytes)| {
			let image = image::load_from_memory(bytes).expect("decode webp");
			(image.width(), image.height())
		})
		.collect::<Vec<_>>();
	assert_eq!(sizes, vec![(100, 150), (50, 75)]);
}

#[test]
fn in_place_keeps_a_backup_of_the_original() {
	let dir = TempDir::new().expect("temp dir");
	let source = comic(dir.path(), "Berserk v01.cbz");
	let before = std::fs::read(&source).expect("read source");

	let plan = WebpConvert
		.plan(
			&ToolInput::new(vec![source.clone()])
				.with_options(serde_json::json!({ "in_place": true })),
		)
		.expect("plan");
	assert_eq!(plan.actions[0].target.as_deref(), Some(source.as_path()));

	let report = WebpConvert.apply(&plan, &mut NoopProgress).expect("apply");
	assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);

	let backup = dir.path().join("Berserk v01.cbz.bak");
	assert_eq!(
		std::fs::read(&backup).expect("read backup"),
		before,
		"the backup must hold the original bytes"
	);
	assert_eq!(
		entry_names(&source),
		vec![
			util::COMIC_INFO_ENTRY.to_string(),
			"0001.webp".to_string(),
			"0002.webp".to_string(),
			"0003.webp".to_string(),
			".stump-covers.json".to_string(),
		]
	);
	// Re-running over the converted archive now finds nothing to do, so the
	// backup is only ever guarded for an archive that would be rewritten.
	let plan = WebpConvert
		.plan(
			&ToolInput::new(vec![source])
				.with_options(serde_json::json!({ "in_place": true })),
		)
		.expect("plan");
	assert!(plan.is_empty());

	let second = comic(dir.path(), "Berserk v02.cbz");
	std::fs::write(dir.path().join("Berserk v02.cbz.bak"), b"older").expect("write");
	let plan = WebpConvert
		.plan(
			&ToolInput::new(vec![second])
				.with_options(serde_json::json!({ "in_place": true })),
		)
		.expect("plan");
	assert!(plan.is_empty(), "an existing backup blocks an in-place run");
	assert!(plan
		.warnings
		.iter()
		.any(|warning| warning.code == "backup-exists"));
}

#[test]
fn an_undecodable_page_skips_the_file_and_keeps_the_original() {
	let dir = TempDir::new().expect("temp dir");
	let source = dir.path().join("Broken.cbz");
	write_archive(
		&source,
		&[
			("0001.png", png_page(80, 120, 1)),
			("0002.png", b"not an image at all".to_vec()),
		],
	);
	let before = std::fs::read(&source).expect("read source");

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![source.clone()]).with_options(options(80)))
		.expect("plan");
	let report = WebpConvert.apply(&plan, &mut NoopProgress).expect("apply");

	assert!(report.applied.is_empty());
	assert_eq!(report.skipped.len(), 1);
	let (_, reason) = &report.skipped[0];
	assert!(reason.contains("decode"), "{reason}");
	assert!(
		!dir.path().join("Broken [webp].cbz").exists(),
		"a failed conversion must not leave an archive behind"
	);
	assert_eq!(std::fs::read(&source).expect("read source"), before);
}

#[test]
fn converted_name_still_parses_as_the_same_volume() {
	let dir = TempDir::new().expect("temp dir");
	let source = comic(dir.path(), "Berserk v01 c003.cbz");

	let plan = WebpConvert
		.plan(&ToolInput::new(vec![source]).with_options(options(80)))
		.expect("plan");
	let target = plan.actions[0].target.clone().expect("target");
	let name = target.file_name().expect("name").to_string_lossy();

	let volume =
		stump_scanner::parse_identifier_of(&name, stump_scanner::SequenceKind::Volume)
			.expect("volume parses");
	let chapter =
		stump_scanner::parse_identifier_of(&name, stump_scanner::SequenceKind::Chapter)
			.expect("chapter parses");
	assert_eq!(volume.start.integer_part(), 1);
	assert_eq!(chapter.start.integer_part(), 3);
}

#[test]
fn apply_refuses_a_plan_from_another_tool() {
	let error = WebpConvert
		.apply(&Plan::new("cbzit"), &mut NoopProgress)
		.expect_err("plan mismatch");
	assert!(
		matches!(&error, ToolError::PlanMismatch { tool, plan } if tool == ID && plan == "cbzit"),
		"{error:?}"
	);
}
