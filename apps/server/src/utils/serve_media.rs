//! Utilities for serving media files, thumbnails, etc.

use axum::{
	body::Body,
	extract::Request,
	http::{header, HeaderMap},
	response::{IntoResponse, Response},
};
use models::{
	entity::media::{self},
	shared::enums::UserPermission,
};
use stump_auth::AuthContext;
use tower_http::services::ServeFile;

use sea_orm::{prelude::*, DatabaseConnection};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	routers::api::v2::source_workers,
};

/// Looks up a piece of media in the database, checks that the user has permission to
/// download it, and serves the local media file to the client.
///
/// Compatibility backends which only retain a database connection continue to
/// use this local-only entry point.  The native API route uses
/// [`serve_media_file_with_ctx`] so remote locations can be proxied without
/// changing those provider trait signatures.
pub async fn serve_media_file(
	req: AuthContext,
	headers: HeaderMap,
	conn: &DatabaseConnection,
	media_id: String,
) -> APIResult<impl IntoResponse> {
	let user = req
		.user_and_enforce_permissions(&[UserPermission::DownloadFile])
		.map_err(|_| {
			tracing::error!("User does not have permission to download file");
			APIError::forbidden_discreet()
		})?;
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(media_id))
		.into_model::<media::MediaIdentSelect>()
		.one(conn)
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;
	validate_range_header(
		&headers,
		tokio::fs::metadata(&book.path).await.ok().map(|m| m.len()),
	)?;
	serve_local_file(&book.path, headers).await
}

/// The reader-facing native API path.  It preserves the existing ACL check and
/// ordinary local `ServeFile`, falling back to a verified healthy remote
/// location only when local bytes are unavailable or the media path is the
/// explicit `worker://` form.
pub async fn serve_media_file_with_ctx(
	req: AuthContext,
	headers: HeaderMap,
	ctx: &AppState,
	media_id: String,
) -> APIResult<impl IntoResponse> {
	let user = req
		.user_and_enforce_permissions(&[UserPermission::DownloadFile])
		.map_err(|_| {
			tracing::error!("User does not have permission to download file");
			APIError::forbidden_discreet()
		})?;
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(media_id.clone()))
		.into_model::<media::MediaIdentSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	let local_size = tokio::fs::metadata(&book.path).await.ok().map(|m| m.len());
	let remote_size = source_workers::remote_media_size(ctx, &media_id).await?;
	let local_unavailable = book.path.starts_with("worker://") || local_size.is_none();
	let range_size = if local_unavailable {
		remote_size
	} else {
		local_size
	};
	let requested = parse_range_header(&headers, range_size)?;

	if local_unavailable && (book.path.starts_with("worker://") || remote_size.is_some())
	{
		let Some(total_size) = remote_size else {
			return Err(APIError::ServiceUnavailable(
				"remote media bytes are temporarily unavailable".to_string(),
			));
		};
		let (offset, length) = requested.unwrap_or((0, total_size));
		let Some((body, item)) =
			source_workers::open_media_transfer(ctx, &media_id, offset, length).await?
		else {
			return Err(APIError::ServiceUnavailable(
				"remote source is offline".to_string(),
			));
		};
		let filename = item
			.relative_path
			.as_deref()
			.and_then(|path| path.rsplit('/').next())
			.or_else(|| {
				std::path::Path::new(&book.path)
					.file_name()
					.and_then(|s| s.to_str())
			})
			.unwrap_or(&media_id);
		return Ok(source_workers::remote_response(
			body,
			&item,
			offset,
			length,
			total_size,
			filename,
			item.media_type.as_deref(),
		));
	}

	if local_unavailable {
		// No remote location exists: retain the ordinary local ServeFile error
		// semantics for a broken local path.
		return serve_local_file(&book.path, headers).await;
	}

	validate_range_header(&headers, local_size)?;
	serve_local_file(&book.path, headers).await
}

