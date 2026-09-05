use chrono::Utc;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use models::{
	entity::{library, media, media_metadata, page_hash, series},
	shared::enums::FileStatus,
};
use sea_orm::{ActiveModelTrait, DatabaseBackend, MockDatabase, Set};
use serde_json::{json, Value};
use std::{
	collections::BTreeMap,
	io::{Cursor, Write},
	path::{Path, PathBuf},
	sync::Arc,
};
use stump_api_types::settings::SettingValues;
use tempfile::NamedTempFile;
use zip::{write::SimpleFileOptions, ZipWriter};

use crate::ingest::contract::{
	score_report, BookSnapshot, IngestMediaKind, IngestPageEntry, QualityCheck,
	QualityStatus,
};

use super::{
	cover_not_page_two::CoverNotPageTwoCheck,
	cover_present::CoverPresentCheck,
	duplicate_existing::DuplicateExistingCheck,
	duplicate_pages_across_books::DuplicatePagesAcrossBooksCheck,
	epub_toc_chapters::EpubTocChaptersCheck,
	filename::{parse_filename, FilenameParseStatus},
	image_dimensions_consistent::ImageDimensionsConsistentCheck,
	page_count_matches_archive_entries::PageCountMatchesArchiveEntriesCheck,
	QualityRegistry,
};

fn snapshot(path: &Path, kind: IngestMediaKind, page_count: usize) -> BookSnapshot {
	BookSnapshot {
		drop_item_id: "drop-test".to_string(),
		library_id: "library-test".to_string(),
		staged_path: path.to_path_buf(),
		source_sha256: "source-digest".to_string(),
		byte_size: std::fs::metadata(path)
			.map(|metadata| metadata.len())
			.unwrap_or(0),
		source_filename: "Series - Title #1.cbz".to_string(),
		relative_path: String::new(),
		media_kind: kind,
		embedded_metadata: None,
		pages: (0..page_count)
			.map(|index| IngestPageEntry {
				index: index as u32,
				path: format!("page-{index}.png"),
				size: None,
				is_image: true,
			})
			.collect(),
		analysis: None,
	}
}

fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
	let image = ImageBuffer::from_pixel(width, height, Rgba(color));
	let mut output = Cursor::new(Vec::new());
	DynamicImage::ImageRgba8(image)
		.write_to(&mut output, ImageFormat::Png)
		.expect("encode PNG fixture");
	output.into_inner()
}

fn cbz(entries: &[(&str, Vec<u8>)]) -> NamedTempFile {
	let mut output = Cursor::new(Vec::new());
	{
		let mut writer = ZipWriter::new(&mut output);
		for (name, bytes) in entries {
			writer
				.start_file(*name, SimpleFileOptions::default())
				.expect("create ZIP fixture entry");
			writer.write_all(bytes).expect("write ZIP fixture entry");
		}
		writer.finish().expect("finish ZIP fixture");
	}
	let mut file = NamedTempFile::new().expect("create CBZ fixture");
	file.write_all(output.get_ref()).expect("write CBZ fixture");
	file
}

async fn run(
	check: &dyn QualityCheck,
	book: &BookSnapshot,
) -> (QualityStatus, f64, Value) {
	let settings = SettingValues::new();
	let result = check
		.run(book, &settings)
		.await
		.expect("quality check succeeds");
	(result.status, result.normalized_score, result.evidence)
}

async fn run_disabled(check: &dyn QualityCheck, book: &BookSnapshot) -> Value {
	let mut settings = SettingValues::new();
	settings.insert("enabled".to_string(), Value::Bool(false));
	let result = check
		.run(book, &settings)
		.await
		.expect("quality check succeeds");
	assert_eq!(result.status, QualityStatus::NotApplicable);
	result.evidence
}

