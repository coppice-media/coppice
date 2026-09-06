//! Audiobooks as OPDS 2.0 acquisitions.
//!
//! A book in this feed has always been one file with a page count, and an
//! audiobook breaks both halves of that. A folder audiobook's `media.path` is
//! a directory, so the single `/opds/v2.0/books/{id}/file` acquisition every
//! other publication carries cannot serve anything: the only acquisition that
//! can succeed is one link per track, pointing at the native
//! `/api/v2/media/{id}/audio/track/{index}` route that already serves ranges.
//! A single-container audiobook is unaffected — one file, one link, as before.
//!
//! Chapters are time marks, not resources, so they become the Readium `toc`:
//! each entry names the track holding the mark plus a W3C media fragment
//! (`#t=`) for the offset inside it, which is the only way a URL can point at
//! a time. The length of that collection is the chapter count a client shows.
//!
//! Playback itself is not this module's business. `GET
//! /api/v2/media/{id}/audio/manifest` is the Readium manifest with the
//! `readingOrder` a player wants; OPDS answers "how do I get the bytes".

use std::collections::HashMap;

use models::{
	entity::{media_audio, media_audio_chapter, media_audio_track},
	services::audio::AudioBook,
};
use sea_orm::{prelude::*, QueryOrder};
use stump_media::ContentType;

use crate::CoreResult;

use super::{
	link::{
		OPDSAudioLinkBuilder, OPDSBaseLinkBuilder, OPDSLink, OPDSLinkFinalizer,
		OPDSLinkRel, OPDSLinkType,
	},
	properties::{OPDSProperties, AUTH_ROUTE},
};

/// The native route serving one track of an audiobook. `index` is 0-based and
/// the route answers range requests, which is what makes it the acquisition
/// target for a multi-file book.
fn track_route(media_id: &str, index: i32) -> String {
	format!("/api/v2/media/{media_id}/audio/track/{index}")
}

/// Milliseconds as the seconds every Readium duration and media fragment is
/// stated in.
fn seconds(milliseconds: i64) -> f64 {
	milliseconds as f64 / 1000.0
}

/// Whether a media row could be an audiobook at all.
///
/// The scanner classifies audio by extension and stores an extension on every
/// media row — a folder audiobook carries its dominant track's — so a row
/// whose extension is not audio has no `media_audio` row to find. Asking here
/// spares every comic and EPUB in a feed page an audio query.
pub fn is_audio(extension: &str) -> bool {
	ContentType::from_extension(extension).is_audio()
}

/// The audiobooks among `media_ids`, keyed by media id.
///
/// [`models::services::audio::book`] is three queries for one book, and a
/// feed page asks for twenty; this issues the same three once with `is_in`
/// and groups the rows in memory. Callers filter `media_ids` with
/// [`is_audio`] first, so a page of comics reaches this and issues nothing.
///
/// Tracks and chapters come back ascending by `index`, matching
/// `models::services::audio` — that ordering is what makes the acquisition
/// order and the `toc` order correct, so it is not incidental.
pub async fn books<C: ConnectionTrait>(
	conn: &C,
	media_ids: Vec<String>,
) -> Result<HashMap<String, AudioBook>, DbErr> {
	if media_ids.is_empty() {
		return Ok(HashMap::new());
	}

	let audio = media_audio::Entity::find()
		.filter(media_audio::Column::MediaId.is_in(media_ids))
		.all(conn)
		.await?;

	if audio.is_empty() {
		return Ok(HashMap::new());
	}

	let audio_ids = audio
		.iter()
		.map(|audio| audio.media_id.clone())
		.collect::<Vec<_>>();

	let tracks = media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.is_in(audio_ids.clone()))
		.order_by_asc(media_audio_track::Column::Index)
		.all(conn)
		.await?;
	let chapters = media_audio_chapter::Entity::find()
		.filter(media_audio_chapter::Column::MediaId.is_in(audio_ids))
		.order_by_asc(media_audio_chapter::Column::Index)
		.all(conn)
		.await?;

	let mut books = audio
		.into_iter()
		.map(|audio| {
			(
				audio.media_id.clone(),
				AudioBook {
					audio,
					tracks: Vec::new(),
					chapters: Vec::new(),
				},
			)
		})
		.collect::<HashMap<_, _>>();

	for track in tracks {
		if let Some(book) = books.get_mut(&track.media_id) {
			book.tracks.push(track);
		}
	}
	for chapter in chapters {
		if let Some(book) = books.get_mut(&chapter.media_id) {
			book.chapters.push(chapter);
		}
	}

	Ok(books)
}

/// One acquisition link per track, ascending by `index`, or `None` when the
/// caller's single file link is still the right answer.
///
/// Fewer than two tracks means the publication is one file: `/file` and track
/// `0` are the same bytes, and the file link already advertises the audio MIME
/// through the media extension. More than one track means `media.path` is a
/// directory and only the per-track route can be acquired.
///
/// Each link carries the track's stored MIME and byte size verbatim; the
/// probe decided both and re-deriving them here could only disagree.
pub fn acquisition_links(
	media_id: &str,
	audio: &AudioBook,
	finalizer: &OPDSLinkFinalizer,
) -> CoreResult<Option<Vec<OPDSLink>>> {
	if audio.tracks.len() < 2 {
		return Ok(None);
	}

	let mut links = Vec::with_capacity(audio.tracks.len());

	for track in &audio.tracks {
		links.push(OPDSLink::Audio(
			OPDSAudioLinkBuilder::default()
				.duration(seconds(track.duration_ms))
				.base_link(
					OPDSBaseLinkBuilder::default()
						.href(track_route(media_id, track.index))
						.rel(OPDSLinkRel::Acquisition.item())
						._type(OPDSLinkType::Custom(track.mime.clone()))
						.properties(
							OPDSProperties::default()
								.with_length(track.byte_size)
								.with_auth(finalizer.format_link(AUTH_ROUTE)),
						)
						.build()?,
				)
				.build()?,
		));
	}

	Ok(Some(links))
}

/// The chapter marks as a Readium `toc`, ascending by `index`.
///
/// A mark is an offset into the publication, so an entry points at the track
/// containing it and carries the remainder as a media fragment. A chapter
/// whose `end_ms` the container never stated has no duration rather than a
/// guessed one.
pub fn toc(
	media_id: &str,
	audio: &AudioBook,
	finalizer: &OPDSLinkFinalizer,
) -> CoreResult<Vec<OPDSLink>> {
	let mut toc = Vec::with_capacity(audio.chapters.len());

	for chapter in &audio.chapters {
		// Only a book with no tracks at all, which has nothing to point at.
		let Some(track) = audio.track_at(chapter.start_ms) else {
			continue;
		};

		let offset_ms = (chapter.start_ms - track.start_offset_ms).max(0);
		let href = format!(
			"{}#t={}",
			track_route(media_id, track.index),
			seconds(offset_ms)
		);

		toc.push(OPDSLink::Audio(
			OPDSAudioLinkBuilder::default()
				.duration(
					chapter
						.end_ms
						.map(|end_ms| seconds(end_ms - chapter.start_ms)),
				)
				.base_link(
					OPDSBaseLinkBuilder::default()
						.title(chapter.title.clone())
						.href(href)
						._type(OPDSLinkType::Custom(track.mime.clone()))
						.properties(
							OPDSProperties::default()
								.with_auth(finalizer.format_link(AUTH_ROUTE)),
						)
						.build()?,
				)
				.build()?,
		));
	}

	Ok(toc)
}
