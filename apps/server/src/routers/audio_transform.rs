//! Per-device audiobook delivery: whether a track's bytes are served as
//! stored or transcoded first.
//!
//! This is the third of the three audio settings layers. The library layer
//! (`stump_ingest::policy::AudioPolicy`) decides what ingest *produces*, the
//! server layer (`StumpConfig::audio`) decides what an assemble *writes*, and
//! this one decides only how a *request* is answered — so a device asking for
//! Opus never changes a byte in the library.
//!
//! Resolution: the requesting device is
//! [`stump_auth::AuthContext::device_id`], already resolved by whichever
//! middleware authenticated the request, and its stored `transform_profile`
//! JSON carries the `audio` section. No device, no profile, or a profile that
//! says [`AudioOutput::Passthrough`] (the default for every preset but
//! `phone-opus`) all mean the same thing: `ServeFile` over the stored path
//! with the stored MIME, exactly as before this module existed.
//!
//! An Opus transcode is cached in the comic transform cache directory under
//! its own key ([`TransformCache::audio_path_for`]) and swept by the same byte
//! budget, because it is the same kind of artifact: an expensive, reproducible
//! re-encode of a file the server already has. A failed transcode falls back
//! to the stored bytes — delivery is best-effort, and a missing `ffmpeg` must
//! degrade playback quality, never break playback.

use std::{
	path::{Path, PathBuf},
	time::UNIX_EPOCH,
};

use axum::{
	body::Body,
	extract::Request,
	http::{header, HeaderMap, HeaderValue},
	response::{IntoResponse, Response},
};
use models::entity::{device, media_audio_track};
use sea_orm::EntityTrait;
use stump_media::transform::{
	AudioOutput, AudioProfile, TransformCache, TransformProfile, AUDIO_OGG,
};
use stump_tools::ffmpeg::{self, Codec, Encode};
use tower_http::services::ServeFile;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

/// Cache file extension for a transcoded track. `ffmpeg` picks its muxer from
/// the output extension, so this is also what selects Ogg for the Opus
/// stream.
const OPUS_EXTENSION: &str = "ogg";

/// The audio delivery rules of the device that authenticated this request.
///
/// Every failure mode — no device, a device row that vanished, unreadable
/// profile JSON — resolves to the default (passthrough) rather than an error:
/// a device whose profile the operator broke must still be able to play its
/// books.
pub(crate) async fn resolve_audio_profile(
	ctx: &AppState,
	device_id: Option<&str>,
) -> AudioProfile {
	let Some(device_id) = device_id else {
		return AudioProfile::default();
	};

	let device = match device::Entity::find_by_id(device_id)
		.one(ctx.conn.as_ref())
		.await
	{
		Ok(Some(device)) => device,
		Ok(None) => return AudioProfile::default(),
		Err(error) => {
			tracing::warn!(
				?error,
				device_id,
				"Failed to resolve device for audio delivery"
			);
			return AudioProfile::default();
		},
	};

	let Some(profile_json) = device.transform_profile.as_ref() else {
		return AudioProfile::default();
	};
	match TransformProfile::from_device_profile(profile_json) {
		Some(Ok(profile)) => profile.audio,
		Some(Err(error)) => {
			tracing::warn!(
				?error,
				device_id = %device.id,
				"Invalid device transform profile; serving audio as stored"
			);
			AudioProfile::default()
		},
		None => AudioProfile::default(),
	}
}

/// Serve one track's bytes, honouring `profile`.
///
/// The response always carries `Accept-Ranges` and answers `Range` with a
/// `206` (`ServeFile` does both), for the stored file and the transcode
/// alike: a player seeks by byte range, and a route that ignored `Range`
/// would force a full download per seek.
pub(crate) async fn serve_track(
	ctx: &AppState,
	headers: HeaderMap,
	track: &media_audio_track::Model,
	profile: &AudioProfile,
) -> APIResult<Response> {
	let source = Path::new(&track.path);

	let AudioOutput::Opus { bitrate } = &profile.output else {
		return stored_response(source, &track.mime, headers).await;
	};

	match transcoded_path(ctx, track, profile, bitrate).await {
		// The stored MIME describes the source, so it would be a lie on a
		// transcoded body; the output's own type is the truth.
		Some(path) => stored_response(&path, AUDIO_OGG, headers).await,
		None => stored_response(source, &track.mime, headers).await,
	}
}

