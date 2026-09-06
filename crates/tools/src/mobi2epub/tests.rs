//! `mobi2epub` fixtures.
//!
//! The input is the committed `.azw3` in `stump_media`'s fixture data, which
//! `boko convert` produced from a small EPUB: it is the only way to exercise
//! the real KF8 skeleton/fragment tables, and no converter that can be
//! installed in this tree writes one at test time. Everything the conversion
//! produces is then checked against `epub-check` and re-opened with
//! `stump_media`'s own EPUB processor, so "it converted" means "Stump can read
//! what came out".

use std::{
	fs::File,
	io::Read,
	path::{Path, PathBuf},
};

use tempfile::TempDir;
use zip::ZipArchive;

use super::*;
use crate::{epub_check, NoopProgress};

/// The Kindle fixture: four sections (a cover page plus three chapters), two
/// PNG resources, one style-sheet flow and a three-entry NCX.
fn azw3_fixture() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("../media/integration-tests/data/kindle-formats.azw3")
}

/// Copy the fixture into `dir` so a conversion never writes next to the
/// checked-in data.
fn staged(dir: &TempDir, name: &str) -> PathBuf {
	let target = dir.path().join(name);
	std::fs::copy(azw3_fixture(), &target).expect("stage fixture");
	target
}

fn convert_fixture(dir: &TempDir, name: &str) -> (PathBuf, Report) {
	let source = staged(dir, name);
	let input = ToolInput::new(vec![source.clone()]);
	let plan = Mobi2Epub.plan(&input).expect("plan");
	assert_eq!(plan.actions.len(), 1, "{:?}", plan.warnings);
	let target = plan.actions[0].target.clone().expect("target");
	assert!(!target.exists(), "plan must not write anything");

	let report = Mobi2Epub.apply(&plan, &mut NoopProgress).expect("apply");
	assert_eq!(report.applied.len(), 1, "{report:?}");
	assert!(report.skipped.is_empty(), "{report:?}");
	assert!(report.warnings.is_empty(), "{report:?}");
	(target, report)
}

fn entries(path: &Path) -> Vec<String> {
	let mut archive = ZipArchive::new(File::open(path).expect("open")).expect("zip");
	(0..archive.len())
		.map(|index| archive.by_index(index).expect("entry").name().to_owned())
		.collect()
}

fn entry_text(path: &Path, name: &str) -> String {
	let mut archive = ZipArchive::new(File::open(path).expect("open")).expect("zip");
	let mut entry = archive.by_name(name).expect("entry");
	let mut text = String::new();
	entry.read_to_string(&mut text).expect("utf-8 entry");
	text
}

// ---------------------------------------------------------------------------
// Registration and planning
// ---------------------------------------------------------------------------

#[test]
fn the_tool_is_registered_under_its_id() {
	let tool = crate::find(ID).expect("mobi2epub is registered");
	assert_eq!(tool.id(), "mobi2epub");
	assert!(!tool.describe().is_empty());
}

#[test]
fn a_plan_names_the_target_and_writes_nothing() {
	let dir = TempDir::new().expect("temp dir");
	let source = staged(&dir, "Book.azw3");
	let plan = Mobi2Epub
		.plan(&ToolInput::new(vec![source.clone()]))
		.expect("plan");

	assert_eq!(plan.actions.len(), 1);
	let action = &plan.actions[0];
	assert_eq!(action.kind, ACTION_CONVERT);
	assert_eq!(action.source.as_deref(), Some(source.as_path()));
	assert_eq!(
		action.target.as_deref(),
		Some(dir.path().join("Book.epub").as_path())
	);

	let detail: ConvertDetail =
		serde_json::from_value(action.detail.clone()).expect("detail");
	assert!(detail.kf8);
	assert_eq!(detail.sections, 4);
	assert_eq!(detail.resources, 2);
	assert_eq!(detail.flows, 1);
	assert_eq!(detail.navigation, 3);
	assert_eq!(detail.title, "Kindle Formats Explained");

	assert!(!dir.path().join("Book.epub").exists());
}

