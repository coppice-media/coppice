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
	entity::{media_analysis, reading_session, user::AuthUser},
	services::{reading_progress::upsert_reading_session, reading_state},
};
use sea_orm::{prelude::*, TransactionTrait};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{
		ChapterDto, ChapterInfoDto, FileDimensionDto, KavitaDateTime, MarkChapterReadDto,
		MarkReadDto, MarkVolumeReadDto, MarkVolumesReadDto, ProgressDto,
	},
	errors::{APIError, APIResult},
	mapper::{map_chapter, map_chapter_info, page_file_name, MediaInput, SeriesInput},
	progress::{
		finished_progression, last_progress_at, progression_for_page, KavitaProgress,
	},
};

use super::{
	image::image_response,
	query::{find_media, find_series_input},
	route_ci,
	series::clear_on_deck_removal,
	KavitaBackend,
};

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
	route_ci(
		router,
		"/api/Reader/mark-multiple-unread",
		post(mark_multiple_unread),
	)
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
	let txn = ctx.conn().begin().await?;
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
	let txn = ctx.conn().begin().await?;
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
/// dimensions Stump's page analysis recorded, plus the double-page pairing
/// derived from them; without recorded dimensions the arrays are empty
/// (Kavita answers `[]`/`{}` for an EPUB too) and without the flag they are
/// `null`.
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
		Some(page_dimensions(ctx, &media.media.id, &media.media.name).await?)
	} else {
		None
	};
	Ok(map_chapter_info(&input, media, dimensions))
}

/// The recorded page dimensions of a media item, in page order.
async fn page_dimensions(
	ctx: &dyn KavitaBackend,
	media_id: &str,
	media_name: &str,
) -> APIResult<Vec<FileDimensionDto>> {
	let Some(analysis) = media_analysis::Entity::find()
		.filter(media_analysis::Column::MediaId.eq(media_id.to_owned()))
		.one(ctx.conn())
		.await?
	else {
		return Ok(Vec::new());
	};
	Ok(analysis
		.data
		.dimensions
		.iter()
		.enumerate()
		.map(|(index, dimension)| {
			let page_number = i32::try_from(index).unwrap_or(i32::MAX);
			FileDimensionDto {
				width: i32::try_from(dimension.width).unwrap_or(i32::MAX),
				height: i32::try_from(dimension.height).unwrap_or(i32::MAX),
				page_number,
				file_name: page_file_name(media_name, page_number),
				is_wide: dimension.width > dimension.height,
			}
		})
		.collect())
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
