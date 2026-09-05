//! The canonical export model.
//!
//! One [`ExportBook`] per `(user, work/media)`; the batch is what sinks
//! receive. Every ordering is fixed in code (and mirrored in SQL `ORDER BY`
//! where rows could otherwise arrive nondeterministically) so that exporting
//! unchanged data re-renders byte-identical files.
//!
//! Sources folded into a book:
//!
//! - native `media_annotations` + `bookmarks` (Stump GraphQL path),
//! - liseur-sync CAS annotations (`liseur_sync_annotations`, tombstones
//!   included), either standalone (`liseur:<work_id>` books) or folded into
//!   the linked Stump media's book via `liseur_sync_media_links`,
//! - reading heads + reading sessions, rendered as a summary block.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use models::entity::{bookmark, media, media_annotation, media_metadata, reading_head, reading_session};
use sea_orm::{prelude::*, DbBackend, QueryOrder, QuerySelect, Statement};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AnnotationSyncError;

/// A batch of books to export for one user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBatch {
	pub user_id: String,
	/// The liseur CAS annotation seq the batch was built from (inclusive
	/// cursor carried by the sinks' state).
	pub liseur_from_seq: i64,
	/// The liseur CAS annotation high-water mark observed while building.
	pub liseur_high_water: i64,
	/// Books to export, sorted by [`ExportBook::key`].
	pub books: Vec<ExportBook>,
}

/// Where a book's identity comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BookSource {
	Native { media_id: String },
	LiseurWork { work_id: String },
}

/// One book's canonical export state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBook {
	/// Stable book key: `native:<media_id>` or `liseur:<work_id>`.
	pub key: String,
	pub source: BookSource,
	pub title: String,
	/// Authors as carried by the metadata (native `writers` split on commas);
	/// the order is the stored order, which is itself stable.
	pub authors: Vec<String>,
	/// Identifiers sorted by `(scheme, value)`, deduplicated.
	pub identifiers: Vec<ExportIdentifier>,
	/// Annotations sorted by `(created_at, id)`; tombstones included.
	pub annotations: Vec<ExportAnnotation>,
	/// Native bookmarks sorted by `(created_at, id)`.
	pub bookmarks: Vec<ExportBookmark>,
	/// Reading-head and session summary, present when the book has one.
	pub reading: Option<ReadingSummary>,
}

impl ExportBook {
	fn new(source: BookSource, title: String) -> Self {
		Self {
			key: book_key(&source),
			source,
			title,
			authors: Vec::new(),
			identifiers: Vec::new(),
			annotations: Vec::new(),
			bookmarks: Vec::new(),
			reading: None,
		}
	}
}

