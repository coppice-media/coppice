//! `epub2cbz`: convert an image-based EPUB (a manga/comic EPUB whose pages are
//! single images) into a CBZ.
//!
//! Behaviour source: Kavita lists `epub2cbz` as "convert an image based EPUB to
//! CBZ" in its external tools guide
//! (<https://wiki.kavitareader.com/guides/external-tools/>). Nothing is copied
//! from that (GPL) tool: pages here are the EPUB's own spine order, resolved
//! with the same spine/resource rule Stump's EPUB processor already uses
//! ([`stump_media::EpubProcessor::structure`]), and the optional
//! `ComicInfo.xml` is generated from the package metadata that
//! [`stump_media::FileProcessor::process_metadata`] parses (OPF + Calibre
//! series fields + sidecar `.opf`).
//!
//! A text EPUB is never converted: it is reported as a warning and skipped, so
//! running the tool over a whole library is safe.

use std::{
	collections::{HashMap, HashSet},
	fs::File,
	io::BufReader,
	path::{Component, Path, PathBuf},
};

use epub::doc::EpubDoc;
use quick_xml::{escape::unescape, events::Event, Reader};
use serde::{Deserialize, Serialize};
use stump_media::{
	ContentType, EpubProcessor, FileProcessor, PathUtils, ProcessedMediaMetadata,
};

use crate::{
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

pub const ID: &str = "epub2cbz";
/// The only action kind this tool plans.
pub const ACTION_CONVERT: &str = "convert";

/// A spine document with more visible text than this is prose, not a comic
/// page. Front matter and colophons routinely carry a sentence or two, so the
/// default tolerates short blurbs while a real prose chapter trips it.
pub const DEFAULT_MAX_TEXT_CHARS: usize = 200;

pub struct Epub2Cbz;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Epub2CbzOptions {
	/// Where CBZs are written. Default: next to each source EPUB.
	pub output_dir: Option<PathBuf>,
	/// Descend into subdirectories when a given path is a directory.
	pub recursive: bool,
	/// Replace an existing target CBZ instead of skipping it.
	pub overwrite: bool,
	/// Write a `ComicInfo.xml` built from the EPUB's package metadata.
	pub comic_info: bool,
	/// Visible-text budget per spine document before it counts as prose.
	pub max_text_chars: usize,
}

impl Default for Epub2CbzOptions {
	fn default() -> Self {
		Self {
			output_dir: None,
			recursive: false,
			overwrite: false,
			comic_info: true,
			max_text_chars: DEFAULT_MAX_TEXT_CHARS,
		}
	}
}

/// Everything `apply` needs to redo a planned conversion without the options
/// blob: a plan is self-contained by contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConvertDetail {
	pages: usize,
	spine_documents: usize,
	prose_documents: usize,
	comic_info: bool,
	overwrite: bool,
	max_text_chars: usize,
}

impl Tool for Epub2Cbz {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Convert image-based EPUBs to CBZ in spine order, with ComicInfo.xml from the EPUB metadata"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<Epub2CbzOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}