#[test]
fn a_folder_of_books_converts_and_a_non_book_is_ignored() {
	let dir = TempDir::new().expect("temp dir");
	staged(&dir, "One.azw3");
	staged(&dir, "Two.mobi");
	std::fs::write(dir.path().join("notes.txt"), b"not a book").expect("write");
	let nested = dir.path().join("nested");
	std::fs::create_dir(&nested).expect("mkdir");
	std::fs::copy(azw3_fixture(), nested.join("Three.azw")).expect("copy");

	let plan = Mobi2Epub
		.plan(&ToolInput::new(vec![dir.path().to_path_buf()]))
		.expect("plan");
	assert_eq!(
		plan.actions
			.iter()
			.map(|action| action.source.clone().unwrap())
			.collect::<Vec<_>>(),
		vec![dir.path().join("One.azw3"), dir.path().join("Two.mobi")],
		"a shallow run stops at the top level"
	);

	let plan = Mobi2Epub
		.plan(
			&ToolInput::new(vec![dir.path().to_path_buf()])
				.with_options(serde_json::json!({ "recursive": true })),
		)
		.expect("plan");
	assert_eq!(plan.actions.len(), 3);
	assert!(plan.actions.iter().any(
		|action| action.source.as_deref() == Some(nested.join("Three.azw").as_path())
	));
}

#[test]
fn an_unreadable_book_is_a_warning_not_a_failure() {
	let dir = TempDir::new().expect("temp dir");
	let good = staged(&dir, "Good.azw3");
	let broken = dir.path().join("Broken.mobi");
	std::fs::write(&broken, b"this is not a palm database").expect("write");

	let plan = Mobi2Epub
		.plan(&ToolInput::new(vec![dir.path().to_path_buf()]))
		.expect("plan");
	assert_eq!(
		plan.actions
			.iter()
			.map(|action| action.source.clone().unwrap())
			.collect::<Vec<_>>(),
		vec![broken.clone(), good.clone()]
			.into_iter()
			.filter(|path| *path == good)
			.collect::<Vec<_>>()
	);
	let warning = plan
		.warnings
		.iter()
		.find(|warning| warning.code == "unreadable")
		.expect("unreadable warning");
	assert_eq!(warning.path.as_deref(), Some(broken.as_path()));
	assert_eq!(warning.severity, Severity::Error);
}

#[test]
fn an_existing_target_is_only_replaced_with_overwrite() {
	let dir = TempDir::new().expect("temp dir");
	let source = staged(&dir, "Book.azw3");
	let target = dir.path().join("Book.epub");
	std::fs::write(&target, b"existing").expect("write");

	let plan = Mobi2Epub
		.plan(&ToolInput::new(vec![source.clone()]))
		.expect("plan");
	assert!(plan.actions.is_empty());
	assert_eq!(plan.warnings[0].code, "target-exists");
	assert_eq!(std::fs::read(&target).unwrap(), b"existing");

	let plan = Mobi2Epub
		.plan(
			&ToolInput::new(vec![source])
				.with_options(serde_json::json!({ "overwrite": true })),
		)
		.expect("plan");
	assert_eq!(plan.actions.len(), 1);
	let report = Mobi2Epub.apply(&plan, &mut NoopProgress).expect("apply");
	assert_eq!(report.applied.len(), 1);
	assert_ne!(std::fs::read(&target).unwrap(), b"existing");
}

#[test]
fn two_sources_that_would_write_one_target_are_reported() {
	let dir = TempDir::new().expect("temp dir");
	let output = TempDir::new().expect("output dir");
	staged(&dir, "Book.azw3");
	staged(&dir, "Book.mobi");

	let plan = Mobi2Epub
		.plan(
			&ToolInput::new(vec![dir.path().to_path_buf()])
				.with_options(serde_json::json!({ "output_dir": output.path() })),
		)
		.expect("plan");
	assert_eq!(plan.actions.len(), 1);
	assert_eq!(plan.warnings[0].code, "target-collision");
}

#[test]
fn apply_refuses_a_plan_from_another_tool() {
	let error = Mobi2Epub
		.apply(&Plan::new("epub2cbz"), &mut NoopProgress)
		.expect_err("plan mismatch");
	assert!(
		matches!(&error, ToolError::PlanMismatch { tool, plan } if tool == ID && plan == "epub2cbz"),
		"{error}"
	);
}

// ---------------------------------------------------------------------------
// The produced container
// ---------------------------------------------------------------------------

