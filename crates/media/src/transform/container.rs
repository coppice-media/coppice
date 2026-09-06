//! Output containers for transformed comics: a stored CBZ (with optional
//! `ComicInfo.xml`) and a fixed-layout KEPUB for Kobo delivery.
//!
//! The KEPUB pages are run through `stump_kepub`'s public content transform,
//! so Kobo's expected DOM wrapper (`div#book-columns > div#book-inner`), the
//! `kobostylehacks` style element, and the `koboSpan` sentence machinery all
//! come from the same code path used for reflowable KEPUB conversion.

use std::io::{Read, Seek, Write};
use std::path::Path;

use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::error::{TransformError, TransformResult};
use super::pipeline::EncodedPage;
use crate::PathUtils;

/// Extract a root-level `ComicInfo.xml` from a CBZ, if present.
///
/// Uses the same entry conventions as the ZIP processor (exact file name,
/// hidden files ignored).
pub fn comic_info_xml(path: &Path) -> Option<Vec<u8>> {
	let file = std::fs::File::open(path).ok()?;
	let mut archive = zip::ZipArchive::new(file).ok()?;

	for index in 0..archive.len() {
		let Ok(mut entry) = archive.by_index(index) else {
			continue;
		};
		if entry.is_dir() {
			continue;
		}
		let entry_path = entry
			.enclosed_name()
			.map(|path| path.to_path_buf())
			.unwrap_or_else(|| Path::new(entry.name()).to_path_buf());
		if entry_path.is_hidden_file() {
			continue;
		}
		if entry_path
			.file_name()
			.is_some_and(|name| name == "ComicInfo.xml")
		{
			let mut contents = Vec::new();
			if entry.read_to_end(&mut contents).is_ok() {
				return Some(contents);
			}
			return None;
		}
	}

	None
}

/// Write a stored (uncompressed) CBZ: `ComicInfo.xml` first when provided,
/// then the pages in delivery order.
pub fn build_cbz<W: Write + Seek>(
	pages: impl Iterator<Item = TransformResult<EncodedPage>>,
	comic_info: Option<&[u8]>,
	sink: W,
) -> TransformResult<W> {
	let mut zip = ZipWriter::new(sink);
	let options: FileOptions<()> = FileOptions::default()
		.compression_method(CompressionMethod::Stored)
		.unix_permissions(0o644);

	if let Some(comic_info) = comic_info {
		zip.start_file("ComicInfo.xml", options)?;
		zip.write_all(comic_info)?;
	}

	for (index, page) in pages.enumerate() {
		let page = page?;
		zip.start_file(
			format!("pages/{index:04}.{}", page.format.extension()),
			options,
		)?;
		zip.write_all(&page.bytes)?;
	}

	Ok(zip.finish()?)
}