		let mut plan = Plan::new(ID);
		let sources = collect_epubs(&input.paths, options.recursive)?;
		if sources.is_empty() {
			plan.warn(
				Warning::new("no-epubs", "no .epub files found in the given paths")
					.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		let mut claimed: HashSet<PathBuf> = HashSet::new();
		for source in sources {
			let mut doc = match open(&source) {
				Ok(doc) => doc,
				Err(error) => {
					plan.warn(
						Warning::new("unreadable", format!("cannot read EPUB: {error}"))
							.at(&source)
							.with_severity(Severity::Error),
					);
					continue;
				},
			};

			let scan = scan_spine(&mut doc, options.max_text_chars);
			if scan.pages.is_empty() {
				plan.warn(
					Warning::new(
						"no-images",
						format!(
							"no page images found in {} spine documents; not an image-based EPUB",
							scan.spine_documents
						),
					)
					.at(&source),
				);
				continue;
			}
			if scan.prose_documents > scan.pages.len() {
				plan.warn(
					Warning::new(
						"not-image-based",
						format!(
							"{} prose documents vs {} image pages; skipped as a text EPUB",
							scan.prose_documents,
							scan.pages.len()
						),
					)
					.at(&source),
				);
				continue;
			}

			let target = target_for(&source, options.output_dir.as_deref());
			if !claimed.insert(target.clone()) {
				plan.warn(
					Warning::new(
						"target-collision",
						format!("another source already writes {}", target.display()),
					)
					.at(&source)
					.with_severity(Severity::Error),
				);
				continue;
			}
			if target.exists() && !options.overwrite {
				plan.warn(
					Warning::new(
						"target-exists",
						format!(
							"{} exists; pass overwrite to replace it",
							target.display()
						),
					)
					.at(&source),
				);
				continue;
			}

			let detail = ConvertDetail {
				pages: scan.pages.len(),
				spine_documents: scan.spine_documents,
				prose_documents: scan.prose_documents,
				comic_info: options.comic_info,
				overwrite: options.overwrite,
				max_text_chars: options.max_text_chars,
			};
			plan.push(
				Action::new(ACTION_CONVERT)
					.with_source(&source)
					.with_target(&target)
					.with_detail(serde_json::to_value(&detail)?),
			);
		}

		Ok(plan)
	}

	fn apply(
		&self,
		plan: &Plan,
		sink: &mut dyn ProgressSink,
	) -> Result<Report, ToolError> {
		plan.expect_tool(ID)?;

		let mut report = Report::for_plan(plan);
		let total = plan.actions.len();
		for (index, action) in plan.actions.iter().enumerate() {
			let (source, target) = match (&action.source, &action.target) {
				(Some(source), Some(target)) => (source, target),
				_ => {
					report.skipped(action.clone(), "action has no source or target");
					continue;
				},
			};
			if action.kind != ACTION_CONVERT {
				report.skipped(
					action.clone(),
					format!("unsupported action kind {:?}", action.kind),
				);
				continue;
			}

			sink.progress(index, total, &format!("converting {}", source.display()));

			let detail =
				match serde_json::from_value::<ConvertDetail>(action.detail.clone()) {
					Ok(detail) => detail,
					Err(error) => {
						report.skipped(
							action.clone(),
							format!("malformed action: {error}"),
						);
						continue;
					},
				};
			if target.exists() && !detail.overwrite {
				report.skipped(action.clone(), "target already exists");
				continue;
			}

			match convert(source, target, &detail) {
				Ok(pages) => {
					if pages != detail.pages {
						report.warn(
							Warning::new(
								"page-count-drift",
								format!(
									"planned {} pages, wrote {pages}; the EPUB changed since planning",
									detail.pages
								),
							)
							.at(source),
						);
					}
					report.applied(action.clone());
				},
				Err(error) => report.skipped(action.clone(), error.to_string()),
			}
		}
		sink.progress(total, total, "done");

		Ok(report)
	}
}

/// One page image, in spine order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PageRef {
	resource_id: String,
	path: PathBuf,
}

#[derive(Debug, Default)]
struct SpineScan {
	pages: Vec<PageRef>,
	/// Spine entries that are documents (not images) — the page candidates.
	spine_documents: usize,
	/// Spine documents that hold no image, or too much text to be a page.
	prose_documents: usize,
}

fn open(path: &Path) -> ToolResult<EpubDoc<BufReader<File>>> {
	Ok(EpubProcessor::open(path_str(path)?)?)
}

fn path_str(path: &Path) -> ToolResult<&str> {
	path.to_str()
		.ok_or_else(|| ToolError::Invalid(format!("path is not valid UTF-8: {path:?}")))
}

