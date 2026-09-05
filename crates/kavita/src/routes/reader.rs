//! `ReaderController`: page images, PDF download, progress and mark-read.

use std::sync::Arc;

use axum::{
	extract::Query,
	http::{HeaderMap, StatusCode},
	response::Response,
	routing::{get, post},
	Extension, Json, Router,
};
use models::{
	domain::reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	entity::{reading_session, user::AuthUser},
	services::{reading_progress::upsert_reading_session, reading_state},
};
use sea_orm::{prelude::*, TransactionTrait};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{ChapterDto, KavitaDateTime, MarkReadDto, MarkVolumeReadDto, ProgressDto},
	errors::{APIError, APIResult},
	mapper::{map_chapter, MediaInput, SeriesInput},
	progress::{finished_progression, last_progress_at, progression_for_page, KavitaProgress},
};

use super::{
	image::image_response,
	query::{find_media, find_series_by_kavita_id, load_series_input},
	route_ci, KavitaBackend,
};

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
	let router = route_ci(router, "/api/Reader/mark-volume-read", post(mark_volume_read));
	route_ci(router, "/api/Reader/mark-volume-unread", post(mark_volume_unread))
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
	let pages = media.pages();
	if pages > 0 && page >= pages {
		return Err(APIError::NotFound(format!(
			"Page {page} is out of range (chapter has {pages} pages)"
		)));
	}
	let image = ctx.media_page(&user, &media.media.id, page + 1).await?;
	let file_name = format!("{}-{page}.img", media.media.name.replace(['/', '\\', '"'], "_"));
	Ok(image_response(image, &file_name))
}

async fn reader_pdf(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterQuery>,
	headers: HeaderMap,
) -> APIResult<Response> {
	let user = auth.user();
	let (input, index) = find_media(ctx.as_ref(), &user, query.chapter_id.unwrap_or_default())
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
	let row = find_series_by_kavita_id(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
		.await?
		.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?;
	let input = load_series_input(ctx.as_ref(), &user, row).await?;
	let media = continue_point_for(&input)
		.ok_or_else(|| APIError::NotFound("Series has no chapters".to_owned()))?;
	Ok(Json(map_chapter(media)))
}

fn progress_dto(input: &SeriesInput, media: &MediaInput, scroll_id: Option<String>) -> ProgressDto {
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
	let scroll_id = KavitaProgress::scroll_id(ctx.conn(), &user.id, &media.media.id).await?;
	Ok(Json(progress_dto(&input, media, scroll_id)))
}

/// `ReaderService.SaveReadingProgress`.
async fn save_progress(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(progress): Json<ProgressDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let (input, index) = find_media(ctx.as_ref(), &user, progress.chapter_id)
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
	let current_scroll = KavitaProgress::scroll_id(ctx.conn(), &user.id, &media.media.id).await?;
	if existing.is_some() && media.pages_read() == page_num && current_scroll.as_deref() == scroll_id {
		return Ok(StatusCode::OK);
	}
	let txn = ctx.conn().begin().await?;
	upsert_reading_session(&txn, &user, &media.media.id, progression_for_page(page_num, pages)).await?;
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
			raw_payload: serde_json::to_value(&progress)?,
		},
	)
	.await?;
	txn.commit().await?;
	Ok(StatusCode::OK)
}

async fn has_progress(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesQuery>,
) -> APIResult<Json<bool>> {
	let user = auth.user();
	let Some(row) = find_series_by_kavita_id(ctx.as_ref(), &user, query.series_id.unwrap_or_default()).await?
	else {
		return Ok(Json(false));
	};
	let input = load_series_input(ctx.as_ref(), &user, row).await?;
	Ok(Json(input.media.iter().any(|media| media.pages_read() > 0)))
}

pub(crate) async fn mark_media_read_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	media: &[&MediaInput],
) -> APIResult<()> {
	let txn = ctx.conn().begin().await?;
	for item in media {
		upsert_reading_session(&txn, user, &item.media.id, finished_progression(item.pages())).await?;
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

async fn mark_media_unread(ctx: &dyn KavitaBackend, user: &AuthUser, media: &[&MediaInput]) -> APIResult<()> {
	let ids = media
		.iter()
		.map(|item| item.media.id.clone())
		.collect::<Vec<_>>();
	if ids.is_empty() {
		return Ok(());
	}
	let txn = ctx.conn().begin().await?;
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
	let row = find_series_by_kavita_id(ctx, user, series_id)
		.await?
		.ok_or_else(|| APIError::BadRequest("Series does not exist".to_owned()))?;
	load_series_input(ctx, user, row).await
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
