//! `ImageController` covers: series, volume and chapter.

use std::sync::Arc;

use axum::{
	extract::Query,
	http::{header, HeaderValue},
	response::{IntoResponse, Response},
	routing::get,
	Extension, Router,
};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	errors::{APIError, APIResult},
	ids::{IdKind, KavitaIds},
	mapper::cover_name,
};

use super::{
	query::{resolve_series_key, SeriesKey},
	route_ci, KavitaBackend, KavitaImage,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesCoverQuery {
	#[serde(default)]
	series_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VolumeCoverQuery {
	#[serde(default)]
	volume_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterCoverQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Image/series-cover", get(series_cover));
	let router = route_ci(router, "/api/Image/volume-cover", get(volume_cover));
	route_ci(router, "/api/Image/chapter-cover", get(chapter_cover))
}

/// Kavita serves covers as attachments named after the cover file
/// (`Content-Disposition: attachment; filename=v1_c1.png`) with
/// `Accept-Ranges: bytes`; the bytes are what clients consume.
pub(crate) fn image_response(image: KavitaImage, file_name: &str) -> Response {
	let mut response = image.data.into_response();
	let headers = response.headers_mut();
	if let Ok(value) = HeaderValue::from_str(&image.content_type) {
		headers.insert(header::CONTENT_TYPE, value);
	}
	if let Ok(value) = HeaderValue::from_str(&format!(
		"attachment; filename={file_name}; filename*=UTF-8''{file_name}"
	)) {
		headers.insert(header::CONTENT_DISPOSITION, value);
	}
	headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
	headers.insert(
		header::CACHE_CONTROL,
		HeaderValue::from_static("private, max-age=0, stale-while-revalidate=0"),
	);
	response
}

/// A grouped series is covered by Stump's series thumbnail; a book by its
/// file's, like every other Kavita series whose cover is its first chapter.
async fn series_cover(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesCoverQuery>,
) -> APIResult<Response> {
	let series_id = query.series_id.unwrap_or_default();
	let image = match resolve_series_key(ctx.as_ref(), series_id).await? {
		Some(SeriesKey::Series(stump_id)) => {
			ctx.series_thumbnail(&auth.user(), &stump_id).await?
		},
		Some(SeriesKey::Book(media_id)) => {
			ctx.media_thumbnail(&auth.user(), &media_id).await?
		},
		None => return Err(APIError::NotFound("Series does not exist".to_owned())),
	};
	Ok(image_response(image, &cover_name(series_id, series_id)))
}

async fn media_cover(
	ctx: &dyn KavitaBackend,
	auth: &AuthContext,
	id: i32,
) -> APIResult<Response> {
	let stump_id = KavitaIds::lookup(ctx.conn(), IdKind::Media, id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let image = ctx.media_thumbnail(&auth.user(), &stump_id).await?;
	Ok(image_response(image, &cover_name(id, id)))
}

async fn volume_cover(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<VolumeCoverQuery>,
) -> APIResult<Response> {
	media_cover(ctx.as_ref(), &auth, query.volume_id.unwrap_or_default()).await
}

async fn chapter_cover(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterCoverQuery>,
) -> APIResult<Response> {
	media_cover(ctx.as_ref(), &auth, query.chapter_id.unwrap_or_default()).await
}
