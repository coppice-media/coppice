//! Pure Rust conversion of EPUB packages to Kobo's KEPUB representation.
//!
//! The content transform follows kepubify @9546034.  EPUB entries which are
//! not package/content documents are copied through the ZIP reader unchanged;
//! only the XML/HTML documents selected by the package manifest are rewritten.
//!
//! Design decisions, the parity contract against the pinned kepubify build,
//! the XML 1.1 deviation, cache-key rules, measured throughput, and the
//! `KOBO_KEPUB_*` server switches are documented in `crates/kepub/README.md`.

use std::{
	borrow::Cow,
	collections::{HashMap, HashSet},
	io::{Cursor, Read},
};

use html5ever::{parse_document, tendril::TendrilSink, ParseOpts};
use quick_xml::{events::Event, Reader};
use rayon::prelude::*;

mod archive;
mod dom;
use archive::{ArchiveWriter, Deflated, EntryLocation, EntryMeta};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zip::ZipArchive;

/// The style rule added to every transformed content document.
///
/// This is the rule used by kepubify's `kobostylehacks` style element.  The
/// wrapper rule is deliberately separate from the optional CSS switches so
/// that the default output remains byte-for-byte stable.
pub const KOBO_STYLE_HACKS: &str = "div#book-inner { margin-top: 0; margin-bottom: 0;}";

/// The output suffix used by Stump's KEPUB delivery cache.
pub const KEPUB_EXTENSION: &str = "kepub.epub";

const CSS_HYPHENATE: &str = "* {
    -webkit-hyphens: auto;
    -moz-hyphens: auto;
    hyphens: auto;

    -webkit-hyphenate-limit-after: 3;
    -webkit-hyphenate-limit-before: 3;
    -webkit-hyphenate-limit-lines: 2;
}

h1, h2, h3, h4, h5, h6, td {
    -moz-hyphens: none !important;
    -webkit-hyphens: none !important;
    hyphens: none !important;
}";

const CSS_NO_HYPHENATE: &str = "* {
    -moz-hyphens: none !important;
    -webkit-hyphens: none !important;
    hyphens: none !important;
}";

const CSS_FULLSCREEN_FIXES: &str = "body {
    margin: 0 !important;
    padding: 0 !important;
}

body>div {
    padding-left: 0.2em !important;
    padding-right: 0.2em !important;
}";

/// Byte-affecting controls for the KEPUB transform.
///
/// The first five fields are the Stump-facing controls.  The remaining fields
/// expose the corresponding kepubify converter controls so that callers that
/// need exact corpus fixtures do not need a second converter implementation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransformOptions {
	/// Apply smartypants punctuation substitutions in body text.
	pub smarten_punctuation: bool,
	/// Add kepubify's fullscreen reading CSS fixes.
	pub fullscreen_reading_fixes: bool,
	/// Add hyphenation CSS (`Some(true)`) or disable it (`Some(false)`).
	pub hyphenate: Option<bool>,
	/// Optional CSS value used for a `body { font-size: ... }` rule.
	pub font_size: Option<String>,
	/// Optional CSS value used for a `body { line-height: ... }` rule.
	pub line_height: Option<String>,
	/// Additional raw CSS blocks as `(class, CSS)` pairs.
	pub extra_css: Vec<(String, String)>,
	/// Byte replacements applied to the rendered content document.
	pub find_replace: Vec<(String, String)>,
	/// Optional input charset label.  Empty/UTF-8 is already handled by the
	/// HTML parser; other labels are decoded before the DOM transform.
	pub charset: Option<String>,
	/// Force dummy-title-page insertion. `Some(false)` disables the heuristic.
	pub dummy_titlepage: Option<bool>,
}
/// Options controlling compression of rewritten ZIP entries.
///
/// Compression affects output bytes and must therefore be included in cache
/// keys by callers. It is intentionally separate from [`TransformOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOptions {
	/// Deflate level, clamped to the libdeflate-supported range 1..=12.
	pub deflate_level: u32,
}

impl Default for WriteOptions {
	fn default() -> Self {
		Self {
			deflate_level: DEFLATE_LEVEL,
		}
	}
}

