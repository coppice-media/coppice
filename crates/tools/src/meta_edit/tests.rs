use std::io::Write;

use pretty_assertions::assert_eq;
use stump_media::{EpubProcessor, FileProcessor};
use stump_scanner::{parse_identifier_of, SequenceKind, SequenceNumber};
use tempfile::TempDir;
use zip::{write::SimpleFileOptions, ZipWriter};

use super::*;
use crate::NoopProgress;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-page-bytes";

/// A hand-made `ComicInfo.xml`: a comment, an extra namespace, an element in
/// that namespace, and three elements (`Penciller`, `Notes`, `Web`)
/// `meta-edit` has no opinion about. Every byte of those has to survive an
/// edit, entity references included — quick-xml 0.38 reports `&amp;` as its
/// own `GeneralRef` event rather than part of the text.
const RICH_COMIC_INFO: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:mm="urn:x-stump-tools-test:1">
  <!-- hand-edited, keep this -->
  <Title>Old Title</Title>
  <Series>Old Series</Series>
  <Number>1</Number>
  <Volume>1</Volume>
  <Penciller>Kentaro Miura</Penciller>
  <Notes>Tom &amp; Jerry &lt;scan&gt;</Notes>
  <Publisher>Old House</Publisher>
  <Web>https://example.invalid/berserk?a=1&amp;b=2</Web>
  <mm:Source mm:confidence="high">scan</mm:Source>
  <LanguageISO>en</LanguageISO>
</ComicInfo>"#;

/// The package metadata of the EPUB fixture: the same starting values as
/// [`RICH_COMIC_INFO`], stored the way calibre stores them.
const EPUB_METADATA: &str = r#"    <dc:title>Old Title</dc:title>
    <dc:creator opf:role="aut">Kentaro Miura</dc:creator>
    <dc:publisher>Old House</dc:publisher>
    <dc:language>en</dc:language>
    <dc:date>1990-11-26</dc:date>
    <meta name="calibre:series" content="Old Series"/>
    <meta name="calibre:series_index" content="1"/>"#;

const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Contents</title></head>
<body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">One</a></li></ol></nav></body>
</html>"#;

const CHAPTER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head>
<body><p>Hello.</p></body></html>"#;

const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
	<rootfiles>
		<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
	</rootfiles>
</container>"#;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn write_archive(path: &Path, entries: &[(&str, &[u8], CompressionMethod)]) {
	let mut writer = ZipWriter::new(File::create(path).unwrap());
	for (name, bytes, method) in entries {
		writer
			.start_file(
				*name,
				SimpleFileOptions::default().compression_method(*method),
			)
			.unwrap();
		writer.write_all(bytes).unwrap();
	}
	writer.finish().unwrap();
}

fn write_cbz(path: &Path, comic_info: Option<&str>, pages: &[(&str, &[u8])]) {
	let mut entries = Vec::new();
	if let Some(xml) = comic_info {
		entries.push((
			util::COMIC_INFO_ENTRY,
			xml.as_bytes(),
			CompressionMethod::Stored,
		));
	}
	entries.extend(
		pages
			.iter()
			.map(|(name, bytes)| (*name, *bytes, CompressionMethod::Stored)),
	);
	write_archive(path, &entries);
}

/// A minimal EPUB 3 that [`EpubProcessor::open`] accepts, with `metadata` as
/// the package document's metadata block.
fn write_epub(path: &Path, metadata: &str) {
	let opf = format!(
		r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="bookid">urn:uuid:fixture-0000-0000-0000-000000000001</dc:identifier>
{metadata}
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
	);

	write_archive(
		path,
		&[
			(
				"mimetype",
				b"application/epub+zip",
				CompressionMethod::Stored,
			),
			(
				CONTAINER_ENTRY,
				CONTAINER.as_bytes(),
				CompressionMethod::Deflated,
			),
			(
				"OEBPS/content.opf",
				opf.as_bytes(),
				CompressionMethod::Deflated,
			),
			(
				"OEBPS/nav.xhtml",
				NAV.as_bytes(),
				CompressionMethod::Deflated,
			),
			(
				"OEBPS/ch1.xhtml",
				CHAPTER.as_bytes(),
				CompressionMethod::Deflated,
			),
		],
	);
}

