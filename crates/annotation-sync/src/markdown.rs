//! Markdown export with a legacy-compatible lane and a safe format-v2 lane.
//!
//! Versionless settings intentionally keep the original UUID/key filenames and
//! byte layout. New connections opt into format 2 explicitly. Format 2 uses a
//! human path template below a per-user directory, structured deterministic YAML
//! frontmatter, source-aware block ids, and an atomic writer guarded by a
//! per-user lock.

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use stump_api_types::settings::{SettingDefinition, SettingKind, SettingValues};
use unicode_normalization::UnicodeNormalization;

use crate::error::AnnotationSyncError;
use crate::model::{
	AnnotationOrigin, BookSource, ExportAnnotation, ExportAnnotationKind, ExportBatch,
	ExportBook, ExportBookmark, ExportReview,
};
use crate::sink::{
	string_setting, Sink, SinkDescriptor, SinkPresetDescriptor, SinkState,
};

pub const MARKDOWN_SINK_ID: &str = "markdown";
pub const FORMAT_VERSION_V2: u32 = 2;
pub const DEFAULT_PRESET_ID: &str = "obsidian";
pub const DEFAULT_PATH_TEMPLATE: &str = "{{author}} - {{title}}/annotations.md";
const MAX_TEMPLATE_BYTES: usize = 16 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_COMPONENT_BYTES: usize = 255;
const LOCK_FILE: &str = ".coppice-annotation.lock";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The `base_url` setting shared by the markdown and git sinks.
pub fn base_url_setting() -> SettingDefinition {
	SettingDefinition {
		key: "base_url",
		label: "Coppice base URL",
		description: "Public URL of this server (e.g. https://stump.example.com). When set, highlight positions link to the book's reader; otherwise they are plain text.",
		kind: SettingKind::String,
		default: Value::Null,
		required: false,
		secret: false,
		help_url: None,
	}
}

fn markdown_settings() -> Vec<SettingDefinition> {
	vec![
		SettingDefinition {
			key: "format_version",
			label: "Export format version",
			description: "Use version 2 for human filenames, structured frontmatter, and safe path templates. Omitted keeps legacy UUID filenames.",
			kind: SettingKind::Int,
			default: serde_json::json!(FORMAT_VERSION_V2),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "preset",
			label: "Export preset",
			description: "A built-in safe layout. Custom templates are constrained to the listed variables and the per-user export root.",
			kind: SettingKind::Enum,
			default: serde_json::json!(DEFAULT_PRESET_ID),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "destination",
			label: "Destination",
			description: "Optional contained relative directory below the per-user annotation root. Absolute paths, traversal, symlinks, and reserved names are rejected.",
			kind: SettingKind::String,
			default: serde_json::json!(""),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "path_template",
			label: "Path template",
			description: "Safe relative template using {{author}}, {{title}}, {{source}}, {{source_id}}, or {{book_id}}; the default is {{author}} - {{title}}/annotations.md.",
			kind: SettingKind::String,
			default: serde_json::json!(DEFAULT_PATH_TEMPLATE),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "body_template",
			label: "Body template",
			description: "Optional constrained template using book fields and {{content}}. It cannot access files, URLs, or arbitrary expressions.",
			kind: SettingKind::String,
			default: Value::Null,
			required: false,
			secret: false,
			help_url: None,
		},
		base_url_setting(),
	]
}

/// Built-in layouts safe to show in a settings editor.
pub fn preset_descriptors() -> Vec<SinkPresetDescriptor> {
	vec![
		SinkPresetDescriptor {
			id: "obsidian",
			name: "Obsidian folders",
			description:
				"One human-named folder per author and title, with annotations.md inside.",
			path_template: DEFAULT_PATH_TEMPLATE,
			body_template: None,
		},
		SinkPresetDescriptor {
			id: "flat",
			name: "Flat notes",
			description:
				"One markdown file per author and title in the destination directory.",
			path_template: "{{author}} - {{title}}.md",
			body_template: None,
		},
		SinkPresetDescriptor {
			id: "author",
			name: "Author folders",
			description:
				"One folder per author with a human-named markdown file for each title.",
			path_template: "{{author}}/{{title}}.md",
			body_template: None,
		},
	]
}

/// Rendering and layout options derived from sink settings.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
	/// Public server URL without a trailing slash; enables reader links.
	pub base_url: Option<String>,
	/// `1` is the legacy byte/path-compatible renderer; `2` is the safe lane.
	pub format_version: u32,
	/// Contained relative destination below a user's export directory.
	pub destination: String,
	/// Preset id retained for UI round-tripping.
	pub preset: String,
	/// Constrained relative path template used by format 2.
	pub path_template: String,
	/// Optional constrained body template used by format 2.
	pub body_template: Option<String>,
}

impl Default for RenderOptions {
	fn default() -> Self {
		Self {
			base_url: None,
			format_version: 1,
			destination: String::new(),
			preset: DEFAULT_PRESET_ID.to_owned(),
			path_template: String::new(),
			body_template: None,
		}
	}
}

impl RenderOptions {
	pub fn from_settings(values: &SettingValues) -> Self {
		let base_url = string_setting(values, "base_url", "");
		let format_version = values
			.get("format_version")
			.and_then(|value| {
				value.as_u64().or_else(|| {
					value.as_str().and_then(|value| value.parse::<u64>().ok())
				})
			})
			.map(|value| value as u32)
			.unwrap_or(1);
		let base_url = base_url.trim_end_matches('/');
		let destination = string_setting(values, "destination", "");
		let preset = string_setting(values, "preset", DEFAULT_PRESET_ID);
		let path_template = values
			.get("path_template")
			.and_then(Value::as_str)
			.filter(|value| !value.is_empty())
			.unwrap_or_else(|| match preset.as_str() {
				"flat" => "{{author}} - {{title}}.md",
				"author" => "{{author}}/{{title}}.md",
				_ => DEFAULT_PATH_TEMPLATE,
			})
			.to_owned();
		let body_template = values
			.get("body_template")
			.and_then(Value::as_str)
			.map(str::to_owned)
			.filter(|value| !value.is_empty());
		Self {
			base_url: (!base_url.is_empty()).then(|| base_url.to_owned()),
			format_version,
			destination,
			preset,
			path_template,
			body_template,
		}
	}

