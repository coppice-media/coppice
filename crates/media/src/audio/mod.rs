//! Probing an audiobook: duration, codec, chapter marks, cover and track
//! order.
//!
//! An audiobook reaches Stump in one of two shapes, and both have to become
//! the same thing:
//!
//! * **One container** — a single `.m4b`/`.mp3`/`.opus` file whose chapters
//!   are embedded in the container (a Nero `chpl` atom, a QuickTime text
//!   chapter track, ID3v2 `CHAP` frames, `CHAPTERxxx` Vorbis comments).
//! * **One folder** — one file per part, where the *file list* is the chapter
//!   list unless the individual files carry their own marks.
//!
//! Both produce a [`ProbedAudio`] whose every time value is milliseconds from
//! the start of the **publication**, never from the start of a file. That is
//! the unit a `reading_heads.position_ms` is expressed in, so a resume never
//! needs a per-track conversion.
//!
//! # Why two libraries
//!
//! [`symphonia`] demuxes every container Stump accepts and is the single
//! source of duration, codec, sample rate, channel count, tags and cover art.
//! It reads chapters from ID3v2 and from Vorbis comments — but **not** from
//! MP4: `symphonia-format-isomp4` 0.6 has no `chpl` or chapter-track reader.
//! So MP4 chapters come from [`mp4ameta`], which is also the only library that
//! distinguishes the two MP4 mechanisms, and that distinction is exactly the
//! provenance [`ChapterSource`] records.

mod probe;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use models::services::audio::{AudioFacts, ChapterFacts, TrackFacts};

pub use models::domain::audio::AudioChapterSource as ChapterSource;
pub use probe::{audio_files_in, probe, probe_file, probe_folder};

use crate::content_type::ContentType;

/// One file of a probed audiobook.
///
/// A single-container book has exactly one of these; a folder book has one per
/// part, in playback order. `start_offset_ms` is deliberately absent: it is
/// the running sum of the preceding durations and is assigned once, on the
/// write path, so a caller cannot produce an inconsistent one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbedTrack {
	pub path: PathBuf,
	pub duration_ms: i64,
	pub byte_size: i64,
	/// The MIME type the track is served with, from its container.
	pub mime: String,
	/// Short lowercase codec name (`aac`, `mp3`, `opus`, `flac`, `vorbis`).
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	/// The track's own title tag, used to name a synthesized chapter.
	pub title: Option<String>,
	/// The `TRCK`/`trkn` tag. A folder book is ordered by this when every
	/// file has one, and by filename otherwise.
	pub track_number: Option<u32>,
}

/// One chapter mark of a probed audiobook, relative to the publication.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProbedChapter {
	pub title: Option<String>,
	pub start_ms: i64,
	/// `None` for the last chapter and for every chapter of a container that
	/// only carries start marks.
	pub end_ms: Option<i64>,
}

/// Everything a scan needs to know about one audio publication.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProbedAudio {
	/// Sum of the track durations, in milliseconds.
	pub duration_ms: i64,
	/// The publication codec, or `mixed` for a folder book whose files do not
	/// agree.
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	/// How [`Self::chapters`] was obtained. Provenance, never a preference.
	pub chapter_source: ChapterSource,
	/// Playback order, always contiguous.
	pub tracks: Vec<ProbedTrack>,
	/// Ascending by `start_ms`. Empty exactly when `chapter_source` is
	/// [`ChapterSource::None`].
	pub chapters: Vec<ProbedChapter>,
	/// The embedded cover art, if the container carries one.
	pub cover: Option<(ContentType, Vec<u8>)>,
	pub title: Option<String>,
	pub author: Option<String>,
	/// The reader. Publishers write this to the `composer`/`narrator` tag;
	/// Audiobookshelf and Plex both read it from there.
	pub narrator: Option<String>,
	pub album: Option<String>,
	pub description: Option<String>,
	pub genre: Option<String>,
	pub year: Option<i32>,
}

impl ProbedAudio {
	/// The chapter containing a publication-relative offset. `None` when the
	/// book has no chapters or the offset precedes the first mark.
	#[must_use]
	pub fn chapter_at(&self, position_ms: i64) -> Option<&ProbedChapter> {
		let index = self
			.chapters
			.partition_point(|chapter| chapter.start_ms <= position_ms);
		(index > 0).then(|| &self.chapters[index - 1])
	}
}

impl From<&ProbedAudio> for AudioFacts {
	fn from(probed: &ProbedAudio) -> Self {
		AudioFacts {
			duration_ms: probed.duration_ms,
			codec: probed.codec.clone(),
			sample_rate: probed.sample_rate,
			channels: probed.channels,
			bitrate: probed.bitrate,
			chapter_source: probed.chapter_source,
			tracks: probed
				.tracks
				.iter()
				.map(|track| TrackFacts {
					path: track.path.to_string_lossy().to_string(),
					duration_ms: track.duration_ms,
					byte_size: track.byte_size,
					mime: track.mime.clone(),
				})
				.collect(),
			chapters: probed
				.chapters
				.iter()
				.map(|chapter| ChapterFacts {
					title: chapter.title.clone(),
					start_ms: chapter.start_ms,
					end_ms: chapter.end_ms,
				})
				.collect(),
		}
	}
}
