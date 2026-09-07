//! Edition pairing and the tier-1 chapter map as GraphQL objects.
//!
//! `Media.editions` is the "Also available as" row on a book page:
//! confirmed editions of the same work, cheap to read because a confirmed pair
//! is nothing but two link rows. `Media.editionSuggestions` is the field that
//! *computes* — it runs the pairing heuristics, including the provider edition
//! lists — so a library grid never pays for pairing it did not ask for.
//!
//! `Media.pairedPosition` is where tier 1 earns its keep: the reader's real
//! position in one edition, expressed in the other edition's coordinates,
//! flagged [`MappedPosition::approximate`]. It is a read: nothing here writes
//! a reading head, because a narrator is not a metronome and a derived
//! position must never overwrite one somebody actually reached.

use async_graphql::{SimpleObject, ID};
use models::{
	domain::{
		edition_pair::{PairEvidence, PairStatus},
		reading_state::MappedPosition as DomainMappedPosition,
	},
	entity::media_chapter_map,
	shared::readium::ReadiumLocator,
};

use super::media::Media;

/// An edition of the same work as the book being viewed, with why pairing
/// believes it.
#[derive(Debug, Clone, SimpleObject)]
pub struct EditionSuggestion {
	pub media: Media,
	/// The work both editions link to.
	pub work_id: ID,
	pub status: PairStatus,
	/// `null` only for a link the native liseur lane created before pairing
	/// existed; every suggestion this server makes records its evidence.
	pub evidence: Option<PairEvidence>,
}

/// A reading position derived from the *other* edition of the same work.
#[derive(Debug, Clone, SimpleObject)]
pub struct MappedPosition {
	/// The edition the position was read from.
	pub source_media_id: ID,
	/// Set when this edition is the ebook: a locator inside the mapped spine
	/// item.
	pub locator: Option<ReadiumLocator>,
	/// Set when this edition is the audiobook: milliseconds from the start of
	/// the publication.
	pub position_ms: Option<i64>,
	/// Whole-publication progression in *this* edition.
	pub progression: f64,
	/// The confidence of the chapter-map entry the conversion went through.
	pub confidence: f64,
	/// Always `true` for a chapter-map conversion: it is worth ±1-3 minutes
	/// on a 30-minute chapter. Clients must label it, and must not write it
	/// back as a real position.
	pub approximate: bool,
}

impl MappedPosition {
	pub fn new(source_media_id: &str, mapped: DomainMappedPosition) -> Self {
		Self {
			source_media_id: ID::from(source_media_id.to_owned()),
			locator: mapped.locator,
			position_ms: mapped.position_ms,
			progression: mapped.progression,
			confidence: mapped.confidence,
			approximate: mapped.approximate,
		}
	}
}

/// One entry of a pair's chapter map: this spine item is that audio chapter.
#[derive(Debug, Clone, SimpleObject)]
pub struct ChapterMapEntry {
	pub ebook_media_id: ID,
	pub audio_media_id: ID,
	/// 0-based index into the ebook's linear spine.
	pub ebook_spine_index: i32,
	/// 0-based `mediaAudioChapter.index`.
	pub audio_chapter_index: i32,
	/// `1.0` for a normalised-title match, `0.85` for an ordinal match, and
	/// lower for a positional fallback. The console reviews the low ones.
	pub confidence: f64,
}

impl From<media_chapter_map::Model> for ChapterMapEntry {
	fn from(model: media_chapter_map::Model) -> Self {
		Self {
			ebook_media_id: ID::from(model.ebook_media_id),
			audio_media_id: ID::from(model.audio_media_id),
			ebook_spine_index: model.ebook_spine_index,
			audio_chapter_index: model.audio_chapter_index,
			confidence: model.confidence,
		}
	}
}

/// What a pairing mutation did. `changed == false` with a `status` is a
/// no-op the client should not treat as an error: re-confirming a confirmed
/// pair, or re-suggesting a rejected one.
#[derive(Debug, Clone, SimpleObject)]
pub struct EditionPairResult {
	/// `null` when nothing was written and there was no pair to write about.
	pub work_id: Option<ID>,
	pub status: Option<PairStatus>,
	pub changed: bool,
	/// Human-readable reason a write was a no-op.
	pub message: Option<String>,
	/// Entries written into the pair's chapter map, when the mutation built
	/// one.
	pub chapter_map_entries: i32,
}
