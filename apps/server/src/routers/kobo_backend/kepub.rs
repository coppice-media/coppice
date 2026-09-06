use std::{path::Path, time::UNIX_EPOCH};

use axum::{
	body::Body,
	extract::{Extension, Path as AxumPath, Request, State},
	http::{header, HeaderMap, HeaderValue},
	response::{IntoResponse, Response},
};
use models::{entity::media, shared::enums::UserPermission};
use sea_orm::{ColumnTrait, QueryFilter};
use stump_auth::AuthContext;
use stump_media::transform::is_comic_source;
use tower_http::services::ServeFile;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

/// Serve a Kobo book: comics as device-transformed fixed-layout KEPUBs when
/// `transform_enabled`, EPUBs as KEPUBs when `kobo_kepub_conversion` is on,
/// everything else as the original file.
///
/// The route and backend seam intentionally remain named `file/epub`: Kobo
/// discovers that URL during sync, while the response's bytes and filename
/// identify the optional KEPUB representation. Conversion is lazy and cached
/// under `<config>/cache/kepub/<media-id>-<mtime>-<options>-d<deflate level>.kepub.epub`
/// (EPUBs) and `<config>/cache/transform/<media-id>-<mtime>-<profile digest>.kepub.epub`
/// (comics).
#[tracing::instrument(skip_all, fields(book_id = %book_id), err)]
pub(crate) async fn book_file(
	ctx: AppState,
	auth: AuthContext,
	book_id: String,
	headers: HeaderMap,
) -> APIResult<Response> {
	let transform_comics = ctx.config.transform.transform_enabled;
	let convert_epubs = ctx.config.protocols.kobo_kepub_conversion;
	if !transform_comics && !convert_epubs {
		return serve_original(ctx, auth, book_id, headers).await;
	}

	let user = auth
		.user_and_enforce_permissions(&[UserPermission::DownloadFile])
		.map_err(|_| {
			tracing::error!("User does not have permission to download file");
			APIError::forbidden_discreet()
		})?;
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(book_id.clone()))
		.into_model::<media::MediaIdentSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_string()))?;

	if transform_comics && is_comic_source(&book.path) {
		return super::comic_transform::serve_comic(ctx, auth, book, headers).await;
	}

	let is_epub = Path::new(&book.path)
		.extension()
		.and_then(|extension| extension.to_str())
		.is_some_and(|extension| extension.eq_ignore_ascii_case("epub"));
	if !convert_epubs || !is_epub {
		return serve_original(ctx, auth, book_id, headers).await;
	}

	let cache_path = ensure_cached(&ctx, &book).await?;
	kepub_response(&book.path, &cache_path, headers).await
}

/// Return the stable cache location for a source EPUB.
pub(crate) async fn cache_path_for(
	ctx: &AppState,
	book: &media::MediaIdentSelect,
) -> APIResult<std::path::PathBuf> {
	let source_metadata = tokio::fs::metadata(&book.path).await.map_err(|error| {
		tracing::error!(error = ?error, path = %book.path, "Failed to inspect EPUB file");
		APIError::InternalServerError("Failed to inspect EPUB file".to_string())
	})?;
	let file_mtime = source_metadata
		.modified()
		.ok()
		.and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
		.map_or(0, |duration| duration.as_nanos());
	let write_options = stump_kepub::WriteOptions {
		deflate_level: ctx.config.protocols.kobo_kepub_deflate_level.clamp(1, 12),
	};
	let cache_dir = ctx.config.get_cache_dir().join("kepub");
	let base_name = stump_kepub::cache_file_name(
		&book.id,
		file_mtime,
		&stump_kepub::TransformOptions::default(),
	);
	let stem = base_name
		.strip_suffix(".kepub.epub")
		.unwrap_or(base_name.as_str());
	Ok(cache_dir.join(format!(
		"{stem}-d{}.kepub.epub",
		write_options.deflate_level
	)))
}

/// Ensure an EPUB has a complete atomically-published KEPUB cache entry.
///
/// Existing entries are treated as hits and have their mtime refreshed for
/// age-based eviction.
pub(crate) async fn ensure_cached(
	ctx: &AppState,
	book: &media::MediaIdentSelect,
) -> APIResult<std::path::PathBuf> {
	let cache_path = cache_path_for(ctx, book).await?;
	if tokio::fs::metadata(&cache_path).await.is_ok() {
		touch_cache_file(&cache_path).await;
		return Ok(cache_path);
	}

	let cache_dir = cache_path
		.parent()
		.expect("KEPUB cache path always has a parent directory");
	tokio::fs::create_dir_all(cache_dir).await.map_err(|error| {
		tracing::error!(error = ?error, path = ?cache_dir, "Failed to create KEPUB cache directory");
		APIError::InternalServerError("Failed to create KEPUB cache directory".to_string())
	})?;
	let temporary_path = cache_dir.join(format!(
		"{}.{}.tmp",
		cache_path
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or("kepub"),
		uuid::Uuid::new_v4()
	));
	if let Err(error) = convert_epub_to_file(
		&book.path,
		&temporary_path,
		ctx.config.protocols.kobo_kepub_deflate_level.clamp(1, 12),
	)
	.await
	{
		let _ = tokio::fs::remove_file(&temporary_path).await;
		return Err(error);
	}
	tokio::fs::rename(&temporary_path, &cache_path)
		.await
		.map_err(|error| {
			tracing::error!(error = ?error, path = ?cache_path, "Failed to publish KEPUB cache");
			APIError::InternalServerError("Failed to publish KEPUB cache".to_string())
		})?;
	Ok(cache_path)
}

