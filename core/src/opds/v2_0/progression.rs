use chrono::{DateTime, FixedOffset};
use models::shared::readium::{ReadiumLocation, ReadiumLocator};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::entity::OPDSProgressionEntity;

pub const OPDS_PROGRESSION_MEDIA_TYPE: &str = "application/opds-progression+json";
pub const OPDS_PROGRESSION_REL: &str = "http://opds-spec.org/progression";

/// An OPDS Progression 1.0 document, used for both GET and PUT.
///
/// This intentionally does not replace the richer native Readium locator API.
/// See <https://drafts.opds.io/opds-progression-1.0.html>.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OPDSProgression {
	pub modified: DateTime<FixedOffset>,
	pub device: OPDSProgressionDevice,
	pub title: Option<String>,
	/// Whole-publication progression, in `0..=1`.
	pub progression: f64,
	/// URI references inside the publication, not server page-delivery URLs.
	pub references: Option<Vec<String>>,
}

impl OPDSProgression {
	pub fn new(data: OPDSProgressionEntity) -> Self {
		let (title, references) = if data.book.extension.eq_ignore_ascii_case("epub") {
			data.head.locator.map(epub_fields).unwrap_or_default()
		} else if let Some(page) = data.head.page {
			(
				Some(format!("Page {page}")),
				Some(vec![format!("#page={page}")]),
			)
		} else {
			(None, None)
		};
		Self {
			modified: data.head.updated_at,
			device: data
				.device
				.map(|device| OPDSProgressionDevice {
					id: device.id,
					name: device.name,
				})
				.unwrap_or_default(),
			title,
			// The head is authoritative even when a fraction-only update kept
			// a more precise locator from an earlier protocol update.
			progression: data.head.progression,
			references,
		}
	}

	pub fn device(&self) -> Option<&OPDSProgressionDevice> {
		if self.device.id.is_empty() && self.device.name.is_empty() {
			None
		} else {
			Some(&self.device)
		}
	}

	/// The PDF-style page reference used for page-addressed publications.
	pub fn page(&self) -> Option<i32> {
		self.references.as_ref()?.iter().find_map(|reference| {
			reference
				.strip_prefix("#page=")
				.and_then(|page| page.parse().ok())
		})
	}

	pub fn percentage_completed(&self) -> Option<Decimal> {
		Decimal::try_from(self.progression).ok()
	}

	/// Project a packaged-publication reference onto the native locator.
	///
	/// A fraction-only document or publication-wide fragment has no resource
	/// anchor. Do not manufacture an empty locator that erases native context.
	pub fn locator(&self) -> Option<ReadiumLocator> {
		let reference = self
			.references
			.as_ref()?
			.iter()
			.find(|reference| !reference.is_empty() && !reference.starts_with('#'))?;
		let (href, fragment) = reference
			.split_once('#')
			.map_or((reference.as_str(), None), |(href, fragment)| {
				(href, (!fragment.is_empty()).then_some(fragment))
			});
		Some(ReadiumLocator {
			href: href.to_string(),
			title: self.title.clone(),
			r#type: "application/xhtml+xml".to_string(),
			chapter_title: String::new(),
			locations: Some(ReadiumLocation {
				fragments: fragment.map(|fragment| vec![fragment.to_string()]),
				total_progression: self.percentage_completed(),
				progression: None,
				position: None,
				css_selector: None,
				partial_cfi: None,
			}),
			text: None,
			kobo_span: None,
		})
	}
}

fn epub_fields(locator: ReadiumLocator) -> (Option<String>, Option<Vec<String>>) {
	let title = if locator.chapter_title.is_empty() {
		locator.title
	} else {
		Some(locator.chapter_title)
	};
	let mut reference = locator.href;
	if reference.is_empty() {
		return (title, None);
	}
	// A native href may already include its fragment. Appending it again
	// would turn a valid anchor into chapter.xhtml#paragraph#paragraph.
	if !reference.contains('#') {
		if let Some(fragment) = locator
			.locations
			.as_ref()
			.and_then(|locations| locations.fragments.as_ref())
			.and_then(|fragments| fragments.first())
			.filter(|fragment| !fragment.is_empty())
		{
			if !fragment.starts_with('#') {
				reference.push('#');
			}
			reference.push_str(fragment);
		}
	}
	(title, Some(vec![reference]))
}

/// The device that supplied the progression, distinct from a native locator.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OPDSProgressionDevice {
	pub id: String,
	pub name: String,
}

