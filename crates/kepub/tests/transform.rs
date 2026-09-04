use std::io::{Cursor, Read};

use stump_kepub::{cache_file_name, transform_content, transform_epub, TransformOptions};
use zip::ZipArchive;

const FIXTURE: &[u8] = include_bytes!("fixtures/book.epub");

fn entry_bytes(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Vec<u8> {
	let mut entry = archive.by_name(name).expect("fixture entry exists");
	let mut bytes = Vec::new();
	entry.read_to_end(&mut bytes).expect("read fixture entry");
	bytes
}

fn compressed_entry_bytes(archive_bytes: &[u8], name: &str) -> Vec<u8> {
	let (start, size) = {
		let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
			.expect("open archive for raw entry");
		let entry = archive.by_name(name).expect("raw entry exists");
		(
			entry.data_start() as usize,
			entry.compressed_size() as usize,
		)
	};
	archive_bytes[start..start + size].to_vec()
}

#[test]
fn fixture_transform_adds_deterministic_kobo_markup() {
	let first =
		transform_epub(FIXTURE, &TransformOptions::default()).expect("transform fixture");
	let second = transform_epub(FIXTURE, &TransformOptions::default())
		.expect("transform fixture twice");
	assert_eq!(first, second, "conversion must be deterministic");

	let mut original = ZipArchive::new(Cursor::new(FIXTURE)).expect("open fixture");
	let mut converted =
		ZipArchive::new(Cursor::new(first.as_slice())).expect("open output");

	let chapter =
		entry_bytes(&mut converted, "OEBPS/6600632123799531513_11-h-1.htm.xhtml");
	let chapter = String::from_utf8(chapter).expect("fixture XHTML is UTF-8");
	assert!(
		chapter.contains("<div id=\"book-columns\"><div id=\"book-inner\">")
			&& chapter.contains("</div></div>")
	);
	assert!(chapter.contains("kobostylehacks"));
	assert!(chapter.contains("class=\"koboSpan\" id=\"kobo.1.1\""));
	assert!(chapter.contains("class=\"koboSpan\" id=\"kobo.1.2\""));

	// Non-content entries are raw-copied; the OPF is rewritten (re-indented,
	// cover/calibre metadata) exactly like kepubify, see the corpus test.
	for name in [
		"mimetype",
		"OEBPS/pgepub.css",
		"OEBPS/394382034267688847_cover.jpg",
	] {
		assert_eq!(
			compressed_entry_bytes(FIXTURE, name),
			compressed_entry_bytes(first.as_slice(), name),
			"raw ZIP payload for {name} changed",
		);
		assert_eq!(
			entry_bytes(&mut original, name),
			entry_bytes(&mut converted, name),
			"non-content entry {name} changed",
		);
	}
}

#[test]
fn content_transform_is_stable_and_does_not_wrap_code_or_pre() {
	let input = br#"<html><head></head><body><p>First sentence. Second sentence.</p><pre>First. Second.</pre><code>Third. Fourth.</code></body></html>"#;
	let first = transform_content(input, &TransformOptions::default())
		.expect("transform content");
	let second = transform_content(input, &TransformOptions::default())
		.expect("transform content twice");
	assert_eq!(first, second);
	let text = String::from_utf8(first).expect("UTF-8 output");
	assert!(text.contains("kobo.1.1") && text.contains("kobo.1.2"));
	assert!(text.contains("<pre>First. Second.</pre>"));
	assert!(text.contains("<code><span class=\"koboSpan\""));
}

#[test]
fn cache_file_name_changes_when_source_mtime_changes() {
	let defaults = TransformOptions::default();
	assert_ne!(
		cache_file_name("media-id", 1, &defaults),
		cache_file_name("media-id", 2, &defaults)
	);
	// Options are part of the key: changing them must invalidate the cache.
	let smart = TransformOptions {
		smarten_punctuation: true,
		..TransformOptions::default()
	};
	assert_ne!(
		cache_file_name("media-id", 2, &defaults),
		cache_file_name("media-id", 2, &smart)
	);
	assert!(cache_file_name("media-id", 2, &defaults).starts_with("media-id-2-"));
	assert!(cache_file_name("media-id", 2, &defaults).ends_with(".kepub.epub"));
}

/// kepubify's own ZIP writer is not byte-stable across runs (entry order and
/// timestamps vary), so parity is defined per entry: same entry set, and
/// byte-identical bytes for every entry.
#[test]
fn kepubify_corpus_matches_reference() {
	use std::{collections::BTreeMap, io::Read};

	fn entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
		let mut archive =
			zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("valid zip archive");
		let mut map = BTreeMap::new();
		for index in 0..archive.len() {
			let mut entry = archive.by_index(index).expect("readable entry");
			let mut data = Vec::with_capacity(entry.size() as usize);
			entry.read_to_end(&mut data).expect("entry bytes");
			map.insert(entry.name().to_owned(), data);
		}
		map
	}

	let input = include_bytes!("fixtures/kepubify/book.epub");
	let expected = entries(include_bytes!("fixtures/kepubify/book.kepub.epub"));
	let actual = entries(
		&transform_epub(input, &TransformOptions::default())
			.expect("transform corpus fixture"),
	);

	let expected_names: Vec<_> = expected.keys().collect();
	let actual_names: Vec<_> = actual.keys().collect();
	assert_eq!(
		actual_names, expected_names,
		"entry set differs from kepubify"
	);
	for (name, expected_bytes) in &expected {
		assert!(
			actual[name] == *expected_bytes,
			"entry {name} differs from kepubify @9546034bc023891af5ce30709de6ae2dcf264628"
		);
	}
}
