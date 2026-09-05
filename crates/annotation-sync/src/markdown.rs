//! The markdown sink: one Obsidian-friendly file per book, byte-stable.
//!
//! Byte stability rules (any violation re-renders a different file for
//! unchanged data):
//!
//! - YAML scalars are always double-quoted JSON strings (`serde_json`
//!   escaping is valid YAML double-quote style);
//! - timestamps render as `RFC 3339` UTC with second precision and `Z`;
//! - progressions render with exactly four decimals;
//! - locator JSON is `serde_json` compact output (object keys are sorted
//!   because `serde_json` maps are `BTreeMap`s without `preserve_order`);
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
use stump_api_types::settings::SettingDefinition;

use crate::error::AnnotationSyncError;
use crate::model::{ExportAnnotation, ExportAnnotationKind, ExportBatch, ExportBook};
use crate::sink::{Sink, SinkDescriptor, SinkState};

pub const MARKDOWN_SINK_ID: &str = "markdown";

const MARKDOWN_SETTINGS: &[SettingDefinition] = &[];

/// Renders the markdown file content for one book.
pub fn render_book(book: &ExportBook) -> String {
	let mut out = String::new();
	out.push_str("---\n");
	push_frontmatter(&mut out, book);
	out.push_str("---\n");

	let highlights: Vec<&ExportAnnotation> = book
		.annotations
		.iter()
		.filter(|annotation| !annotation.deleted && annotation.kind != ExportAnnotationKind::Bookmark)
		.collect();
	let bookmark_rows: Vec<&ExportAnnotation> = book
		.annotations
		.iter()
		.filter(|annotation| !annotation.deleted && annotation.kind == ExportAnnotationKind::Bookmark)
		.collect();

	if !highlights.is_empty() {
		out.push_str("\n## Highlights\n");
		for annotation in highlights {
			push_annotation(&mut out, annotation);
		}
	}
	if !book.bookmarks.is_empty() || !bookmark_rows.is_empty() {
		out.push_str("\n## Bookmarks\n");
		for bookmark in &book.bookmarks {
			out.push_str(&format!("- bookmark `{}`\n", quote(&bookmark.id)));
			if let Some(preview) = bookmark
				.preview_content
				.as_deref()
				.filter(|preview| !preview.is_empty())
			{
				push_quote(&mut out, preview);
			}
			let mut details = Vec::new();
			if let Some(page) = bookmark.page {
				details.push(format!("page {page}"));
			}
			if let Some(locator) = &bookmark.locator {
				details.push(format!("↗ {}", quote(&locator_json(locator))));
			}
			details.push(format!("created {}", timestamp(bookmark.created_at)));
			for detail in details {
				out.push_str(&format!("  - {detail}\n"));
			}
		}
		for annotation in bookmark_rows {
			push_annotation(&mut out, annotation);
		}
	}

	out
}

fn push_frontmatter(out: &mut String, book: &ExportBook) {
	out.push_str(&format!("book: {}\n", quote(&book.title)));
	if !book.authors.is_empty() {
		out.push_str("authors:\n");
		for author in &book.authors {
			out.push_str(&format!("  - {}\n", quote(author)));
		}
	}
	match &book.source {
		crate::model::BookSource::Native { media_id } => {
			out.push_str("source: native\n");
			out.push_str(&format!("media_id: {}\n", quote(media_id)));
		},
		crate::model::BookSource::LiseurWork { work_id } => {
			out.push_str("source: liseur\n");
			out.push_str(&format!("work_id: {}\n", quote(work_id)));
		},
	}
	if !book.identifiers.is_empty() {
		out.push_str("identifiers:\n");
		for identifier in &book.identifiers {
			out.push_str(&format!(
				"  {}: {}\n",
				quote(&identifier.scheme),
				quote(&identifier.value)
			));
		}
	}
	if let Some(reading) = &book.reading {
		if let Some(progression) = reading.progression {
			out.push_str(&format!("progression: {:.4}\n", progression));
		}
		out.push_str(&format!("completed: {}\n", reading.completed));
		if let Some(last_read_at) = reading.last_read_at {
			out.push_str(&format!("last_read: {}\n", timestamp(last_read_at)));
		}
		if let Some(protocol) = &reading.source_protocol {
			out.push_str(&format!("last_read_via: {}\n", quote(protocol)));
		}
		out.push_str(&format!("sessions: {}\n", reading.session_count));
		if let Some(total_seconds) = reading.total_seconds {
			out.push_str(&format!("reading_time_seconds: {total_seconds}\n"));
		}
		if reading.finished {
			out.push_str("finished: true\n");
		}
	}
}

