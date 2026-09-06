//! Audio-publication domain types shared by the probe, the schema and the
//! wire profiles.
//!
//! The only value here is the *provenance* of an audiobook's chapter list.
//! It lives in the domain layer rather than next to the probe because three
//! independent consumers need the same vocabulary: `stump_media::audio`
//! re-exports it as `ChapterSource` when it probes a file, the
//! `media_audio.chapter_source` column stores it, and the compatibility
//! profiles report it so a client can tell a publisher-authored chapter list
//! from one Stump synthesized.

use sea_orm::{prelude::*, DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// How an audiobook's chapter marks were obtained.
///
/// This is provenance, never a preference: a list synthesized one-chapter-
/// per-file ([`Self::PerTrack`]) must not be presented as if the publisher
/// shipped it, and a client that wants to hide synthetic chapters can only do
/// that if the mechanism survives the probe. The variants name the concrete
/// container mechanism rather than a quality tier so a new container format
/// adds a variant instead of silently widening an existing one.
#[derive(
	Eq,
	Copy,
	Hash,
	Debug,
	Clone,
	EnumIter,
	PartialEq,
	Serialize,
	Deserialize,
	DeriveActiveEnum,
	EnumString,
	Display,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[sea_orm(
	rs_type = "String",
	rename_all = "snake_case",
	db_type = "String(StringLen::None)"
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AudioChapterSource {
	/// The Nero `chpl` atom in `moov/udta` of an MP4/M4B container.
	Mp4Chpl,
	/// A QuickTime text chapter track, linked from the audio track by a
	/// `tref`/`chap` reference.
	Mp4ChapterTrack,
	/// ID3v2 `CHAP`/`CTOC` frames.
	Id3Chap,
	/// `CHAPTERxxx`/`CHAPTERxxxNAME` Vorbis comments (Ogg, Opus, FLAC).
	VorbisComment,
	/// Synthesized: one chapter per file of a folder audiobook. The publisher
	/// shipped no chapter marks at all.
	PerTrack,
	/// The publication has no chapters.
	None,
}
