//! `ReaderController`: page images, PDF download, progress and mark-read.

use std::sync::Arc;

use axum::{
	extract::Query,
	http::{HeaderMap, StatusCode},
	response::Response,
	routing::{get, post},
	Extension, Json, Router,
};
use models::txn::begin_write;
use models::{
	domain::reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	entity::{bookmark, media_analysis, reading_session, user::AuthUser},
	services::{reading_progress::upsert_reading_session, reading_state},
};
use sea_orm::{prelude::*, QueryOrder};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{
		BookmarkDto, ChapterDto, ChapterInfoDto, FileDimensionDto, KavitaDateTime,
		MangaFormat, MarkChapterReadDto, MarkReadDto, MarkVolumeReadDto,
		MarkVolumesReadDto, ProgressDto,
	},
	errors::{APIError, APIResult},
	filter::SeriesFilterV2Dto,
	ids::{IdKind, KavitaIds, LOOKUP_CHUNK},
	mapper::{
		map_bookmark, map_chapter, map_chapter_info, page_file_name, MediaInput,
		SeriesInput,
	},
	progress::{
		finished_progression, last_progress_at, progression_for_page, KavitaProgress,
	},
};

use super::{
	image::image_response,
	query::{find_media, find_series_input, group_by_media},
	route_ci,
	series::{clear_on_deck_removal, list_series, UserParams},
	KavitaBackend,
};

/// How long `chapter-info` will wait for the one page it measures to size a
/// provider-backed chapter (see [`probed_page_dimensions`]). Deliberately far
/// below the provider client's own 30 s per-request limit
/// (`crates/provider/src/http.rs` `DEFAULT_TIMEOUT`), because a client that
/// cannot get `chapter-info` cannot render a page at all.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterInfoQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
	#[serde(default)]
	include_dimensions: bool,
	/// Kavita pre-extracts a PDF into images when set; Stump renders PDF
	/// pages on demand, so the flag changes nothing here.
	#[serde(default)]
	#[allow(dead_code)]
	extract_pdf: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
	#[serde(default)]
	page: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesQuery {
	#[serde(default)]
	series_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookmarkImageQuery {
	#[serde(default)]
	series_id: Option<i32>,
	#[serde(default)]
	page: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Reader/image", get(reader_image));
	let router = route_ci(router, "/api/Reader/pdf", get(reader_pdf));
	let router = route_ci(router, "/api/Reader/continue-point", get(continue_point));
	let router = route_ci(router, "/api/Reader/get-progress", get(get_progress));
	let router = route_ci(router, "/api/Reader/progress", post(save_progress));
	let router = route_ci(router, "/api/Reader/has-progress", get(has_progress));
	let router = route_ci(router, "/api/Reader/mark-read", post(mark_read));
	let router = route_ci(router, "/api/Reader/mark-unread", post(mark_unread));
	let router = route_ci(
		router,
		"/api/Reader/mark-volume-read",
		post(mark_volume_read),
	);
	let router = route_ci(
		router,
		"/api/Reader/mark-volume-unread",
		post(mark_volume_unread),
	);
	let router = route_ci(router, "/api/Reader/chapter-info", get(chapter_info));
	let router = route_ci(
		router,
		"/api/Reader/mark-chapter-read",
		post(mark_chapter_read),
	);
	let router = route_ci(
		router,
		"/api/Reader/mark-multiple-read",
		post(mark_multiple_read),
	);
	let router = route_ci(
		router,
		"/api/Reader/mark-multiple-unread",
		post(mark_multiple_unread),
	);
	// `kavita-ref` answers `404` for `GET /api/Reader/all-bookmarks` — its
	// attribute routing never matches the path for that verb — where axum
	// would answer `405`; the fallback arm keeps the reference's status.
	let router = route_ci(
		router,
		"/api/Reader/all-bookmarks",
		post(all_bookmarks).get(unmatched_verb),
	);
	let router = route_ci(
		router,
		"/api/Reader/series-bookmarks",
		get(series_bookmarks),
	);
	let router = route_ci(
		router,
		"/api/Reader/chapter-bookmarks",
		get(chapter_bookmarks),
	);
	let router = route_ci(router, "/api/Reader/bookmark", post(add_bookmark));
	let router = route_ci(router, "/api/Reader/unbookmark", post(remove_bookmark));
	route_ci(router, "/api/Reader/bookmark-image", get(bookmark_image))
}

/// A verb `kavita-ref`'s attribute routing does not bind on an otherwise
/// known path: `404`, not `405`.
async fn unmatched_verb() -> StatusCode {
	StatusCode::NOT_FOUND
}

/// Kavita has no image lane for an EPUB. `kavita-ref` 0.9.1.4 answers `404`
/// for `GET /api/Reader/image?chapterId=3&page=N` on its EPUB chapter for
/// every page and every flag combination, including after the
/// `chapter-info?extractPdf=true` extraction that does turn a PDF chapter
/// into `200 image/png`. Stump's EPUB `get_page` hands back the cover for
/// page 1 and the spine document's XHTML for every page after it, so without
/// this guard an image reader — Kamigura's is the only reader it ships
/// (`reader/ReaderScreen.kt:687-692`) — renders the cover on page 0 and then
/// fails to decode `application/xhtml+xml` as an image on every page after
/// it, which is indistinguishable from a hung reader.
fn reject_page_image(media: &MediaInput) -> APIResult<()> {
	if media.format() == MangaFormat::Epub {
		return Err(APIError::NotFound("Chapter has no page images".to_owned()));
	}
	Ok(())
}

async fn reader_image(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ImageQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let chapter_id = query.chapter_id.unwrap_or_default();
	let page = query.page.unwrap_or_default().max(0);
	let (input, index) = find_media(ctx.as_ref(), &user, chapter_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let media = &input.media[index];
	reject_page_image(media)?;
	let pages = media.pages();
	if pages > 0 && page >= pages {
		return Err(APIError::NotFound(format!(
			"Page {page} is out of range (chapter has {pages} pages)"
		)));
	}
	let image = ctx.media_page(&user, &media.media.id, page + 1).await?;
	let file_name = format!(
		"{}-{page}.img",
		media.media.name.replace(['/', '\\', '"'], "_")
	);
	Ok(image_response(image, &file_name))
}

async fn reader_pdf(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterQuery>,
	headers: HeaderMap,
) -> APIResult<Response> {
	let user = auth.user();
	let (input, index) =
		find_media(ctx.as_ref(), &user, query.chapter_id.unwrap_or_default())
			.await?
			.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let media_id = input.media[index].media.id.clone();
	ctx.serve_media_file(auth, headers, &media_id).await
}

/// `ReaderService.GetContinuePoint` over the series' single-chapter volumes
/// in volume order: the first chapter when it has no progress, then the most
/// recently touched partially-read chapter, then `FindNextReadingChapter`.
pub(crate) fn continue_point_for(input: &SeriesInput) -> Option<&MediaInput> {
	let chapters = &input.media;
	let first = chapters.first()?;
	if first.pages_read() == 0 {
		return Some(first);
	}
	let currently_reading = chapters
		.iter()
		.filter(|media| media.pages_read() > 0 && media.pages_read() < media.pages())
		.max_by_key(|media| last_progress_at(media.session.as_ref()));
	if let Some(media) = currently_reading {
		return Some(media);
	}
	let with_progress = chapters
		.iter()
		.filter(|media| media.pages_read() > 0)
		.collect::<Vec<_>>();
	let Some(last_chapter) = with_progress.last().copied() else {
		return Some(first);
	};
	if last_chapter.pages_read() < last_chapter.pages() {
		return Some(last_chapter);
	}
	if let Some(unfinished) = chapters
		.iter()
		.find(|media| media.pages_read() < media.pages())
	{
		return Some(unfinished);
	}
	let last_index = chapters
		.iter()
		.position(|media| media.id == last_chapter.id)
		.unwrap_or(0);
	chapters.get(last_index + 1).or(Some(first))
}

async fn continue_point(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesQuery>,
) -> APIResult<Json<ChapterDto>> {
	let user = auth.user();
	let input =
		find_series_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
			.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?;
	let media = continue_point_for(&input)
		.ok_or_else(|| APIError::NotFound("Series has no chapters".to_owned()))?;
	Ok(Json(map_chapter(media)))
}

fn progress_dto(
	input: &SeriesInput,
	media: &MediaInput,
	scroll_id: Option<String>,
) -> ProgressDto {
	ProgressDto {
		volume_id: media.id,
		chapter_id: media.id,
		page_num: media.pages_read(),
		series_id: input.id,
		library_id: input.library_id,
		book_scroll_id: scroll_id,
		last_modified_utc: KavitaDateTime::from(last_progress_at(media.session.as_ref())),
	}
}

/// Always `200`; an unknown chapter or no progress yields the empty record.
async fn get_progress(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterQuery>,
) -> APIResult<Json<ProgressDto>> {
	let user = auth.user();
	let chapter_id = query.chapter_id.unwrap_or_default();
	let Some((input, index)) = find_media(ctx.as_ref(), &user, chapter_id).await? else {
		return Ok(Json(ProgressDto::empty(chapter_id)));
	};
	let media = &input.media[index];
	if media.session.is_none() {
		return Ok(Json(ProgressDto::empty(chapter_id)));
	}
	let scroll_id =
		KavitaProgress::scroll_id(ctx.conn(), &user.id, &media.media.id).await?;
	Ok(Json(progress_dto(&input, media, scroll_id)))
}

/// `ReaderService.SaveReadingProgress`.
async fn save_progress(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(progress): Json<ProgressDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	save_progress_for(ctx.as_ref(), &user, &progress).await
}

/// The write behind `POST /api/Reader/progress`: the chapter's media gets the
/// user's reading session (shared with every other protocol), the Kavita
/// `bookScrollId` and a unified reading-head update.
pub(crate) async fn save_progress_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	progress: &ProgressDto,
) -> APIResult<StatusCode> {
	let (input, index) = find_media(ctx, user, progress.chapter_id)
		.await?
		.ok_or_else(|| APIError::BadRequest("Could not save progress".to_owned()))?;
	let media = &input.media[index];
	let pages = media.pages();
	let page_num = progress.page_num.clamp(0, pages.max(0));
	let existing = media.session.as_ref();
	let scroll_id = progress
		.book_scroll_id
		.as_deref()
		.map(str::trim)
		.filter(|value| !value.is_empty());
	if existing.is_none() && page_num == 0 {
		return Ok(StatusCode::OK);
	}
	let current_scroll =
		KavitaProgress::scroll_id(ctx.conn(), &user.id, &media.media.id).await?;
	if existing.is_some()
		&& media.pages_read() == page_num
		&& current_scroll.as_deref() == scroll_id
	{
		return Ok(StatusCode::OK);
	}
	let txn = begin_write(ctx.conn()).await?;
	upsert_reading_session(
		&txn,
		user,
		&media.media.id,
		progression_for_page(page_num, pages),
	)
	.await?;
	KavitaProgress::set_scroll_id(&txn, &user.id, &media.media.id, scroll_id).await?;
	reading_state::apply(
		&txn,
		&user.id,
		Publication::from(&media.media),
		ProtocolUpdate {
			protocol: SourceProtocol::Kavita,
			device_id: None,
			updated_at: None,
			position: if pages > 0 {
				Position::Page((page_num + 1).min(pages))
			} else {
				Position::None
			},
			progression: None,
			completed: (pages > 0 && page_num >= pages).then_some(true),
			raw_payload: serde_json::to_value(progress)?,
		},
	)
	.await?;
	txn.commit().await?;
	// `ReaderService.SaveReadingProgress`: a read event puts the series back
	// on deck.
	clear_on_deck_removal(ctx, &user.id, &input.key()).await?;
	Ok(StatusCode::OK)
}

async fn has_progress(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesQuery>,
) -> APIResult<Json<bool>> {
	let user = auth.user();
	let Some(input) =
		find_series_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
	else {
		return Ok(Json(false));
	};
	Ok(Json(input.media.iter().any(|media| media.pages_read() > 0)))
}

pub(crate) async fn mark_media_read_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	media: &[&MediaInput],
) -> APIResult<()> {
	let txn = begin_write(ctx.conn()).await?;
	for item in media {
		upsert_reading_session(
			&txn,
			user,
			&item.media.id,
			finished_progression(item.pages()),
		)
		.await?;
		reading_state::apply(
			&txn,
			&user.id,
			Publication::from(&item.media),
			ProtocolUpdate {
				protocol: SourceProtocol::Kavita,
				device_id: None,
				updated_at: None,
				position: if item.pages() > 0 {
					Position::Page(item.pages())
				} else {
					Position::None
				},
				progression: Some(1.0),
				completed: Some(true),
				raw_payload: serde_json::json!({"markRead": true, "chapterId": item.id}),
			},
		)
		.await?;
	}
	txn.commit().await?;
	Ok(())
}

