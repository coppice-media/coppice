//! Utilities for serving media files, thumbnails, etc.

use axum::body::Bytes;
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
use sea_orm::{prelude::*, DatabaseConnection, FromQueryResult};
use stump_auth::AuthContext;
use stump_media::virtual_media::{self, VirtualArchive};
use tower_http::services::ServeFile;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	routers::api::v2::source_workers,
};

#[derive(FromQueryResult)]
struct MediaDownloadSelect {
	name: String,
	path: String,
}

/// Looks up media in the database, checks download permission, and serves the
/// local file or the archive built for a registered virtual-media path.
///
/// Compatibility backends which retain only a database connection use this
/// entry point; the native API route uses [`serve_media_file_with_ctx`].
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
		.into_model::<MediaDownloadSelect>()
		.one(conn)
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;
	if virtual_media::is_virtual_path(&book.path) {
		return serve_provider_archive(&book.path, &book.name, headers).await;
	}
	validate_range_header(
		&headers,
		tokio::fs::metadata(&book.path).await.ok().map(|m| m.len()),
	)?;
	serve_local_file(&book.path, headers).await
}

/// The reader-facing native API path. It preserves the existing ACL check and
/// ordinary local `ServeFile`, while serving provider archives from the
/// registered virtual-media resolver.
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
		.into_model::<MediaDownloadSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;
	if virtual_media::is_virtual_path(&book.path) {
		return serve_provider_archive(&book.path, &book.name, headers).await;
	}
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
async fn serve_provider_archive(
	path: &str,
	file_stem: &str,
	headers: HeaderMap,
) -> APIResult<Response> {
	let archive = virtual_media::get_archive(path, file_stem)
		.await
		.map_err(APIError::from)?;
	virtual_archive_response(archive, &headers)
}

fn virtual_archive_response(
	archive: VirtualArchive,
	headers: &HeaderMap,
) -> APIResult<Response> {
	let bytes = Bytes::from(archive.bytes);
	let total = u64::try_from(bytes.len())
		.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	let range = parse_range_header(headers, Some(total))?;
	let (body, status, content_range) = match range {
		Some((offset, length)) => {
			let end = offset + length;
			let body = bytes.slice(offset as usize..end as usize);
			(
				body,
				axum::http::StatusCode::PARTIAL_CONTENT,
				Some(format!("bytes {offset}-{}/{total}", end - 1)),
			)
		},
		None => (bytes, axum::http::StatusCode::OK, None),
	};
	let mut response = Response::builder()
		.status(status)
		.header(header::CONTENT_TYPE, archive.content_type.to_string())
		.header(header::CONTENT_LENGTH, body.len())
		.header(
			header::CONTENT_DISPOSITION,
			format!("attachment; filename=\"{}\"", archive.file_name),
		)
		.header(header::ACCEPT_RANGES, "bytes");
	if let Some(content_range) = content_range {
		response = response.header(header::CONTENT_RANGE, content_range);
	}
	response
		.body(Body::from(body))
		.map_err(|error| APIError::InternalServerError(error.to_string()))
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
	#[tokio::test]
	async fn virtual_archive_response_is_downloadable_and_supports_ranges() {
		let archive = VirtualArchive {
			file_name: "Mahou.cbz".to_owned(),
			content_type: stump_media::ContentType::COMIC_ZIP,
			bytes: b"abcdef".to_vec(),
		};
		let response = virtual_archive_response(archive, &HeaderMap::new()).unwrap();
		assert_eq!(response.status(), axum::http::StatusCode::OK);
		assert_eq!(
			response.headers()[header::CONTENT_TYPE],
			stump_media::ContentType::COMIC_ZIP.to_string()
		);
		assert_eq!(
			response.headers()[header::CONTENT_DISPOSITION],
			"attachment; filename=\"Mahou.cbz\""
		);
		assert_eq!(response.headers()[header::CONTENT_LENGTH], "6");
		assert_eq!(
			axum::body::to_bytes(response.into_body(), 16)
				.await
				.unwrap(),
			"abcdef"
		);

		let archive = VirtualArchive {
			file_name: "Mahou.cbz".to_owned(),
			content_type: stump_media::ContentType::COMIC_ZIP,
			bytes: b"abcdef".to_vec(),
		};
		let mut headers = HeaderMap::new();
		headers.insert(header::RANGE, "bytes=1-3".parse().unwrap());
		let response = virtual_archive_response(archive, &headers).unwrap();
		assert_eq!(response.status(), axum::http::StatusCode::PARTIAL_CONTENT);
		assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 1-3/6");
		assert_eq!(response.headers()[header::CONTENT_LENGTH], "3");
		assert_eq!(
			axum::body::to_bytes(response.into_body(), 16)
				.await
				.unwrap(),
			"bcd"
		);
	}
}