#[tokio::test]
async fn cover_present_covers_all_statuses() {
	let image = png(4, 4, [255, 0, 0, 255]);
	let valid = cbz(&[("001.png", image)]);
	let book = snapshot(valid.path(), IngestMediaKind::ComicArchive, 1);
	let check = CoverPresentCheck::new();
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Pass);
	assert_eq!(score, 1.0);
	assert_eq!(evidence["decoded"], true);

	let invalid = cbz(&[("001.png", vec![1, 2, 3])]);
	let book = snapshot(invalid.path(), IngestMediaKind::ComicArchive, 1);
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Fail);
	assert_eq!(score, 0.0);
	assert_eq!(evidence["decoded"], false);

	let book = snapshot(Path::new("not-used"), IngestMediaKind::Unknown, 0);
	let (status, _, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::NotApplicable);
	assert_eq!(run_disabled(&check, &book).await["disabled"], true);
}

#[tokio::test]
async fn cover_not_page_two_covers_all_statuses() {
	let first = png(4, 4, [255, 0, 0, 255]);
	let second = png(4, 4, [0, 255, 0, 255]);
	let different = cbz(&[("001.png", first.clone()), ("002.png", second)]);
	let check = CoverNotPageTwoCheck::new();
	let book = snapshot(different.path(), IngestMediaKind::ComicArchive, 2);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Pass);
	assert_eq!(score, 1.0);

	let same = cbz(&[("001.png", first.clone()), ("002.png", first)]);
	let book = snapshot(same.path(), IngestMediaKind::ComicArchive, 2);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Fail);
	assert_eq!(score, 0.0);

	let undecodable = cbz(&[
		("001.png", png(4, 4, [1, 2, 3, 255])),
		("002.png", vec![1, 2, 3]),
	]);
	let book = snapshot(undecodable.path(), IngestMediaKind::ComicArchive, 2);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Warn);
	assert_eq!(score, 0.5);

	let one = cbz(&[("001.png", png(4, 4, [1, 2, 3, 255]))]);
	let book = snapshot(one.path(), IngestMediaKind::ComicArchive, 1);
	let (status, _, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::NotApplicable);
}

#[tokio::test]
async fn archive_page_count_covers_all_statuses() {
	let entries = (0..4)
		.map(|index| {
			(
				format!("{index:03}.png"),
				png(4, 4, [index as u8, 0, 0, 255]),
			)
		})
		.collect::<Vec<_>>();
	let entry_refs = entries
		.iter()
		.map(|(name, bytes)| (name.as_str(), bytes.clone()))
		.collect::<Vec<_>>();
	let archive = cbz(&entry_refs);
	let check = PageCountMatchesArchiveEntriesCheck::new();
	let book = snapshot(archive.path(), IngestMediaKind::ComicArchive, 4);
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Pass);
	assert_eq!(score, 1.0);
	assert_eq!(evidence["entry_count"], 4);

	let book = snapshot(archive.path(), IngestMediaKind::ComicArchive, 3);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Warn);
	assert_eq!(score, 0.5);

	let book = snapshot(archive.path(), IngestMediaKind::ComicArchive, 1);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Fail);
	assert_eq!(score, 0.0);

	let book = snapshot(archive.path(), IngestMediaKind::Epub, 4);
	let (status, _, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::NotApplicable);
}

