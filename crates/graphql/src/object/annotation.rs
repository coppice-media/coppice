use std::collections::{BTreeSet, HashMap};

use async_graphql::{Enum, Result, SimpleObject, ID};
use chrono::{DateTime, Utc};
use models::{
	entity::{bookmark, media, media_annotation, media_metadata, series, user::AuthUser},
	shared::{
		enums::{DeviceCredentialKind, DeviceKind},
		readium::ReadiumLocator,
	},
};
use num_traits::cast::ToPrimitive;
use sea_orm::{prelude::*, DatabaseConnection, FromQueryResult, QuerySelect};
use stump_annotation_sync::SinkDescriptor;
use stump_core::annotation_sync::SinkStatusRow;

use crate::{
	input::annotation::AnnotationFilterInput, object::ingest::IngestSettingDefinition,
	pagination::OffsetPagination, utils::db_statement,
};

/// A compiled-in annotation export sink and its setting schema.
#[derive(Debug, Clone, SimpleObject)]
pub struct AnnotationSink {
	pub id: String,
	pub name: String,
	pub description: String,
	pub settings: Vec<IngestSettingDefinition>,
}

impl From<SinkDescriptor> for AnnotationSink {
	fn from(descriptor: SinkDescriptor) -> Self {
		Self {
			id: descriptor.id.to_owned(),
			name: descriptor.name.to_owned(),
			description: descriptor.description.to_owned(),
			settings: descriptor.settings.into_iter().map(Into::into).collect(),
		}
	}
}

/// Per-sink configuration state for one user.
#[derive(Debug, Clone, SimpleObject)]
pub struct AnnotationSinkStatus {
	pub sink_id: String,
	pub enabled: bool,
	pub last_run_at: Option<sea_orm::prelude::DateTimeWithTimeZone>,
	pub last_error: Option<String>,
}

impl From<SinkStatusRow> for AnnotationSinkStatus {
	fn from(row: SinkStatusRow) -> Self {
		Self {
			sink_id: row.sink_id,
			enabled: row.enabled,
			last_run_at: row.last_run_at,
			last_error: row.last_error,
		}
	}
}

/// The annotation sync state for one user.
#[derive(Debug, Clone, SimpleObject)]
pub struct AnnotationSyncStatus {
	pub user_id: String,
	/// A debounced export is scheduled for this user.
	pub pending: bool,
	pub sinks: Vec<AnnotationSinkStatus>,
}

/// What an annotation is.
///
/// A native `media_annotations` row always carries a Readium locator, so the
/// kind follows the locator's `text.highlight`: a row with that selected
/// passage is a `HIGHLIGHT`, a row carrying only the user's text is a `NOTE`.
/// Native `bookmarks` rows and liseur `bookmark` records are `BOOKMARK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum AnnotationKind {
	Highlight,
	Note,
	Bookmark,
}

/// The book an annotation belongs to.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct AnnotationBook {
	/// Stable grouping key — `native:<media_id>` or `liseur:<work_id>`, the
	/// same key the annotation-sync export uses for its per-book file.
	pub key: String,
	/// The Stump book, when the annotation anchors to one. `null` for a
	/// liseur work with no link to a book this user can see; those
	/// annotations are listed but cannot be opened in a reader.
	pub media_id: Option<ID>,
	pub title: String,
	pub authors: Vec<String>,
	pub series_id: Option<ID>,
	pub series_name: Option<String>,
	pub library_id: Option<ID>,
	/// The book's file extension, so a client can pick a reader lane
	pub extension: Option<String>,
}

/// One highlight, note, or bookmark, wherever it came from.
#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct AnnotationEntry {
	pub id: ID,
	pub kind: AnnotationKind,
	/// Where the annotation came from. Native rows carry no per-row device,
	/// so they report `WEB` — the native lane, i.e. this server's readers and
	/// GraphQL API. A liseur-sync CAS record reports the kind of the device
	/// that pushed it (`KOBO` for NickelStump on a Kobo, `KOREADER`,
	/// `LISEUR`), falling back to `LISEUR` when that device is no longer
	/// registered.
	pub source: DeviceKind,
	pub source_device_id: Option<ID>,
	pub source_device_name: Option<String>,
	/// Whether `updateAnnotation`/`deleteAnnotation` accept this row. Only
	/// native rows are editable here: a liseur CAS record is owned by its
	/// device and replicated with compare-and-set revisions.
	pub editable: bool,
	/// The chapter the anchor names, when the locator carries one
	pub chapter_title: Option<String>,
	/// The publication-relative resource the anchor points into
	pub href: Option<String>,
	/// The in-resource fragment id, when the anchor carries one
	pub fragment: Option<String>,
	/// The page, for a paged book's bookmark or a positioned locator
	pub page: Option<i32>,
	/// Whole-publication progression in `0..=1`, when known
	pub progression: Option<f64>,
	/// The selected passage: the locator's `text.highlight`, the liseur
	/// `excerpt`, or a bookmark's preview text
	pub excerpt: Option<String>,
	/// The user's own note, distinct from the selected passage
	pub note: Option<String>,
	/// Liseur palette token (`yellow`, `green`, …); native rows have no colour
	pub color: Option<String>,
	pub created_at: Option<DateTime<Utc>>,
	pub updated_at: Option<DateTime<Utc>>,
	pub book: AnnotationBook,
}