/// Walk the spine in reading order and collect the page images.
///
/// Resource paths are resolved exactly as [`EpubProcessor::structure`] does
/// (`resources[idref].path`, which is already the full path inside the
/// container), so an `<img src>` is resolved against its own document.
/// An image referenced twice (a cover repeated as the first page) is written
/// once, keeping the first position.
fn scan_spine(doc: &mut EpubDoc<BufReader<File>>, max_text_chars: usize) -> SpineScan {
	let by_path = doc
		.resources
		.iter()
		.map(|(id, resource)| (resource.path.clone(), id.clone()))
		.collect::<HashMap<PathBuf, String>>();
	let spine = doc
		.spine
		.iter()
		.map(|item| item.idref.clone())
		.collect::<Vec<_>>();

	let mut scan = SpineScan::default();
	let mut seen = HashSet::new();

	for idref in spine {
		let Some(resource) = doc.resources.get(&idref).cloned() else {
			continue;
		};

		// Some publishers put the image itself in the spine.
		if resource.mime.starts_with("image/") {
			if seen.insert(idref.clone()) {
				scan.pages.push(PageRef {
					resource_id: idref,
					path: resource.path,
				});
			}
			continue;
		}

		scan.spine_documents += 1;
		let Some(content) = doc.get_resource_str(&idref).map(|(content, _)| content)
		else {
			scan.prose_documents += 1;
			continue;
		};

		let document = scan_document(&content);
		let mut found = 0;
		for href in document.images {
			let Some(resolved) = resolve_href(&resource.path, &href) else {
				continue;
			};
			let Some(resource_id) = by_path.get(&resolved) else {
				continue;
			};
			found += 1;
			if seen.insert(resource_id.clone()) {
				scan.pages.push(PageRef {
					resource_id: resource_id.clone(),
					path: resolved,
				});
			}
		}

		if found == 0 || document.text_chars > max_text_chars {
			scan.prose_documents += 1;
		}
	}

	scan
}

#[derive(Debug, Default, PartialEq)]
struct DocumentScan {
	/// `img@src` and SVG `image@xlink:href` targets, in document order.
	images: Vec<String>,
	/// Visible text length, ignoring `head`, `style`, `script`, and `title`.
	text_chars: usize,
}

/// Elements whose text is markup or metadata, never a visible page.
const INVISIBLE: [&str; 4] = ["head", "style", "script", "title"];

fn scan_document(content: &str) -> DocumentScan {
	let mut reader = Reader::from_str(content);
	// XHTML in the wild is not always well-formed; a page list must survive a
	// stray unclosed tag instead of failing the whole conversion.
	reader.config_mut().check_end_names = false;

	let mut scan = DocumentScan::default();
	let mut suppressed = 0usize;

	loop {
		match reader.read_event() {
			Ok(Event::Eof) | Err(_) => break,
			Ok(Event::Start(element)) => {
				let name = local_name(element.name().as_ref());
				if INVISIBLE.contains(&name.as_str()) {
					suppressed += 1;
				}
				collect_image(&element, &name, &mut scan);
			},
			// Self-closing: it carries an image reference but opens no span.
			Ok(Event::Empty(element)) => {
				let name = local_name(element.name().as_ref());
				collect_image(&element, &name, &mut scan);
			},
			Ok(Event::End(element)) => {
				let name = local_name(element.name().as_ref());
				if INVISIBLE.contains(&name.as_str()) {
					suppressed = suppressed.saturating_sub(1);
				}
			},
			Ok(Event::Text(text)) => {
				if suppressed == 0 {
					// `BytesText::unescape` is gone in quick-xml 0.38; the
					// repo decodes then unescapes (see
					// `crates/media/src/media/format/epub.rs:794`).
					let raw = String::from_utf8_lossy(text.as_ref());
					let text = unescape(&raw)
						.map(|value| value.into_owned())
						.unwrap_or_else(|_| raw.into_owned());
					scan.text_chars += text.trim().chars().count();
				}
			},
			// quick-xml 0.38 splits every entity out of the surrounding text
			// node, so `A &amp; B` arrives as Text/GeneralRef/Text. Each
			// reference stands for one visible character.
			Ok(Event::GeneralRef(_)) => {
				if suppressed == 0 {
					scan.text_chars += 1;
				}
			},
			_ => {},
		}
	}

	scan
}

fn collect_image(
	element: &quick_xml::events::BytesStart<'_>,
	name: &str,
	scan: &mut DocumentScan,
) {
	let href = match name {
		"img" => attribute(element, &["src"]),
		// SVG-wrapped fixed-layout pages, the common manga EPUB shape.
		"image" => attribute(element, &["xlink:href", "href"]),
		_ => None,
	};
	if let Some(href) = href {
		scan.images.push(href);
	}
}

