//! Ports `kepub/transform_test.go`'s OPF, file-filter, and dummy-titlepage tests
//! from kepubify commit 9546034bc023891af5ce30709de6ae2dcf264628.
//!
//! TransformOptions fields referenced by these cases:
//! - `dummy_titlepage`

use std::io::{Cursor, Read, Write};

use stump_kepub::parts::{opf_calibre_meta, opf_cover_image};
use stump_kepub::{
	should_skip_file, transform_dummy_titlepage, transform_opf, TransformOptions,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

fn trim_utf8(bytes: Vec<u8>) -> String {
	String::from_utf8(bytes).expect("transformation output is UTF-8")
}

#[test]
fn test_transform_opf() {
	// note: the individual parts below aren't tested again here
	let input = r#"
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- other stuff left out for brevity -->
    </metadata><manifest>
    <item id="cover-image" href="book_cover.jpg" media-type="image/jpeg"/>
       <item id="xhtml_text1" href="xhtml/text1.xhtml" media-type="application/xhtml+xml"/>
       <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    </manifest>
    <spine toc="ncx">
        <itemref idref="xhtml_text1"/>
    </spine>
</package>"#;
	let expected = r#"
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- other stuff left out for brevity -->
    </metadata>
    <manifest>
        <item id="cover-image" href="book_cover.jpg" media-type="image/jpeg"/>
        <item id="xhtml_text1" href="xhtml/text1.xhtml" media-type="application/xhtml+xml"/>
        <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
        <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    </manifest>
    <spine toc="ncx">
        <itemref idref="xhtml_text1"/>
    </spine>
</package>"#;

	let actual =
		trim_utf8(transform_opf(input.as_bytes(), &TransformOptions::default()).unwrap());
	assert_eq!(actual.trim(), expected.trim());
}

#[test]
fn test_transform_opf_parts() {
	let cases = [
		(
			"set cover-image property on ID from meta[name=cover]",
			r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- other stuff left out for brevity -->
        <meta name="cover" content="cover-image"/>
    </metadata>
    <manifest>
        <item id="cover-image" href="book_cover.jpg" media-type="image/jpeg"/>
        <item id="xhtml_text1" href="xhtml/text1.xhtml" media-type="application/xhtml+xml"/>
        <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
        <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    </manifest>
    <spine toc="ncx">
        <itemref idref="xhtml_text1"/>
    </spine>
</package>"#,
			r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- other stuff left out for brevity -->
        <meta name="cover" content="cover-image"/>
    </metadata>
    <manifest>
        <item id="cover-image" href="book_cover.jpg" media-type="image/jpeg" properties="cover-image"/>
        <item id="xhtml_text1" href="xhtml/text1.xhtml" media-type="application/xhtml+xml"/>
        <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
        <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    </manifest>
    <spine toc="ncx">
        <itemref idref="xhtml_text1"/>
    </spine>
</package>"#,
			opf_cover_image as fn(&[u8]) -> Vec<u8>,
		),
		(
			"set cover-image property on #cover if meta[name=cover] not present",
			r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- other stuff left out for brevity -->
    </metadata>
    <manifest>
        <item id="cover" href="book_cover.jpg" media-type="image/jpeg"/>
        <item id="xhtml_text1" href="xhtml/text1.xhtml" media-type="application/xhtml+xml"/>
        <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
        <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    </manifest>
    <spine toc="ncx">
        <itemref idref="xhtml_text1"/>
    </spine>
</package>"#,
			r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- other stuff left out for brevity -->
    </metadata>
    <manifest>
        <item id="cover" href="book_cover.jpg" media-type="image/jpeg" properties="cover-image"/>
        <item id="xhtml_text1" href="xhtml/text1.xhtml" media-type="application/xhtml+xml"/>
        <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
        <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    </manifest>
    <spine toc="ncx">
        <itemref idref="xhtml_text1"/>
    </spine>
</package>"#,
			opf_cover_image as fn(&[u8]) -> Vec<u8>,
		),
		(
			"remove calibre:timestamp and calibre contributor",
			r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- --><meta content="whatever" name="calibre:timestamp"/>
        <!-- --><dc:contributor file-as="calibre" role="bkp">calibre (#.#.#) [https://calibre-ebook.com]</dc:contributor>
        <!-- --><dc:contributor>whatever</dc:contributor>
    </metadata>
    <!-- other stuff left out for brevity -->
</package>"#,
			r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uuid_id">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <!-- -->
        <!-- -->
        <!-- --><dc:contributor>whatever</dc:contributor>
    </metadata>
    <!-- other stuff left out for brevity -->
</package>"#,
			opf_calibre_meta as fn(&[u8]) -> Vec<u8>,
		),
	];

	for (what, input, expected, func) in cases {
		let actual = trim_utf8(func(input.as_bytes()));
		assert_eq!(actual.trim(), expected.trim(), "case {what:?}");
	}
}

#[test]
fn test_transform_file_filter() {
	for fn_name in [
		"META-INF/calibre_bookmarks.txt",
		"iTunesMetadata.plist",
		"iTunesArtwork.plist",
		".DS_STORE",
		"__MACOSX/test.txt",
		"thumbs.db",
	] {
		assert!(
			should_skip_file(fn_name),
			"expected {fn_name:?} to be filtered"
		);
	}
}

const LOREM: &str = "Lorem ipsum dolor, sit amet consectetur adipisicing elit. Dolorem, placeat. Porro animi architecto pariatur laudantium voluptate, at, odit delectus fugiat beatae autem odio. Iure iste maiores corrupti porro quibusdam. Sunt?";
const DUMMY_TITLEPAGE: &str = "<!DOCTYPE html><html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\"><head><title></title></head><body><p style=\"text-align: center; margin: 4em 0; font-size: .7em; font-style: italic;\">Page intentionally left blank by kepubify.</p></body></html>";

struct DummyTitlepageCase<'a> {
	what: &'a str,
	manifest: &'a str,
	spine: &'a str,
	content: Vec<(&'a str, String)>,
	should_error: bool,
	should_detect: bool,
	dummy_titlepage: Option<bool>,
}

fn build_epub(case: &DummyTitlepageCase<'_>) -> Vec<u8> {
	const OPF: &str = "OEBPS/content.opf";
	let mut output = Cursor::new(Vec::new());
	let mut writer = ZipWriter::new(&mut output);
	let stored =
		SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
	let deflated =
		SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

	writer.start_file("mimetype", stored).unwrap();
	writer.write_all(b"application/epub+zip").unwrap();
	writer
		.start_file("META-INF/container.xml", deflated)
		.unwrap();
	writer
		.write_all(
			br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
	<rootfiles>
		<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
	</rootfiles>
</container>
"#,
		)
		.unwrap();
	writer.start_file(OPF, deflated).unwrap();
	write!(
		writer,
		r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
    <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
        <dc:title>Test</dc:title>
    </metadata>
    <manifest>{}</manifest>
    <spine toc="ncx">{}</spine>
</package>
"#,
		case.manifest, case.spine
	)
	.unwrap();
	for (name, data) in &case.content {
		writer
			.start_file(format!("OEBPS/{name}"), deflated)
			.unwrap();
		writer.write_all(data.as_bytes()).unwrap();
	}
	writer.finish().unwrap();
	drop(writer);
	output.into_inner()
}

fn read_zip_entry(
	archive: &mut ZipArchive<Cursor<&[u8]>>,
	name: &str,
) -> Option<Vec<u8>> {
	let mut entry = archive.by_name(name).ok()?;
	let mut data = Vec::new();
	entry.read_to_end(&mut data).ok()?;
	Some(data)
}

/// This is the Rust equivalent of transformDummyTitlepageTestCase.CheckOPF:
/// locate the generated manifest item, require XHTML media type and a linear,
/// first-in-flow spine item, then require that its content can be read.
fn check_dummy_opf(output: &[u8], what: &str) {
	let mut archive = ZipArchive::new(Cursor::new(output))
		.unwrap_or_else(|error| panic!("case {what:?}: open transformed EPUB: {error}"));
	let opf = read_zip_entry(&mut archive, "OEBPS/content.opf")
		.unwrap_or_else(|| panic!("case {what:?}: OPF exists"));
	let opf = String::from_utf8(opf)
		.unwrap_or_else(|error| panic!("case {what:?}: OPF is UTF-8: {error}"));
	let manifest_item = r#"<item id="kepubify-titlepage-dummy""#;
	let manifest_offset = opf
		.find(manifest_item)
		.unwrap_or_else(|| panic!("case {what:?}: OPF missing dummy manifest item"));
	let manifest_item_end = opf[manifest_offset..]
		.find("/>")
		.unwrap_or_else(|| panic!("case {what:?}: dummy manifest item closes"));
	let manifest_item = &opf[manifest_offset..manifest_offset + manifest_item_end];
	assert!(
		manifest_item.contains(r#"href="kepubify-titlepage-dummy.xhtml""#),
		"case {what:?}: dummy manifest item has wrong href"
	);
	assert!(
		manifest_item.contains(r#"media-type="application/xhtml+xml""#),
		"case {what:?}: dummy manifest item has wrong media type"
	);

	let spine_start = opf
		.find("<spine")
		.unwrap_or_else(|| panic!("case {what:?}: OPF spine exists"));
	let spine_end = opf[spine_start..]
		.find("</spine>")
		.map(|offset| spine_start + offset)
		.unwrap_or_else(|| panic!("case {what:?}: OPF spine closes"));
	let spine = &opf[spine_start..spine_end];
	let first_itemref = spine
		.find("<itemref")
		.unwrap_or_else(|| panic!("case {what:?}: spine has an itemref"));
	let actual_first_itemref = spine[first_itemref..]
		.split_once("/>")
		.unwrap_or_else(|| panic!("case {what:?}: spine itemref closes"))
		.0;
	assert!(
		actual_first_itemref.contains(r#"idref="kepubify-titlepage-dummy""#)
			&& !actual_first_itemref.contains(r#"linear="no""#),
		"case {what:?}: dummy spine item is not first in content flow"
	);

	let content = read_zip_entry(&mut archive, "OEBPS/kepubify-titlepage-dummy.xhtml")
		.unwrap_or_else(|| panic!("case {what:?}: dummy content entry exists"));
	assert_eq!(
		content,
		DUMMY_TITLEPAGE.as_bytes(),
		"case {what:?}: dummy content differs"
	);
}

fn run_dummy_titlepage_case(case: DummyTitlepageCase<'_>) {
	let input = build_epub(&case);
	let result = transform_dummy_titlepage(&input, case.dummy_titlepage);
	if case.should_error {
		assert!(result.is_err(), "case {:?}: expected error", case.what);
		return;
	}
	let output = result.unwrap_or_else(|error| {
		panic!("case {:?}: unexpected error: {error}", case.what)
	});
	let mut archive =
		ZipArchive::new(Cursor::new(output.as_slice())).unwrap_or_else(|error| {
			panic!("case {:?}: open output EPUB: {error}", case.what)
		});
	let opf = read_zip_entry(&mut archive, "OEBPS/content.opf")
		.unwrap_or_else(|| panic!("case {:?}: output OPF exists", case.what));
	let dummy_item = "kepubify-titlepage-dummy";
	if case.should_detect {
		drop(archive);
		check_dummy_opf(&output, case.what);
	} else {
		let mut original_archive =
			ZipArchive::new(Cursor::new(input.as_slice())).expect("open original EPUB");
		let original_opf = read_zip_entry(&mut original_archive, "OEBPS/content.opf")
			.expect("original OPF exists");
		assert_eq!(
			opf, original_opf,
			"case {:?}: heuristic returned false but OPF was modified",
			case.what
		);
		assert!(
			!String::from_utf8_lossy(&opf).contains(dummy_item),
			"case {:?}: heuristic unexpectedly detected",
			case.what
		);
		assert!(
			read_zip_entry(&mut archive, "OEBPS/kepubify-titlepage-dummy.xhtml")
				.is_none(),
			"case {:?}: unexpected dummy content entry",
			case.what
		);
	}
}
#[test]
fn test_transform_dummy_titlepage() {
	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "separate titlepage (no dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
			<item id="cover" href="cover.png" media-type="image/png"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.html",
                r#"<!DOCTYPE html><html><head><title></title></head><body><img src="cover.png"></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            ("cover.png", String::new()),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "separate titlepage, but with a non-standard extension (no dummy)",
        manifest: r#"
			<item id="item1" href="item1.xml" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
			<item id="cover" href="cover.png" media-type="image/png"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.xml",
                r#"<!DOCTYPE html><html><head><title></title></head><body><img src="cover.png"></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            ("cover.png", String::new()),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "separate titlepage, but with a non-standard media type (no dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="text/xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
			<item id="cover" href="cover.png" media-type="image/png"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.html",
                r#"<!DOCTYPE html><html><head><title></title></head><body><img src="cover.png"></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            ("cover.png", String::new()),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "separate titlepage, but with non-linear content spine item before it (no dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
			<item id="item3" href="item3.html" media-type="application/xhtml+xml"/>
			<item id="cover" href="cover.png" media-type="image/png"/>
		"#,
        spine: r#"
			<itemref idref="item3" linear="no"/>
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.html",
                r#"<!DOCTYPE html><html><head><title></title></head><body><img src="cover.png"></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            (
                "item3.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            ("cover.png", String::new()),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "separate titlepage, but with non-existent spine item before it (no dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
			<item id="cover" href="cover.png" media-type="image/png"/>
		"#,
        spine: r#"
			<itemref idref="item3"/>
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.html",
                r#"<!DOCTYPE html><html><head><title></title></head><body><img src="cover.png"></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            ("cover.png", String::new()),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "separate titlepage (force enable)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
			<item id="cover" href="cover.png" media-type="image/png"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.html",
                r#"<!DOCTYPE html><html><head><title></title></head><body><img src="cover.png"></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            ("cover.png", String::new()),
        ],
        should_error: false,
        should_detect: true,
        dummy_titlepage: Some(true),
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "separate titlepage (force disable)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
			<item id="cover" href="cover.png" media-type="image/png"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.html",
                r#"<!DOCTYPE html><html><head><title></title></head><body><img src="cover.png"></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            ("cover.png", String::new()),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: Some(false),
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
		what: "no titlepage, many words (dummy)",
		manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
		"#,
		spine: r#"
			<itemref idref="item1"/>
		"#,
		content: vec![(
			"item1.html",
			format!(
				"<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
				format!("<div>{LOREM}</div>").repeat(5)
			),
		)],
		should_error: false,
		should_detect: true,
		dummy_titlepage: None,
	});

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "no titlepage, many images (dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
		"#,
        content: vec![(
            "item1.html",
            "<!DOCTYPE html><html><head><title></title></head><body><img><svg></svg><img><svg></svg><img><svg></svg><img><svg></svg></body></html>".to_owned(),
        )],
        should_error: false,
        should_detect: true,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "no titlepage, many short paragraphs (dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
		"#,
        content: vec![(
            "item1.html",
            r#"<!DOCTYPE html><html><head><title></title></head><body><p>Paragraph 1.</p><p>Paragraph 2.</p><p>Paragraph 3.</p><p>Paragraph 4.</p><p>Paragraph 5.</p></body></html>"#.to_owned(),
        )],
        should_error: false,
        should_detect: true,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
		what: "titlepage doesn't match heuristic but name includes cover (no dummy)",
		manifest: r#"
			<item id="item1" href="Cover.html" media-type="application/xhtml+xml"/>
		"#,
		spine: r#"
			<itemref idref="item1"/>
		"#,
		content: vec![(
			"item1.html",
			format!(
				"<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
				format!("<p>{LOREM}</p>").repeat(5)
			),
		)],
		should_error: false,
		should_detect: false,
		dummy_titlepage: None,
	});

	run_dummy_titlepage_case(DummyTitlepageCase {
		what: "titlepage doesn't match heuristic but name includes title (no dummy)",
		manifest: r#"
			<item id="item1" href="Title.html" media-type="application/xhtml+xml"/>
		"#,
		spine: r#"
			<itemref idref="item1"/>
		"#,
		content: vec![(
			"item1.html",
			format!(
				"<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
				format!("<p>{LOREM}</p>").repeat(5)
			),
		)],
		should_error: false,
		should_detect: false,
		dummy_titlepage: None,
	});

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "textual titlepage (no dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "item1.html",
                r#"<!DOCTYPE html><html><head><title></title></head><body><h1>A Long Title on a Title Page</h1><h2>The Subtitle of the Book</h2></body></html>"#.to_owned(),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
        what: "nonexistent first manifest file (no dummy)",
        manifest: r#"
			<item id="item1" href="item1.html" media-type="application/xhtml+xml"/>
			<item id="item2" href="item2.html" media-type="application/xhtml+xml"/>
		"#,
        spine: r#"
			<itemref idref="item1"/>
			<itemref idref="item2"/>
		"#,
        content: vec![
            (
                "Item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
            (
                "item2.html",
                format!(
                    "<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
                    format!("<p>{LOREM}</p>").repeat(5)
                ),
            ),
        ],
        should_error: false,
        should_detect: false,
        dummy_titlepage: None,
    });

	run_dummy_titlepage_case(DummyTitlepageCase {
		what: "empty opf package",
		manifest: "",
		spine: "",
		content: Vec::new(),
		should_error: false,
		should_detect: false,
		dummy_titlepage: None,
	});

	run_dummy_titlepage_case(DummyTitlepageCase {
		what: "bad opf package",
		manifest: r#"
			<invalid>
		"#,
		spine: r#"
			<itemref idref="item1"/>
		"#,
		content: vec![(
			"item1.html",
			format!(
				"<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
				format!("<p>{LOREM}</p>").repeat(5)
			),
		)],
		should_error: true,
		should_detect: false,
		dummy_titlepage: None,
	});

	run_dummy_titlepage_case(DummyTitlepageCase {
		what: "bad opf package, but titlepage force disabled (no dummy)",
		manifest: r#"
			<invalid>
		"#,
		spine: r#"
			<itemref idref="item1"/>
		"#,
		content: vec![(
			"item1.html",
			format!(
				"<!DOCTYPE html><html><head><title></title></head><body>{}</body></html>",
				format!("<p>{LOREM}</p>").repeat(5)
			),
		)],
		should_error: false,
		should_detect: false,
		dummy_titlepage: Some(false),
	});
}
