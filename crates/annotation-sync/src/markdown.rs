//! The markdown sink: one Obsidian-friendly file per book, byte-stable.
//!
//! File layout (see `README.md` for a full example):
//!
//! ```text
//! ---                      YAML frontmatter: title, authors, source ids,
//! title: "..."             identifiers, reading summary
//! ---
//! # <title>
//! ## Highlights            one list item per highlight/note, in canonical
//! - > quote                order: blockquoted excerpt, note paragraph, then a
//!   note                   `↗` meta line (position link, progression, color,
//!   ↗ ... ^<id>            created time) ending in an Obsidian block id
//! ## Bookmarks             native bookmarks, then liseur bookmark rows
//! ```
//!
//! Byte stability rules (any violation re-renders a different file for
//! unchanged data):
//!
//! - YAML scalars are always double-quoted JSON strings (`serde_json`
//!   escaping is valid YAML double-quote style);
//! - timestamps render as RFC 3339 UTC with second precision and `Z`;
//! - progressions render with exactly four decimals (frontmatter) or as a
//!   percentage with two decimals (meta lines);
//! - rows appear in the canonical model order; tombstones are skipped;
//! - sections without content are omitted entirely.
//!
//! Writes are skipped when the existing file already matches byte-for-byte,
//! so re-exports do not touch mtimes. Files for books whose media row was
//! deleted entirely (cascading their annotations away) are left in place; the
//! sink only manages books that still have data.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::SecondsFormat;
use serde_json::Value;
use stump_api_types::settings::{SettingDefinition, SettingKind, SettingValues};

use crate::error::AnnotationSyncError;
use crate::model::{
	BookSource, ExportAnnotation, ExportAnnotationKind, ExportBatch, ExportBook,
	ExportBookmark,
};
use crate::sink::{string_setting, Sink, SinkDescriptor, SinkState};

pub const MARKDOWN_SINK_ID: &str = "markdown";

/// The `base_url` setting shared by the markdown and git sinks.
pub fn base_url_setting() -> SettingDefinition {
	SettingDefinition {
		key: "base_url",
		label: "Stump base URL",
		description: "Public URL of this server (e.g. https://stump.example.com). When set, highlight positions link to the book's reader; otherwise they are plain text.",
		kind: SettingKind::String,
		default: Value::Null,
		required: false,
		secret: false,
		help_url: None,
	}
}

/// Rendering options derived from sink settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderOptions {
	/// Public server URL without a trailing slash; enables reader links.
	pub base_url: Option<String>,
}

impl RenderOptions {
	pub fn from_settings(values: &SettingValues) -> Self {
		let base_url = string_setting(values, "base_url", "");
		let base_url = base_url.trim_end_matches('/');
		Self {
			base_url: (!base_url.is_empty()).then(|| base_url.to_owned()),
		}
	}
}

/// Renders the markdown file content for one book.
pub fn render_book(book: &ExportBook, options: &RenderOptions) -> String {
	let mut out = String::new();
	out.push_str("---\n");
	push_frontmatter(&mut out, book);
	out.push_str("---\n\n");
	out.push_str(&format!("# {}\n", book.title));

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
			push_annotation(&mut out, annotation, reader_url.as_deref());
		}
	}
	if !book.bookmarks.is_empty() || !bookmark_rows.is_empty() {
		out.push_str("\n## Bookmarks\n");
		for bookmark in &book.bookmarks {
			push_bookmark(&mut out, bookmark, reader_url.as_deref());
		}
		for annotation in bookmark_rows {
			push_annotation(&mut out, annotation, reader_url.as_deref());
		}
	}

	out
}