async fn mark_media_unread(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	media: &[&MediaInput],
) -> APIResult<()> {
	let ids = media
		.iter()
		.map(|item| item.media.id.clone())
		.collect::<Vec<_>>();
	if ids.is_empty() {
		return Ok(());
	}
	let txn = begin_write(ctx.conn()).await?;
	reading_session::Entity::delete_many()
		.filter(reading_session::Column::UserId.eq(user.id.clone()))
		.filter(reading_session::Column::MediaId.is_in(ids.clone()))
		.exec(&txn)
		.await?;
	KavitaProgress::clear(&txn, &user.id, &ids).await?;
	reading_state::clear(&txn, &user.id, &ids, SourceProtocol::Kavita, None).await?;
	txn.commit().await?;
	Ok(())
}

async fn series_input_or_bad_request(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_id: i32,
) -> APIResult<SeriesInput> {
	find_series_input(ctx, user, series_id)
		.await?
		.ok_or_else(|| APIError::BadRequest("Series does not exist".to_owned()))
}

async fn mark_read(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<MarkReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let input = series_input_or_bad_request(ctx.as_ref(), &user, body.series_id).await?;
	let media = input.media.iter().collect::<Vec<_>>();
	mark_media_read_for(ctx.as_ref(), &user, &media).await?;
	clear_on_deck_removal(ctx.as_ref(), &user.id, &input.key()).await?;
	Ok(StatusCode::OK)
}
async fn mark_unread(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<MarkReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let input = series_input_or_bad_request(ctx.as_ref(), &user, body.series_id).await?;
	let media = input.media.iter().collect::<Vec<_>>();
	mark_media_unread(ctx.as_ref(), &user, &media).await?;
	Ok(StatusCode::OK)
}

