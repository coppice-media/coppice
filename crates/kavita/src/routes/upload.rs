//! Komf's Kavita cover-upload compatibility routes.
//!
//! Komf sends JSON with a base64 `url`, rather than multipart form data.  The
//! server stores the decoded bytes through the backend's native thumbnail path;
//! an empty chapter payload is the lock-reset request used by Komf.

use std::sync::Arc;

use axum::{http::StatusCode, routing::post, Extension, Json, Router};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use stump_auth::AuthContext;

use crate::{
	dto::CoverUploadDto,
	errors::{APIError, APIResult},
};

use super::{
	query::{find_media, find_series_input},
	route_ci,
	series::target_for_input,
	KavitaBackend,
};

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Upload/series", post(upload_series));
	let router = route_ci(router, "/api/Upload/volume", post(upload_volume));
	route_ci(router, "/api/Upload/chapter", post(upload_chapter))
}

fn decode_cover(url: &str) -> APIResult<Vec<u8>> {
	STANDARD
		.decode(url.as_bytes())
		.map_err(|error| APIError::BadRequest(format!("Invalid cover image: {error}")))
}

fn media_id_from_input(
	input: &crate::mapper::SeriesInput,
	index: usize,
) -> APIResult<String> {
	input
		.media
		.get(index)
		.map(|media| media.media.id.clone())
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))
}

async fn upload_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(upload): Json<CoverUploadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let Some(input) = find_series_input(ctx.as_ref(), &user, upload.id).await? else {
		return Err(APIError::NotFound("Series does not exist".to_owned()));
	};
	if upload.url.trim().is_empty() {
		return Err(APIError::BadRequest(
			"Series cover upload requires image bytes".to_owned(),
		));
	}
	let bytes = decode_cover(&upload.url)?;
	ctx.upload_series_cover(&user, target_for_input(&input), bytes, upload.lock_cover)
		.await?;
	Ok(StatusCode::OK)
}

async fn upload_volume(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(upload): Json<CoverUploadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let Some((input, index)) = find_media(ctx.as_ref(), &user, upload.id).await? else {
		return Err(APIError::NotFound("Volume does not exist".to_owned()));
	};
	if upload.url.trim().is_empty() {
		return Err(APIError::BadRequest(
			"Volume cover upload requires image bytes".to_owned(),
		));
	}
	let bytes = decode_cover(&upload.url)?;
	let media_id = media_id_from_input(&input, index)?;
	ctx.upload_media_cover(&user, media_id, bytes, upload.lock_cover)
		.await?;
	Ok(StatusCode::OK)
}

async fn upload_chapter(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(upload): Json<CoverUploadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let Some((input, index)) = find_media(ctx.as_ref(), &user, upload.id).await? else {
		return Err(APIError::NotFound("Chapter does not exist".to_owned()));
	};
	let media_id = media_id_from_input(&input, index)?;
	if upload.url.trim().is_empty() {
		ctx.reset_media_cover_lock(&user, media_id).await?;
	} else {
		let bytes = decode_cover(&upload.url)?;
		ctx.upload_media_cover(&user, media_id, bytes, upload.lock_cover)
			.await?;
	}
	Ok(StatusCode::OK)
}