#[tokio::test]
async fn image_dimensions_covers_all_statuses() {
	let same_entries = (0..4)
		.map(|index| {
			(
				format!("{index:03}.png"),
				png(100, 200, [index as u8, 0, 0, 255]),
			)
		})
		.collect::<Vec<_>>();
	let same_refs = same_entries
		.iter()
		.map(|(name, bytes)| (name.as_str(), bytes.clone()))
		.collect::<Vec<_>>();
	let same = cbz(&same_refs);
	let check = ImageDimensionsConsistentCheck::new();
	let book = snapshot(same.path(), IngestMediaKind::ComicArchive, 4);
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Pass);
	assert_eq!(score, 1.0);
	assert_eq!(evidence["matching_pages"], 4);

	let mut mostly_same = Vec::new();
	for index in 0..20 {
		mostly_same.push((
			format!("{index:03}.png"),
			if index == 19 {
				png(120, 200, [0, 0, 0, 255])
			} else {
				png(100, 200, [index as u8, 0, 0, 255])
			},
		));
	}
	let mostly_refs = mostly_same
		.iter()
		.map(|(name, bytes)| (name.as_str(), bytes.clone()))
		.collect::<Vec<_>>();
	let mostly = cbz(&mostly_refs);
	let book = snapshot(mostly.path(), IngestMediaKind::ComicArchive, 20);
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Warn);
	assert!((score - 0.95).abs() < 0.0001);
	assert_eq!(evidence["matching_pages"], 19);

	let mut mostly_bad = Vec::new();
	for index in 0..5 {
		mostly_bad.push((
			format!("{index:03}.png"),
			if index == 0 {
				png(120, 200, [0, 0, 0, 255])
			} else {
				png(100, 200, [index as u8, 0, 0, 255])
			},
		));
	}
	let mostly_bad_refs = mostly_bad
		.iter()
		.map(|(name, bytes)| (name.as_str(), bytes.clone()))
		.collect::<Vec<_>>();
	let bad = cbz(&mostly_bad_refs);
	let book = snapshot(bad.path(), IngestMediaKind::ComicArchive, 5);
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Fail);
	assert!((score - 0.8).abs() < 0.0001);
	assert_eq!(evidence["matching_pages"], 4);

	let one = cbz(&[("001.png", png(4, 4, [0, 0, 0, 255]))]);
	let book = snapshot(one.path(), IngestMediaKind::ComicArchive, 1);
	let (status, _, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::NotApplicable);
}

fn synthetic_epub(with_toc: bool) -> NamedTempFile {
	let mut output = Cursor::new(Vec::new());
	{
		let mut writer = ZipWriter::new(&mut output);
		let stored = SimpleFileOptions::default()
			.compression_method(zip::CompressionMethod::Stored);
		writer
			.start_file("mimetype", stored)
			.expect("EPUB mimetype");
		writer
			.write_all(b"application/epub+zip")
			.expect("EPUB mimetype bytes");
		writer
			.start_file("META-INF/container.xml", SimpleFileOptions::default())
			.expect("EPUB container");
		writer
			.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#)
			.expect("EPUB container bytes");
		writer
			.start_file("OEBPS/content.opf", SimpleFileOptions::default())
			.expect("EPUB package");
		let toc_manifest = if with_toc {
			"<item id=\"toc\" href=\"toc.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>"
		} else {
			""
		};
		let package = format!(
			"<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" version=\"3.0\"><metadata><dc:title>Fixture</dc:title></metadata><manifest><item id=\"chapter\" href=\"chapter.xhtml\" media-type=\"application/xhtml+xml\"/>{toc_manifest}</manifest><spine><itemref idref=\"chapter\"/></spine></package>"
		);
		writer
			.write_all(package.as_bytes())
			.expect("EPUB package bytes");
		writer
			.start_file("OEBPS/chapter.xhtml", SimpleFileOptions::default())
			.expect("EPUB chapter");
		writer
			.write_all(b"<html><body><h1>Chapter</h1></body></html>")
			.expect("EPUB chapter bytes");
		if with_toc {
			writer
				.start_file("OEBPS/toc.xhtml", SimpleFileOptions::default())
				.expect("EPUB nav");
			writer
				.write_all(b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><nav epub:type=\"toc\"><ol><li><a href=\"chapter.xhtml\">Chapter</a></li></ol></nav></body></html>")
				.expect("EPUB nav bytes");
		}
		writer.finish().expect("finish EPUB fixture");
	}
	let mut file = NamedTempFile::new().expect("create EPUB fixture");
	file.write_all(output.get_ref())
		.expect("write EPUB fixture");
	file
}

#[tokio::test]
async fn epub_toc_covers_all_statuses() {
	let check = EpubTocChaptersCheck::new();
	let pass = synthetic_epub(true);
	let book = snapshot(pass.path(), IngestMediaKind::Epub, 1);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Pass);
	assert_eq!(score, 1.0);

	let warn = synthetic_epub(false);
	let book = snapshot(warn.path(), IngestMediaKind::Epub, 1);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Warn);
	assert_eq!(score, 0.5);

	let invalid = NamedTempFile::new().expect("create invalid EPUB");
	let book = snapshot(invalid.path(), IngestMediaKind::Epub, 1);
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Fail);
	assert_eq!(score, 0.0);

	let book = snapshot(Path::new("not-used"), IngestMediaKind::ComicArchive, 1);
	let (status, _, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::NotApplicable);
}