/// Write a fixed-layout KEPUB: one pre-paginated XHTML page per image, each
/// image sized to the panel, transformed through `stump_kepub`.
///
/// Pages are written as they arrive, so a whole comic is never held in
/// memory; the package document is written last, once every page is known.
pub fn build_kepub<W: Write + Seek>(
	pages: impl Iterator<Item = TransformResult<EncodedPage>>,
	title: &str,
	sink: W,
) -> TransformResult<W> {
	#[cfg(feature = "kepub")]
	{
		let mut zip = ZipWriter::new(sink);
		let stored: FileOptions<()> = FileOptions::default()
			.compression_method(CompressionMethod::Stored)
			.unix_permissions(0o644);
		let deflated: FileOptions<()> = FileOptions::default()
			.compression_method(CompressionMethod::Deflated)
			.unix_permissions(0o644);

		// The mimetype entry must be first and stored for EPUB readers to
		// recognise the container.
		zip.start_file("mimetype", stored)?;
		zip.write_all(b"application/epub+zip")?;

		zip.start_file("META-INF/container.xml", deflated)?;
		zip.write_all(container_xml().as_bytes())?;

		let mut manifest = String::new();
		let mut spine = String::new();
		let mut viewport = None;
		let mut page_count = 0usize;
		for (index, page) in pages.enumerate() {
			let page = page?;
			viewport.get_or_insert((page.width, page.height));
			page_count += 1;

			let image_href = format!("images/p{index:04}.{}", page.format.extension());
			let page_href = format!("text/p{index:04}.xhtml");
			manifest.push_str(&format!(
				"    <item id=\"page-{index}\" href=\"{page_href}\" media-type=\"application/xhtml+xml\"/>\n"
			));
			manifest.push_str(&format!(
				"    <item id=\"img-{index}\" href=\"{image_href}\" media-type=\"{}\"/>\n",
				page.format.media_type()
			));
			spine.push_str(&format!(
				"    <itemref idref=\"page-{index}\" properties=\"layout-pre-paginated\"/>\n"
			));

			zip.start_file(format!("OEBPS/{image_href}"), stored)?;
			zip.write_all(&page.bytes)?;

			let xhtml = page_xhtml(index, &page);
			let transformed = stump_kepub::transform_content(
				xhtml.as_bytes(),
				&stump_kepub::TransformOptions::default(),
			)
			.map_err(|error| {
				TransformError::Other(format!(
					"KEPUB content transform failed for page {index}: {error}"
				))
			})?;
			zip.start_file(format!("OEBPS/{page_href}"), deflated)?;
			zip.write_all(&transformed)?;
		}

		if page_count == 0 {
			return Err(TransformError::Other(
				"refusing to build an empty KEPUB".to_string(),
			));
		}

		zip.start_file("OEBPS/content.opf", deflated)?;
		zip.write_all(content_opf(title, &manifest, &spine, viewport).as_bytes())?;

		Ok(zip.finish()?)
	}

	#[cfg(not(feature = "kepub"))]
	{
		let _ = (pages, title, sink);
		Err(TransformError::FeatureDisabled("KEPUB"))
	}
}

#[cfg(feature = "kepub")]
fn container_xml() -> String {
	String::from(
		r#"<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#,
	)
}

#[cfg(feature = "kepub")]
fn content_opf(
	title: &str,
	manifest: &str,
	spine: &str,
	viewport: Option<(u32, u32)>,
) -> String {
	let viewport_meta = viewport.map_or_else(String::new, |(width, height)| {
		format!(
			"    <meta property=\"rendition:viewport\">width={width}, height={height}</meta>\n"
		)
	});
	format!(
		r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id" xml:lang="en">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:stump:comic-transform:{}</dc:identifier>
    <dc:title>{}</dc:title>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">{}</meta>
    <meta property="rendition:layout">pre-paginated</meta>
{}  </metadata>
  <manifest>
{}  </manifest>
  <spine>
{}  </spine>
</package>
"#,
		uuid_slug(title),
		xml_escape(title),
		chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
		viewport_meta,
		manifest,
		spine,
	)
}

