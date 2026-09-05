//! `TachiyomiController` (Mihon tracker): latest read chapter and
//! mark-until-as-read with Kavita's `volume / 10000` encoding.

use std::sync::Arc;

use axum::{
	extract::Query,
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::{get, post},
	Extension, Json, Router,
};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{ChapterDto, MangaFileDto, TachiyomiChapterDto},
	errors::{APIError, APIResult},
	mapper::{map_chapter, MediaInput, SeriesInput},
};

use super::{
	query::{find_series_by_kavita_id, load_series_input},
	reader::{continue_point_for, mark_media_read_for},
	route_ci, KavitaBackend,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesQuery {
	#[serde(default)]
	series_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkUntilQuery {
	#[serde(default)]
	series_id: Option<i32>,
	#[serde(default)]
	chapter_number: Option<f32>,
	#[serde(default)]
	generate_reading_sessions: Option<bool>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Tachiyomi/latest-chapter", get(latest_chapter));
	route_ci(
		router,
		"/api/Tachiyomi/mark-chapter-until-as-read",
		post(mark_chapter_until_as_read),
	)
}

/// `TachiyomiService.CreateTachiyomiChapterDto`: a chapter whose `number`
/// encodes a volume number as `number / 10000` (`R` round-trip formatting).
pub(crate) fn encoded_volume_chapter(
	template: &MediaInput,
	volume_number: i32,
) -> TachiyomiChapterDto {
	let mut chapter: ChapterDto = map_chapter(template);
	chapter.number = format_r(volume_number as f32 / 10_000.0);
	chapter.files = Vec::<MangaFileDto>::new();
	TachiyomiChapterDto { chapter }
}

/// .NET `float.ToString("R", en-US)`: shortest round-trip representation.
fn format_r(value: f32) -> String {
	let mut rendered = format!("{value}");
	if rendered.ends_with(".0") {
		rendered.truncate(rendered.len() - 2);
	}
	rendered
}

/// `TachiyomiService.GetLatestChapter` for a series made of single-file
/// volumes: the volume with the most recent progress, encoded; `None` when
/// the user has read nothing (Kavita answers `204`).
pub(crate) fn latest_chapter_for(input: &SeriesInput) -> Option<TachiyomiChapterDto> {
	let pages = input.pages();
	let pages_read = input.pages_read();
	let user_has_progress = pages_read != 0 && pages_read <= pages;
	if !user_has_progress {
		return None;
	}
	// The continue point is the next unread volume; the one before it (in
	// volume order) is what the user last finished or is reading.
	let continue_point = continue_point_for(input)?;
	let index = input
		.media
		.iter()
		.position(|media| media.id == continue_point.id)?;
	let target = if continue_point.pages_read() > 0 {
		&input.media[index]
	} else if index > 0 {
		&input.media[index - 1]
	} else {
		input.media.last()?
	};
	Some(encoded_volume_chapter(target, target.number()))
}

async fn latest_chapter(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesQuery>,
) -> APIResult<Response> {
	let user = auth.user();
	let row = find_series_by_kavita_id(
		ctx.as_ref(),
		&user,
		query.series_id.unwrap_or_default(),
	)
	.await?
	.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?;
	let input = load_series_input(ctx.as_ref(), &user, row).await?;
	match latest_chapter_for(&input) {
		Some(chapter) => Ok(Json(chapter).into_response()),
		None => Ok(StatusCode::NO_CONTENT.into_response()),
	}
}

/// `TachiyomiService.MarkChaptersUntilAsRead`: `0` is ignored, `< 1` is an
/// encoded volume number, anything else a chapter number (which single-file
/// volumes never carry, so nothing matches).
async fn mark_chapter_until_as_read(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<MarkUntilQuery>,
) -> APIResult<Json<bool>> {
	let user = auth.user();
	let chapter_number = query.chapter_number.unwrap_or_default();
	let _ = query.generate_reading_sessions;
	let row = find_series_by_kavita_id(
		ctx.as_ref(),
		&user,
		query.series_id.unwrap_or_default(),
	)
	.await?
	.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?;
	let input = load_series_input(ctx.as_ref(), &user, row).await?;
	let targets: Vec<&MediaInput> = if chapter_number == 0.0 {
		Vec::new()
	} else if chapter_number < 1.0 && chapter_number > 0.0 {
		let volume_number = (chapter_number * 10_000.0) as i32;
		input
			.media
			.iter()
			.filter(|media| media.number() > 0 && media.number() <= volume_number)
			.collect()
	} else {
		Vec::new()
	};
	if targets.is_empty() {
		return Ok(Json(true));
	}
	mark_media_read_for(ctx.as_ref(), &user, &targets).await?;
	Ok(Json(true))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn volume_numbers_encode_like_dotnet_round_trip() {
		assert_eq!(format_r(1.0 / 10_000.0), "0.0001");
		assert_eq!(format_r(12.0 / 10_000.0), "0.0012");
		assert_eq!(format_r(100_000.0 / 10_000.0), "10");
		assert_eq!(format_r(3.0 / 10_000.0), "0.0003");
	}
}