	fn is_v2(&self) -> bool {
		self.format_version >= FORMAT_VERSION_V2
	}
}

/// Renders one book. Invalid custom templates are rejected by
/// [`render_book_checked`] during sink export; this infallible compatibility
/// helper falls back to the default body for callers that used the old API.
pub fn render_book(book: &ExportBook, options: &RenderOptions) -> String {
	render_book_checked(book, options).unwrap_or_else(|_| render_book_v1(book, options))
}

/// Fallible rendering entry point used by sinks after validating settings.
pub fn render_book_checked(
	book: &ExportBook,
	options: &RenderOptions,
) -> Result<String, AnnotationSyncError> {
	if !options.is_v2() {
		return Ok(render_book_v1(book, options));
	}
	validate_templates(options)?;
	let mut out = String::new();
	out.push_str("---\n");
	push_frontmatter_v2(&mut out, book);
	out.push_str("---\n\n");
	let mut body = render_body(book, options, true);
	if let Some(template) = options.body_template.as_deref() {
		body = expand_body_template(template, book, &body)?;
	}
	out.push_str(&body);
	Ok(out)
}

fn render_book_v1(book: &ExportBook, options: &RenderOptions) -> String {
	let mut out = String::new();
	out.push_str("---\n");
	push_frontmatter_v1(&mut out, book);
	out.push_str("---\n\n");
	out.push_str(&render_body(book, options, false));
	out
}

fn push_review_body(out: &mut String, review: &ExportReview) {
	out.push_str("\n## Review\n\n");
	out.push_str(&format!("Rating: {}\n", review.rating));
	out.push_str(&format!("Private: {}\n", review.is_private));
	if let Some(content) = &review.content {
		out.push_str("\n");
		for line in content.lines() {
			out.push_str("> ");
			out.push_str(line);
			out.push('\n');
		}
	}
}

fn render_body(book: &ExportBook, options: &RenderOptions, source_aware: bool) -> String {
	let mut out = String::new();
	out.push_str(&format!("# {}\n", book.title));
	if let Some(review) = &book.review {
		push_review_body(&mut out, review);
	}
	let reader_url = match &book.source {
		BookSource::Native { media_id } => options
			.base_url
			.as_ref()
			.map(|base| format!("{base}/books/{media_id}/epub-reader")),
		BookSource::LiseurWork { .. } => None,
	};
	let highlights: Vec<&ExportAnnotation> = book
		.annotations
		.iter()
		.filter(|annotation| {
			!annotation.deleted && annotation.kind != ExportAnnotationKind::Bookmark
		})
		.collect();
	let bookmark_rows: Vec<&ExportAnnotation> = book
		.annotations
		.iter()
		.filter(|annotation| {
			!annotation.deleted && annotation.kind == ExportAnnotationKind::Bookmark
		})
		.collect();
	if !highlights.is_empty() {
		out.push_str("\n## Highlights\n");
		for annotation in highlights {
			push_annotation(
				&mut out,
				book,
				annotation,
				reader_url.as_deref(),
				source_aware,
			);
		}
	}
	if !book.bookmarks.is_empty() || !bookmark_rows.is_empty() {
		out.push_str("\n## Bookmarks\n");
		for bookmark in &book.bookmarks {
			push_bookmark(
				&mut out,
				book,
				bookmark,
				reader_url.as_deref(),
				source_aware,
			);
		}
		for annotation in bookmark_rows {
			push_annotation(
				&mut out,
				book,
				annotation,
				reader_url.as_deref(),
				source_aware,
			);
		}
	}
	out
}

fn push_frontmatter_v1(out: &mut String, book: &ExportBook) {
	out.push_str(&format!("title: {}\n", quote(&book.title)));
	if !book.authors.is_empty() {
		out.push_str("authors:\n");
		for author in &book.authors {
			out.push_str(&format!("  - {}\n", quote(author)));
		}
	}
	match &book.source {
		BookSource::Native { media_id } => {
			out.push_str("source: \"native\"\n");
			out.push_str(&format!("media_id: {}\n", quote(media_id)));
		},
		BookSource::LiseurWork { work_id } => {
			out.push_str("source: \"liseur\"\n");
			out.push_str(&format!("work_id: {}\n", quote(work_id)));
		},
	}
	if !book.identifiers.is_empty() {
		out.push_str("identifiers:\n");
		for identifier in &book.identifiers {
			out.push_str(&format!(
				"  {}: {}\n",
				yaml_key(&identifier.scheme),
				quote(&identifier.value)
			));
		}
	}
	push_reading_v1(out, book);
	if let Some(review) = &book.review {
		push_review_frontmatter_v1(out, review);
	}
}

fn push_review_frontmatter_v1(out: &mut String, review: &ExportReview) {
	out.push_str(&format!("review_rating: {}\n", review.rating));
	out.push_str(&format!("review_private: {}\n", review.is_private));
	if let Some(content) = &review.content {
		out.push_str(&format!("review: {}\n", quote(content)));
	}
}
fn push_reading_v1(out: &mut String, book: &ExportBook) {
	if let Some(reading) = &book.reading {
		if let Some(progression) = reading.progression {
			out.push_str(&format!("progression: {progression:.4}\n"));
		}
		if let Some(page) = reading.page {
			out.push_str(&format!("page: {page}\n"));
		}
		out.push_str(&format!("completed: {}\n", reading.completed));
		if reading.finished {
			out.push_str("finished: true\n");
		}
		if let Some(last_read_at) = reading.last_read_at {
			out.push_str(&format!("last_read: {}\n", quote(&timestamp(last_read_at))));
		}
		if let Some(protocol) = &reading.source_protocol {
			out.push_str(&format!("last_read_via: {}\n", quote(protocol)));
		}
		out.push_str(&format!("sessions: {}\n", reading.session_count));
		if let Some(total_seconds) = reading.total_seconds {
			out.push_str(&format!("reading_time_seconds: {total_seconds}\n"));
		}
	}
}

