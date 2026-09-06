//! Turn a remote series into ordinary `series`/`media` rows.
//!
//! Every readable chapter becomes one `media` row with `extension = "cbz"`, a
//! `provider://` path, and the chapter's known page count (0 until the page
//! list is first resolved). Re-running for the same series only adds chapters
//! that are not stored yet, which is also how a refresh works.
//!
//! Chapters the source cannot serve ([`RemoteChapter::readable`] `== false`)
//! are skipped and reported instead: writing a row for one only produces a
//! book whose every page 404s.
//!
//! A refresh also reconciles in the other direction: a stored chapter the
//! source has since stopped serving is hidden (`media.deleted_at`, the
//! tombstone every lane filters on), so a series materialised before this
//! rule — or before the source retracted the chapter — stops advertising
//! books that cannot be opened, without losing the progress recorded
//! against them. Because a feed can lie about what it will serve, the
//! refresh verifies a budget of rows against the page manifest as well; see
//! [`verify_stored_chapters`].

use std::collections::BTreeSet;

use chrono::Utc;
use models::{
	entity::{media, media_metadata, series, series_metadata},
	shared::enums::FileStatus,
};
use rust_decimal::Decimal;
use sea_orm::{
	prelude::DateTimeWithTimeZone, sea_query::Expr, ActiveModelTrait, ActiveValue::Set,
	ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};

use crate::{
	host::{ProviderError, ProviderHost},
	source::{ContentRating, RemoteChapter, RemoteSeries},
	virtual_path::{self, VirtualPath},
};

/// Why a chapter was not materialised. Reported per series so an operator
/// can tell "this title has no readable chapters here" from "the fetch
/// failed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
	/// The source hosts the chapter somewhere else
	/// (MangaDex `ChapterAttributes.externalUrl`).
	ExternallyHosted,
	/// The source reports no readable images
	/// (MangaDex `ChapterAttributes.pages == 0`).
	NoPages,
	/// The source marks the chapter unavailable
	/// (MangaDex `ChapterAttributes.isUnavailable`).
	Unavailable,
}

impl SkipReason {
	/// The reason a source reports for `chapter`, or `None` when it is
	/// readable. `external_url` is checked first because it is the only
	/// reason that names somewhere the reader could go instead.
	pub fn of(chapter: &RemoteChapter) -> Option<Self> {
		if chapter.readable {
			return None;
		}
		if chapter.external_url.is_some() {
			return Some(SkipReason::ExternallyHosted);
		}
		if chapter.page_count.is_none_or(|pages| pages == 0) {
			return Some(SkipReason::NoPages);
		}
		Some(SkipReason::Unavailable)
	}

	pub fn as_str(self) -> &'static str {
		match self {
			SkipReason::ExternallyHosted => "externally hosted",
			SkipReason::NoPages => "no readable pages",
			SkipReason::Unavailable => "marked unavailable by the source",
		}
	}
}

/// One chapter materialisation refused to write a row for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedChapter {
	pub remote_id: String,
	pub reason: SkipReason,
}

/// The rows touched by [`add_series`] / [`refresh_series`].
#[derive(Debug, Clone, PartialEq)]
pub struct Materialized {
	pub series: series::Model,
	/// Newly inserted chapters, in remote order.
	pub created: Vec<media::Model>,
	/// Chapters already present before this run.
	pub existing: usize,
	/// Chapters the source's own feed flags as unserveable, in remote order.
	/// Never written as rows; a series whose whole feed is unreadable
	/// materialises with no books and every skip listed here.
	pub skipped: Vec<SkippedChapter>,
	/// `media` ids hidden (`media.deleted_at`) because the source will not
	/// serve the chapter: the feed flagged it, or the page manifest 404'd
	/// under verification. Series materialised before this rule — or before
	/// the source retracted a chapter — carry rows whose every page 404s;
	/// a refresh takes them out of every listing while keeping the reading
	/// progress recorded against them.
	pub removed: Vec<String>,
	/// `media` ids listed again because the source serves the chapter after
	/// all. Only a resolved page manifest restores a row; the feed calling
	/// a chapter readable is never enough.
	pub restored: Vec<String>,
	/// The cross-source duplicate this series was linked to, when the same
	/// work already existed from another source; see [`crate::identity`].
	pub duplicate_of: Option<String>,
}

/// Materialise (or extend) `remote_id` from `source_id` into `library_id`.
pub async fn add_series(
	host: &ProviderHost,
	library_id: &str,
	source_id: &str,
	remote_id: &str,
) -> Result<Materialized, ProviderError> {
	let source = host.source(source_id)?;
	let details = source.details(remote_id).await?;
	let chapters = source.chapters(remote_id).await?;
	let conn = host.conn();

	let existing_series = series::Entity::find()
		.filter(series::Column::SourceProvider.eq(source_id))
		.filter(series::Column::RemoteId.eq(remote_id))
		.one(conn)
		.await?;
	let series_row = match existing_series {
		Some(existing) => update_series(conn, existing, &details).await?,
		None => insert_series(conn, library_id, source_id, &details).await?,
	};
	// Stump stores age restrictions per user and compares them against
	// `series_metadata.age_rating`; there is no library-level rating to set,
	// so an adult source is applied to each of its series.
	let adult_source = host.source_is_adult(source_id).await;
	upsert_series_metadata(conn, &series_row, source_id, &details, adult_source).await?;
	// Dedupe is recorded, never enforced: a link only tells an operator the
	// same work exists twice.
	let duplicate_of =
		crate::identity::record_identity(conn, &series_row, source_id, &details)
			.await?
			.map(|link| link.canonical_series_id);
	let (created, existing, skipped, declared) =
		insert_chapters(conn, &series_row, source_id, remote_id, &chapters).await?;
	let (proven, restored) =
		verify_stored_chapters(host, &series_row.id, source_id, &chapters).await?;
	let removed = declared.into_iter().chain(proven).collect::<Vec<_>>();
	if !skipped.is_empty() || !removed.is_empty() || !restored.is_empty() {
		tracing::info!(
			series = series_row.id,
			source = source_id,
			skipped = skipped.len(),
			removed = removed.len(),
			restored = restored.len(),
			readable = chapters.len() - skipped.len(),
			reasons = ?skipped
				.iter()
				.map(|skip| skip.reason.as_str())
				.collect::<BTreeSet<_>>(),
			"Reconciled chapters against what the source can serve"
		);
	}
	host.emit(crate::event::ProviderEvent::SeriesMaterialized {
		series_id: series_row.id.clone(),
		source: source_id.to_string(),
		// The row's own library, not the requested one: re-materialising a
		// series that already exists never moves it between libraries.
		library_id: series_row
			.library_id
			.clone()
			.unwrap_or_else(|| library_id.to_string()),
	});
	Ok(Materialized {
		series: series_row,
		created,
		existing,
		skipped,
		removed,
		restored,
		duplicate_of,
	})
}