#[test]
fn the_epub_envelope_follows_the_ocf_rules() {
	let dir = TempDir::new().expect("temp dir");
	let (target, _) = convert_fixture(&dir, "Book.azw3");

	let names = entries(&target);
	assert_eq!(names[0], "mimetype", "mimetype must be the first entry");
	assert_eq!(
		names[1], "META-INF/container.xml",
		"the rootfile pointer comes next"
	);

	let mut archive = ZipArchive::new(File::open(&target).unwrap()).unwrap();
	let mimetype = archive.by_name("mimetype").unwrap();
	assert_eq!(mimetype.compression(), zip::CompressionMethod::Stored);
	// Local header (30 bytes) + the name, and no extra field: the same
	// invariant `epub-check`'s own writer test asserts.
	assert_eq!(mimetype.data_start(), 30 + 8);
	drop(mimetype);

	assert_eq!(
		entry_text(&target, "mimetype"),
		"application/epub+zip",
		"exact media type, no trailing newline"
	);

	let container = entry_text(&target, "META-INF/container.xml");
	assert!(
		container.contains("full-path=\"OEBPS/content.opf\""),
		"{container}"
	);
}

#[test]
fn every_section_flow_and_resource_is_in_the_manifest_and_the_archive() {
	let dir = TempDir::new().expect("temp dir");
	let (target, _) = convert_fixture(&dir, "Book.azw3");

	let names = entries(&target);
	for expected in [
		"OEBPS/content.opf",
		"OEBPS/nav.xhtml",
		"OEBPS/toc.ncx",
		"OEBPS/part0000.html",
		"OEBPS/part0001.html",
		"OEBPS/part0002.html",
		"OEBPS/part0003.html",
		"OEBPS/styles/style0001.css",
		"OEBPS/images/image0000.png",
		"OEBPS/images/image0001.png",
	] {
		assert!(
			names.contains(&expected.to_string()),
			"{expected} missing from {names:?}"
		);
	}

	let opf = entry_text(&target, "OEBPS/content.opf");
	assert!(
		opf.contains("<dc:title>Kindle Formats Explained</dc:title>"),
		"{opf}"
	);
	assert!(opf.contains("<dc:creator>Ada Palmer</dc:creator>"), "{opf}");
	assert!(opf.contains("<dc:language>en</dc:language>"), "{opf}");
	assert!(opf.contains("properties=\"nav\""), "{opf}");
	assert!(opf.contains("properties=\"cover-image\""), "{opf}");
	assert!(opf.contains("media-type=\"text/css\""), "{opf}");
	assert!(opf.contains("<spine toc=\"ncx\">"), "{opf}");
	// Four spine items, in reading order.
	assert_eq!(opf.matches("<itemref").count(), 4);
	let first = opf.find("idref=\"s0000\"").expect("first section");
	let last = opf.find("idref=\"s0003\"").expect("last section");
	assert!(first < last, "{opf}");

	// Every content document's links resolve against the package's own
	// directory, so a section reference and an image reference are relative.
	let chapter = entry_text(&target, "OEBPS/part0001.html");
	assert!(chapter.contains("styles/style0001.css"), "{chapter}");
	assert!(!chapter.contains("kindle:"), "{chapter}");
	let cover_page = entry_text(&target, "OEBPS/part0000.html");
	assert!(cover_page.contains("images/image0000.png"), "{cover_page}");
}

#[test]
fn navigation_comes_from_the_books_own_index() {
	let dir = TempDir::new().expect("temp dir");
	let (target, _) = convert_fixture(&dir, "Book.azw3");

	let nav = entry_text(&target, "OEBPS/nav.xhtml");
	assert!(nav.contains("epub:type=\"toc\""), "{nav}");
	for (title, href) in [
		("The Palm Database", "part0001.html"),
		("Compression", "part0002.html"),
		("Kindle Format 8", "part0003.html"),
	] {
		assert!(
			nav.contains(&format!("<a href=\"{href}\">{title}</a>")),
			"{nav}"
		);
	}

	let ncx = entry_text(&target, "OEBPS/toc.ncx");
	assert_eq!(ncx.matches("<navPoint").count(), 3, "{ncx}");
	assert!(ncx.contains("playOrder=\"1\""), "{ncx}");
	assert!(ncx.contains("<text>Kindle Format 8</text>"), "{ncx}");
}