fn push_frontmatter_v2(out: &mut String, book: &ExportBook) {
	out.push_str("schema: \"coppice.annotation/v2\"\n");
	out.push_str("book:\n");
	out.push_str(&format!("  key: {}\n", quote(&book.key)));
	out.push_str(&format!("  title: {}\n", quote(&book.title)));
	if book.authors.is_empty() {
		out.push_str("  authors: []\n");
	} else {
		out.push_str("  authors:\n");
		for author in &book.authors {
			out.push_str(&format!("    - {}\n", quote(author)));
		}
	}
	if book.identifiers.is_empty() {
		out.push_str("  identifiers: {}\n");
	} else {
		out.push_str("  identifiers:\n");
		for identifier in &book.identifiers {
			out.push_str(&format!(
				"    {}: {}\n",
				yaml_key(&identifier.scheme),
				quote(&identifier.value)
			));
		}
	}
	push_source_v2(out, book);
	if let Some(updated) = source_updated_at(book) {
		out.push_str(&format!(
			"source_updated_at: {}\n",
			quote(&timestamp(updated))
		));
	} else {
		out.push_str("source_updated_at: null\n");
	}
	if let Some(review) = &book.review {
		push_review_frontmatter_v2(out, review);
	} else {
		out.push_str("review: null\n");
	}
	if let Some(reading) = &book.reading {
		out.push_str("reading:\n");
		if let Some(progression) = reading.progression {
			out.push_str(&format!("  progression: {progression:.4}\n"));
		} else {
			out.push_str("  progression: null\n");
		}
		if let Some(page) = reading.page {
			out.push_str(&format!("  page: {page}\n"));
		} else {
			out.push_str("  page: null\n");
		}
		out.push_str(&format!("  completed: {}\n", reading.completed));
		out.push_str(&format!("  finished: {}\n", reading.finished));
		push_optional_timestamp(out, "last_read_at", reading.last_read_at, 2);
		if let Some(protocol) = &reading.source_protocol {
			out.push_str(&format!("  source_protocol: {}\n", quote(protocol)));
		} else {
			out.push_str("  source_protocol: null\n");
		}
		out.push_str(&format!("  session_count: {}\n", reading.session_count));
		if let Some(seconds) = reading.total_seconds {
			out.push_str(&format!("  total_seconds: {seconds}\n"));
		} else {
			out.push_str("  total_seconds: null\n");
		}
		push_optional_timestamp(out, "last_session_at", reading.last_session_at, 2);
	} else {
		out.push_str("reading: null\n");
	}
	if book.annotations.is_empty() {
		out.push_str("annotations: []\n");
	} else {
		out.push_str("annotations:\n");
		for annotation in &book.annotations {
			push_annotation_frontmatter(out, book, annotation);
		}
	}
	if book.bookmarks.is_empty() {
		out.push_str("bookmarks: []\n");
	} else {
		out.push_str("bookmarks:\n");
		for bookmark in &book.bookmarks {
			push_bookmark_frontmatter(out, book, bookmark);
		}
	}
}

fn push_review_frontmatter_v2(out: &mut String, review: &ExportReview) {
	out.push_str("review:\n");
	out.push_str(&format!("  rating: {}\n", review.rating));
	out.push_str(&format!("  private: {}\n", review.is_private));
	if let Some(content) = &review.content {
		out.push_str(&format!("  content: {}\n", quote(content)));
	} else {
		out.push_str("  content: null\n");
	}
	out.push_str(&format!(
		"  created_at: {}\n",
		quote(&timestamp(review.created_at))
	));
	out.push_str(&format!(
		"  updated_at: {}\n",
		quote(&timestamp(review.updated_at))
	));
	let scope = if review.work_id.is_some() {
		"work"
	} else {
		"media"
	};
	out.push_str(&format!("  scope: {}\n", quote(scope)));
}

fn push_source_v2(out: &mut String, book: &ExportBook) {
	out.push_str("source:\n");
	match &book.source {
		BookSource::Native { media_id } => {
			out.push_str("  kind: \"native\"\n");
			out.push_str(&format!("  id: {}\n", quote(media_id)));
			out.push_str(&format!("  media_id: {}\n", quote(media_id)));
		},
		BookSource::LiseurWork { work_id } => {
			out.push_str("  kind: \"liseur\"\n");
			out.push_str(&format!("  id: {}\n", quote(work_id)));
			out.push_str(&format!("  work_id: {}\n", quote(work_id)));
		},
	}
	out.push_str(&format!("  key: {}\n", quote(&book.key)));
}

fn push_annotation_frontmatter(
	out: &mut String,
	book: &ExportBook,
	annotation: &ExportAnnotation,
) {
	out.push_str("  - id: ");
	out.push_str(&quote(&annotation.id));
	out.push('\n');
	out.push_str(&format!(
		"    kind: {}\n",
		quote(annotation_kind(annotation.kind))
	));
	out.push_str(&format!("    source: {}\n", quote(source_kind(book))));
	out.push_str(&format!("    source_id: {}\n", quote(source_id(book))));
	out.push_str("    provenance:\n");
	match &annotation.origin {
		AnnotationOrigin::Native => {
			out.push_str("      kind: \"native\"\n");
			out.push_str(&format!("      source_id: {}\n", quote(source_id(book))));
		},
		AnnotationOrigin::Liseur { rev, seq } => {
			out.push_str("      kind: \"liseur\"\n");
			out.push_str(&format!("      source_id: {}\n", quote(source_id(book))));
			out.push_str(&format!("      rev: {rev}\n      seq: {seq}\n"));
		},
	}
	if let Some(locator) = &annotation.locator {
		out.push_str("    locator: ");
		out.push_str(
			&serde_json::to_string(locator).unwrap_or_else(|_| "null".to_owned()),
		);
		out.push('\n');
	} else {
		out.push_str("    locator: null\n");
	}
	if let Some(progression) = annotation.progression {
		out.push_str(&format!("    progression: {progression:.4}\n"));
	} else {
		out.push_str("    progression: null\n");
	}
	push_optional_string(out, "color", annotation.color.as_deref(), 4);
	push_optional_string(out, "excerpt", annotation.excerpt.as_deref(), 4);
	push_optional_string(out, "note", annotation.note.as_deref(), 4);
	push_optional_timestamp(out, "created_at", annotation.created_at, 4);
	push_optional_timestamp(out, "updated_at", annotation.updated_at, 4);
	out.push_str(&format!("    deleted: {}\n", annotation.deleted));
	push_optional_timestamp(out, "deleted_at", annotation.deleted_at, 4);
}