/// Re-fetch details and chapters for a materialised series.
pub async fn refresh_series(
	host: &ProviderHost,
	series_id: &str,
) -> Result<Materialized, ProviderError> {
	let existing = series::Entity::find_by_id(series_id)
		.one(host.conn())
		.await?
		.ok_or_else(|| ProviderError::Other(format!("Series {series_id} not found")))?;
	let (Some(source_id), Some(remote_id), Some(library_id)) = (
		existing.source_provider.clone(),
		existing.remote_id.clone(),
		existing.library_id.clone(),
	) else {
		return Err(ProviderError::NotVirtual(existing.path));
	};
	add_series(host, &library_id, &source_id, &remote_id).await
}

async fn insert_series<C: ConnectionTrait>(
	conn: &C,
	library_id: &str,
	source_id: &str,
	details: &RemoteSeries,
) -> Result<series::Model, ProviderError> {
	let row = series::ActiveModel {
		id: Set(virtual_path::series_id(source_id, &details.remote_id)),
		name: Set(details.title.clone()),
		description: Set(details.description.clone()),
		path: Set(VirtualPath::series(source_id, &details.remote_id).to_string()),
		status: Set(FileStatus::Ready),
		library_id: Set(Some(library_id.to_string())),
		source_provider: Set(Some(source_id.to_string())),
		remote_id: Set(Some(details.remote_id.clone())),
		..Default::default()
	}
	.insert(conn)
	.await?;
	Ok(row)
}

async fn update_series<C: ConnectionTrait>(
	conn: &C,
	existing: series::Model,
	details: &RemoteSeries,
) -> Result<series::Model, ProviderError> {
	let unchanged = existing.name == details.title
		&& existing.description == details.description
		&& existing.status == FileStatus::Ready;
	if unchanged {
		return Ok(existing);
	}
	let mut active: series::ActiveModel = existing.into();
	active.name = Set(details.title.clone());
	active.description = Set(details.description.clone());
	active.status = Set(FileStatus::Ready);
	Ok(active.update(conn).await?)
}

/// Project the remote description onto `series_metadata`, including the age
/// rating Stump's per-user age restriction filters on.
///
/// `adult_source` is the catalog's `nsfw` flag for the source instance. Stump
/// has no library-level age rating, so an adult source raises every series it
/// materialises to 18 regardless of the per-title rating.
async fn upsert_series_metadata<C: ConnectionTrait>(
	conn: &C,
	series_row: &series::Model,
	source_id: &str,
	details: &RemoteSeries,
	adult_source: bool,
) -> Result<(), ProviderError> {
	let joined = |values: &[String]| {
		if values.is_empty() {
			None
		} else {
			Some(values.join(", "))
		}
	};
	let fields = series_metadata::ActiveModel {
		series_id: Set(series_row.id.clone()),
		title: Set(Some(details.title.clone())),
		summary: Set(details.description.clone()),
		status: Set(Some(details.status.as_metadata_status().to_string())),
		genres: Set(joined(&details.genres)),
		writers: Set(joined(&details.authors)),
		comic_image: Set(details.thumbnail_url.clone()),
		language: Set(details.original_language.clone()),
		links: Set(details.url.clone()),
		metadata_source: Set(Some(source_id.to_string())),
		metadata_external_id: Set(Some(details.remote_id.clone())),
		age_rating: Set(age_rating(details, adult_source)),
		..Default::default()
	};
	let exists = series_metadata::Entity::find_by_id(&series_row.id)
		.one(conn)
		.await?
		.is_some();
	if exists {
		fields.update(conn).await?;
	} else {
		let mut fields = fields;
		fields.title_sort_lock = Set(false);
		fields.reading_direction_lock = Set(false);
		fields.language_lock = Set(false);
		fields.alternate_titles_lock = Set(false);
		fields.insert(conn).await?;
	}
	Ok(())
}

/// The `series_metadata.age_rating` for a remote series: the source's own
/// content rating ([`ContentRating::age_rating`]), raised to 18 when the
/// series is adult or comes from an adult source.
pub fn age_rating(details: &RemoteSeries, adult_source: bool) -> Option<i32> {
	if adult_source || details.nsfw {
		return Some(18);
	}
	details.content_rating.and_then(ContentRating::age_rating)
}

/// A chapter of this series that already has a `media` row.
struct KnownChapter {
	id: String,
	chapter: String,
	/// The row is a tombstone (`media.deleted_at`): stored, keyed by the
	/// same id, but listed by nothing.
	hidden: bool,
}