async fn insert_media(
	conn: &sea_orm::DatabaseConnection,
	id: &str,
	hash: Option<&str>,
	title: Option<&str>,
) {
	media::ActiveModel {
		id: Set(id.to_string()),
		name: Set(title.unwrap_or("Existing").to_string()),
		size: Set(1),
		extension: Set("cbz".to_string()),
		pages: Set(1),
		path: Set(format!("/tmp/{id}.cbz")),
		status: Set(FileStatus::Ready),
		hash: Set(hash.map(ToOwned::to_owned)),
		created_at: Set(Utc::now().into()),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("insert media fixture");
	if let Some(title) = title {
		media_metadata::ActiveModel {
			media_id: Set(Some(id.to_string())),
			title: Set(Some(title.to_string())),
			..Default::default()
		}
		.insert(conn)
		.await
		.expect("insert media metadata fixture");
	}
}

#[tokio::test]
async fn duplicate_existing_covers_pass_warn_and_fail() {
	let conn = ::tests::db::test_database().await;
	let check = DuplicateExistingCheck::new(Arc::new(conn));
	let file = cbz(&[("001.png", png(4, 4, [0, 0, 0, 255]))]);
	let mut book = snapshot(file.path(), IngestMediaKind::ComicArchive, 1);
	book.source_filename = "Unique Title.cbz".to_string();
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Pass);
	assert_eq!(score, 1.0);

	let conn = ::tests::db::test_database().await;
	insert_media(&conn, "existing-title", None, Some("Wayfarers")).await;
	let check = DuplicateExistingCheck::new(Arc::new(conn));
	book.source_filename = "Wayfarers.cbz".to_string();
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Warn);
	assert_eq!(score, 0.5);
	assert_eq!(evidence["matching_media_ids"][0], "existing-title");

	let conn = ::tests::db::test_database().await;
	insert_media(&conn, "existing-hash", Some("source-digest"), None).await;
	let check = DuplicateExistingCheck::new(Arc::new(conn));
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Fail);
	assert_eq!(score, 0.0);
	assert_eq!(evidence["matching_media_ids"][0], "existing-hash");
}


