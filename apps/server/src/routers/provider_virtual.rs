//! Mode B: virtual-library live-browse helpers shared by the Komga adapter
//! and the OPDS v1.2 router.
//!
//! A virtual library is a `libraries` row backed by a provider source
//! (`source_provider`, `path = provider://<source>`). Browsing it hits the
//! source live through the [`ProviderHost`] and returns deterministic ids;
//! nothing is written until a client opens a series' books, at which point
//! [`materialise_virtual_series`] creates ordinary `series`/`media` rows
//! under the same ids. This module is the single place where provider
//! results are turned into protocol DTOs.

use std::sync::Arc;

use models::entity::{library, series, user::AuthUser};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use stump_core::Ctx;
use stump_komga::{
	KomgaAuthor, KomgaLibraryId, KomgaSeries, KomgaSeriesBookMetadata, KomgaSeriesId,
	KomgaSeriesMetadata, KomgaSeriesStatus,
};
use stump_media::ContentType;
use stump_provider::virtual_path;
use stump_provider::{BrowseKind, ProviderHost, RemoteSeries, SeriesStatus};

/// The host, when providers are compiled in and enabled at runtime.
pub fn provider_host(ctx: &Ctx) -> Option<Arc<ProviderHost>> {
	ctx.provider_host()
}

/// Whether `user`'s age restriction admits a row rated `age_rating`.
///
/// Stored rows are filtered in SQL by
/// [`series::Entity::find_for_user`](models::entity::series::Entity), which
/// compares `series_metadata.age_rating` against the user's
/// `age_restrictions` row. A live-browse row has no `series_metadata` to
/// join, so the identical rule is applied to the remote rating here —
/// before the title can be listed, opened, or materialised.
pub fn age_restriction_allows(user: &AuthUser, age_rating: Option<i32>) -> bool {
	let Some(restriction) = user.age_restriction.as_ref() else {
		return true;
	};
	match age_rating {
		Some(rating) => rating <= restriction.age,
		// `restrict_on_unset` is the "unrated means hidden" switch stored
		// rows already honour.
		None => !restriction.restrict_on_unset,
	}
}

/// The age rating a remote series would materialise with, so live rows carry
/// the same value the stored row will.
pub fn remote_age_rating(remote: &RemoteSeries, adult_source: bool) -> Option<i32> {
	stump_provider::materialize::age_rating(remote, adult_source)
}

/// Whether the catalog marks the source instance as adult. Stump has no
/// library-level age rating, so this is applied per series.
pub async fn source_is_adult(ctx: &Ctx, source_id: &str) -> bool {
	match provider_host(ctx) {
		Some(host) => host.source_is_adult(source_id).await,
		None => false,
	}
}

/// The enabled source instance backing a virtual library, if the host can
/// serve it. `None` for ordinary libraries or disabled sources.
pub async fn virtual_library_source(ctx: &Ctx, library_id: &str) -> Option<String> {
	let host = provider_host(ctx)?;
	let row = library::Entity::find_by_id(library_id)
		.one(ctx.conn.as_ref())
		.await
		.ok()
		.flatten()?;
	let source_id = row.source_provider?;
	host.source(&source_id).ok()?;
	Some(source_id)
}

/// Map a client request onto the Mihon-shaped browse kinds.
///
/// * `fullTextSearch` (Komga condition DSL) becomes a source search.
/// * Engagement sorts (`readCount`, `booksCount`, `readProgress`) become the
///   popular feed.
/// * Everything else — including the unsorted default — is the latest feed.
pub fn browse_kind(full_text_search: Option<&str>, sorts: &[String]) -> BrowseKind {
	if let Some(query) = full_text_search.map(str::trim).filter(|t| !t.is_empty()) {
		return BrowseKind::Search {
			query: query.to_string(),
			filters: Vec::new(),
		};
	}
	let engagement = sorts.iter().any(|sort| {
		let field = sort.split(',').next().unwrap_or_default();
		matches!(field, "readCount" | "booksCount" | "readProgress")
	});
	if engagement {
		BrowseKind::Popular
	} else {
		BrowseKind::Latest
	}
}