async fn insert_chapters<C: ConnectionTrait>(
	conn: &C,
	series_row: &series::Model,
	source_id: &str,
	remote_id: &str,
	chapters: &[RemoteChapter],
) -> Result<(Vec<media::Model>, usize, Vec<SkippedChapter>, Vec<String>), ProviderError> {
	// Every chapter already stored, with its row id and whether that row is
	// already hidden: a chapter the source has stopped serving has to be
	// *found*, not merely recognised.
	let known: Vec<KnownChapter> = media::Entity::find()
		.select_only()
		.column(media::Column::Id)
		.column(media::Column::RemoteChapterId)
		.column(media::Column::DeletedAt)
		.filter(media::Column::SeriesId.eq(series_row.id.clone()))
		.filter(media::Column::SourceProvider.eq(source_id))
		.into_tuple::<(String, Option<String>, Option<DateTimeWithTimeZone>)>()
		.all(conn)
		.await?
		.into_iter()
		.filter_map(|(id, chapter, deleted_at)| {
			chapter.map(|chapter| KnownChapter {
				id,
				chapter,
				hidden: deleted_at.is_some(),
			})
		})
		.collect();
	let mut created = Vec::new();
	let mut existing = 0usize;
	let mut skipped = Vec::new();
	let mut removed = Vec::new();
	for chapter in chapters {
		if let Some(reason) = SkipReason::of(chapter) {
			skipped.push(SkippedChapter {
				remote_id: chapter.remote_id.clone(),
				reason,
			});
			// Rematerialise: a row written before the source retracted this
			// chapter (or before unreadable chapters were skipped at all) is
			// a book whose every page 404s and whose stale `pages` count
			// still promises otherwise. Hide it.
			if let Some(stored) = known
				.iter()
				.find(|stored| stored.chapter == chapter.remote_id)
				.filter(|stored| !stored.hidden)
			{
				removed.push(stored.id.clone());
			}
			continue;
		}
		if known
			.iter()
			.any(|stored| stored.chapter == chapter.remote_id)
		{
			existing += 1;
			continue;
		}
		let name = chapter.display_name();
		let row = media::ActiveModel {
			id: Set(virtual_path::media_id(source_id, &chapter.remote_id)),
			name: Set(name.clone()),
			size: Set(0),
			extension: Set("cbz".to_string()),
			pages: Set(chapter.page_count.map(|count| count as i32).unwrap_or(0)),
			modified_at: Set(chapter.uploaded_at.map(Into::into)),
			path: Set(
				VirtualPath::chapter(source_id, remote_id, &chapter.remote_id)
					.to_string(),
			),
			status: Set(FileStatus::Ready),
			series_id: Set(Some(series_row.id.clone())),
			source_provider: Set(Some(source_id.to_string())),
			remote_id: Set(Some(remote_id.to_string())),
			remote_chapter_id: Set(Some(chapter.remote_id.clone())),
			..Default::default()
		}
		.insert(conn)
		.await?;
		media_metadata::ActiveModel {
			media_id: Set(Some(row.id.clone())),
			title: Set(Some(name)),
			series: Set(Some(series_row.name.clone())),
			number: Set(chapter
				.number
				.and_then(|number| Decimal::try_from(number).ok())),
			volume: Set(chapter
				.volume
				.as_deref()
				.and_then(|volume| volume.parse::<i32>().ok())),
			language: Set(chapter.lang.clone()),
			publisher: Set(chapter.scanlator.clone()),
			page_count: Set(chapter.page_count.map(|count| count as i32)),
			year: Set(chapter.uploaded_at.map(|at| {
				use chrono::Datelike;
				at.year()
			})),
			month: Set(chapter.uploaded_at.map(|at| {
				use chrono::Datelike;
				at.month() as i32
			})),
			day: Set(chapter.uploaded_at.map(|at| {
				use chrono::Datelike;
				at.day() as i32
			})),
			links: Set(chapter.url.clone()),
			metadata_source: Set(Some(source_id.to_string())),
			metadata_external_id: Set(Some(chapter.remote_id.clone())),
			..Default::default()
		}
		.insert(conn)
		.await?;
		created.push(row);
	}
	hide(conn, &removed).await?;
	Ok((created, existing, skipped, removed))
}

/// Hide the rows the source can no longer serve.
///
/// A tombstone (`media.deleted_at`), not a delete: every lane already
/// filters `deleted_at IS NULL`, so the book stops being listed anywhere,
/// while reading progress, bookmarks, and annotations keyed on the id
/// survive — and the row stays known, so a feed that still advertises the
/// chapter cannot resurrect it on the next refresh.
async fn hide<C: ConnectionTrait>(conn: &C, ids: &[String]) -> Result<(), ProviderError> {
	if ids.is_empty() {
		return Ok(());
	}
	let now = Utc::now().fixed_offset();
	media::Entity::update_many()
		.col_expr(media::Column::DeletedAt, Expr::value(now))
		.col_expr(media::Column::UpdatedAt, Expr::value(now))
		.filter(media::Column::Id.is_in(ids.to_vec()))
		.exec(conn)
		.await?;
	Ok(())
}

/// Undo [`hide`] for a chapter the source serves again.
async fn unhide<C: ConnectionTrait>(conn: &C, id: &str) -> Result<(), ProviderError> {
	media::Entity::update_many()
		.col_expr(
			media::Column::DeletedAt,
			Expr::value(None::<DateTimeWithTimeZone>),
		)
		.col_expr(
			media::Column::UpdatedAt,
			Expr::value(Utc::now().fixed_offset()),
		)
		.filter(media::Column::Id.eq(id))
		.exec(conn)
		.await?;
	Ok(())
}

/// Record that a row was re-checked and found unchanged. `updated_at` is the
/// cursor [`verify_stored_chapters`] walks, so it has to move even when
/// nothing else does.
async fn touch<C: ConnectionTrait>(conn: &C, id: &str) -> Result<(), ProviderError> {
	media::Entity::update_many()
		.col_expr(
			media::Column::UpdatedAt,
			Expr::value(Utc::now().fixed_offset()),
		)
		.filter(media::Column::Id.eq(id))
		.exec(conn)
		.await?;
	Ok(())
}

/// How many stored chapters one refresh re-checks against the source.
const VERIFY_PER_REFRESH: usize = 16;