async fn serve_local_file(path: &str, headers: HeaderMap) -> APIResult<Response> {
	// Note: reusing the original headers is required for range requests.
	let mut serve_req = Request::new(Body::empty());
	*serve_req.headers_mut() = headers;
	match ServeFile::new(path).try_call(serve_req).await {
		Ok(response) => {
			let mut response = response.map(Body::new);
			if let Some(filename) = std::path::Path::new(path)
				.file_name()
				.and_then(|os_str| os_str.to_str())
			{
				response.headers_mut().insert(
					header::CONTENT_DISPOSITION,
					format!("attachment; filename=\"{}\"", filename)
						.parse()
						.unwrap_or_else(|_| "attachment".parse().unwrap()),
				);
			}
			Ok(response)
		},
		Err(e) => {
			tracing::error!(error = ?e, path = %path, "Error serving media file");
			Err(APIError::InternalServerError(format!(
				"Failed to serve file: {}",
				e
			)))
		},
	}
}

/// Parse exactly one byte range.  Multiple ranges are intentionally rejected:
/// the source protocol grants one contiguous bounded stream, and returning a
/// multipart response would make direct and tunnel transports observably
/// different.
fn parse_range_header(
	headers: &HeaderMap,
	size: Option<u64>,
) -> APIResult<Option<(u64, u64)>> {
	let Some(value) = headers.get(header::RANGE) else {
		return Ok(None);
	};
	let text = value
		.to_str()
		.map_err(|_| APIError::RangeNotSatisfiable("invalid range header".to_string()))?;
	let Some(spec) = text.strip_prefix("bytes=") else {
		return Err(APIError::RangeNotSatisfiable(
			"only byte ranges are supported".to_string(),
		));
	};
	if spec.contains(',') {
		return Err(APIError::RangeNotSatisfiable(
			"multiple ranges are not supported".to_string(),
		));
	}
	let Some((start, end)) = spec.split_once('-') else {
		return Err(APIError::RangeNotSatisfiable(
			"invalid byte range".to_string(),
		));
	};
	let Some(size) = size else {
		return Err(APIError::RangeNotSatisfiable(
			"range size is unknown".to_string(),
		));
	};
	if size == 0 {
		return Err(APIError::RangeNotSatisfiable(
			"range is not satisfiable".to_string(),
		));
	}
	let (offset, length) = if start.is_empty() {
		let suffix = end.parse::<u64>().map_err(|_| {
			APIError::RangeNotSatisfiable("invalid suffix range".to_string())
		})?;
		if suffix == 0 {
			return Err(APIError::RangeNotSatisfiable(
				"range is not satisfiable".to_string(),
			));
		}
		let length = suffix.min(size);
		(size - length, length)
	} else {
		let offset = start.parse::<u64>().map_err(|_| {
			APIError::RangeNotSatisfiable("invalid range start".to_string())
		})?;
		if offset >= size {
			return Err(APIError::RangeNotSatisfiable(
				"range is not satisfiable".to_string(),
			));
		}
		let end = if end.is_empty() {
			size - 1
		} else {
			end.parse::<u64>()
				.map_err(|_| {
					APIError::RangeNotSatisfiable("invalid range end".to_string())
				})?
				.min(size - 1)
		};
		if end < offset {
			return Err(APIError::RangeNotSatisfiable(
				"range is not satisfiable".to_string(),
			));
		}
		(offset, end - offset + 1)
	};
	Ok(Some((offset, length)))
}

fn validate_range_header(headers: &HeaderMap, size: Option<u64>) -> APIResult<()> {
	let _ = parse_range_header(headers, size)?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rejects_multiple_ranges() {
		let mut headers = HeaderMap::new();
		headers.insert(header::RANGE, "bytes=0-1,2-3".parse().unwrap());
		assert_eq!(
			parse_range_header(&headers, Some(8))
				.unwrap_err()
				.status_code(),
			axum::http::StatusCode::RANGE_NOT_SATISFIABLE
		);
	}

	#[test]
	fn parses_suffix_and_open_ranges() {
		let mut headers = HeaderMap::new();
		headers.insert(header::RANGE, "bytes=-3".parse().unwrap());
		assert_eq!(
			parse_range_header(&headers, Some(10)).unwrap(),
			Some((7, 3))
		);
		headers.insert(header::RANGE, "bytes=4-".parse().unwrap());
		assert_eq!(
			parse_range_header(&headers, Some(10)).unwrap(),
			Some((4, 6))
		);
	}
}