fn push_bookmark_frontmatter(
	out: &mut String,
	book: &ExportBook,
	bookmark: &ExportBookmark,
) {
	out.push_str("  - id: ");
	out.push_str(&quote(&bookmark.id));
	out.push('\n');
	out.push_str("    kind: \"bookmark\"\n");
	out.push_str(&format!("    source: {}\n", quote(source_kind(book))));
	out.push_str(&format!("    source_id: {}\n", quote(source_id(book))));
	out.push_str("    provenance:\n      kind: \"native\"\n");
	out.push_str(&format!("      source_id: {}\n", quote(source_id(book))));
	if let Some(locator) = &bookmark.locator {
		out.push_str("    locator: ");
		out.push_str(
			&serde_json::to_string(locator).unwrap_or_else(|_| "null".to_owned()),
		);
		out.push('\n');
	} else {
		out.push_str("    locator: null\n");
	}
	push_optional_string(
		out,
		"preview_content",
		bookmark.preview_content.as_deref(),
		4,
	);
	if let Some(page) = bookmark.page {
		out.push_str(&format!("    page: {page}\n"));
	} else {
		out.push_str("    page: null\n");
	}
	out.push_str(&format!(
		"    created_at: {}\n",
		quote(&timestamp(bookmark.created_at))
	));
}

fn push_optional_string(out: &mut String, key: &str, value: Option<&str>, indent: usize) {
	let prefix = " ".repeat(indent);
	match value {
		Some(value) => out.push_str(&format!("{prefix}{key}: {}\n", quote(value))),
		None => out.push_str(&format!("{prefix}{key}: null\n")),
	}
}

fn push_optional_timestamp(
	out: &mut String,
	key: &str,
	value: Option<DateTime<Utc>>,
	indent: usize,
) {
	let value = value.map(timestamp);
	push_optional_string(out, key, value.as_deref(), indent);
}

fn source_kind(book: &ExportBook) -> &'static str {
	match &book.source {
		BookSource::Native { .. } => "native",
		BookSource::LiseurWork { .. } => "liseur",
	}
}

fn source_id(book: &ExportBook) -> &str {
	match &book.source {
		BookSource::Native { media_id } => media_id,
		BookSource::LiseurWork { work_id } => work_id,
	}
}

fn annotation_kind(kind: ExportAnnotationKind) -> &'static str {
	match kind {
		ExportAnnotationKind::Highlight => "highlight",
		ExportAnnotationKind::Note => "note",
		ExportAnnotationKind::Bookmark => "bookmark",
	}
}

fn source_updated_at(book: &ExportBook) -> Option<DateTime<Utc>> {
	book.annotations
		.iter()
		.flat_map(|annotation| {
			[
				annotation.created_at,
				annotation.updated_at,
				annotation.deleted_at,
			]
			.into_iter()
			.flatten()
		})
		.chain(book.bookmarks.iter().map(|bookmark| bookmark.created_at))
		.chain(book.reading.as_ref().into_iter().flat_map(|reading| {
			[reading.last_read_at, reading.last_session_at]
				.into_iter()
				.flatten()
		}))
		.max()
}

/// One list item in the legacy and v2 body. A note without an excerpt renders
/// its text as the item body.
fn push_annotation(
	out: &mut String,
	book: &ExportBook,
	annotation: &ExportAnnotation,
	reader_url: Option<&str>,
	source_aware: bool,
) {
	out.push('\n');
	let excerpt = annotation
		.excerpt
		.as_deref()
		.filter(|excerpt| !excerpt.is_empty());
	let note = annotation.note.as_deref().filter(|note| !note.is_empty());
	let mut first = true;
	if let Some(excerpt) = excerpt {
		push_quote(out, excerpt);
		first = false;
	}
	if let Some(note) = note {
		if !first {
			out.push('\n');
		}
		push_paragraph(out, note, first);
		first = false;
	}
	let mut meta = Vec::new();
	if let Some(position) = annotation.locator.as_ref().and_then(position_label) {
		meta.push(link(&position, reader_url));
	}
	if let Some(progression) = annotation.progression {
		meta.push(percent(progression));
	}
	if let Some(color) = annotation
		.color
		.as_deref()
		.filter(|color| !color.is_empty())
	{
		meta.push(color.to_owned());
	}
	if let Some(created) = annotation.created_at {
		meta.push(timestamp(created));
	}
	let id = if source_aware {
		block_id_v2(book, &annotation.id)
	} else {
		block_id_legacy(&annotation.id)
	};
	push_meta(out, &meta, &id, first);
}

fn push_bookmark(
	out: &mut String,
	book: &ExportBook,
	bookmark: &ExportBookmark,
	reader_url: Option<&str>,
	source_aware: bool,
) {
	out.push('\n');
	let preview = bookmark
		.preview_content
		.as_deref()
		.filter(|preview| !preview.is_empty());
	let mut first = true;
	if let Some(preview) = preview {
		push_quote(out, preview);
		first = false;
	}
	let mut meta = Vec::new();
	let position = bookmark.locator.as_ref().and_then(position_label);
	let position = match (position, bookmark.page) {
		(Some(position), Some(page)) => Some(format!("{position} · page {page}")),
		(Some(position), None) => Some(position),
		(None, Some(page)) => Some(format!("page {page}")),
		(None, None) => None,
	};
	if let Some(position) = position {
		meta.push(link(&position, reader_url));
	}
	meta.push(timestamp(bookmark.created_at));
	let id = if source_aware {
		block_id_v2(book, &bookmark.id)
	} else {
		block_id_legacy(&bookmark.id)
	};
	push_meta(out, &meta, &id, first);
}

fn push_quote(out: &mut String, text: &str) {
	for (index, line) in text.lines().enumerate() {
		let prefix = if index == 0 { "- " } else { "  " };
		if line.is_empty() {
			out.push_str(&format!("{prefix}>\n"));
		} else {
			out.push_str(&format!("{prefix}> {line}\n"));
		}
	}
}

fn push_paragraph(out: &mut String, text: &str, first: bool) {
	for (index, line) in text.lines().enumerate() {
		let prefix = if first && index == 0 { "- " } else { "  " };
		out.push_str(&format!("{prefix}{line}\n"));
	}
}

fn push_meta(out: &mut String, meta: &[String], id: &str, first: bool) {
	let prefix = if first { "- " } else { "\n  " };
	out.push_str(&format!("{prefix}↗ {} ^{}\n", meta.join(" · "), id));
}