/// Re-check stored chapters against the page manifest, the only authority on
/// whether a chapter can actually be read.
///
/// A feed can list a chapter its own source will not serve: MangaDex reports
/// `pages: 14, externalUrl: null, isUnavailable: false` for One Piece
/// ch. 1191 while `/at-home/server/391c1555-…` answers
/// `404 Chapter with ID … not found`. [`SkipReason`] cannot see that, so a
/// refresh also asks for the manifest — one request per chapter, hence the
/// [`VERIFY_PER_REFRESH`] budget, spent on the least recently checked rows
/// (`updated_at`, bumped on every check) so successive refreshes walk the
/// whole series.
///
/// Returns `(hidden, restored)`.
async fn verify_stored_chapters(
	host: &ProviderHost,
	series_id: &str,
	source_id: &str,
	chapters: &[RemoteChapter],
) -> Result<(Vec<String>, Vec<String>), ProviderError> {
	let readable: BTreeSet<&str> = chapters
		.iter()
		.filter(|chapter| SkipReason::of(chapter).is_none())
		.map(|chapter| chapter.remote_id.as_str())
		.collect();
	let stored: Vec<(String, Option<String>, Option<DateTimeWithTimeZone>)> =
		media::Entity::find()
			.select_only()
			.column(media::Column::Id)
			.column(media::Column::RemoteChapterId)
			.column(media::Column::DeletedAt)
			.filter(media::Column::SeriesId.eq(series_id))
			.filter(media::Column::SourceProvider.eq(source_id))
			.order_by_asc(media::Column::UpdatedAt)
			.into_tuple()
			.all(host.conn())
			.await?;
	let mut hidden = Vec::new();
	let mut restored = Vec::new();
	let mut checked = 0usize;
	for (id, chapter, deleted_at) in stored {
		if checked >= VERIFY_PER_REFRESH {
			break;
		}
		// A chapter the feed itself calls unreadable was already answered
		// for; spending a request on it would prove nothing.
		let Some(chapter) = chapter.filter(|stored| readable.contains(stored.as_str()))
		else {
			continue;
		};
		checked += 1;
		match host.verify_chapter(source_id, &chapter).await {
			Ok(()) if deleted_at.is_some() => {
				unhide(host.conn(), &id).await?;
				restored.push(id);
			},
			Err(ProviderError::Unavailable { .. }) if deleted_at.is_none() => {
				hide(host.conn(), std::slice::from_ref(&id)).await?;
				hidden.push(id);
			},
			Ok(()) | Err(ProviderError::Unavailable { .. }) => {
				touch(host.conn(), &id).await?;
			},
			// A transport failure says nothing about the chapter, and a
			// source that is down will fail the next fifteen the same way.
			Err(error) => {
				tracing::warn!(
					?error,
					source = source_id,
					chapter,
					"Stopped verifying chapter availability"
				);
				break;
			},
		}
	}
	Ok((hidden, restored))
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use models::entity::{
		library, library_config, media, media_metadata, series_metadata,
	};
	use sea_orm::{
		ActiveModelTrait, ActiveValue::Set, DatabaseConnection, PaginatorTrait,
	};
	use stump_media::{virtual_media::VirtualMediaResolver, ContentType};

	use super::*;
	use crate::{
		host::{ProviderHostConfig, VirtualArchive},
		mock::{
			MockSource, ALPHA_CHAPTERS, BETA_CHAPTERS, MOCK_SOURCE_ID, PAGES_PER_CHAPTER,
			SERIES_ALPHA, SERIES_BETA,
		},
		source::ContentRating,
	};

	async fn library(conn: &DatabaseConnection) -> library::Model {
		let config = library_config::ActiveModel {
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
		library::ActiveModel {
			name: Set("Remote".to_string()),
			path: Set("/remote".to_string()),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap()
	}

	async fn host(
		cache_max_bytes: u64,
	) -> (Arc<ProviderHost>, Arc<MockSource>, tempfile::TempDir) {
		let conn = ::tests::db::test_database().await;
		let dir = tempfile::tempdir().unwrap();
		let host = ProviderHost::open(
			Arc::new(conn),
			Vec::new(),
			Vec::new(),
			ProviderHostConfig {
				cache_dir: dir.path().to_path_buf(),
				cache_max_bytes,
				catalog_url: Some("http://127.0.0.1:9/".to_string()),
				definitions_url: Some("http://127.0.0.1:9/".to_string()),
				virtual_series_ttl: std::time::Duration::from_secs(300),
			},
		)
		.await
		.unwrap();
		let source = MockSource::new();
		host.register_source(source.clone());
		(host, source, dir)
	}

	/// A source instance enabled from an NSFW catalog entry, so
	/// `source_is_adult` has something to read.
	async fn adult_host() -> (Arc<ProviderHost>, tempfile::TempDir) {
		let conn = ::tests::db::test_database().await;
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join(crate::catalog::INDEX_FILE_NAME),
			include_bytes!("../tests/fixtures/keiyoushi-index.json"),
		)
		.unwrap();
		let factory = crate::host::SourceFactory {
			implementation: "mock",
			name: "Mock",
			// The fixture's only `CONTENT_WARNING_NSFW` extension.
			catalog_pkg: "eu.kanade.tachiyomi.extension.all.beauty3600000",
			base_url: "https://3600000.xyz",
			build: |row| Ok(MockSource::with_id(&row.id)),
		};
		let host = ProviderHost::open(
			Arc::new(conn),
			vec![factory],
			Vec::new(),
			ProviderHostConfig {
				cache_dir: dir.path().to_path_buf(),
				cache_max_bytes: u64::MAX,
				catalog_url: Some("http://127.0.0.1:9/".to_string()),
				definitions_url: Some("http://127.0.0.1:9/".to_string()),
				virtual_series_ttl: std::time::Duration::from_secs(300),
			},
		)
		.await
		.unwrap();
		host.enable_catalog_source("5498091984644576825", None)
			.await
			.unwrap();
		(host, dir)
	}

	/// The One Piece / "The Witch and the Beast" defect: a chapter the source
	/// cannot serve must not become a book whose every page 404s. It is
	/// skipped, counted, and the reason is reported.
	#[tokio::test]
	async fn unreadable_chapters_are_skipped_and_counted_with_a_reason() {
		let (host, _source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;

		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_BETA)
			.await
			.unwrap();

		assert!(
			materialized.created.is_empty(),
			"no rows for chapters the source cannot serve"
		);
		assert_eq!(materialized.existing, 0);
		assert_eq!(
			materialized
				.skipped
				.iter()
				.map(|skip| (skip.remote_id.as_str(), skip.reason))
				.collect::<Vec<_>>(),
			vec![
				(BETA_CHAPTERS[0], SkipReason::ExternallyHosted),
				(BETA_CHAPTERS[1], SkipReason::NoPages),
				(BETA_CHAPTERS[2], SkipReason::Unavailable),
			]
		);
		assert_eq!(
			media::Entity::find()
				.filter(media::Column::SeriesId.eq(materialized.series.id.clone()))
				.count(host.conn())
				.await
				.unwrap(),
			0,
			"the series materialises with zero books"
		);

		// A re-run reports the same skips rather than accumulating rows.
		let again = refresh_series(&host, &materialized.series.id)
			.await
			.unwrap();
		assert!(again.created.is_empty());
		assert_eq!(again.skipped.len(), BETA_CHAPTERS.len());

		// A readable feed is unaffected.
		let alpha = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert_eq!(alpha.created.len(), ALPHA_CHAPTERS.len());
		assert!(alpha.skipped.is_empty());
	}

	/// Skipping unreadable chapters only fixes new materialisations. A series
	/// materialised before the rule — or before the source retracted a
	/// chapter — keeps rows whose `pages` count promises images the page
	/// manifest now 404s on, which is what "One Piece shows 14 pages on an
	/// unavailable chapter" looked like. A refresh has to take them out of
	/// every listing.
	#[tokio::test]
	async fn refresh_hides_rows_for_chapters_the_source_stopped_serving() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;

		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert_eq!(materialized.created.len(), ALPHA_CHAPTERS.len());
		assert!(materialized.removed.is_empty());
		let mut stale_ids = materialized
			.created
			.iter()
			.map(|row| row.id.clone())
			.collect::<Vec<_>>();
		stale_ids.sort();

		// An unchanged feed hides nothing: `removed` is a reconciliation
		// result, not a refresh side effect.
		let unchanged = refresh_series(&host, &materialized.series.id)
			.await
			.unwrap();
		assert!(unchanged.removed.is_empty());
		assert_eq!(unchanged.existing, ALPHA_CHAPTERS.len());

		source.set_chapters_retracted(true);
		let refreshed = refresh_series(&host, &materialized.series.id)
			.await
			.unwrap();

		let mut removed = refreshed.removed.clone();
		removed.sort();
		assert_eq!(
			removed, stale_ids,
			"every stored row for a now-unreadable chapter is hidden"
		);
		assert_eq!(refreshed.skipped.len(), ALPHA_CHAPTERS.len());
		assert!(refreshed.created.is_empty(), "nothing is re-inserted");
		assert_eq!(
			listed_books(&host, &materialized.series.id).await,
			0,
			"the series stops advertising books it cannot serve"
		);
		assert_eq!(
			media::Entity::find()
				.filter(media::Column::Id.is_in(stale_ids.clone()))
				.count(host.conn())
				.await
				.unwrap(),
			ALPHA_CHAPTERS.len() as u64,
			"the rows survive as tombstones, so progress keyed on them does too"
		);

		// The feed alone cannot resurrect them; the manifest has to resolve.
		source.set_chapters_retracted(false);
		let recovered = refresh_series(&host, &materialized.series.id)
			.await
			.unwrap();
		let mut restored = recovered.restored.clone();
		restored.sort();
		assert_eq!(restored, stale_ids);
		assert!(
			recovered.created.is_empty(),
			"a restored chapter keeps its original id"
		);
		assert_eq!(
			listed_books(&host, &materialized.series.id).await,
			ALPHA_CHAPTERS.len() as u64
		);
	}

	/// The One Piece case exactly: the feed still reports the chapter as
	/// readable (`pages: 14`, no `externalUrl`, `isUnavailable: false`) while
	/// the page manifest answers 404. Flags cannot see that, so a refresh
	/// re-asks the source and hides what it will not serve.
	#[tokio::test]
	async fn refresh_hides_a_chapter_the_feed_still_calls_readable() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		let withheld = virtual_path::media_id(MOCK_SOURCE_ID, ALPHA_CHAPTERS[0]);

		source.withhold_pages(ALPHA_CHAPTERS[0]);
		let refreshed = refresh_series(&host, &materialized.series.id)
			.await
			.unwrap();

		assert!(
			refreshed.skipped.is_empty(),
			"the feed never admits this one is unreadable"
		);
		assert_eq!(
			refreshed.removed,
			vec![withheld.clone()],
			"the manifest 404 is what convicts it"
		);
		assert_eq!(
			listed_books(&host, &materialized.series.id).await,
			(ALPHA_CHAPTERS.len() - 1) as u64
		);
		// A cached manifest must not vouch for it: verification re-asks, so
		// a second refresh reaches the same verdict without a second hide.
		let again = refresh_series(&host, &materialized.series.id)
			.await
			.unwrap();
		assert!(again.removed.is_empty());
		assert!(again.restored.is_empty());
		assert_eq!(
			listed_books(&host, &materialized.series.id).await,
			(ALPHA_CHAPTERS.len() - 1) as u64
		);
	}

	/// Books a client would be offered: what every lane's
	/// `deleted_at IS NULL` filter leaves.
	async fn listed_books(host: &ProviderHost, series_id: &str) -> u64 {
		media::Entity::find()
			.filter(media::Column::SeriesId.eq(series_id))
			.filter(media::Column::DeletedAt.is_null())
			.count(host.conn())
			.await
			.unwrap()
	}

	/// A materialised chapter whose page manifest 404s later is unavailable,
	/// not a missing file: every lane must answer 404 with the source named,
	/// which is what `FileError::Unavailable` gets mapped to.
	#[tokio::test]
	async fn a_chapter_whose_manifest_404s_is_unavailable_not_missing() {
		let (host, _source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();

		// The mock only resolves pages for the three alpha chapters; any
		// other id answers 404 exactly as MangaDex does for a licensed one.
		let error = host
			.pages(MOCK_SOURCE_ID, "alpha-ch9")
			.await
			.expect_err("manifest 404");
		assert!(
			matches!(&error, ProviderError::Unavailable { source_id } if source_id == MOCK_SOURCE_ID),
			"expected Unavailable, got {error:?}"
		);
		assert_eq!(error.to_string(), "Chapter is not available from mock-en");

		let file_error = stump_media::FileError::from(error);
		assert!(
			matches!(&file_error, stump_media::FileError::Unavailable(message)
				if message == "Chapter is not available from mock-en"),
			"expected FileError::Unavailable, got {file_error:?}"
		);

		// Through the resolver a stored row pointing at a vanished chapter
		// takes the same path.
		let path =
			VirtualPath::chapter(MOCK_SOURCE_ID, SERIES_ALPHA, "alpha-ch9").to_string();
		let resolver: &dyn VirtualMediaResolver = host.as_ref();
		assert!(matches!(
			resolver.get_page(&path, 1).await,
			Err(stump_media::FileError::Unavailable(_))
		));
		// A readable chapter is untouched.
		assert!(resolver
			.get_page(&materialized.created[0].path, 1)
			.await
			.is_ok());
	}

	/// `series/{id}/thumbnail` must serve a cover even when the row it was
	/// materialised from carried none: the host falls back to a detail fetch
	/// and writes the URL back.
	#[tokio::test]
	async fn cover_falls_back_to_a_detail_fetch_and_is_written_back() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		let series_id = materialized.series.id.clone();

		// The browse/detail response carried the cover into the row.
		let stored = series_metadata::Entity::find_by_id(&series_id)
			.one(host.conn())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			stored.comic_image.as_deref(),
			Some("http://mock.invalid/covers/alpha.png")
		);
		let (content_type, bytes) = host
			.cover_bytes(MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert!(content_type.is_image());
		assert_eq!(bytes, crate::mock::PNG_PIXEL);
		let detail_fetches = source.detail_fetches();

		// A row whose cover never arrived (an older materialisation) must
		// still serve one.
		let mut blank: series_metadata::ActiveModel = stored.into();
		blank.comic_image = Set(None);
		blank.update(host.conn()).await.unwrap();
		host.cache()
			.invalidate_item(MOCK_SOURCE_ID, &format!("cover:{SERIES_ALPHA}"))
			.await;

		let (content_type, bytes) = host
			.cover_bytes(MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert!(content_type.is_image());
		assert_eq!(bytes, crate::mock::PNG_PIXEL);
		assert_eq!(
			source.detail_fetches(),
			detail_fetches + 1,
			"the missing cover triggered exactly one detail fetch"
		);
		assert_eq!(
			series_metadata::Entity::find_by_id(&series_id)
				.one(host.conn())
				.await
				.unwrap()
				.unwrap()
				.comic_image
				.as_deref(),
			Some("http://mock.invalid/covers/alpha.png"),
			"the fetched URL is written back"
		);
	}

	/// `contentRating` reaches `series_metadata.age_rating`, and an NSFW
	/// catalog source raises every series it materialises to 18 because
	/// Stump has no library-level age rating.
	#[tokio::test]
	async fn content_rating_and_nsfw_sources_set_the_age_rating() {
		let (host, _source, _dir) = host(u64::MAX).await;
		let safe_library = library(host.conn()).await;

		let safe = add_series(&host, &safe_library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert_eq!(
			age_rating_of(&host, &safe.series.id).await,
			None,
			"a safe title stores no rating"
		);

		let adult = add_series(&host, &safe_library.id, MOCK_SOURCE_ID, SERIES_BETA)
			.await
			.unwrap();
		assert_eq!(age_rating_of(&host, &adult.series.id).await, Some(18));

		// The per-rating table, independent of any source.
		let mut details = RemoteSeries {
			remote_id: "x".to_string(),
			content_rating: Some(ContentRating::Suggestive),
			..Default::default()
		};
		assert_eq!(age_rating(&details, false), Some(13));
		details.content_rating = Some(ContentRating::Erotica);
		assert_eq!(age_rating(&details, false), Some(16));
		details.content_rating = Some(ContentRating::Safe);
		assert_eq!(age_rating(&details, false), None);
		assert_eq!(
			age_rating(&details, true),
			Some(18),
			"an adult source overrides a safe title"
		);
		details.content_rating = None;
		assert_eq!(age_rating(&details, false), None);

		// End to end: the same safe title from an NSFW catalog source.
		let (adult_host, _adult_dir) = adult_host().await;
		let adult_library = library(adult_host.conn()).await;
		assert!(adult_host.source_is_adult("mock-all").await);
		assert!(!host.source_is_adult(MOCK_SOURCE_ID).await);
		let from_adult_source =
			add_series(&adult_host, &adult_library.id, "mock-all", SERIES_ALPHA)
				.await
				.unwrap();
		assert_eq!(
			age_rating_of(&adult_host, &from_adult_source.series.id).await,
			Some(18),
			"every series of an NSFW source is rated 18"
		);
	}

	async fn age_rating_of(host: &ProviderHost, series_id: &str) -> Option<i32> {
		series_metadata::Entity::find_by_id(series_id)
			.one(host.conn())
			.await
			.unwrap()
			.unwrap()
			.age_rating
	}

	#[tokio::test]
	async fn add_series_creates_rows_and_refresh_is_idempotent() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;

		let first = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert_eq!(first.created.len(), ALPHA_CHAPTERS.len());
		assert_eq!(first.existing, 0);
		let series_row = &first.series;
		assert_eq!(series_row.name, "Alpha Adventures");
		assert_eq!(series_row.path, "provider://mock-en/alpha");
		assert_eq!(
			series_row.id,
			virtual_path::series_id(MOCK_SOURCE_ID, SERIES_ALPHA)
		);
		assert_eq!(series_row.source_provider.as_deref(), Some(MOCK_SOURCE_ID));
		assert_eq!(series_row.remote_id.as_deref(), Some(SERIES_ALPHA));
		assert_eq!(series_row.library_id.as_deref(), Some(library.id.as_str()));

		let metadata = series_metadata::Entity::find_by_id(&series_row.id)
			.one(host.conn())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(metadata.status.as_deref(), Some("Continuing"));
		assert_eq!(metadata.genres.as_deref(), Some("Action, Comedy"));
		assert_eq!(
			metadata.comic_image.as_deref(),
			Some("http://mock.invalid/covers/alpha.png")
		);

		let chapter3 = &first.created[0];
		assert_eq!(chapter3.name, "Vol. 1 Ch. 3 - Chapter 3");
		assert_eq!(chapter3.extension, "cbz");
		assert_eq!(chapter3.pages, PAGES_PER_CHAPTER as i32);
		assert_eq!(chapter3.path, "provider://mock-en/alpha/alpha-ch3");
		assert_eq!(chapter3.remote_chapter_id.as_deref(), Some("alpha-ch3"));
		// The mock omits the page count for chapter 2: unknown until first open.
		assert_eq!(first.created[1].pages, 0);
		let chapter_metadata = media_metadata::Entity::find()
			.filter(media_metadata::Column::MediaId.eq(chapter3.id.clone()))
			.one(host.conn())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(chapter_metadata.number, Some(Decimal::from(3)));
		assert_eq!(chapter_metadata.volume, Some(1));
		assert_eq!(chapter_metadata.publisher.as_deref(), Some("Mock Scans"));

		let again = refresh_series(&host, &series_row.id).await.unwrap();
		assert!(again.created.is_empty());
		assert_eq!(again.existing, ALPHA_CHAPTERS.len());
		assert_eq!(again.series.id, series_row.id);
		let count = media::Entity::find()
			.filter(media::Column::SeriesId.eq(series_row.id.clone()))
			.all(host.conn())
			.await
			.unwrap()
			.len();
		assert_eq!(count, ALPHA_CHAPTERS.len());
		assert_eq!(source.detail_fetches(), 2);
	}

	#[tokio::test]
	async fn unknown_series_and_disabled_sources_error() {
		let (host, _source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		assert!(matches!(
			add_series(&host, &library.id, MOCK_SOURCE_ID, "nope").await,
			Err(ProviderError::Source(crate::source::SourceError::NotFound(
				_
			)))
		));
		assert!(matches!(
			add_series(&host, &library.id, "missing-source", SERIES_ALPHA).await,
			Err(ProviderError::UnknownSource(_))
		));
	}

	/// Cross-source dedupe: the same work materialised from a second source
	/// gets its own rows plus a link back to the first series.
	#[tokio::test]
	async fn materialising_the_same_work_twice_links_the_duplicate() {
		let (host, _source, _dir) = host(u64::MAX).await;
		let mirror = crate::mock::MockSource::with_id("mock-de");
		host.register_source(mirror);
		let library = library(host.conn()).await;

		let first = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert!(first.duplicate_of.is_none());

		let second = add_series(&host, &library.id, "mock-de", SERIES_ALPHA)
			.await
			.unwrap();
		assert_ne!(second.series.id, first.series.id, "one series per source");
		assert_eq!(
			second.duplicate_of.as_deref(),
			Some(first.series.id.as_str())
		);
		assert_eq!(second.created.len(), ALPHA_CHAPTERS.len());

		let duplicates = crate::identity::duplicates(host.conn()).await.unwrap();
		assert_eq!(duplicates.len(), 1);
		assert_eq!(duplicates[0].link.series_id, second.series.id);
		assert_eq!(duplicates[0].canonical.id, first.series.id);
		// Both series carry the same AniList id, so that is the reason.
		assert_eq!(
			duplicates[0].link.reason,
			crate::identity::REASON_EXTERNAL_KEY
		);
	}

	/// Mode B end to end at the host level: a live browse hands out
	/// deterministic ids without writing rows, the ids are stable across
	/// calls (served from the TTL cache, then re-fetched), materialising one
	/// of them creates the `series`/`media` rows under the same id, and the
	/// first page of its first chapter resolves to image bytes.
	#[tokio::test]
	async fn browse_ids_are_stable_and_materialise_under_the_same_id() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let kind = crate::BrowseKind::Latest;

		let first = host
			.browse(MOCK_SOURCE_ID, &library.id, &kind, 0)
			.await
			.unwrap();
		let second = host
			.browse(MOCK_SOURCE_ID, &library.id, &kind, 0)
			.await
			.unwrap();
		assert!(Arc::ptr_eq(&first, &second), "second browse is a cache hit");
		assert_eq!(first.items.len(), 2);
		assert_eq!(
			series::Entity::find().count(host.conn()).await.unwrap(),
			0,
			"browse must not write series rows"
		);

		let alpha = &first.items[0];
		let stump_id = crate::virtual_path::series_id(MOCK_SOURCE_ID, &alpha.remote_id);
		let origin = host
			.virtual_series_origin(&stump_id)
			.expect("browse indexed id");
		assert_eq!(origin.source_id, MOCK_SOURCE_ID);
		assert_eq!(origin.library_id, library.id);
		assert_eq!(origin.remote_id, SERIES_ALPHA);

		let materialized = host
			.materialise_series(&origin.library_id, &origin.source_id, &origin.remote_id)
			.await
			.unwrap();
		assert_eq!(materialized.series.id, stump_id);
		assert_eq!(materialized.created.len(), ALPHA_CHAPTERS.len());

		let first_chapter = materialized
			.created
			.iter()
			.find(|media| media.remote_chapter_id.as_deref() == Some("alpha-ch1"))
			.expect("chapter 1 materialised");
		assert_eq!(
			first_chapter.id,
			crate::virtual_path::media_id(MOCK_SOURCE_ID, "alpha-ch1")
		);
		let (content_type, bytes) = host
			.page_bytes(MOCK_SOURCE_ID, "alpha-ch1", 0)
			.await
			.unwrap();
		assert!(content_type.is_image());
		assert_eq!(bytes, MockSource::page_bytes("alpha-ch1", 0));
		assert_eq!(source.page_fetches(), 1);
	}

	#[tokio::test]
	async fn pages_resolve_through_cache_and_store_page_count_once() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		let chapter2 = &materialized.created[1];
		assert_eq!(chapter2.pages, 0);

		let resolver: &dyn VirtualMediaResolver = host.as_ref();
		assert!(resolver.owns(&chapter2.path));
		assert!(!resolver.owns("/library/book.cbz"));
		assert_eq!(resolver.get_page_count(&chapter2.path).await.unwrap(), 3);
		let stored = media::Entity::find_by_id(&chapter2.id)
			.one(host.conn())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(stored.pages, PAGES_PER_CHAPTER as i32);

		let (content_type, bytes) = resolver.get_page(&chapter2.path, 2).await.unwrap();
		assert_eq!(content_type, ContentType::PNG);
		assert_eq!(bytes, MockSource::page_bytes("alpha-ch2", 1));
		assert_eq!(source.page_fetches(), 1);
		let (_, again) = resolver.get_page(&chapter2.path, 2).await.unwrap();
		assert_eq!(again, bytes);
		assert_eq!(source.page_fetches(), 1, "second read is a cache hit");

		let types = resolver
			.page_content_types(&chapter2.path, &[1, 2])
			.unwrap();
		assert_eq!(types[&2], ContentType::PNG);
		assert_eq!(
			types[&1],
			ContentType::PNG,
			"manifest extension answers uncached pages"
		);

		assert!(matches!(
			resolver.get_page(&chapter2.path, 9).await,
			Err(stump_media::FileError::PageNotFound {
				page: 9,
				available: 3
			})
		));
		assert!(resolver.get_page("/library/book.cbz", 1).await.is_err());

		// The series path serves the cover through the same cache.
		let (cover_type, cover) = resolver
			.get_page(&materialized.series.path, 1)
			.await
			.unwrap();
		assert_eq!(cover_type, ContentType::PNG);
		assert_eq!(cover, crate::mock::PNG_PIXEL);
		assert_eq!(
			resolver
				.get_page_count(&materialized.series.path)
				.await
				.unwrap(),
			1
		);
	}

	#[tokio::test]
	async fn archive_packs_every_page_as_stored_cbz() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		let chapter = &materialized.created[2];
		let VirtualArchive {
			file_name,
			content_type,
			bytes,
		} = host
			.build_archive(MOCK_SOURCE_ID, "alpha-ch1", &chapter.name)
			.await
			.unwrap();
		assert_eq!(file_name, "Vol. 1 Ch. 1 - Chapter 1.cbz");
		assert_eq!(content_type, ContentType::COMIC_ZIP);
		assert_eq!(source.page_fetches(), PAGES_PER_CHAPTER as usize);

		let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
		assert_eq!(archive.len(), PAGES_PER_CHAPTER as usize);
		let names: Vec<String> = (0..archive.len())
			.map(|index| archive.by_index(index).unwrap().name().to_string())
			.collect();
		assert_eq!(names, vec!["0001.png", "0002.png", "0003.png"]);
		let mut first = archive.by_name("0001.png").unwrap();
		assert_eq!(first.compression(), zip::CompressionMethod::Stored);
		let mut contents = Vec::new();
		std::io::Read::read_to_end(&mut first, &mut contents).unwrap();
		assert_eq!(contents, MockSource::page_bytes("alpha-ch1", 0));

		// Pages fetched for the archive are cache hits afterwards.
		let again = host
			.build_archive(MOCK_SOURCE_ID, "alpha-ch1", "x")
			.await
			.unwrap();
		assert_eq!(again.file_name, "x.cbz");
		assert_eq!(source.page_fetches(), PAGES_PER_CHAPTER as usize);
	}

	#[tokio::test]
	async fn enable_and_disable_instances_round_trip_through_the_registry() {
		fn build(
			row: &models::entity::provider_source::Model,
		) -> Result<Arc<dyn crate::source::Source>, ProviderError> {
			assert_eq!(row.implementation, "mock");
			Ok(MockSource::new())
		}
		let conn = ::tests::db::test_database().await;
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join(crate::catalog::INDEX_FILE_NAME),
			include_bytes!("../tests/fixtures/keiyoushi-index.json"),
		)
		.unwrap();
		let factory = crate::host::SourceFactory {
			implementation: "mock",
			name: "Mock",
			catalog_pkg: "eu.kanade.tachiyomi.extension.all.mangadex",
			base_url: "https://mangadex.org",
			build,
		};
		let host = ProviderHost::open(
			Arc::new(conn),
			vec![factory],
			Vec::new(),
			ProviderHostConfig {
				cache_dir: dir.path().to_path_buf(),
				cache_max_bytes: 1024,
				catalog_url: Some("http://127.0.0.1:9/".to_string()),
				definitions_url: Some("http://127.0.0.1:9/".to_string()),
				virtual_series_ttl: std::time::Duration::from_secs(300),
			},
		)
		.await
		.unwrap();
		assert!(host.sources().is_empty());

		let row = host
			.enable_catalog_source("2499283573021220255", Some("user"))
			.await
			.unwrap();
		assert_eq!(row.id, "mock-en");
		assert_eq!(row.lang, "en");
		assert_eq!(row.catalog_id.as_deref(), Some("2499283573021220255"));
		assert!(row.enabled);
		assert_eq!(host.sources().len(), 1);

		assert!(matches!(
			host.enable_catalog_source("5498091984644576825", None)
				.await,
			Err(ProviderError::NotImplemented(_))
		));
		assert!(matches!(
			host.enable_catalog_source("0", None).await,
			Err(ProviderError::CatalogSourceNotFound(_))
		));

		assert!(host.disable_source("mock-en").await.unwrap());
		assert!(host.sources().is_empty());
		assert!(matches!(
			host.source("mock-en"),
			Err(ProviderError::UnknownSource(_))
		));
		assert!(!host.disable_source("never").await.unwrap());

		// Re-enabling reuses the row and reload picks it up from the database.
		let row = host
			.enable_implementation("mock", "EN", None)
			.await
			.unwrap();
		assert_eq!(row.id, "mock-en");
		assert!(row.updated_at.is_some());
		host.clear_sources();
		host.reload_sources().await.unwrap();
		assert_eq!(host.sources().len(), 1);
	}
}