async fn touch_cache_file(path: &Path) {
	let path = path.to_path_buf();
	let log_path = path.clone();
	match tokio::task::spawn_blocking(move || {
		filetime::set_file_mtime(&path, filetime::FileTime::now())
	})
	.await
	{
		Ok(Ok(())) => {},
		Ok(Err(error)) => {
			tracing::debug!(?error, path = ?log_path, "KEPUB cache mtime refresh failed");
		},
		Err(error) => {
			tracing::debug!(?error, path = ?log_path, "KEPUB cache mtime refresh task failed");
		},
	}
}

pub(crate) async fn serve_original(
	ctx: AppState,
	auth: AuthContext,
	book_id: String,
	headers: HeaderMap,
) -> APIResult<Response> {
	super::router::book_download(
		State(ctx),
		Extension(auth),
		AxumPath(super::router::KoboAPIKeyAndBookId {
			api_key: String::new(),
			book_id,
		}),
		headers,
	)
	.await
	.map(IntoResponse::into_response)
}

/// Stream the cached KEPUB with `ServeFile` (range requests included, which
/// Kobo devices use to resume large downloads), relabelled as a KEPUB.
async fn kepub_response(
	source_path: &str,
	cache_path: &Path,
	headers: HeaderMap,
) -> APIResult<Response> {
	let mut serve_req = Request::new(Body::empty());
	*serve_req.headers_mut() = headers;
	let mut response = ServeFile::new(cache_path)
		.try_call(serve_req)
		.await
		.map_err(|error| {
			tracing::error!(error = ?error, path = ?cache_path, "Failed to serve KEPUB cache");
			APIError::InternalServerError("Failed to serve KEPUB".to_string())
		})?
		.into_response();
	response.headers_mut().insert(
		header::CONTENT_TYPE,
		HeaderValue::from_static("application/epub+zip"),
	);
	if let Ok(value) = HeaderValue::from_str(&format!(
		"attachment; filename=\"{}\"",
		kepub_filename(source_path)
	)) {
		response
			.headers_mut()
			.insert(header::CONTENT_DISPOSITION, value);
	}
	Ok(response)
}

fn kepub_filename(source_path: &str) -> String {
	let source_name = Path::new(source_path)
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or("book.epub")
		.replace('"', "_");
	if let Some(stem) = source_name.strip_suffix(".epub") {
		format!("{stem}.kepub.epub")
	} else if let Some(stem) = source_name.strip_suffix(".EPUB") {
		format!("{stem}.kepub.epub")
	} else {
		format!("{source_name}.kepub.epub")
	}
}

/// Convert an EPUB on disk with the built-in `stump_kepub` converter.
///
/// `stump_kepub` is a verified byte-for-byte port of kepubify's transform, so
/// there is no external binary to prefer; conversion runs on the blocking pool
/// and parallelises across content documents internally.
async fn convert_epub_to_file(
	source_path: &str,
	output_path: &Path,
	deflate_level: u32,
) -> APIResult<()> {
	let source = tokio::fs::read(source_path).await.map_err(|error| {
		tracing::error!(error = ?error, path = %source_path, "Failed to read EPUB file");
		APIError::InternalServerError("Failed to read EPUB file".to_string())
	})?;
	let output_path = output_path.to_path_buf();
	tokio::task::spawn_blocking(move || {
		let file = std::fs::File::create(&output_path)?;
		let sink = std::io::BufWriter::with_capacity(1 << 16, file);
		stump_kepub::transform_epub_to(
			&source,
			&stump_kepub::TransformOptions::default(),
			&stump_kepub::WriteOptions { deflate_level },
			sink,
		)?
		.into_inner()
		.map_err(|error| error.into_error())?
		.sync_all()?;
		Ok::<(), stump_kepub::KepubError>(())
	})
	.await
	.map_err(|error| {
		tracing::error!(error = ?error, "KEPUB conversion task failed");
		APIError::InternalServerError("KEPUB conversion task failed".to_string())
	})?
	.map_err(|error| {
		tracing::error!(error = ?error, "Failed to convert EPUB to KEPUB");
		APIError::InternalServerError("Failed to convert EPUB to KEPUB".to_string())
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn output_filename_uses_kepub_suffix() {
		assert_eq!(kepub_filename("/books/alice.epub"), "alice.kepub.epub");
		assert_eq!(kepub_filename("/books/alice.EPUB"), "alice.kepub.epub");
	}

	#[test]
	fn output_filename_does_not_allow_quote_injection() {
		assert_eq!(kepub_filename("book\".epub"), "book_.kepub.epub");
	}
}
