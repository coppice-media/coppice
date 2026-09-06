//! Reading and writing the audio facts of an audiobook.
//!
//! Three tables describe one audiobook — `media_audio` (the publication),
//! `media_audio_tracks` (the files) and `media_audio_chapters` (the marks) —
//! and every consumer wants them together: the manifest route, the OPDS
//! acquisition feed, the GraphQL object and the compatibility profiles. This
//! module is the one place that assembles them, so the ordering guarantees
//! (tracks and chapters ascending by `index`) are stated once.
//!
//! [`replace`] is the scanner's write path. It deletes and re-inserts rather
//! than diffing: a re-probe is the authority on a book's track split, and a
//! partial update would leave a stale track claiming an `index` the new split
//! no longer has. The unique `(media_id, index)` indexes make that safe to
//! repeat.

use sea_orm::{prelude::*, ActiveValue::Set, QueryOrder, QuerySelect, TransactionTrait};

use crate::{
	domain::audio::AudioChapterSource,
	entity::{media_audio, media_audio_chapter, media_audio_track},
};

/// One audiobook: the publication facts plus its files and chapter marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioBook {
	pub audio: media_audio::Model,
	/// Ascending by `index`, contiguous from 0 for a book Stump probed.
	pub tracks: Vec<media_audio_track::Model>,
	/// Ascending by `index`. Empty when `audio.chapter_source` is
	/// [`AudioChapterSource::None`].
	pub chapters: Vec<media_audio_chapter::Model>,
}

impl AudioBook {
	/// The track containing a publication-relative offset, or the last track
	/// when the offset is at or past the end. `None` only for a book with no
	/// tracks at all.
	#[must_use]
	pub fn track_at(&self, position_ms: i64) -> Option<&media_audio_track::Model> {
		let index = self
			.tracks
			.partition_point(|track| track.start_offset_ms <= position_ms);
		self.tracks
			.get(index.saturating_sub(1))
			.or(self.tracks.last())
	}

	/// The chapter containing a publication-relative offset. `None` when the
	/// book has no chapters or the offset precedes the first mark.
	#[must_use]
	pub fn chapter_at(&self, position_ms: i64) -> Option<&media_audio_chapter::Model> {
		let index = self
			.chapters
			.partition_point(|chapter| chapter.start_ms <= position_ms);
		(index > 0).then(|| &self.chapters[index - 1])
	}
}

/// The whole-publication duration in milliseconds, or `None` when the media
/// is not an audiobook. One primary-key lookup: this is on the reading-state
/// write path for every book, audio or not.
pub async fn duration_ms<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
) -> Result<Option<i64>, DbErr> {
	media_audio::Entity::find_by_id(media_id.to_string())
		.select_only()
		.column(media_audio::Column::DurationMs)
		.into_tuple::<i64>()
		.one(conn)
		.await
}

/// The full audiobook for one media item, or `None` when it is not an
/// audiobook.
pub async fn book<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
) -> Result<Option<AudioBook>, DbErr> {
	let Some(audio) = media_audio::Entity::find_by_id(media_id.to_string())
		.one(conn)
		.await?
	else {
		return Ok(None);
	};

	let tracks = tracks(conn, media_id).await?;
	let chapters = chapters(conn, media_id).await?;

	Ok(Some(AudioBook {
		audio,
		tracks,
		chapters,
	}))
}

/// The files of an audiobook, ascending by `index`.
pub async fn tracks<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
) -> Result<Vec<media_audio_track::Model>, DbErr> {
	media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(media_id))
		.order_by_asc(media_audio_track::Column::Index)
		.all(conn)
		.await
}

/// One file of an audiobook by its 0-based `index`.
pub async fn track<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
	index: i32,
) -> Result<Option<media_audio_track::Model>, DbErr> {
	media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(media_id))
		.filter(media_audio_track::Column::Index.eq(index))
		.one(conn)
		.await
}

