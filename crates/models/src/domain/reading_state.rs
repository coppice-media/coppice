//! Protocol-neutral projection of a reading position onto the unified
//! `reading_heads` row.
//!
//! Every wire protocol converts its native payload into a [`ProtocolUpdate`];
//! [`project`] turns that update into the head fields and [`resolve`] decides
//! whether the update moves the head or is only recorded as provenance. Both
//! functions are pure so the per-protocol projection rules and the conflict
//! rule can be tested without a database.

use chrono::{DateTime, Duration, Utc};
use sea_orm::{prelude::*, DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

use crate::{
	entity::media,
	shared::readium::{ReadiumLocation, ReadiumLocator},
};

/// A later update whose device timestamp trails the head by more than this
/// many seconds (and whose progression is lower) is stale provenance, not a
/// regression the reader asked for. The window absorbs device clock skew.
pub const STALE_TOLERANCE_SECS: i64 = 60;

/// The wire protocol that produced a reading-state update.
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
#[sea_orm(
	rs_type = "String",
	rename_all = "snake_case",
	db_type = "String(StringLen::None)"
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SourceProtocol {
	/// Stump's own GraphQL API (browser and mobile apps)
	Stump,
	/// The Komga compatibility profile (Komelia, Mihon, Grimmory)
	Komga,
	/// The Kobo `ReadingState` API
	Kobo,
	/// The KOReader progress sync API
	Koreader,
	/// OPDS 1.2 page streaming and the OPDS 2.0 progression resource
	Opds,
	/// The native liseur-sync operation log
	Liseur,
	/// The Kavita compatibility profile
	Kavita,
	/// The Audiobookshelf compatibility profile (Lissen)
	Abs,
}

/// Whether the effective time of an update came from the device or was
/// assigned by the server at ingestion.
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
#[sea_orm(
	rs_type = "String",
	rename_all = "snake_case",
	db_type = "String(StringLen::None)"
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TimestampKind {
	Device,
	Server,
}

/// How an update locates the reader within the publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Position {
	/// A 1-based page of a page-addressed publication.
	Page(i32),
	/// A Readium locator carried verbatim by a locator-based protocol.
	Locator(ReadiumLocator),
	/// A moment in a recording: milliseconds from the start of the
	/// *publication*, plus the 0-based track the client was in for a
	/// multi-file audiobook. This is never a page ordinal — an audiobook has
	/// no pages, and rounding a millisecond offset into `progression` alone
	/// loses the only thing a listener needs to resume.
	Time {
		position_ms: i64,
		track_index: Option<i32>,
	},
	/// No projectable position: the update only carries progression and/or
	/// completion (a KOReader x-pointer, a Kobo status-only state, ...). The
	/// native position stays in the raw payload.
	None,
}

/// A reading-state update expressed in protocol-neutral terms.
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolUpdate {
	pub protocol: SourceProtocol,
	pub device_id: Option<String>,
	/// The effective source (device) time. `None` stamps the server's
	/// ingestion time and records `TimestampKind::Server`.
	pub updated_at: Option<DateTime<Utc>>,
	pub position: Position,
	/// Whole-publication progression asserted by the source (`0..=1`). When
	/// absent it is derived from the position where possible.
	pub progression: Option<f64>,
	/// `Some(true)` marks the publication finished, `Some(false)` is an
	/// explicit un-read, and `None` leaves the (sticky) completion untouched.
	pub completed: Option<bool>,
	/// The complete native request, retained as provenance.
	pub raw_payload: serde_json::Value,
}

/// The publication context a projection needs.
#[derive(Clone, Copy, Debug)]
pub struct Publication<'a> {
	pub media_id: &'a str,
	/// The page count, `<= 0` when the publication is not page-addressed.
	pub pages: i32,
	/// The whole-publication duration in milliseconds, `Some` only for an
	/// audio publication. It is what a [`Position::Time`] progression is
	/// expressed against.
	pub duration_ms: Option<i64>,
}

