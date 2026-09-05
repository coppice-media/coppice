//! Comic (CBZ/ZIP/CBR/RAR/PDF) delivery for Kobo devices: the book file route
//! serves a fixed-layout KEPUB whose pages were transformed for the requesting
//! device instead of the original container.
//!
//! Profile resolution: the device's stored `transform_profile` (resolved via
//! `DeviceService::device_for_credential`) wins; otherwise the configured
//! `transform.transform_kobo_profile` preset applies. Every failure falls back
//! to the original file — transform delivery is best-effort and must never
//! break a download.

use std::path::Path;

use axum::{
	body::Body,
	extract::Request,
	http::{header, HeaderMap, HeaderValue},
	response::{IntoResponse, Response},
};
use models::entity::{device, media};
use stump_auth::AuthContext;
use stump_devices::{CredentialRef, DeviceResult};
use stump_media::transform::container::{build_cbz, build_kepub};
use stump_media::transform::{is_comic_source, TransformCache, TransformProfile};
use tower_http::services::ServeFile;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

/// Resolve the effective transform profile for this request, or `None` when
/// no profile applies (unknown config preset, broken device profile, …).
///
/// The Kobo route always forces the fixed-layout KEPUB container: Kobo
/// firmware can only open EPUB-family files at this route.
async fn resolve_profile(
	ctx: &AppState,
	auth: &AuthContext,
) -> Option<TransformProfile> {
	let mut requested = resolve_device_profile(ctx, auth).await;

	if requested.is_none() {
		let configured = &ctx.config.transform.transform_kobo_profile;
		match TransformProfile::preset(configured) {
			Some(profile) => requested = Some(profile),
			None => {
				tracing::warn!(
					profile = configured,
					"Configured transform_kobo_profile is not a known preset; \
					 comic transform delivery is disabled"
				);
			},
		}
	}

	requested.map(|profile| TransformProfile {
		container: stump_media::transform::ComicContainer::KepubFixedLayout,
		..profile
	})
}

/// Look up the requesting device and interpret its stored profile JSON.
async fn resolve_device_profile(
	ctx: &AppState,
	auth: &AuthContext,
) -> Option<TransformProfile> {
	let api_key = auth.api_key.as_deref()?;

	let lookup: DeviceResult<Option<device::Model>> = ctx
		.devices()
		.device_for_credential(CredentialRef::ApiKey(api_key))
		.await;
	let device = match lookup {
		Ok(Some(device)) => device,
		Ok(None) => return None,
		Err(error) => {
			tracing::warn!(?error, "Failed to resolve device for comic transform");
			return None;
		},
	};

	let profile_json = device.transform_profile.as_ref()?;
	match TransformProfile::from_device_profile(profile_json) {
		Some(Ok(profile)) => Some(profile),
		Some(Err(error)) => {
			tracing::warn!(
				?error,
				device_id = %device.id,
				"Invalid device transform profile; falling back to the configured preset"
			);
			None
		},
		None => None,
	}
}

/// Serve the comic `book` as a transformed KEPUB, falling back to the original
/// file when the feature is off, no profile resolves, or the transform fails.
pub(crate) async fn serve_comic(
	ctx: AppState,
	auth: AuthContext,
	book: media::MediaIdentSelect,
	headers: HeaderMap,
) -> APIResult<Response> {
	if !ctx.config.transform.transform_enabled {
		return super::kepub::serve_original(ctx, auth, book.id, headers).await;
	}

	if auth
		.user_and_enforce_permissions(&[models::shared::enums::UserPermission::DownloadFile])
		.is_err()
	{
		tracing::error!("User does not have permission to download file");
		return Err(APIError::forbidden_discreet());
	}

	let Some(profile) = resolve_profile(&ctx, &auth).await else {
		return super::kepub::serve_original(ctx, auth, book.id, headers).await;
	};

	let source_mtime_ns = match source_mtime_nanos(&book.path).await {
		Ok(mtime) => mtime,
		Err(error) => {
			tracing::error!(?error, path = %book.path, "Failed to inspect comic file");
			return Err(APIError::InternalServerError(
				"Failed to inspect comic file".to_string(),
			));
		},
	};

	let cache = TransformCache::new(
		ctx.config.get_transform_cache_dir(),
		ctx.config.transform.transform_cache_max_bytes,
	);
	let cache_path = cache.path_for(&book.id, source_mtime_ns, &profile, "kepub.epub");

	if cache.hit(&cache_path) {
		return comic_kepub_response(&book.path, &cache_path, headers).await;
	}

	let build = tokio::task::spawn_blocking({
		let source_path = book.path.clone();
		let cache_dir = cache.dir().to_path_buf();
		let cache_path = cache_path.clone();
		let media_config = ctx.config.media.clone();
		move || {
			build_transformed(
				Path::new(&source_path),
				&cache_dir,
				&cache_path,
				&profile,
				&media_config,
			)
		}
	})
	.await;

	match build {
		Ok(Ok(())) => {},
		Ok(Err(error)) => {
			tracing::warn!(
				?error,
				path = %book.path,
				"Comic transform failed; serving original file"
			);
			let _ = tokio::fs::remove_file(&cache_path).await;
			return super::kepub::serve_original(ctx, auth, book.id, headers).await;
		},
		Err(error) => {
			tracing::error!(?error, "Comic transform task panicked");
			let _ = tokio::fs::remove_file(&cache_path).await;
			return super::kepub::serve_original(ctx, auth, book.id, headers).await;
		},
	}

	// Best-effort LRU sweep after publishing a new entry.
	let _ = tokio::task::spawn_blocking({
		let cache = cache.clone();
		move || {
			if let Ok(sweep) = cache.sweep() {
				tracing::trace!(?sweep, "Transform cache sweep complete");
			}
		}
	})
	.await;

	comic_kepub_response(&book.path, &cache_path, headers).await
}