/// The chapter marks of an audiobook, ascending by `index`.
pub async fn chapters<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
) -> Result<Vec<media_audio_chapter::Model>, DbErr> {
	media_audio_chapter::Entity::find()
		.filter(media_audio_chapter::Column::MediaId.eq(media_id))
		.order_by_asc(media_audio_chapter::Column::Index)
		.all(conn)
		.await
}

/// The audio facts a probe produced, in the shape [`replace`] persists. The
/// probe crate converts its own result into this so `models` never depends on
/// the media crate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioFacts {
	pub duration_ms: i64,
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	pub chapter_source: AudioChapterSource,
	pub tracks: Vec<TrackFacts>,
	pub chapters: Vec<ChapterFacts>,
}

/// One file of a probed audiobook. `index` and `start_offset_ms` are assigned
/// by [`replace`] from the position in the vector, so a caller cannot produce
/// a gap or a wrong running sum.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackFacts {
	pub path: String,
	pub duration_ms: i64,
	pub byte_size: i64,
	pub mime: String,
}

/// One probed chapter mark. `index` is assigned by [`replace`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChapterFacts {
	pub title: Option<String>,
	pub start_ms: i64,
	pub end_ms: Option<i64>,
}

/// Persist the probed audio facts of `media_id`, replacing whatever was there.
///
/// Runs in its own transaction so a re-scan is atomic: a reader either sees
/// the previous track split or the new one, never a half-replaced one.
pub async fn replace<C: ConnectionTrait + TransactionTrait>(
	conn: &C,
	media_id: &str,
	facts: &AudioFacts,
) -> Result<(), DbErr> {
	let txn = conn.begin().await?;
	replace_in(&txn, media_id, facts).await?;
	txn.commit().await
}