#[test]
fn the_produced_epub_passes_epub_check_with_no_findings() {
	let dir = TempDir::new().expect("temp dir");
	let (target, _) = convert_fixture(&dir, "Book.azw3");

	let audit = epub_check::audit(&target).expect("audit");
	assert!(
		audit.findings.is_empty(),
		"epub-check findings: {:#?}",
		audit.findings
	);
	assert!(audit.repair.is_none(), "nothing should need repairing");
}

#[test]
fn the_produced_epub_opens_with_the_epub_processor() {
	let dir = TempDir::new().expect("temp dir");
	let (target, _) = convert_fixture(&dir, "Book.azw3");
	let path = target.to_string_lossy().to_string();

	let doc = stump_media::EpubProcessor::open(&path).expect("open as epub");
	assert_eq!(doc.spine.len(), 4);
	assert_eq!(
		doc.mdata("title").map(|item| item.value.as_str()),
		Some("Kindle Formats Explained")
	);

	// The cover the Kindle header pointed at is the cover Stump serves.
	let (content_type, cover) =
		stump_media::EpubProcessor::get_cover(&path).expect("cover");
	assert_eq!(content_type, stump_media::ContentType::PNG);
	let mut book = stump_media::MobiBook::open(&azw3_fixture()).expect("open azw3");
	let (_, original) = book.cover().expect("kindle cover");
	assert_eq!(cover, original);

	// The metadata survives the round trip through the processor's own reader.
	let metadata =
		<stump_media::EpubProcessor as stump_media::FileProcessor>::process_metadata(
			&path,
		)
		.expect("metadata")
		.expect("some metadata");
	assert_eq!(metadata.title.as_deref(), Some("Kindle Formats Explained"));
	assert_eq!(metadata.writers, Some(vec!["Ada Palmer".to_string()]));

	// And the chapter text is readable through the spine.
	let (_, chapter) =
		stump_media::EpubProcessor::get_chapter(&path, 1).expect("chapter");
	assert!(
		String::from_utf8_lossy(&chapter).contains("A Palm database stores"),
		"{}",
		String::from_utf8_lossy(&chapter)
	);
}

#[test]
fn a_failed_conversion_leaves_no_partial_file() {
	let dir = TempDir::new().expect("temp dir");
	let source = staged(&dir, "Book.azw3");
	let plan = Mobi2Epub
		.plan(&ToolInput::new(vec![source.clone()]))
		.expect("plan");
	let target = plan.actions[0].target.clone().expect("target");

	// Truncating the source between plan and apply is the realistic failure:
	// the reader refuses it and the staged temp file must be gone.
	File::options()
		.write(true)
		.open(&source)
		.expect("open source")
		.set_len(64)
		.expect("truncate");

	let report = Mobi2Epub.apply(&plan, &mut NoopProgress).expect("apply");
	assert!(report.applied.is_empty(), "{report:?}");
	assert_eq!(report.skipped.len(), 1);
	assert!(!target.exists(), "no half-written EPUB");
	let leftovers = std::fs::read_dir(dir.path())
		.expect("read dir")
		.filter_map(Result::ok)
		.map(|entry| entry.file_name().to_string_lossy().into_owned())
		.filter(|name| name.contains("stump-tools"))
		.collect::<Vec<_>>();
	assert!(
		leftovers.is_empty(),
		"temp files left behind: {leftovers:?}"
	);
}

#[test]
fn a_moved_source_is_skipped_rather_than_half_converted() {
	let dir = TempDir::new().expect("temp dir");
	let source = staged(&dir, "Book.azw3");
	let plan = Mobi2Epub
		.plan(&ToolInput::new(vec![source.clone()]))
		.expect("plan");
	std::fs::remove_file(&source).expect("remove source");

	let report = Mobi2Epub.apply(&plan, &mut NoopProgress).expect("apply");
	assert!(report.applied.is_empty());
	assert_eq!(report.skipped.len(), 1);
	assert!(report.skipped[0].1.contains("no longer a file"));
}

// ---------------------------------------------------------------------------
// Package rules
// ---------------------------------------------------------------------------