/// One page of the cross-book annotation hub.
#[derive(Debug, Clone, SimpleObject)]
pub struct AnnotationPage {
	/// Book-contiguous: books ordered by their most recent annotation
	/// (newest first), annotations inside a book by `(createdAt, id)`
	/// ascending, so a page can be grouped by `book.key` as it arrives.
	pub items: Vec<AnnotationEntry>,
	/// Matching annotations across every book, before pagination
	pub total: i64,
	/// Distinct books among the matches
	pub book_count: i64,
	pub has_next: bool,
}

#[derive(Debug, FromQueryResult)]
struct BookRow {
	id: String,
	name: String,
	extension: String,
	series_id: Option<String>,
	series_name: Option<String>,
	library_id: Option<String>,
	title: Option<String>,
	writers: Option<String>,
}

impl From<BookRow> for AnnotationBook {
	fn from(row: BookRow) -> Self {
		Self {
			key: format!("native:{}", row.id),
			media_id: Some(ID(row.id)),
			title: row
				.title
				.filter(|title| !title.is_empty())
				.unwrap_or(row.name),
			authors: split_authors(row.writers.as_deref()),
			series_id: row.series_id.map(ID),
			series_name: row.series_name,
			library_id: row.library_id.map(ID),
			extension: Some(row.extension),
		}
	}
}