impl<'a> Publication<'a> {
	/// Attach the audio duration of the publication so a
	/// [`Position::Time`] update can derive progression. Callers that hold a
	/// `media_audio` row use this on top of the `media`-derived context.
	#[must_use]
	pub fn with_duration_ms(mut self, duration_ms: i64) -> Self {
		self.duration_ms = Some(duration_ms);
		self
	}
}

impl<'a> From<&'a media::Model> for Publication<'a> {
	fn from(media: &'a media::Model) -> Self {
		Publication {
			media_id: &media.id,
			pages: media.pages,
			duration_ms: None,
		}
	}
}

/// The head fields an update asserts. `None` keeps the current head value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projection {
	pub locator: Option<ReadiumLocator>,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	pub track_index: Option<i32>,
	pub progression: Option<f64>,
	pub completed: Option<bool>,
}

/// The current head values that [`resolve`] compares an update against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadState {
	pub updated_at: DateTime<Utc>,
	pub progression: f64,
	pub completed: bool,
}

/// The outcome of applying an update to a head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
	/// The update moved the head.
	Accepted,
	/// The update was stale and is retained as provenance only.
	ProvenanceOnly,
}

/// The resolved head values after an accepted update.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
	pub outcome: Outcome,
	pub progression: f64,
	pub completed: bool,
}

/// Whole-publication progression of a 1-based page, clamped to `0..=1`.
pub fn page_progression(page: i32, pages: i32) -> Option<f64> {
	(pages > 0).then(|| (f64::from(page) / f64::from(pages)).clamp(0.0, 1.0))
}

/// Whole-publication progression of a millisecond offset, clamped to `0..=1`.
pub fn time_progression(position_ms: i64, duration_ms: i64) -> Option<f64> {
	(duration_ms > 0)
		.then(|| ((position_ms as f64) / (duration_ms as f64)).clamp(0.0, 1.0))
}

/// The locator synthesized for a page-based position. The `href` is Stump's
/// own page route, so it is a provider-derived anchor rather than a resource
/// of the publication; readers that need a protocol-specific page URL derive
/// it from `locations.position`.
pub fn page_locator(media_id: &str, page: i32, pages: i32) -> ReadiumLocator {
	let progression =
		page_progression(page, pages).and_then(|value| Decimal::try_from(value).ok());
	ReadiumLocator {
		chapter_title: String::new(),
		href: format!("/api/v2/media/{media_id}/page/{page}"),
		title: Some(format!("Page {page}")),
		locations: Some(ReadiumLocation {
			fragments: None,
			progression,
			position: Some(page),
			total_progression: progression,
			css_selector: None,
			partial_cfi: None,
		}),
		text: None,
		kobo_span: None,
		r#type: "image/jpeg".to_string(),
	}
}

fn clamp_unit(value: f64) -> Option<f64> {
	value.is_finite().then(|| value.clamp(0.0, 1.0))
}

fn decimal_to_f64(value: Decimal) -> Option<f64> {
	value.to_string().parse::<f64>().ok().and_then(clamp_unit)
}

