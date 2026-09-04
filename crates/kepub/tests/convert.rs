//! Direct port of kepubify's convert_test.go (all cases, 32-432).
//!
//! Referenced TransformOptions fields: `smarten_punctuation`, `hyphenate`,
//! `fullscreen_reading_fixes`, `extra_css`, `find_replace`, and
//! `dummy_titlepage`. `charset`, `font_size`, and `line_height` are not used by
//! convert_test.go. Go's `ConvertTestCase.Run`, `ShouldFunc.Because`, and the
//! Should* helpers are represented by `run_case`, `assert_*` helpers, and
//! inlined assertions below; no check is intentionally omitted. Go's
//! `epubFsToZip`, `overlayMapFS`, and `stringFor` are `zip_fixture`,
//! `overlay`, and `string_for`. Go's image/png blank 300x600 builder is
//! represented by a static valid PNG because stump_kepub has no image dev
//! dependency; the image is only used by the unchanged-file assertion.
//!
//! The Go runner's fs.FS-vs-zip conversion comparison cannot be reproduced
//! literally because the Rust public contract exposes only
//! `transform_epub(&[u8], ...)`, not a filesystem conversion entry point. We
//! retain the same entry-by-entry comparison (names and uncompressed bytes) by
//! converting the same generated filesystem twice with opposite ZIP entry
//! orders and comparing the resulting archives.

use std::{
	collections::BTreeMap,
	io::{Cursor, Read, Write},
};

use stump_kepub::{transform_epub, TransformOptions};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

type Files = BTreeMap<String, Vec<u8>>;
type Entries = BTreeMap<String, Vec<u8>>;

const PNG_1X1: &[u8] = &[
	0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
	b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
	0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x78,
	0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
	0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
];

// This deterministic generator preserves the Go fixture's shape and seeded
// per-chapter construction; the assertions intentionally do not depend on the
// particular pseudo-random values.
struct GoRand {
	vec: [i64; 607],
	tap: usize,
	feed: usize,
}
impl GoRand {
	fn new(seed: i64) -> Self {
		let mut r = Self {
			vec: [0; 607],
			tap: 0,
			feed: 0,
		};
		let max = (1_i64 << 31) - 1;
		let mut x = seed % max;
		if x < 0 {
			x += max;
		}
		if x == 0 {
			x = 89_482_311;
		}
		for i in 0..20 {
			x = (x * 48_271) % max;
			if x < 0 {
				x += max;
			}
			r.vec[i] = x;
		}
		for i in 0..7 {
			for j in 0..607 {
				let k = (i + 1) * 20 + j;
				let k = if k >= 607 { k - 607 } else { k };
				r.vec[k] += r.vec[j];
			}
		}
		r.tap = 0;
		r.feed = 607 - 273;
		r
	}
	fn int63(&mut self) -> i64 {
		self.tap = if self.tap == 0 { 606 } else { self.tap - 1 };
		self.feed = if self.feed == 0 { 606 } else { self.feed - 1 };
		let x = self.vec[self.feed].wrapping_add(self.vec[self.tap]);
		self.vec[self.feed] = x;
		x & i64::MAX
	}
	fn intn(&mut self, n: i64) -> i64 {
		if n > 0 && (n & (n - 1)) == 0 {
			return self.int63() & (n - 1);
		}
		let max = i64::MAX - ((1_u64 << 63) % n as u64) as i64;
		let mut v = self.int63();
		while v > max {
			v = self.int63();
		}
		v % n
	}
}

fn string_for<F: Fn(String, usize) -> String>(
	template: &str,
	start: usize,
	end: usize,
	f: F,
) -> String {
	(start..end).map(|i| f(template.to_owned(), i)).collect()
}
fn replace_bytes(input: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
	if from.is_empty() {
		return input.to_vec();
	}
	let mut out = Vec::with_capacity(input.len());
	let mut rest = input;
	while let Some(index) = rest.windows(from.len()).position(|window| window == from) {
		out.extend_from_slice(&rest[..index]);
		out.extend_from_slice(to);
		rest = &rest[index + from.len()..];
	}
	out.extend_from_slice(rest);
	out
}

