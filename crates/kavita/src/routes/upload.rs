//! Komf's Kavita cover-upload compatibility routes.
//!
//! Komf sends JSON with a base64 `url`, rather than multipart form data.  The
//! server stores the decoded bytes through the backend's native thumbnail path;
//! an empty chapter payload is the lock-reset request used by Komf.
//!
//! Kavita's `UploadController` is `RequireAdminRole`; here every upload
//! requires `EditMetadata` ([`enforce_edit_metadata`]). The request body is
//! budgeted from the backend's cover limit rather than Axum's 2 MiB default,
//! which would have refused any cover over ~1.5 MiB once base64-encoded.

use std::sync::Arc;

use axum::{
	extract::DefaultBodyLimit, http::StatusCode, routing::post, Extension, Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use stump_auth::AuthContext;

use crate::{
	dto::CoverUploadDto,
	errors::{APIError, APIResult},
};

use super::{
	mutations::enforce_edit_metadata,
	query::{find_media, find_series_input},
	route_ci,
	series::target_for_input,
	KavitaBackend,
};

/// Room for the JSON envelope around the base64 cover (`id`, `lockCover`,
/// quoting, whitespace); Komf's payload needs a few dozen bytes of it.
const JSON_ENVELOPE_BYTES: usize = 1024;

/// The upload routes, with a request-body budget sized for a base64 cover of
/// `max_cover_bytes` (the backend's [`KavitaBackend::max_cover_upload_bytes`]).
pub(crate) fn routes<S>(max_cover_bytes: usize) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Upload/series", post(upload_series));
	let router = route_ci(router, "/api/Upload/volume", post(upload_volume));
	route_ci(router, "/api/Upload/chapter", post(upload_chapter))
		.layer(DefaultBodyLimit::max(encoded_body_limit(max_cover_bytes)))
}

/// The request-body budget for a decoded cover of at most `max_cover_bytes`:
/// base64 encodes every 3 bytes as 4 (padding included), plus the envelope.
fn encoded_body_limit(max_cover_bytes: usize) -> usize {
	max_cover_bytes
		.div_ceil(3)
		.saturating_mul(4)
		.saturating_add(JSON_ENVELOPE_BYTES)
}

/// Decode the base64 cover and refuse one over the backend's limit: the
/// body budget above only bounds the encoded request, so a cover just over
/// the limit still fits its envelope slack and is caught here.
fn decode_cover(ctx: &dyn KavitaBackend, url: &str) -> APIResult<Vec<u8>> {
	let bytes = STANDARD
		.decode(url.as_bytes())
		.map_err(|error| APIError::BadRequest(format!("Invalid cover image: {error}")))?;
	let limit = ctx.max_cover_upload_bytes();
	if bytes.len() > limit {
		return Err(APIError::PayloadTooLarge(format!(
			"Cover image exceeds the upload limit of {limit} bytes"
		)));
	}
	Ok(bytes)
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
	let user = enforce_edit_metadata(&auth)?;
	let Some(input) = find_series_input(ctx.as_ref(), &user, upload.id).await? else {
		return Err(APIError::NotFound("Series does not exist".to_owned()));
	};
	if upload.url.trim().is_empty() {
		return Err(APIError::BadRequest(
			"Series cover upload requires image bytes".to_owned(),
		));
	}
	let bytes = decode_cover(ctx.as_ref(), &upload.url)?;
	ctx.upload_series_cover(&user, target_for_input(&input), bytes, upload.lock_cover)
		.await?;
	Ok(StatusCode::OK)
}