#[derive(Debug, FromQueryResult)]
struct LiseurRow {
	annotation_id: String,
	work_id: String,
	kind: String,
	locator: Option<String>,
	progression: Option<f64>,
	excerpt: Option<String>,
	color: Option<String>,
	body: Option<String>,
	device_id: String,
	client_ts: String,
	updated_at: String,
	media_id: Option<String>,
	work_title: Option<String>,
	work_author: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct LiseurDeviceRow {
	liseur_device_id: String,
	device_id: String,
	name: String,
	kind: DeviceKind,
}

/// The device a liseur CAS record was pushed from.
#[derive(Debug, Clone)]
struct SourceDevice {
	id: String,
	name: String,
	kind: DeviceKind,
}

/// Native `media_metadata.writers` is one comma-separated string.
fn split_authors(writers: Option<&str>) -> Vec<String> {
	writers
		.unwrap_or_default()
		.split(',')
		.map(str::trim)
		.filter(|author| !author.is_empty())
		.map(str::to_owned)
		.collect()
}

/// Whole-publication progression, falling back to the resource-local one —
/// the same rule the annotation-sync export applies.
fn locator_progression(locator: &ReadiumLocator) -> Option<f64> {
	let locations = locator.locations.as_ref()?;
	locations
		.total_progression
		.or(locations.progression)
		.and_then(|value| value.to_f64())
}

fn locator_fragment(locator: &ReadiumLocator) -> Option<String> {
	locator
		.locations
		.as_ref()?
		.fragments
		.as_ref()?
		.first()
		.filter(|fragment| !fragment.is_empty())
		.cloned()
}

fn non_empty(value: Option<String>) -> Option<String> {
	value.filter(|value| !value.trim().is_empty())
}

fn parse_ts(value: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(value)
		.ok()
		.map(|at| at.with_timezone(&Utc))
}

/// A liseur locator is opaque reader-native JSON that the server replays
/// verbatim. Anchor fields are read only when it happens to be Readium
/// shaped (NickelStump and Liseur both send that shape); anything else keeps
/// its anchor fields null rather than guessing.
fn liseur_anchor(locator: Option<&str>) -> Option<ReadiumLocator> {
	serde_json::from_str::<ReadiumLocator>(locator?).ok()
}

impl AnnotationPage {
	/// The user's highlights, notes, and bookmarks across every source.
	///
	/// Native rows come from `media_annotations`/`bookmarks`, liseur-sync CAS
	/// records from `liseur_sync_annotations` (tombstones excluded), folded
	/// onto the Stump book their work is linked to. Book scoping, the `since`
	/// bound on native rows, and library visibility are SQL-side; the source
	/// and text filters run on the assembled entries because a liseur row's
	/// source is only known after its device is resolved, and a native row's
	/// selected passage lives inside the locator JSON.
	pub async fn fetch(
		conn: &DatabaseConnection,
		user: &AuthUser,
		filter: &AnnotationFilterInput,
		pagination: &OffsetPagination,
	) -> Result<Self> {
		let liseur = load_liseur_rows(conn, &user.id, filter).await?;
		let mut media_ids = BTreeSet::new();
		if filter.wants(AnnotationKind::Highlight) || filter.wants(AnnotationKind::Note) {
			media_ids.extend(
				media_annotation::Entity::find()
					.filter(media_annotation::Column::UserId.eq(&user.id))
					.select_only()
					.column(media_annotation::Column::MediaId)
					.distinct()
					.into_tuple::<String>()
					.all(conn)
					.await?,
			);
		}
		if filter.wants(AnnotationKind::Bookmark) {
			media_ids.extend(
				bookmark::Entity::find()
					.filter(bookmark::Column::UserId.eq(&user.id))
					.select_only()
					.column(bookmark::Column::MediaId)
					.distinct()
					.into_tuple::<String>()
					.all(conn)
					.await?,
			);
		}
		media_ids.extend(liseur.iter().filter_map(|row| row.media_id.clone()));

		let books = load_books(conn, user, filter, media_ids).await?;
		let visible: Vec<String> = books.keys().cloned().collect();

		let mut entries = Vec::new();
		if !visible.is_empty() {
			if filter.wants(AnnotationKind::Highlight)
				|| filter.wants(AnnotationKind::Note)
			{
				let mut query = media_annotation::Entity::find()
					.filter(media_annotation::Column::UserId.eq(&user.id))
					.filter(media_annotation::Column::MediaId.is_in(visible.clone()));
				if let Some(since) = filter.since {
					query = query.filter(media_annotation::Column::CreatedAt.gte(since));
				}
				for row in query.all(conn).await? {
					let Some(book) = books.get(&row.media_id) else {
						continue;
					};
					let excerpt = non_empty(
						row.locator
							.text
							.as_ref()
							.and_then(|text| text.highlight.clone()),
					);
					let kind = if excerpt.is_some() {
						AnnotationKind::Highlight
					} else {
						AnnotationKind::Note
					};
					if !filter.wants(kind) {
						continue;
					}
					entries.push(AnnotationEntry {
						id: ID(row.id),
						kind,
						source: DeviceKind::Web,
						source_device_id: None,
						source_device_name: None,
						editable: true,
						chapter_title: non_empty(Some(row.locator.chapter_title.clone())),
						href: non_empty(Some(row.locator.href.clone())),
						fragment: locator_fragment(&row.locator),
						page: row.locator.locations.as_ref().and_then(|at| at.position),
						progression: locator_progression(&row.locator),
						excerpt,
						note: non_empty(row.annotation_text),
						color: None,
						created_at: Some(row.created_at),
						updated_at: Some(row.updated_at),
						book: book.clone(),
					});
				}
			}

			if filter.wants(AnnotationKind::Bookmark) {
				let mut query = bookmark::Entity::find()
					.filter(bookmark::Column::UserId.eq(&user.id))
					.filter(bookmark::Column::MediaId.is_in(visible.clone()));
				if let Some(since) = filter.since {
					query = query.filter(bookmark::Column::CreatedAt.gte(since));
				}
				for row in query.all(conn).await? {
					let Some(book) = books.get(&row.media_id) else {
						continue;
					};
					entries.push(AnnotationEntry {
						id: ID(row.id),
						kind: AnnotationKind::Bookmark,
						source: DeviceKind::Web,
						source_device_id: None,
						source_device_name: None,
						// `updateAnnotation`/`deleteAnnotation` address
						// `media_annotations`; a bookmark has its own
						// `deleteBookmark` mutation and no note to edit.
						editable: false,
						chapter_title: row
							.locator
							.as_ref()
							.and_then(|at| non_empty(Some(at.chapter_title.clone()))),
						href: row
							.locator
							.as_ref()
							.and_then(|at| non_empty(Some(at.href.clone()))),
						fragment: row.locator.as_ref().and_then(locator_fragment),
						page: row.page.or_else(|| {
							row.locator
								.as_ref()
								.and_then(|at| at.locations.as_ref())
								.and_then(|at| at.position)
						}),
						progression: row.locator.as_ref().and_then(locator_progression),
						excerpt: non_empty(row.preview_content),
						note: None,
						color: None,
						created_at: Some(row.created_at),
						updated_at: Some(row.created_at),
						book: book.clone(),
					});
				}
			}
		}

		let devices = load_liseur_devices(conn, &user.id).await?;
		for row in liseur {
			let kind = match row.kind.as_str() {
				"note" => AnnotationKind::Note,
				"bookmark" => AnnotationKind::Bookmark,
				_ => AnnotationKind::Highlight,
			};
			if !filter.wants(kind) {
				continue;
			}
			let created_at = parse_ts(&row.client_ts);
			if let (Some(since), Some(created_at)) = (filter.since, created_at) {
				if created_at < since {
					continue;
				}
			}
			let book = match row.media_id.as_deref() {
				Some(media_id) => match books.get(media_id) {
					Some(book) => book.clone(),
					// The work is linked to a book this request may not see.
					None => continue,
				},
				// A standalone work is not a Stump book, so a book-scoped
				// filter excludes it.
				None if filter.scoped_to_media() => continue,
				None => AnnotationBook {
					key: format!("liseur:{}", row.work_id),
					media_id: None,
					title: row
						.work_title
						.clone()
						.filter(|title| !title.is_empty())
						.unwrap_or_else(|| row.work_id.clone()),
					authors: row
						.work_author
						.clone()
						.filter(|author| !author.is_empty())
						.map(|author| vec![author])
						.unwrap_or_default(),
					series_id: None,
					series_name: None,
					library_id: None,
					extension: None,
				},
			};
			let device = devices.get(&row.device_id);
			let anchor = liseur_anchor(row.locator.as_deref());
			entries.push(AnnotationEntry {
				id: ID(row.annotation_id),
				kind,
				source: device.map_or(DeviceKind::Liseur, |device| device.kind),
				source_device_id: device.map(|device| ID(device.id.clone())),
				source_device_name: device.map(|device| device.name.clone()),
				editable: false,
				chapter_title: anchor
					.as_ref()
					.and_then(|at| non_empty(Some(at.chapter_title.clone()))),
				href: anchor
					.as_ref()
					.and_then(|at| non_empty(Some(at.href.clone()))),
				fragment: anchor.as_ref().and_then(locator_fragment),
				page: anchor
					.as_ref()
					.and_then(|at| at.locations.as_ref())
					.and_then(|at| at.position),
				progression: row
					.progression
					.or_else(|| anchor.as_ref().and_then(locator_progression)),
				excerpt: non_empty(row.excerpt),
				note: non_empty(row.body),
				color: non_empty(row.color),
				created_at,
				updated_at: parse_ts(&row.updated_at),
				book,
			});
		}

		if let Some(sources) = filter.sources() {
			entries.retain(|entry| sources.contains(&entry.source));
		}
		if let Some(needle) = filter.needle() {
			entries.retain(|entry| {
				[
					entry.excerpt.as_deref(),
					entry.note.as_deref(),
					Some(entry.book.title.as_str()),
				]
				.into_iter()
				.flatten()
				.any(|field| field.to_lowercase().contains(&needle))
			});
		}

		// Books lead with their most recent annotation so the hub reads
		// newest-first, while a book's own annotations stay in the order they
		// were made — the order the export renders them in.
		let mut latest: HashMap<&str, Option<DateTime<Utc>>> = HashMap::new();
		for entry in &entries {
			let slot = latest.entry(entry.book.key.as_str()).or_default();
			if entry.created_at > *slot {
				*slot = entry.created_at;
			}
		}
		let mut order: Vec<(Option<DateTime<Utc>>, String)> = latest
			.into_iter()
			.map(|(key, at)| (at, key.to_owned()))
			.collect();
		order.sort_by(|left, right| {
			right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1))
		});
		let rank: HashMap<String, usize> = order
			.into_iter()
			.enumerate()
			.map(|(index, (_, key))| (key, index))
			.collect();
		let book_count = rank.len() as i64;
		entries.sort_by(|left, right| {
			rank[&left.book.key]
				.cmp(&rank[&right.book.key])
				.then_with(|| left.created_at.cmp(&right.created_at))
				.then_with(|| left.id.cmp(&right.id))
		});