/// One page of live browse results for a virtual library.
pub async fn browse_page(
	ctx: &Ctx,
	source_id: &str,
	library_id: &str,
	kind: &BrowseKind,
	page: u32,
) -> Result<Arc<stump_provider::SourcePage<RemoteSeries>>, String> {
	let host =
		provider_host(ctx).ok_or_else(|| "Provider host is not enabled".to_string())?;
	host.browse(source_id, library_id, kind, page)
		.await
		.map_err(|error| error.to_string())
}

/// The Komga DTO for a remote series. The id is deterministic
/// (`uuid5(source, remote_id)`); no rows are written for it. The book count
/// is unknown until the series is materialised, so live cards report zero
/// books.
///
/// `adult_source` is the catalog's NSFW flag for the source, so the card's
/// `ageRating` matches what materialisation will store and per-user age
/// restriction filters the same way on both.
pub fn map_remote_series(
	source_id: &str,
	library_id: &str,
	remote: &RemoteSeries,
	adult_source: bool,
) -> KomgaSeries {
	let stump_id = virtual_path::series_id(source_id, &remote.remote_id);
	let now = chrono::Utc::now();
	let title = remote.title.clone();
	let summary = remote.description.clone().unwrap_or_default();

	let mut authors: Vec<KomgaAuthor> = remote
		.authors
		.iter()
		.map(|name| KomgaAuthor {
			name: name.clone(),
			role: "writer".to_string(),
		})
		.collect();
	authors.extend(remote.artists.iter().map(|name| KomgaAuthor {
		name: name.clone(),
		role: "penciller".to_string(),
	}));

	KomgaSeries {
		id: KomgaSeriesId::new(stump_id),
		library_id: KomgaLibraryId::new(library_id.to_string()),
		name: title.clone(),
		url: format!("{}{}/{}", virtual_path::SCHEME, source_id, remote.remote_id),
		books_count: 0,
		books_read_count: 0,
		books_unread_count: 0,
		books_in_progress_count: 0,
		metadata: KomgaSeriesMetadata {
			status: map_series_status(remote.status),
			status_lock: false,
			title: title.clone(),
			alternate_titles: Vec::new(),
			alternate_titles_lock: false,
			title_lock: false,
			title_sort: title.clone(),
			title_sort_lock: false,
			summary: summary.clone(),
			summary_lock: false,
			reading_direction: None,
			reading_direction_lock: false,
			publisher: String::new(),
			publisher_lock: false,
			age_rating: remote_age_rating(remote, adult_source),
			age_rating_lock: false,
			language: remote.original_language.clone(),
			language_lock: false,
			genres: remote.genres.clone(),
			genres_lock: false,
			tags: Vec::new(),
			tags_lock: false,
			total_book_count: None,
			total_book_count_lock: false,
			sharing_labels: Vec::new(),
			sharing_labels_lock: false,
			links: Vec::new(),
			links_lock: false,
		},
		deleted: false,
		oneshot: false,
		books_metadata: KomgaSeriesBookMetadata {
			authors,
			tags: remote.genres.clone(),
			release_date: None,
			summary,
			summary_number: String::new(),
			created: now,
			last_modified: now,
		},
		created: now,
		last_modified: now,
		file_last_modified: now,
	}
}

fn map_series_status(status: SeriesStatus) -> KomgaSeriesStatus {
	match status {
		SeriesStatus::Ongoing | SeriesStatus::Unknown => KomgaSeriesStatus::Ongoing,
		SeriesStatus::Completed => KomgaSeriesStatus::Ended,
		SeriesStatus::Hiatus => KomgaSeriesStatus::Hiatus,
		SeriesStatus::Cancelled => KomgaSeriesStatus::Abandoned,
	}
}

/// Whether `user` may see `library_id`; live results never bypass the
/// per-user library filter stored rows go through.
async fn user_can_access_library(ctx: &Ctx, user: &AuthUser, library_id: &str) -> bool {
	library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(library_id))
		.one(ctx.conn.as_ref())
		.await
		.ok()
		.flatten()
		.is_some()
}