async fn volume_media<'a>(input: &'a SeriesInput, volume_id: i32) -> Vec<&'a MediaInput> {
	input
		.media
		.iter()
		.filter(|media| media.id == volume_id)
		.collect()
}

async fn mark_volume_read(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<MarkVolumeReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let input = series_input_or_bad_request(ctx.as_ref(), &user, body.series_id).await?;
	let media = volume_media(&input, body.volume_id).await;
	mark_media_read_for(ctx.as_ref(), &user, &media).await?;
	clear_on_deck_removal(ctx.as_ref(), &user.id, &input.key()).await?;
	Ok(StatusCode::OK)
}

async fn mark_volume_unread(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<MarkVolumeReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let input = series_input_or_bad_request(ctx.as_ref(), &user, body.series_id).await?;
	let media = volume_media(&input, body.volume_id).await;
	mark_media_unread(ctx.as_ref(), &user, &media).await?;
	Ok(StatusCode::OK)
}

/// `ReaderController.GetChapterInfo`. `includeDimensions` adds the page
/// dimensions of the chapter plus the double-page pairing derived from them;
/// without the flag both are `null`.
pub(crate) async fn chapter_info_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
	include_dimensions: bool,
) -> APIResult<ChapterInfoDto> {
	let (input, index) = find_media(ctx, user, chapter_id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let media = &input.media[index];
	let dimensions = if include_dimensions {
		Some(page_dimensions(ctx, user, media).await?)
	} else {
		None
	};
	Ok(map_chapter_info(&input, media, dimensions))
}

/// The page dimensions `chapter-info` reports, in page order.
///
/// A stored file has them recorded by Stump's page analysis, so they are
/// exact and per page. A provider-backed chapter never does, and not because
/// the analysis job skips it: the job runs, tries to read every page off the
/// `provider://` path as a file, fails on all of them, and writes the row
/// anyway with an **empty** dimension list
/// (`core/src/filesystem/media/analysis/analyze.rs:219-257,308`). The
/// fallback therefore keys on an empty list rather than on a missing row —
/// keying on the row is what left `pageDimensions: []` on every MangaDex
/// chapter in the fixture.
///
/// Those dimensions are probed instead: page 1 is fetched through the same
/// virtual resolver `GET /api/Reader/image` uses and its header read for a
/// size that is then assumed for every page of the chapter.
///
/// Kamigura needs the array to be non-empty: `reader/ReaderScreen.kt:575-578`
/// turns it into the layout map every page decision reads
/// (`reader/internal/ReaderLayout.kt:33-53` pairs a spread only when neither
/// side is wide, `reader/internal/ReaderPrefetchPlan.kt:73-88` sizes the
/// decode from the source pixels), and `mapper::double_pairs` cannot emit a
/// single pairing without it — an empty array leaves the reader pairing a
/// remote chapter's pages blind and re-decoding every page at viewport size.
/// Assuming page 1's size is the honest approximation available in one fetch:
/// a scanlation chapter is uniform by construction, and the only thing the
/// value drives is spread pairing and decode budget.
async fn page_dimensions(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	media: &MediaInput,
) -> APIResult<Vec<FileDimensionDto>> {
	let recorded = media_analysis::Entity::find()
		.filter(media_analysis::Column::MediaId.eq(media.media.id.clone()))
		.one(ctx.conn())
		.await?
		.map(|analysis| analysis.data.dimensions)
		.unwrap_or_default();
	if !recorded.is_empty() {
		return Ok(recorded
			.iter()
			.enumerate()
			.map(|(index, dimension)| {
				let page_number = i32::try_from(index).unwrap_or(i32::MAX);
				dimension_dto(
					&media.media.name,
					page_number,
					dimension.width,
					dimension.height,
				)
			})
			.collect());
	}
	if !super::is_provider_media(&media.media) {
		return Ok(Vec::new());
	}
	Ok(probed_page_dimensions(ctx, user, media).await)
}

/// One `pageDimensions` entry. `fileName` is the name
/// `GET /api/Reader/image` serves the page under, because Stump records no
/// per-page archive entry name, and `isWide` is Kavita's `width > height`.
fn dimension_dto(
	media_name: &str,
	page_number: i32,
	width: u32,
	height: u32,
) -> FileDimensionDto {
	FileDimensionDto {
		width: i32::try_from(width).unwrap_or(i32::MAX),
		height: i32::try_from(height).unwrap_or(i32::MAX),
		page_number,
		file_name: page_file_name(media_name, page_number),
		is_wide: width > height,
	}
}

/// One measured page of a provider-backed chapter, repeated for every page
/// the chapter reports.
///
/// One page fetch, `imagesize` reading only the header — no decode. Measured
/// against the fixture's MangaDex source: a cold page costs 0.17–0.79 s
/// (2 MB pages) and the very first touch of a chapter adds the source's own
/// chapter resolution (3.2 s worst observed); the host then caches the page,
/// so every later `chapter-info` of that chapter answers in 7–25 ms. It is
/// the same fetch the reader makes for its first page immediately
/// afterwards. Exact per-page dimensions are out of reach here: one remote
/// fetch per page, which no client would wait for.
///
/// The fetch is capped by [`PROBE_TIMEOUT`]. `chapter-info` is the call the
/// reader cannot render without, and the provider client's own limit is 30 s
/// plus rate-limit waits — long enough to look exactly like the hang this
/// probe exists to fix. Past the cap the chapter reports no dimensions,
/// which is what it did before the probe existed.
///
/// The page measured is the **middle** one, not page 1. Kavita's page 0 of a
/// scanlation chapter is usually the group's banner or a colour splash
/// rather than a page of the story — 1268×634 for two of the three MangaDex
/// chapters in the fixture, against a 1671×2400 interior — and measuring it
/// would mark every page of the chapter `isWide`, costing the reader its
/// spreads for the whole chapter. A genuine double-page spread mid-chapter
/// is still reported at the interior size and so still pairs, which is the
/// residual cost of one fetch; it is what an empty array does today for
/// every page.
async fn probed_page_dimensions(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	media: &MediaInput,
) -> Vec<FileDimensionDto> {
	let pages = media.pages();
	if pages <= 0 {
		return Vec::new();
	}
	let probe_page = pages / 2 + 1;
	let fetch = ctx.media_page(user, &media.media.id, probe_page);
	let page = match tokio::time::timeout(PROBE_TIMEOUT, fetch).await {
		Ok(Ok(page)) => page,
		Ok(Err(error)) => {
			tracing::debug!(
				media_id = %media.media.id,
				probe_page,
				?error,
				"could not fetch a page to measure a provider-backed chapter",
			);
			return Vec::new();
		},
		Err(_) => {
			tracing::warn!(
				media_id = %media.media.id,
				probe_page,
				timeout = ?PROBE_TIMEOUT,
				"timed out measuring a provider-backed chapter; reporting no \
				 page dimensions",
			);
			return Vec::new();
		},
	};
	let Ok(size) = imagesize::blob_size(&page.data) else {
		tracing::debug!(
			media_id = %media.media.id,
			probe_page,
			content_type = %page.content_type,
			"a provider-backed chapter's page carries no readable image header",
		);
		return Vec::new();
	};
	let (Ok(width), Ok(height)) = (u32::try_from(size.width), u32::try_from(size.height))
	else {
		return Vec::new();
	};
	(0..pages)
		.map(|page_number| dimension_dto(&media.media.name, page_number, width, height))
		.collect()
}

async fn chapter_info(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterInfoQuery>,
) -> APIResult<Json<ChapterInfoDto>> {
	let user = auth.user();
	Ok(Json(
		chapter_info_for(
			ctx.as_ref(),
			&user,
			query.chapter_id.unwrap_or_default(),
			query.include_dimensions,
		)
		.await?,
	))
}

/// The media a `MarkVolumesReadDto` names. A Stump media item is both the
/// Kavita volume and its chapter, so `volumeIds` and `chapterIds` select from
/// the same set; ids outside the series are ignored, as Kavita's
/// "all volumes must belong to the same Series" contract implies.
fn marked_media<'a>(
	input: &'a SeriesInput,
	body: &MarkVolumesReadDto,
) -> Vec<&'a MediaInput> {
	input
		.media
		.iter()
		.filter(|media| {
			body.volume_ids.contains(&media.id) || body.chapter_ids.contains(&media.id)
		})
		.collect()
}

