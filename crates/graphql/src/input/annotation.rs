use async_graphql::{InputObject, ID};
use chrono::{DateTime, Utc};
use models::shared::enums::DeviceKind;

use crate::object::annotation::AnnotationKind;

/// Narrows the cross-book annotation hub. Every field is a conjunction; an
/// empty list is the same as omitting it.
#[derive(Debug, Clone, Default, InputObject)]
pub struct AnnotationFilterInput {
	/// One book, by its Stump media id
	pub media_id: Option<ID>,
	/// Every book of one series
	pub series_id: Option<ID>,
	/// Every book of one library
	pub library_id: Option<ID>,
	pub kind: Option<Vec<AnnotationKind>>,
	/// Where the annotation came from, as reported by
	/// [`AnnotationEntry::source`](crate::object::annotation::AnnotationEntry)
	pub source: Option<Vec<DeviceKind>>,
	/// Case-insensitive substring match over the selected passage, the note,
	/// and the book title
	pub query: Option<String>,
	/// Only annotations created at or after this instant
	pub since: Option<DateTime<Utc>>,
}

impl AnnotationFilterInput {
	/// The kinds to include, `None` when unrestricted.
	pub(crate) fn kinds(&self) -> Option<&[AnnotationKind]> {
		self.kind.as_deref().filter(|kinds| !kinds.is_empty())
	}

	pub(crate) fn wants(&self, kind: AnnotationKind) -> bool {
		self.kinds().is_none_or(|kinds| kinds.contains(&kind))
	}

	/// The sources to include, `None` when unrestricted.
	pub(crate) fn sources(&self) -> Option<&[DeviceKind]> {
		self.source.as_deref().filter(|sources| !sources.is_empty())
	}

	/// The lowercased search needle, when the caller gave a non-blank one.
	pub(crate) fn needle(&self) -> Option<String> {
		self.query
			.as_deref()
			.map(str::trim)
			.filter(|query| !query.is_empty())
			.map(str::to_lowercase)
	}

	/// Whether the filter is scoped to Stump books, which excludes liseur
	/// works that are not linked to one.
	pub(crate) fn scoped_to_media(&self) -> bool {
		self.media_id.is_some() || self.series_id.is_some() || self.library_id.is_some()
	}
}