/// [`replace`] against a caller-owned transaction.
pub async fn replace_in<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
	facts: &AudioFacts,
) -> Result<(), DbErr> {
	media_audio_track::Entity::delete_many()
		.filter(media_audio_track::Column::MediaId.eq(media_id))
		.exec(conn)
		.await?;
	media_audio_chapter::Entity::delete_many()
		.filter(media_audio_chapter::Column::MediaId.eq(media_id))
		.exec(conn)
		.await?;

	let audio = media_audio::ActiveModel {
		media_id: Set(media_id.to_string()),
		duration_ms: Set(facts.duration_ms),
		codec: Set(facts.codec.clone()),
		sample_rate: Set(facts.sample_rate),
		channels: Set(facts.channels),
		bitrate: Set(facts.bitrate),
		chapter_source: Set(facts.chapter_source),
	};
	// `media_audio` is keyed by `media_id`, so a re-scan updates in place.
	media_audio::Entity::insert(audio)
		.on_conflict(
			sea_orm::sea_query::OnConflict::column(media_audio::Column::MediaId)
				.update_columns([
					media_audio::Column::DurationMs,
					media_audio::Column::Codec,
					media_audio::Column::SampleRate,
					media_audio::Column::Channels,
					media_audio::Column::Bitrate,
					media_audio::Column::ChapterSource,
				])
				.to_owned(),
		)
		.exec(conn)
		.await?;

	if !facts.tracks.is_empty() {
		let mut start_offset_ms = 0_i64;
		let rows = facts
			.tracks
			.iter()
			.enumerate()
			.map(|(index, track)| {
				let offset = start_offset_ms;
				start_offset_ms += track.duration_ms;
				// `Entity::insert_many` never runs
				// `ActiveModelBehavior::before_save`, so the id the entity
				// would generate on a single insert has to be set here — the
				// column is `TEXT NOT NULL PRIMARY KEY`.
				media_audio_track::ActiveModel {
					id: Set(uuid::Uuid::new_v4().to_string()),
					media_id: Set(media_id.to_string()),
					index: Set(index as i32),
					path: Set(track.path.clone()),
					duration_ms: Set(track.duration_ms),
					start_offset_ms: Set(offset),
					byte_size: Set(track.byte_size),
					mime: Set(track.mime.clone()),
				}
			})
			.collect::<Vec<_>>();
		media_audio_track::Entity::insert_many(rows)
			.exec(conn)
			.await?;
	}

	if !facts.chapters.is_empty() {
		let rows = facts
			.chapters
			.iter()
			.enumerate()
			.map(|(index, chapter)| media_audio_chapter::ActiveModel {
				id: Set(uuid::Uuid::new_v4().to_string()),
				media_id: Set(media_id.to_string()),
				index: Set(index as i32),
				title: Set(chapter.title.clone()),
				start_ms: Set(chapter.start_ms),
				end_ms: Set(chapter.end_ms),
			})
			.collect::<Vec<_>>();
		media_audio_chapter::Entity::insert_many(rows)
			.exec(conn)
			.await?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fixture(tracks: &[(i64, i64)], chapters: &[i64]) -> AudioBook {
		let mut start_offset_ms = 0;
		let tracks = tracks
			.iter()
			.enumerate()
			.map(|(index, (duration_ms, byte_size))| {
				let offset = start_offset_ms;
				start_offset_ms += duration_ms;
				media_audio_track::Model {
					id: format!("t{index}"),
					media_id: "book".to_string(),
					index: index as i32,
					path: format!("/books/{index}.mp3"),
					duration_ms: *duration_ms,
					start_offset_ms: offset,
					byte_size: *byte_size,
					mime: "audio/mpeg".to_string(),
				}
			})
			.collect::<Vec<_>>();
		let chapters = chapters
			.iter()
			.enumerate()
			.map(|(index, start_ms)| media_audio_chapter::Model {
				id: format!("c{index}"),
				media_id: "book".to_string(),
				index: index as i32,
				title: Some(format!("Chapter {}", index + 1)),
				start_ms: *start_ms,
				end_ms: None,
			})
			.collect::<Vec<_>>();
		AudioBook {
			audio: media_audio::Model {
				media_id: "book".to_string(),
				duration_ms: start_offset_ms,
				codec: "mp3".to_string(),
				sample_rate: Some(44_100),
				channels: Some(2),
				bitrate: Some(64_000),
				chapter_source: AudioChapterSource::Id3Chap,
			},
			tracks,
			chapters,
		}
	}

	/// A byte-range request resolves an offset to exactly one file: the track
	/// whose half-open `[start_offset_ms, start_offset_ms + duration_ms)`
	/// window contains it. The boundary belongs to the *later* track.
	#[test]
	fn track_at_resolves_boundaries_to_the_later_track() {
		let book = fixture(&[(1_000, 10), (2_000, 20), (500, 5)], &[]);

		assert_eq!(book.track_at(0).unwrap().index, 0);
		assert_eq!(book.track_at(999).unwrap().index, 0);
		assert_eq!(book.track_at(1_000).unwrap().index, 1);
		assert_eq!(book.track_at(2_999).unwrap().index, 1);
		assert_eq!(book.track_at(3_000).unwrap().index, 2);
		// Past the end clamps to the last track rather than failing: a client
		// resuming a re-muxed book must still get audio.
		assert_eq!(book.track_at(9_999).unwrap().index, 2);
		assert!(fixture(&[], &[]).track_at(0).is_none());
	}

	/// "Resume at chapter" compares against start marks only, so an offset
	/// before the first mark is in no chapter at all.
	#[test]
	fn chapter_at_needs_a_start_mark_at_or_before_the_offset() {
		let book = fixture(&[(10_000, 100)], &[500, 4_000]);

		assert!(book.chapter_at(0).is_none());
		assert!(book.chapter_at(499).is_none());
		assert_eq!(book.chapter_at(500).unwrap().index, 0);
		assert_eq!(book.chapter_at(3_999).unwrap().index, 0);
		assert_eq!(book.chapter_at(4_000).unwrap().index, 1);
		assert_eq!(book.chapter_at(999_999).unwrap().index, 1);
	}
}