/// Live details for a series id that is not materialised.
///
/// Returns `None` when the id belongs to a stored (or non-provider) series
/// and the ordinary database path should serve it, when the user cannot see
/// the virtual library the id was browsed from, or when the user's age
/// restriction hides the remote rating.
pub async fn virtual_series_by_id(
	ctx: &Ctx,
	user: &AuthUser,
	series_id: &str,
) -> Option<KomgaSeries> {
	if series_exists(ctx, series_id).await {
		return None;
	}
	let host = provider_host(ctx)?;
	let origin = host.virtual_series_origin(series_id)?;
	if !user_can_access_library(ctx, user, &origin.library_id).await {
		return None;
	}
	let details = host
		.remote_series_details(&origin.source_id, &origin.remote_id)
		.await
		.ok()?;
	let adult_source = host.source_is_adult(&origin.source_id).await;
	if !age_restriction_allows(user, remote_age_rating(&details, adult_source)) {
		return None;
	}
	Some(map_remote_series(
		&origin.source_id,
		&origin.library_id,
		&details,
		adult_source,
	))
}

/// Materialise a live-only virtual series on first books/pages access.
///
/// * `None` — a row already exists (materialised or ordinary; the caller's
///   per-user database path decides visibility), the id was never served
///   by a live browse, the user cannot see the virtual library, or the
///   user's age restriction hides the remote rating.
/// * `Some(Ok(row))` — the series row now exists under the deterministic id.
/// * `Some(Err(_))` — the source fetch failed.
pub async fn materialise_virtual_series(
	ctx: &Ctx,
	user: &AuthUser,
	series_id: &str,
) -> Option<Result<series::Model, String>> {
	if series_exists(ctx, series_id).await {
		return None;
	}

	let host = provider_host(ctx)?;
	let origin = host.virtual_series_origin(series_id)?;
	if !user_can_access_library(ctx, user, &origin.library_id).await {
		return None;
	}
	// A restricted title must not be materialised either: writing the rows
	// is how a user would otherwise reach its books.
	let details = host
		.remote_series_details(&origin.source_id, &origin.remote_id)
		.await
		.ok()?;
	let adult_source = host.source_is_adult(&origin.source_id).await;
	if !age_restriction_allows(user, remote_age_rating(&details, adult_source)) {
		return None;
	}
	Some(
		host.materialise_series(&origin.library_id, &origin.source_id, &origin.remote_id)
			.await
			.map(|materialized| materialized.series)
			.map_err(|error| error.to_string()),
	)
}

/// Cover bytes for a provider-backed series, whether materialised or
/// live-only. `None` for non-provider series, and for any series the user's
/// library filter or age restriction hides.
pub async fn virtual_series_cover(
	ctx: &Ctx,
	user: &AuthUser,
	series_id: &str,
) -> Option<Result<(ContentType, Vec<u8>), String>> {
	let host = provider_host(ctx)?;
	// A stored row that `find_for_user` filtered out must not fall through to
	// the live path: the browse cache would happily serve its cover.
	if series_exists(ctx, series_id).await {
		let row = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id))
			.one(ctx.conn.as_ref())
			.await
			.ok()
			.flatten()?;
		let (Some(source_id), Some(remote_id)) =
			(row.source_provider.clone(), row.remote_id.clone())
		else {
			return None;
		};
		return Some(
			host.cover_bytes(&source_id, &remote_id)
				.await
				.map_err(|error| error.to_string()),
		);
	}
	let origin = host.virtual_series_origin(series_id)?;
	if !user_can_access_library(ctx, user, &origin.library_id).await {
		return None;
	}
	let details = host
		.remote_series_details(&origin.source_id, &origin.remote_id)
		.await
		.ok()?;
	let adult_source = host.source_is_adult(&origin.source_id).await;
	if !age_restriction_allows(user, remote_age_rating(&details, adult_source)) {
		return None;
	}
	Some(
		host.cover_bytes(&origin.source_id, &origin.remote_id)
			.await
			.map_err(|error| error.to_string()),
	)
}

async fn series_exists(ctx: &Ctx, series_id: &str) -> bool {
	series::Entity::find_by_id(series_id)
		.one(ctx.conn.as_ref())
		.await
		.ok()
		.flatten()
		.is_some()
}