		let total = entries.len() as i64;
		let offset = pagination.offset() as usize;
		let limit = pagination.limit() as usize;
		let items: Vec<AnnotationEntry> =
			entries.into_iter().skip(offset).take(limit).collect();

		Ok(Self {
			has_next: offset + items.len() < total as usize,
			items,
			total,
			book_count,
		})
	}
}

/// The books among `media_ids` this request may see, keyed by media id.
async fn load_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	filter: &AnnotationFilterInput,
	media_ids: BTreeSet<String>,
) -> Result<HashMap<String, AnnotationBook>> {
	if media_ids.is_empty() {
		return Ok(HashMap::new());
	}

	let mut query = media::Entity::find_for_user(user)
		.select_only()
		.column(media::Column::Id)
		.column(media::Column::Name)
		.column(media::Column::Extension)
		.column(media::Column::SeriesId)
		.column_as(series::Column::Name, "series_name")
		.column_as(series::Column::LibraryId, "library_id")
		.column_as(media_metadata::Column::Title, "title")
		.column_as(media_metadata::Column::Writers, "writers")
		.filter(media::Column::Id.is_in(media_ids))
		.filter(media::Column::DeletedAt.is_null());
	if let Some(media_id) = &filter.media_id {
		query = query.filter(media::Column::Id.eq(media_id.as_str()));
	}
	if let Some(series_id) = &filter.series_id {
		query = query.filter(media::Column::SeriesId.eq(series_id.as_str()));
	}
	if let Some(library_id) = &filter.library_id {
		query = query.filter(series::Column::LibraryId.eq(library_id.as_str()));
	}

	Ok(query
		.into_model::<BookRow>()
		.all(conn)
		.await?
		.into_iter()
		.map(|row| (row.id.clone(), AnnotationBook::from(row)))
		.collect())
}

