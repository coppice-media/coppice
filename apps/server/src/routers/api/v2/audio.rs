//! The audiobook playback surface: a manifest and the track bytes.
//!
//! A player needs two things Stump's page-oriented routes cannot express: the
//! shape of the publication (how long it is, how it is split into files, where
//! the chapter marks fall) and a way to fetch one file's bytes with a `Range`
//! request. Those are the two routes here.
//!
//! Every time value in the manifest is milliseconds from the start of the
//! **publication**, the same unit as `reading_heads.position_ms`, so a client
//! resumes by comparing numbers rather than converting between per-file and
//! per-book offsets. `startOffsetMs` is the running sum that maps a
//! publication offset onto a track, and `byteSize` is present so a client can
//! plan its range requests without a `HEAD` per track.

use axum::{
	body::Body,
	extract::{Path, Request, State},
	http::{header, HeaderMap},
	response::IntoResponse,
	Extension, Json,
};
use models::{
	entity::{media, media_audio, media_audio_chapter, media_audio_track},
	services::audio,
};
use sea_orm::prelude::*;
use serde::{Deserialize, Serialize};
use stump_auth::AuthContext;
use tower_http::services::ServeFile;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
};

/// The shape of one audio publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioManifest {
	pub media_id: String,
	/// The whole-publication duration; the sum of the track durations.
	pub duration_ms: i64,
	/// Short lowercase codec name, or `mixed` for a folder book whose files
	/// do not agree.
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	/// How the chapter marks were obtained: `mp4_chpl`, `mp4_chapter_track`,
	/// `id3_chap`, `vorbis_comment`, `per_track` or `none`. A client that
	/// wants to hide chapters Stump synthesized branches on `per_track`.
	pub chapter_source: String,
	/// Playback order, contiguous from index 0.
	pub tracks: Vec<AudioManifestTrack>,
	/// Ascending by `startMs`. Empty exactly when `chapterSource` is `none`.
	pub chapters: Vec<AudioManifestChapter>,
}

/// One file of the publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioManifestTrack {
	/// 0-based position in playback order; also the `{index}` of [`url`].
	///
	/// [`url`]: AudioManifestTrack::url
	pub index: i32,
	pub mime: String,
	pub duration_ms: i64,
	/// Milliseconds from the start of the publication to the start of this
	/// file. A publication offset belongs to the last track whose
	/// `startOffsetMs` is at or below it.
	pub start_offset_ms: i64,
	pub byte_size: i64,
	/// Where to fetch the bytes. Relative to the server root so the client
	/// does not have to know how Stump is mounted.
	pub url: String,
}

/// One chapter mark of the publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioManifestChapter {
	pub index: i32,
	pub title: Option<String>,
	pub start_ms: i64,
	/// The last chapter's end is the publication duration; a container that
	/// only carries start marks still gets an end here, because the probe
	/// closes every window against the next mark.
	pub end_ms: Option<i64>,
}

impl AudioManifest {
	/// Build the wire manifest from the persisted rows.
	fn new(
		audio: media_audio::Model,
		tracks: Vec<media_audio_track::Model>,
		chapters: Vec<media_audio_chapter::Model>,
	) -> Self {
		let media_id = audio.media_id;
		AudioManifest {
			tracks: tracks
				.into_iter()
				.map(|track| AudioManifestTrack {
					url: track_url(&media_id, track.index),
					index: track.index,
					mime: track.mime,
					duration_ms: track.duration_ms,
					start_offset_ms: track.start_offset_ms,
					byte_size: track.byte_size,
				})
				.collect(),
			chapters: chapters
				.into_iter()
				.map(|chapter| AudioManifestChapter {
					index: chapter.index,
					title: chapter.title,
					start_ms: chapter.start_ms,
					end_ms: chapter.end_ms,
				})
				.collect(),
			duration_ms: audio.duration_ms,
			codec: audio.codec,
			sample_rate: audio.sample_rate,
			channels: audio.channels,
			bitrate: audio.bitrate,
			chapter_source: audio.chapter_source.to_string(),
			media_id,
		}
	}
}

/// The route a client fetches one track's bytes from.
pub(crate) fn track_url(media_id: &str, index: i32) -> String {
	format!("/api/v2/media/{media_id}/audio/track/{index}")
}

/// Assert the requesting user can see the book at all. Audio inherits the
/// same visibility rule as every other book route: an age-restricted or
/// out-of-scope book is a 404, never a 403 that confirms it exists.
async fn visible_book(
	ctx: &AppState,
	req: &AuthContext,
	media_id: &str,
) -> APIResult<media::MediaIdentSelect> {
	media::Entity::find_for_user(&req.user())
		.filter(media::Column::Id.eq(media_id))
		.into_model::<media::MediaIdentSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_string()))
}

/// `GET /api/v2/media/{id}/audio/manifest`
///
/// A book that is not an audiobook is a 404 rather than an empty manifest: a
/// player must be able to tell "no audio here" from "an audiobook with no
/// tracks", and only one of those is a real publication.
pub(crate) async fn get_audio_manifest(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<AudioManifest>> {
	let book = visible_book(&ctx, &req, &id).await?;

	let audio::AudioBook {
		audio,
		tracks,
		chapters,
	} = audio::book(ctx.conn.as_ref(), &book.id)
		.await?
		.ok_or_else(|| APIError::NotFound("Book has no audio".to_string()))?;

	Ok(Json(AudioManifest::new(audio, tracks, chapters)))
}

/// `GET /api/v2/media/{id}/audio/track/{index}`
///
/// Serves the file's bytes through `ServeFile`, which answers a `Range`
/// request with `206` and always sets `Accept-Ranges: bytes` — a player seeks
/// by byte range, so a route that ignored `Range` would force a full download
/// per seek. The content type comes from the stored `mime` rather than from
/// sniffing the file again, so it always matches what the manifest advertised.
pub(crate) async fn get_audio_track(
	Path((id, index)): Path<(String, i32)>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	let book = visible_book(&ctx, &req, &id).await?;

	let track = audio::track(ctx.conn.as_ref(), &book.id, index)
		.await?
		.ok_or_else(|| APIError::NotFound("Track not found".to_string()))?;

	// Reuse the incoming headers so `Range`, `If-Range` and the conditional
	// headers all reach `ServeFile` untouched.
	let mut serve_req = Request::new(Body::empty());
	*serve_req.headers_mut() = headers;

	// `ServeFile` guesses a type from the extension, and `.m4b` is missing
	// from most mime tables; the manifest already told the client what this
	// track is, so the stored `mime` overwrites the guess and the two can
	// never disagree.
	match ServeFile::new(&track.path).try_call(serve_req).await {
		Ok(mut response) => {
			if let Ok(mime) = track.mime.parse::<header::HeaderValue>() {
				response.headers_mut().insert(header::CONTENT_TYPE, mime);
			}
			if let Some(filename) = std::path::Path::new(&track.path)
				.file_name()
				.and_then(|name| name.to_str())
			{
				// `inline`, not `attachment`: this is a playback route, and a
				// browser that downloads every track cannot play the book.
				response.headers_mut().insert(
					header::CONTENT_DISPOSITION,
					format!("inline; filename=\"{filename}\"")
						.parse()
						.unwrap_or_else(|_| header::HeaderValue::from_static("inline")),
				);
			}
			Ok(response)
		},
		Err(error) => {
			tracing::error!(?error, path = %track.path, "Failed to serve audio track");
			Err(APIError::NotFound("Track file is missing".to_string()))
		},
	}
}