/// Project an update onto head fields.
///
/// - A page-based position derives progression from `page / pages` and
///   synthesizes a page locator.
/// - A locator-based position is stored verbatim; progression comes from the
///   asserted value, then `locations.total_progression`, then
///   `locations.position / pages`.
/// - A time-based position is stored verbatim; progression comes from the
///   asserted value, then `position_ms / duration_ms`. It never synthesizes
///   a page or a locator: a recording has neither.
/// - An update without a position keeps the head's locator and page.
/// - `completed == Some(true)` without a position or progression lands on the
///   last page at `1.0`.
pub fn project(update: &ProtocolUpdate, publication: &Publication<'_>) -> Projection {
	let asserted = update.progression.and_then(clamp_unit);
	let mut projection = match &update.position {
		Position::Page(page) => Projection {
			locator: Some(page_locator(publication.media_id, *page, publication.pages)),
			page: Some(*page),
			progression: asserted.or_else(|| page_progression(*page, publication.pages)),
			completed: update.completed,
			..Projection::default()
		},
		Position::Locator(locator) => {
			let locations = locator.locations.as_ref();
			let position = locations.and_then(|locations| locations.position);
			let progression = asserted
				.or_else(|| {
					locations
						.and_then(|locations| locations.total_progression)
						.and_then(decimal_to_f64)
				})
				.or_else(|| {
					position.and_then(|page| page_progression(page, publication.pages))
				});
			Projection {
				locator: Some(locator.clone()),
				page: position,
				progression,
				completed: update.completed,
				..Projection::default()
			}
		},
		Position::Time {
			position_ms,
			track_index,
		} => Projection {
			position_ms: Some(*position_ms),
			track_index: *track_index,
			progression: asserted.or_else(|| {
				publication
					.duration_ms
					.and_then(|duration| time_progression(*position_ms, duration))
			}),
			completed: update.completed,
			..Projection::default()
		},
		Position::None => Projection {
			progression: asserted,
			completed: update.completed,
			..Projection::default()
		},
	};

	if update.completed == Some(true)
		&& projection.page.is_none()
		&& projection.position_ms.is_none()
		&& projection.progression.is_none()
	{
		match publication.duration_ms {
			// A finished audiobook lands on its final millisecond; it has no
			// last page to land on.
			Some(duration) if duration > 0 => projection.position_ms = Some(duration),
			_ => projection.page = (publication.pages > 0).then_some(publication.pages),
		}
		projection.progression = Some(1.0);
	}

	projection
}

/// Apply the conflict rule: the newest update wins unless it is older than the
/// head by more than [`STALE_TOLERANCE`] *and* reports lower progression, in
/// which case it is provenance only. Completion is sticky: only an explicit
/// un-read (`completed == Some(false)`) clears it.
pub fn resolve(
	head: Option<HeadState>,
	projection: &Projection,
	incoming_at: DateTime<Utc>,
) -> Resolved {
	let Some(head) = head else {
		let completed = projection.completed.unwrap_or(false);
		return Resolved {
			outcome: Outcome::Accepted,
			progression: projection.progression.unwrap_or(if completed {
				1.0
			} else {
				0.0
			}),
			completed,
		};
	};

	let progression = projection.progression.unwrap_or(head.progression);
	let older_by = head.updated_at - incoming_at;
	if older_by > Duration::seconds(STALE_TOLERANCE_SECS)
		&& progression < head.progression
	{
		return Resolved {
			outcome: Outcome::ProvenanceOnly,
			progression: head.progression,
			completed: head.completed,
		};
	}

	Resolved {
		outcome: Outcome::Accepted,
		progression,
		completed: projection.completed.unwrap_or(head.completed),
	}
}

/// One linear spine item of an ebook edition.
///
/// Mirrors what the Readium positions generator already enumerates
/// (`stump_media::media::readium::SpinePositionMeta`), reduced to the three
/// facts a position conversion needs. It is repeated here rather than
/// imported because `models` sits below `stump_media`, and because the
/// conversion has to stay a pure function testable without an EPUB on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpineItem {
	/// 0-based index into the linear spine.
	pub index: i32,
	/// The package-relative path of the item, which is what a locator's
	/// `href` ends with.
	pub href: String,
	/// The item's content length — `SpinePositionMeta::size`, the weight
	/// `positions.json` divides the book by. For XHTML it tracks character
	/// count closely enough that a chapter fraction lands in the right
	/// paragraph, which is all tier 1 promises.
	pub char_count: i64,
}

/// One chapter mark of an audio edition with its end resolved.
///
/// `media_audio_chapters.end_ms` is nullable — most containers only carry
/// start marks — so the caller resolves the last chapter's end against
/// `media_audio.duration_ms` before converting. A span with a non-positive
/// length carries no position information and is skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioChapterSpan {
	/// 0-based `media_audio_chapters.index`.
	pub index: i32,
	pub start_ms: i64,
	pub end_ms: i64,
}

