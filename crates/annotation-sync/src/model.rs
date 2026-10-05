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
//!   included), standalone (`liseur:<work_id>`) when no readable edition
//!   matches or folded into the confirmed, visible native edition named by
//!   the record's `edition_sha` (verified links take priority when unbound),
//! - reading heads + reading sessions, rendered as a summary block.

use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use models::{
	domain::edition_pair::PairStatus,
	entity::{
		book_review, bookmark, liseur_sync_media_link, media, media_annotation,
		media_metadata, reading_head, reading_session,
		user::{AuthUser, LoginUser},
	},
	shared::{
		liseur_annotation_projection::{
			is_liseur_sync_projection_id, is_stump_native_annotation_id,
		},
		readium::ReadiumLocator,
	},
};
use rust_decimal::prelude::ToPrimitive;
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

/// One explicit review included in a book export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportReview {
	pub work_id: Option<String>,
	pub media_id: Option<String>,
	pub rating: i32,
	pub content: Option<String>,
	pub is_private: bool,
	pub created_at: DateTime<Utc>,
	pub updated_at: DateTime<Utc>,
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
	/// The current user's explicit work/edition review, if one exists.
	pub review: Option<ExportReview>,
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
			review: None,
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
	Liseur {
		rev: i64,
		seq: i64,
	},
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
	/// A moment in an audiobook, in milliseconds from the start of the
	/// publication: the anchor of a native note made while listening, whose
	/// locator names no resource. Liseur records have no time anchor.
	pub position_ms: Option<i64>,
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
/// Native books (every visible media with at least one annotation or bookmark)
/// are always rebuilt in full, folding only the liseur records assigned to
/// that edition: native rows are hard-deleted on removal, so a cursor-based
/// delta would miss deletions, and a partial rebuild would drop linked rows.
/// The annotated-media set is bounded by the user's actual annotation count, not
/// library size. Liseur works are incremental by CAS `seq` (a changed work
/// surfaces its book, linked or standalone, in full); deletions are
/// tombstones and therefore visible.
pub async fn build_export_batch(
	conn: &DatabaseConnection,
	user_id: &str,
	options: BuildOptions,
) -> Result<ExportBatch, AnnotationSyncError> {
	let Some(user) = LoginUser::find_by_id(user_id.to_owned())
		.into_model::<LoginUser>()
		.one(conn)
		.await?
		.map(AuthUser::from)
	else {
		return Ok(ExportBatch {
			user_id: user_id.to_owned(),
			liseur_from_seq: options.liseur_from_seq,
			liseur_high_water: 0,
			books: Vec::new(),
		});
	};
	let mut books: Vec<ExportBook> = Vec::new();
	let mut built: BTreeSet<String> = BTreeSet::new();

	// ---- native books ---------------------------------------------------
	let annotation_media_ids: Vec<String> = media_annotation::Entity::find()
		.filter(media_annotation::Column::UserId.eq(user_id))
		.select_only()
		.distinct()
		.column(media_annotation::Column::MediaId)
		.into_tuple::<String>()
		.all(conn)
		.await?
		.into_iter()
		.filter(|id| !is_liseur_sync_projection_id(id))
		.collect();
	let bookmark_media_ids: Vec<String> = bookmark::Entity::find()
		.filter(bookmark::Column::UserId.eq(user_id))
		.select_only()
		.distinct()
		.column(bookmark::Column::MediaId)
		.into_tuple::<String>()
		.all(conn)
		.await?
		.into_iter()
		.filter(|id| !is_liseur_sync_projection_id(id))
		.collect();

	let review_media_ids: Vec<String> = book_review::Entity::find()
		.filter(book_review::Column::UserId.eq(user_id))
		.filter(book_review::Column::MediaId.is_not_null())
		.select_only()
		.distinct()
		.column(book_review::Column::MediaId)
		.into_tuple::<Option<String>>()
		.all(conn)
		.await?
		.into_iter()
		.flatten()
		.collect();
	let review_work_ids: Vec<String> = book_review::Entity::find()
		.filter(book_review::Column::UserId.eq(user_id))
		.filter(book_review::Column::WorkId.is_not_null())
		.select_only()
		.distinct()
		.column(book_review::Column::WorkId)
		.into_tuple::<Option<String>>()
		.all(conn)
		.await?
		.into_iter()
		.flatten()
		.collect();

	let mut native_media_ids: BTreeSet<String> = BTreeSet::new();
	native_media_ids.extend(annotation_media_ids);
	native_media_ids.extend(bookmark_media_ids);
	native_media_ids.extend(review_media_ids);

	// ---- liseur CAS works ------------------------------------------------
	let (changed_work_ids, high_water) =
		liseur_changed_works(conn, user_id, options.liseur_from_seq).await?;

	// A work may have annotations on multiple editions. Route each record to
	// its own confirmed, visible edition, or retain it in a standalone book
	// when its edition is no longer available.
	let mut standalone_work_ids: BTreeSet<String> = BTreeSet::new();
	let reviewed_work_ids: BTreeSet<String> = review_work_ids.into_iter().collect();
	let work_ids: BTreeSet<String> = changed_work_ids
		.into_iter()
		.chain(reviewed_work_ids.iter().cloned())
		.collect();
	for work_id in work_ids {
		let links = export_links(conn, &user, &work_id).await?;
		let default = liseur_linked_media(&links, None);
		if reviewed_work_ids.contains(&work_id) {
			if let Some(media_id) = default {
				native_media_ids.insert(media_id.to_owned());
			}
		}
		let mut standalone = default.is_none();
		for annotation in liseur_annotations(conn, user_id, &work_id).await? {
			match liseur_linked_media(&links, annotation.edition_sha.as_deref()) {
				Some(media_id) => {
					native_media_ids.insert(media_id.to_owned());
				},
				None => standalone = true,
			}
		}
		if standalone {
			standalone_work_ids.insert(work_id);
		}
	}

	for media_id in native_media_ids {
		if let Some(book) = build_native_book(conn, &user, &media_id).await? {
			built.insert(book.key.clone());
			books.push(book);
		}
	}
	for work_id in standalone_work_ids {
		let book = build_liseur_work_book(conn, &user, &work_id).await?;
		if built.insert(book.key.clone()) {
			books.push(book);
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
	user: &AuthUser,
	media_id: &str,
) -> Result<Option<ExportBook>, AnnotationSyncError> {
	let user_id = user.id.as_str();
	let Some(media) = media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(media_id))
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::Status.eq(models::shared::enums::FileStatus::Ready))
		.one(conn)
		.await?
	else {
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
	let mut book = ExportBook::new(
		BookSource::Native {
			media_id: media_id.to_owned(),
		},
		title,
	);

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
		.filter(|annotation| !is_liseur_sync_projection_id(&annotation.id))
		.map(|annotation| {
			Ok(ExportAnnotation {
				id: annotation.id.clone(),
				kind: ExportAnnotationKind::Highlight,
				origin: AnnotationOrigin::Native,
				locator: Some(serde_json::to_value(&annotation.locator)?),
				progression: locator_progression(&annotation.locator),
				position_ms: annotation.position_ms,
				color: annotation.color.clone(),
				excerpt: annotation
					.locator
					.text
					.as_ref()
					.and_then(|text| text.highlight.clone())
					.filter(|highlight| !highlight.is_empty()),
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
		.filter(|row| !is_liseur_sync_projection_id(&row.id))
		.map(|row| {
			Ok(ExportBookmark {
				id: row.id.clone(),
				locator: row.locator.as_ref().map(serde_json::to_value).transpose()?,
				preview_content: row.preview_content.clone(),
				page: row.page,
				created_at: row.created_at,
			})
		})
		.collect::<Result<Vec<_>, serde_json::Error>>()?;

	book.reading = build_reading_summary(conn, user_id, media_id).await?;
	book.review = review_for_native(conn, user, media_id).await?;

	for work_id in liseur_works_for_media(conn, user, media_id).await? {
		fold_liseur_work(conn, user, &work_id, media_id, &mut book).await?;
	}

	Ok(Some(book))
}

/// Resolve the one review that belongs in a native book export. A confirmed
/// work review wins over a legacy media review; suggestions are not work
/// identity and therefore cannot move a review between editions.
async fn review_for_native(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_id: &str,
) -> Result<Option<ExportReview>, AnnotationSyncError> {
	let user_id = user.id.as_str();
	let links = liseur_sync_media_link::Entity::find()
		.filter(liseur_sync_media_link::Column::UserId.eq(user_id))
		.filter(liseur_sync_media_link::Column::MediaId.eq(media_id))
		.order_by_asc(liseur_sync_media_link::Column::CreatedAt)
		.order_by_asc(liseur_sync_media_link::Column::Id)
		.all(conn)
		.await?;
	for link in links {
		if PairStatus::from_stored(&link.pair_status) != PairStatus::Confirmed
			|| liseur_linked_media(&export_links(conn, user, &link.work_id).await?, None)
				!= Some(media_id)
		{
			continue;
		}
		if let Some(review) = book_review::Entity::find_for_work(user_id, &link.work_id)
			.one(conn)
			.await?
		{
			return Ok(Some(review.into()));
		}
	}
	Ok(book_review::Entity::find_for_media(user_id, media_id)
		.one(conn)
		.await?
		.map(Into::into))
}

fn review_from_model(model: book_review::Model) -> ExportReview {
	ExportReview {
		work_id: model.work_id,
		media_id: model.media_id,
		rating: model.rating,
		content: model.content,
		is_private: model.is_private,
		created_at: model.created_at.with_timezone(&Utc),
		updated_at: model.updated_at.with_timezone(&Utc),
	}
}

impl From<book_review::Model> for ExportReview {
	fn from(model: book_review::Model) -> Self {
		review_from_model(model)
	}
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
		last_read_at: head
			.as_ref()
			.map(|head| head.changed_at.with_timezone(&Utc)),
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
	user: &AuthUser,
	work_id: &str,
) -> Result<ExportBook, AnnotationSyncError> {
	let user_id = user.id.as_str();
	let (title, author) = liseur_work_meta(conn, user_id, work_id).await?;
	let mut book = ExportBook::new(
		BookSource::LiseurWork {
			work_id: work_id.to_owned(),
		},
		title,
	);
	if let Some(author) = author.filter(|author| !author.is_empty()) {
		book.authors.push(author);
	}
	let links = export_links(conn, user, work_id).await?;
	if liseur_linked_media(&links, None).is_none() {
		book.review = book_review::Entity::find_for_work(user_id, work_id)
			.one(conn)
			.await?
			.map(Into::into);
	}
	merge_annotations(conn, user_id, work_id, &links, None, &mut book).await?;
	Ok(book)
}

/// Folds one linked liseur work's annotations, author, and identifiers into a
/// native book without transferring locators from another edition.
async fn fold_liseur_work(
	conn: &DatabaseConnection,
	user: &AuthUser,
	work_id: &str,
	media_id: &str,
	book: &mut ExportBook,
) -> Result<(), AnnotationSyncError> {
	let user_id = user.id.as_str();
	let links = export_links(conn, user, work_id).await?;
	let (_, author) = liseur_work_meta(conn, user_id, work_id).await?;
	if let Some(author) = author.filter(|author| !author.is_empty()) {
		if !book.authors.contains(&author) {
			book.authors.push(author);
		}
	}
	let mut identifiers = liseur_work_identifiers(conn, user_id, work_id).await?;
	book.identifiers.append(&mut identifiers);
	sort_identifiers(&mut book.identifiers);
	merge_annotations(conn, user_id, work_id, &links, Some(media_id), book).await?;
	Ok(())
}

/// Appends only annotations assigned to this book in `(created_at, id)` order.
async fn merge_annotations(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
	links: &[ExportLink],
	media_id: Option<&str>,
	book: &mut ExportBook,
) -> Result<(), AnnotationSyncError> {
	let mut annotations: Vec<_> = liseur_annotations(conn, user_id, work_id)
		.await?
		.into_iter()
		.filter(|row| liseur_linked_media(links, row.edition_sha.as_deref()) == media_id)
		.map(|row| row.annotation)
		.collect();
	annotations.sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
	book.annotations.extend(annotations);
	Ok(())
}

fn sort_identifiers(identifiers: &mut Vec<ExportIdentifier>) {
	identifiers.sort_by(|a, b| (&a.scheme, &a.value).cmp(&(&b.scheme, &b.value)));
	identifiers.dedup_by(|a, b| a.scheme == b.scheme && a.value == b.value);
}

/// Whole-publication progression carried by a Readium locator, falling back
/// to the resource-local progression.
fn locator_progression(locator: &ReadiumLocator) -> Option<f64> {
	let locations = locator.locations.as_ref()?;
	locations
		.total_progression
		.or(locations.progression)
		.and_then(|value| value.to_f64())
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

/// Candidate links are user-scoped, confirmed, present, and visible under the
/// same library/age policy as ordinary media queries. A digest verified against
/// the file wins over an unverified manual pair; ties use link creation/id.
struct ExportLink {
	media_id: String,
	edition_sha: String,
	verified: bool,
}

async fn export_links(
	conn: &DatabaseConnection,
	user: &AuthUser,
	work_id: &str,
) -> Result<Vec<ExportLink>, AnnotationSyncError> {
	let links = liseur_sync_media_link::Entity::find()
		.filter(liseur_sync_media_link::Column::UserId.eq(&user.id))
		.filter(liseur_sync_media_link::Column::WorkId.eq(work_id))
		.order_by_asc(liseur_sync_media_link::Column::CreatedAt)
		.order_by_asc(liseur_sync_media_link::Column::Id)
		.all(conn)
		.await?;
	let candidates: Vec<_> = links
		.iter()
		.filter(|link| {
			PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
		})
		.map(|link| link.media_id.as_str())
		.collect();
	if candidates.is_empty() {
		return Ok(Vec::new());
	}
	let readable: HashSet<String> = media::Entity::find_for_user(user)
		.filter(media::Column::Id.is_in(candidates))
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::Status.eq(models::shared::enums::FileStatus::Ready))
		.select_only()
		.columns([media::Column::Id, media::Column::Extension])
		.into_tuple::<(String, String)>()
		.all(conn)
		.await?
		.into_iter()
		.filter(|(_, extension)| {
			!media::AUDIO_EXTENSIONS
				.iter()
				.any(|audio| extension.eq_ignore_ascii_case(audio))
		})
		.map(|(id, _)| id)
		.collect();
	let mut eligible: Vec<_> = links
		.into_iter()
		.filter(|link| {
			PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
				&& readable.contains(&link.media_id)
		})
		.map(|link| ExportLink {
			media_id: link.media_id,
			edition_sha: link.edition_sha,
			verified: link.resolution_status == "verified",
		})
		.collect();
	// Stable sort retains `(created_at, id)` for equally trusted links.
	eligible.sort_by_key(|link| !link.verified);
	Ok(eligible)
}

/// An annotation's known edition must match its own file. Without a digest,
/// use the best available readable edition; never attach ebook text to audio.
fn liseur_linked_media<'a>(
	links: &'a [ExportLink],
	edition_sha: Option<&str>,
) -> Option<&'a str> {
	match edition_sha.filter(|sha| !sha.is_empty()) {
		Some(sha) => links.iter().find(|link| link.edition_sha == sha),
		None => links.first(),
	}
	.map(|link| link.media_id.as_str())
}

/// Confirmed works linked to this readable media, in deterministic link order.
async fn liseur_works_for_media(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_id: &str,
) -> Result<Vec<String>, AnnotationSyncError> {
	let links = liseur_sync_media_link::Entity::find()
		.filter(liseur_sync_media_link::Column::UserId.eq(&user.id))
		.filter(liseur_sync_media_link::Column::MediaId.eq(media_id))
		.order_by_asc(liseur_sync_media_link::Column::CreatedAt)
		.order_by_asc(liseur_sync_media_link::Column::Id)
		.all(conn)
		.await?;
	Ok(links
		.into_iter()
		.filter(|link| {
			PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
		})
		.map(|link| link.work_id)
		.collect())
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
	identifiers.retain(|identifier| {
		!identifier.scheme.is_empty() && !identifier.value.is_empty()
	});
	sort_identifiers(&mut identifiers);
	Ok(identifiers)
}

struct LiseurExportAnnotation {
	edition_sha: Option<String>,
	annotation: ExportAnnotation,
}

/// Reads every CAS annotation of one work (tombstones included).
async fn liseur_annotations(
	conn: &DatabaseConnection,
	user_id: &str,
	work_id: &str,
) -> Result<Vec<LiseurExportAnnotation>, AnnotationSyncError> {
	let backend = conn.get_database_backend();
	let rows = conn
		.query_all(statement(
			backend,
			"SELECT annotation_id, rev, seq, edition_sha, kind, locator, progression, excerpt,
                    color, body, client_ts, updated_at, deleted, deleted_at
             FROM liseur_sync_annotations
             WHERE user_id = $1 AND work_id = $2
             ORDER BY client_ts ASC, annotation_id ASC",
			vec![user_id.into(), work_id.into()],
		))
		.await?;

	let mut annotations = Vec::with_capacity(rows.len());
	for row in &rows {
		let id: String = row.try_get("", "annotation_id")?;
		if is_stump_native_annotation_id(&id) {
			continue;
		}
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

		annotations.push(LiseurExportAnnotation {
			edition_sha: row.try_get("", "edition_sha")?,
			annotation: ExportAnnotation {
				id,
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
				position_ms: None,
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
			},
		});
	}

	Ok(annotations)
}

/// `$N` placeholders are valid for both supported backends (SQLite accepts
/// `$`-prefixed parameters, matching the liseur-sync storage convention).
fn statement(backend: DbBackend, sql: &str, values: Vec<sea_orm::Value>) -> Statement {
	Statement::from_sql_and_values(backend, sql, values)
}

#[cfg(test)]
mod tests {
	use ::tests::{db::test_database, fake_data};
	use chrono::TimeZone;
	use models::{
		domain::reading_state::SourceProtocol,
		entity::{bookmark, media_annotation, media_metadata, reading_head},
		shared::liseur_annotation_projection::{
			liseur_sync_projection_id, stump_native_annotation_id,
		},
		shared::readium::{ReadiumLocation, ReadiumLocator, ReadiumText},
	};
	use rust_decimal::Decimal;
	use sea_orm::{ActiveValue::Set, ConnectionTrait, DbBackend, Schema};

	use super::*;

	const LISEUR_DDL: &[&str] = &[
		"CREATE TABLE liseur_sync_counters (user_id TEXT PRIMARY KEY, op_seq BIGINT NOT NULL DEFAULT 0, annotation_seq BIGINT NOT NULL DEFAULT 0)",
		"CREATE TABLE liseur_sync_works (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, title TEXT NOT NULL, author TEXT NOT NULL)",
		"CREATE TABLE liseur_sync_aliases (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, kind TEXT NOT NULL, value TEXT NOT NULL, work_id TEXT NOT NULL)",
		"CREATE TABLE liseur_sync_media_links (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, media_id TEXT NOT NULL, work_id TEXT NOT NULL, edition_sha TEXT NOT NULL, resolution_status TEXT NOT NULL, created_at TEXT NOT NULL, pair_status TEXT NOT NULL DEFAULT 'confirmed', pair_evidence TEXT)",
		"CREATE TABLE book_reviews (id TEXT PRIMARY KEY, work_id TEXT, media_id TEXT, user_id TEXT NOT NULL, rating INTEGER NOT NULL, content TEXT, is_private BOOLEAN NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
		"CREATE TABLE liseur_sync_annotations (
			row_id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT NOT NULL, annotation_id TEXT NOT NULL,
			rev BIGINT NOT NULL, seq BIGINT NOT NULL, work_id TEXT NOT NULL, edition_sha TEXT, kind TEXT NOT NULL,
			locator TEXT, progression DOUBLE, excerpt TEXT NOT NULL, color TEXT NOT NULL, body TEXT NOT NULL,
			device_id TEXT NOT NULL, client_ts TEXT NOT NULL, updated_at TEXT NOT NULL,
			deleted BOOLEAN NOT NULL DEFAULT FALSE, deleted_at TEXT, payload TEXT NOT NULL)",
	];

	async fn seeded_db() -> (DatabaseConnection, String, String) {
		let conn = test_database().await;
		let schema = Schema::new(DbBackend::Sqlite);
		for stmt in [
			schema.create_table_from_entity(media_annotation::Entity),
			schema.create_table_from_entity(bookmark::Entity),
		] {
			conn.execute(conn.get_database_backend().build(&stmt))
				.await
				.unwrap();
		}
		for sql in LISEUR_DDL {
			conn.execute(Statement::from_string(DbBackend::Sqlite, *sql))
				.await
				.unwrap();
		}

		let user = fake_data::User::new("reader").insert(&conn).await;
		let library = fake_data::Library::default().insert(&conn).await;
		let series = fake_data::Series {
			library_id: Some(library.id),
			..Default::default()
		}
		.insert(&conn)
		.await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("m1".to_string()),
			name: Some("dune.epub".to_string()),
			..Default::default()
		}
		.insert(&conn)
		.await;
		media_metadata::ActiveModel {
			media_id: Set(Some(media.id.clone())),
			title: Set(Some("Dune".to_string())),
			writers: Set(Some("Frank Herbert, Brian Herbert".to_string())),
			identifier_isbn: Set(Some("9780441172696".to_string())),
			identifier_uuid: Set(Some("".to_string())),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();
		(conn, user.id, media.id)
	}

	fn ts(secs: i64) -> DateTime<Utc> {
		Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
	}

	async fn insert_native_annotation(
		conn: &DatabaseConnection,
		user_id: &str,
		media_id: &str,
		id: &str,
		created: DateTime<Utc>,
		note: Option<&str>,
		color: Option<&str>,
	) {
		let locator = ReadiumLocator {
			href: "ch1.xhtml".to_string(),
			locations: Some(ReadiumLocation {
				fragments: None,
				progression: Some(Decimal::new(5, 1)),
				position: None,
				total_progression: Some(Decimal::new(25, 2)),
				css_selector: None,
				partial_cfi: None,
			}),
			text: Some(ReadiumText {
				after: None,
				before: None,
				highlight: Some(format!("highlight {id}")),
			}),
			..Default::default()
		};
		let inserted = media_annotation::ActiveModel {
			id: Set(id.to_string()),
			locator: Set(locator),
			annotation_text: Set(note.map(str::to_string)),
			color: Set(color.map(str::to_owned)),
			media_id: Set(media_id.to_string()),
			user_id: Set(user_id.to_string()),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
		// `before_save` stamps `created_at`; pin it for deterministic ordering.
		let mut active: media_annotation::ActiveModel = inserted.into();
		active.created_at = Set(created);
		active.update(conn).await.unwrap();
	}

	async fn exec(conn: &DatabaseConnection, sql: &str, values: Vec<sea_orm::Value>) {
		conn.execute(statement(DbBackend::Sqlite, sql, values))
			.await
			.unwrap();
	}

	async fn insert_liseur_annotation(
		conn: &DatabaseConnection,
		user_id: &str,
		work_id: &str,
		id: &str,
		seq: i64,
		kind: &str,
		deleted: bool,
	) {
		exec(
			conn,
			"INSERT INTO liseur_sync_annotations
				(user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator, progression,
				 excerpt, color, body, device_id, client_ts, updated_at, deleted, deleted_at, payload)
			 VALUES ($1, $2, 1, $3, $4, NULL, $5, '{\"cfi\":\"/6/4\"}', 0.5, 'excerpt', 'yellow', '',
				 'd1', $6, $6, $7, NULL, '{}')",
			vec![
				user_id.into(),
				id.into(),
				seq.into(),
				work_id.into(),
				kind.into(),
				ts(seq * 10).to_rfc3339().into(),
				deleted.into(),
			],
		)
		.await;
		exec(
			conn,
			"UPDATE liseur_sync_counters SET annotation_seq = MAX(annotation_seq, $1) WHERE user_id = $2",
			vec![seq.into(), user_id.into()],
		)
		.await;
	}

	#[tokio::test]
	async fn builds_native_and_liseur_books_in_canonical_order() {
		let (conn, user_id, media_id) = seeded_db().await;
		exec(
			&conn,
			"INSERT INTO liseur_sync_counters (user_id) VALUES ($1)",
			vec![user_id.clone().into()],
		)
		.await;

		// Native rows inserted out of chronological order.
		insert_native_annotation(&conn, &user_id, &media_id, "n2", ts(20), None, None)
			.await;
		insert_native_annotation(
			&conn,
			&user_id,
			&media_id,
			"n1",
			ts(10),
			Some("first"),
			Some("red"),
		)
		.await;
		insert_liseur_annotation(
			&conn,
			&user_id,
			"w-linked",
			&stump_native_annotation_id("annotation", "n1"),
			1,
			"highlight",
			false,
		)
		.await;
		insert_native_annotation(
			&conn,
			&user_id,
			&media_id,
			&liseur_sync_projection_id(&user_id, "l1"),
			ts(30),
			None,
			Some("yellow"),
		)
		.await;
		bookmark::ActiveModel {
			id: Set("b1".to_string()),
			preview_content: Set(Some("preview".to_string())),
			locator: Set(None),
			page: Set(Some(7)),
			media_id: Set(media_id.clone()),
			user_id: Set(user_id.clone()),
			created_at: Set(ts(30)),
			position_ms: Set(None),
		}
		.insert(&conn)
		.await
		.unwrap();
		fake_data::ReadingSession::completed(&media_id, &user_id)
			.insert(&conn)
			.await;
		// The session fixture materializes the head like every writer does; a
		// later Kobo write is what this book's summary must report.
		reading_head::ActiveModel {
			user_id: Set(user_id.clone()),
			media_id: Set(media_id.clone()),
			locator: Set(None),
			progression: Set(0.42),
			page: Set(Some(100)),
			completed: Set(false),
			updated_at: Set(ts(40).fixed_offset()),
			created_at: Set(ts(40).fixed_offset()),
			changed_at: Set(ts(41).fixed_offset()),
			source_protocol: Set(SourceProtocol::Kobo),
			source_device_id: Set(None),
			revision: Set(1),
			event_id: Set(1),
			position_ms: Set(None),
			track_index: Set(None),
		}
		.update(&conn)
		.await
		.unwrap();

		// A liseur work linked to the media (folded) and a standalone one.
		for (work, title, author) in [
			("w-linked", "Dune (liseur)", "Frank Herbert"),
			("w-alone", "Standalone", "Someone Else"),
		] {
			exec(
				&conn,
				"INSERT INTO liseur_sync_works (id, user_id, title, author) VALUES ($1, $2, $3, $4)",
				vec![work.into(), user_id.clone().into(), title.into(), author.into()],
			)
			.await;
		}
		exec(
			&conn,
			"INSERT INTO liseur_sync_media_links (id, user_id, media_id, work_id, edition_sha, resolution_status, created_at)
			 VALUES ('link-1', $1, $2, 'w-linked', 'sha', 'verified', '2026-01-01T00:00:00Z')",
			vec![user_id.clone().into(), media_id.clone().into()],
		)
		.await;
		for (id, kind, value, work) in [
			("al-1", "isbn", "9780441172696", "w-linked"),
			("al-2", "goodreads", "234", "w-linked"),
			("al-3", "isbn", "111", "w-alone"),
		] {
			exec(
				&conn,
				"INSERT INTO liseur_sync_aliases (id, user_id, kind, value, work_id) VALUES ($1, $2, $3, $4, $5)",
				vec![id.into(), user_id.clone().into(), kind.into(), value.into(), work.into()],
			)
			.await;
		}
		insert_liseur_annotation(
			&conn,
			&user_id,
			"w-linked",
			"l1",
			1,
			"highlight",
			false,
		)
		.await;
		insert_liseur_annotation(&conn, &user_id, "w-linked", "l2", 2, "note", true)
			.await;
		insert_liseur_annotation(&conn, &user_id, "w-alone", "s1", 3, "bookmark", false)
			.await;

		let batch = build_export_batch(&conn, &user_id, BuildOptions::default())
			.await
			.unwrap();
		assert_eq!(batch.liseur_high_water, 3);
		assert_eq!(
			batch
				.books
				.iter()
				.map(|book| book.key.as_str())
				.collect::<Vec<_>>(),
			vec!["liseur:w-alone", "native:m1"]
		);

		let native = &batch.books[1];
		assert_eq!(native.title, "Dune");
		assert_eq!(native.authors, vec!["Frank Herbert", "Brian Herbert"]);
		assert_eq!(
			native
				.identifiers
				.iter()
				.map(|id| format!("{}:{}", id.scheme, id.value))
				.collect::<Vec<_>>(),
			vec!["goodreads:234", "isbn:9780441172696"],
			"identifiers are merged, sorted, deduplicated, and empty ones dropped"
		);
		assert_eq!(
			native
				.annotations
				.iter()
				.map(|annotation| annotation.id.as_str())
				.collect::<Vec<_>>(),
			vec!["n1", "n2", "l1", "l2"],
			"native rows by created_at, then the folded liseur rows"
		);
		let n1 = &native.annotations[0];
		assert_eq!(n1.excerpt.as_deref(), Some("highlight n1"));
		assert_eq!(n1.note.as_deref(), Some("first"));
		assert_eq!(n1.progression, Some(0.25));
		assert_eq!(n1.color.as_deref(), Some("red"));
		assert_eq!(native.annotations[2].id, "l1");
		assert_eq!(native.annotations[2].color.as_deref(), Some("yellow"));
		assert_eq!(n1.origin, AnnotationOrigin::Native);
		let l2 = &native.annotations[3];
		assert!(l2.deleted);
		assert_eq!(l2.kind, ExportAnnotationKind::Note);
		assert_eq!(l2.origin, AnnotationOrigin::Liseur { rev: 1, seq: 2 });
		assert_eq!(native.bookmarks.len(), 1);
		assert_eq!(native.bookmarks[0].page, Some(7));
		let reading = native.reading.as_ref().unwrap();
		assert_eq!(reading.progression, Some(0.42));
		assert_eq!(reading.page, Some(100));
		assert_eq!(reading.source_protocol.as_deref(), Some("kobo"));
		assert_eq!(reading.session_count, 1);
		assert!(reading.finished);
		assert_eq!(reading.last_read_at, Some(ts(41)));

		let alone = &batch.books[0];
		assert_eq!(alone.title, "Standalone");
		assert_eq!(alone.authors, vec!["Someone Else"]);
		assert_eq!(alone.annotations.len(), 1);
		assert_eq!(alone.annotations[0].kind, ExportAnnotationKind::Bookmark);
		assert!(alone.reading.is_none());

		// Incremental: nothing above the cursor still rebuilds the native
		// book in full (linked liseur rows included) and drops the untouched
		// standalone work.
		let batch =
			build_export_batch(&conn, &user_id, BuildOptions { liseur_from_seq: 3 })
				.await
				.unwrap();
		assert_eq!(batch.liseur_high_water, 3);
		assert_eq!(
			batch
				.books
				.iter()
				.map(|book| book.key.as_str())
				.collect::<Vec<_>>(),
			vec!["native:m1"]
		);
		assert_eq!(batch.books[0].annotations.len(), 4);

		// A changed linked work surfaces the media even without native rows.
		media_annotation::Entity::delete_many()
			.exec(&conn)
			.await
			.unwrap();
		bookmark::Entity::delete_many().exec(&conn).await.unwrap();
		insert_liseur_annotation(
			&conn,
			&user_id,
			"w-linked",
			"l3",
			4,
			"highlight",
			false,
		)
		.await;
		let batch =
			build_export_batch(&conn, &user_id, BuildOptions { liseur_from_seq: 3 })
				.await
				.unwrap();
		assert_eq!(batch.liseur_high_water, 4);
		assert_eq!(batch.books.len(), 1);
		assert_eq!(batch.books[0].key, "native:m1");
		assert_eq!(
			batch.books[0]
				.annotations
				.iter()
				.map(|annotation| annotation.id.as_str())
				.collect::<Vec<_>>(),
			vec!["l1", "l2", "l3"]
		);
	}

	#[tokio::test]
	async fn exports_only_to_visible_confirmed_readable_editions() {
		let (conn, user_id, old_audio) = seeded_db().await;
		let other_user = fake_data::User::new("other-reader").insert(&conn).await;
		let hidden_library = fake_data::Library::default().insert(&conn).await;
		let series = fake_data::Series {
			library_id: Some(hidden_library.id.clone()),
			..Default::default()
		}
		.insert(&conn)
		.await;
		for (id, extension) in [
			("m-suggested", "epub"),
			("m-old-ebook", "epub"),
			("m-missing", "epub"),
			("m-retired", "epub"),
			("m-hidden", "epub"),
			("m-edition-a", "epub"),
			("m-edition-b", "epub"),
			("m-other-user", "epub"),
		] {
			fake_data::Media {
				series_id: series.id.clone(),
				id: Some(id.to_owned()),
				name: Some(format!("{id}.{extension}")),
				extension: Some(extension.to_owned()),
				..Default::default()
			}
			.insert(&conn)
			.await;
		}
		exec(
			&conn,
			"UPDATE media SET extension = 'm4b' WHERE id = $1",
			vec![old_audio.clone().into()],
		)
		.await;
		exec(
			&conn,
			"UPDATE media SET deleted_at = '2026-01-01T00:00:00Z' WHERE id = 'm-retired'",
			vec![],
		)
		.await;
		exec(
			&conn,
			"UPDATE media SET status = 'MISSING' WHERE id = 'm-missing'",
			vec![],
		)
		.await;
		exec(
			&conn,
			"INSERT INTO library_exclusions (user_id, library_id) SELECT $1, library_id FROM series WHERE id = $2",
			vec![user_id.clone().into(), series.id.clone().into()],
		)
		.await;
		// Keep all but the hidden candidate outside the excluded library.
		let visible_library = fake_data::Library::default().insert(&conn).await;
		let visible_series = fake_data::Series {
			library_id: Some(visible_library.id),
			..Default::default()
		}
		.insert(&conn)
		.await;
		exec(
			&conn,
			"UPDATE media SET series_id = $1 WHERE id IN ('m-edition-a', 'm-edition-b', 'm-suggested', 'm-retired', 'm-other-user', 'm-old-ebook', 'm-missing')",
			vec![visible_series.id.into()],
		)
		.await;
		for (id, owner, media_id, sha, resolution, pair, created) in [
			(
				"old",
				user_id.as_str(),
				old_audio.as_str(),
				"sha-audio",
				"unverified",
				"confirmed",
				"01",
			),
			(
				"old-ebook",
				user_id.as_str(),
				"m-old-ebook",
				"sha-old",
				"unverified",
				"confirmed",
				"02",
			),
			(
				"guess",
				user_id.as_str(),
				"m-suggested",
				"sha-guess",
				"verified",
				"suggested",
				"03",
			),
			(
				"gone",
				user_id.as_str(),
				"m-retired",
				"sha-gone",
				"verified",
				"confirmed",
				"04",
			),
			(
				"missing",
				user_id.as_str(),
				"m-missing",
				"sha-missing",
				"verified",
				"confirmed",
				"05",
			),
			(
				"hidden",
				user_id.as_str(),
				"m-hidden",
				"sha-hidden",
				"verified",
				"confirmed",
				"06",
			),
			(
				"a",
				user_id.as_str(),
				"m-edition-a",
				"sha-a",
				"verified",
				"confirmed",
				"07",
			),
			(
				"b",
				user_id.as_str(),
				"m-edition-b",
				"sha-b",
				"verified",
				"confirmed",
				"08",
			),
			(
				"foreign",
				other_user.id.as_str(),
				"m-other-user",
				"sha-b",
				"verified",
				"confirmed",
				"00",
			),
		] {
			exec(
				&conn,
				"INSERT INTO liseur_sync_media_links
				 (id, user_id, media_id, work_id, edition_sha, resolution_status, pair_status, created_at)
				 VALUES ($1, $2, $3, 'w', $4, $5, $6, $7)",
				vec![id.into(), owner.into(), media_id.into(), sha.into(), resolution.into(), pair.into(), created.into()],
			)
			.await;
		}
		for (id, sha, seq) in [
			("on-a", "sha-a", 1),
			("on-b", "sha-b", 2),
			("unknown", "sha-unknown", 3),
			("unbound", "", 4),
			("on-audio", "sha-audio", 5),
			("on-guess", "sha-guess", 6),
			("on-gone", "sha-gone", 7),
			("on-hidden", "sha-hidden", 8),
			("on-missing", "sha-missing", 9),
		] {
			insert_liseur_annotation(&conn, &user_id, "w", id, seq, "highlight", false)
				.await;
			exec(
				&conn,
				"UPDATE liseur_sync_annotations SET edition_sha = $1 WHERE user_id = $2 AND annotation_id = $3",
				vec![sha.into(), user_id.clone().into(), id.into()],
			)
			.await;
		}
		insert_liseur_annotation(
			&conn,
			&other_user.id,
			"w",
			"foreign-note",
			10,
			"highlight",
			false,
		)
		.await;
		for (id, owner, content) in [
			("r-own", user_id.as_str(), "own review"),
			("r-foreign", other_user.id.as_str(), "foreign review"),
		] {
			exec(
				&conn,
				"INSERT INTO book_reviews
				 (id, work_id, media_id, user_id, rating, content, is_private, created_at, updated_at)
				 VALUES ($1, 'w', NULL, $2, 5, $3, TRUE, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
				vec![id.into(), owner.into(), content.into()],
			)
			.await;
		}
		insert_native_annotation(
			&conn,
			&user_id,
			"m-hidden",
			"hidden-native",
			ts(1),
			None,
			None,
		)
		.await;
		let batch = build_export_batch(&conn, &user_id, BuildOptions::default())
			.await
			.unwrap();
		let ids = |key: &str| -> Vec<&str> {
			batch.books.iter().find(|book| book.key == key).map_or_else(
				Vec::new,
				|book| {
					book.annotations
						.iter()
						.map(|annotation| annotation.id.as_str())
						.collect()
				},
			)
		};
		assert_eq!(ids("native:m-edition-a"), ["on-a", "unbound"]);
		assert_eq!(ids("native:m-edition-b"), ["on-b"]);
		assert_eq!(
			ids("liseur:w"),
			[
				"unknown",
				"on-audio",
				"on-guess",
				"on-gone",
				"on-hidden",
				"on-missing"
			]
		);
		let review = |key: &str| {
			batch
				.books
				.iter()
				.find(|book| book.key == key)
				.and_then(|book| book.review.as_ref())
				.and_then(|review| review.content.as_deref())
		};
		assert_eq!(review("native:m-edition-a"), Some("own review"));
		assert_eq!(review("native:m-edition-b"), None);
		assert_eq!(review("liseur:w"), None);
		assert!(batch
			.books
			.iter()
			.flat_map(|book| &book.annotations)
			.all(|annotation| annotation.id != "foreign-note"
				&& annotation.id != "hidden-native"));
		for media_id in [
			old_audio.as_str(),
			"m-old-ebook",
			"m-suggested",
			"m-retired",
			"m-missing",
			"m-hidden",
			"m-other-user",
		] {
			assert!(
				ids(&format!("native:{media_id}")).is_empty(),
				"unusable edition {media_id}"
			);
		}
	}

	#[tokio::test]
	async fn user_without_annotations_yields_empty_batch() {
		let (conn, user_id, _) = seeded_db().await;
		let batch = build_export_batch(&conn, &user_id, BuildOptions::default())
			.await
			.unwrap();
		assert!(batch.books.is_empty());
		assert_eq!(batch.liseur_high_water, 0);
	}

	/// A note made in an audiobook is exported with its time anchor; its
	/// locator names no resource and no passage, so nothing text-shaped is
	/// inferred from it.
	#[tokio::test]
	async fn audiobook_notes_export_their_time_anchor() {
		let (conn, user_id, media_id) = seeded_db().await;
		media_annotation::ActiveModel {
			id: Set("audio-1".to_string()),
			locator: Set(ReadiumLocator {
				chapter_title: "The Litany".to_string(),
				locations: Some(ReadiumLocation {
					fragments: None,
					progression: None,
					position: None,
					total_progression: Some(Decimal::new(2094, 4)),
					css_selector: None,
					partial_cfi: None,
				}),
				r#type: String::new(),
				..Default::default()
			}),
			position_ms: Set(Some(754_000)),
			annotation_text: Set(Some("Fear is the mind-killer".to_string())),
			media_id: Set(media_id.clone()),
			user_id: Set(user_id.clone()),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		let batch = build_export_batch(&conn, &user_id, BuildOptions::default())
			.await
			.unwrap();
		let annotation = &batch.books[0].annotations[0];
		assert_eq!(annotation.id, "audio-1");
		assert_eq!(annotation.position_ms, Some(754_000));
		assert_eq!(annotation.progression, Some(0.2094));
		assert_eq!(annotation.excerpt, None);
		assert_eq!(annotation.note.as_deref(), Some("Fear is the mind-killer"));
		assert_eq!(
			annotation.locator.as_ref().unwrap()["href"],
			serde_json::json!("")
		);
	}
}