/// Live liseur-sync CAS records for `user_id`, each with the work's title and
/// the book its work is linked to. Tombstones are excluded: a deleted record
/// is not an annotation the user has.
async fn load_liseur_rows(
	conn: &DatabaseConnection,
	user_id: &str,
	filter: &AnnotationFilterInput,
) -> Result<Vec<LiseurRow>> {
	// `WEB` is the native lane only, so a source filter without a liseur-side
	// kind never needs the CAS table.
	if filter
		.sources()
		.is_some_and(|sources| sources.iter().all(|source| *source == DeviceKind::Web))
	{
		return Ok(Vec::new());
	}

	Ok(conn
		.query_all(db_statement(
			conn,
			"SELECT a.annotation_id AS annotation_id, a.work_id AS work_id,
				a.kind AS kind, a.locator AS locator, a.progression AS progression,
				a.excerpt AS excerpt, a.color AS color, a.body AS body,
				a.device_id AS device_id, a.client_ts AS client_ts,
				a.updated_at AS updated_at,
				w.title AS work_title, w.author AS work_author,
				(
					SELECT l.media_id FROM liseur_sync_media_links l
					WHERE l.user_id = a.user_id AND l.work_id = a.work_id
					ORDER BY l.created_at ASC, l.id ASC
					LIMIT 1
				) AS media_id
			FROM liseur_sync_annotations a
			LEFT JOIN liseur_sync_works w
				ON w.user_id = a.user_id AND w.id = a.work_id
			WHERE a.user_id = $1 AND a.deleted = FALSE
			ORDER BY a.client_ts ASC, a.annotation_id ASC",
			[user_id.into()],
		))
		.await?
		.iter()
		.map(|row| LiseurRow::from_query_result(row, ""))
		.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// The registered device behind every liseur token of `user_id`, keyed by the
/// liseur device id that stamps a pushed annotation. A login-session token is
/// bound to no device and simply does not appear.
async fn load_liseur_devices(
	conn: &DatabaseConnection,
	user_id: &str,
) -> Result<HashMap<String, SourceDevice>> {
	Ok(conn
		.query_all(db_statement(
			conn,
			"SELECT t.device_id AS liseur_device_id, d.id AS device_id,
				d.name AS name, d.kind AS kind
			FROM liseur_sync_tokens t
			JOIN device_credentials c
				ON c.credential_ref = t.id AND c.credential_kind = $2
			JOIN devices d ON d.id = c.device_id
			WHERE t.user_id = $1",
			[
				user_id.into(),
				DeviceCredentialKind::LiseurToken.to_string().into(),
			],
		))
		.await?
		.iter()
		.map(|row| {
			LiseurDeviceRow::from_query_result(row, "").map(|row| {
				(
					row.liseur_device_id,
					SourceDevice {
						id: row.device_id,
						name: row.name,
						kind: row.kind,
					},
				)
			})
		})
		.collect::<std::result::Result<HashMap<_, _>, _>>()?)
}

#[cfg(test)]
mod tests {
	use ::tests::{db::test_database, fake_data};
	use chrono::TimeZone;
	use models::{
		entity::{device, device_credential, library_exclusion, liseur_sync_token},
		shared::{
			enums::DeviceProtocol,
			readium::{ReadiumLocation, ReadiumText},
		},
	};
	use sea_orm::{
		prelude::Decimal, ActiveValue::Set, ConnectionTrait, DbBackend, Schema, Statement,
	};

	use crate::tests::common::get_default_user;

	use super::*;

	const LISEUR_DDL: &[&str] = &[
		"CREATE TABLE liseur_sync_works (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, title TEXT NOT NULL, author TEXT NOT NULL)",
		"CREATE TABLE liseur_sync_media_links (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, media_id TEXT NOT NULL, work_id TEXT NOT NULL, edition_sha TEXT NOT NULL, resolution_status TEXT NOT NULL, created_at TEXT NOT NULL)",
		"CREATE TABLE liseur_sync_annotations (
			row_id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT NOT NULL, annotation_id TEXT NOT NULL,
			rev BIGINT NOT NULL, seq BIGINT NOT NULL, work_id TEXT NOT NULL, edition_sha TEXT, kind TEXT NOT NULL,
			locator TEXT, progression DOUBLE, excerpt TEXT NOT NULL, color TEXT NOT NULL, body TEXT NOT NULL,
			device_id TEXT NOT NULL, client_ts TEXT NOT NULL, updated_at TEXT NOT NULL,
			deleted BOOLEAN NOT NULL DEFAULT FALSE, deleted_at TEXT, payload TEXT NOT NULL)",
	];

	fn ts(secs: i64) -> DateTime<Utc> {
		Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
	}

	fn highlight_locator(
		chapter: &str,
		href: &str,
		text: Option<&str>,
	) -> ReadiumLocator {
		ReadiumLocator {
			chapter_title: chapter.to_owned(),
			href: href.to_owned(),
			locations: Some(ReadiumLocation {
				fragments: Some(vec!["p12".to_owned()]),
				progression: Some(Decimal::new(5, 1)),
				position: Some(12),
				total_progression: Some(Decimal::new(25, 2)),
				css_selector: None,
				partial_cfi: None,
			}),
			text: text.map(|highlight| ReadiumText {
				after: None,
				before: None,
				highlight: Some(highlight.to_owned()),
			}),
			..Default::default()
		}
	}

	/// One user, two books in one library, and:
	///
	/// - `n-highlight` on *Dune* (selected passage + note),
	/// - `n-note` on *Dune* (note only, no selected passage),
	/// - `b-mark` bookmark on *Emma*,
	/// - `l-kobo` liseur highlight on *Emma*, pushed by a Kobo device.
	async fn seeded() -> (DatabaseConnection, AuthUser, String, String, String) {
		let conn = test_database().await;
		let schema = Schema::new(DbBackend::Sqlite);
		for statement in [
			schema.create_table_from_entity(media_annotation::Entity),
			schema.create_table_from_entity(bookmark::Entity),
		] {
			conn.execute(conn.get_database_backend().build(&statement))
				.await
				.unwrap();
		}
		for sql in LISEUR_DDL {
			conn.execute(Statement::from_string(DbBackend::Sqlite, *sql))
				.await
				.unwrap();
		}

		let row = fake_data::User::new("reader").insert(&conn).await;
		let user = AuthUser {
			id: row.id.clone(),
			username: row.username.clone(),
			is_server_owner: false,
			..get_default_user()
		};
		let library = fake_data::Library::default().insert(&conn).await;
		let series = fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&conn)
		.await;
		let mut books = Vec::new();
		for (id, name, title, writers) in [
			(
				"m-dune",
				"dune.epub",
				"Dune",
				"Frank Herbert, Brian Herbert",
			),
			("m-emma", "emma.epub", "Emma", "Jane Austen"),
		] {
			let media = fake_data::Media {
				series_id: series.id.clone(),
				id: Some(id.to_owned()),
				name: Some(name.to_owned()),
				..Default::default()
			}
			.insert(&conn)
			.await;
			media_metadata::ActiveModel {
				media_id: Set(Some(media.id.clone())),
				title: Set(Some(title.to_owned())),
				writers: Set(Some(writers.to_owned())),
				..Default::default()
			}
			.insert(&conn)
			.await
			.unwrap();
			books.push(media.id);
		}

		for (id, note, text, created) in [
			(
				"n-highlight",
				Some("spice"),
				Some("The spice must flow."),
				10,
			),
			("n-note", Some("re-read this chapter"), None, 20),
		] {
			let inserted = media_annotation::ActiveModel {
				id: Set(id.to_owned()),
				locator: Set(highlight_locator("Chapter One", "ch1.xhtml", text)),
				annotation_text: Set(note.map(str::to_owned)),
				media_id: Set(books[0].clone()),
				user_id: Set(user.id.clone()),
				..Default::default()
			}
			.insert(&conn)
			.await
			.unwrap();
			// `before_save` stamps `created_at`; pin it so ordering is fixed.
			let mut active: media_annotation::ActiveModel = inserted.into();
			active.created_at = Set(ts(created));
			active.update(&conn).await.unwrap();
		}

		bookmark::ActiveModel {
			id: Set("b-mark".to_owned()),
			preview_content: Set(Some("A beginning is the time".to_owned())),
			locator: Set(None),
			page: Set(Some(42)),
			media_id: Set(books[1].clone()),
			user_id: Set(user.id.clone()),
			created_at: Set(ts(30)),
		}
		.insert(&conn)
		.await
		.unwrap();
		// `bookmark::before_save` stamps `created_at` unconditionally on
		// insert, so the fixture time has to be written afterwards.
		bookmark::ActiveModel {
			id: Set("b-mark".to_owned()),
			created_at: Set(ts(30)),
			..Default::default()
		}
		.update(&conn)
		.await
		.unwrap();

		// A Kobo running NickelStump: a registered device whose liseur token
		// stamps the CAS records it pushes.
		device::ActiveModel {
			id: Set("dev-kobo".to_owned()),
			user_id: Set(user.id.clone()),
			name: Set("Kobo Clara".to_owned()),
			kind: Set(DeviceKind::Kobo),
			created_at: Set(ts(0).fixed_offset()),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();
		liseur_sync_token::ActiveModel {
			id: Set("tok-1".to_owned()),
			user_id: Set(user.id.clone()),
			device_id: Set("liseur-dev-1".to_owned()),
			secret_hash: Set("hash".to_owned()),
			scopes: Set("[\"sync\"]".to_owned()),
			created_at: Set(ts(0).to_rfc3339()),
			expires_at: Set(ts(100_000).to_rfc3339()),
			last_used_at: Set(None),
			revoked_at: Set(None),
			name: Set(Some("Kobo Clara".to_owned())),
			token_kind: Set(Some("device".to_owned())),
		}
		.insert(&conn)
		.await
		.unwrap();
		device_credential::ActiveModel {
			device_id: Set("dev-kobo".to_owned()),
			protocol: Set(DeviceProtocol::Liseur),
			credential_kind: Set(DeviceCredentialKind::LiseurToken),
			credential_ref: Set("tok-1".to_owned()),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		exec(
			&conn,
			"INSERT INTO liseur_sync_works (id, user_id, title, author)
			 VALUES ('work-emma', $1, 'Emma', 'Jane Austen')",
			[user.id.clone().into()],
		)
		.await;
		exec(
			&conn,
			"INSERT INTO liseur_sync_media_links
				(id, user_id, media_id, work_id, edition_sha, resolution_status, created_at)
			 VALUES ('link-1', $1, 'm-emma', 'work-emma', '', 'linked', $2)",
			[user.id.clone().into(), ts(0).to_rfc3339().into()],
		)
		.await;
		insert_liseur(&conn, &user.id, "l-kobo", "highlight", false, 40).await;
		// A tombstone is not an annotation the user has.
		insert_liseur(&conn, &user.id, "l-gone", "highlight", true, 50).await;

		(conn, user, books[0].clone(), books[1].clone(), library.id)
	}

	async fn exec(
		conn: &DatabaseConnection,
		sql: &str,
		values: impl IntoIterator<Item = sea_orm::Value>,
	) {
		conn.execute(db_statement(conn, sql, values)).await.unwrap();
	}

	async fn insert_liseur(
		conn: &DatabaseConnection,
		user_id: &str,
		id: &str,
		kind: &str,
		deleted: bool,
		secs: i64,
	) {
		exec(
			conn,
			"INSERT INTO liseur_sync_annotations
				(user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
				 progression, excerpt, color, body, device_id, client_ts, updated_at,
				 deleted, deleted_at, payload)
			 VALUES ($1, $2, 1, 1, 'work-emma', NULL, $3,
				 '{\"href\":\"OEBPS/ch3.xhtml\",\"chapterTitle\":\"Chapter Three\"}',
				 0.62, 'Emma Woodhouse, handsome, clever', 'yellow', 'her father',
				 'liseur-dev-1', $4, $4, $5, NULL, '{}')",
			[
				user_id.into(),
				id.into(),
				kind.into(),
				ts(secs).to_rfc3339().into(),
				deleted.into(),
			],
		)
		.await;
	}

	async fn fetch(
		conn: &DatabaseConnection,
		user: &AuthUser,
		filter: AnnotationFilterInput,
	) -> AnnotationPage {
		AnnotationPage::fetch(conn, user, &filter, &OffsetPagination::default())
			.await
			.unwrap()
	}

	fn ids(page: &AnnotationPage) -> Vec<&str> {
		page.items.iter().map(|entry| entry.id.as_str()).collect()
	}

	#[tokio::test]
	async fn folds_every_source_into_book_contiguous_pages() {
		let (conn, user, ..) = seeded().await;
		let page = fetch(&conn, &user, AnnotationFilterInput::default()).await;

		// Emma leads: its newest annotation (the liseur highlight at +40s) is
		// more recent than Dune's newest (+20s). Inside a book, annotations
		// keep the order they were made.
		assert_eq!(ids(&page), ["b-mark", "l-kobo", "n-highlight", "n-note"]);
		assert_eq!(page.total, 4);
		assert_eq!(page.book_count, 2);
		assert!(!page.has_next);

		let by_id = |id: &str| {
			page.items
				.iter()
				.find(|entry| entry.id.as_str() == id)
				.unwrap()
		};

		// A native row with a selected passage is a highlight, one carrying
		// only the user's text is a note, and both are editable here.
		let highlight = by_id("n-highlight");
		assert_eq!(highlight.kind, AnnotationKind::Highlight);
		assert_eq!(highlight.excerpt.as_deref(), Some("The spice must flow."));
		assert_eq!(highlight.note.as_deref(), Some("spice"));
		assert_eq!(highlight.source, DeviceKind::Web);
		assert!(highlight.editable);
		assert_eq!(highlight.href.as_deref(), Some("ch1.xhtml"));
		assert_eq!(highlight.fragment.as_deref(), Some("p12"));
		assert_eq!(highlight.chapter_title.as_deref(), Some("Chapter One"));
		assert_eq!(highlight.progression, Some(0.25));
		assert_eq!(highlight.book.title, "Dune");
		assert_eq!(highlight.book.key, "native:m-dune");
		assert_eq!(highlight.book.authors, ["Frank Herbert", "Brian Herbert"]);
		assert_eq!(by_id("n-note").kind, AnnotationKind::Note);
		assert!(by_id("n-note").excerpt.is_none());

		let bookmark = by_id("b-mark");
		assert_eq!(bookmark.kind, AnnotationKind::Bookmark);
		assert_eq!(bookmark.page, Some(42));
		assert!(!bookmark.editable);

		// The liseur record reports the kind of the device that pushed it and
		// is folded onto the book its work is linked to.
		let kobo = by_id("l-kobo");
		assert_eq!(kobo.source, DeviceKind::Kobo);
		assert_eq!(
			kobo.source_device_id.as_ref().map(|id| id.as_str()),
			Some("dev-kobo")
		);
		assert_eq!(kobo.source_device_name.as_deref(), Some("Kobo Clara"));
		assert!(!kobo.editable);
		assert_eq!(kobo.color.as_deref(), Some("yellow"));
		assert_eq!(kobo.progression, Some(0.62));
		assert_eq!(kobo.href.as_deref(), Some("OEBPS/ch3.xhtml"));
		assert_eq!(
			kobo.book.media_id.as_ref().map(|id| id.as_str()),
			Some("m-emma")
		);
		assert_eq!(kobo.book.key, "native:m-emma");
	}

	#[tokio::test]
	async fn filters_narrow_by_kind_source_book_text_and_since() {
		let (conn, user, dune, _emma, library) = seeded().await;

		let notes = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				kind: Some(vec![AnnotationKind::Note]),
				..Default::default()
			},
		)
		.await;
		assert_eq!(ids(&notes), ["n-note"]);

		// `WEB` is the native lane, so the CAS table is not even read.
		let native = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				source: Some(vec![DeviceKind::Web]),
				..Default::default()
			},
		)
		.await;
		assert_eq!(ids(&native), ["b-mark", "n-highlight", "n-note"]);
		let from_kobo = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				source: Some(vec![DeviceKind::Kobo]),
				..Default::default()
			},
		)
		.await;
		assert_eq!(ids(&from_kobo), ["l-kobo"]);

		let one_book = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				media_id: Some(ID(dune)),
				..Default::default()
			},
		)
		.await;
		assert_eq!(ids(&one_book), ["n-highlight", "n-note"]);
		assert_eq!(one_book.book_count, 1);

		// The needle matches the selected passage, the note, or the title.
		let searched = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				query: Some("  SPICE ".to_owned()),
				..Default::default()
			},
		)
		.await;
		assert_eq!(ids(&searched), ["n-highlight"]);
		let by_title = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				query: Some("emma".to_owned()),
				..Default::default()
			},
		)
		.await;
		assert_eq!(ids(&by_title), ["b-mark", "l-kobo"]);

		// `since` bounds both lanes.
		let recent = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				since: Some(ts(25)),
				..Default::default()
			},
		)
		.await;
		assert_eq!(ids(&recent), ["b-mark", "l-kobo"]);

		let by_library = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				library_id: Some(ID(library)),
				..Default::default()
			},
		)
		.await;
		assert_eq!(by_library.total, 4);
		let elsewhere = fetch(
			&conn,
			&user,
			AnnotationFilterInput {
				library_id: Some(ID("other".to_owned())),
				..Default::default()
			},
		)
		.await;
		assert_eq!(elsewhere.total, 0);
	}

	#[tokio::test]
	async fn hides_books_the_user_may_not_see() {
		let (conn, user, .., library) = seeded().await;
		library_exclusion::ActiveModel {
			user_id: Set(user.id.clone()),
			library_id: Set(library),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		// Both books live in the hidden library, and the liseur record is
		// linked to one of them, so nothing is left — not even the CAS row.
		let page = fetch(&conn, &user, AnnotationFilterInput::default()).await;
		assert!(page.items.is_empty());
		assert_eq!(page.total, 0);
		assert_eq!(page.book_count, 0);
	}

	#[tokio::test]
	async fn paginates_over_the_merged_set() {
		let (conn, user, ..) = seeded().await;
		let first = AnnotationPage::fetch(
			&conn,
			&user,
			&AnnotationFilterInput::default(),
			&OffsetPagination {
				page: 1,
				page_size: Some(3),
				zero_based: Some(false),
			},
		)
		.await
		.unwrap();
		assert_eq!(ids(&first), ["b-mark", "l-kobo", "n-highlight"]);
		assert_eq!(first.total, 4);
		assert!(first.has_next);

		let second = AnnotationPage::fetch(
			&conn,
			&user,
			&AnnotationFilterInput::default(),
			&OffsetPagination {
				page: 2,
				page_size: Some(3),
				zero_based: Some(false),
			},
		)
		.await
		.unwrap();
		assert_eq!(ids(&second), ["n-note"]);
		assert!(!second.has_next);
	}
}
