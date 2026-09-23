//! Authenticated delivery for accepted read-aloud EPUB derivatives.
//!
//! These handlers are deliberately cache-only. They inspect the current user's
//! confirmed pair and deterministic cache path; they never enqueue alignment,
//! call Storyteller, or perform a fallback render during a GET.

use std::path::{Path, PathBuf};

use axum::{
	body::Body,
	extract::{Path as AxumPath, State},
	http::{header, HeaderMap, HeaderValue, Response, StatusCode},
	middleware,
	response::IntoResponse,
	routing::get,
	Extension, Json, Router,
};
use models::{entity::media, shared::enums::UserPermission};
use sea_orm::{ColumnTrait, QueryFilter};
use serde::Serialize;
use stump_auth::AuthContext;
use stump_core::config::StumpConfig;
use stump_library::sync_maps;
use stump_worker::AlignGranularity;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::auth::auth_middleware,
};

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	Router::new()
		.route("/media/{id}/read-aloud.epub", get(get_read_aloud))
		.route("/media/{id}/read-aloud", get(get_read_aloud_status))
		.layer(middleware::from_fn_with_state(app_state, auth_middleware))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadAloudStatus {
	status: &'static str,
	reason: Option<&'static str>,
	url: Option<String>,
}

async fn get_read_aloud_status(
	AxumPath(id): AxumPath<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<impl IntoResponse> {
	let user = req
		.user_and_enforce_permissions(&[UserPermission::DownloadFile])
		.map_err(|_| APIError::forbidden_discreet())?;
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(&id))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_owned()))?;
	let Some((_map, path)) =
		accepted_cache(&ctx.config, ctx.conn.as_ref(), &user.id, &book.id).await?
	else {
		return Ok(Json(ReadAloudStatus {
			status: "not_ready",
			reason: Some("accepted_cache_missing"),
			url: None,
		}));
	};
	let _ = path;
	Ok(Json(ReadAloudStatus {
		status: "ready",
		reason: None,
		url: Some(format!("/api/v2/media/{id}/read-aloud.epub")),
	}))
}

async fn get_read_aloud(
	AxumPath(id): AxumPath<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = req
		.user_and_enforce_permissions(&[UserPermission::DownloadFile])
		.map_err(|_| APIError::forbidden_discreet())?;
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(&id))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_owned()))?;
	let Some((map, path)) =
		accepted_cache(&ctx.config, ctx.conn.as_ref(), &user.id, &book.id).await?
	else {
		return Err(APIError::Conflict(
			"read-aloud EPUB is not ready; run explicit alignment first".to_owned(),
		));
	};
	serve_cached_epub(&path, &map, &headers).await
}

async fn accepted_cache(
	config: &StumpConfig,
	conn: &sea_orm::DatabaseConnection,
	user_id: &str,
	media_id: &str,
) -> APIResult<Option<(models::entity::media_sync_map::Model, PathBuf)>> {
	let Some(map) = sync_maps::latest_sync_map_for_media_user(
		conn,
		user_id,
		media_id,
		AlignGranularity::Sentence,
	)
	.await
	.map_err(|error| APIError::InternalServerError(error.to_string()))?
	else {
		return Ok(None);
	};
	if map.ebook_media_id != media_id {
		return Ok(None);
	}
	if !sync_maps::map_matches_current_sources(conn, &map)
		.await
		.map_err(|error| APIError::InternalServerError(error.to_string()))?
	{
		return Ok(None);
	}
	let path = sync_maps::read_aloud_cache_path(config.get_transform_cache_dir(), &map)
		.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	if tokio::fs::metadata(&path).await.is_err() {
		return Ok(None);
	}
	Ok(Some((map, path)))
}

async fn serve_cached_epub(
	path: &Path,
	map: &models::entity::media_sync_map::Model,
	headers: &HeaderMap,
) -> APIResult<Response<Body>> {
	let metadata = tokio::fs::metadata(path).await.map_err(|error| {
		APIError::InternalServerError(format!("read read-aloud cache: {error}"))
	})?;
	let total = metadata.len();
	if total == 0 {
		return Err(APIError::InternalServerError(
			"read-aloud cache is empty".to_owned(),
		));
	}
	let etag = format!("\"{}\"", cache_key_from_path(path, map));
	if headers
		.get(header::IF_NONE_MATCH)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value == etag)
	{
		let mut response = Response::new(Body::empty());
		*response.status_mut() = StatusCode::NOT_MODIFIED;
		response
			.headers_mut()
			.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
		return Ok(response);
	}
	let range = match headers
		.get(header::RANGE)
		.and_then(|value| value.to_str().ok())
	{
		Some(value) => match parse_range(value, total) {
			Some(range) => Some(range),
			None => return Ok(range_response(total)),
		},
		None => None,
	};
	let ranged = range.is_some();
	let (start, end) = range.unwrap_or((0, total - 1));
	let mut file = tokio::fs::File::open(path).await.map_err(|error| {
		APIError::InternalServerError(format!("open read-aloud cache: {error}"))
	})?;
	file.seek(SeekFrom::Start(start)).await.map_err(|error| {
		APIError::InternalServerError(format!("seek read-aloud cache: {error}"))
	})?;
	let stream = ReaderStream::new(file.take(end - start + 1));
	let mut response = Response::new(Body::from_stream(stream));
	if ranged {
		*response.status_mut() = StatusCode::PARTIAL_CONTENT;
	}
	let response_headers = response.headers_mut();
	response_headers.insert(
		header::CONTENT_TYPE,
		HeaderValue::from_static("application/epub+zip"),
	);
	response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
	response_headers.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
	response_headers.insert(
		header::CONTENT_LENGTH,
		HeaderValue::from_str(&(end - start + 1).to_string()).unwrap(),
	);
	response_headers.insert(
		header::CONTENT_DISPOSITION,
		HeaderValue::from_static("attachment; filename=\"read-aloud.epub\""),
	);
	if ranged {
		response_headers.insert(
			header::CONTENT_RANGE,
			HeaderValue::from_str(&format!("bytes {start}-{end}/{total}")).unwrap(),
		);
	}
	Ok(response)
}

fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
	let value = value.strip_prefix("bytes=")?;
	if value.contains(',') {
		return None;
	}
	let (start, end) = value.trim().split_once('-')?;
	if start.is_empty() {
		let suffix = end.parse::<u64>().ok()?;
		if suffix == 0 {
			return None;
		}
		return Some((total.saturating_sub(suffix), total - 1));
	}
	let start = start.parse::<u64>().ok()?;
	if start >= total {
		return None;
	}
	let end = if end.trim().is_empty() {
		total - 1
	} else {
		end.parse::<u64>().ok()?.min(total - 1)
	};
	(start <= end).then_some((start, end))
}

fn range_response(total: u64) -> Response<Body> {
	let mut response = Response::new(Body::empty());
	*response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
	response
		.headers_mut()
		.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
	response.headers_mut().insert(
		header::CONTENT_RANGE,
		HeaderValue::from_str(&format!("bytes */{total}")).unwrap(),
	);
	response
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn invalid_range_response_exposes_resource_length() {
		let response = range_response(123);
		assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
		assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
		assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */123");
	}
}

fn cache_key_from_path(
	path: &Path,
	map: &models::entity::media_sync_map::Model,
) -> String {
	path.file_stem()
		.and_then(|stem| stem.to_str())
		.filter(|stem| !stem.is_empty())
		.unwrap_or(&map.id)
		.to_owned()
}