fn link(text: &str, url: Option<&str>) -> String {
	match url {
		Some(url) => format!("[{text}]({url})"),
		None => text.to_owned(),
	}
}

fn position_label(locator: &Value) -> Option<String> {
	if let Some(href) = locator.get("href").and_then(Value::as_str) {
		return Some(href.to_owned());
	}
	if let Some(cfi) = locator.get("cfi").and_then(Value::as_str) {
		return Some(cfi.to_owned());
	}
	locator
		.get("locations")
		.and_then(|locations| locations.get("position"))
		.or_else(|| locator.get("position"))
		.and_then(Value::as_i64)
		.map(|position| format!("position {position}"))
}

fn percent(progression: f64) -> String {
	format!("{:.2}%", progression * 100.0)
}

fn block_id_legacy(id: &str) -> String {
	id.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || c == '-' {
				c
			} else {
				'-'
			}
		})
		.collect()
}

/// Source-aware ids prevent identical row ids from different backends linking
/// to the same block.
fn block_id_v2(book: &ExportBook, id: &str) -> String {
	let source = block_id_legacy(&book.key);
	let row = block_id_legacy(id);
	format!("{source}-{row}")
}

fn quote(value: &str) -> String {
	serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn yaml_key(key: &str) -> String {
	let plain = !key.is_empty()
		&& key
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
	if plain {
		key.to_owned()
	} else {
		quote(key)
	}
}

fn timestamp(value: DateTime<Utc>) -> String {
	value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The legacy file name for one book; retained for old versionless rows.
pub fn file_name(book: &ExportBook) -> String {
	format!("{}.md", book.key.replace(':', "-"))
}

const PATH_VARIABLES: &[&str] = &["author", "title", "source", "source_id", "book_id"];
const BODY_VARIABLES: &[&str] = &[
	"title",
	"author",
	"authors",
	"source",
	"source_id",
	"book_id",
	"key",
	"schema",
	"content",
];

/// Validates a contained relative destination. This never canonicalizes or
/// follows a path, so callers can validate before creating missing segments.
pub fn validate_relative_destination(
	destination: &str,
) -> Result<PathBuf, AnnotationSyncError> {
	if destination.len() > MAX_PATH_BYTES {
		return Err(AnnotationSyncError::sink("destination is too long"));
	}
	if destination.is_empty() {
		return Ok(PathBuf::new());
	}
	if destination.contains('\\') || destination.contains('\0') {
		return Err(AnnotationSyncError::sink(
			"destination contains an invalid character",
		));
	}
	let normalized = destination.nfc().collect::<String>();
	let path = Path::new(&normalized);
	if path.is_absolute() {
		return Err(AnnotationSyncError::sink("destination must be relative"));
	}
	let mut clean = PathBuf::new();
	for component in path.components() {
		let Component::Normal(value) = component else {
			return Err(AnnotationSyncError::sink(
				"destination may not contain root, current, or parent components",
			));
		};
		let value = value.to_string_lossy();
		validate_component(&value)?;
		clean.push(value.as_ref());
	}
	if clean.as_os_str().is_empty() {
		return Ok(PathBuf::new());
	}
	Ok(clean)
}

/// Normalizes a book/user component without ever allowing it to become a path
/// separator. User-authored title/author values are sanitized; destinations
/// and templates are rejected instead of sanitized.
pub fn normalize_component(value: &str) -> Result<String, AnnotationSyncError> {
	let normalized = value.nfc().collect::<String>();
	let mut output = String::with_capacity(normalized.len());
	let mut whitespace = false;
	for character in normalized.chars() {
		if character.is_control() || character == '/' || character == '\\' {
			output.push('-');
			whitespace = false;
		} else if character.is_whitespace() {
			if !output.is_empty() {
				whitespace = true;
			}
		} else {
			if whitespace {
				output.push(' ');
			}
			whitespace = false;
			output.push(character);
		}
	}
	let output = output
		.trim_matches(|character: char| character == '.' || character.is_whitespace());
	let mut output = output.to_owned();
	if output.is_empty() || output == "." || output == ".." {
		output = "untitled".to_owned();
	}
	if is_reserved_component(&output) {
		output.insert(0, '-');
	}
	if output.as_bytes().len() > MAX_COMPONENT_BYTES {
		return Err(AnnotationSyncError::sink(
			"generated path component is too long",
		));
	}
	Ok(output)
}

fn validate_component(value: &str) -> Result<(), AnnotationSyncError> {
	if value.is_empty() || value == "." || value == ".." {
		return Err(AnnotationSyncError::sink(
			"path contains an empty or traversal component",
		));
	}
	if value.as_bytes().len() > MAX_COMPONENT_BYTES {
		return Err(AnnotationSyncError::sink("path component is too long"));
	}
	if value.chars().any(|character| character.is_control())
		|| is_reserved_component(value)
	{
		return Err(AnnotationSyncError::sink(
			"path contains a reserved component",
		));
	}
	Ok(())
}

fn is_reserved_component(value: &str) -> bool {
	let stem = value.split('.').next().unwrap_or(value);
	let lower = stem.to_ascii_lowercase();
	matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
		|| (lower.len() == 4
			&& (lower.starts_with("com") || lower.starts_with("lpt"))
			&& lower.as_bytes()[3].is_ascii_digit())
}

fn validate_template(
	template: &str,
	allowed: &[&str],
	label: &str,
) -> Result<(), AnnotationSyncError> {
	if template.len() > MAX_TEMPLATE_BYTES {
		return Err(AnnotationSyncError::sink(format!(
			"{label} template is too long"
		)));
	}
	let mut rest = template;
	while let Some(start) = rest.find("{{") {
		let after = &rest[start + 2..];
		let Some(end) = after.find("}}") else {
			return Err(AnnotationSyncError::sink(format!(
				"{label} template has an unclosed variable"
			)));
		};
		let variable = after[..end].trim();
		if variable.is_empty() || !allowed.contains(&variable) {
			return Err(AnnotationSyncError::sink(format!(
				"{label} template variable `{variable}` is not allowed"
			)));
		}
		rest = &after[end + 2..];
	}
	if rest.contains("}}") || template.contains("{ ") || template.contains(" }") {
		return Err(AnnotationSyncError::sink(format!(
			"{label} template has malformed braces"
		)));
	}
	Ok(())
}

pub fn validate_path_template(template: &str) -> Result<(), AnnotationSyncError> {
	if template.trim().is_empty() {
		return Err(AnnotationSyncError::sink("path template must not be empty"));
	}
	validate_template(template, PATH_VARIABLES, "path")?;
	if template.contains('\\') || template.as_bytes().contains(&0) {
		return Err(AnnotationSyncError::sink(
			"path template contains an invalid character",
		));
	}
	if Path::new(template).components().any(|component| {
		matches!(
			component,
			Component::Prefix(_) | Component::RootDir | Component::ParentDir
		)
	}) {
		return Err(AnnotationSyncError::sink(
			"path template must stay relative to the export destination",
		));
	}
	Ok(())
}

pub fn validate_body_template(template: &str) -> Result<(), AnnotationSyncError> {
	validate_template(template, BODY_VARIABLES, "body")
}
/// Resolves a user directory without permitting a user id to escape the
/// configured export root.
pub(crate) fn user_dir_for_root(
	root: &Path,
	user_id: &str,
) -> Result<PathBuf, AnnotationSyncError> {
	if user_id.is_empty() || user_id.contains('/') || user_id.contains('\\') {
		return Err(AnnotationSyncError::sink(
			"user id must be one safe path component",
		));
	}
	validate_component(user_id)?;
	Ok(root.join(user_id))
}
fn validate_templates(options: &RenderOptions) -> Result<(), AnnotationSyncError> {
	validate_relative_destination(&options.destination)?;
	let path_template = if options.path_template.is_empty() {
		DEFAULT_PATH_TEMPLATE
	} else {
		&options.path_template
	};
	validate_path_template(path_template)?;
	if let Some(template) = &options.body_template {
		validate_body_template(template)?;
	}
	Ok(())
}

fn template_value<'a>(name: &str, book: &'a ExportBook) -> String {
	match name {
		"title" => {
			normalize_component(&book.title).unwrap_or_else(|_| "untitled".to_owned())
		},
		"author" => book
			.authors
			.first()
			.map(|author| {
				normalize_component(author)
					.unwrap_or_else(|_| "unknown-author".to_owned())
			})
			.unwrap_or_else(|| "unknown-author".to_owned()),
		"authors" => book
			.authors
			.iter()
			.map(|author| {
				normalize_component(author)
					.unwrap_or_else(|_| "unknown-author".to_owned())
			})
			.collect::<Vec<_>>()
			.join(", "),
		"source" => source_kind(book).to_owned(),
		"source_id" | "book_id" => normalize_component(source_id(book))
			.unwrap_or_else(|_| "unknown-book".to_owned()),
		"key" => {
			normalize_component(&book.key).unwrap_or_else(|_| "unknown-book".to_owned())
		},
		"schema" => "coppice.annotation/v2".to_owned(),
		_ => String::new(),
	}
}

fn expand_path_template(
	template: &str,
	book: &ExportBook,
) -> Result<PathBuf, AnnotationSyncError> {
	validate_path_template(template)?;
	let mut expanded = String::new();
	let mut rest = template;
	while let Some(start) = rest.find("{{") {
		expanded.push_str(&rest[..start]);
		let after = &rest[start + 2..];
		let end = after.find("}}").ok_or_else(|| {
			AnnotationSyncError::sink("path template has an unclosed variable")
		})?;
		expanded.push_str(&template_value(after[..end].trim(), book));
		rest = &after[end + 2..];
	}
	expanded.push_str(rest);
	if expanded.len() > MAX_PATH_BYTES {
		return Err(AnnotationSyncError::sink("expanded path is too long"));
	}
	validate_relative_destination(&expanded)
}

fn expand_body_template(
	template: &str,
	book: &ExportBook,
	content: &str,
) -> Result<String, AnnotationSyncError> {
	validate_body_template(template)?;
	let mut output = String::with_capacity(template.len() + content.len());
	let mut rest = template;
	while let Some(start) = rest.find("{{") {
		output.push_str(&rest[..start]);
		let after = &rest[start + 2..];
		let end = after.find("}}").ok_or_else(|| {
			AnnotationSyncError::sink("body template has an unclosed variable")
		})?;
		let variable = after[..end].trim();
		if variable == "content" {
			output.push_str(content);
		} else {
			output.push_str(&template_value(variable, book));
		}
		rest = &after[end + 2..];
	}
	output.push_str(rest);
	if output.len() > MAX_TEMPLATE_BYTES + content.len() {
		return Err(AnnotationSyncError::sink("expanded body is too long"));
	}
	Ok(output)
}

/// Resolves and validates the format-v2 path relative to a user's directory.
pub fn path_for_book(
	book: &ExportBook,
	options: &RenderOptions,
) -> Result<PathBuf, AnnotationSyncError> {
	if !options.is_v2() {
		return Ok(PathBuf::from(file_name(book)));
	}
	validate_templates(options)?;
	let destination = validate_relative_destination(&options.destination)?;
	let template = if options.path_template.is_empty() {
		DEFAULT_PATH_TEMPLATE
	} else {
		&options.path_template
	};
	let rendered = expand_path_template(template, book)?;
	let mut combined = destination;
	combined.push(rendered);
	let as_string = combined.to_string_lossy().to_string();
	validate_relative_destination(&as_string)
}

fn collision_path(path: &Path, index: usize, folder_suffix: bool) -> PathBuf {
	let suffix = format!(" ({index})");
	if folder_suffix {
		if let Some(parent) = path
			.parent()
			.filter(|parent| !parent.as_os_str().is_empty())
		{
			let name = parent
				.file_name()
				.and_then(|name| name.to_str())
				.unwrap_or("book");
			let file_name = path
				.file_name()
				.unwrap_or_else(|| OsStr::new("annotations.md"));
			let mut renamed = parent.to_path_buf();
			renamed.set_file_name(format!("{name}{suffix}"));
			return renamed.join(file_name);
		}
	}
	let stem = path
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or("annotations");
	let extension = path
		.extension()
		.and_then(|extension| extension.to_str())
		.unwrap_or("md");
	let file_name = format!("{stem}{suffix}.{extension}");
	path.parent()
		.map(|parent| parent.join(&file_name))
		.unwrap_or_else(|| PathBuf::from(file_name))
}
fn prior_path_map(state: Option<&SinkState>) -> BTreeMap<String, String> {
	state
		.and_then(|state| state.data.get("paths"))
		.and_then(Value::as_object)
		.map(|paths| {
			paths
				.iter()
				.filter_map(|(key, value)| {
					value.as_str().map(|value| (key.clone(), value.to_owned()))
				})
				.collect()
		})
		.unwrap_or_default()
}

fn plan_paths(
	batch: &ExportBatch,
	options: &RenderOptions,
	state: Option<&SinkState>,
) -> Result<BTreeMap<String, PathBuf>, AnnotationSyncError> {
	let old = prior_path_map(state);
	let folder_suffix = if options.path_template.is_empty() {
		DEFAULT_PATH_TEMPLATE.ends_with("/annotations.md")
	} else {
		options.path_template.ends_with("/annotations.md")
	};
	let mut occupied = BTreeMap::<PathBuf, String>::new();
	for (key, path) in &old {
		if let Ok(path) = validate_relative_destination(path) {
			occupied.insert(path, key.clone());
		}
	}
	let mut planned = BTreeMap::new();
	for book in &batch.books {
		let preferred = old
			.get(&book.key)
			.and_then(|path| validate_relative_destination(path).ok())
			.unwrap_or(path_for_book(book, options)?);
		let mut path = preferred.clone();
		let mut index = 2;
		while let Some(owner) = occupied.get(&path) {
			if owner == &book.key {
				break;
			}
			path = collision_path(&preferred, index, folder_suffix);
			index += 1;
			if index > 10_000 {
				return Err(AnnotationSyncError::sink("too many export path collisions"));
			}
		}
		occupied.insert(path.clone(), book.key.clone());
		planned.insert(book.key.clone(), path);
	}
	Ok(planned)
}

fn ensure_no_symlink(path: &Path) -> Result<(), AnnotationSyncError> {
	let mut current = PathBuf::new();
	for component in path.components() {
		match component {
			Component::Prefix(prefix) => current.push(prefix.as_os_str()),
			Component::RootDir => current.push(Path::new("/")),
			Component::Normal(value) => current.push(value),
			Component::CurDir | Component::ParentDir => {
				return Err(AnnotationSyncError::sink("path contains traversal"));
			},
		}
		if let Ok(metadata) = fs::symlink_metadata(&current) {
			if metadata.file_type().is_symlink() {
				return Err(AnnotationSyncError::sink(
					"symlinks are not allowed in export paths",
				));
			}
		}
	}
	Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), AnnotationSyncError> {
	ensure_no_symlink(path)?;
	if let Ok(metadata) = fs::symlink_metadata(path) {
		if !metadata.is_dir() {
			return Err(AnnotationSyncError::sink(
				"export path component is not a directory",
			));
		}
		return Ok(());
	}
	fs::create_dir_all(path)?;
	ensure_no_symlink(path)
}

/// A process-safe and cross-process lock shared by Markdown and Git exports.
pub(crate) struct UserLock {
	path: PathBuf,
	_file: File,
}

impl Drop for UserLock {
	fn drop(&mut self) {
		let _ = fs::remove_file(&self.path);
	}
}

pub(crate) fn acquire_user_lock(dir: &Path) -> Result<UserLock, AnnotationSyncError> {
	ensure_directory(dir)?;
	let path = dir.join(LOCK_FILE);
	ensure_no_symlink(&path)?;
	let deadline = Instant::now() + Duration::from_secs(30);
	loop {
		let mut options = OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		match options.open(&path) {
			Ok(file) => return Ok(UserLock { path, _file: file }),
			Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
				if Instant::now() >= deadline {
					return Err(AnnotationSyncError::sink(
						"timed out waiting for user export lock",
					));
				}
				std::thread::sleep(Duration::from_millis(10));
			},
			Err(error) => return Err(error.into()),
		}
	}
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<bool, AnnotationSyncError> {
	ensure_no_symlink(path)?;
	if let Ok(existing) = fs::read(path) {
		if existing == content {
			return Ok(false);
		}
	}
	let parent = path
		.parent()
		.ok_or_else(|| AnnotationSyncError::sink("export path has no parent"))?;
	ensure_directory(parent)?;
	let name = path
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or("annotation");
	let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
	let temp = parent.join(format!(".{name}.tmp-{}-{counter}", std::process::id()));
	ensure_no_symlink(&temp)?;
	let result = (|| -> Result<(), AnnotationSyncError> {
		let mut options = OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		let mut file = options.open(&temp)?;
		file.write_all(content)?;
		file.sync_all()?;
		ensure_no_symlink(path)?;
		fs::rename(&temp, path)?;
		if let Ok(directory) = File::open(parent) {
			let _ = directory.sync_all();
		}
		Ok(())
	})();
	if result.is_err() {
		let _ = fs::remove_file(&temp);
	}
	result.map(|()| true)
}

fn write_books_locked(
	dir: &Path,
	batch: &ExportBatch,
	options: &RenderOptions,
	state: Option<&SinkState>,
) -> Result<MarkdownWriteResult, AnnotationSyncError> {
	ensure_directory(dir)?;
	if options.is_v2() {
		validate_templates(options)?;
	}
	let planned = plan_paths(batch, options, state)?;
	let mut written = 0;
	let mut managed_paths = Vec::with_capacity(planned.len());
	for book in &batch.books {
		let relative = planned
			.get(&book.key)
			.ok_or_else(|| AnnotationSyncError::sink("missing planned book path"))?;
		let path = dir.join(relative);
		ensure_no_symlink(&path)?;
		let parent = path
			.parent()
			.ok_or_else(|| AnnotationSyncError::sink("book path has no parent"))?;
		ensure_directory(parent)?;
		let content = render_book_checked(book, options)?;
		if write_atomic(&path, content.as_bytes())? {
			written += 1;
		}
		managed_paths.push(relative.clone());
	}
	let path_map = planned
		.iter()
		.map(|(key, path)| (key.clone(), path.to_string_lossy().to_string()))
		.collect();
	Ok(MarkdownWriteResult {
		written,
		managed_paths,
		path_map,
	})
}

/// Result returned to the Git sink so it can stage exactly managed files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownWriteResult {
	pub written: usize,
	pub managed_paths: Vec<PathBuf>,
	pub path_map: BTreeMap<String, String>,
}