fn push_annotation(out: &mut String, annotation: &ExportAnnotation) {
	out.push('\n');
	if let Some(excerpt) = annotation
		.excerpt
		.as_deref()
		.filter(|excerpt| !excerpt.is_empty())
	{
		push_quote(out, excerpt);
	}
	if annotation.kind != ExportAnnotationKind::Highlight {
		let kind_label = if matches!(annotation.kind, ExportAnnotationKind::Bookmark) {
			"bookmark"
		} else {
			"note"
		};
		out.push_str(&format!("  - kind: {kind_label}\n"));
	}
	if let Some(color) = &annotation.color {
		out.push_str(&format!("  - color: {}\n", quote(color)));
	}
	if let Some(note) = annotation
		.note
		.as_deref()
		.filter(|note| !note.is_empty())
	{
		out.push_str(&format!("  - note: {}\n", quote(note)));
	}
	let mut details = Vec::new();
	if let Some(progression) = annotation.progression {
		details.push(format!("↗ {:.2}%", progression * 100.0));
	}
	if let Some(locator) = &annotation.locator {
		details.push(format!("locator {}", quote(&locator_json(locator))));
	}
	if let Some(created) = annotation.created_at {
		details.push(format!("created {}", timestamp(created)));
	}
	for detail in details {
		out.push_str(&format!("  - {detail}\n"));
	}
}

/// Renders a blockquote; continuation lines keep the two-space list indent.
fn push_quote(out: &mut String, text: &str) {
	for line in text.lines() {
		out.push_str(&if line.is_empty() {
			"  >\n".to_string()
		} else {
			format!("  > {line}\n")
		});
	}
}

fn quote(value: &str) -> String {
	// serde_json string escaping is valid YAML double-quoted style.
	serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn locator_json(locator: &serde_json::Value) -> String {
	serde_json::to_string(locator).unwrap_or_else(|_| "{}".to_string())
}

fn timestamp(value: chrono::DateTime<chrono::Utc>) -> String {
	value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The file name for one book; derived from the stable book key.
pub fn file_name(book: &ExportBook) -> String {
	format!("{}.md", book.key.replace(':', "-"))
}

/// The markdown sink: renders every book in the batch into
/// `<root>/<user_id>/`.
pub struct MarkdownSink {
	root: PathBuf,
}

impl MarkdownSink {
	pub fn new(root: &Path) -> Self {
		Self { root: root.to_path_buf() }
	}

	pub fn book_path(&self, batch: &ExportBatch, book: &ExportBook) -> PathBuf {
		self.root.join(&batch.user_id).join(file_name(book))
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
			settings: MARKDOWN_SETTINGS.to_vec(),
			description: "Writes one Obsidian-friendly markdown file per book into the configured annotation root.",
		}
	}

	async fn export(
		&self,
		batch: &ExportBatch,
		state: &SinkState,
	) -> Result<SinkState, AnnotationSyncError> {
		let dir = self.root.join(&batch.user_id);
		std::fs::create_dir_all(&dir)?;

		let mut written = 0usize;
		for book in &batch.books {
			let content = render_book(book);
			let path = self.book_path(batch, book);
			let unchanged = std::fs::read(&path)
				.map(|existing| existing == content.as_bytes())
				.unwrap_or(false);
			if !unchanged {
				std::fs::write(&path, content.as_bytes())?;
				written += 1;
			}
		}

		tracing::debug!(
			user_id = %batch.user_id,
			books = batch.books.len(),
			written,
			"markdown sink export complete"
		);

		// The markdown sink keeps no state beyond the host-owned cursor.
		Ok(SinkState {
			liseur_seq: state.liseur_seq,
			data: serde_json::Value::Null,
		})
	}
}
