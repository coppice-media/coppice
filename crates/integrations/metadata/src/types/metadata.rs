use serde::{Deserialize, Serialize};

use crate::types::PublicationStatus;

#[cfg_attr(feature = "graphql", derive(async_graphql::Union))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExternalMetadata {
	Media(ExternalMediaMetadata),
	Series(ExternalSeriesMetadata),
}

impl ExternalMetadata {
	/// Returns a reference to the series metadata if this is a Series variant
	pub fn as_series(&self) -> Option<&ExternalSeriesMetadata> {
		match self {
			Self::Series(s) => Some(s),
			_ => None,
		}
	}

	/// Returns a reference to the media metadata if this is a Media variant
	pub fn as_media(&self) -> Option<&ExternalMediaMetadata> {
		match self {
			Self::Media(m) => Some(m),
			_ => None,
		}
	}
}

// TODO: Hone the fields we can pull across different providers

/// Metadata about a media item from an external metadata provider
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalMediaMetadata {
	pub provider: String,
	pub external_id: String,

	pub title: Option<String>,
	pub summary: Option<String>,
	pub page_count: Option<i32>,

	pub series_name: Option<String>,
	pub series_external_id: Option<String>,
	pub number: Option<f32>, // TODO: string?

	pub day: Option<i32>,
	pub month: Option<i32>,
	pub year: Option<i32>,

	pub genres: Option<Vec<String>>,
	pub tags: Option<Vec<String>>,

	pub isbn: Option<String>,
	pub isbn_13: Option<String>,

	pub writers: Option<Vec<String>>,
	pub artists: Option<Vec<String>>,
	pub colorists: Option<Vec<String>>,
	pub letterers: Option<Vec<String>>,
	pub cover_artists: Option<Vec<String>>,

	pub cover_url: Option<String>,

	pub provider_url: Option<String>,

	/// The edition's secondary title, as an audiobook's "A Novel" or
	/// "Special Edition" line. Kept out of `title` because a subtitle is not
	/// part of the name anyone searches for: folding it in would drag the
	/// title score of every later match down with it.
	pub subtitle: Option<String>,

	/// The readers of an audiobook edition. Deliberately *not* folded into
	/// `writers` or `artists`: a narrator is not an author, and merging the
	/// two would file a performer in the author column of every audiobook,
	/// where no later edit could tell them apart again.
	pub narrators: Option<Vec<String>>,

	/// The publisher of *this edition*. [`ExternalSeriesMetadata`] has one
	/// because a series has a publisher; a media item never did. An
	/// audiobook's publisher is a per-edition fact -- the same book is
	/// issued by a different studio in every market, often years apart -- so
	/// it cannot be inherited from the series it sits in.
	pub publisher: Option<String>,

	/// The runtime the provider advertises, in whole minutes.
	///
	/// This is *candidate evidence for the operator*, not a value to store:
	/// it is what tells an abridged edition from an unabridged one at a
	/// glance while picking a match. The authoritative duration is the one
	/// measured from the file itself (`media_audio.duration_ms`), which this
	/// never overwrites.
	pub runtime_minutes: Option<i32>,

	/// Whether the provider knows an ebook edition exists. `None` when the
	/// provider does not say; only Hardcover's search index carries the flag.
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub has_ebook: Option<bool>,

	/// Whether the provider knows an audiobook edition exists. Audible hits
	/// are audiobooks by construction; Hardcover reports its index flag.
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub has_audiobook: Option<bool>,

	/// The default audiobook edition's length in whole seconds, as Hardcover's
	/// index states it. Audible reports [`Self::runtime_minutes`] instead;
	/// callers wanting seconds from either provider derive them there.
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub audio_seconds: Option<i32>,
	/// Whether this audiobook edition is abridged, when the provider labels
	/// its format (Audible's `format_type`). `None` when unstated.
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub abridged: Option<bool>,

	/// The edition's language as the provider spells it, lowercased (Audible
	/// says `english`/`italian`); `None` when unstated.
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub language: Option<String>,
}

/// Metadata about a series from an external metadata provider
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalSeriesMetadata {
	pub provider: String,
	pub external_id: String,
	// pub provider_url: Option<String>,
	pub title: String,
	pub alternative_titles: Vec<String>,
	pub summary: Option<String>,
	pub status: Option<PublicationStatus>,
	pub year: Option<i32>,
	pub end_year: Option<i32>,

	pub genres: Option<Vec<String>>,
	pub tags: Option<Vec<String>>,
	pub age_rating: Option<String>,

	// TODO: Consider something like Vec<ExternalAuthor> to capture IDs
	pub authors: Option<Vec<String>>,
	pub artists: Option<Vec<String>>,
	pub publisher: Option<String>,

	pub cover_url: Option<String>,
	pub volume_count: Option<i32>,
}

/// One audiobook edition of a work the provider already identifies, as
/// returned by [`crate::MetadataProvider::audiobook_editions`]. Deliberately
/// narrower than [`ExternalMediaMetadata`]: the caller asked "who reads this
/// book and how long is it", not for a new candidate to match against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudiobookEdition {
	/// The provider's edition identifier, when it has one.
	pub external_id: Option<String>,
	/// The readers credited on this edition, in the provider's order.
	pub narrators: Vec<String>,
	/// Advertised length in whole seconds.
	pub audio_seconds: Option<i32>,
	/// Whether the edition is labelled abridged; `None` when unstated.
	pub abridged: Option<bool>,
	pub asin: Option<String>,
	/// The edition's language as the provider spells it (a name or ISO 639
	/// code, lowercased); `None` when unstated.
	pub language: Option<String>,
	/// How many provider users shelved this edition; a popularity order for
	/// callers listing several editions.
	pub users_count: Option<i32>,
}