/// Errors returned by EPUB discovery, transformation, or ZIP writing.
#[derive(Debug, Error)]
pub enum KepubError {
	#[error("invalid EPUB: {0}")]
	InvalidEpub(String),
	#[error("I/O error while converting EPUB: {0}")]
	Io(#[from] std::io::Error),
	#[error("ZIP error while converting EPUB: {0}")]
	Zip(#[from] zip::result::ZipError),
	#[error("XML error while reading EPUB package metadata: {0}")]
	Xml(#[from] quick_xml::Error),
	#[error("invalid content document encoding: {0}")]
	Encoding(String),
}

/// Build a deterministic cache filename for a media item and transform.
///
/// The options digest is part of the key.  Changing any byte-affecting option
/// therefore selects a new cache entry rather than serving an older KEPUB.
pub fn cache_file_name(
	media_id: &str,
	file_mtime: u128,
	options: &TransformOptions,
) -> String {
	let digest = options_digest(options);
	format!("{media_id}-{file_mtime}-{digest}.{KEPUB_EXTENSION}")
}

/// Alias for [`cache_file_name`] used by delivery adapters.
pub fn cache_key(media_id: &str, file_mtime: u128, options: &TransformOptions) -> String {
	cache_file_name(media_id, file_mtime, options)
}

fn options_digest(options: &TransformOptions) -> String {
	let mut hasher = Sha256::new();
	hasher.update([u8::from(options.smarten_punctuation)]);
	hasher.update([u8::from(options.fullscreen_reading_fixes)]);
	hash_optional_bool(&mut hasher, options.hyphenate);
	hash_optional_string(&mut hasher, options.font_size.as_deref());
	hash_optional_string(&mut hasher, options.line_height.as_deref());
	hasher.update((options.extra_css.len() as u64).to_le_bytes());
	for (class, css) in &options.extra_css {
		hash_string(&mut hasher, class);
		hash_string(&mut hasher, css);
	}
	hasher.update((options.find_replace.len() as u64).to_le_bytes());
	for (find, replace) in &options.find_replace {
		hash_string(&mut hasher, find);
		hash_string(&mut hasher, replace);
	}
	hash_optional_string(&mut hasher, options.charset.as_deref());
	hash_optional_bool(&mut hasher, options.dummy_titlepage);
	let digest = hasher.finalize();
	digest[..8]
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect()
}

fn hash_optional_bool(hasher: &mut Sha256, value: Option<bool>) {
	match value {
		Some(value) => {
			hasher.update([1, u8::from(value)]);
		},
		None => hasher.update([0]),
	}
}
fn hash_optional_string(hasher: &mut Sha256, value: Option<&str>) {
	match value {
		Some(value) => {
			hasher.update([1]);
			hash_string(hasher, value);
		},
		None => hasher.update([0]),
	}
}

fn hash_string(hasher: &mut Sha256, value: &str) {
	hasher.update((value.len() as u64).to_le_bytes());
	hasher.update(value.as_bytes());
}

/// Convert an EPUB byte slice to a KEPUB byte slice using the default
/// compression level (6).
pub fn transform_epub(
	input: &[u8],
	options: &TransformOptions,
) -> Result<Vec<u8>, KepubError> {
	transform_epub_to(
		input,
		options,
		&WriteOptions::default(),
		Vec::with_capacity(input.len() + input.len() / 8),
	)
}

/// Convert an EPUB byte slice, streaming the KEPUB into `sink` (for example a
/// cache file) so the output is never held in memory as a whole.
///
/// Content documents are transformed and deflated on the rayon pool in
/// bounded batches and written in source order, so peak memory is roughly
/// `2 × threads` documents in flight regardless of the book's size; every
/// other entry is copied from `input` by offset without being inflated.
pub fn transform_epub_to<W: std::io::Write>(
	input: &[u8],
	options: &TransformOptions,
	write_options: &WriteOptions,
	sink: W,
) -> Result<W, KepubError> {
	let deflate_level = write_options.deflate_level.clamp(1, 12);
	let mut source = ZipArchive::new(Cursor::new(input))?;
	let package_path = package_path(&mut source)?;
	let opf = read_entry(&mut source, &package_path)?;
	let package_dir = package_path
		.rsplit_once('/')
		.map(|(dir, _)| dir)
		.unwrap_or("");
	let manifest = parse_manifest(&opf)?;
	let content_paths = content_paths_from_manifest(package_dir, &manifest);
	let dummy = dummy_titlepage_plan(&mut source, package_dir, &opf, &manifest, options)?;
	let transformed_opf = match &dummy {
		Some(dummy) => transform_opf_with_dummy(&opf, dummy)?,
		None => transform_opf(&opf, options)?,
	};

	// Everything the parallel stage needs about a content document, without
	// touching its bytes yet.
	struct Document {
		name: String,
		meta: EntryMeta,
		location: EntryLocation,
	}
	enum Planned {
		Raw(usize),
		Package(String, EntryMeta),
		Content(usize),
	}
	let mut plan = Vec::with_capacity(source.len());
	let mut documents = Vec::new();
	for index in 0..source.len() {
		let file = source.by_index_raw(index)?;
		let key = archive_path_key(file.name());
		if key == "mimetype" || should_filter_path(file.name()) {
			continue;
		}
		if key == archive_path_key(&package_path) {
			plan.push(Planned::Package(
				file.name().to_owned(),
				EntryMeta::from_source(&file),
			));
		} else if content_paths.contains(&key) {
			plan.push(Planned::Content(documents.len()));
			documents.push(Document {
				name: file.name().to_owned(),
				meta: EntryMeta::from_source(&file),
				location: EntryLocation::from_source(&file),
			});
		} else {
			plan.push(Planned::Raw(index));
		}
	}

	let batch = (2 * rayon::current_num_threads()).max(1);
	let mut writer = ArchiveWriter::new(sink);
	writer.add_stored("mimetype", EntryMeta::default(), b"application/epub+zip")?;
	let mut ready: Vec<Deflated> = Vec::new();
	let mut next_batch = 0;
	for planned in plan {
		match planned {
			Planned::Raw(index) => {
				writer.copy_entry(input, &source.by_index_raw(index)?)?
			},
			Planned::Package(name, meta) => {
				writer.add_deflated(&name, meta, &transformed_opf, deflate_level)?;
			},
			Planned::Content(index) => {
				// Documents are planned in source order, so the batch holding
				// `index` is always the next one to compute.
				if index >= next_batch {
					let end = (next_batch + batch).min(documents.len());
					ready = documents[next_batch..end]
						.par_iter()
						.map(|document| {
							let bytes = document.location.read(input, &document.name)?;
							let transformed = transform_content_impl(&bytes, options)?;
							Deflated::compress(
								&document.name,
								document.meta,
								&transformed,
								deflate_level,
							)
						})
						.collect::<Result<_, KepubError>>()?;
					next_batch = end;
				}
				writer.add_precompressed(&ready[index - (next_batch - ready.len())])?;
			},
		}
	}
	if let Some(dummy) = dummy {
		writer.add_deflated(
			&dummy.path,
			EntryMeta::default(),
			DUMMY_TITLEPAGE.as_bytes(),
			deflate_level,
		)?;
	}
	writer.finish()
}

/// Deflate level for rewritten entries; 6 matches kepubify's default.
const DEFLATE_LEVEL: u32 = 6;
/// Convert an EPUB file held in memory using the supplied options.
pub fn convert(input: &[u8], options: &TransformOptions) -> Result<Vec<u8>, KepubError> {
	transform_epub(input, options)
}

/// Apply only kepubify's dummy-title-page fix to an EPUB.
///
/// `None` runs the detector, `Some(true)` forces insertion, and
/// `Some(false)` leaves the package byte-for-byte untouched (including a
/// malformed OPF, matching kepubify's force-disable short circuit).
pub fn transform_dummy_titlepage(
	epub: &[u8],
	force: Option<bool>,
) -> Result<Vec<u8>, KepubError> {
	if force == Some(false) {
		return Ok(epub.to_vec());
	}
	let mut source = ZipArchive::new(Cursor::new(epub))?;
	let package_path = package_path(&mut source)?;
	let opf = read_entry(&mut source, &package_path)?;
	let package_dir = package_path
		.rsplit_once('/')
		.map(|(dir, _)| dir)
		.unwrap_or("");
	let manifest = parse_manifest(&opf)?;
	let options = TransformOptions {
		dummy_titlepage: force,
		..TransformOptions::default()
	};
	let Some(dummy) =
		dummy_titlepage_plan(&mut source, package_dir, &opf, &manifest, &options)?
	else {
		return Ok(epub.to_vec());
	};
	let transformed_opf = transform_opf_with_dummy(&opf, &dummy)?;
	let mut writer =
		ArchiveWriter::new(Vec::with_capacity(epub.len() + DUMMY_TITLEPAGE.len()));
	writer.add_stored("mimetype", EntryMeta::default(), b"application/epub+zip")?;
	for index in 0..source.len() {
		let file = source.by_index(index)?;
		let key = archive_path_key(file.name());
		if key == "mimetype" || should_filter_path(file.name()) {
			continue;
		}
		if key == archive_path_key(&package_path) {
			writer.add_deflated(
				file.name(),
				EntryMeta::from_source(&file),
				&transformed_opf,
				DEFLATE_LEVEL,
			)?;
		} else {
			writer.copy_entry(epub, &file)?;
		}
	}
	writer.add_deflated(
		&dummy.path,
		EntryMeta::default(),
		DUMMY_TITLEPAGE.as_bytes(),
		DEFLATE_LEVEL,
	)?;
	writer.finish()
}

fn archive_path_key(path: &str) -> String {
	path.replace('\\', "/").to_ascii_lowercase()
}

fn should_filter_path(path: &str) -> bool {
	let normalized = path.replace('\\', "/");
	if normalized.ends_with('/') || normalized.is_empty() {
		return true;
	}
	let basename = normalized.rsplit('/').next().unwrap_or_default();
	if normalized
		.rsplit_once('/')
		.map(|(directory, _)| directory)
		.is_some_and(|directory| directory == "__MACOSX")
	{
		return true;
	}
	matches!(
		basename,
		"calibre_bookmarks.txt"
			| "iTunesMetadata.plist"
			| "iTunesArtwork.plist"
			| ".DS_STORE"
			| "thumbs.db"
			| "Thumbs.db"
	)
}

/// Return whether an archive path names an HTML/XHTML content document.
pub fn is_content_file(path: &str) -> bool {
	is_html_path(path)
}

/// Return whether kepubify's file filter removes this archive path.
pub fn should_skip_file(path: &str) -> bool {
	should_filter_path(path)
}

fn content_paths_from_manifest(
	package_dir: &str,
	manifest: &HashMap<String, ManifestItem>,
) -> HashSet<String> {
	manifest
		.values()
		.filter(|item| is_content_item(item))
		.map(|item| archive_path_key(&join_archive_path(package_dir, &item.href)))
		.collect()
}

fn package_path(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<String, KepubError> {
	let container =
		find_archive_entry(archive, "META-INF/container.xml")?.ok_or_else(|| {
			KepubError::InvalidEpub("could not locate META-INF/container.xml".to_string())
		})?;
	let bytes = read_entry(archive, &container)?;
	let mut reader = Reader::from_reader(Cursor::new(bytes));
	reader.config_mut().trim_text(false);
	let mut buf = Vec::new();
	loop {
		match reader.read_event_into(&mut buf)? {
			Event::Start(element) | Event::Empty(element)
				if local_name(element.name().as_ref()) == b"container" =>
			{
				let version = attribute(&element, b"version")?.unwrap_or_default();
				if version != "1.0" {
					return Err(KepubError::InvalidEpub(format!(
						"unsupported container.xml version {version:?}"
					)));
				}
			},
			Event::Start(element) | Event::Empty(element)
				if local_name(element.name().as_ref()) == b"rootfile" =>
			{
				let media_type = attribute(&element, b"media-type")?.unwrap_or_default();
				if media_type != "application/oebps-package+xml" {
					return Err(KepubError::InvalidEpub(format!(
						"unsupported rootfile media-type {media_type:?}"
					)));
				}
				if let Some(path) = attribute(&element, b"full-path")? {
					return Ok(path);
				}
			},
			Event::Eof => break,
			_ => {},
		}
		buf.clear();
	}
	Err(KepubError::InvalidEpub(
		"container.xml has no OEBPS package rootfile".to_string(),
	))
}

fn find_archive_entry(
	archive: &mut ZipArchive<Cursor<&[u8]>>,
	wanted: &str,
) -> Result<Option<String>, KepubError> {
	let wanted = archive_path_key(wanted);
	for index in 0..archive.len() {
		let file = archive.by_index(index)?;
		if archive_path_key(file.name()) == wanted {
			return Ok(Some(file.name().to_owned()));
		}
	}
	Ok(None)
}

fn read_entry(
	archive: &mut ZipArchive<Cursor<&[u8]>>,
	name: &str,
) -> Result<Vec<u8>, KepubError> {
	let mut file = archive.by_name(name)?;
	let mut bytes = Vec::with_capacity(file.size() as usize);
	file.read_to_end(&mut bytes)?;
	Ok(bytes)
}

#[derive(Debug, Clone)]
struct ManifestItem {
	href: String,
	media_type: String,
}

fn parse_manifest(input: &[u8]) -> Result<HashMap<String, ManifestItem>, KepubError> {
	let mut reader = Reader::from_reader(Cursor::new(input));
	reader.config_mut().trim_text(false);
	let mut buf = Vec::new();
	let mut manifest = HashMap::new();
	loop {
		match reader.read_event_into(&mut buf)? {
			Event::Start(element) | Event::Empty(element)
				if local_name(element.name().as_ref()) == b"item" =>
			{
				let Some(id) = attribute(&element, b"id")? else {
					buf.clear();
					continue;
				};
				let Some(href) = attribute(&element, b"href")? else {
					buf.clear();
					continue;
				};
				manifest.insert(
					id,
					ManifestItem {
						href,
						media_type: attribute(&element, b"media-type")?
							.unwrap_or_default(),
					},
				);
			},
			Event::Eof => break,
			_ => {},
		}
		buf.clear();
	}
	Ok(manifest)
}

fn attribute(
	element: &quick_xml::events::BytesStart<'_>,
	wanted: &[u8],
) -> Result<Option<String>, KepubError> {
	for result in element.attributes().with_checks(false) {
		let attr = result.map_err(quick_xml::Error::from)?;
		if local_name(attr.key.as_ref()) == wanted {
			return Ok(Some(attr.unescape_value()?.into_owned()));
		}
	}
	Ok(None)
}

fn local_name(name: &[u8]) -> &[u8] {
	name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn is_content_item(item: &ManifestItem) -> bool {
	let media_type = item.media_type.to_ascii_lowercase();
	media_type == "application/xhtml+xml"
		|| media_type == "text/html"
		|| is_html_path(&item.href)
}

fn is_html_path(path: &str) -> bool {
	let path = path.split(['#', '?']).next().unwrap_or(path);
	matches!(
		path.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()),
		Some(ext) if matches!(ext.as_str(), "xhtml" | "html" | "htm")
	)
}

fn join_archive_path(base: &str, href: &str) -> String {
	let href = href.split(['#', '?']).next().unwrap_or(href);
	let mut parts = Vec::new();
	for part in base
		.split('/')
		.chain(href.trim_start_matches('/').split('/'))
	{
		match part {
			"" | "." => {},
			".." => {
				parts.pop();
			},
			part => parts.push(part),
		}
	}
	parts.join("/")
}

/// Transform one XHTML/HTML content document.
pub fn transform_content(
	input: &[u8],
	options: &TransformOptions,
) -> Result<Vec<u8>, KepubError> {
	transform_content_impl(input, options)
}

/// Transform one XHTML/HTML content document using kepubify-compatible DOM
/// repair, spans, and optional CSS/punctuation controls.
fn transform_content_impl(
	input: &[u8],
	options: &TransformOptions,
) -> Result<Vec<u8>, KepubError> {
	let source = decode_content(input, options.charset.as_deref())?;
	let (xml_declaration, source) = extract_xml_declaration(&source);
	let source = rewrite_lenient_self_closing(&source);
	let mut dom = parse_document(dom::Arena::default(), ParseOpts::default())
		.from_utf8()
		.read_from(&mut Cursor::new(source.as_bytes()))?;
	let document = dom.document();
	dom::normalize_document_whitespace(&mut dom, document);

	let Some(head) = dom::find_element(&dom, document, "head") else {
		return Ok(input.to_vec());
	};
	let Some(body) = dom::find_element(&dom, document, "body") else {
		return Ok(input.to_vec());
	};

	dom::transform_content_charset(&mut dom, head);
	dom::add_style(&mut dom, head, "kobostylehacks", KOBO_STYLE_HACKS);
	for (class, css) in &options.extra_css {
		dom::add_style(&mut dom, head, class, css);
	}

	if let Some(hyphenate) = options.hyphenate {
		dom::add_style(
			&mut dom,
			head,
			"kepubify-hyphenate",
			if hyphenate {
				CSS_HYPHENATE
			} else {
				CSS_NO_HYPHENATE
			},
		);
	}
	if options.fullscreen_reading_fixes {
		dom::add_style(
			&mut dom,
			head,
			"kepubify-fullscreen-fixes",
			CSS_FULLSCREEN_FIXES,
		);
	}
	if options.font_size.is_some() || options.line_height.is_some() {
		let mut css = String::from("body {");
		if let Some(value) = &options.font_size {
			css.push_str("\n    font-size: ");
			css.push_str(value);
			css.push(';');
		}
		if let Some(value) = &options.line_height {
			css.push_str("\n    line-height: ");
			css.push_str(value);
			css.push(';');
		}
		css.push_str("\n}");
		dom::add_style(&mut dom, head, "kepubify-font-metrics", &css);
	}
	dom::transform_divs(&mut dom, body);
	let emit_spans = !dom::has_kobo_span(&dom, body);
	dom::clean_node(&mut dom, document);

	let mut output = String::with_capacity(source.len() + 512);
	if let Some(declaration) = xml_declaration {
		output.push_str(&declaration);
	}
	dom::serialize_node(
		&dom,
		document,
		&mut output,
		false,
		true,
		Some(body),
		emit_spans,
		options.smarten_punctuation,
	);
	for (find, replace) in &options.find_replace {
		if !find.is_empty() {
			output = output.replace(find, replace);
		}
	}
	Ok(output.into_bytes())
}
/// Individual transform stages used by kepubify's table-driven tests and by
/// callers that need to inspect one mutation without running the full pipeline.
pub mod parts {
	/// Rewrite charset metadata to UTF-8.
	pub fn charset_utf8(input: &[u8]) -> Vec<u8> {
		super::transform_content_part(input, super::ContentPart::Charset)
	}

	/// Append the Kobo style-hacks stylesheet.
	pub fn kobo_styles(input: &[u8]) -> Vec<u8> {
		super::transform_content_part(input, super::ContentPart::KoboStyles)
	}

	/// Wrap body children with `book-columns` and `book-inner`.
	pub fn kobo_divs(input: &[u8]) -> Vec<u8> {
		super::transform_content_part(input, super::ContentPart::KoboDivs)
	}

	/// Add Kobo sentence spans.
	pub fn kobo_spans(input: &[u8]) -> Vec<u8> {
		super::transform_content_part(input, super::ContentPart::KoboSpans)
	}

	/// Add an arbitrary style element to the document head.
	pub fn add_style(input: &[u8], class: &str, css: &str) -> Vec<u8> {
		super::transform_content_part(
			input,
			super::ContentPart::AddStyle {
				class: class.to_owned(),
				css: css.to_owned(),
			},
		)
	}

	/// Apply smartypants punctuation substitutions to body text.
	pub fn smarten_punctuation(input: &[u8]) -> Vec<u8> {
		super::transform_content_part(input, super::ContentPart::Punctuation)
	}

	/// Remove known Adobe/Microsoft artifacts and replacement characters.
	pub fn clean_html(input: &[u8]) -> Vec<u8> {
		super::transform_content_part(input, super::ContentPart::Clean)
	}

	/// Apply raw serialized find/replace pairs.
	pub fn replacements(input: &[u8], replacements: &[(String, String)]) -> Vec<u8> {
		super::transform_content_part(
			input,
			super::ContentPart::Replacements(replacements.to_vec()),
		)
	}

	/// Add the OPF cover-image property.
	pub fn opf_cover_image(input: &[u8]) -> Vec<u8> {
		super::transform_opf_part(input, super::OpfPart::CoverImage)
	}

	/// Remove calibre timestamp/backup contributor metadata.
	pub fn opf_calibre_meta(input: &[u8]) -> Vec<u8> {
		super::transform_opf_part(input, super::OpfPart::CalibreMeta)
	}
}

#[derive(Debug)]
enum ContentPart {
	Charset,
	KoboStyles,
	KoboDivs,
	KoboSpans,
	AddStyle { class: String, css: String },
	Punctuation,
	Clean,
	Replacements(Vec<(String, String)>),
}

fn transform_content_part(input: &[u8], part: ContentPart) -> Vec<u8> {
	let Ok(source) = decode_content(input, None) else {
		return input.to_vec();
	};
	let (xml_declaration, source) = extract_xml_declaration(&source);
	let Ok(mut dom) = parse_document(dom::Arena::default(), ParseOpts::default())
		.from_utf8()
		.read_from(&mut Cursor::new(source.as_bytes()))
	else {
		return input.to_vec();
	};
	let document = dom.document();
	dom::normalize_document_whitespace(&mut dom, document);
	let head = dom::find_element(&dom, document, "head");
	let body = dom::find_element(&dom, document, "body");
	let mut span_body = None;
	let mut emit_spans = false;
	let serialize_smarten = false;
	match part {
		ContentPart::Charset => {
			if let Some(head) = head {
				dom::transform_content_charset(&mut dom, head);
			}
		},
		ContentPart::KoboStyles => {
			if let Some(head) = head {
				dom::add_style(&mut dom, head, "kobostylehacks", KOBO_STYLE_HACKS);
			}
		},
		ContentPart::KoboDivs => {
			if let Some(body) = body {
				dom::transform_divs(&mut dom, body);
			}
		},
		ContentPart::KoboSpans => {
			if let Some(body) = body {
				if !dom::has_kobo_span(&dom, body) {
					span_body = Some(body);
					emit_spans = true;
				}
			}
		},
		ContentPart::AddStyle { class, css } => {
			if let Some(head) = head {
				dom::add_style(&mut dom, head, &class, &css);
			}
		},
		ContentPart::Punctuation => {
			if let Some(body) = body {
				dom::smarten_body(&mut dom, body);
			}
		},
		ContentPart::Clean => dom::clean_node(&mut dom, document),
		ContentPart::Replacements(replacements) => {
			let mut output = String::with_capacity(source.len());
			if let Some(declaration) = xml_declaration.as_deref() {
				output.push_str(declaration);
			}
			dom::serialize_node(
				&dom,
				document,
				&mut output,
				false,
				false,
				None,
				false,
				false,
			);
			for (find, replace) in replacements {
				if !find.is_empty() {
					output = output.replace(&find, &replace);
				}
			}
			return output.into_bytes();
		},
	}
	let mut output = String::with_capacity(source.len() + 128);
	if let Some(declaration) = xml_declaration {
		output.push_str(&declaration);
	}
	dom::serialize_node(
		&dom,
		document,
		&mut output,
		false,
		false,
		span_body.or(body),
		emit_spans,
		serialize_smarten,
	);
	output.into_bytes()
}

fn decode_content(input: &[u8], charset: Option<&str>) -> Result<String, KepubError> {
	match charset.map(str::trim).filter(|value| !value.is_empty()) {
		None => Ok(String::from_utf8_lossy(input).into_owned()),
		Some(value)
			if value.eq_ignore_ascii_case("utf-8")
				|| value.eq_ignore_ascii_case("utf8") =>
		{
			Ok(String::from_utf8_lossy(input).into_owned())
		},
		Some(value) => Err(KepubError::Encoding(format!(
			"charset {value:?} is not supported by the UTF-8 parser"
		))),
	}
}

fn extract_xml_declaration(input: &str) -> (Option<String>, String) {
	let trimmed_start = input.trim_start_matches([' ', '\t', '\r', '\n', '\u{feff}']);
	if !trimmed_start.starts_with("<?xml") {
		return (None, input.to_string());
	}
	let Some(end) = trimmed_start.find("?>") else {
		return (None, input.to_string());
	};
	let end = end + 2;
	let declaration = trimmed_start[..end].to_string();
	let mut rest = trimmed_start[end..].to_string();
	while rest.starts_with([' ', '\t', '\r', '\n']) {
		rest.remove(0);
	}
	(Some(declaration), rest)
}

/// The fork used by kepubify accepts self-closing HTML `p`, `div`, `a`,
/// `span`, `title`, and `script` tags by turning them into an open/close pair.
/// html5ever otherwise treats these as ordinary HTML self-closing parse errors.
/// Emulates the fork's `ParseOptionLenientSelfClosing` ahead of html5ever:
/// self-closing `p`/`div`/`a`/`span`/`title`/`script` become an empty
/// start+end pair.  Raw-text elements (`script`, `style`, `textarea`,
/// `title`) are copied verbatim up to their end tag — unless they were the
/// self-closing form, which never enters raw-text mode.
fn rewrite_lenient_self_closing(input: &str) -> Cow<'_, str> {
	let bytes = input.as_bytes();
	let mut output = String::new();
	let mut copied = 0; // input[..copied] is already in `output` (or untouched)
	let mut index = 0;
	while let Some(offset) = memchr(b'<', &bytes[index..]) {
		index += offset;
		if bytes[index..].starts_with(b"<!--") {
			index = match find_bytes(&bytes[index + 4..], b"-->") {
				Some(end) => index + 4 + end + 3,
				None => break,
			};
			continue;
		}
		if bytes[index..].starts_with(b"<![CDATA[") {
			index = match find_bytes(&bytes[index + 9..], b"]]>") {
				Some(end) => index + 9 + end + 3,
				None => break,
			};
			continue;
		}
		let Some(end) = find_tag_end(bytes, index + 1) else {
			break;
		};
		let raw = &input[index..=end];
		if let Some(tag) = self_closing_tag_name(raw) {
			output.push_str(&input[copied..index]);
			output.push('<');
			output.push_str(raw[1..raw.len() - 2].trim_end());
			output.push_str("></");
			output.push_str(tag);
			output.push('>');
			copied = end + 1;
		} else if let Some(tag) = start_tag_name(raw) {
			// Raw text: skip to the matching end tag (case-insensitive).
			let close = ["</", tag].concat();
			index =
				match find_bytes_ignore_ascii_case(&bytes[end + 1..], close.as_bytes()) {
					Some(offset) => end + 1 + offset,
					None => break,
				};
			continue;
		}
		index = end + 1;
	}
	if copied == 0 {
		return Cow::Borrowed(input);
	}
	output.push_str(&input[copied..]);
	Cow::Owned(output)
}

fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
	haystack.iter().position(|byte| *byte == needle)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	haystack
		.windows(needle.len())
		.position(|window| window == needle)
}

fn find_bytes_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	haystack
		.windows(needle.len())
		.position(|window| window.eq_ignore_ascii_case(needle))
}

fn find_tag_end(bytes: &[u8], mut index: usize) -> Option<usize> {
	let mut quote = None;
	while index < bytes.len() {
		match (quote, bytes[index]) {
			(Some(q), byte) if byte == q => quote = None,
			(None, b'\'' | b'"') => quote = Some(bytes[index]),
			(None, b'>') => return Some(index),
			_ => {},
		}
		index += 1;
	}
	None
}

fn self_closing_tag_name(raw: &str) -> Option<&'static str> {
	if !raw.ends_with("/>") || raw.starts_with("</") {
		return None;
	}
	let name = raw[1..]
		.split(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
		.next()?
		.to_ascii_lowercase();
	match name.as_str() {
		"p" => Some("p"),
		"div" => Some("div"),
		"a" => Some("a"),
		"span" => Some("span"),
		"title" => Some("title"),
		"script" => Some("script"),
		_ => None,
	}
}

fn start_tag_name(raw: &str) -> Option<&'static str> {
	if raw.starts_with("</") || raw.starts_with("<!") || raw.starts_with("<?") {
		return None;
	}
	let name = raw[1..]
		.split(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
		.next()?
		.to_ascii_lowercase();
	match name.as_str() {
		"script" => Some("script"),
		"style" => Some("style"),
		"textarea" => Some("textarea"),
		"title" => Some("title"),
		_ => None,
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SentenceClass {
	Space,
	Punct,
	Extra,
	Other,
}

/// Classifies the rune starting at `bytes[i]` and returns its byte width.
///
/// Mirrors Go's `utf8.DecodeRuneInString`: ASCII is classified directly from
/// the byte, multi-byte sequences are decoded leniently, and an invalid
/// sequence yields width 1 and the `Other` class (Go's `InputInvalid`, which
/// the state machine treats like any other rune).
#[inline]
fn classify_sentence_at(bytes: &[u8], i: usize) -> (SentenceClass, usize) {
	let b = bytes[i];
	if b < 0x80 {
		let class = match b {
			b'\t' | b'\n' | 0x0c | b'\r' | b' ' => SentenceClass::Space,
			b'.' | b'!' | b'?' => SentenceClass::Punct,
			b'\'' | b'"' => SentenceClass::Extra,
			_ => SentenceClass::Other,
		};
		return (class, 1);
	}
	let width = match b {
		0xC0..=0xDF => 2,
		0xE0..=0xEF => 3,
		0xF0..=0xF7 => 4,
		_ => return (SentenceClass::Other, 1),
	};
	let Some(seq) = bytes.get(i..i + width) else {
		return (SentenceClass::Other, 1);
	};
	match std::str::from_utf8(seq) {
		Ok(s) => {
			let class = match s {
				"”" | "’" | "“" | "…" => SentenceClass::Extra,
				_ => SentenceClass::Other,
			};
			(class, width)
		},
		Err(_) => (SentenceClass::Other, 1),
	}
}

/// Split text using kepubify's byte-oriented sentence state machine.
pub fn split_sentences(text: &str) -> Vec<&str> {
	#[derive(Clone, Copy)]
	enum State {
		Default,
		AfterPunct,
		AfterPunctExtra,
		AfterSpace,
	}

	let mut state = State::Default;
	let mut start = 0;
	let mut output = Vec::new();
	let bytes = text.as_bytes();
	let mut offset = 0;
	while offset < bytes.len() {
		let (class, width) = classify_sentence_at(bytes, offset);
		let split = match state {
			State::Default => {
				state = match class {
					SentenceClass::Punct => State::AfterPunct,
					SentenceClass::Extra
					| SentenceClass::Space
					| SentenceClass::Other => State::Default,
				};
				false
			},
			State::AfterPunct => {
				state = match class {
					SentenceClass::Punct => State::AfterPunct,
					SentenceClass::Extra => State::AfterPunctExtra,
					SentenceClass::Space => State::AfterSpace,
					SentenceClass::Other => State::Default,
				};
				false
			},
			State::AfterPunctExtra => {
				state = match class {
					SentenceClass::Punct => State::AfterPunct,
					SentenceClass::Extra | SentenceClass::Other => State::Default,
					SentenceClass::Space => State::AfterSpace,
				};
				false
			},
			State::AfterSpace => {
				state = match class {
					SentenceClass::Punct => State::AfterPunct,
					SentenceClass::Extra | SentenceClass::Other => State::Default,
					SentenceClass::Space => State::AfterSpace,
				};
				!matches!(class, SentenceClass::Space)
			},
		};
		if split {
			output.push(&text[start..offset]);
			start = offset;
		}
		offset += width;
	}
	// kepubify's OutputRest: append the remainder when it is non-empty, or when
	// nothing has been emitted at all (so "" splits to [""]).
	let rest = &text[start..];
	if !rest.is_empty() || output.is_empty() {
		output.push(rest);
	}
	output
}

#[derive(Debug, Clone)]
struct DummyPlan {
	path: String,
}

const DUMMY_TITLEPAGE: &str = "<!DOCTYPE html><html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\"><head><title></title></head><body><p style=\"text-align: center; margin: 4em 0; font-size: .7em; font-style: italic;\">Page intentionally left blank by kepubify.</p></body></html>";

fn dummy_titlepage_plan(
	archive: &mut ZipArchive<Cursor<&[u8]>>,
	package_dir: &str,
	opf: &[u8],
	manifest: &HashMap<String, ManifestItem>,
	options: &TransformOptions,
) -> Result<Option<DummyPlan>, KepubError> {
	if options.dummy_titlepage == Some(false) {
		return Ok(None);
	}
	if options.dummy_titlepage == Some(true) {
		return Ok(Some(DummyPlan {
			path: join_archive_path(package_dir, "kepubify-titlepage-dummy.xhtml"),
		}));
	}
	let spine = parse_spine(opf)?;
	let Some(first_id) = spine.first() else {
		return Ok(None);
	};
	let Some(item) = manifest.get(first_id) else {
		return Ok(None);
	};
	// The detector intentionally requires an HTML/XHTML extension.  This is
	// separate from the manifest content-document test: kepubify's detector
	// ignores an item with a non-standard extension even when its media type
	// claims XHTML.
	if !is_html_path(&item.href) {
		return Ok(None);
	}
	let basename = item
		.href
		.rsplit('/')
		.next()
		.unwrap_or(&item.href)
		.to_ascii_lowercase();
	if basename.contains("cover") || basename.contains("title") {
		return Ok(None);
	}
	let path = join_archive_path(package_dir, &item.href);
	let Some(entry) = find_archive_entry(archive, &path)? else {
		return Ok(None);
	};
	let content = read_entry(archive, &entry)?;
	let source = decode_content(&content, None)?;
	let (_, source) = extract_xml_declaration(&source);
	let source = rewrite_lenient_self_closing(&source);
	let Ok(dom) = parse_document(dom::Arena::default(), ParseOpts::default())
		.from_utf8()
		.read_from(&mut Cursor::new(source.as_bytes()))
	else {
		return Ok(None);
	};
	let document = dom.document();
	let Some(body) = dom::find_element(&dom, document, "body") else {
		return Ok(None);
	};
	let plan = || {
		Some(DummyPlan {
			path: join_archive_path(package_dir, "kepubify-titlepage-dummy.xhtml"),
		})
	};
	// Mirrors kepubify's walk: `<p>` is counted but never descended into, so
	// its words do not count; images/svg and raw-text elements are skipped.
	let mut stack = vec![body];
	let mut paragraphs = 0;
	let mut images = 0;
	let mut words = 0;
	while let Some(node) = stack.pop() {
		if let Some(name) = dom::element_name(&dom, node) {
			match name.as_str() {
				"p" => {
					paragraphs += 1;
					if paragraphs > 4 {
						return Ok(plan());
					}
				},
				"img" | "svg" => images += 1,
				"script" | "style" | "pre" | "audio" | "video" | "math" => {},
				_ => {
					stack.extend(dom::children(&dom, node).into_iter().rev());
				},
			}
		} else if let Some(contents) = dom::text_contents(&dom, node) {
			// Go's `len(w) > 3` is a byte length.
			words += contents
				.split_whitespace()
				.filter(|word| word.len() > 3)
				.count();
			if words > 20 {
				return Ok(plan());
			}
		}
	}
	if (images == 0 && words < 5) || images > 4 {
		Ok(plan())
	} else {
		Ok(None)
	}
}

fn parse_spine(input: &[u8]) -> Result<Vec<String>, KepubError> {
	let mut reader = Reader::from_reader(Cursor::new(input));
	reader.config_mut().trim_text(false);
	let mut buf = Vec::new();
	let mut spine = Vec::new();
	loop {
		match reader.read_event_into(&mut buf)? {
			Event::Start(element) | Event::Empty(element)
				if local_name(element.name().as_ref()) == b"itemref" =>
			{
				if let Some(idref) = attribute(&element, b"idref")? {
					if attribute(&element, b"linear")?.as_deref() != Some("no") {
						spine.push(idref);
					}
				}
			},
			Event::Eof => break,
			_ => {},
		}
		buf.clear();
	}
	Ok(spine)
}

#[derive(Debug, Clone)]
enum OpfNode {
	Element {
		name: String,
		attrs: Vec<(String, String)>,
		children: Vec<OpfNode>,
	},
	Text(String),
	Comment(String),
	Declaration(String),
	Doctype(String),
	ProcessingInstruction(String),
}

/// Rewrite OPF metadata and indent it as kepubify does.
pub fn transform_opf(
	input: &[u8],
	_options: &TransformOptions,
) -> Result<Vec<u8>, KepubError> {
	transform_opf_with_dummy(
		input,
		&DummyPlan {
			path: String::new(),
		},
	)
}

fn transform_opf_with_dummy(
	input: &[u8],
	dummy: &DummyPlan,
) -> Result<Vec<u8>, KepubError> {
	let mut roots = parse_opf_tree(input)?;
	let Some(root) = roots.iter_mut().find_map(|node| match node {
		OpfNode::Element { .. } => Some(node),
		_ => None,
	}) else {
		return Err(KepubError::InvalidEpub(
			"OPF has no package element".to_string(),
		));
	};
	let mut cover_id = None;
	walk_opf_mut(root, &mut |node| {
		if let OpfNode::Element { name, attrs, .. } = node {
			if local_name_str(name) == "meta" && attr_str(attrs, "name") == Some("cover")
			{
				cover_id = attr_str(attrs, "content").map(str::to_owned);
			}
		}
	});
	let cover_id = cover_id.unwrap_or_else(|| "cover".to_owned());
	walk_opf_mut(root, &mut |node| {
		if let OpfNode::Element { name, attrs, .. } = node {
			if local_name_str(name) == "item"
				&& attr_str(attrs, "id") == Some(cover_id.as_str())
			{
				set_attr(attrs, "properties", "cover-image");
			}
		}
	});
	remove_opf_nodes(root);
	if !dummy.path.is_empty() {
		add_dummy_opf_nodes(root, dummy);
	}
	let mut output = String::new();
	serialize_opf_nodes(&roots, &mut output, 0, true);
	Ok(output.into_bytes())
}

#[derive(Debug, Clone, Copy)]
enum OpfPart {
	CoverImage,
	CalibreMeta,
}

fn transform_opf_part(input: &[u8], part: OpfPart) -> Vec<u8> {
	let Ok(mut roots) = parse_opf_tree(input) else {
		return input.to_vec();
	};
	let Some(root) = roots.iter_mut().find_map(|node| match node {
		OpfNode::Element { .. } => Some(node),
		_ => None,
	}) else {
		return input.to_vec();
	};
	match part {
		OpfPart::CoverImage => apply_opf_cover_image(root),
		OpfPart::CalibreMeta => remove_opf_nodes(root),
	}
	let mut output = String::new();
	serialize_opf_nodes(&roots, &mut output, 0, true);
	output.into_bytes()
}

fn apply_opf_cover_image(root: &mut OpfNode) {
	let mut cover_id = None;
	walk_opf_mut(root, &mut |node| {
		if let OpfNode::Element { name, attrs, .. } = node {
			if local_name_str(name) == "meta" && attr_str(attrs, "name") == Some("cover")
			{
				cover_id = attr_str(attrs, "content").map(str::to_owned);
			}
		}
	});
	let cover_id = cover_id.unwrap_or_else(|| "cover".to_owned());
	walk_opf_mut(root, &mut |node| {
		if let OpfNode::Element { name, attrs, .. } = node {
			if local_name_str(name) == "item"
				&& attr_str(attrs, "id") == Some(cover_id.as_str())
			{
				set_attr(attrs, "properties", "cover-image");
			}
		}
	});
}

fn parse_opf_tree(input: &[u8]) -> Result<Vec<OpfNode>, KepubError> {
	let mut reader = Reader::from_reader(Cursor::new(input));
	reader.config_mut().trim_text(false);
	let mut buf = Vec::new();
	let mut roots = Vec::new();
	let mut stack: Vec<OpfNode> = Vec::new();
	loop {
		match reader.read_event_into(&mut buf)? {
			Event::Decl(decl) => {
				let text = String::from_utf8_lossy(decl.as_ref()).into_owned();
				push_opf_node(
					&mut roots,
					&mut stack,
					OpfNode::Declaration(format!("<?{text}?>")),
				);
			},
			Event::DocType(doctype) => {
				push_opf_node(
					&mut roots,
					&mut stack,
					OpfNode::Doctype(
						String::from_utf8_lossy(doctype.as_ref()).into_owned(),
					),
				);
			},
			Event::Start(element) => {
				stack.push(OpfNode::Element {
					name: String::from_utf8_lossy(element.name().as_ref()).into_owned(),
					attrs: opf_attributes(&element)?,
					children: Vec::new(),
				});
			},
			Event::Empty(element) => {
				push_opf_node(
					&mut roots,
					&mut stack,
					OpfNode::Element {
						name: String::from_utf8_lossy(element.name().as_ref())
							.into_owned(),
						attrs: opf_attributes(&element)?,
						children: Vec::new(),
					},
				);
			},
			Event::End(_) => {
				let Some(node) = stack.pop() else {
					return Err(KepubError::InvalidEpub(
						"unbalanced OPF element".to_string(),
					));
				};
				push_opf_node(&mut roots, &mut stack, node);
			},
			Event::Text(text) => {
				let decoded = text
					.decode()
					.map_err(|error| KepubError::InvalidEpub(error.to_string()))?;
				let normalized = normalize_xml_line_ends(&decoded);
				let decoded = quick_xml::escape::unescape(&normalized)
					.map_err(|error| KepubError::InvalidEpub(error.to_string()))?;
				push_opf_node(
					&mut roots,
					&mut stack,
					OpfNode::Text(decoded.into_owned()),
				);
			},
			Event::CData(text) => {
				let decoded = text
					.decode()
					.map_err(|error| KepubError::InvalidEpub(error.to_string()))?;
				push_opf_node(
					&mut roots,
					&mut stack,
					OpfNode::Text(normalize_xml_line_ends(&decoded).into_owned()),
				);
			},
			Event::Comment(comment) => push_opf_node(
				&mut roots,
				&mut stack,
				OpfNode::Comment(String::from_utf8_lossy(comment.as_ref()).into_owned()),
			),
			Event::PI(pi) => push_opf_node(
				&mut roots,
				&mut stack,
				OpfNode::ProcessingInstruction(
					String::from_utf8_lossy(pi.as_ref()).into_owned(),
				),
			),
			Event::GeneralRef(reference) => {
				// etree resolves references while parsing and re-escapes on
				// write, so `&lt;` round-trips as `&lt;` rather than `&amp;lt;`.
				let name = String::from_utf8_lossy(reference.as_ref());
				let resolved = quick_xml::escape::unescape(&format!("&{name};"))
					.map(|value| value.into_owned())
					.unwrap_or_else(|_| format!("&{name};"));
				push_opf_node(&mut roots, &mut stack, OpfNode::Text(resolved));
			},
			Event::Eof => break,
		}
		buf.clear();
	}
	if !stack.is_empty() {
		return Err(KepubError::InvalidEpub("unclosed OPF element".to_string()));
	}
	Ok(roots)
}

/// encoding/xml rewrites unescaped `\r\n` and `\r` to `\n` in character data
/// and attribute values before entity expansion; etree inherits that.
fn normalize_xml_line_ends(value: &str) -> Cow<'_, str> {
	if !value.contains('\r') {
		return Cow::Borrowed(value);
	}
	Cow::Owned(value.replace("\r\n", "\n").replace('\r', "\n"))
}

fn opf_attributes(
	element: &quick_xml::events::BytesStart<'_>,
) -> Result<Vec<(String, String)>, KepubError> {
	element
		.attributes()
		.with_checks(false)
		.map(|result| {
			let attr = result.map_err(quick_xml::Error::from)?;
			let raw = String::from_utf8_lossy(attr.value.as_ref());
			let normalized = normalize_xml_line_ends(&raw);
			let value = quick_xml::escape::unescape(&normalized)
				.map_err(|error| KepubError::InvalidEpub(error.to_string()))?;
			Ok((
				String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
				value.into_owned(),
			))
		})
		.collect()
}

fn push_opf_node(roots: &mut Vec<OpfNode>, stack: &mut [OpfNode], node: OpfNode) {
	let siblings = match stack.last_mut() {
		Some(OpfNode::Element { children, .. }) => children,
		_ => roots,
	};
	// quick-xml splits text around entity references; etree holds one
	// character-data node, so merge adjacent text back together.
	if let (OpfNode::Text(value), Some(OpfNode::Text(previous))) =
		(&node, siblings.last_mut())
	{
		previous.push_str(value);
		return;
	}
	siblings.push(node);
}

fn walk_opf_mut(node: &mut OpfNode, callback: &mut impl FnMut(&mut OpfNode)) {
	callback(node);
	if let OpfNode::Element { children, .. } = node {
		for child in children {
			walk_opf_mut(child, callback);
		}
	}
}

fn remove_opf_nodes(node: &mut OpfNode) {
	if let OpfNode::Element { children, .. } = node {
		children.retain(|child| {
			let OpfNode::Element { name, attrs, .. } = child else {
				return true;
			};
			let local = local_name_str(name);
			!(local == "meta" && attr_str(attrs, "name") == Some("calibre:timestamp"))
				&& !(local == "contributor" && attr_str(attrs, "role") == Some("bkp"))
		});
		for child in children {
			remove_opf_nodes(child);
		}
	}
}

fn add_dummy_opf_nodes(root: &mut OpfNode, dummy: &DummyPlan) {
	let OpfNode::Element { children, .. } = root else {
		return;
	};
	let Some(manifest_index) = children.iter().position(|node| {
        matches!(node, OpfNode::Element { name, .. } if local_name_str(name) == "manifest")
    }) else {
        return;
    };
	let Some(spine_index) = children.iter().position(
		|node| matches!(node, OpfNode::Element { name, .. } if local_name_str(name) == "spine"),
	) else {
		return;
	};
	let item = OpfNode::Element {
		name: "item".to_owned(),
		attrs: vec![
			("id".to_owned(), "kepubify-titlepage-dummy".to_owned()),
			(
				"href".to_owned(),
				dummy
					.path
					.rsplit('/')
					.next()
					.unwrap_or(&dummy.path)
					.to_owned(),
			),
			("media-type".to_owned(), "application/xhtml+xml".to_owned()),
		],
		children: Vec::new(),
	};
	if let OpfNode::Element { children, .. } = &mut children[manifest_index] {
		children.push(item);
	}
	if let OpfNode::Element { children, .. } = &mut children[spine_index] {
		// etree's `InsertChildAt(1, …)` lands after the first token (normally
		// the leading whitespace), clamped to append on an empty spine.
		let index = 1.min(children.len());
		children.insert(
			index,
			OpfNode::Element {
				name: "itemref".to_owned(),
				attrs: vec![("idref".to_owned(), "kepubify-titlepage-dummy".to_owned())],
				children: Vec::new(),
			},
		);
	}
}

fn local_name_str(name: &str) -> &str {
	name.rsplit(':').next().unwrap_or(name)
}

fn attr_str<'a>(attrs: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
	attrs
		.iter()
		.find(|(name, _)| local_name_str(name).eq_ignore_ascii_case(wanted))
		.map(|(_, value)| value.as_str())
}

fn set_attr(attrs: &mut Vec<(String, String)>, wanted: &str, value: &str) {
	if let Some((_, current)) = attrs
		.iter_mut()
		.find(|(name, _)| local_name_str(name).eq_ignore_ascii_case(wanted))
	{
		*current = value.to_owned();
	} else {
		attrs.push((wanted.to_owned(), value.to_owned()));
	}
}

fn serialize_opf_nodes(nodes: &[OpfNode], output: &mut String, depth: usize, root: bool) {
	for (index, node) in nodes.iter().enumerate() {
		match node {
			OpfNode::Declaration(value) => {
				output.push_str(value);
				output.push('\n');
			},
			OpfNode::Doctype(value) => {
				indent_opf(output, depth);
				output.push_str("<!DOCTYPE ");
				output.push_str(value);
				output.push('>');
				output.push('\n');
			},
			OpfNode::Comment(value) => {
				indent_opf(output, depth);
				output.push_str("<!--");
				output.push_str(value);
				output.push_str("-->");
				if !matches!(nodes.get(index + 1), Some(OpfNode::Element { .. })) {
					output.push('\n');
				}
			},
			OpfNode::ProcessingInstruction(value) => {
				indent_opf(output, depth);
				output.push_str("<?");
				output.push_str(value);
				output.push_str("?>");
				output.push('\n');
			},
			OpfNode::Text(value) => {
				if !value.trim().is_empty() {
					output.push_str(&escape_xml(value));
				}
			},
			OpfNode::Element {
				name,
				attrs,
				children,
			} => {
				let inline_after_comment =
					index > 0 && matches!(nodes[index - 1], OpfNode::Comment { .. });
				if !inline_after_comment && (!root || !output.is_empty()) {
					indent_opf(output, depth);
				}
				output.push('<');
				output.push_str(name);
				for (attr_name, attr_value) in attrs {
					output.push(' ');
					output.push_str(attr_name);
					output.push_str("=\"");
					output.push_str(&escape_xml(attr_value));
					output.push('"');
				}
				if children.is_empty() {
					output.push_str("/>");
					output.push('\n');
					continue;
				}
				let has_element_child = children.iter().any(|child| {
					matches!(child, OpfNode::Element { .. } | OpfNode::Comment { .. })
				});
				if !has_element_child {
					output.push('>');
					for child in children {
						if let OpfNode::Text(value) = child {
							output.push_str(&escape_xml(value));
						}
					}
					output.push_str("</");
					output.push_str(name);
					output.push('>');
					output.push('\n');
				} else {
					output.push('>');
					output.push('\n');
					serialize_opf_nodes(children, output, depth + 1, false);
					indent_opf(output, depth);
					output.push_str("</");
					output.push_str(name);
					output.push('>');
					output.push('\n');
				}
			},
		}
	}
}

fn indent_opf(output: &mut String, depth: usize) {
	if !output.is_empty() && !output.ends_with('\n') {
		output.push('\n');
	}
	for _ in 0..depth {
		output.push_str("    ");
	}
}

fn escape_xml(value: &str) -> String {
	let mut output = String::with_capacity(value.len());
	for ch in value.chars() {
		match ch {
			'&' => output.push_str("&amp;"),
			'\'' => output.push_str("&apos;"),
			'<' => output.push_str("&lt;"),
			'>' => output.push_str("&gt;"),
			'"' => output.push_str("&quot;"),
			_ => output.push(ch),
		}
	}
	output
}

#[cfg(test)]
mod tests {
	use super::{cache_file_name, split_sentences, TransformOptions};

	#[test]
	fn sentence_split_matches_kepubify_boundaries() {
		assert_eq!(
			split_sentences("One. Two! Three?"),
			["One. ", "Two! ", "Three?"]
		);
		assert_eq!(split_sentences("One.\nTwo"), ["One.\n", "Two"]);
		// Extra punctuation (quotes) never splits on its own: kepubify's state
		// machine only leaves StateDefault on `.`, `!`, or `?`.
		assert_eq!(
			split_sentences("quoted \"text\" here"),
			["quoted \"text\" here"]
		);
		assert_eq!(split_sentences(""), [""]);
	}

	#[test]
	fn cache_key_includes_options_digest() {
		let defaults = TransformOptions::default();
		let mut changed = defaults.clone();
		changed.smarten_punctuation = true;
		let first = cache_file_name("book", 42, &defaults);
		let second = cache_file_name("book", 42, &changed);
		assert_ne!(first, second);
		assert!(first.ends_with(".kepub.epub"));
	}
}