fn entry_bytes(path: &Path, name: &str) -> Vec<u8> {
	let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
	let mut bytes = Vec::new();
	archive
		.by_name(name)
		.unwrap()
		.read_to_end(&mut bytes)
		.unwrap();
	bytes
}

fn entry_names(path: &Path) -> Vec<String> {
	let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
	(0..archive.len())
		.map(|index| archive.by_index(index).unwrap().name().to_owned())
		.collect()
}

fn entry_text_of(path: &Path, name: &str) -> String {
	String::from_utf8(entry_bytes(path, name)).unwrap()
}

fn plan_for(paths: Vec<PathBuf>, options: serde_json::Value) -> Plan {
	MetaEdit
		.plan(&ToolInput::new(paths).with_options(options))
		.unwrap()
}

fn plan_and_apply(path: &Path, options: serde_json::Value) -> Report {
	let plan = plan_for(vec![path.to_path_buf()], options);
	MetaEdit.apply(&plan, &mut NoopProgress).unwrap()
}

fn detail_of(plan: &Plan, index: usize) -> EditDetail {
	serde_json::from_value(plan.actions[index].detail.clone()).unwrap()
}

fn codes_of(warnings: &[Warning]) -> Vec<&str> {
	warnings
		.iter()
		.map(|warning| warning.code.as_str())
		.collect()
}