fn push_frontmatter(out: &mut String, book: &ExportBook) {
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

/// One list item: the blockquoted excerpt, the note paragraph, and the meta
/// line. A note without an excerpt renders its text as the item body.
fn push_annotation(
	out: &mut String,
	annotation: &ExportAnnotation,
	reader_url: Option<&str>,
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
	push_meta(out, &meta, &annotation.id, first);
}

fn push_bookmark(out: &mut String, bookmark: &ExportBookmark, reader_url: Option<&str>) {
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
	push_meta(out, &meta, &bookmark.id, first);
}

/// A blockquote as (the start of) a list item; continuation lines keep the
/// two-space item indent.
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

/// A paragraph inside the list item (or opening it when `first`).
fn push_paragraph(out: &mut String, text: &str, first: bool) {
	for (index, line) in text.lines().enumerate() {
		let prefix = if first && index == 0 { "- " } else { "  " };
		out.push_str(&format!("{prefix}{line}\n"));
	}
}

/// The `↗` meta line ending in an Obsidian block id derived from the row id.
fn push_meta(out: &mut String, meta: &[String], id: &str, first: bool) {
	let prefix = if first { "- " } else { "\n  " };
	out.push_str(&format!(
		"{prefix}↗ {} ^{}\n",
		meta.join(" · "),
		block_id(id)
	));
}

fn link(text: &str, url: Option<&str>) -> String {
	match url {
		Some(url) => format!("[{text}]({url})"),
		None => text.to_owned(),
	}
}

/// A human-readable position from a locator: the Readium `href`, an EPUB
/// `cfi`, or a `position` number, whichever the locator carries first.
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

/// Obsidian block ids allow letters, digits, and dashes only.
fn block_id(id: &str) -> String {
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

fn quote(value: &str) -> String {
	// serde_json string escaping is valid YAML double-quoted style.
	serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
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

fn timestamp(value: chrono::DateTime<chrono::Utc>) -> String {
	value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The file name for one book; derived from the stable book key.
pub fn file_name(book: &ExportBook) -> String {
	format!("{}.md", book.key.replace(':', "-"))
}

/// Renders every book into `dir`, skipping byte-identical files; returns how
/// many files were written.
pub fn write_books(
	dir: &Path,
	batch: &ExportBatch,
	options: &RenderOptions,
) -> Result<usize, AnnotationSyncError> {
	std::fs::create_dir_all(dir)?;
	let mut written = 0;
	for book in &batch.books {
		let content = render_book(book, options);
		let path = dir.join(file_name(book));
		let unchanged = std::fs::read(&path)
			.map(|existing| existing == content.as_bytes())
			.unwrap_or(false);
		if !unchanged {
			std::fs::write(&path, content.as_bytes())?;
			written += 1;
		}
	}
	Ok(written)
}

/// The markdown sink: renders every book in the batch into
/// `<root>/<user_id>/`.
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
			description: "Writes one Obsidian-friendly markdown file per book into the configured annotation root.",
			settings: vec![base_url_setting()],
		}
	}

	async fn export(
		&self,
		batch: &ExportBatch,
		state: &SinkState,
	) -> Result<SinkState, AnnotationSyncError> {
		let written = write_books(&self.user_dir(&batch.user_id), batch, &self.options)?;

		tracing::debug!(
			user_id = %batch.user_id,
			books = batch.books.len(),
			written,
			"markdown sink export complete"
		);

		// The markdown sink keeps no state beyond the host-owned cursor.
		Ok(SinkState {
			liseur_seq: state.liseur_seq,
			data: Value::Null,
		})
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;
	use crate::test_support::{fixture_batch, fixture_book};

	const EXPECTED: &str = r#"---
title: "Dune \"Messiah\""
authors:
  - "Frank Herbert"
source: "native"
media_id: "m1"
identifiers:
  isbn: "9780441172696"
progression: 0.7512
page: 300
completed: false
last_read: "2023-11-14T22:14:30Z"
last_read_via: "liseur"
sessions: 2
reading_time_seconds: 5400
---

# Dune "Messiah"

## Highlights

- > The spice must flow.

  Opening line

  ↗ [ch1.xhtml](https://stump.example.com/books/m1/epub-reader) · 25.00% · 2023-11-14T22:13:30Z ^a1

- > Fear is the mind-killer.
  > I will face my fear.

  ↗ [/6/4!/4/2](https://stump.example.com/books/m1/epub-reader) · 50.00% · yellow · 2023-11-14T22:13:40Z ^l1

- Re-read chapter two

  ↗ 2023-11-14T22:13:50Z ^l2

## Bookmarks

- > A beginning is the time

  ↗ [ch3.xhtml · page 42](https://stump.example.com/books/m1/epub-reader) · 2023-11-14T22:14:20Z ^b1

- ↗ [/6/8!/2](https://stump.example.com/books/m1/epub-reader) · 75.00% · 2023-11-14T22:14:10Z ^l4
"#;

	fn options() -> RenderOptions {
		RenderOptions {
			base_url: Some("https://stump.example.com".to_string()),
		}
	}

	#[test]
	fn renders_obsidian_layout_and_skips_tombstones() {
		let book = fixture_book();
		let rendered = render_book(&book, &options());
		assert_eq!(rendered, EXPECTED);
		assert!(!rendered.contains("deleted text"));
	}

	#[test]
	fn render_without_base_url_has_plain_positions() {
		let rendered = render_book(&fixture_book(), &RenderOptions::default());
		assert!(rendered.contains("↗ ch1.xhtml · 25.00% · 2023-11-14T22:13:30Z ^a1"));
		assert!(!rendered.contains("]("));
	}

	#[test]
	fn liseur_work_books_never_link_and_key_the_file_name() {
		let mut book = fixture_book();
		book.key = "liseur:w-1".to_string();
		book.source = BookSource::LiseurWork {
			work_id: "w-1".to_string(),
		};
		assert_eq!(file_name(&book), "liseur-w-1.md");
		let rendered = render_book(&book, &options());
		assert!(rendered.contains("source: \"liseur\"\nwork_id: \"w-1\"\n"));
		assert!(!rendered.contains("]("));
	}

	#[test]
	fn base_url_setting_is_normalized() {
		let values: SettingValues = [(
			"base_url".to_string(),
			Value::String("https://s.example/".into()),
		)]
		.into_iter()
		.collect();
		assert_eq!(
			RenderOptions::from_settings(&values).base_url.as_deref(),
			Some("https://s.example")
		);
		assert_eq!(
			RenderOptions::from_settings(&SettingValues::default()),
			RenderOptions::default()
		);
	}

	#[tokio::test]
	async fn export_is_byte_stable_and_leaves_unchanged_files_alone() {
		let root = tempfile::tempdir().unwrap();
		let sink = MarkdownSink::new(root.path(), &SettingValues::default());
		let batch = fixture_batch(vec![fixture_book()]);
		let state = SinkState::default();

		sink.export(&batch, &state).await.unwrap();
		let path = root.path().join("user-1").join("native-m1.md");
		let first = std::fs::read(&path).unwrap();
		assert_eq!(
			first,
			render_book(&batch.books[0], &RenderOptions::default()).as_bytes()
		);
		let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

		std::thread::sleep(Duration::from_millis(20));
		let out = sink.export(&batch, &state).await.unwrap();
		assert_eq!(std::fs::read(&path).unwrap(), first);
		assert_eq!(
			std::fs::metadata(&path).unwrap().modified().unwrap(),
			modified,
			"an unchanged book must not be rewritten"
		);
		assert_eq!(out.liseur_seq, state.liseur_seq);
	}
}