fn base_fixture() -> Files {
	let mut files = Files::new();
	files.insert("mimetype".into(), b"application/epub+zip".to_vec());
	files.insert(
		"META-INF/container.xml".into(),
		br##"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
	<rootfiles>
		<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
	</rootfiles>
</container>
"##
		.to_vec(),
	);
	let manifest = string_for(
		r##"
		<item id="xhtml_ch00" href="xhtml/ch00.xhtml" media-type="application/xhtml+xml"/>"##,
		1,
		100,
		|s, i| s.replace("00", &format!("{i:02}")),
	);
	let spine = string_for(
		r##"
		<itemref idref="xhtml_ch00"/>"##,
		1,
		100,
		|s, i| s.replace("00", &format!("{i:02}")),
	);
	let opf = format!(
		r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:title>Test</dc:title>
	</metadata>
	<manifest>
		<item href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
		<item id="xhtml_title" href="xhtml/title.xhtml" media-type="application/xhtml+xml"/>{manifest}
		<item id="cover" href="cover.png" media-type="image/png"/>
	</manifest>
	<spine>
		<itemref idref="xhtml_title"/>{spine}
	</spine>
	<guide>
		<reference href="xhtml/title.xhtml" title="Cover Page" type="cover"/>
	</guide>
</package>
"##
	);
	files.insert("OEBPS/content.opf".into(), opf.into_bytes());
	files.insert("OEBPS/cover.png".into(), PNG_1X1.to_vec());
	let nav = format!(
		r##"<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="en">
	<head>
		<title>Test Book</title>
		<meta charset="utf-8/>
	</head>
	<body>
		<nav epub:type="toc">
			<ol>
				<li><a href="xhtml/title.xhtml">Cover Page</a></li>{}
			</ol>
		</nav>
	</body>
</html>	
"##,
		string_for(
			r##"
				<li><a href="xhtml/ch00.xhtml">Chapter #</a></li>"##,
			1,
			100,
			|s, i| s
				.replace("00", &format!("{i:02}"))
				.replace('#', &i.to_string())
		)
	);
	files.insert("OEBPS/nav.xhtml".into(), nav.into_bytes());
	let title = br##"<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="en">
	<head>
		<title>Test Book</title>
		<meta charset="utf-8/>
	</head>
	<body>
		<img style="height: 100%; max-width: 100%" src="cover.png" alt="Cover image"/>
	</body>
</html>	
"##;
	files.insert("OEBPS/xhtml/title.xhtml".into(), title.to_vec());

	for i in 1..100 {
		let mut body = format!("<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"en\">\n<head>\n\t<title>Test Book</title>\n\t<meta charset=\"utf-8/>\n</head>\n<body>\n<h1>Chapter {i}</h1>\n");
		let mut r = GoRand::new(i as i64);
		for _ in 0..(r.intn(200) + 50) {
			body.push_str("\t\t");
			match r.intn(3) {
				0 => {
					body.push_str("<p>");
					body.push_str(match r.intn(4) {
                        0 => "Lorem ipsum dolor sit amet, consectetur adipisicing elit. Nostrum eveniet doloremque pariatur facilis doloribus id amet laudantium voluptatibus, libero a esse. Corrupti sunt earum laborum eum unde aspernatur assumenda deleniti.",
                        1 => "Lorem <b>ipsum dolor sit amet consectetur</b> adipisicing elit. Non pariatur veniam ratione cupiditate et, enim <i>aperiam necessitatibus</i> ad quisquam molestiae quae delectus tempora fuga distinctio minima repellendus, maxime nobis? Eligendi!",
                        2 => "Lorem ipsum dolor sit amet consectetur <i>adipisicing elit. Beatae alias ipsa</i>, quisquam deserunt et error sequi deleniti odio harum quae odit tempora recusandae magni expedita temporibus, quia modi officiis dolorum.",
                        _ => "Lorem ipsum, dolor sit amet consectetur adipisicing elit. Eligendi, vero! Mollitia explicabo quod aperiam hic dolores commodi vero perferendis sequi. Ratione accusantium repellat distinctio quaerat architecto fuga totam, minima deserunt?",
                    });
					body.push_str("</p>");
				},
				1 => {
					body.push_str("<img src=\"cover.png\"/>\n");
				},
				_ => {
					body.push_str("<ul>");
					for _ in 0..(r.intn(5) + 5) {
						body.push_str("<li>Test item.</li>");
					}
					body.push_str("</ul>");
				},
			}
			body.push('\n');
		}
		body.push_str("\n</body>\n</html>\t\n");
		files.insert(format!("OEBPS/xhtml/ch{i:02}.xhtml"), body.into_bytes());
	}
	files
}