/// `ReaderController.MarkMultipleAsRead`.
pub(crate) async fn mark_multiple_read_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	body: &MarkVolumesReadDto,
) -> APIResult<()> {
	let input = series_input_or_bad_request(ctx, user, body.series_id).await?;
	let media = marked_media(&input, body);
	mark_media_read_for(ctx, user, &media).await?;
	clear_on_deck_removal(ctx, &user.id, &input.key()).await
}

async fn mark_multiple_read(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<MarkVolumesReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	mark_multiple_read_for(ctx.as_ref(), &user, &body).await?;
	Ok(StatusCode::OK)
}

/// `ReaderController.MarkMultipleAsUnread`: the reset Kamigura's reader fires
/// when a chapter is re-read from the start
/// (`reader/ReaderScreen.kt:391-396`).
pub(crate) async fn mark_multiple_unread_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	body: &MarkVolumesReadDto,
) -> APIResult<()> {
	let input = series_input_or_bad_request(ctx, user, body.series_id).await?;
	let media = marked_media(&input, body);
	mark_media_unread(ctx, user, &media).await
}

async fn mark_multiple_unread(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<MarkVolumesReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	mark_multiple_unread_for(ctx.as_ref(), &user, &body).await?;
	Ok(StatusCode::OK)
}

/// `ReaderController.MarkChapterAsRead`: one chapter of one series.
async fn mark_chapter_read(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<MarkChapterReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	mark_multiple_read_for(
		ctx.as_ref(),
		&user,
		&MarkVolumesReadDto {
			series_id: body.series_id,
			volume_ids: Vec::new(),
			chapter_ids: vec![body.chapter_id],
			generate_reading_session: body.generate_reading_session,
		},
	)
	.await?;
	Ok(StatusCode::OK)
}

/// The user's bookmarks with the Kavita series and chapter they point into.
pub(crate) struct LoadedBookmarks {
	pub inputs: Vec<SeriesInput>,
	/// `(bookmark, Kavita bookmark id, series index, media index)`.
	pub rows: Vec<(bookmark::Model, i32, usize, usize)>,
}

/// Load the user's bookmarks, optionally restricted to `media_scope`, and
/// resolve each one onto the Kavita series and chapter it belongs to.
///
/// Stump stores a bookmark against a media item, which is the Kavita volume
/// and its single chapter at once, so `volumeId == chapterId`. Bookmarks
/// whose media the user can no longer see are dropped rather than reported
/// with a dangling series.
pub(crate) async fn load_bookmarks(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	media_scope: Option<&[String]>,
) -> APIResult<LoadedBookmarks> {
	let mut rows = Vec::new();
	match media_scope {
		Some(media_ids) if media_ids.is_empty() => {
			return Ok(LoadedBookmarks {
				inputs: Vec::new(),
				rows: Vec::new(),
			})
		},
		Some(media_ids) => {
			for chunk in media_ids.chunks(LOOKUP_CHUNK) {
				rows.extend(
					bookmark_query(user)
						.filter(bookmark::Column::MediaId.is_in(chunk.to_vec()))
						.all(ctx.conn())
						.await?,
				);
			}
			rows.sort_by(|left, right| {
				left.created_at
					.cmp(&right.created_at)
					.then_with(|| left.id.cmp(&right.id))
			});
		},
		None => rows.extend(bookmark_query(user).all(ctx.conn()).await?),
	}
	if rows.is_empty() {
		return Ok(LoadedBookmarks {
			inputs: Vec::new(),
			rows: Vec::new(),
		});
	}
	let ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
	let kavita_ids = KavitaIds::resolve_many(ctx.conn(), IdKind::Bookmark, &ids).await?;
	let grouped = group_by_media(ctx, user, rows, |row| row.media_id.as_str()).await?;
	Ok(LoadedBookmarks {
		inputs: grouped.inputs,
		rows: grouped
			.rows
			.into_iter()
			.map(|(row, series_index, media_index)| {
				let id = kavita_ids[&row.id];
				(row, id, series_index, media_index)
			})
			.collect(),
	})
}

fn bookmark_query(user: &AuthUser) -> Select<bookmark::Entity> {
	bookmark::Entity::find_for_user(user)
		.order_by_asc(bookmark::Column::CreatedAt)
		.order_by_asc(bookmark::Column::Id)
}

fn bookmark_dtos(loaded: &LoadedBookmarks) -> Vec<BookmarkDto> {
	loaded
		.rows
		.iter()
		.map(|(row, id, series_index, media_index)| {
			let input = &loaded.inputs[*series_index];
			map_bookmark(*id, row, input, &input.media[*media_index], true)
		})
		.collect()
}

/// `POST /api/Reader/all-bookmarks`: every bookmark of the user. Kavita takes
/// a `SeriesFilterV2Dto` body here and applies it to the bookmarks' series;
/// Stump narrows to the series that filter selects, so a default (empty)
/// filter returns everything, which is what Kamigura sends
/// (`KavitaApi.kt:114`). This route is `POST` only, exactly as on
/// `kavita-ref` — `GET` answers `404` there.
async fn all_bookmarks(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(filter): Json<SeriesFilterV2Dto>,
) -> APIResult<Json<Vec<BookmarkDto>>> {
	let user = auth.user();
	let loaded = load_bookmarks(ctx.as_ref(), &user, None).await?;
	if filter.statements.is_empty() {
		return Ok(Json(bookmark_dtos(&loaded)));
	}
	let (selected, _) = list_series(
		ctx.as_ref(),
		&user,
		&filter,
		UserParams::parse(""),
		None,
		Some(
			&loaded
				.inputs
				.iter()
				.map(SeriesInput::key)
				.collect::<Vec<_>>(),
		),
	)
	.await?;
	let allowed = selected.iter().map(|dto| dto.id).collect::<Vec<_>>();
	Ok(Json(
		bookmark_dtos(&loaded)
			.into_iter()
			.filter(|dto| allowed.contains(&dto.series_id))
			.collect(),
	))
}