/// The stable per-book key used for file names and sorting.
pub fn book_key(source: &BookSource) -> String {
	match source {
		BookSource::Native { media_id } => format!("native:{media_id}"),
		BookSource::LiseurWork { work_id } => format!("liseur:{work_id}"),
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportIdentifier {
	pub scheme: String,
	pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportAnnotationKind {
	Highlight,
	Note,
	Bookmark,
}

/// Which backend produced an annotation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnnotationOrigin {
	Native,
	/// A liseur-sync CAS row; `rev`/`seq` are its CAS coordinates.
	Liseur { rev: i64, seq: i64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportAnnotation {
	pub id: String,
	pub kind: ExportAnnotationKind,
	pub origin: AnnotationOrigin,
	/// The Readium locator for native rows, the raw liseur locator JSON
	/// otherwise.
	pub locator: Option<Value>,
	pub progression: Option<f64>,
	/// Liseur palette token (`yellow`, `green`, ...).
	pub color: Option<String>,
	/// The highlighted text (liseur `excerpt`).
	pub excerpt: Option<String>,
	/// The user's note (`annotation_text` for native rows, `body` otherwise).
	pub note: Option<String>,
	pub created_at: Option<DateTime<Utc>>,
	pub updated_at: Option<DateTime<Utc>>,
	/// Liseur deletion tombstone; native rows are hard-deleted so they simply
	/// stop appearing in the next full rebuild of the book.
	pub deleted: bool,
	pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBookmark {
	pub id: String,
	pub locator: Option<Value>,
	pub preview_content: Option<String>,
	pub page: Option<i32>,
	pub created_at: DateTime<Utc>,
}

/// Reading head + session summary block for one book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingSummary {
	pub progression: Option<f64>,
	pub page: Option<i32>,
	pub completed: bool,
	pub last_read_at: Option<DateTime<Utc>>,
	pub source_protocol: Option<String>,
	pub session_count: i64,
	pub total_seconds: Option<i64>,
	pub last_session_at: Option<DateTime<Utc>>,
	pub finished: bool,
}

/// Options for [`build_export_batch`].
#[derive(Debug, Clone, Copy, Default)]
pub struct BuildOptions {
	/// Build liseur books only for works with annotation `seq` strictly above
	/// this cursor (native books are always rebuilt in full). `0` includes
	/// everything.
	pub liseur_from_seq: i64,
}

/// Builds the canonical export batch for a user.
///
/// Native books (every media with at least one annotation or bookmark) are
/// always rebuilt in full: native rows are hard-deleted on removal, so a
/// cursor-based delta would miss deletions. The annotated-media set is bounded
/// by the user's actual annotation count, not library size. Liseur works are
/// incremental by CAS `seq`; deletions are tombstones and therefore visible.
pub async fn build_export_batch(
	conn: &DatabaseConnection,
	user_id: &str,
	options: BuildOptions,
) -> Result<ExportBatch, AnnotationSyncError> {
	let mut books: Vec<ExportBook> = Vec::new();
	let mut book_by_key: HashMap<String, usize> = HashMap::new();

	// ---- native books ---------------------------------------------------
	let annotation_media_ids: Vec<String> = media_annotation::Entity::find()
		.filter(media_annotation::Column::UserId.eq(user_id))
		.select_only()
		.distinct()
		.column(media_annotation::Column::MediaId)
		.into_tuple()
		.all(conn)
		.await?;
	let bookmark_media_ids: Vec<String> = bookmark::Entity::find()
		.filter(bookmark::Column::UserId.eq(user_id))
		.select_only()
		.distinct()
		.column(bookmark::Column::MediaId)
		.into_tuple()
		.all(conn)
		.await?;

	let mut native_media_ids: BTreeSet<String> = BTreeSet::new();
	native_media_ids.extend(annotation_media_ids);
	native_media_ids.extend(bookmark_media_ids);

	for media_id in native_media_ids {
		if let Some(book) = build_native_book(conn, user_id, &media_id).await? {
			book_by_key.insert(book.key.clone(), books.len());
			books.push(book);
		}
	}

	// ---- liseur CAS works ------------------------------------------------
	let (changed_work_ids, high_water) =
		liseur_changed_works(conn, user_id, options.liseur_from_seq).await?;

	// Linked works fold into their media's book; standalone works get their
	// own book. A linked work may also surface a book the user has no native
	// annotations for (the media exists but was only annotated via liseur).
	for work_id in changed_work_ids {
		let linked_media_id = liseur_linked_media(conn, user_id, &work_id).await?;
		match linked_media_id {
			Some(media_id) => {
				let key = book_key(&BookSource::Native { media_id: media_id.clone() });
				if let Some(index) = book_by_key.get(&key).copied() {
					fold_liseur_work(conn, user_id, &work_id, &mut books[index]).await?;
				} else if let Some(mut book) = build_native_book(conn, user_id, &media_id).await? {
					fold_liseur_work(conn, user_id, &work_id, &mut book).await?;
					book_by_key.insert(book.key.clone(), books.len());
					books.push(book);
				}
			},
			None => {
				let mut book = build_liseur_work_book(conn, user_id, &work_id).await?;
				// A standalone liseur work has no unified head/session rows
				// (those are keyed by media; a linked work would not be here).
				book.reading = None;
				book_by_key.insert(book.key.clone(), books.len());
				books.push(book);
			},
		}
	}

	books.sort_by(|a, b| a.key.cmp(&b.key));

	Ok(ExportBatch {
		user_id: user_id.to_owned(),
		liseur_from_seq: options.liseur_from_seq,
		liseur_high_water: high_water,
		books,
	})
}

/// Builds one native book (media + metadata + annotations + bookmarks +
/// summary), or `None` when the media row has disappeared.
async fn build_native_book(
	conn: &DatabaseConnection,
	user_id: &str,
	media_id: &str,
) -> Result<Option<ExportBook>, AnnotationSyncError> {
	let Some(media) = media::Entity::find_by_id(media_id).one(conn).await? else {
		return Ok(None);
	};
	let metadata = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.eq(media_id))
		.one(conn)
		.await?;

	let title = metadata
		.as_ref()
		.and_then(|metadata| metadata.title.clone())
		.unwrap_or_else(|| media.name.clone());
	let mut book = ExportBook::new(BookSource::Native { media_id: media_id.to_owned() }, title);

	if let Some(metadata) = metadata {
		if let Some(writers) = &metadata.writers {
			book.authors = writers
				.split(',')
				.map(str::trim)
				.filter(|author| !author.is_empty())
				.map(str::to_owned)
				.collect();
		}
		for (scheme, value) in [
			("isbn", &metadata.identifier_isbn),
			("google", &metadata.identifier_google),
			("amazon", &metadata.identifier_amazon),
			("mobi-asin", &metadata.identifier_mobi_asin),
			("uuid", &metadata.identifier_uuid),
			("calibre", &metadata.identifier_calibre),
		] {
			if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
				book.identifiers.push(ExportIdentifier {
					scheme: scheme.to_owned(),
					value: value.to_owned(),
				});
			}
		}
	}
	sort_identifiers(&mut book.identifiers);

	let annotations = media_annotation::Entity::find()
		.filter(media_annotation::Column::UserId.eq(user_id))
		.filter(media_annotation::Column::MediaId.eq(media_id))
		.order_by_asc(media_annotation::Column::CreatedAt)
		.order_by_asc(media_annotation::Column::Id)
		.all(conn)
		.await?;
	book.annotations = annotations
		.iter()
		.map(|annotation| {
			Ok(ExportAnnotation {
				id: annotation.id.clone(),
				kind: ExportAnnotationKind::Highlight,
				origin: AnnotationOrigin::Native,
				locator: Some(serde_json::to_value(&annotation.locator)?),
				progression: None,
				color: None,
				excerpt: None,
				note: annotation.annotation_text.clone(),
				created_at: Some(annotation.created_at),
				updated_at: Some(annotation.updated_at),
				deleted: false,
				deleted_at: None,
			})
		})
		.collect::<Result<Vec<_>, serde_json::Error>>()?;

	let bookmarks = bookmark::Entity::find()
		.filter(bookmark::Column::UserId.eq(user_id))
		.filter(bookmark::Column::MediaId.eq(media_id))
		.order_by_asc(bookmark::Column::CreatedAt)
		.order_by_asc(bookmark::Column::Id)
		.all(conn)
		.await?;
	book.bookmarks = bookmarks
		.iter()
		.map(|row| {
			Ok(ExportBookmark {
				id: row.id.clone(),
				locator: row
					.locator
					.as_ref()
					.map(serde_json::to_value)
					.transpose()?,
				preview_content: row.preview_content.clone(),
				page: row.page,
				created_at: row.created_at,
			})
		})
		.collect::<Result<Vec<_>, serde_json::Error>>()?;

	book.reading = build_reading_summary(conn, user_id, media_id).await?;

	Ok(Some(book))
}

async fn build_reading_summary(
	conn: &DatabaseConnection,
	user_id: &str,
	media_id: &str,
) -> Result<Option<ReadingSummary>, AnnotationSyncError> {
	let head = reading_head::Entity::find()
		.filter(reading_head::Column::UserId.eq(user_id))
		.filter(reading_head::Column::MediaId.eq(media_id))
		.one(conn)
		.await?;

	let sessions = reading_session::Entity::find()
		.filter(reading_session::Column::UserId.eq(user_id))
		.filter(reading_session::Column::MediaId.eq(media_id))
		.order_by_asc(reading_session::Column::Id)
		.all(conn)
		.await?;

	if head.is_none() && sessions.is_empty() {
		return Ok(None);
	}

	let session_count = sessions.len() as i64;
	let total_seconds = sessions
		.iter()
		.filter_map(|session| session.elapsed_seconds)
		.reduce(i64::saturating_add);
	let last_session_at = sessions
		.iter()
		.filter_map(|session| session.updated_at.or(Some(session.created_at)))
		.map(|value| value.with_timezone(&Utc))
		.max();
	let finished = sessions.iter().any(|session| session.is_complete());

	Ok(Some(ReadingSummary {
		progression: head.as_ref().map(|head| head.progression),
		page: head.as_ref().and_then(|head| head.page),
		completed: head.as_ref().is_some_and(|head| head.completed),
		last_read_at: head.as_ref().map(|head| head.changed_at.with_timezone(&Utc)),
		source_protocol: head.as_ref().map(|head| head.source_protocol.to_string()),
		session_count,
		total_seconds,
		last_session_at,
		finished,
	}))
}

/// Builds a standalone liseur work book.
async fn build_liseur_work_book(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
) -> Result<ExportBook, AnnotationSyncError> {
	let (title, author) = liseur_work_meta(conn, user_id, work_id).await?;
	let mut book = ExportBook::new(BookSource::LiseurWork { work_id: work_id.to_owned() }, title);
	if let Some(author) = author.filter(|author| !author.is_empty()) {
		book.authors.push(author);
	}
	book.identifiers = liseur_work_identifiers(conn, user_id, work_id).await?;
	merge_annotations(conn, user_id, work_id, &mut book).await?;
	Ok(book)
}

/// Folds one linked liseur work's annotations, author, and identifiers into a
/// native book.
async fn fold_liseur_work(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
	book: &mut ExportBook,
) -> Result<(), AnnotationSyncError> {
	let (_, author) = liseur_work_meta(conn, user_id, work_id).await?;
	if let Some(author) = author.filter(|author| !author.is_empty()) {
		if !book.authors.contains(&author) {
			book.authors.push(author);
		}
	}
	let mut identifiers = liseur_work_identifiers(conn, user_id, work_id).await?;
	book.identifiers.append(&mut identifiers);
	sort_identifiers(&mut book.identifiers);
	merge_annotations(conn, user_id, work_id, book).await?;
	Ok(())
}

/// Appends a work's CAS annotations in the deterministic `(created_at, id)`
/// order.
async fn merge_annotations(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
	book: &mut ExportBook,
) -> Result<(), AnnotationSyncError> {
	let mut annotations = liseur_annotations(conn, user_id, work_id).await?;
	annotations.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
	book.annotations.extend(annotations);
	Ok(())
}

fn sort_identifiers(identifiers: &mut Vec<ExportIdentifier>) {
	identifiers.sort_by(|a, b| (&a.scheme, &a.value).cmp(&(&b.scheme, &b.value)));
	identifiers.dedup_by(|a, b| a.scheme == b.scheme && a.value == b.value);
}

/// Reads the liseur CAS high-water and the works changed above `from_seq`.
async fn liseur_changed_works(
	conn: &DatabaseConnection,
	user_id: &str,
	from_seq: i64,
) -> Result<(Vec<String>, i64), AnnotationSyncError> {
	let backend = conn.get_database_backend();
	let counters = conn
		.query_one(statement(
			backend,
			"SELECT annotation_seq FROM liseur_sync_counters WHERE user_id = $1",
			vec![user_id.into()],
		))
		.await?;
	let high_water = counters
		.map(|row| row.try_get::<i64>("", "annotation_seq"))
		.transpose()?
		.unwrap_or(0);

	let rows = conn
		.query_all(statement(
			backend,
			"SELECT DISTINCT work_id FROM liseur_sync_annotations
             WHERE user_id = $1 AND seq > $2
             ORDER BY work_id",
			vec![user_id.into(), from_seq.into()],
		))
		.await?;
	let work_ids = rows
		.iter()
		.map(|row| row.try_get::<String>("", "work_id"))
		.collect::<Result<Vec<_>, _>>()?;

	Ok((work_ids, high_water))
}

async fn liseur_linked_media(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
) -> Result<Option<String>, AnnotationSyncError> {
	let backend = conn.get_database_backend();
	let row = conn
		.query_one(statement(
			backend,
			"SELECT media_id FROM liseur_sync_media_links
             WHERE user_id = $1 AND work_id = $2
             ORDER BY created_at ASC, id ASC
             LIMIT 1",
			vec![user_id.into(), work_id.into()],
		))
		.await?;
	Ok(row
		.map(|row| row.try_get::<Option<String>>("", "media_id"))
		.transpose()?
		.flatten())
}

/// Returns `(title, author)` for a liseur work; the work id stands in for a
/// missing title.
async fn liseur_work_meta(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
) -> Result<(String, Option<String>), AnnotationSyncError> {
	let backend = conn.get_database_backend();
	let row = conn
		.query_one(statement(
			backend,
			"SELECT title, author FROM liseur_sync_works
             WHERE user_id = $1 AND id = $2",
			vec![user_id.into(), work_id.into()],
		))
		.await?;
	let Some(row) = row else {
		return Ok((work_id.to_owned(), None));
	};
	let title: Option<String> = row.try_get("", "title")?;
	let author: Option<String> = row.try_get("", "author")?;
	Ok((title.unwrap_or_else(|| work_id.to_owned()), author))
}

async fn liseur_work_identifiers(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
) -> Result<Vec<ExportIdentifier>, AnnotationSyncError> {
	let backend = conn.get_database_backend();
	let rows = conn
		.query_all(statement(
			backend,
			"SELECT kind, value FROM liseur_sync_aliases
             WHERE user_id = $1 AND work_id = $2
             ORDER BY kind, value",
			vec![user_id.into(), work_id.into()],
		))
		.await?;
	let mut identifiers: Vec<ExportIdentifier> = rows
		.iter()
		.map(|row| -> Result<ExportIdentifier, sea_orm::DbErr> {
			Ok(ExportIdentifier {
				scheme: row
					.try_get::<Option<String>>("", "kind")?
					.unwrap_or_default(),
				value: row
					.try_get::<Option<String>>("", "value")?
					.unwrap_or_default(),
			})
		})
		.collect::<Result<Vec<_>, _>>()?;
	identifiers.retain(|identifier| !identifier.scheme.is_empty() && !identifier.value.is_empty());
	sort_identifiers(&mut identifiers);
	Ok(identifiers)
}

/// Reads every CAS annotation of one work (tombstones included).
async fn liseur_annotations(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
) -> Result<Vec<ExportAnnotation>, AnnotationSyncError> {
	let backend = conn.get_database_backend();
	let rows = conn
		.query_all(statement(
			backend,
			"SELECT annotation_id, rev, seq, kind, locator, progression, excerpt,
                    color, body, client_ts, updated_at, deleted, deleted_at
             FROM liseur_sync_annotations
             WHERE user_id = $1 AND work_id = $2
             ORDER BY client_ts ASC, annotation_id ASC",
			vec![user_id.into(), work_id.into()],
		))
		.await?;

	let mut annotations = Vec::with_capacity(rows.len());
	for row in &rows {
		let kind: String = row.try_get("", "kind")?;
		let locator_raw: Option<String> = row.try_get("", "locator")?;
		let created: Option<DateTime<Utc>> = row
			.try_get::<Option<String>>("", "client_ts")?
			.and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
			.map(|value| value.with_timezone(&Utc));
		let updated: Option<DateTime<Utc>> = row
			.try_get::<Option<String>>("", "updated_at")?
			.and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
			.map(|value| value.with_timezone(&Utc));
		let deleted_at: Option<DateTime<Utc>> = row
			.try_get::<Option<String>>("", "deleted_at")?
			.and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
			.map(|value| value.with_timezone(&Utc));

		annotations.push(ExportAnnotation {
			id: row.try_get("", "annotation_id")?,
			kind: match kind.as_str() {
				"note" => ExportAnnotationKind::Note,
				"bookmark" => ExportAnnotationKind::Bookmark,
				_ => ExportAnnotationKind::Highlight,
			},
			origin: AnnotationOrigin::Liseur {
				rev: row.try_get("", "rev")?,
				seq: row.try_get("", "seq")?,
			},
			locator: locator_raw.and_then(|raw| serde_json::from_str(&raw).ok()),
			progression: row.try_get("", "progression")?,
			color: row
				.try_get::<Option<String>>("", "color")?
				.filter(|color| !color.is_empty()),
			excerpt: row
				.try_get::<Option<String>>("", "excerpt")?
				.filter(|excerpt| !excerpt.is_empty()),
			note: row
				.try_get::<Option<String>>("", "body")?
				.filter(|body| !body.is_empty()),
			created_at: created,
			updated_at: updated,
			deleted: row.try_get::<bool>("", "deleted")?,
			deleted_at,
		});
	}

	Ok(annotations)
}

/// `$N` placeholders are valid for both supported backends (SQLite accepts
/// `$`-prefixed parameters, matching the liseur-sync storage convention).
fn statement(backend: DbBackend, sql: &str, values: Vec<sea_orm::Value>) -> Statement {
	Statement::from_sql_and_values(backend, sql, values)
}
