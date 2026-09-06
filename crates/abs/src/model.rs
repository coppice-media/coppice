//! The Stump-side facts the profile maps onto the Audiobookshelf wire.
//!
//! Everything here is what the *server* knows, in Stump's own units
//! (milliseconds from the start of the publication), independent of the JSON
//! in [`crate::dto`]. The server adapter fills these out of
//! `media_audio`/`media_audio_tracks`/`media_audio_chapters`, the unified
//! reading state and the bookmark table; the profile itself never touches
//! those tables, which is what keeps the whole route surface testable against
//! an in-memory stub.

use chrono::{DateTime, Utc};

/// Audio facts about one Stump media row: what
/// [`AbsBackend::audio`](crate::routes::AbsBackend::audio) answers.
///
/// `duration_ms` is the publication's total, i.e. the sum of the track
/// durations, and every track's `start_offset_ms` is measured from the start
/// of the publication rather than of its file.
///
/// There is deliberately no narrator *here*: it is not an audio fact but a
/// metadata one, and since `m20260942` it has its own column,
/// `media_metadata.narrators`, alongside every other credit. The probe fills
/// it from the container's `composer`/`©wrt` tag, and the mapper reads
/// Audiobookshelf's `narrators`/`narratorName` straight out of it — so this
/// struct stays exactly what it is, the shape of the recording.
#[derive(Debug, Clone, PartialEq)]
pub struct AbsAudio {
	pub duration_ms: i64,
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	pub tracks: Vec<AbsAudioTrack>,
	pub chapters: Vec<AbsAudioChapter>,
}

impl AbsAudio {
	/// The track holding `index`, by its 0-based playback index — the `ino`
	/// Audiobookshelf clients pass to `GET /api/items/{id}/file/{ino}`.
	pub fn track(&self, index: i32) -> Option<&AbsAudioTrack> {
		self.tracks.iter().find(|track| track.index == index)
	}

	/// The track playing at `position_ms`, for a position that arrived
	/// without a track index.
	pub fn track_at(&self, position_ms: i64) -> Option<&AbsAudioTrack> {
		self.tracks
			.iter()
			.filter(|track| track.start_offset_ms <= position_ms)
			.next_back()
			.or_else(|| self.tracks.first())
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsAudioTrack {
	/// 0-based playback order.
	pub index: i32,
	pub path: String,
	pub duration_ms: i64,
	/// Milliseconds of the publication before this track starts.
	pub start_offset_ms: i64,
	pub byte_size: i64,
	pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsAudioChapter {
	/// 0-based; Audiobookshelf's `chapters[].id`.
	pub index: i32,
	pub title: Option<String>,
	pub start_ms: i64,
	pub end_ms: Option<i64>,
}

/// One user's listening position for one media row, as the unified reading
/// state resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsProgress {
	pub position_ms: i64,
	pub track_index: Option<i32>,
	pub is_finished: bool,
	pub started_at: DateTime<Utc>,
	pub last_update: DateTime<Utc>,
	pub finished_at: Option<DateTime<Utc>>,
}

/// A position update arriving from a client, in Stump's units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbsPositionUpdate {
	pub position_ms: i64,
	pub track_index: Option<i32>,
	/// Total publication length, for the progression the reading state
	/// stores alongside the position.
	pub duration_ms: i64,
	/// Wall-clock listening time this update accounts for; `0` for a
	/// position-only update such as `PATCH /api/me/progress/{id}`.
	pub elapsed_ms: i64,
	pub is_finished: Option<bool>,
}

/// One audio bookmark. Audiobookshelf identifies a bookmark by
/// `(libraryItemId, time)` with whole-second resolution, so `position_ms` is
/// always a multiple of 1000 on this surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsBookmark {
	pub position_ms: i64,
	pub title: String,
	pub created_at: DateTime<Utc>,
}

/// Image bytes with their MIME type, as the cover route serves them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsImage {
	pub content_type: String,
	pub data: Vec<u8>,
}

/// Which of abs-ref's three library-item shapes a response carries.
///
/// abs-ref signals the shape only by which keys are present, so the mapper
/// takes it as an argument instead of guessing:
///
/// | shape | request | keys |
/// | --- | --- | --- |
/// | [`ItemShape::Minified`] | `?minified=1` list rows | `numFiles`, `size`, minified metadata |
/// | [`ItemShape::Detail`] | `GET /api/items/{id}` | `libraryFiles`, `lastScan`, full metadata |
/// | [`ItemShape::Expanded`] | `?expanded=1`, search hits, `batch/get` | both, plus `media.tracks[]` |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemShape {
	Minified,
	Detail,
	Expanded,
}

impl ItemShape {
	pub fn is_minified(self) -> bool {
		matches!(self, ItemShape::Minified)
	}

	/// Whether the shape carries `numFiles`/`size` and the minified
	/// `authorName`/`seriesName` pair.
	pub fn has_minified_keys(self) -> bool {
		matches!(self, ItemShape::Minified | ItemShape::Expanded)
	}

	/// Whether the shape carries `libraryFiles`/`lastScan`/`scanVersion` and
	/// the full `authors`/`series` objects.
	pub fn has_detail_keys(self) -> bool {
		matches!(self, ItemShape::Detail | ItemShape::Expanded)
	}

	/// Whether `media.tracks[]` (audio files with `contentUrl`) is emitted.
	pub fn has_tracks(self) -> bool {
		matches!(self, ItemShape::Expanded)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn track(index: i32, start_offset_ms: i64, duration_ms: i64) -> AbsAudioTrack {
		AbsAudioTrack {
			index,
			path: format!("/books/part{index}.mp3"),
			duration_ms,
			start_offset_ms,
			byte_size: 1024,
			mime: "audio/mpeg".to_owned(),
		}
	}

	fn audio(tracks: Vec<AbsAudioTrack>) -> AbsAudio {
		AbsAudio {
			duration_ms: tracks.iter().map(|t| t.duration_ms).sum(),
			codec: "mp3".to_owned(),
			sample_rate: Some(44100),
			channels: Some(2),
			bitrate: Some(64000),
			tracks,
			chapters: Vec::new(),
		}
	}

	#[test]
	fn track_at_picks_the_track_covering_the_position() {
		let audio = audio(vec![
			track(0, 0, 10_000),
			track(1, 10_000, 10_000),
			track(2, 20_000, 5_000),
		]);

		// A boundary belongs to the track that starts there, not the one
		// that ends there: a client seeking to 10s must get track 1.
		assert_eq!(audio.track_at(0).unwrap().index, 0);
		assert_eq!(audio.track_at(9_999).unwrap().index, 0);
		assert_eq!(audio.track_at(10_000).unwrap().index, 1);
		assert_eq!(audio.track_at(24_999).unwrap().index, 2);
		// Past the end stays on the last track rather than falling off it.
		assert_eq!(audio.track_at(999_999).unwrap().index, 2);
	}

	#[test]
	fn track_at_never_returns_none_for_a_negative_position() {
		let audio = audio(vec![track(0, 0, 10_000)]);
		assert_eq!(audio.track_at(-5_000).unwrap().index, 0);
	}

	#[test]
	fn track_lookup_is_by_playback_index_not_position() {
		let audio = audio(vec![track(0, 0, 10_000), track(7, 10_000, 10_000)]);
		assert_eq!(audio.track(7).unwrap().start_offset_ms, 10_000);
		assert!(audio.track(1).is_none());
	}
}