fn change(field: Field, from: Option<&str>, to: &str) -> FieldChange {
	FieldChange {
		field,
		from: from.map(str::to_string),
		to: to.to_string(),
	}
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[test]
fn plan_writes_nothing_and_reports_only_the_changed_fields() {
	let dir = TempDir::new().unwrap();
	let cbz = dir.path().join("Berserk v01 c001.cbz");
	write_cbz(&cbz, Some(RICH_COMIC_INFO), &[("0001.png", PNG)]);
	let epub = dir.path().join("Berserk v01.epub");
	write_epub(&epub, EPUB_METADATA);
	let before = (std::fs::read(&cbz).unwrap(), std::fs::read(&epub).unwrap());

	let plan = plan_for(
		vec![cbz.clone(), epub.clone()],
		serde_json::json!({
			"set": {
				"series": "Berserk",
				"title": "The Brand",
				"publisher": "Old House",
			}
		}),
	);

	assert_eq!(
		(std::fs::read(&cbz).unwrap(), std::fs::read(&epub).unwrap()),
		before,
		"plan is pure: not one byte of either container may move"
	);
	assert_eq!(codes_of(&plan.warnings), Vec::<&str>::new());
	assert_eq!(plan.actions.len(), 2);

	let comic = detail_of(&plan, 0);
	assert_eq!(plan.actions[0].kind, ACTION_EDIT);
	assert_eq!(comic.container, Container::ComicInfo);
	assert_eq!(comic.entry, util::COMIC_INFO_ENTRY);
	assert_eq!(comic.rename, None);
	assert_eq!(
		comic.changes,
		vec![
			change(Field::Series, Some("Old Series"), "Berserk"),
			change(Field::Title, Some("Old Title"), "The Brand"),
		],
		"the publisher already matches, so it is not a change"
	);

	let package = detail_of(&plan, 1);
	assert_eq!(package.container, Container::Opf);
	assert_eq!(package.entry, "OEBPS/content.opf");
	assert_eq!(
		package.changes,
		vec![
			change(Field::Series, Some("Old Series"), "Berserk"),
			change(Field::Title, Some("Old Title"), "The Brand"),
		]
	);
}

#[test]
fn a_file_with_nothing_to_change_is_reported_and_gets_no_action() {
	let dir = TempDir::new().unwrap();
	let cbz = dir.path().join("book.cbz");
	write_cbz(&cbz, Some(RICH_COMIC_INFO), &[("0001.png", PNG)]);

	let plan = plan_for(
		vec![cbz.clone()],
		serde_json::json!({ "set": { "series": "Old Series", "language": "en" } }),
	);

	assert_eq!(plan.actions, vec![]);
	assert_eq!(codes_of(&plan.warnings), vec![codes::NO_CHANGES]);
	assert_eq!(plan.warnings[0].severity, Severity::Info);
}

#[test]
fn unusable_options_are_rejected_before_a_file_is_touched() {
	let dir = TempDir::new().unwrap();
	let cbz = dir.path().join("book.cbz");
	write_cbz(&cbz, None, &[("0001.png", PNG)]);

	for options in [
		serde_json::json!({ "set": { "volume": "3.5" } }),
		serde_json::json!({ "set": { "year": "nineteen-ninety" } }),
		serde_json::json!({ "set": { "number": "one" } }),
		serde_json::json!({ "set": { "series": "  " } }),
		serde_json::json!({}),
		serde_json::json!({
			"set": { "series": "Berserk" },
			"rename_from_metadata": true,
			"pattern": "{series} {arc}",
		}),
		serde_json::json!({
			"set": { "series": "Berserk" },
			"rename_from_metadata": true,
			"pattern": "{series",
		}),
	] {
		let error = MetaEdit
			.plan(&ToolInput::new(vec![cbz.clone()]).with_options(options.clone()))
			.expect_err(&format!("{options} must not plan"));
		assert!(
			matches!(error, ToolError::Options(_)),
			"{options} => {error:?}"
		);
	}
	assert_eq!(entry_names(&cbz), vec!["0001.png"]);
}

// ---------------------------------------------------------------------------
// ComicInfo
// ---------------------------------------------------------------------------

#[test]
fn an_edit_keeps_every_unknown_comic_info_byte_and_every_page() {
	let dir = TempDir::new().unwrap();
	let cbz = dir.path().join("book.cbz");
	write_cbz(
		&cbz,
		Some(RICH_COMIC_INFO),
		&[("0001.png", PNG), ("0002.png", b"second-page")],
	);
	let pages = (entry_bytes(&cbz, "0001.png"), entry_bytes(&cbz, "0002.png"));

	let report = plan_and_apply(
		&cbz,
		serde_json::json!({
			"set": {
				"series": "Berserk",
				"number": "10.5",
				"volume": "3",
				"title": "The Brand",
				"writer": "Kentaro Miura",
				"year": "1990",
				"language": "ja",
				"tags": ["seinen", "dark fantasy"],
			}
		}),
	);
	assert_eq!(report.applied.len(), 1);
	assert_eq!(report.skipped, vec![]);

	let xml = entry_text_of(&cbz, util::COMIC_INFO_ENTRY);
	assert!(
		xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"),
		"the declaration is kept verbatim: {xml}"
	);
	for kept in [
		"<!-- hand-edited, keep this -->",
		"xmlns:mm=\"urn:x-stump-tools-test:1\"",
		"<Penciller>Kentaro Miura</Penciller>",
		// Entity references in an element this tool does not own survive
		// intact: quick-xml 0.38 reports each one as its own `GeneralRef`.
		"<Notes>Tom &amp; Jerry &lt;scan&gt;</Notes>",
		"<Web>https://example.invalid/berserk?a=1&amp;b=2</Web>",
		"<mm:Source mm:confidence=\"high\">scan</mm:Source>",
		"<Publisher>Old House</Publisher>",
	] {
		assert!(xml.contains(kept), "{kept} must survive: {xml}");
	}
	assert!(!xml.contains("Old Title"), "{xml}");
	assert!(!xml.contains("Old Series"), "{xml}");
	assert!(
		xml.contains("<LanguageISO>ja</LanguageISO>"),
		"the schema element is patched in place: {xml}"
	);

	let parsed: ProcessedMediaMetadata = quick_xml::de::from_str(&xml).unwrap();
	assert_eq!(parsed.series.as_deref(), Some("Berserk"));
	assert_eq!(parsed.number, Some(10.5));
	assert_eq!(parsed.volume, Some(3));
	assert_eq!(parsed.title.as_deref(), Some("The Brand"));
	assert_eq!(parsed.year, Some(1990));
	assert_eq!(parsed.writers, Some(vec!["Kentaro Miura".to_string()]));
	assert_eq!(parsed.publisher.as_deref(), Some("Old House"));
	assert_eq!(
		parsed.tags,
		Some(vec!["seinen".to_string(), "dark fantasy".to_string()]),
		"tags are one comma-separated element, as stump_media splits them"
	);
	assert_eq!(
		parsed.pencillers,
		Some(vec!["Kentaro Miura".to_string()]),
		"an element this tool does not own still reads back"
	);

	assert_eq!(
		(entry_bytes(&cbz, "0001.png"), entry_bytes(&cbz, "0002.png")),
		pages,
		"pages are raw-copied, byte for byte"
	);
}

#[test]
fn a_cbz_without_comic_info_gains_one_carrying_the_set_fields() {
	let dir = TempDir::new().unwrap();
	let cbz = dir.path().join("book.cbz");
	write_cbz(&cbz, None, &[("0001.png", PNG)]);

	let plan = plan_for(
		vec![cbz.clone()],
		serde_json::json!({
			"set": {
				"series": "Berserk",
				"number": "007",
				"volume": "3",
				"title": "The Brand",
				"language": "ja",
				"tags": ["seinen"],
			}
		}),
	);
	assert_eq!(
		detail_of(&plan, 0).changes,
		vec![
			change(Field::Series, None, "Berserk"),
			change(Field::Number, None, "007"),
			change(Field::Volume, None, "3"),
			change(Field::Title, None, "The Brand"),
			change(Field::Language, None, "ja"),
			change(Field::Tags, None, "seinen"),
		],
		"an archive with no ComicInfo.xml has no current value for anything"
	);

	let report = MetaEdit.apply(&plan, &mut NoopProgress).unwrap();
	assert_eq!(report.applied.len(), 1);
	assert_eq!(report.skipped, vec![]);

	assert_eq!(
		entry_names(&cbz),
		vec![util::COMIC_INFO_ENTRY, "0001.png"],
		"a generated ComicInfo.xml goes first, where Stump's own writers put it"
	);
	let xml = entry_text_of(&cbz, util::COMIC_INFO_ENTRY);
	let parsed: ProcessedMediaMetadata = quick_xml::de::from_str(&xml).unwrap();
	assert_eq!(parsed.series.as_deref(), Some("Berserk"));
	assert_eq!(parsed.number, Some(7.0));
	assert_eq!(parsed.volume, Some(3));
	assert_eq!(parsed.title.as_deref(), Some("The Brand"));
	assert_eq!(parsed.tags, Some(vec!["seinen".to_string()]));
	assert!(
		xml.contains("<LanguageISO>ja</LanguageISO>"),
		"language is the schema element: {xml}"
	);
	assert!(
		xml.contains("<Number>007</Number>"),
		"a number keeps the operator's own formatting: {xml}"
	);
	assert_eq!(entry_bytes(&cbz, "0001.png"), PNG);
}

// ---------------------------------------------------------------------------
// EPUB
// ---------------------------------------------------------------------------

#[test]
fn an_epub_edit_reads_back_through_the_epub_processor() {
	let dir = TempDir::new().unwrap();
	let epub = dir.path().join("book.epub");
	write_epub(&epub, EPUB_METADATA);
	let chapter = entry_bytes(&epub, "OEBPS/ch1.xhtml");

	let report = plan_and_apply(
		&epub,
		serde_json::json!({
			"set": {
				"series": "Berserk",
				"number": "10.5",
				"volume": "3",
				"title": "The Brand",
				"language": "ja",
				"year": "1991",
				"tags": ["seinen", "dark fantasy"],
			}
		}),
	);
	assert_eq!(report.applied.len(), 1);
	assert_eq!(report.skipped, vec![]);
	assert_eq!(
		codes_of(&report.warnings),
		vec![codes::FIELD_UNSUPPORTED],
		"a package document has no volume, and the book's own values are left alone"
	);

	let path = epub.to_str().unwrap();
	EpubProcessor::open(path).expect("the edited epub still opens");
	let metadata = <EpubProcessor as FileProcessor>::process_metadata(path)
		.unwrap()
		.unwrap();
	assert_eq!(metadata.title.as_deref(), Some("The Brand"));
	assert_eq!(metadata.language.as_deref(), Some("ja"));
	assert_eq!(metadata.series.as_deref(), Some("Berserk"));
	assert_eq!(metadata.number, Some(10.5));
	assert_eq!(metadata.year, Some(1991));
	assert_eq!(
		metadata.genres,
		Some(vec!["seinen".to_string(), "dark fantasy".to_string()]),
		"one dc:subject per tag"
	);

	let opf = entry_text_of(&epub, "OEBPS/content.opf");
	assert!(
		opf.contains(r#"<dc:creator opf:role="aut">Kentaro Miura</dc:creator>"#),
		"an untouched element keeps its attributes: {opf}"
	);
	assert!(
		opf.contains(r#"<dc:identifier id="bookid">"#),
		"the identifier is untouched: {opf}"
	);
	assert!(
		opf.contains(r#"<dc:date>1991</dc:date>"#),
		"a changed year is written as the bare YYYY: {opf}"
	);
	assert!(
		opf.contains(r#"<meta name="calibre:series_index" content="10.5"/>"#),
		"series index is patched in place: {opf}"
	);
	assert_eq!(
		entry_bytes(&epub, "OEBPS/ch1.xhtml"),
		chapter,
		"content documents are raw-copied"
	);
}

#[test]
fn an_epub_without_a_package_document_is_warned_about_and_skipped() {
	let dir = TempDir::new().unwrap();
	let epub = dir.path().join("bare.epub");
	write_archive(
		&epub,
		&[(
			"mimetype",
			b"application/epub+zip",
			CompressionMethod::Stored,
		)],
	);

	let plan = plan_for(
		vec![epub.clone()],
		serde_json::json!({ "set": { "title": "The Brand" } }),
	);

	assert_eq!(plan.actions, vec![]);
	assert_eq!(codes_of(&plan.warnings), vec![codes::OPF_MISSING]);
}

// ---------------------------------------------------------------------------
// Renaming
// ---------------------------------------------------------------------------

#[test]
fn a_rename_from_metadata_round_trips_through_the_scanner_parser() {
	let dir = TempDir::new().unwrap();
	let source = dir.path().join("raw-scan-0001.cbz");
	write_cbz(&source, None, &[("0001.png", PNG)]);

	let report = plan_and_apply(
		&source,
		serde_json::json!({
			"set": {
				"series": "Berserk",
				"volume": "3",
				"number": "10.5",
				"title": "The Brand",
			},
			"rename_from_metadata": true,
		}),
	);
	assert_eq!(report.applied.len(), 1);
	assert_eq!(report.skipped, vec![]);
	assert!(!source.exists(), "the source was renamed, not copied");

	let renamed = dir.path().join("Berserk v03 c010.5 - The Brand.cbz");
	assert!(renamed.is_file(), "renamed from the default pattern");

	let name = renamed.file_name().unwrap().to_string_lossy().into_owned();
	assert_eq!(
		parse_identifier_of(&name, SequenceKind::Volume)
			.unwrap()
			.start,
		SequenceNumber::from_integer(3)
	);
	assert_eq!(
		parse_identifier_of(&name, SequenceKind::Chapter)
			.unwrap()
			.start,
		SequenceNumber::from_thousandths(10_500)
	);
}

#[test]
fn a_placeholder_with_no_value_drops_its_whole_chunk() {
	let dir = TempDir::new().unwrap();
	let source = dir.path().join("loose.cbz");
	write_cbz(&source, None, &[("0001.png", PNG)]);

	let report = plan_and_apply(
		&source,
		serde_json::json!({
			"set": { "series": "Berserk", "number": "7" },
			"rename_from_metadata": true,
		}),
	);
	assert_eq!(report.skipped, vec![]);
	assert!(
		dir.path().join("Berserk c007.cbz").is_file(),
		"no orphan `v` and no trailing separator, got {:?}",
		std::fs::read_dir(dir.path())
			.unwrap()
			.flatten()
			.map(|entry| entry.file_name())
			.collect::<Vec<_>>()
	);
}

#[test]
fn separator_chunks_only_survive_between_two_chunks_that_carry_something() {
	let source = "{series} - v{volume:02} - {title} [{language}]";
	let pattern = Pattern::parse(source).unwrap();
	let mut values = Values::default();
	values.set(Field::Series, "Berserk".to_string());
	values.set(Field::Volume, "3.5".to_string());
	values.set(Field::Title, "The Brand".to_string());
	values.set(Field::Language, "ja".to_string());
	assert_eq!(
		pattern.render(&values).as_deref(),
		Some("Berserk - v03.5 - The Brand [ja]"),
		"the integer part is padded and the decimal kept"
	);

	// Only the series is known: both separators lose what they separated.
	let mut values = Values::default();
	values.set(Field::Series, "Berserk".to_string());
	assert_eq!(pattern.render(&values).as_deref(), Some("Berserk"));

	// Nothing is known: there is no name to render.
	let bare = Pattern::parse("{series} - {title}").unwrap();
	assert_eq!(bare.render(&Values::default()), None);
}

#[test]
fn a_rename_collision_is_blocked_unless_overwrite_is_set() {
	let dir = TempDir::new().unwrap();
	let source = dir.path().join("raw-0001.cbz");
	write_cbz(&source, None, &[("0001.png", PNG)]);
	let occupied = dir.path().join("Berserk c001.cbz");
	write_cbz(&occupied, None, &[("0001.png", b"the-occupant")]);

	let options = serde_json::json!({
		"set": { "series": "Berserk", "number": "1" },
		"rename_from_metadata": true,
	});

	let plan = plan_for(vec![source.clone()], options.clone());
	assert_eq!(codes_of(&plan.warnings), vec![codes::RENAME_TARGET_EXISTS]);
	assert_eq!(
		detail_of(&plan, 0).rename,
		None,
		"the metadata edit stands; only the rename is dropped"
	);
	let report = MetaEdit.apply(&plan, &mut NoopProgress).unwrap();
	assert_eq!(report.skipped, vec![]);
	assert!(source.is_file(), "the source keeps its name");
	assert_eq!(
		entry_bytes(&occupied, "0001.png"),
		b"the-occupant",
		"the occupant is untouched"
	);

	let mut overwriting = options;
	overwriting["overwrite"] = serde_json::json!(true);
	let plan = plan_for(vec![source.clone()], overwriting);
	assert_eq!(codes_of(&plan.warnings), Vec::<&str>::new());
	assert_eq!(plan.actions[0].kind, ACTION_RENAME);
	assert_eq!(plan.actions[0].target.as_deref(), Some(occupied.as_path()));

	let report = MetaEdit.apply(&plan, &mut NoopProgress).unwrap();
	assert_eq!(report.applied.len(), 1);
	assert_eq!(report.skipped, vec![]);
	assert!(!source.exists());
	assert_eq!(
		entry_bytes(&occupied, "0001.png"),
		PNG,
		"overwrite replaced the occupant with the renamed book"
	);
}

#[test]
fn two_books_rendering_to_one_name_never_overwrite_each_other() {
	let dir = TempDir::new().unwrap();
	let first = dir.path().join("scan-a.cbz");
	let second = dir.path().join("scan-b.cbz");
	write_cbz(&first, None, &[("0001.png", PNG)]);
	write_cbz(&second, None, &[("0001.png", b"the-second-book")]);

	let plan = plan_for(
		vec![dir.path().to_path_buf()],
		serde_json::json!({
			"set": { "series": "Berserk", "number": "1" },
			"rename_from_metadata": true,
		}),
	);

	assert_eq!(
		codes_of(&plan.warnings),
		vec![codes::RENAME_TARGET_EXISTS],
		"the second book cannot claim a name this plan already gave away"
	);
	assert_eq!(detail_of(&plan, 1).rename, None);

	let report = MetaEdit.apply(&plan, &mut NoopProgress).unwrap();
	assert_eq!(report.applied.len(), 2);
	assert_eq!(report.skipped, vec![]);
	assert_eq!(
		entry_bytes(&dir.path().join("Berserk c001.cbz"), "0001.png"),
		PNG
	);
	assert_eq!(
		entry_bytes(&second, "0001.png"),
		b"the-second-book",
		"the blocked book keeps its own name and its own pages"
	);
}