async fn upload_volume(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(upload): Json<CoverUploadDto>,
) -> APIResult<StatusCode> {
	let user = enforce_edit_metadata(&auth)?;
	let Some((input, index)) = find_media(ctx.as_ref(), &user, upload.id).await? else {
		return Err(APIError::NotFound("Volume does not exist".to_owned()));
	};
	if upload.url.trim().is_empty() {
		return Err(APIError::BadRequest(
			"Volume cover upload requires image bytes".to_owned(),
		));
	}
	let bytes = decode_cover(ctx.as_ref(), &upload.url)?;
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
	let user = enforce_edit_metadata(&auth)?;
	let Some((input, index)) = find_media(ctx.as_ref(), &user, upload.id).await? else {
		return Err(APIError::NotFound("Chapter does not exist".to_owned()));
	};
	let media_id = media_id_from_input(&input, index)?;
	if upload.url.trim().is_empty() {
		ctx.reset_media_cover_lock(&user, media_id).await?;
	} else {
		let bytes = decode_cover(ctx.as_ref(), &upload.url)?;
		ctx.upload_media_cover(&user, media_id, bytes, upload.lock_cover)
			.await?;
	}
	Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ids::{IdKind, KavitaIds};
	use crate::test_support::{
		auth_user, db, library_of_type, request, request_raw, series_with_files,
		TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	const MIB: usize = 1024 * 1024;

	/// A cover exactly at the limit (a size that is not a multiple of three,
	/// so base64 pads) fits its JSON envelope inside the budget, and an
	/// unbounded limit saturates rather than overflows.
	#[test]
	fn body_budget_covers_a_base64_cover_at_the_limit() {
		let limit = 3 * MIB + 1;
		let encoded = STANDARD.encode(vec![0xAB; limit]);
		let envelope = serde_json::json!({"id": 1, "url": encoded, "lockCover": true});
		let body = serde_json::to_vec(&envelope).unwrap();
		assert!(body.len() <= encoded_body_limit(limit));
		assert_eq!(encoded_body_limit(usize::MAX), usize::MAX);
	}

	/// The default Axum budget (2 MiB) refused any cover over ~1.5 MiB; the
	/// route must accept a cover up to the backend's limit and answer `413`
	/// — never `400` — for one past it, whether the encoded body or only the
	/// decoded image crosses the line.
	#[tokio::test]
	async fn cover_uploads_are_bounded_by_the_backend_limit_not_axums_default() {
		let conn = db().await;
		let user_row = fake_data::User::new("cover-limits").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (series_row, files) =
			series_with_files(&conn, &library.id, "Big covers", &[("v01", "cbz", 4)])
				.await;
		let mut backend = TestBackend::new(conn);
		backend.max_cover_bytes = 3 * MIB;
		let backend = std::sync::Arc::new(backend);
		let series_id =
			KavitaIds::resolve(backend.conn(), IdKind::Series, &series_row.id)
				.await
				.unwrap();
		let chapter_id = KavitaIds::resolve(backend.conn(), IdKind::Media, &files[0].id)
			.await
			.unwrap();

		let cover = vec![0x5A; 3 * MIB];
		let (status, body) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Upload/series",
			Some(serde_json::json!({
				"id": series_id,
				"url": STANDARD.encode(&cover),
				"lockCover": true,
			})),
		)
		.await;
		assert_eq!(status, StatusCode::OK, "{body}");
		let (status, bytes) = request_raw(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Image/series-cover?seriesId={series_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(bytes, cover);

		// Just over the limit: within the body budget, caught after decoding.
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Upload/chapter",
			Some(serde_json::json!({
				"id": chapter_id,
				"url": STANDARD.encode(vec![0x5A; 3 * MIB + 1]),
				"lockCover": true,
			})),
		)
		.await;
		assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

		// Far over the limit: the request body itself is refused.
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Upload/volume",
			Some(serde_json::json!({
				"id": chapter_id,
				"url": STANDARD.encode(vec![0x5A; 5 * MIB]),
				"lockCover": true,
			})),
		)
		.await;
		assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

		// Neither oversized upload touched the chapter.
		let (status, chapter) = request(
			backend,
			&user,
			"GET",
			&format!("/api/Series/chapter?chapterId={chapter_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(chapter["coverImageLocked"], false);
	}
}
