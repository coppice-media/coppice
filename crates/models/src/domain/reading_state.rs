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
}

impl<'a> From<&'a media::Model> for Publication<'a> {
	fn from(media: &'a media::Model) -> Self {
		Publication {
			media_id: &media.id,
			pages: media.pages,
		}
	}
}

/// The head fields an update asserts. `None` keeps the current head value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projection {
	pub locator: Option<ReadiumLocator>,
	pub page: Option<i32>,
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
			}
		},
		Position::None => Projection {
			locator: None,
			page: None,
			progression: asserted,
			completed: update.completed,
		},
	};

	if update.completed == Some(true)
		&& projection.page.is_none()
		&& projection.progression.is_none()
	{
		projection.page = (publication.pages > 0).then_some(publication.pages);
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