fn local_name(name: &[u8]) -> String {
	let name = String::from_utf8_lossy(name).to_lowercase();
	match name.split_once(':') {
		// `xlink:href` is an attribute, but `svg:image` is an element: keep the
		// attribute prefix and drop the element one.
		Some((_, local)) => local.to_string(),
		None => name,
	}
}

fn attribute(
	element: &quick_xml::events::BytesStart<'_>,
	names: &[&str],
) -> Option<String> {
	for attribute in element.attributes().flatten() {
		let key = String::from_utf8_lossy(attribute.key.as_ref()).to_lowercase();
		if names.contains(&key.as_str()) {
			let value = attribute.unescape_value().ok()?;
			let value = value.trim();
			if !value.is_empty() {
				return Some(value.to_string());
			}
		}
	}
	None
}

/// Resolve an `href` from `document` into a container path.
///
/// Fragments are dropped, percent escapes are decoded (EPUB hrefs are URLs),
/// and `.`/`..` are folded lexically. Absolute URLs (a remote image) resolve to
/// `None` and are simply not pages.
fn resolve_href(document: &Path, href: &str) -> Option<PathBuf> {
	let href = href.split('#').next().unwrap_or(href);
	if href.is_empty() || href.contains("://") || href.starts_with("data:") {
		return None;
	}

	let decoded = urlencoding::decode(href).ok()?;
	let base = document.parent().unwrap_or(Path::new(""));
	let joined = base.join(decoded.as_ref());

	let mut resolved = PathBuf::new();
	for component in joined.components() {
		match component {
			Component::CurDir => {},
			Component::ParentDir => {
				resolved.pop();
			},
			other => resolved.push(other.as_os_str()),
		}
	}

	(!resolved.as_os_str().is_empty()).then_some(resolved)
}

/// Expand the given paths into EPUB files: a file is taken as-is (whatever its
/// extension), a directory contributes its `.epub` children.
fn collect_epubs(paths: &[PathBuf], recursive: bool) -> ToolResult<Vec<PathBuf>> {
	let mut sources = Vec::new();
	let mut seen = HashSet::new();

	for path in paths {
		if path.is_dir() {
			for file in util::sorted_files(path, recursive)? {
				let is_epub = file
					.extension()
					.is_some_and(|ext| ext.eq_ignore_ascii_case("epub"));
				if is_epub && !file.is_hidden_file() && seen.insert(file.clone()) {
					sources.push(file);
				}
			}
		} else if path.is_file() {
			if seen.insert(path.clone()) {
				sources.push(path.clone());
			}
		} else {
			return Err(ToolError::Invalid(format!(
				"path does not exist: {}",
				path.display()
			)));
		}
	}

	Ok(sources)
}

fn target_for(source: &Path, output_dir: Option<&Path>) -> PathBuf {
	let stem = source
		.file_stem()
		.map(|stem| stem.to_string_lossy().into_owned())
		.unwrap_or_else(|| "untitled".to_string());
	let name = format!("{}.cbz", util::sanitize_file_stem(&stem));
	let parent = output_dir
		.map(Path::to_path_buf)
		.or_else(|| source.parent().map(Path::to_path_buf))
		.unwrap_or_default();

	parent.join(name)
}

/// Extract the pages of `source` into a CBZ at `target`, returning the page
/// count. The EPUB is only read.
fn convert(source: &Path, target: &Path, detail: &ConvertDetail) -> ToolResult<usize> {
	let mut doc = open(source)?;
	let scan = scan_spine(&mut doc, detail.max_text_chars);
	if scan.pages.is_empty() {
		return Err(ToolError::Invalid(format!(
			"no page images in {}",
			source.display()
		)));
	}

	let comic_info = detail
		.comic_info
		.then(|| comic_info_xml(source, scan.pages.len()))
		.transpose()?
		.flatten();

	let pages = scan.pages.iter().enumerate().map(|(index, page)| {
		let (bytes, mime) = doc.get_resource(&page.resource_id).ok_or_else(|| {
			ToolError::Invalid(format!(
				"EPUB resource {:?} vanished while writing",
				page.resource_id
			))
		})?;
		let extension =
			util::page_extension(ContentType::from(mime.as_str()), page.path.to_str());

		Ok((util::page_entry_name(index, &extension), bytes))
	});

	util::write_cbz_file(target, comic_info.as_ref().map(|xml| xml.as_bytes()), pages)
}