/// `GET /api/Reader/series-bookmarks?seriesId` (Inkita, `KavitaApi.kt:126`).
/// An unknown series is an empty list.
async fn series_bookmarks(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesQuery>,
) -> APIResult<Json<Vec<BookmarkDto>>> {
	let user = auth.user();
	let Some(input) =
		find_series_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
	else {
		return Ok(Json(Vec::new()));
	};
	let media_ids = input
		.media
		.iter()
		.map(|media| media.media.id.clone())
		.collect::<Vec<_>>();
	let loaded = load_bookmarks(ctx.as_ref(), &user, Some(&media_ids)).await?;
	Ok(Json(bookmark_dtos(&loaded)))
}

/// `GET /api/Reader/chapter-bookmarks?chapterId`. An unknown chapter is an
/// empty list.
async fn chapter_bookmarks(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterQuery>,
) -> APIResult<Json<Vec<BookmarkDto>>> {
	let user = auth.user();
	let Some((input, index)) =
		find_media(ctx.as_ref(), &user, query.chapter_id.unwrap_or_default()).await?
	else {
		return Ok(Json(Vec::new()));
	};
	let media_ids = vec![input.media[index].media.id.clone()];
	let loaded = load_bookmarks(ctx.as_ref(), &user, Some(&media_ids)).await?;
	Ok(Json(bookmark_dtos(&loaded)))
}

/// The media item a bookmark body names, by its `chapterId` (Kavita's
/// `volumeId` is the same file, so `chapterId` alone identifies it).
async fn bookmark_target(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	body: &BookmarkDto,
) -> APIResult<String> {
	let Some((input, index)) = find_media(ctx, user, body.chapter_id).await? else {
		return Err(APIError::BadRequest("Chapter does not exist".to_owned()));
	};
	Ok(input.media[index].media.id.clone())
}

/// `POST /api/Reader/bookmark`: save a page. Kavita answers `200` with an
/// empty body and is idempotent per `(chapter, page)`.
async fn add_bookmark(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<BookmarkDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let media_id = bookmark_target(ctx.as_ref(), &user, &body).await?;
	let existing = bookmark::Entity::find_for_user(&user)
		.filter(bookmark::Column::MediaId.eq(media_id.clone()))
		.filter(bookmark::Column::Page.eq(body.page))
		.one(ctx.conn())
		.await?;
	if existing.is_none() {
		bookmark::ActiveModel {
			page: sea_orm::Set(Some(body.page)),
			media_id: sea_orm::Set(media_id),
			user_id: sea_orm::Set(user.id.clone()),
			..Default::default()
		}
		.insert(ctx.conn())
		.await?;
	}
	Ok(StatusCode::OK)
}

/// `POST /api/Reader/unbookmark`: drop a saved page. Kavita answers `200`
/// whether or not the bookmark existed.
async fn remove_bookmark(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<BookmarkDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let media_id = bookmark_target(ctx.as_ref(), &user, &body).await?;
	bookmark::Entity::delete_many()
		.filter(bookmark::Column::UserId.eq(user.id.clone()))
		.filter(bookmark::Column::MediaId.eq(media_id))
		.filter(bookmark::Column::Page.eq(body.page))
		.exec(ctx.conn())
		.await?;
	Ok(StatusCode::OK)
}

/// `GET /api/Reader/bookmark-image?seriesId&apiKey&page`: the bookmarked page
/// itself (Kamigura's bookmark grid, `BookmarksScreen.kt:194`).
///
/// Kavita keeps a copy of the bookmarked image on disk and serves that; Stump
/// renders the page from the file it is bookmarked in, so the bookmark must
/// still resolve to a readable chapter, and an EPUB page is refused for the
/// same reason `Reader/image` refuses it. `page` is Kavita's zero-based page
/// number, the same space `Reader/image` uses.
async fn bookmark_image(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<BookmarkImageQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let page = query.page.unwrap_or_default();
	let Some(input) =
		find_series_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
	else {
		return Err(APIError::BadRequest("Series does not exist".to_owned()));
	};
	let media_ids = input
		.media
		.iter()
		.map(|media| media.media.id.clone())
		.collect::<Vec<_>>();
	let loaded = load_bookmarks(ctx.as_ref(), &user, Some(&media_ids)).await?;
	let Some((_, _, _, media_index)) = loaded
		.rows
		.iter()
		.find(|(row, _, _, _)| row.page == Some(page))
	else {
		return Err(APIError::NotFound("Bookmark does not exist".to_owned()));
	};
	let media = &input.media[*media_index];
	reject_page_image(media)?;
	let image = ctx.media_page(&user, &media.media.id, page + 1).await?;
	Ok(image_response(
		image,
		&page_file_name(&media.media.name, page),
	))
}

