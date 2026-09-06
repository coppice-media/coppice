//! Comic (CBZ/ZIP/CBR/RAR/PDF) delivery for Kobo devices: with
//! `transform_enabled`, the sync covers comic containers and the book file
//! route serves a fixed-layout KEPUB whose pages were transformed for the
//! requesting device instead of the original container.
//!
//! Profile resolution: the device's stored `transform_profile` (resolved via
//! `DeviceService::device_for_credential`) wins; otherwise the configured
//! `transform_kobo_profile` preset applies. A failed transform falls back to
//! the original file — delivery is best-effort and must never break a
//! download.
//!
//! The sync advertises each comic as `KEPUB`; its `Size` is the cached
//! transformed file's size once one exists for the device's profile, and the
//! source size until then (as the EPUB→KEPUB path already does).

use std::{path::Path, sync::LazyLock};

use axum::{
	body::Body,
	extract::Request,
	http::{header, HeaderMap, HeaderValue},
	response::{IntoResponse, Response},
};
use models::entity::{device, media};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use stump_auth::AuthContext;
use stump_core::{
	config::StumpConfig,
	kobo::sync_types::{BookMetadata, Format, SyncItem},
};
use stump_devices::{CredentialRef, DeviceResult};
use stump_media::transform::{
	comic_info_xml,
	container::{build_cbz, build_kepub},
	transform_pages_blocking, ComicContainer, ComicPages, JpegSubsampling,
	TransformCache, TransformError, TransformFormat, TransformProfile, COMIC_EXTENSIONS,
};
use tower_http::services::ServeFile;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

/// Cache file extension for transformed comics.
const KEPUB_EXTENSION: &str = "kepub.epub";

/// Media extensions a Kobo sync session covers: EPUBs always, and the comic
/// containers too when transform delivery is enabled (they are then served
/// as KEPUB, so the sync advertises them that way).
pub(crate) fn sync_extensions(config: &StumpConfig) -> &'static [&'static str] {
	static WITH_COMICS: LazyLock<Vec<&'static str>> =
		LazyLock::new(|| std::iter::once("epub").chain(COMIC_EXTENSIONS).collect());

	if config.transform.transform_enabled {
		&WITH_COMICS
	} else {
		&["epub"]
	}
}

