//! `GET /api/v2/reading/continue` — the caller's most recent reading heads.
//!
//! OPDS 2.0 already names the books a user is part-way through
//! (`/opds/v2.0/books/keep-reading`), but an OPDS publication carries its
//! position only as a *link* (`rel=http://www.cantook.com/api/progression`),
//! so a device dashboard that wants to draw a progress bar needs one extra
//! request per row. This route answers the same question in a single request,
//! and it reads [`reading_heads`] — the unified reading state, one canonical
//! head per `(user, media)` — rather than `reading_sessions`, so a position
//! that arrived over KOReader, Kobo, Komga, or ABS is projected the same way.
//!
//! [`reading_heads`]: models::entity::reading_head
//!
//! The row carries `koreaderHash` because that is the one identifier a
//! KOReader-class client already has for a *local* file: it is the partial MD5
//! KOReader stores as `partial_md5_checksum` and the value Stump matches in
//! `media.koreader_hash`. Handing it back lets a device map the document it has
//! open onto a Stump book without a second lookup or a title guess.

use axum::{
	extract::{Query, State},
	middleware,
	routing::get,
	Extension, Json, Router,
};
use models::{
	domain::reading_state::SourceProtocol,
	entity::{media, media_metadata, reading_head, series},
};
use sea_orm::{prelude::*, FromQueryResult, QueryOrder, QuerySelect, QueryTrait};
use serde::{Deserialize, Serialize};
use stump_auth::AuthContext;

use crate::{
	config::state::AppState, errors::APIResult, middleware::auth::auth_middleware,
};

/// The number of heads returned when the caller does not ask for a count.
const DEFAULT_LIMIT: u64 = 10;
/// The most heads one request will ever return.
const MAX_LIMIT: u64 = 50;

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	Router::new()
		.route("/reading/continue", get(continue_reading))
		.layer(middleware::from_fn_with_state(app_state, auth_middleware))
}

#[derive(Debug, Default, Deserialize)]
pub struct ContinueReadingQuery {
	limit: Option<u64>,
}

/// One book the caller is part-way through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueReadingItem {
	pub media_id: String,
	pub name: String,
	/// Display title: the metadata title when present, else `name` (the file
	/// or folder name, e.g. "Enders Game (MP3)"). Same rule as GraphQL
	/// `Media.resolvedName`.
	pub title: String,
	/// The file extension without the leading period, so a client can decide
	/// whether it can open the book before downloading it.
	pub extension: String,
	pub series_id: Option<String>,
	pub series_name: Option<String>,
	/// `media.koreader_hash`: the partial MD5 that identifies this book to a
	/// KOReader client. `None` when Stump never hashed the file (archives, or
	/// a library scanned without hashing).
	pub koreader_hash: Option<String>,
	/// Whole-publication progression in `0..=1`.
	pub progression: f64,
	/// The 1-based page when the head is page-addressed.
	pub page: Option<i32>,
	/// `media.pages`; `-1` for a format Stump does not page.
	pub pages: i32,
	/// Milliseconds from the start of the publication for a time-addressed
	/// (audio) head. Never a page ordinal.
	pub position_ms: Option<i64>,
	/// The effective source time of the winning update, RFC 3339.
	pub updated_at: String,
	/// Which protocol wrote the winning update.
	pub source_protocol: SourceProtocol,
	/// Server-relative; same authentication as this route.
	pub download_url: String,
	/// Server-relative; same authentication as this route.
	pub thumbnail_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueReadingResponse {
	/// Most recently read first.
	pub items: Vec<ContinueReadingItem>,
}

#[derive(Debug, FromQueryResult)]
struct ContinueReadingBook {
	id: String,
	name: String,
	metadata_title: Option<String>,
	extension: String,
	pages: i32,
	koreader_hash: Option<String>,
	series_id: Option<String>,
	series_name: Option<String>,
}

/// `GET /api/v2/reading/continue?limit=N`
///
/// Completed heads are excluded: "continue reading" is the unfinished set, and
/// a client that wants the finished ones is asking a different question.
///
/// Visibility is enforced by filtering the heads through
/// [`media::Entity::find_for_user`], so an age-restricted or out-of-scope book
/// is absent rather than listed without a title. The limit is applied to that
/// already-filtered set, so a hidden book never eats a row.
pub(crate) async fn continue_reading(
	State(ctx): State<AppState>,
	Query(params): Query<ContinueReadingQuery>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<ContinueReadingResponse>> {
	let user = req.user();
	let conn = ctx.conn.as_ref();
	let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

	let visible_books = media::Entity::find_for_user(&user)
		.select_only()
		.column(media::Column::Id)
		.into_query();

	let heads = reading_head::Entity::find()
		.filter(reading_head::Column::UserId.eq(user.id.clone()))
		.filter(reading_head::Column::Completed.eq(false))
		.filter(reading_head::Column::MediaId.in_subquery(visible_books))
		.order_by_desc(reading_head::Column::UpdatedAt)
		.limit(limit)
		.all(conn)
		.await?;

	if heads.is_empty() {
		return Ok(Json(ContinueReadingResponse { items: vec![] }));
	}

	let media_ids = heads
		.iter()
		.map(|head| head.media_id.clone())
		.collect::<Vec<_>>();
	let books = media::Entity::find()
		.select_only()
		.columns([
			media::Column::Id,
			media::Column::Name,
			media::Column::Extension,
			media::Column::Pages,
			media::Column::KoreaderHash,
			media::Column::SeriesId,
		])
		.column_as(series::Column::Name, "series_name")
		.column_as(media_metadata::Column::Title, "metadata_title")
		.left_join(series::Entity)
		.left_join(media_metadata::Entity)
		.filter(media::Column::Id.is_in(media_ids))
		.into_model::<ContinueReadingBook>()
		.all(conn)
		.await?;
	let books = books
		.into_iter()
		.map(|book| (book.id.clone(), book))
		.collect::<std::collections::HashMap<_, _>>();

	// The head order is the answer; the book fetch is only a name lookup, so
	// it never reorders the list.
	let items = heads
		.into_iter()
		.filter_map(|head| {
			let book = books.get(&head.media_id)?;
			Some(ContinueReadingItem {
				media_id: head.media_id.clone(),
				name: book.name.clone(),
				title: book
					.metadata_title
					.clone()
					.filter(|title| !title.trim().is_empty())
					.unwrap_or_else(|| book.name.clone()),
				extension: book.extension.clone(),
				series_id: book.series_id.clone(),
				series_name: book.series_name.clone(),
				koreader_hash: book.koreader_hash.clone(),
				progression: head.progression,
				page: head.page,
				pages: book.pages,
				position_ms: head.position_ms,
				updated_at: head.updated_at.to_rfc3339(),
				source_protocol: head.source_protocol,
				download_url: format!("/api/v2/media/{}/file", head.media_id),
				thumbnail_url: format!("/api/v2/media/{}/thumbnail", head.media_id),
			})
		})
		.collect::<Vec<_>>();

	Ok(Json(ContinueReadingResponse { items }))
}