/// Build `ComicInfo.xml` from the EPUB's package metadata, or `None` when the
/// EPUB carries nothing worth writing.
fn comic_info_xml(source: &Path, page_count: usize) -> ToolResult<Option<String>> {
	let metadata = <EpubProcessor as FileProcessor>::process_metadata(path_str(source)?)?
		.unwrap_or_default();
	let info = comic_info(metadata, page_count);

	Ok((!info.is_empty()).then(|| info.to_xml()))
}

fn comic_info(metadata: ProcessedMediaMetadata, page_count: usize) -> util::ComicInfo {
	util::ComicInfo {
		title: metadata.title,
		series: metadata.series,
		number: metadata.number.map(format_number),
		volume: metadata.volume,
		summary: metadata.summary,
		year: metadata.year,
		month: metadata.month,
		day: metadata.day,
		writers: metadata.writers.unwrap_or_default(),
		page_count: Some(page_count),
		language: metadata.language,
	}
}

/// `ComicInfo` numbers are text: keep `1` as `1`, not `1.0`.
fn format_number(number: f64) -> String {
	if number.fract() == 0.0 {
		format!("{number:.0}")
	} else {
		let formatted = format!("{number}");
		formatted
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{NoopProgress, Severity};
	use std::io::Write;

	const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-fixture";

	struct Entry(&'static str, Vec<u8>);

	/// A minimal EPUB 2 container: `mimetype`, `META-INF/container.xml`, an OPF
	/// at `OEBPS/content.opf`, plus whatever `extra` entries the test wants.
	fn write_epub(
		path: &Path,
		metadata: &str,
		manifest: &str,
		spine: &str,
		extra: Vec<Entry>,
	) {
		let opf = format!(
			r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="bookid">urn:uuid:fixture</dc:identifier>
{metadata}
  </metadata>
  <manifest>
{manifest}
  </manifest>
  <spine toc="ncx">
{spine}
  </spine>
</package>"#
		);

		let file = File::create(path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		let stored = zip::write::SimpleFileOptions::default()
			.compression_method(zip::CompressionMethod::Stored);
		let deflated = zip::write::SimpleFileOptions::default();

		zip.start_file("mimetype", stored).unwrap();
		zip.write_all(b"application/epub+zip").unwrap();
		zip.start_file("META-INF/container.xml", deflated).unwrap();
		zip.write_all(
			br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
		)
		.unwrap();
		zip.start_file("OEBPS/content.opf", deflated).unwrap();
		zip.write_all(opf.as_bytes()).unwrap();
		for Entry(name, bytes) in extra {
			zip.start_file(name, deflated).unwrap();
			zip.write_all(&bytes).unwrap();
		}
		zip.finish().unwrap();
	}

	fn page_xhtml(image: &str) -> Vec<u8> {
		format!(
			r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>page</title>
<style type="text/css">body {{ margin: 0; padding: 0; text-align: center; background: #ffffff; }}</style>
</head><body><div class="page"><img src="{image}" alt="page"/></div></body></html>"#
		)
		.into_bytes()
	}

	/// A three-page image EPUB with full package metadata.
	fn manga_epub(dir: &Path, name: &str) -> PathBuf {
		let path = dir.join(name);
		let metadata = r#"    <dc:title>Berserk v01</dc:title>
    <dc:creator opf:role="aut">Kentaro Miura</dc:creator>
    <dc:description>The Black Swordsman &amp; friends</dc:description>
    <dc:language>ja</dc:language>
    <dc:date>1990-11-26</dc:date>
    <meta name="calibre:series" content="Berserk"/>
    <meta name="calibre:series_index" content="1"/>"#;
		let mut manifest = String::new();
		let mut spine = String::new();
		let mut extra = Vec::new();
		for index in 1..=3 {
			manifest.push_str(&format!(
				"    <item id=\"p{index}\" href=\"Text/p{index}.xhtml\" media-type=\"application/xhtml+xml\"/>\n    <item id=\"img{index}\" href=\"Images/{index:03}.png\" media-type=\"image/png\"/>\n"
			));
			spine.push_str(&format!("    <itemref idref=\"p{index}\"/>\n"));
			extra.push(Entry(
				match index {
					1 => "OEBPS/Text/p1.xhtml",
					2 => "OEBPS/Text/p2.xhtml",
					_ => "OEBPS/Text/p3.xhtml",
				},
				page_xhtml(&format!("../Images/{index:03}.png")),
			));
			extra.push(Entry(
				match index {
					1 => "OEBPS/Images/001.png",
					2 => "OEBPS/Images/002.png",
					_ => "OEBPS/Images/003.png",
				},
				PNG.to_vec(),
			));
		}
		write_epub(&path, metadata, &manifest, &spine, extra);

		path
	}

	/// An illustrated novel: one image page plus three prose chapters, i.e.
	/// more prose than pages. `images` off makes it a pure text EPUB.
	fn prose_epub(dir: &Path, name: &str, images: bool) -> PathBuf {
		let path = dir.join(name);
		let chapter = format!(
			"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>{}</p></body></html>",
			"Guts walked on. ".repeat(40)
		);

		let mut manifest = String::new();
		let mut spine = String::new();
		let mut extra = Vec::new();
		if images {
			manifest.push_str(
				"    <item id=\"p0\" href=\"Text/p0.xhtml\" media-type=\"application/xhtml+xml\"/>\n    <item id=\"img0\" href=\"Images/plate.png\" media-type=\"image/png\"/>\n",
			);
			spine.push_str("    <itemref idref=\"p0\"/>\n");
			extra.push(Entry(
				"OEBPS/Text/p0.xhtml",
				page_xhtml("../Images/plate.png"),
			));
			extra.push(Entry("OEBPS/Images/plate.png", PNG.to_vec()));
		}
		for index in 1..=3 {
			manifest.push_str(&format!(
				"    <item id=\"c{index}\" href=\"Text/c{index}.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
			));
			spine.push_str(&format!("    <itemref idref=\"c{index}\"/>\n"));
			extra.push(Entry(
				match index {
					1 => "OEBPS/Text/c1.xhtml",
					2 => "OEBPS/Text/c2.xhtml",
					_ => "OEBPS/Text/c3.xhtml",
				},
				chapter.clone().into_bytes(),
			));
		}
		write_epub(
			&path,
			"    <dc:title>A Novel</dc:title>",
			&manifest,
			&spine,
			extra,
		);

		path
	}

	fn entries(path: &Path) -> Vec<String> {
		let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
		(0..archive.len())
			.map(|index| archive.by_index(index).unwrap().name().to_string())
			.collect()
	}

	fn read_entry(path: &Path, name: &str) -> Vec<u8> {
		let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
		let mut entry = archive.by_name(name).unwrap();
		let mut bytes = Vec::new();
		std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
		bytes
	}

	#[test]
	fn plans_and_converts_an_image_epub_with_comic_info() {
		let dir = tempfile::tempdir().unwrap();
		let source = manga_epub(dir.path(), "Berserk v01.epub");

		let plan = Epub2Cbz
			.plan(&ToolInput::new(vec![source.clone()]))
			.unwrap();
		assert_eq!(plan.tool, ID);
		assert_eq!(plan.warnings, vec![]);
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.actions[0].kind, ACTION_CONVERT);
		assert_eq!(plan.actions[0].detail["pages"], 3);
		let target = plan.actions[0].target.clone().unwrap();
		assert_eq!(target, dir.path().join("Berserk v01.cbz"));
		assert!(!target.exists(), "plan must not write anything");

		let report = Epub2Cbz.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(report.applied.len(), 1);
		assert_eq!(report.skipped, vec![]);
		assert!(source.exists(), "the original EPUB must survive");

		assert_eq!(
			entries(&target),
			vec![
				"ComicInfo.xml".to_string(),
				"0001.png".to_string(),
				"0002.png".to_string(),
				"0003.png".to_string(),
			]
		);
		assert_eq!(read_entry(&target, "0002.png"), PNG);

		let xml = String::from_utf8(read_entry(&target, "ComicInfo.xml")).unwrap();
		let parsed: ProcessedMediaMetadata = quick_xml::de::from_str(&xml).unwrap();
		assert_eq!(parsed.title.as_deref(), Some("Berserk v01"));
		assert_eq!(parsed.series.as_deref(), Some("Berserk"));
		assert_eq!(parsed.number, Some(1.0));
		assert_eq!(parsed.writers, Some(vec!["Kentaro Miura".to_string()]));
		assert_eq!(
			parsed.summary.as_deref(),
			Some("The Black Swordsman & friends")
		);
		assert_eq!(parsed.page_count, Some(3));
		assert_eq!(
			(parsed.year, parsed.month, parsed.day),
			(Some(1990), Some(11), Some(26))
		);
		assert!(xml.contains("<LanguageISO>ja</LanguageISO>"), "{xml}");
	}

	#[test]
	fn bulk_over_a_folder_skips_prose_epubs_with_a_warning() {
		let dir = tempfile::tempdir().unwrap();
		let nested = dir.path().join("nested");
		std::fs::create_dir(&nested).unwrap();
		manga_epub(dir.path(), "manga.epub");
		prose_epub(dir.path(), "novel.epub", true);
		prose_epub(dir.path(), "plain.epub", false);
		manga_epub(&nested, "deep.epub");
		std::fs::write(dir.path().join("notes.txt"), b"not an epub").unwrap();

		let plan = Epub2Cbz
			.plan(&ToolInput::new(vec![dir.path().to_path_buf()]))
			.unwrap();
		assert_eq!(
			plan.actions
				.iter()
				.map(|action| action.source.clone().unwrap())
				.collect::<Vec<_>>(),
			vec![dir.path().join("manga.epub")],
			"non-recursive planning stays in the given directory"
		);
		let codes = plan
			.warnings
			.iter()
			.map(|warning| {
				(
					warning.code.as_str(),
					warning.path.clone().unwrap_or_default(),
					warning.severity,
				)
			})
			.collect::<Vec<_>>();
		assert_eq!(
			codes,
			vec![
				// A text EPUB with an illustration plate: more prose than pages.
				(
					"not-image-based",
					dir.path().join("novel.epub"),
					Severity::Warn
				),
				// No images at all.
				("no-images", dir.path().join("plain.epub"), Severity::Warn),
			]
		);

		let plan = Epub2Cbz
			.plan(
				&ToolInput::new(vec![dir.path().to_path_buf()])
					.with_options(serde_json::json!({ "recursive": true })),
			)
			.unwrap();
		assert_eq!(plan.actions.len(), 2, "recursive picks up the nested EPUB");

		let report = Epub2Cbz.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(report.applied.len(), 2);
		assert!(nested.join("deep.cbz").exists());
		assert!(!dir.path().join("novel.cbz").exists());
	}

	#[test]
	fn existing_targets_are_kept_unless_overwrite_is_set() {
		let dir = tempfile::tempdir().unwrap();
		manga_epub(dir.path(), "manga.epub");
		let target = dir.path().join("manga.cbz");
		std::fs::write(&target, b"do not clobber").unwrap();

		let plan = Epub2Cbz
			.plan(&ToolInput::new(vec![dir.path().to_path_buf()]))
			.unwrap();
		assert!(plan.is_empty());
		assert_eq!(plan.warnings[0].code, "target-exists");
		assert_eq!(std::fs::read(&target).unwrap(), b"do not clobber");

		let plan = Epub2Cbz
			.plan(
				&ToolInput::new(vec![dir.path().to_path_buf()])
					.with_options(serde_json::json!({ "overwrite": true })),
			)
			.unwrap();
		assert_eq!(plan.actions.len(), 1);
		Epub2Cbz.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(entries(&target).len(), 4);
	}

	#[test]
	fn options_control_comic_info_and_output_dir() {
		let dir = tempfile::tempdir().unwrap();
		let out = dir.path().join("out");
		let source = manga_epub(dir.path(), "manga.epub");

		let plan = Epub2Cbz
			.plan(
				&ToolInput::new(vec![source]).with_options(serde_json::json!({
					"comic_info": false,
					"output_dir": out,
				})),
			)
			.unwrap();
		Epub2Cbz.apply(&plan, &mut NoopProgress).unwrap();

		let target = out.join("manga.cbz");
		assert_eq!(entries(&target), vec!["0001.png", "0002.png", "0003.png"]);
	}

	#[test]
	fn svg_wrapped_images_and_duplicate_references_are_handled() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("svg.epub");
		let svg_page = br#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 150"><image width="100" height="150" xlink:href="../Images/cover%20page.png"/></svg></body></html>"#;
		write_epub(
			&path,
			"    <dc:title>Cover Only</dc:title>",
			"    <item id=\"p1\" href=\"Text/p1.xhtml\" media-type=\"application/xhtml+xml\"/>\n    <item id=\"p2\" href=\"Text/p2.xhtml\" media-type=\"application/xhtml+xml\"/>\n    <item id=\"img1\" href=\"Images/cover page.png\" media-type=\"image/png\"/>",
			"    <itemref idref=\"p1\"/>\n    <itemref idref=\"p2\"/>",
			vec![
				Entry("OEBPS/Text/p1.xhtml", svg_page.to_vec()),
				Entry("OEBPS/Text/p2.xhtml", svg_page.to_vec()),
				Entry("OEBPS/Images/cover page.png", PNG.to_vec()),
			],
		);

		let plan = Epub2Cbz.plan(&ToolInput::new(vec![path])).unwrap();
		assert_eq!(plan.actions.len(), 1, "{:?}", plan.warnings);
		assert_eq!(
			plan.actions[0].detail["pages"], 1,
			"the same image referenced twice is one page"
		);

		Epub2Cbz.apply(&plan, &mut NoopProgress).unwrap();
		assert_eq!(
			entries(&dir.path().join("svg.cbz")),
			vec!["ComicInfo.xml", "0001.png"]
		);
	}

	#[test]
	fn style_and_script_text_does_not_make_a_page_prose() {
		let scan = scan_document(&String::from_utf8(page_xhtml("../a.png")).unwrap());
		assert_eq!(scan.images, vec!["../a.png".to_string()]);
		assert_eq!(
			scan.text_chars, 0,
			"CSS and <title> must not count as visible text"
		);

		let scan = scan_document("<html><body><p>Hello there</p></body></html>");
		assert_eq!(scan.text_chars, "Hello there".len());
		assert!(scan.images.is_empty());
	}

	#[test]
	fn hrefs_resolve_relative_to_their_document() {
		let doc = Path::new("OEBPS/Text/p1.xhtml");
		assert_eq!(
			resolve_href(doc, "../Images/001.png"),
			Some(PathBuf::from("OEBPS/Images/001.png"))
		);
		assert_eq!(
			resolve_href(doc, "./001.png#anchor"),
			Some(PathBuf::from("OEBPS/Text/001.png"))
		);
		assert_eq!(
			resolve_href(doc, "a%20b.png"),
			Some(PathBuf::from("OEBPS/Text/a b.png"))
		);
		assert_eq!(resolve_href(doc, "https://example.com/x.png"), None);
		assert_eq!(resolve_href(doc, ""), None);
	}

	#[test]
	fn plan_from_another_tool_is_refused_and_bad_paths_error() {
		let error = Epub2Cbz
			.apply(&Plan::new("cbzit"), &mut NoopProgress)
			.unwrap_err();
		assert!(matches!(error, ToolError::PlanMismatch { .. }), "{error}");

		let error = Epub2Cbz.plan(&ToolInput::new(vec![])).unwrap_err();
		assert!(matches!(error, ToolError::Invalid(_)), "{error}");

		let error = Epub2Cbz
			.plan(&ToolInput::new(vec![PathBuf::from("/nope/missing.epub")]))
			.unwrap_err();
		assert!(matches!(error, ToolError::Invalid(_)), "{error}");
	}

	#[test]
	fn comic_info_numbers_drop_the_trailing_zero() {
		assert_eq!(format_number(1.0), "1");
		assert_eq!(format_number(20.1), "20.1");
		assert_eq!(format_number(0.5), "0.5");
	}
}