/// Resolve the effective transform profile for this request, or `None` when
/// no profile applies (unknown config preset, broken device profile, …).
///
/// The result is coerced to what Kobo firmware can open at this route: the
/// fixed-layout KEPUB container, and JPEG in place of WebP pages (Kobo
/// renders JPEG/PNG/GIF/BMP/TIFF only).
pub(crate) async fn resolve_profile(
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
		container: ComicContainer::KepubFixedLayout,
		format: match profile.format {
			TransformFormat::Webp { quality } => TransformFormat::Jpeg {
				quality,
				subsampling: JpegSubsampling::Auto,
			},
			format => format,
		},
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

fn cache(ctx: &AppState) -> TransformCache {
	TransformCache::new(
		ctx.config.get_transform_cache_dir(),
		ctx.config.transform.transform_cache_max_bytes,
	)
}

/// The size of the cached transformed KEPUB for `book` under `profile`, when
/// one exists; this is the `Content-Length` the download route will send.
async fn cached_kepub_size(
	ctx: &AppState,
	profile: &TransformProfile,
	book: &media::MediaIdentSelect,
) -> Option<u64> {
	let source_mtime_ns = source_mtime_nanos(&book.path).await.ok()?;
	let cache_path =
		cache(ctx).path_for(&book.id, source_mtime_ns, profile, KEPUB_EXTENSION);
	tokio::fs::metadata(cache_path)
		.await
		.ok()
		.map(|metadata| metadata.len())
}

/// The download descriptor the metadata route advertises for a comic: the
/// KEPUB format and, once transformed for this device, its exact size.
pub(crate) async fn advertised_download(
	ctx: &AppState,
	auth: &AuthContext,
	book: &media::MediaIdentSelect,
) -> (Format, Option<u64>) {
	let Some(profile) = resolve_profile(ctx, auth).await else {
		return (Format::KEPUB, None);
	};
	(Format::KEPUB, cached_kepub_size(ctx, &profile, book).await)
}

/// Replace the advertised `Size` of every comic in a sync page with the cached
/// transformed KEPUB's size when one exists for this device's profile.
///
/// Comics are the entitlements the sync marked `KEPUB`; EPUBs keep their
/// source size exactly as before.
pub(crate) async fn advertise_cached_sizes(
	ctx: &AppState,
	auth: &AuthContext,
	items: &mut [SyncItem],
) {
	let mut comics: Vec<&mut BookMetadata> = items
		.iter_mut()
		.filter_map(|item| match item {
			SyncItem::NewEntitlement(entitlement)
			| SyncItem::ChangedEntitlement(entitlement) => Some(&mut entitlement.book_metadata),
			SyncItem::ChangedProductMetadata(metadata) => Some(metadata),
			_ => None,
		})
		.filter(|metadata| {
			metadata
				.download_urls
				.iter()
				.any(|url| url.format == Format::KEPUB)
		})
		.collect();
	if comics.is_empty() {
		return;
	}
	let Some(profile) = resolve_profile(ctx, auth).await else {
		return;
	};

	let ids: Vec<&str> = comics
		.iter()
		.map(|metadata| metadata.entitlement_id.as_str())
		.collect();
	let books = match media::Entity::find()
		.filter(media::Column::Id.is_in(ids))
		.into_model::<media::MediaIdentSelect>()
		.all(ctx.conn.as_ref())
		.await
	{
		Ok(books) => books,
		Err(error) => {
			tracing::warn!(?error, "Failed to load comics for Kobo size advertisement");
			return;
		},
	};

	for book in books {
		let Some(size) = cached_kepub_size(ctx, &profile, &book).await else {
			continue;
		};
		for metadata in comics
			.iter_mut()
			.filter(|metadata| metadata.entitlement_id == book.id)
		{
			for url in &mut metadata.download_urls {
				url.size = size;
			}
		}
	}
}

/// Serve the comic `book` as a transformed KEPUB, falling back to the original
/// file when no profile resolves or the transform fails.
pub(crate) async fn serve_comic(
	ctx: AppState,
	auth: AuthContext,
	book: media::MediaIdentSelect,
	headers: HeaderMap,
) -> APIResult<Response> {
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

	let cache = cache(&ctx);
	let cache_path = cache.path_for(&book.id, source_mtime_ns, &profile, KEPUB_EXTENSION);

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
			return super::kepub::serve_original(ctx, auth, book.id, headers).await;
		},
		Err(error) => {
			tracing::error!(?error, "Comic transform task panicked");
			return super::kepub::serve_original(ctx, auth, book.id, headers).await;
		},
	}

	// Best-effort LRU sweep after publishing a new entry.
	let _ = tokio::task::spawn_blocking(move || match cache.sweep() {
		Ok(sweep) => tracing::trace!(?sweep, "Transform cache sweep complete"),
		Err(error) => tracing::debug!(?error, "Transform cache sweep failed"),
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
) -> Result<(), TransformError> {
	std::fs::create_dir_all(cache_dir)?;

	let temp_path = cache_dir.join(format!(
		"{}.{}.tmp",
		cache_path
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or("transform"),
		uuid::Uuid::new_v4()
	));

	let result =
		build_to_temp(source_path, &temp_path, profile, media_config).and_then(|()| {
			TransformCache::publish(&temp_path, cache_path).map_err(TransformError::from)
		});

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
) -> Result<(), TransformError> {
	let source = ComicPages::open(source_path, media_config)?;
	let title = source_path
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or("comic");

	let file = std::fs::File::create(temp_path)?;
	let sink = std::io::BufWriter::with_capacity(1 << 16, file);
	let pages = transform_pages_blocking(
		source.into_iter(media_config),
		profile.clone(),
		media_config.cpu_concurrency_limit().max(1),
	);

	let sink = match profile.container {
		ComicContainer::KepubFixedLayout => build_kepub(pages, title, sink)?,
		ComicContainer::Cbz => {
			let comic_info = comic_info_xml(source_path);
			build_cbz(pages, comic_info.as_deref(), sink)?
		},
	};
	sink.into_inner()
		.map_err(|error| error.into_error())?
		.sync_all()?;
	Ok(())
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
			APIError::InternalServerError("Failed to serve transformed comic".to_string())
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

	let stem = Path::new(&source_name)
		.extension()
		.and_then(|extension| extension.to_str())
		.filter(|extension| {
			COMIC_EXTENSIONS
				.iter()
				.any(|comic| comic.eq_ignore_ascii_case(extension))
		})
		.map_or(source_name.as_str(), |extension| {
			&source_name[..source_name.len() - extension.len() - 1]
		});
	format!("{stem}.{KEPUB_EXTENSION}")
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
		assert_eq!(comic_kepub_filename("/books/comic.rar"), "comic.kepub.epub");
		assert_eq!(comic_kepub_filename("/books/comic.pdf"), "comic.kepub.epub");
		assert_eq!(
			comic_kepub_filename("/books/weird.name"),
			"weird.name.kepub.epub"
		);
		assert_eq!(comic_kepub_filename("/books/pdf"), "pdf.kepub.epub");
	}

	#[test]
	fn filenames_never_allow_quote_injection() {
		assert_eq!(comic_kepub_filename("book\".cbz"), "book_.kepub.epub");
	}
}