#[cfg(test)]
mod chapter_info_tests {
	use super::*;
	use crate::dto::{LibraryType, MangaFormat};
	use crate::ids::{IdKind, KavitaIds};
	use crate::routes::series::list_volumes;
	use crate::test_support::{
		auth_user, db, library_of_type, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::analysis::{MediaAnalysisData, PageDimension};
	use models::shared::enums::LibraryType as StumpLibraryType;
	use sea_orm::ActiveValue::Set;

	async fn record_dimensions(
		conn: &sea_orm::DatabaseConnection,
		media_id: &str,
		dimensions: Vec<PageDimension>,
	) {
		media_analysis::ActiveModel {
			data: Set(MediaAnalysisData {
				dimensions,
				content_types: Vec::new(),
			}),
			media_id: Set(media_id.to_owned()),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
	}

	/// A manga chapter: `kavita-ref`
	/// `chapter-info?chapterId=1&includeDimensions=true` for `science comics`.
	/// Stump groups the file into a numbered volume rather than Kavita's
	/// loose-leaf special, so `volumeNumber`/`isSpecial`/`subtitle` follow
	/// `GetChapterInfo`'s numbered-volume branch; every other field matches.
	#[tokio::test]
	async fn manga_chapter_info_carries_the_reference_field_set() {
		let conn = db().await;
		let user_row = fake_data::User::new("reader").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (series, files) = series_with_files(
			&backend.conn,
			&library.id,
			"science comics",
			&[("science_comics_001", "cbz", 3)],
		)
		.await;
		// Two portrait pages then a landscape one: the wide page breaks the
		// double-page pairing, as `ReaderService.GetPairs` does.
		record_dimensions(
			&backend.conn,
			&files[0].id,
			vec![
				PageDimension::new(726, 480),
				PageDimension::new(725, 480),
				PageDimension::new(480, 960),
			],
		)
		.await;
		let series_id = KavitaIds::resolve(&backend.conn, IdKind::Series, &series.id)
			.await
			.unwrap();
		let chapter_id =
			list_volumes(&backend, &user, series_id).await.unwrap()[0].chapters[0].id;

		let info = chapter_info_for(&backend, &user, chapter_id, true)
			.await
			.unwrap();
		assert_eq!(info.chapter_number, "-100000");
		assert_eq!(info.volume_number, "1");
		assert_eq!(info.volume_id, chapter_id);
		assert_eq!(info.series_name, "science comics");
		assert_eq!(info.series_format, MangaFormat::Archive);
		assert_eq!(info.series_id, series_id);
		assert_eq!(info.chapter_title, "");
		assert_eq!(info.pages, 3);
		assert_eq!(info.file_name, "science_comics_001.cbz");
		assert!(!info.is_special);
		assert_eq!(info.subtitle, "Volume 1");
		assert_eq!(info.title, "science comics");
		assert_eq!(
			(info.series_total_pages, info.series_total_pages_read),
			(3, 0)
		);
		// Kavita never assigns `LibraryType` here, so `kavita-ref` reports `0`
		// for a Comic-library chapter too.
		assert_eq!(info.library_type, LibraryType::Manga);

		let dimensions = info.page_dimensions.expect("dimensions were requested");
		assert_eq!(
			dimensions
				.iter()
				.map(|d| (d.page_number, d.width, d.height, d.is_wide))
				.collect::<Vec<_>>(),
			[
				(0, 480, 726, false),
				(1, 480, 725, false),
				(2, 960, 480, true)
			]
		);
		assert_eq!(dimensions[0].file_name, "science_comics_001-0.img");
		assert_eq!(
			info.double_pairs.expect("pairs accompany dimensions"),
			[("0", 0), ("1", 1), ("2", 2)]
				.into_iter()
				.map(|(page, pair)| (page.to_owned(), pair))
				.collect::<std::collections::BTreeMap<_, _>>()
		);

		// Without the flag both are absent, exactly as `kavita-ref` answers
		// `includeDimensions=false`.
		let plain = chapter_info_for(&backend, &user, chapter_id, false)
			.await
			.unwrap();
		assert!(plain.page_dimensions.is_none() && plain.double_pairs.is_none());

		assert!(chapter_info_for(&backend, &user, 999_999, true)
			.await
			.is_err());
	}

	/// A Book-library EPUB: `kavita-ref` `chapter-info?chapterId=4`, whose
	/// chapter is a special of a loose-leaf volume, titled
	/// `"<series> - <chapter title>"` and subtitled with the file stem. With
	/// no recorded page dimensions the arrays are empty rather than absent —
	/// what `kavita-ref` returns for an EPUB.
	#[tokio::test]
	async fn book_chapter_info_uses_the_special_shape() {
		let conn = db().await;
		let user_row = fake_data::User::new("bookish").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		let (_, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Collection",
			&[("alice", "epub", 15)],
		)
		.await;
		let book_id = KavitaIds::resolve(&backend.conn, IdKind::BookSeries, &files[0].id)
			.await
			.unwrap();
		let chapter_id = crate::routes::series::list_volumes(&backend, &user, book_id)
			.await
			.unwrap()[0]
			.chapters[0]
			.id;

		let info = chapter_info_for(&backend, &user, chapter_id, true)
			.await
			.unwrap();
		assert_eq!(info.chapter_number, "-100000");
		assert_eq!(info.volume_number, "-100000");
		assert_eq!(info.series_id, book_id);
		assert_eq!(info.series_format, MangaFormat::Epub);
		assert_eq!(info.file_name, "alice.epub");
		assert!(info.is_special);
		assert_eq!(info.subtitle, "alice");
		assert_eq!(info.series_name, "alice");
		assert_eq!(info.title, "alice");
		assert_eq!(info.pages, 15);
		assert_eq!(info.page_dimensions.as_deref(), Some(&[][..]));
		assert_eq!(info.double_pairs, Some(std::collections::BTreeMap::new()));
	}

	/// A real 8×12 PNG, produced once with `zlib`/`struct` and embedded so a
	/// test can hand the reader bytes with a genuine image header:
	/// `imagesize` reads the header, so a hand-cut prefix would prove
	/// nothing about the real thing.
	const PROBE_PNG: &[u8] = &[
		0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49,
		0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x0c, 0x08, 0x02,
		0x00, 0x00, 0x00, 0xd0, 0xfc, 0x6b, 0xca, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44,
		0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0x8f, 0x03, 0x30, 0x8c, 0x4a, 0xa0, 0x03,
		0x00, 0x22, 0x44, 0x1e, 0xf0, 0xbf, 0x61, 0x58, 0xdb, 0x00, 0x00, 0x00, 0x00,
		0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
	];

	/// A provider-backed chapter's analysis row is written empty (the job
	/// cannot read its remote pages), so `chapter-info` measures one page and
	/// reports that size for every page, instead of the empty array that
	/// leaves Kamigura's reader with no layout information at all
	/// (`reader/ReaderScreen.kt:575-578`).
	///
	/// The page it measures is the middle one: only that page carries a real
	/// image here, so measuring page 1 (a scanlation banner in two of the
	/// three MangaDex chapters in the fixture) would report nothing.
	#[tokio::test]
	async fn provider_backed_chapter_info_measures_its_pages() {
		let conn = db().await;
		let user_row = fake_data::User::new("remote").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Manga).await;
		let (_, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Alpha Adventures",
			&[("Vol. 1 Ch. 1", "cbz", 3)],
		)
		.await;
		models::entity::media::ActiveModel {
			path: Set("provider://mock-en/alpha/alpha-ch1".to_owned()),
			source_provider: Set(Some("mock-en".to_owned())),
			remote_chapter_id: Set(Some("alpha-ch1".to_owned())),
			..files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		// The state the analysis job leaves behind for a provider-backed row:
		// it ran, failed to read every page off the `provider://` path, and
		// wrote the row with no dimensions at all. Keying the fallback on a
		// *missing* row instead of an empty list is what left every MangaDex
		// chapter reporting `pageDimensions: []`.
		record_dimensions(&backend.conn, &files[0].id, Vec::new()).await;
		// Only the middle page (3 pages -> Stump page 2) carries a real
		// image, so this passes only if that is the page measured.
		backend.store_page_image(&files[0].id, 2, PROBE_PNG);
		let chapter_id = KavitaIds::resolve(&backend.conn, IdKind::Media, &files[0].id)
			.await
			.unwrap();

		let info = chapter_info_for(&backend, &user, chapter_id, true)
			.await
			.unwrap();

		assert_eq!(info.pages, 3);
		let dimensions = info.page_dimensions.expect("dimensions were requested");
		assert_eq!(
			dimensions
				.iter()
				.map(|dimension| (
					dimension.page_number,
					dimension.width,
					dimension.height,
					dimension.is_wide,
					dimension.file_name.as_str()
				))
				.collect::<Vec<_>>(),
			vec![
				(0, 8, 12, false, "Vol. 1 Ch. 1-0.img"),
				(1, 8, 12, false, "Vol. 1 Ch. 1-1.img"),
				(2, 8, 12, false, "Vol. 1 Ch. 1-2.img"),
			],
			"page 1's measured size stands in for every page of a remote chapter",
		);
		// Non-empty dimensions are what let `GetPairs` pair a spread at all:
		// page 0 stands alone, then 1 and 2 pair.
		assert_eq!(
			info.double_pairs.expect("pairs follow the dimensions"),
			[("0", 0), ("1", 1), ("2", 1)]
				.into_iter()
				.map(|(page, pair)| (page.to_owned(), pair))
				.collect::<std::collections::BTreeMap<_, _>>(),
		);
	}

	/// A provider that cannot serve page 1, or serves something with no
	/// readable image header, must not take `chapter-info` down with it: the
	/// reader needs `pages` to open the chapter, and opens fine without the
	/// layout hints.
	#[tokio::test]
	async fn an_unmeasurable_provider_chapter_still_answers_chapter_info() {
		let conn = db().await;
		let user_row = fake_data::User::new("offline").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Manga).await;
		let (_, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Beta Adventures",
			&[("Vol. 1 Ch. 1", "cbz", 3)],
		)
		.await;
		models::entity::media::ActiveModel {
			path: Set("provider://mock-en/beta/beta-ch1".to_owned()),
			..files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		// No `store_page_image`: the stand-in bytes carry no image header,
		// which is what a provider error page looks like from here.
		let chapter_id = KavitaIds::resolve(&backend.conn, IdKind::Media, &files[0].id)
			.await
			.unwrap();

		let info = chapter_info_for(&backend, &user, chapter_id, true)
			.await
			.unwrap();

		assert_eq!(info.pages, 3);
		assert_eq!(info.page_dimensions.as_deref(), Some(&[][..]));
	}

	/// `mark-multiple-read` then `mark-multiple-unread` on the same chapter
	/// ids: the first finishes the chapter, the second clears the session the
	/// way Kamigura's "restart chapter" does.
	#[tokio::test]
	async fn mark_multiple_read_and_unread_round_trip() {
		let conn = db().await;
		let user_row = fake_data::User::new("marker").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (series, _) = series_with_files(
			&backend.conn,
			&library.id,
			"science comics",
			&[("one", "cbz", 10), ("two", "cbz", 20)],
		)
		.await;
		let series_id = KavitaIds::resolve(&backend.conn, IdKind::Series, &series.id)
			.await
			.unwrap();
		let volumes = list_volumes(&backend, &user, series_id).await.unwrap();
		let (first, second) = (volumes[0].id, volumes[1].id);

		// `volumeIds` and `chapterIds` address the same Stump media item.
		let body = MarkVolumesReadDto {
			series_id,
			volume_ids: vec![first],
			chapter_ids: vec![second],
			generate_reading_session: false,
		};
		mark_multiple_read_for(&backend, &user, &body)
			.await
			.unwrap();
		let read = list_volumes(&backend, &user, series_id).await.unwrap();
		assert_eq!(
			read.iter().map(|v| v.pages_read).collect::<Vec<_>>(),
			[10, 20]
		);

		mark_multiple_unread_for(
			&backend,
			&user,
			&MarkVolumesReadDto {
				series_id,
				volume_ids: Vec::new(),
				chapter_ids: vec![second],
				generate_reading_session: false,
			},
		)
		.await
		.unwrap();
		let after = list_volumes(&backend, &user, series_id).await.unwrap();
		assert_eq!(
			after.iter().map(|v| v.pages_read).collect::<Vec<_>>(),
			[10, 0],
			"only the named chapter is reset"
		);

		// An id outside the series is ignored, not an error.
		mark_multiple_unread_for(
			&backend,
			&user,
			&MarkVolumesReadDto {
				series_id,
				volume_ids: Vec::new(),
				chapter_ids: vec![999_999],
				generate_reading_session: false,
			},
		)
		.await
		.unwrap();
		assert!(mark_multiple_unread_for(
			&backend,
			&user,
			&MarkVolumesReadDto {
				series_id: 999_999,
				..Default::default()
			},
		)
		.await
		.is_err());
	}

	/// `kavita-ref` 0.9.1.4 answers `404` for `Reader/image` on its EPUB
	/// chapter at every page, so the same chapter that reports a positive
	/// `chapter-info.pages` has no image lane at all — an image-only reader
	/// such as Kamigura's must fail on page 0 rather than be handed a cover
	/// followed by undecodable XHTML. An archive chapter still serves.
	#[tokio::test]
	async fn an_epub_has_no_page_image_lane() {
		let conn = db().await;
		let user_row = fake_data::User::new("reader").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let books = library_of_type(backend.conn(), StumpLibraryType::Book).await;
		let (_, book_files) = series_with_files(
			backend.conn(),
			&books.id,
			"Collection",
			&[("alice", "epub", 15)],
		)
		.await;
		let comics = library_of_type(backend.conn(), StumpLibraryType::Comic).await;
		let (comic_series, comic_files) = series_with_files(
			backend.conn(),
			&comics.id,
			"science comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let epub_chapter =
			KavitaIds::resolve(backend.conn(), IdKind::Media, &book_files[0].id)
				.await
				.unwrap();
		let comic_chapter =
			KavitaIds::resolve(backend.conn(), IdKind::Media, &comic_files[0].id)
				.await
				.unwrap();
		let comic_series_id =
			KavitaIds::resolve(backend.conn(), IdKind::Series, &comic_series.id)
				.await
				.unwrap();
		let book_series_id =
			KavitaIds::resolve(backend.conn(), IdKind::BookSeries, &book_files[0].id)
				.await
				.unwrap();

		// The EPUB still reports pages, exactly as the reference does.
		let info = chapter_info_for(backend.as_ref(), &user, epub_chapter, true)
			.await
			.unwrap();
		assert_eq!((info.series_format, info.pages), (MangaFormat::Epub, 15));

		for page in [0, 1, 14] {
			let (status, _) = crate::test_support::request(
				backend.clone(),
				&user,
				"GET",
				&format!("/api/Reader/image?chapterId={epub_chapter}&page={page}"),
				None,
			)
			.await;
			assert_eq!(status, StatusCode::NOT_FOUND, "epub page {page}");
		}
		let (status, _) = crate::test_support::request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Reader/image?chapterId={comic_chapter}&page=0"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);

		// `bookmark-image` renders through the same projection, so a
		// bookmark on an EPUB page is refused too while a comic's is served.
		for (series_id, chapter_id, expected) in [
			(book_series_id, epub_chapter, StatusCode::NOT_FOUND),
			(comic_series_id, comic_chapter, StatusCode::OK),
		] {
			let (status, _) = crate::test_support::request(
				backend.clone(),
				&user,
				"POST",
				"/api/Reader/bookmark",
				Some(serde_json::json!({
					"chapterId": chapter_id,
					"volumeId": chapter_id,
					"seriesId": series_id,
					"page": 2,
				})),
			)
			.await;
			assert_eq!(status, StatusCode::OK, "bookmarking {chapter_id}");
			let (status, _) = crate::test_support::request(
				backend.clone(),
				&user,
				"GET",
				&format!("/api/Reader/bookmark-image?seriesId={series_id}&page=2"),
				None,
			)
			.await;
			assert_eq!(status, expected, "bookmark image for {chapter_id}");
		}
	}
}

#[cfg(test)]
mod book_progress {
	use super::*;
	use crate::ids::{IdKind, KavitaIds};
	use crate::mapper::SeriesKind;
	use crate::routes::query::{on_deck_removals, SeriesKey};
	use crate::routes::series::{list_volumes, remove_from_on_deck};
	use crate::test_support::{
		auth_user, db, library_of_type, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	/// Progress saved through a book's chapter lands on the media's
	/// `reading_sessions` row (the one OPDS, Komga and the native API share),
	/// reads back through the book series and clears its on-deck removal.
	#[tokio::test]
	async fn progress_through_a_book_chapter_lands_on_the_media_session() {
		let conn = db().await;
		let user_row = fake_data::User::new("bookworm").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		let (_, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Collection",
			&[("alice", "epub", 15), ("leaves", "epub", 383)],
		)
		.await;
		let alice = &files[0];
		let book_id = KavitaIds::resolve(&backend.conn, IdKind::BookSeries, &alice.id)
			.await
			.unwrap();
		let volumes = list_volumes(&backend, &user, book_id).await.unwrap();
		let chapter = &volumes[0].chapters[0];
		remove_from_on_deck(&backend, &user_row.id, &SeriesKey::Book(alice.id.clone()))
			.await
			.unwrap();

		let status = save_progress_for(
			&backend,
			&user,
			&ProgressDto {
				volume_id: volumes[0].id,
				chapter_id: chapter.id,
				page_num: 3,
				series_id: book_id,
				library_id: 0,
				book_scroll_id: None,
				last_modified_utc: KavitaDateTime::default(),
			},
		)
		.await
		.unwrap();
		assert_eq!(status, StatusCode::OK);

		// Exactly one session, on the media row, holding Stump's one-based page.
		let sessions = reading_session::Entity::find()
			.filter(reading_session::Column::UserId.eq(user_row.id.clone()))
			.all(&backend.conn)
			.await
			.unwrap();
		assert_eq!(sessions.len(), 1);
		assert_eq!(sessions[0].media_id, alice.id);
		assert_eq!(sessions[0].end_page, Some(4));
		assert!(!sessions[0].is_complete());

		// The same progress reads back as the book series' single chapter.
		let (input, index) = find_media(&backend, &user, chapter.id)
			.await
			.unwrap()
			.expect("chapter resolves");
		assert_eq!(
			(input.kind, input.id, index),
			(SeriesKind::Book, book_id, 0)
		);
		assert_eq!(input.media[0].pages_read(), 3);
		assert_eq!(map_chapter(&input.media[0]).pages_read, 3);
		let on_deck = crate::routes::series::list_on_deck(
			&backend,
			&user,
			0,
			crate::routes::series::UserParams::parse(""),
		)
		.await
		.unwrap()
		.0;
		assert_eq!(
			on_deck.iter().map(|dto| dto.id).collect::<Vec<_>>(),
			[book_id],
			"the read event put the book back on deck"
		);
		assert!(on_deck_removals(&backend, &user_row.id)
			.await
			.unwrap()
			.is_empty());
	}
}

/// Bookmarks: the read projections Kamigura and Inkita list, and the write
/// pair that fills them.
#[cfg(test)]
mod bookmarks {
	use super::*;
	use crate::test_support::{
		auth_user, db, library_of_type, request, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	#[tokio::test]
	async fn bookmarks_round_trip_through_stumps_bookmark_table() {
		let conn = db().await;
		let user_row = fake_data::User::new("marker").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (series_row, files) = series_with_files(
			&conn,
			&library.id,
			"science comics",
			&[("v01", "cbz", 36), ("v02", "cbz", 20)],
		)
		.await;
		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let series_id =
			KavitaIds::resolve(backend.conn(), IdKind::Series, &series_row.id)
				.await
				.unwrap();
		let first = KavitaIds::resolve(backend.conn(), IdKind::Media, &files[0].id)
			.await
			.unwrap();
		let second = KavitaIds::resolve(backend.conn(), IdKind::Media, &files[1].id)
			.await
			.unwrap();

		// Nothing bookmarked yet: every read is an empty list.
		for uri in [
			"/api/Reader/series-bookmarks?seriesId=",
			"/api/Reader/chapter-bookmarks?chapterId=",
		] {
			let id = if uri.contains("series") {
				series_id
			} else {
				first
			};
			let (status, body) =
				request(backend.clone(), &user, "GET", &format!("{uri}{id}"), None).await;
			assert_eq!(status, StatusCode::OK);
			assert!(body.as_array().unwrap().is_empty(), "{uri}");
		}

		for (chapter, page) in [(first, 3), (first, 4), (second, 1)] {
			let (status, _) = request(
				backend.clone(),
				&user,
				"POST",
				"/api/Reader/bookmark",
				Some(serde_json::json!({
					"chapterId": chapter,
					"volumeId": chapter,
					"seriesId": series_id,
					"page": page,
				})),
			)
			.await;
			assert_eq!(status, StatusCode::OK);
		}
		// Bookmarking the same page twice is idempotent, like Kavita.
		request(
			backend.clone(),
			&user,
			"POST",
			"/api/reader/bookmark",
			Some(serde_json::json!({
				"chapterId": first, "volumeId": first, "seriesId": series_id, "page": 3
			})),
		)
		.await;

		// `all-bookmarks` is POST-only with a filter body, as Kamigura calls it.
		let (status, body) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Reader/all-bookmarks",
			Some(serde_json::json!({})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let all = body.as_array().unwrap();
		assert_eq!(all.len(), 3);
		let dto = &all[0];
		assert_eq!(dto["page"], 3);
		assert_eq!(dto["chapterId"], first);
		assert_eq!(dto["volumeId"], dto["chapterId"]);
		assert_eq!(dto["seriesId"], series_id);
		assert_eq!(dto["imageOffset"], 0);
		assert!(dto["xPath"].is_null());
		assert!(dto["chapterTitle"].is_null());
		assert_eq!(dto["series"]["id"], series_id);
		assert_eq!(dto["series"]["name"], "science comics");
		assert!(dto["id"].as_i64().unwrap() > 0);
		// `GET` is 404 on `kavita-ref`; the profile matches.
		let (status, _) = request(
			backend.clone(),
			&user,
			"GET",
			"/api/Reader/all-bookmarks",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::NOT_FOUND);

		// Per-series and per-chapter narrow the same projection.
		let (_, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Reader/series-bookmarks?seriesId={series_id}"),
			None,
		)
		.await;
		assert_eq!(body.as_array().unwrap().len(), 3);
		let (_, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Reader/chapter-bookmarks?chapterId={second}"),
			None,
		)
		.await;
		let chapter_only = body.as_array().unwrap();
		assert_eq!(chapter_only.len(), 1);
		assert_eq!(chapter_only[0]["chapterId"], second);

		// Unbookmarking drops exactly that page.
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Reader/unbookmark",
			Some(serde_json::json!({
				"chapterId": first, "volumeId": first, "seriesId": series_id, "page": 3
			})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let (_, body) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Reader/all-bookmarks",
			Some(serde_json::json!({})),
		)
		.await;
		let pages = body
			.as_array()
			.unwrap()
			.iter()
			.map(|dto| dto["page"].as_i64().unwrap())
			.collect::<Vec<_>>();
		assert_eq!(pages, vec![4, 1]);

		// Another user's bookmarks are invisible.
		let other = fake_data::User::new("stranger")
			.insert(backend.conn())
			.await;
		let (_, body) = request(
			backend.clone(),
			&auth_user(&other),
			"POST",
			"/api/Reader/all-bookmarks",
			Some(serde_json::json!({})),
		)
		.await;
		assert!(body.as_array().unwrap().is_empty());

		// A body naming no chapter is a bad request, like Kavita's.
		let (status, _) = request(
			backend,
			&user,
			"POST",
			"/api/Reader/bookmark",
			Some(serde_json::json!({"chapterId": 999999, "page": 1})),
		)
		.await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
	}
}