/// Renders every book into `dir`, skipping byte-identical files; returns how
/// many files were written. This compatibility wrapper uses the legacy lane.
pub fn write_books(
	dir: &Path,
	batch: &ExportBatch,
	options: &RenderOptions,
) -> Result<usize, AnnotationSyncError> {
	let _lock = acquire_user_lock(dir)?;
	Ok(write_books_locked(dir, batch, options, None)?.written)
}

/// Renders every book and returns the managed relative paths and stable source
/// key map. The map belongs in [`SinkState::data`] for future collision-safe
/// exports.
pub fn write_books_with_state(
	dir: &Path,
	batch: &ExportBatch,
	options: &RenderOptions,
	state: Option<&SinkState>,
) -> Result<MarkdownWriteResult, AnnotationSyncError> {
	let _lock = acquire_user_lock(dir)?;
	write_books_locked(dir, batch, options, state)
}

/// Internal Git entry point: the caller owns the user lock while it commits.
pub(crate) fn write_books_locked_for_git(
	dir: &Path,
	batch: &ExportBatch,
	options: &RenderOptions,
	state: Option<&SinkState>,
) -> Result<MarkdownWriteResult, AnnotationSyncError> {
	write_books_locked(dir, batch, options, state)
}

/// The markdown sink: renders every book into `<root>/<user_id>/`.
pub struct MarkdownSink {
	root: PathBuf,
	options: RenderOptions,
}