impl AudioChapterSpan {
	fn duration_ms(&self) -> i64 {
		self.end_ms - self.start_ms
	}
}

/// One `media_chapter_map` entry: spine item ↔ audio chapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChapterMapping {
	pub ebook_spine_index: i32,
	pub audio_chapter_index: i32,
	pub confidence: f64,
}

/// A position derived from the *other* edition of the same work.
///
/// It is never written to a reading head. Narrators are not metronomes, so a
/// chapter-fraction conversion is worth ±1-3 minutes on a 30-minute chapter:
/// good enough to resume near, and nothing that should overwrite a position
/// the reader actually reached. `approximate` is a field rather than a
/// constant because tier 2 (forced alignment) produces the same shape from
/// real cues, where it is `false`.
#[derive(Clone, Debug, PartialEq)]
pub struct MappedPosition {
	/// Set when the target edition is the ebook.
	pub locator: Option<ReadiumLocator>,
	/// Set when the target edition is the audiobook.
	pub position_ms: Option<i64>,
	/// Whole-publication progression in the *target* edition.
	pub progression: f64,
	/// The confidence of the chapter-map entry the conversion went through.
	pub confidence: f64,
	/// Always `true` for a chapter-map conversion.
	pub approximate: bool,
}

/// The chapter containing `position_ms`: the last span that starts at or
/// before it. A position past the end of the recording belongs to the final
/// chapter, which is what a client sending `duration_ms` means.
fn chapter_at(
	chapters: &[AudioChapterSpan],
	position_ms: i64,
) -> Option<AudioChapterSpan> {
	chapters
		.iter()
		.filter(|chapter| chapter.duration_ms() > 0 && chapter.start_ms <= position_ms)
		.max_by_key(|chapter| chapter.start_ms)
		.copied()
}

fn mapped_spine_index(
	mappings: &[ChapterMapping],
	audio_chapter_index: i32,
) -> Option<&ChapterMapping> {
	mappings
		.iter()
		.find(|entry| entry.audio_chapter_index == audio_chapter_index)
}

fn mapped_chapter_index(
	mappings: &[ChapterMapping],
	ebook_spine_index: i32,
) -> Option<&ChapterMapping> {
	mappings
		.iter()
		.find(|entry| entry.ebook_spine_index == ebook_spine_index)
}

/// Characters before `index` and the whole book's characters.
fn spine_offsets(spine: &[SpineItem], index: i32) -> Option<(i64, i64)> {
	let total: i64 = spine.iter().map(|item| item.char_count.max(0)).sum();
	let before: i64 = spine
		.iter()
		.take_while(|item| item.index != index)
		.map(|item| item.char_count.max(0))
		.sum();
	spine
		.iter()
		.any(|item| item.index == index)
		.then_some((before, total.max(1)))
}