/// The cached Opus rendition of `track`, transcoding it first if necessary.
/// `None` on any failure, which the caller answers with the stored bytes.
async fn transcoded_path(
	ctx: &AppState,
	track: &media_audio_track::Model,
	profile: &AudioProfile,
	bitrate: &str,
) -> Option<PathBuf> {
	let mtime_ns = source_mtime_nanos(&track.path).await.ok()?;
	let cache = cache(ctx);
	let cache_path = cache.audio_path_for(
		&track.media_id,
		track.index,
		mtime_ns,
		profile,
		OPUS_EXTENSION,
	);

	if cache.hit(&cache_path) {
		return Some(cache_path);
	}

	let bin_dir = ffmpeg_bin_dir(ctx);
	let transcode = tokio::task::spawn_blocking({
		let source = PathBuf::from(&track.path);
		let cache_dir = cache.dir().to_path_buf();
		let cache_path = cache_path.clone();
		let bitrate = bitrate.to_string();
		let duration_ms = track.duration_ms;
		move || {
			build_opus(
				&source,
				&cache_dir,
				&cache_path,
				&bitrate,
				duration_ms,
				bin_dir.as_deref(),
			)
		}
	})
	.await;

	match transcode {
		Ok(Ok(())) => {},
		Ok(Err(error)) => {
			tracing::warn!(
				%error,
				path = %track.path,
				"Opus transcode failed; serving the stored audio"
			);
			return None;
		},
		Err(error) => {
			tracing::error!(?error, "Opus transcode task panicked");
			return None;
		},
	}

	// Best-effort LRU sweep after publishing a new entry, as the comic
	// transform does: the audio and comic caches share one byte budget
	// because they share one directory.
	let _ = tokio::task::spawn_blocking(move || match cache.sweep() {
		Ok(sweep) => tracing::trace!(?sweep, "Transform cache sweep complete"),
		Err(error) => tracing::debug!(?error, "Transform cache sweep failed"),
	})
	.await;

	Some(cache_path)
}

/// Transcode into the cache: staging directory → rename, so a killed or
/// failed run never leaves a truncated file behind for the next request to
/// serve.
///
/// The staging directory is a temp *subdirectory* of the cache, for two
/// reasons: the rename into place is then within one filesystem, so it is
/// atomic, and the sweep enumerates only the files directly in the cache
/// directory, so it can neither delete an in-flight transcode nor count it
/// against the budget. The directory is removed when this returns, whichever
/// way it returns.
fn build_opus(
	source: &Path,
	cache_dir: &Path,
	cache_path: &Path,
	bitrate: &str,
	duration_ms: i64,
	bin_dir: Option<&Path>,
) -> Result<(), String> {
	std::fs::create_dir_all(cache_dir).map_err(|error| error.to_string())?;

	let install = ffmpeg::locate(bin_dir).map_err(|error| error.to_string())?;

	let staging = tempfile::Builder::new()
		.prefix("audio-transcode-")
		.tempdir_in(cache_dir)
		.map_err(|error| error.to_string())?;
	// `ffmpeg` reads the muxer off the output path, so the staging name
	// carries the real extension rather than a `.tmp` suffix.
	let staged = staging.path().join(format!("track.{OPUS_EXTENSION}"));

	let inputs = [source.to_path_buf()];
	let output = ffmpeg::encode(
		&install,
		&Encode {
			inputs: &inputs,
			output: &staged,
			codec: Codec::Opus,
			bitrate: Some(bitrate),
			metadata: None,
			cover: None,
			// MP4-only flag; an Ogg stream has no `moov` atom to move.
			faststart: false,
			timeout: ffmpeg::timeout_for(duration_ms),
		},
	)
	.map_err(|error| error.to_string())?;

	if !output.succeeded() {
		return Err(format!(
			"ffmpeg {}: {}",
			output.describe_status(),
			output.stderr.trim()
		));
	}

	TransformCache::publish(&staged, cache_path).map_err(|error| error.to_string())
}

/// Stream `path` with `ServeFile`, forcing `mime` as the content type.
///
/// `ServeFile` guesses from the extension, and `.m4b` is missing from most
/// mime tables; the manifest already told the client what a track is, so the
/// two can never disagree.
async fn stored_response(
	path: &Path,
	mime: &str,
	headers: HeaderMap,
) -> APIResult<Response> {
	let mut serve_req = Request::new(Body::empty());
	*serve_req.headers_mut() = headers;

	let mut response = ServeFile::new(path)
		.try_call(serve_req)
		.await
		.map_err(|error| {
			tracing::error!(?error, ?path, "Failed to serve audio track");
			APIError::NotFound("Track file is missing".to_string())
		})?
		.into_response();

	if let Ok(value) = mime.parse::<HeaderValue>() {
		response.headers_mut().insert(header::CONTENT_TYPE, value);
	}
	if let Some(filename) = path.file_name().and_then(|name| name.to_str()) {
		// `inline`, not `attachment`: this is a playback route, and a browser
		// that downloads every track cannot play the book.
		response.headers_mut().insert(
			header::CONTENT_DISPOSITION,
			format!("inline; filename=\"{filename}\"")
				.parse()
				.unwrap_or_else(|_| HeaderValue::from_static("inline")),
		);
	}
	Ok(response)
}

fn cache(ctx: &AppState) -> TransformCache {
	TransformCache::new(
		ctx.config.get_transform_cache_dir(),
		ctx.config.transform.transform_cache_max_bytes,
	)
}

/// The directory `ffmpeg` is looked for in, from `STUMP_AUDIO_FFMPEG`.
///
/// The config holds the binary's own path (validated at startup), and
/// `ffmpeg::locate` searches a *directory*, so the parent is what it wants;
/// empty means "search `PATH`".
fn ffmpeg_bin_dir(ctx: &AppState) -> Option<PathBuf> {
	let configured = ctx.config.audio.audio_ffmpeg.trim();
	if configured.is_empty() {
		return None;
	}
	Path::new(configured).parent().map(Path::to_path_buf)
}

async fn source_mtime_nanos(path: &str) -> std::io::Result<u128> {
	let metadata = tokio::fs::metadata(path).await?;
	Ok(metadata
		.modified()?
		.duration_since(UNIX_EPOCH)
		.map_or(0, |duration| duration.as_nanos()))
}