async fn seed_library_book(
	conn: &sea_orm::DatabaseConnection,
	library_id: &str,
	series_id: &str,
	media_id: &str,
) {
	library::ActiveModel {
		id: Set(library_id.to_string()),
		name: Set(library_id.to_string()),
		path: Set(format!("/{library_id}")),
		status: Set(FileStatus::Ready),
		config_id: Set(1),
		created_at: Set(Utc::now().into()),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("insert library fixture");
	series::ActiveModel {
		id: Set(series_id.to_string()),
		name: Set(series_id.to_string()),
		path: Set(format!("/{library_id}/{series_id}")),
		status: Set(FileStatus::Ready),
		library_id: Set(Some(library_id.to_string())),
		created_at: Set(Utc::now().into()),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("insert series fixture");
	media::ActiveModel {
		id: Set(media_id.to_string()),
		name: Set(media_id.to_string()),
		path: Set(format!("/{library_id}/{series_id}/{media_id}.cbz")),
		extension: Set("cbz".to_string()),
		series_id: Set(Some(series_id.to_string())),
		pages: Set(1),
		size: Set(1),
		status: Set(FileStatus::Ready),
		created_at: Set(Utc::now().into()),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("insert media fixture");
}

async fn seed_page_hash(
	conn: &sea_orm::DatabaseConnection,
	media_id: &str,
	page: i32,
	dhash: i64,
) {
	page_hash::ActiveModel {
		media_id: Set(media_id.to_string()),
		page: Set(page),
		dhash: Set(dhash),
		created_at: Set(Utc::now().into()),
	}
	.insert(conn)
	.await
	.expect("insert page hash fixture");
}

#[tokio::test]
async fn duplicate_pages_across_books_covers_statuses_and_threshold() {
	// Not applicable for reflowable books.
	let conn = ::tests::db::test_database().await;
	let check = DuplicatePagesAcrossBooksCheck::new(Arc::new(conn));
	let epub = synthetic_epub(false);
	let book = snapshot(epub.path(), IngestMediaKind::Epub, 1);
	let (status, _, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::NotApplicable);

	let page = png(8, 8, [0, 0, 0, 255]);
	let own_dhash = stump_media::page_dhash(&page).expect("hash fixture page") as i64;

	// Pass when no analyzed page hashes exist in the library yet.
	let conn = ::tests::db::test_database().await;
	let file = cbz(&[("001.png", page.clone())]);
	let mut book = snapshot(file.path(), IngestMediaKind::ComicArchive, 1);
	book.library_id = "lib".to_string();
	let check = DuplicatePagesAcrossBooksCheck::new(Arc::new(conn));
	let (status, score, _) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Pass);
	assert_eq!(score, 1.0);

	// Warn once the hash reaches the default threshold of 3 distinct books;
	// the third book only matches within the Hamming tolerance and the book
	// under rework never counts as its own duplicate.
	let conn = ::tests::db::test_database().await;
	seed_library_book(&conn, "lib", "series-a", "book-a").await;
	seed_library_book(&conn, "lib", "series-b", "book-b").await;
	seed_library_book(&conn, "lib", "series-c", "book-c").await;
	seed_library_book(&conn, "lib", "series-self", "drop-test").await;
	seed_page_hash(&conn, "book-a", 1, own_dhash).await;
	seed_page_hash(&conn, "book-b", 1, own_dhash).await;
	seed_page_hash(&conn, "book-c", 1, own_dhash ^ 0b11).await;
	seed_page_hash(&conn, "drop-test", 1, own_dhash).await;
	let check = DuplicatePagesAcrossBooksCheck::new(Arc::new(conn));
	let mut book = snapshot(file.path(), IngestMediaKind::ComicArchive, 1);
	book.library_id = "lib".to_string();
	book.drop_item_id = "drop-test".to_string();
	let (status, score, evidence) = run(&check, &book).await;
	assert_eq!(status, QualityStatus::Warn);
	assert_eq!(score, 0.5);
	let books = evidence["duplicate_groups"][0]["books"].as_array().unwrap();
	assert_eq!(books.len(), 3);
	assert_eq!(evidence["duplicate_groups"][0]["dhash"], json!(format!("{own_dhash:016x}")));
	assert_eq!(evidence["matched_books"].as_array().unwrap().len(), 3);

	// Raising the threshold above the matched book count silences the check.
	let conn = ::tests::db::test_database().await;
	seed_library_book(&conn, "lib", "series-a", "book-a").await;
	seed_library_book(&conn, "lib", "series-b", "book-b").await;
	seed_page_hash(&conn, "book-a", 1, own_dhash).await;
	seed_page_hash(&conn, "book-b", 1, own_dhash).await;
	let check = DuplicatePagesAcrossBooksCheck::new(Arc::new(conn));
	let mut book = snapshot(file.path(), IngestMediaKind::ComicArchive, 1);
	book.library_id = "lib".to_string();
	let mut settings = SettingValues::new();
	settings.insert("minBooks".to_string(), json!(4));
	let result = check.run(&book, &settings).await.expect("check succeeds");
	assert_eq!(result.status, QualityStatus::Pass);
}

#[tokio::test]
async fn filename_parser_covers_fixed_grammar() {
	let cases = [
		(
			"Book.cbz",
			FilenameParseStatus::Pass,
			None,
			"Book",
			None,
			None,
		),
		(
			"Series/Book_v3.epub",
			FilenameParseStatus::Pass,
			Some("Series"),
			"Book",
			Some(3.0),
			None,
		),
		(
			"Series - Book vol 2 2020.cbz",
			FilenameParseStatus::Pass,
			Some("Series"),
			"Book",
			Some(2.0),
			Some(2020),
		),
		(
			"Series - Book #1 v2.cbz",
			FilenameParseStatus::Warn,
			Some("Series"),
			"Book",
			Some(1.0),
			None,
		),
		(
			"Other/Series - Book.cbz",
			FilenameParseStatus::Warn,
			Some("Series"),
			"Book",
			None,
			None,
		),
		(
			"Series - #1.cbz",
			FilenameParseStatus::Fail,
			Some("Series"),
			"",
			Some(1.0),
			None,
		),
		(
			"Series - Book #.cbz",
			FilenameParseStatus::Fail,
			Some("Series"),
			"Book",
			None,
			None,
		),
		(
			"Series - Book 1899.cbz",
			FilenameParseStatus::Fail,
			Some("Series"),
			"Book",
			None,
			None,
		),
	];
	for (input, status, series, title, number, year) in cases {
		let parsed = parse_filename(input);
		assert_eq!(parsed.status, status, "{input}");
		assert_eq!(parsed.series.as_deref(), series, "{input}");
		assert_eq!(parsed.title, title, "{input}");
		assert_eq!(parsed.number, number, "{input}");
		assert_eq!(parsed.year, year, "{input}");
	}
	let normalized = parse_filename("Ｓｅｒｉｅｓ - Ｔｉｔｌｅ ＃１.cbz");
	assert_eq!(normalized.status, FilenameParseStatus::Pass);
	assert_eq!(normalized.number, Some(1.0));
}

#[tokio::test]
async fn registry_and_score_preserve_contract_identities() {
	let conn = MockDatabase::new(DatabaseBackend::Sqlite).into_connection();
	let registry = QualityRegistry::builtin(Arc::new(conn));
	assert_eq!(registry.checks().len(), 8);
	assert_eq!(registry.total_weight(), 100);
	assert_eq!(
		registry
			.catalog()
			.iter()
			.map(|descriptor| descriptor.id.as_str())
			.collect::<Vec<_>>(),
		vec![
			"cover_present",
			"cover_not_page_two",
			"page_count_matches_archive_entries",
			"image_dimensions_consistent",
			"epub_toc_chapters",
			"duplicate_existing",
			"duplicate_pages_across_books",
			"filename_series_parse",
		]
	);

	let mut settings = BTreeMap::new();
	for check in registry.checks() {
		settings.insert(
			check.id().to_string(),
			BTreeMap::from([(String::from("enabled"), json!(false))]),
		);
	}
	let book = BookSnapshot {
		drop_item_id: "drop".to_string(),
		library_id: "library".to_string(),
		staged_path: PathBuf::from("none"),
		source_sha256: "digest".to_string(),
		byte_size: 0,
		source_filename: "Book.cbz".to_string(),
		relative_path: String::new(),
		media_kind: IngestMediaKind::Unknown,
		embedded_metadata: None,
		pages: Vec::new(),
		analysis: None,
	};
	let report = registry
		.run_all(&book, &settings)
		.await
		.expect("disabled report");
	assert_eq!(report.score, 0);
	assert_eq!(report.checks.len(), 7);
	assert!(report
		.checks
		.iter()
		.all(|check| check.outcome.status == QualityStatus::NotApplicable));
	assert_eq!(report.settings_snapshot.len(), 7);

	let outcomes = vec![
		(
			super::outcome("pass", "pass", QualityStatus::Pass, 1.0, json!({})),
			20,
		),
		(
			super::outcome("warn", "warn", QualityStatus::Warn, 0.5, json!({})),
			10,
		),
		(
			super::outcome("fail", "fail", QualityStatus::Fail, 0.0, json!({})),
			20,
		),
		(
			super::outcome("na", "na", QualityStatus::NotApplicable, 0.0, json!({})),
			50,
		),
	];
	let (score, checks) = score_report(&outcomes);
	let sum: f64 = checks.iter().map(|check| check.contribution).sum();
	assert_eq!(score, 50);
	assert!((sum - 50.0).abs() < 1e-9);
	assert!((checks[0].contribution - 40.0).abs() < 1e-9);
	assert!((checks[1].contribution - 10.0).abs() < 1e-9);
	assert_eq!(checks[3].contribution, 0.0);
}