/// Audio → ebook: `f` of the containing chapter becomes `f` of the mapped
/// spine item's characters.
///
/// `None` when the chapter has no counterpart — front and back matter
/// (opening credits, an end-of-book advert) is deliberately unmapped, and
/// guessing a spine item for it would land the reader in the copyright page.
#[must_use]
pub fn map_time_to_locator(
	position_ms: i64,
	chapters: &[AudioChapterSpan],
	spine: &[SpineItem],
	mappings: &[ChapterMapping],
) -> Option<MappedPosition> {
	let chapter = chapter_at(chapters, position_ms)?;
	let mapping = mapped_spine_index(mappings, chapter.index)?;
	let item = spine
		.iter()
		.find(|item| item.index == mapping.ebook_spine_index)?;
	let (chars_before, total_chars) = spine_offsets(spine, item.index)?;

	let within = clamp_unit(
		(position_ms - chapter.start_ms) as f64 / chapter.duration_ms() as f64,
	)?;
	let char_offset = chars_before as f64 + within * item.char_count.max(0) as f64;
	let total_progression = clamp_unit(char_offset / total_chars as f64)?;

	Some(MappedPosition {
		locator: Some(ReadiumLocator {
			chapter_title: String::new(),
			href: item.href.clone(),
			title: None,
			locations: Some(ReadiumLocation {
				fragments: None,
				progression: Decimal::try_from(within).ok(),
				// A character offset is not a `positions.json` ordinal, and
				// publishing it as one would make readers jump to page 4812.
				position: None,
				total_progression: Decimal::try_from(total_progression).ok(),
				css_selector: None,
				partial_cfi: None,
			}),
			text: None,
			kobo_span: None,
			r#type: "application/xhtml+xml".to_string(),
		}),
		position_ms: None,
		progression: total_progression,
		confidence: mapping.confidence,
		approximate: true,
	})
}

/// Which spine item a locator is inside, and how far through it.
///
/// The `href` is matched by suffix in either direction, because a locator
/// minted by a reader carries a resource URL
/// (`/api/v2/media/{id}/resource/OEBPS/ch2.xhtml`) while the spine carries the
/// package-relative path. Without an `href` hit the item is derived from
/// `locations.total_progression` over the cumulative character counts, which
/// is how a Kobo or KOReader state — progression and nothing else — still
/// converts.
fn locate_in_spine(
	locator: &ReadiumLocator,
	spine: &[SpineItem],
) -> Option<(SpineItem, f64)> {
	let locations = locator.locations.as_ref();
	let href = locator.href.trim_end_matches('/');
	let by_href = (!href.is_empty())
		.then(|| {
			spine.iter().find(|item| {
				!item.href.is_empty()
					&& (href.ends_with(item.href.as_str()) || item.href.ends_with(href))
			})
		})
		.flatten();

	if let Some(item) = by_href {
		let within = locations
			.and_then(|locations| locations.progression)
			.and_then(decimal_to_f64)
			.unwrap_or(0.0);
		return Some((item.clone(), within));
	}

	let total_progression = locations
		.and_then(|locations| locations.total_progression)
		.and_then(decimal_to_f64)?;
	let total: i64 = spine.iter().map(|item| item.char_count.max(0)).sum();
	let target = total_progression * total.max(1) as f64;
	let mut cumulative = 0i64;
	for item in spine {
		let chars = item.char_count.max(0);
		if target < (cumulative + chars) as f64 || item.index == spine.last()?.index {
			let within = if chars > 0 {
				clamp_unit((target - cumulative as f64) / chars as f64)?
			} else {
				0.0
			};
			return Some((item.clone(), within));
		}
		cumulative += chars;
	}
	None
}