/// Build one transformed comic into the cache: temp file → publish.
fn build_transformed(
	source_path: &Path,
	cache_dir: &Path,
	cache_path: &Path,
	profile: &TransformProfile,
	media_config: &stump_media::MediaConfig,
) -> Result<(), stump_media::transform::TransformError> {
	std::fs::create_dir_all(cache_dir)?;

	let temp_path = cache_dir.join(format!(
		"{}.{}.tmp",
		cache_path
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or("transform"),
		uuid::Uuid::new_v4()
	));

	let result = build_to_temp(source_path, &temp_path, profile, media_config)
		.and_then(|()| TransformCache::publish(&temp_path, cache_path));

	if result.is_err() {
		let _ = std::fs::remove_file(&temp_path);
	}
	result
}

fn build_to_temp(
	source_path: &Path,
	temp_path: &Path,
	profile: &TransformProfile,
	media_config: &stump_media::MediaConfig,
) -> Result<(), stump_media::transform::TransformError> {
	let source = stump_media::transform::ComicPages::open(source_path, media_config)?;
	let title = source_path
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or("comic")
		.to_string();

	let file = std::fs::File::create(temp_path)?;
	let sink = std::io::BufWriter::with_capacity(1 << 16, file);
	let pages = source.into_iter(media_config);
	let concurrency = media_config.cpu_concurrency_limit().max(1);

	match profile.container {
		stump_media::transform::ComicContainer::KepubFixedLayout => {
			build_kepub(
				stump_media::transform::transform_pages_blocking(
					pages,
					profile.clone(),
					concurrency,
				),
				&title,
				sink,
			)?
			.into_inner()
			.map_err(|error| error.into_error())?
			.sync_all()?;
			Ok(())
		},
		stump_media::transform::ComicContainer::Cbz => {
			let comic_info = stump_media::transform::comic_info_xml(source_path);
			build_cbz(
				stump_media::transform::transform_pages_blocking(
					pages,
					profile.clone(),
					concurrency,
				),
				comic_info.as_deref(),
				sink,
			)?
			.into_inner()
			.map_err(|error| error.into_error())?
			.sync_all()?;
			Ok(())
		},
	}
}

async fn source_mtime_nanos(path: &str) -> std::io::Result<u128> {
	let metadata = tokio::fs::metadata(path).await?;
	Ok(metadata
		.modified()?
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |duration| duration.as_nanos()))
}

/// Stream the transformed KEPUB with `ServeFile` (range requests included,
/// which Kobo devices use to resume downloads).
async fn comic_kepub_response(
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
			tracing::error!(
				?error,
				path = ?cache_path,
				"Failed to serve transformed comic"
			);
			APIError::InternalServerError(
				"Failed to serve transformed comic".to_string(),
			)
		})?
		.into_response();

	response.headers_mut().insert(
		header::CONTENT_TYPE,
		HeaderValue::from_static("application/epub+zip"),
	);
	if let Ok(value) = HeaderValue::from_str(&format!(
		"attachment; filename=\"{}\"",
		comic_kepub_filename(source_path)
	)) {
		response
			.headers_mut()
			.insert(header::CONTENT_DISPOSITION, value);
	}
	Ok(response)
}

fn comic_kepub_filename(source_path: &str) -> String {
	let source_name = Path::new(source_path)
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or("book")
		.replace('"', "_");

	for extension in [".cbz", ".zip", ".cbr", ".rar", ".pdf"] {
		if let Some(stem) = source_name
			.strip_suffix(extension)
			.or_else(|| source_name.strip_suffix(&extension.to_ascii_uppercase()))
		{
			return format!("{stem}.kepub.epub");
		}
	}
	format!("{source_name}.kepub.epub")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn comic_filenames_strip_comic_extensions() {
		assert_eq!(comic_kepub_filename("/books/comic.cbz"), "comic.kepub.epub");
		assert_eq!(comic_kepub_filename("/books/comic.CBZ"), "comic.kepub.epub");
		assert_eq!(comic_kepub_filename("/books/comic.zip"), "comic.kepub.epub");
		assert_eq!(comic_kepub_filename("/books/comic.cbr"), "comic.kepub.epub");
		assert_eq!(comic_kepub_filename("/books/comic.pdf"), "comic.kepub.epub");
		assert_eq!(
			comic_kepub_filename("/books/weird.name"),
			"weird.name.kepub.epub"
		);
	}

	#[test]
	fn filenames_never_allow_quote_injection() {
		assert_eq!(comic_kepub_filename("book\".cbz"), "book_.kepub.epub");
	}

	#[test]
	fn comic_source_detection_matches_the_media_crate() {
		assert!(is_comic_source("/books/a.cbz"));
		assert!(!is_comic_source("/books/a.epub"));
	}
}