#[cfg(test)]
mod tests {
	use super::*;
	use models::{domain::reading_state::SourceProtocol, entity::reading_head};
	use serde_json::json;

	use crate::opds::v2_0::entity::OPDSProgressionBookRef;

	fn document(references: Option<Vec<&str>>) -> OPDSProgression {
		serde_json::from_value(json!({
			"modified": "2026-01-28T08:17:11.986000-07:00",
			"device": { "id": "urn:uuid:device-123", "name": "OPDS Reader" },
			"progression": 0.25,
			"title": "Chapter 1",
			"references": references,
		}))
		.unwrap()
	}

	fn entity(
		locator: Option<ReadiumLocator>,
		page: Option<i32>,
	) -> OPDSProgressionEntity {
		let now = DateTime::parse_from_rfc3339("2026-01-28T08:17:11-07:00").unwrap();
		OPDSProgressionEntity {
			head: reading_head::Model {
				user_id: "user".into(),
				media_id: "book".into(),
				locator,
				progression: 0.5,
				page,
				position_ms: None,
				track_index: None,
				completed: false,
				updated_at: now,
				created_at: now,
				changed_at: now,
				source_protocol: SourceProtocol::Opds,
				source_device_id: None,
				revision: 1,
				event_id: 1,
			},
			device: None,
			book: OPDSProgressionBookRef {
				id: "book".into(),
				extension: if page.is_some() { "pdf" } else { "epub" }.into(),
				pages: if page.is_some() { 20 } else { -1 },
				analysis: None,
			},
		}
	}

	#[test]
	fn stable_document_deserializes_and_page_is_not_a_fake_resource() {
		let input = document(Some(vec!["#page=5"]));
		assert_eq!(input.page(), Some(5));
		assert_eq!(input.device().unwrap().id, "urn:uuid:device-123");
		assert_eq!(input.percentage_completed(), Some(Decimal::new(25, 2)));
		assert!(input.locator().is_none());
	}

	#[test]
	fn stable_document_requires_progression_not_the_legacy_locator_envelope() {
		assert!(serde_json::from_value::<OPDSProgression>(json!({
			"modified": "2026-01-28T08:17:11-07:00",
			"device": { "id": "device", "name": "Reader" },
			"locator": { "href": "chapter.xhtml", "type": "application/xhtml+xml" }
		}))
		.is_err());
	}

	#[test]
	fn fraction_only_and_fragment_only_documents_preserve_native_locator_ownership() {
		for references in [None, Some(vec![]), Some(vec![""]), Some(vec!["#t=42"])] {
			assert!(document(references).locator().is_none());
		}
	}

	#[test]
	fn epub_reference_round_trips_resource_and_fragment_without_inventing_position() {
		let input = document(Some(vec!["#t=42", "OPS/chapter.xhtml#paragraph"]));
		let locator = input.locator().unwrap();
		assert_eq!(locator.href, "OPS/chapter.xhtml");
		let locations = locator.locations.as_ref().unwrap();
		assert_eq!(locations.fragments, Some(vec!["paragraph".into()]));
		assert_eq!(locations.total_progression, Some(Decimal::new(25, 2)));
		assert!(locations.position.is_none());
		assert!(locations.progression.is_none());
		assert!(locator.text.is_none());

		let output = OPDSProgression::new(entity(Some(locator), None));
		assert_eq!(
			output.references,
			Some(vec!["OPS/chapter.xhtml#paragraph".into()])
		);
		assert_eq!(
			output.progression, 0.5,
			"the canonical head owns the fraction"
		);
	}

	#[test]
	fn native_epub_href_with_fragment_is_not_duplicated() {
		let mut locator = document(Some(vec!["OPS/chapter.xhtml#paragraph"]))
			.locator()
			.unwrap();
		locator.href.push_str("#paragraph");
		let output = OPDSProgression::new(entity(Some(locator), None));
		assert_eq!(
			output.references,
			Some(vec!["OPS/chapter.xhtml#paragraph".into()])
		);
	}

	#[test]
	fn paged_output_uses_publication_fragment_and_flat_progression() {
		let output =
			serde_json::to_value(OPDSProgression::new(entity(None, Some(10)))).unwrap();
		assert_eq!(output["references"], json!(["#page=10"]));
		assert_eq!(output["progression"], 0.5);
		assert_eq!(output["title"], "Page 10");
		assert!(output.get("locator").is_none());
	}
}