/// One fixed-layout page: the image fills the viewport and is never
/// letterboxed by the reader.
#[cfg(feature = "kepub")]
fn page_xhtml(index: usize, page: &EncodedPage) -> String {
	format!(
		r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" epub:type="fiction">
<head>
<title>Page {}</title>
<style type="text/css">body {{ margin: 0; padding: 0; background-color: #000; }}</style>
<meta name="viewport" content="width={}, height={}"/>
</head>
<body>
<div style="text-align: center; padding: 0; margin: 0; width: {}px; height: {}px;">
<img src="../images/p{:04}.{}" alt="Page {}" style="width: {}px; height: {}px; max-width: 100%; max-height: 100%;"/>
</div>
</body>
</html>
"#,
		index,
		page.width,
		page.height,
		page.width,
		page.height,
		index,
		page.format.extension(),
		index,
		page.width,
		page.height,
	)
}

#[cfg(feature = "kepub")]
fn xml_escape(value: &str) -> String {
	value
		.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}

#[cfg(feature = "kepub")]
fn uuid_slug(value: &str) -> String {
	value
		.chars()
		.map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::transform::pipeline::PageFormat;

	fn page(width: u32, height: u32, format: PageFormat) -> EncodedPage {
		let bytes = match format {
			PageFormat::Jpeg => vec![0xFF, 0xD8, 0xFF, 0xDB, 1, 2, 3],
			PageFormat::Png => vec![0x89, b'P', b'N', b'G', 1, 2, 3],
			PageFormat::Webp => {
				let mut bytes = b"RIFF".to_vec();
				bytes.extend_from_slice(&[0, 0, 0, 0]);
				bytes.extend_from_slice(b"WEBP");
				bytes.extend_from_slice(&[1, 2, 3]);
				bytes
			},
		};
		EncodedPage {
			width,
			height,
			format,
			bytes,
		}
	}

	fn pages(count: usize) -> impl Iterator<Item = TransformResult<EncodedPage>> {
		(0..count).map(move |i| {
			Ok(page(
				100 + i as u32,
				200,
				if i % 2 == 0 {
					PageFormat::Jpeg
				} else {
					PageFormat::Webp
				},
			))
		})
	}

	fn read_entries(buffer: &[u8]) -> Vec<(String, CompressionMethod, Vec<u8>)> {
		let cursor = Cursor::new(buffer);
		let mut archive = zip::ZipArchive::new(cursor).unwrap();
		let mut entries = Vec::new();
		for index in 0..archive.len() {
			let mut entry = archive.by_index(index).unwrap();
			let mut contents = Vec::new();
			entry.read_to_end(&mut contents).unwrap();
			entries.push((entry.name().to_string(), entry.compression(), contents));
		}
		entries
	}

	use std::io::{Cursor, Read};

	#[test]
	fn cbz_has_comic_info_first_and_stored_pages() {
		let comic_info = br#"<ComicInfo><Title>Science Comics</Title></ComicInfo>"#;
		let mut buffer = Cursor::new(Vec::new());
		build_cbz(pages(3), Some(comic_info), &mut buffer).unwrap();

		let entries = read_entries(buffer.get_ref());
		assert_eq!(entries.len(), 4);
		assert_eq!(entries[0].0, "ComicInfo.xml");
		assert_eq!(entries[0].2, comic_info.to_vec());
		// Pages alternate jpg/webp (see `pages`), named by delivery index.
		for (index, entry) in entries[1..].iter().enumerate() {
			let extension = if index % 2 == 0 { "jpg" } else { "webp" };
			assert_eq!(entry.0, format!("pages/{index:04}.{extension}"));
			assert_eq!(entry.1, CompressionMethod::Stored);
		}
	}

	#[test]
	fn cbz_without_comic_info_omits_the_entry() {
		let mut buffer = Cursor::new(Vec::new());
		build_cbz(pages(2), None, &mut buffer).unwrap();
		let entries = read_entries(buffer.get_ref());
		assert_eq!(entries.len(), 2);
		assert!(entries
			.iter()
			.all(|(name, _, _)| name.starts_with("pages/")));
	}

	#[test]
	fn cbz_propagates_page_errors() {
		let results: Vec<TransformResult<EncodedPage>> = vec![
			Ok(page(10, 10, PageFormat::Jpeg)),
			Err(TransformError::Decode("boom".to_string())),
		];
		let mut buffer = Cursor::new(Vec::new());
		assert!(build_cbz(results.into_iter(), None, &mut buffer).is_err());
	}

	#[cfg(feature = "kepub")]
	#[test]
	fn kepub_container_is_structurally_valid() {
		let mut buffer = Cursor::new(Vec::new());
		build_kepub(pages(3), "Science Comics #1", &mut buffer).unwrap();
		let entries = read_entries(buffer.get_ref());

		// mimetype first + stored
		assert_eq!(entries[0].0, "mimetype");
		assert_eq!(entries[0].1, CompressionMethod::Stored);
		assert_eq!(entries[0].2, b"application/epub+zip");

		let names: Vec<&str> = entries.iter().map(|entry| entry.0.as_str()).collect();
		assert!(names.contains(&"META-INF/container.xml"));
		assert!(names.contains(&"OEBPS/content.opf"));
		assert_eq!(
			names
				.iter()
				.filter(|name| name.starts_with("OEBPS/text/"))
				.count(),
			3
		);
		assert_eq!(
			names
				.iter()
				.filter(|name| name.starts_with("OEBPS/images/"))
				.count(),
			3
		);

		// Page order follows delivery order (jpg, webp, jpg).
		assert!(names.contains(&"OEBPS/images/p0000.jpg"));
		assert!(names.contains(&"OEBPS/images/p0001.webp"));
		assert!(names.contains(&"OEBPS/images/p0002.jpg"));

		let opf = &entries
			.iter()
			.find(|entry| entry.0 == "OEBPS/content.opf")
			.unwrap()
			.2;
		let opf = std::str::from_utf8(opf).unwrap();
		assert!(opf.contains("<meta property=\"rendition:layout\">pre-paginated</meta>"));
		assert!(opf.contains("properties=\"layout-pre-paginated\""));
		assert!(opf.contains("<dc:title>Science Comics #1</dc:title>"));
		assert!(opf.contains("media-type=\"image/webp\""));

		let container = &entries
			.iter()
			.find(|entry| entry.0 == "META-INF/container.xml")
			.unwrap()
			.2;
		assert!(std::str::from_utf8(container)
			.unwrap()
			.contains("OEBPS/content.opf"));
	}

	#[cfg(feature = "kepub")]
	#[test]
	fn kepub_pages_go_through_stump_kepub_transform() {
		let mut buffer = Cursor::new(Vec::new());
		build_kepub(pages(1), "T", &mut buffer).unwrap();
		let entries = read_entries(buffer.get_ref());
		let page = &entries
			.iter()
			.find(|entry| entry.0 == "OEBPS/text/p0000.xhtml")
			.unwrap()
			.2;
		let page = std::str::from_utf8(page).unwrap();

		// stump_kepub's DOM transform applied: wrapper + style hacks + spans.
		assert!(page.contains("book-columns"));
		assert!(page.contains("book-inner"));
		assert!(page.contains("kobostylehacks"));
		assert!(page.contains("img"));
	}

	#[cfg(feature = "kepub")]
	#[test]
	fn kepub_refuses_empty_input() {
		let mut buffer = Cursor::new(Vec::new());
		let empty: Vec<TransformResult<EncodedPage>> = Vec::new();
		assert!(matches!(
			build_kepub(empty.into_iter(), "T", &mut buffer),
			Err(TransformError::Other(_))
		));
	}

	#[cfg(feature = "kepub")]
	#[test]
	fn kepub_propagates_page_errors() {
		let results: Vec<TransformResult<EncodedPage>> = vec![
			Ok(page(10, 10, PageFormat::Jpeg)),
			Err(TransformError::Decode("boom".to_string())),
		];
		let mut buffer = Cursor::new(Vec::new());
		assert!(matches!(
			build_kepub(results.into_iter(), "T", &mut buffer),
			Err(TransformError::Decode(_))
		));
	}

	#[cfg(not(feature = "kepub"))]
	#[test]
	fn kepub_is_disabled_without_the_feature() {
		let mut buffer = Cursor::new(Vec::new());
		assert!(matches!(
			build_kepub(pages(1), "T", &mut buffer),
			Err(TransformError::FeatureDisabled("KEPUB"))
		));
	}

	#[test]
	fn comic_info_extraction_finds_root_xml() {
		// Build a CBZ-like archive with ComicInfo.xml + pages.
		let mut buffer = Cursor::new(Vec::new());
		{
			let mut zip = ZipWriter::new(&mut buffer);
			let options: FileOptions<()> =
				FileOptions::default().compression_method(CompressionMethod::Deflated);
			zip.start_file("pages/0000.jpg", options).unwrap();
			zip.write_all(b"page").unwrap();
			zip.start_file("ComicInfo.xml", options).unwrap();
			zip.write_all(b"<ComicInfo/>").unwrap();
			zip.finish().unwrap();
		}

		// Write to a temp file for comic_info_xml(path).
		let temp = tempfile::NamedTempFile::new().unwrap();
		std::fs::write(temp.path(), buffer.get_ref()).unwrap();
		let info = comic_info_xml(temp.path()).expect("ComicInfo found");
		assert_eq!(info, b"<ComicInfo/>");
	}

	#[test]
	fn comic_info_extraction_ignores_macos_shadow_copies() {
		let mut buffer = Cursor::new(Vec::new());
		{
			let mut zip = ZipWriter::new(&mut buffer);
			let options: FileOptions<()> = FileOptions::default();
			zip.start_file("__MACOSX/._ComicInfo.xml", options).unwrap();
			zip.write_all(b"resource fork").unwrap();
			zip.start_file("pages/0000.jpg", options).unwrap();
			zip.write_all(b"page").unwrap();
			zip.finish().unwrap();
		}
		let temp = tempfile::NamedTempFile::new().unwrap();
		std::fs::write(temp.path(), buffer.get_ref()).unwrap();
		assert!(comic_info_xml(temp.path()).is_none());
	}
}
