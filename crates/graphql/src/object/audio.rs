//! The audiobook shape as GraphQL objects.
//!
//! These are read-only projections of `media_audio`, `media_audio_tracks` and
//! `media_audio_chapters`. They are deliberately **not** gated behind the
//! `web` feature: an audiobook manifest has protocol callers (Audiobookshelf,
//! liseur, Kavita) as well as the SPA, so a headless build must still be able
//! to answer it.
//!
//! Every time value is milliseconds from the start of the **publication**, the
//! same unit as a reading head's `positionMs`, so a client resumes by
//! comparing numbers instead of converting per-file offsets.

use async_graphql::{Object, SimpleObject};
use models::{
	domain::audio::AudioChapterSource,
	entity::{media_audio, media_audio_chapter, media_audio_track},
	services::audio::AudioBook,
};

/// One file of an audiobook.
#[derive(Debug, Clone, SimpleObject)]
pub struct MediaAudioTrack {
	/// 0-based position in playback order.
	pub index: i32,
	/// The MIME type the track is served with; exact per file, so a
	/// mixed-codec folder book still plays.
	pub mime: String,
	pub duration_ms: i64,
	/// Milliseconds from the start of the publication to the start of this
	/// file. A publication offset belongs to the last track whose
	/// `startOffsetMs` is at or below it.
	pub start_offset_ms: i64,
	pub byte_size: i64,
	/// Where to fetch the bytes:
	/// `/api/v2/media/{mediaId}/audio/track/{index}`.
	pub url: String,
}

impl MediaAudioTrack {
	fn new(track: media_audio_track::Model) -> Self {
		Self {
			url: format!(
				"/api/v2/media/{}/audio/track/{}",
				track.media_id, track.index
			),
			index: track.index,
			mime: track.mime,
			duration_ms: track.duration_ms,
			start_offset_ms: track.start_offset_ms,
			byte_size: track.byte_size,
		}
	}
}

/// One chapter mark of an audiobook.
#[derive(Debug, Clone, SimpleObject)]
pub struct MediaAudioChapter {
	/// 0-based ordinal of the chapter.
	pub index: i32,
	pub title: Option<String>,
	pub start_ms: i64,
	/// The last chapter ends at the publication duration.
	pub end_ms: Option<i64>,
}

impl From<media_audio_chapter::Model> for MediaAudioChapter {
	fn from(chapter: media_audio_chapter::Model) -> Self {
		Self {
			index: chapter.index,
			title: chapter.title,
			start_ms: chapter.start_ms,
			end_ms: chapter.end_ms,
		}
	}
}

/// The audio facts of one audiobook, with its files and chapter marks.
#[derive(Debug, Clone)]
pub struct MediaAudio {
	audio: media_audio::Model,
	tracks: Vec<media_audio_track::Model>,
	chapters: Vec<media_audio_chapter::Model>,
}

impl From<AudioBook> for MediaAudio {
	fn from(book: AudioBook) -> Self {
		Self {
			audio: book.audio,
			tracks: book.tracks,
			chapters: book.chapters,
		}
	}
}

#[Object]
impl MediaAudio {
	/// The whole-publication duration; the sum of the track durations.
	async fn duration_ms(&self) -> i64 {
		self.audio.duration_ms
	}

	/// Short lowercase codec name (`aac`, `mp3`, `opus`, `flac`), or `mixed`
	/// for a folder book whose files do not agree.
	async fn codec(&self) -> &str {
		&self.audio.codec
	}

	async fn sample_rate(&self) -> Option<i32> {
		self.audio.sample_rate
	}

	async fn channels(&self) -> Option<i32> {
		self.audio.channels
	}

	/// Bits per second: the container's stated average where it has one, and
	/// derived from size over duration otherwise.
	async fn bitrate(&self) -> Option<i32> {
		self.audio.bitrate
	}

	/// How the chapter marks were obtained. This is provenance, never a
	/// preference: `PER_TRACK` means Stump synthesized one chapter per file
	/// and the publisher shipped no marks at all, so a client that only wants
	/// to show real chapters can tell the difference.
	async fn chapter_source(&self) -> AudioChapterSource {
		self.audio.chapter_source
	}

	/// The files, in playback order, contiguous from index 0.
	async fn tracks(&self) -> Vec<MediaAudioTrack> {
		self.tracks
			.iter()
			.cloned()
			.map(MediaAudioTrack::new)
			.collect()
	}

	/// The chapter marks, ascending by `startMs`. Empty exactly when
	/// `chapterSource` is `NONE`.
	async fn chapters(&self) -> Vec<MediaAudioChapter> {
		self.chapters
			.iter()
			.cloned()
			.map(MediaAudioChapter::from)
			.collect()
	}

	/// The playback manifest route, for a client that would rather fetch the
	/// whole shape over HTTP than through GraphQL:
	/// `/api/v2/media/{mediaId}/audio/manifest`.
	async fn manifest_url(&self) -> String {
		format!("/api/v2/media/{}/audio/manifest", self.audio.media_id)
	}
}