/// Ebook → audio: the inverse. The fraction through the spine item becomes the
/// same fraction through the mapped chapter's runtime.
///
/// `None` when the spine item has no counterpart, which is what keeps a
/// dedication or a copyright page from resolving to second 0 of chapter 1.
#[must_use]
pub fn map_locator_to_time(
	locator: &ReadiumLocator,
	spine: &[SpineItem],
	chapters: &[AudioChapterSpan],
	mappings: &[ChapterMapping],
) -> Option<MappedPosition> {
	let (item, within) = locate_in_spine(locator, spine)?;
	let mapping = mapped_chapter_index(mappings, item.index)?;
	let chapter = chapters
		.iter()
		.find(|chapter| chapter.index == mapping.audio_chapter_index)
		.filter(|chapter| chapter.duration_ms() > 0)?;

	let within = clamp_unit(within)?;
	let position_ms =
		chapter.start_ms + (within * chapter.duration_ms() as f64).round() as i64;
	let duration_ms = chapters
		.iter()
		.map(|chapter| chapter.end_ms)
		.max()
		.unwrap_or_default();

	Some(MappedPosition {
		locator: None,
		position_ms: Some(position_ms),
		progression: time_progression(position_ms, duration_ms).unwrap_or(0.0),
		confidence: mapping.confidence,
		approximate: true,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn audiobook(duration_ms: i64) -> Publication<'static> {
		Publication {
			media_id: "book",
			// An audiobook has no pages; a projection that leaned on `pages`
			// would silently produce a page ordinal for a recording.
			pages: 0,
			duration_ms: Some(duration_ms),
		}
	}

	fn update(position: Position) -> ProtocolUpdate {
		ProtocolUpdate {
			protocol: SourceProtocol::Stump,
			device_id: None,
			updated_at: None,
			position,
			progression: None,
			completed: None,
			raw_payload: serde_json::Value::Null,
		}
	}

	/// A listening position is stored verbatim and its progression comes from
	/// the publication duration. It must never synthesize a page or a
	/// locator: a recording has neither, and a fabricated page would be
	/// served to page-addressed clients as if it meant something.
	#[test]
	fn reading_state_projects_a_time_position_without_a_page() {
		let projection = project(
			&update(Position::Time {
				position_ms: 1_500,
				track_index: Some(2),
			}),
			&audiobook(6_000),
		);

		assert_eq!(projection.position_ms, Some(1_500));
		assert_eq!(projection.track_index, Some(2));
		assert_eq!(projection.progression, Some(0.25));
		assert_eq!(projection.page, None);
		assert_eq!(projection.locator, None);
	}

	/// Without a known duration there is nothing to divide by, so the
	/// position is still recorded but progression stays unknown rather than
	/// defaulting to 0 and reporting the listener back at the start.
	#[test]
	fn reading_state_keeps_a_time_position_when_the_duration_is_unknown() {
		let publication = Publication {
			media_id: "book",
			pages: 0,
			duration_ms: None,
		};

		let projection = project(
			&update(Position::Time {
				position_ms: 1_500,
				track_index: None,
			}),
			&publication,
		);

		assert_eq!(projection.position_ms, Some(1_500));
		assert_eq!(projection.progression, None);
	}

	/// An asserted progression wins over the derived one: a protocol that
	/// states where it thinks it is knows something the duration does not,
	/// such as a book whose parts were re-muxed since the last scan.
	#[test]
	fn reading_state_prefers_an_asserted_progression_over_the_derived_one() {
		let mut incoming = update(Position::Time {
			position_ms: 1_500,
			track_index: None,
		});
		incoming.progression = Some(0.9);

		let projection = project(&incoming, &audiobook(6_000));

		assert_eq!(projection.progression, Some(0.9));
		assert_eq!(projection.position_ms, Some(1_500));
	}

	/// A finished audiobook lands on its final millisecond. Falling back to
	/// the last *page* would leave the head with no position at all, so a
	/// device resuming a completed book would start it over.
	#[test]
	fn reading_state_completes_an_audiobook_at_its_duration() {
		let mut incoming = update(Position::None);
		incoming.completed = Some(true);

		let projection = project(&incoming, &audiobook(6_000));

		assert_eq!(projection.position_ms, Some(6_000));
		assert_eq!(projection.progression, Some(1.0));
		assert_eq!(projection.page, None);

		// A paged publication keeps the existing last-page behaviour.
		let paged = Publication {
			media_id: "book",
			pages: 42,
			duration_ms: None,
		};
		let projection = project(&incoming, &paged);
		assert_eq!(projection.page, Some(42));
		assert_eq!(projection.position_ms, None);
	}

	/// Progression is a ratio clamped to `0..=1`: a container that reports a
	/// mark past its own duration must not produce a head above 1.0, and a
	/// zero-duration row must not divide by zero.
	#[test]
	fn reading_state_time_progression_is_clamped() {
		assert_eq!(time_progression(0, 6_000), Some(0.0));
		assert_eq!(time_progression(6_000, 6_000), Some(1.0));
		assert_eq!(time_progression(99_999, 6_000), Some(1.0));
		assert_eq!(time_progression(-1, 6_000), Some(0.0));
		assert_eq!(time_progression(1_000, 0), None);
	}

	/// A three-chapter audiobook of deliberately unequal chapters, so a
	/// conversion that mistook "chapter 2 of 3" for "two thirds of the book"
	/// would be visibly wrong.
	fn fixture_chapters() -> Vec<AudioChapterSpan> {
		vec![
			// Opening credits: real, unmapped front matter.
			AudioChapterSpan {
				index: 0,
				start_ms: 0,
				end_ms: 10_000,
			},
			AudioChapterSpan {
				index: 1,
				start_ms: 10_000,
				end_ms: 30_000,
			},
			AudioChapterSpan {
				index: 2,
				start_ms: 30_000,
				end_ms: 90_000,
			},
			AudioChapterSpan {
				index: 3,
				start_ms: 90_000,
				end_ms: 120_000,
			},
		]
	}

	/// Spine item 0 is the copyright page: front matter with no counterpart.
	fn fixture_spine() -> Vec<SpineItem> {
		vec![
			SpineItem {
				index: 0,
				href: "OEBPS/copyright.xhtml".into(),
				char_count: 400,
			},
			SpineItem {
				index: 1,
				href: "OEBPS/ch1.xhtml".into(),
				char_count: 1_000,
			},
			SpineItem {
				index: 2,
				href: "OEBPS/ch2.xhtml".into(),
				char_count: 3_000,
			},
			SpineItem {
				index: 3,
				href: "OEBPS/ch3.xhtml".into(),
				char_count: 1_600,
			},
		]
	}

	fn fixture_map() -> Vec<ChapterMapping> {
		vec![
			ChapterMapping {
				ebook_spine_index: 1,
				audio_chapter_index: 1,
				confidence: 1.0,
			},
			ChapterMapping {
				ebook_spine_index: 2,
				audio_chapter_index: 2,
				confidence: 0.6,
			},
			ChapterMapping {
				ebook_spine_index: 3,
				audio_chapter_index: 3,
				confidence: 1.0,
			},
		]
	}

	fn progression_of(locator: &ReadiumLocator) -> (f64, f64) {
		let locations = locator.locations.as_ref().expect("locations");
		(
			decimal_to_f64(locations.progression.expect("progression")).expect("finite"),
			decimal_to_f64(locations.total_progression.expect("total")).expect("finite"),
		)
	}

	/// Audio → ebook lands at the same fraction of the *mapped* spine item,
	/// not at the same fraction of the book: chapter 2 runs 30 s-90 s of a
	/// 120 s recording but its spine item is 3,000 of 6,000 characters.
	#[test]
	fn reading_state_maps_an_audio_position_into_the_mapped_spine_item() {
		let mapped = map_time_to_locator(
			45_000,
			&fixture_chapters(),
			&fixture_spine(),
			&fixture_map(),
		)
		.expect("chapter 2 is mapped");

		let locator = mapped.locator.as_ref().expect("locator");
		assert_eq!(locator.href, "OEBPS/ch2.xhtml");
		let (within, total) = progression_of(locator);
		// 15 s into a 60 s chapter.
		assert!((within - 0.25).abs() < 1e-9, "within = {within}");
		// 400 + 1000 characters before it, plus a quarter of its 3,000, over
		// the book's 6,000.
		assert!((total - 0.358_333_333).abs() < 1e-6, "total = {total}");
		assert!((mapped.progression - total).abs() < 1e-9);
		// A character offset is not a positions.json ordinal.
		assert_eq!(locator.locations.as_ref().and_then(|l| l.position), None);
		assert!(mapped.approximate);
		assert_eq!(mapped.confidence, 0.6);
		assert_eq!(mapped.position_ms, None);
	}

	/// Ebook → audio is the inverse: a quarter into the spine item is a
	/// quarter into the chapter's runtime, and the progression is measured
	/// against the recording, not the text.
	#[test]
	fn reading_state_maps_a_locator_into_the_mapped_chapter() {
		let locator = ReadiumLocator {
			chapter_title: String::new(),
			href: "/api/v2/media/book/resource/OEBPS/ch2.xhtml".into(),
			title: None,
			locations: Some(ReadiumLocation {
				fragments: None,
				progression: Decimal::try_from(0.25).ok(),
				position: None,
				total_progression: None,
				css_selector: None,
				partial_cfi: None,
			}),
			text: None,
			kobo_span: None,
			r#type: "application/xhtml+xml".into(),
		};

		let mapped = map_locator_to_time(
			&locator,
			&fixture_spine(),
			&fixture_chapters(),
			&fixture_map(),
		)
		.expect("spine item 2 is mapped");

		assert_eq!(mapped.position_ms, Some(45_000));
		assert!((mapped.progression - 0.375).abs() < 1e-9);
		assert!(mapped.approximate);
		assert_eq!(mapped.confidence, 0.6);
		assert!(mapped.locator.is_none());
	}

	/// A state that carries only progression (Kobo, KOReader) still converts:
	/// the spine item is derived from the cumulative character counts.
	#[test]
	fn reading_state_maps_a_progression_only_locator() {
		let locator = ReadiumLocator {
			chapter_title: String::new(),
			href: String::new(),
			title: None,
			locations: Some(ReadiumLocation {
				fragments: None,
				progression: None,
				position: None,
				// 2,900 of 6,000 characters: inside item 2, which spans
				// 1,400..4,400.
				total_progression: Decimal::try_from(0.483_333_333).ok(),
				css_selector: None,
				partial_cfi: None,
			}),
			text: None,
			kobo_span: None,
			r#type: "application/xhtml+xml".into(),
		};

		let mapped = map_locator_to_time(
			&locator,
			&fixture_spine(),
			&fixture_chapters(),
			&fixture_map(),
		)
		.expect("derived from total progression");

		// Half of item 2 -> half of chapter 2's 60 s.
		assert_eq!(mapped.position_ms, Some(60_000));
	}

	/// Front and back matter has no counterpart and must convert to nothing.
	/// Forcing a guess would resume the listener on the copyright page and
	/// the reader on the opening credits.
	#[test]
	fn reading_state_skips_unmapped_front_matter() {
		let chapters = fixture_chapters();
		let spine = fixture_spine();
		let map = fixture_map();

		// Inside the opening-credits chapter.
		assert_eq!(map_time_to_locator(3_000, &chapters, &spine, &map), None);

		// Inside the copyright page.
		let front = ReadiumLocator {
			chapter_title: String::new(),
			href: "OEBPS/copyright.xhtml".into(),
			title: None,
			locations: Some(ReadiumLocation {
				fragments: None,
				progression: Decimal::try_from(0.5).ok(),
				position: None,
				total_progression: None,
				css_selector: None,
				partial_cfi: None,
			}),
			text: None,
			kobo_span: None,
			r#type: "application/xhtml+xml".into(),
		};
		assert_eq!(map_locator_to_time(&front, &spine, &chapters, &map), None);
	}

	/// Round-tripping a mapped position must not walk: the same chapter
	/// fraction has to come back out, or a reader that switches devices twice
	/// would drift a chapter at a time.
	#[test]
	fn reading_state_round_trips_a_mapped_position() {
		let chapters = fixture_chapters();
		let spine = fixture_spine();
		let map = fixture_map();

		let forward =
			map_time_to_locator(75_000, &chapters, &spine, &map).expect("mapped chapter");
		let back = map_locator_to_time(
			forward.locator.as_ref().expect("locator"),
			&spine,
			&chapters,
			&map,
		)
		.expect("mapped spine item");

		assert_eq!(back.position_ms, Some(75_000));
	}
}