fn overlay(
	mut original: Files,
	changes: impl IntoIterator<Item = (&'static str, Option<Vec<u8>>)>,
) -> Files {
	for (name, value) in changes {
		match value {
			Some(bytes) => {
				original.insert(name.into(), bytes);
			},
			None => {
				original.remove(name);
			},
		}
	}
	original
}

fn zip_fixture(files: &Files) -> Vec<u8> {
	let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
	for (name, data) in files {
		let method = if name == "mimetype" {
			CompressionMethod::Stored
		} else {
			CompressionMethod::Deflated
		};
		let options = SimpleFileOptions::default().compression_method(method);
		writer
			.start_file(name.as_str(), options)
			.expect("create source ZIP entry");
		writer.write_all(data).expect("write source ZIP entry");
	}
	writer.finish().expect("finish source ZIP").into_inner()
}
fn zip_fixture_reversed(files: &Files) -> Vec<u8> {
	let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
	for (name, data) in files.iter().rev() {
		let method = if name == "mimetype" {
			CompressionMethod::Stored
		} else {
			CompressionMethod::Deflated
		};
		let options = SimpleFileOptions::default().compression_method(method);
		writer
			.start_file(name.as_str(), options)
			.expect("create reversed source ZIP entry");
		writer
			.write_all(data)
			.expect("write reversed source ZIP entry");
	}
	writer
		.finish()
		.expect("finish reversed source ZIP")
		.into_inner()
}

fn archive_entries(bytes: &[u8]) -> Entries {
	let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open output EPUB");
	let mut entries = Entries::new();
	for i in 0..archive.len() {
		let mut file = archive.by_index(i).expect("read output entry");
		if !file.is_dir() {
			let mut data = Vec::new();
			file.read_to_end(&mut data)
				.expect("read output entry bytes");
			entries.insert(file.name().to_owned(), data);
		}
	}
	entries
}

fn source_documents(files: &Files) -> Vec<String> {
	files
		.keys()
		.filter(|name| {
			name.starts_with("OEBPS/")
				&& !matches!(
					name.as_str(),
					"OEBPS/content.opf"
						| "OEBPS/content.xhtml"
						| "OEBPS/nav.xhtml"
						| "OEBPS/cover.png"
				)
		})
		.cloned()
		.collect()
}

fn assert_sane_documents(
	source: &Files,
	output: &Entries,
	with_new: usize,
) -> Result<(), String> {
	for name in source_documents(source) {
		if !output.contains_key(&name) {
			return Err(format!("EPUB has {name:?}, but KEPUB does not"));
		}
	}
	let source_count = source_documents(source).len();
	let output_count = output
		.keys()
		.filter(|name| {
			name.starts_with("OEBPS/")
				&& !matches!(
					name.as_str(),
					"OEBPS/content.opf"
						| "OEBPS/content.xhtml"
						| "OEBPS/nav.xhtml"
						| "OEBPS/cover.png"
				)
		})
		.count();
	if output_count != source_count + with_new {
		return Err(format!(
			"KEPUB has {output_count} content documents, expected {}",
			source_count + with_new
		));
	}
	Ok(())
}

fn assert_all_docs_have_spans(output: &Entries, excluded: &[&str]) -> Result<(), String> {
	for (name, bytes) in output {
		if !name.starts_with("OEBPS/")
			|| !name.ends_with(".xhtml")
			|| name == "OEBPS/nav.xhtml"
			|| excluded.iter().any(|x| *x == name)
		{
			continue;
		}
		let text = String::from_utf8_lossy(bytes);
		if !text.contains("class=\"koboSpan\"") {
			return Err(format!("no spans in {name}"));
		}
	}
	Ok(())
}
fn assert_all_docs_contain(output: &Entries, needle: &str) -> Result<(), String> {
	for (name, bytes) in output {
		if !name.starts_with("OEBPS/")
			|| !name.ends_with(".xhtml")
			|| name == "OEBPS/nav.xhtml"
		{
			continue;
		}
		if !String::from_utf8_lossy(bytes).contains(needle) {
			return Err(format!("document {name} missing {needle:?}"));
		}
	}
	Ok(())
}

fn run_case<F>(
	name: &str,
	source: Files,
	options: TransformOptions,
	should_error: bool,
	check: F,
) where
	F: Fn(&Files, &Entries) -> Result<(), String>,
{
	let source_zip = zip_fixture(&source);
	let result = transform_epub(&source_zip, &options);
	if should_error {
		assert!(result.is_err(), "case {name:?}: expected error");
		return;
	}
	let output =
		result.unwrap_or_else(|error| panic!("case {name:?}: unexpected error: {error}"));
	let entries = archive_entries(&output);
	check(&source, &entries)
		.unwrap_or_else(|error| panic!("case {name:?}: check: {error}"));
	// Go's runner compares sorted entry names and SHA-1 content hashes. Byte
	// equality is stronger and avoids adding a hashing-only test dependency.
	let second_source_zip = zip_fixture_reversed(&source);
	let second = transform_epub(&second_source_zip, &options).expect("repeat conversion");
	assert_eq!(
		entries,
		archive_entries(&second),
		"case {name:?}: entry-by-entry conversion mismatch"
	);
}

#[test]
fn test_convert() {
	run_case(
		"empty source",
		Files::new(),
		TransformOptions::default(),
		true,
		|_, _| Ok(()),
	);

	run_case(
		"invalid container",
		overlay(
			base_fixture(),
			[("META-INF/container.xml", Some(b"test".to_vec()))],
		),
		TransformOptions::default(),
		true,
		|_, _| Ok(()),
	);
	run_case("invalid container version", overlay(base_fixture(), [("META-INF/container.xml", Some(br##"<?xml version="1.0" encoding="UTF-8"?><container version="1234.5" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"></container>"##.to_vec()))]), TransformOptions::default(), true, |_, _| Ok(()));
	run_case("no package documents in container", overlay(base_fixture(), [("META-INF/container.xml", Some(br##"<?xml version="1.0" encoding="UTF-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"></container>"##.to_vec()))]), TransformOptions::default(), true, |_, _| Ok(()));
	run_case(
		"invalid package",
		overlay(
			base_fixture(),
			[("OEBPS/content.opf", Some(b"test".to_vec()))],
		),
		TransformOptions::default(),
		true,
		|_, _| Ok(()),
	);

	run_case(
		"simple",
		base_fixture(),
		TransformOptions::default(),
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			assert_all_docs_have_spans(output, &["OEBPS/xhtml/title.xhtml"])?;
			if output.get("OEBPS/cover.png") != source.get("OEBPS/cover.png") {
				return Err("cover.png changed".into());
			}
			Ok(())
		},
	);
	run_case(
		"with missing mimetype",
		overlay(base_fixture(), [("mimetype", None)]),
		TransformOptions::default(),
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			assert_all_docs_have_spans(output, &["OEBPS/xhtml/title.xhtml"])?;
			if output.get("OEBPS/cover.png") != source.get("OEBPS/cover.png") {
				return Err("cover.png changed".into());
			}
			Ok(())
		},
	);
	run_case(
		"with incorrect content filename casing",
		overlay(
			base_fixture(),
			[
				("OEBPS/xhtml/ch01.xhtml", None),
				(
					"OEBPS/xhtml/Ch01.XHTML",
					Some(base_fixture()["OEBPS/xhtml/ch01.xhtml"].clone()),
				),
			],
		),
		TransformOptions::default(),
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			if !output.contains_key("OEBPS/xhtml/Ch01.XHTML") {
				return Err("missing case-preserved chapter".into());
			}
			let text = String::from_utf8_lossy(&output["OEBPS/xhtml/Ch01.XHTML"]);
			if !text.contains("koboSpan") {
				return Err("case-preserved chapter has no spans".into());
			}
			Ok(())
		},
	);

	let wrong_opf = String::from_utf8(base_fixture()["OEBPS/content.opf"].clone())
		.unwrap()
		.replace(
			"href=\"xhtml/ch01.xhtml\" media-type=\"application/xhtml+xml\"/>",
			"href=\"xhtml/ch01.xhtml\" media-type=\"application/xml\"/><!--1-->",
		)
		.replace(
			"href=\"xhtml/ch02.xhtml\" media-type=\"application/xhtml+xml\"/>",
			"href=\"xhtml/ch02.xhtml\" media-type=\"text/xml\"/><!--2-->",
		)
		.replace(
			"href=\"xhtml/ch03.xhtml\" media-type=\"application/xhtml+xml\"/>",
			"href=\"xhtml/ch03.xhtml\" media-type=\"text/html\"/><!--3-->",
		)
		.replace(
			"href=\"xhtml/ch04.xhtml\" media-type=\"application/xhtml+xml\"/>",
			"href=\"xhtml/ch04.xml\" media-type=\"application/xhtml+xml\"/><!--4-->",
		)
		.replace(
			"href=\"xhtml/ch05.xhtml\" media-type=\"application/xhtml+xml\"/>",
			"href=\"xhtml/ch05.XHTML\" media-type=\"application/xhtml+xml\"/><!--5-->",
		)
		.replace(
			"href=\"xhtml/ch06.xhtml\" media-type=\"application/xhtml+xml\"/>",
			"href=\"xhtml/ch06.invalid\" media-type=\"application/xhtml+xml\"/><!--6-->",
		);
	let mut wrong = base_fixture();
	wrong.insert(
		"META-INF/container.xml".into(),
		replace_bytes(
			&wrong["META-INF/container.xml"],
			b"content.opf",
			b"content.xhtml",
		),
	);
	wrong.remove("OEBPS/content.opf");
	wrong.insert("OEBPS/content.xhtml".into(), wrong_opf.into_bytes());
	for (old, new) in [
		("OEBPS/xhtml/ch04.xhtml", "OEBPS/xhtml/ch04.xml"),
		("OEBPS/xhtml/ch05.xhtml", "OEBPS/xhtml/ch05.XHTML"),
		("OEBPS/xhtml/ch06.xhtml", "OEBPS/xhtml/ch06.invalid"),
	] {
		let data = wrong.remove(old).unwrap();
		wrong.insert(new.into(), data);
	}
	let wrong_opts = TransformOptions {
		dummy_titlepage: Some(false),
		..TransformOptions::default()
	};
	run_case(
		"with incorrect extensions and mimetypes",
		wrong,
		wrong_opts,
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			for i in 1..=6 {
				if !String::from_utf8_lossy(&output["OEBPS/content.xhtml"])
					.contains(&format!("<!--{i}-->"))
				{
					return Err("opf patch failed".into());
				}
			}
			for name in [
				"OEBPS/xhtml/ch01.xhtml",
				"OEBPS/xhtml/ch02.xhtml",
				"OEBPS/xhtml/ch03.xhtml",
				"OEBPS/xhtml/ch04.xml",
				"OEBPS/xhtml/ch05.XHTML",
				"OEBPS/xhtml/ch06.invalid",
			] {
				if !String::from_utf8_lossy(&output[name]).contains("koboSpan") {
					return Err(format!("no spans in {name}"));
				}
			}
			if String::from_utf8_lossy(&output["OEBPS/content.xhtml"])
				.contains("koboSpan")
			{
				return Err("package document transformed as content".into());
			}
			Ok(())
		},
	);

	let force = TransformOptions {
		dummy_titlepage: Some(true),
		..TransformOptions::default()
	};
	run_case(
		"with cover fix forced",
		base_fixture(),
		force,
		false,
		|source, output| {
			assert_sane_documents(source, output, 1)?;
			if !output.contains_key("OEBPS/kepubify-titlepage-dummy.xhtml") {
				return Err("missing dummy titlepage".into());
			}
			if output.get("OEBPS/cover.png") != source.get("OEBPS/cover.png") {
				return Err("cover.png changed".into());
			}
			assert_all_docs_have_spans(
				output,
				&[
					"OEBPS/xhtml/title.xhtml",
					"OEBPS/kepubify-titlepage-dummy.xhtml",
				],
			)
		},
	);
	let detected = overlay(
		base_fixture(),
		[(
			"OEBPS/content.opf",
			Some(replace_bytes(
				&base_fixture()["OEBPS/content.opf"],
				br##"<itemref idref="xhtml_title"/>"##,
				br##"<!--removed-->"##,
			)),
		)],
	);
	run_case(
		"with cover fix detected",
		detected,
		TransformOptions::default(),
		false,
		|source, output| {
			assert_sane_documents(source, output, 1)?;
			if !String::from_utf8_lossy(&output["OEBPS/content.opf"])
				.contains("<!--removed-->")
				|| !output.contains_key("OEBPS/kepubify-titlepage-dummy.xhtml")
			{
				return Err("cover fix not detected".into());
			}
			Ok(())
		},
	);
	let detected_disabled = overlay(
		base_fixture(),
		[(
			"OEBPS/content.opf",
			Some(replace_bytes(
				&base_fixture()["OEBPS/content.opf"],
				br##"<itemref idref="xhtml_title"/>"##,
				br##"<!--removed-->"##,
			)),
		)],
	);
	let disabled = TransformOptions {
		dummy_titlepage: Some(false),
		..TransformOptions::default()
	};
	run_case(
		"with cover fix detected and disabled",
		detected_disabled,
		disabled,
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			if !String::from_utf8_lossy(&output["OEBPS/content.opf"])
				.contains("<!--removed-->")
				|| output.contains_key("OEBPS/kepubify-titlepage-dummy.xhtml")
			{
				return Err("disabled cover fix mismatch".into());
			}
			Ok(())
		},
	);
	let replacement = overlay(base_fixture(), [("OEBPS/xhtml/ch01.xhtml", Some(br##"<!DOCTYPE html><html><head><title>Replaced Chapter</title></head><body><p>Lorem ipsum dolor sit amet.</p></body></html>"##.to_vec()))]);
	let replacements = TransformOptions {
		find_replace: [
			("Lorem", "*****"),
			("*****", "***"),
			("ipsum", "dolor"),
			("dolor", "ipsum"),
			("<i>", "<em>"),
			("</i>", "</em>"),
		]
		.into_iter()
		.map(|(a, b)| (a.into(), b.into()))
		.collect(),
		..TransformOptions::default()
	};
	run_case(
		"with replacement",
		replacement,
		replacements,
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			assert_all_docs_have_spans(output, &["OEBPS/xhtml/title.xhtml"])?;
			for (name, bytes) in output {
				let text = String::from_utf8_lossy(bytes);
				if name.ends_with(".xhtml")
					&& (text.contains("Lorem")
						|| text.contains("dolor")
						|| text.contains("<i>")
						|| text.contains("</i>"))
				{
					return Err(format!("replacement remained in {name}"));
				}
			}
			if !String::from_utf8_lossy(&output["OEBPS/xhtml/ch01.xhtml"])
				.contains(">*** ipsum ipsum sit amet.<")
			{
				return Err("incorrect replacement".into());
			}
			Ok(())
		},
	);

	let cleanup = overlay(base_fixture(), [("META-INF/calibre_bookmarks.txt", Some(b"dummy".to_vec())), ("META-INF/not_calibre_bookmarks.txt", Some(b"dummy".to_vec())), ("OEBPS/xhtml/ch01.xhtml", Some(br##"<!DOCTYPE html><html><head><title>Replaced Chapter</title></head><body><o:p></o:p><p>Test</p></body></html>"##.to_vec()))]);
	run_case(
		"with cleanup",
		cleanup,
		TransformOptions::default(),
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			if output.contains_key("META-INF/calibre_bookmarks.txt")
				|| !output.contains_key("META-INF/not_calibre_bookmarks.txt")
			{
				return Err("cleanup mismatch".into());
			}
			let text = String::from_utf8_lossy(&output["OEBPS/xhtml/ch01.xhtml"]);
			if text.contains("o:p") {
				return Err("contains o:p tag".into());
			}
			Ok(())
		},
	);

	let smart = overlay(base_fixture(), [("OEBPS/xhtml/ch01.xhtml", Some(br##"<!DOCTYPE html><html><head><title>Replaced Chapter</title></head><body><p>"asd sdf" 'asd asd' asd-sdf asd - sdf asd--sdf asd -- sdf asd... sdf 1/2 1/4 3/4 (c) (r) (tm)</p></body></html>"##.to_vec()))]);
	let smart_opts = TransformOptions {
		smarten_punctuation: true,
		..TransformOptions::default()
	};
	run_case(
		"with smart punctuation",
		smart,
		smart_opts,
		false,
		|_, output| {
			let text = String::from_utf8_lossy(&output["OEBPS/xhtml/ch01.xhtml"]);
			if !text.contains("koboSpan") {
				return Err("no spans in smart punctuation document".into());
			}
			for expected in [
				"“asd sdf”",
				"‘asd asd’",
				"asd-sdf",
				"asd - sdf",
				"asd–sdf",
				"asd – sdf",
				"asd…",
				"½",
				"¼",
				"¾",
				"©",
				"®",
				"™",
			] {
				if !text.contains(expected) {
					return Err(format!("missing {expected}"));
				}
			}
			Ok(())
		},
	);
	let hyphenate = TransformOptions {
		hyphenate: Some(true),
		..TransformOptions::default()
	};
	run_case(
		"with hyphenation enable css",
		base_fixture(),
		hyphenate,
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			assert_all_docs_have_spans(output, &["OEBPS/xhtml/title.xhtml"])?;
			assert_all_docs_contain(output, "-webkit-hyphens: auto")
		},
	);
	let no_hyphenate = TransformOptions {
		hyphenate: Some(false),
		..TransformOptions::default()
	};
	run_case(
		"with hyphenation disable css",
		base_fixture(),
		no_hyphenate,
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			assert_all_docs_have_spans(output, &["OEBPS/xhtml/title.xhtml"])?;
			assert_all_docs_contain(output, "-moz-hyphens: none !important")
		},
	);
	let fullscreen = TransformOptions {
		fullscreen_reading_fixes: true,
		..TransformOptions::default()
	};
	run_case(
		"with full-screen fixes css",
		base_fixture(),
		fullscreen,
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			assert_all_docs_have_spans(output, &["OEBPS/xhtml/title.xhtml"])?;
			assert_all_docs_contain(output, "body>div")
		},
	);
	let custom = TransformOptions {
		extra_css: vec![
			("kepubify-extracss".into(), ".css1 {}".into()),
			("kepubify-extracss".into(), ".css2 {}".into()),
		],
		..TransformOptions::default()
	};
	run_case(
		"with custom css",
		base_fixture(),
		custom,
		false,
		|source, output| {
			assert_sane_documents(source, output, 0)?;
			assert_all_docs_have_spans(output, &["OEBPS/xhtml/title.xhtml"])?;
			assert_all_docs_contain(output, ".css1")
				.and_then(|_| assert_all_docs_contain(output, ".css2"))
		},
	);
}