impl MarkdownSink {
	pub fn new(root: &Path, values: &SettingValues) -> Self {
		Self {
			root: root.to_path_buf(),
			options: RenderOptions::from_settings(values),
		}
	}

	pub fn user_dir(&self, user_id: &str) -> PathBuf {
		self.root.join(user_id)
	}
}

#[async_trait]
impl Sink for MarkdownSink {
	fn id(&self) -> &'static str {
		MARKDOWN_SINK_ID
	}

	fn descriptor(&self) -> SinkDescriptor {
		SinkDescriptor {
			id: MARKDOWN_SINK_ID,
			name: "Markdown vault",
			description: "Writes safe, human-named format-v2 notes below the mounted annotation root, while preserving versionless legacy exports.",
			settings: markdown_settings(),
			presets: preset_descriptors(),
		}
	}

	async fn export(
		&self,
		batch: &ExportBatch,
		state: &SinkState,
	) -> Result<SinkState, AnnotationSyncError> {
		let dir = user_dir_for_root(&self.root, &batch.user_id)?;
		let result = write_books_with_state(&dir, batch, &self.options, Some(state))?;
		tracing::debug!(
			user_id = %batch.user_id,
			books = batch.books.len(),
			written = result.written,
			"markdown sink export complete"
		);
		let data = if self.options.is_v2() {
			serde_json::json!({
				"format_version": FORMAT_VERSION_V2,
				"preset": self.options.preset.clone(),
				"paths": result.path_map,
			})
		} else {
			Value::Null
		};
		Ok(SinkState {
			liseur_seq: state.liseur_seq,
			data,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::{fixture_batch, fixture_book};

	#[test]
	fn legacy_render_stays_byte_compatible() {
		let rendered = render_book(&fixture_book(), &RenderOptions::default());
		assert!(rendered.starts_with("---\ntitle: \"Dune \\\"Messiah\\\"\""));
		assert!(rendered.contains("source: \"native\"\nmedia_id: \"m1\"\n"));
		assert!(rendered.contains("^a1"));
	}

	#[test]
	fn v2_has_structured_frontmatter_and_source_aware_blocks() {
		let options = RenderOptions {
			format_version: FORMAT_VERSION_V2,
			path_template: DEFAULT_PATH_TEMPLATE.to_owned(),
			..Default::default()
		};
		let rendered = render_book_checked(&fixture_book(), &options).unwrap();
		assert!(rendered.contains("schema: \"coppice.annotation/v2\""));
		assert!(rendered.contains("book:\n"));
		assert!(rendered.contains("source_updated_at:"));
		assert!(rendered.contains("provenance:"));
		assert!(rendered.contains("^native-m1-a1"));
		assert!(!rendered.contains("generated_at"));
	}

	#[test]
	fn rejects_traversal_reserved_and_oversized_destinations() {
		assert!(validate_relative_destination("../outside").is_err());
		assert!(validate_relative_destination("CON").is_err());
		assert!(validate_relative_destination(&"x".repeat(256)).is_err());
		assert!(validate_path_template("/tmp/{{title}}.md").is_err());
		assert!(validate_path_template("{{unknown}}.md").is_err());
	}

	#[tokio::test]
	async fn v2_paths_are_human_and_collision_map_is_stable() {
		let root = tempfile::tempdir().unwrap();
		let values: SettingValues = [("format_version".to_owned(), serde_json::json!(2))]
			.into_iter()
			.collect();
		let sink = MarkdownSink::new(root.path(), &values);
		let mut first = fixture_book();
		first.title = "Book".to_owned();
		first.authors = vec!["Author".to_owned()];
		let mut second = first.clone();
		second.key = "native:m2".to_owned();
		if let BookSource::Native { media_id } = &mut second.source {
			*media_id = "m2".to_owned();
		}
		let batch = fixture_batch(vec![second.clone(), first.clone()]);
		let state = sink.export(&batch, &SinkState::default()).await.unwrap();
		assert!(root
			.path()
			.join("user-1/Author - Book/annotations.md")
			.exists());
		assert!(root
			.path()
			.join("user-1/Author - Book (2)/annotations.md")
			.exists());
		let paths = state.data["paths"].as_object().unwrap();
		let first_path = paths["native:m1"].as_str().unwrap().to_owned();
		let second_path = paths["native:m2"].as_str().unwrap().to_owned();
		let mut changed = first.clone();
		changed.title = "Book".to_owned();
		let next = sink
			.export(&fixture_batch(vec![changed, second]), &state)
			.await
			.unwrap();
		assert_eq!(next.data["paths"]["native:m1"], first_path);
		assert_eq!(next.data["paths"]["native:m2"], second_path);
	}

	#[test]
	fn component_unicode_is_nfc_and_never_a_separator() {
		assert_eq!(normalize_component("Cafe\u{301}").unwrap(), "Café");
		assert!(!normalize_component("../escape").unwrap().contains('/'));
	}
}