#[test]
fn xml_predefined_entities_are_escaped_in_the_metadata() {
	assert_eq!(
		escape("A & B <c> \"d\" 'e'"),
		"A &amp; B &lt;c&gt; &quot;d&quot; &apos;e&apos;"
	);
	assert_eq!(escape("plain"), "plain");
}

#[test]
fn a_navigation_entry_past_the_last_section_is_clamped() {
	let dir = TempDir::new().expect("temp dir");
	let source = staged(&dir, "Book.azw3");
	let book = MobiBook::open(&source).expect("open");
	let mut package = Package::of(&book);
	package.navigation = vec![MobiNavEntry {
		title: "Off the end".to_string(),
		section: 99,
		fragment: "aid".to_string(),
		children: Vec::new(),
	}];

	assert_eq!(package.href_of(&package.navigation[0]), "part0003.html#aid");
	let nav = package.nav();
	assert!(nav.contains("part0003.html#aid"), "{nav}");
}

#[test]
fn a_book_with_no_navigation_index_gets_one_entry_per_section() {
	let dir = TempDir::new().expect("temp dir");
	let source = staged(&dir, "Book.azw3");
	let book = MobiBook::open(&source).expect("open");
	let mut package = Package::of(&book);
	package.navigation.clear();

	let entries = package.entries();
	assert_eq!(entries.len(), 4);
	assert_eq!(
		entries
			.iter()
			.map(|entry| entry.title.clone())
			.collect::<Vec<_>>(),
		vec![
			"Cover",
			"The Palm Database",
			"Compression",
			"Kindle Format 8"
		]
	);
	let ncx = package.ncx();
	assert_eq!(ncx.matches("<navPoint").count(), 4, "{ncx}");
}

#[test]
fn a_nested_navigation_tree_becomes_a_nested_nav_and_a_flat_ncx() {
	let child = |title: &str, section: usize| MobiNavEntry {
		title: title.to_string(),
		section,
		fragment: String::new(),
		children: Vec::new(),
	};
	let dir = TempDir::new().expect("temp dir");
	let source = staged(&dir, "Book.azw3");
	let book = MobiBook::open(&source).expect("open");
	let mut package = Package::of(&book);
	package.navigation = vec![MobiNavEntry {
		children: vec![child("Chapter", 2)],
		..child("Part", 1)
	}];

	let nav = package.nav();
	// The child list opens inside the parent's `<li>`.
	let parent = nav.find("Part</a>").expect("parent");
	let nested = nav[parent..].find("<ol>").expect("nested list");
	let close = nav[parent..].find("</li>").expect("parent close");
	assert!(nested < close, "{nav}");

	// The NCX is flat, and the child still gets its own play order.
	let ncx = package.ncx();
	assert_eq!(ncx.matches("<navPoint").count(), 2, "{ncx}");
	assert!(ncx.contains("playOrder=\"2\""), "{ncx}");
	assert!(ncx.contains("<text>Chapter</text>"), "{ncx}");
}

#[test]
fn an_unknown_resource_type_becomes_an_octet_stream() {
	assert_eq!(
		media_type_of(ContentType::UNKNOWN),
		"application/octet-stream"
	);
	assert_eq!(media_type_of(ContentType::PNG), "image/png");
}

#[test]
fn options_reject_an_unknown_key() {
	let input = ToolInput::new(vec![PathBuf::from("/tmp")])
		.with_options(serde_json::json!({ "nope": true }));
	let error = input.parse_options::<Mobi2EpubOptions>().unwrap_err();
	assert!(matches!(error, ToolError::Options(_)), "{error}");
}

#[test]
fn a_run_with_no_paths_is_refused() {
	let error = Mobi2Epub.plan(&ToolInput::default()).expect_err("no paths");
	assert!(matches!(error, ToolError::Invalid(_)), "{error}");
}

#[test]
fn a_folder_with_no_kindle_books_is_reported() {
	let dir = TempDir::new().expect("temp dir");
	std::fs::write(dir.path().join("notes.txt"), b"nothing here").expect("write");
	let plan = Mobi2Epub
		.plan(&ToolInput::new(vec![dir.path().to_path_buf()]))
		.expect("plan");
	assert!(plan.actions.is_empty());
	assert_eq!(plan.warnings[0].code, "no-books");
	assert_eq!(plan.warnings[0].severity, Severity::Error);
}
