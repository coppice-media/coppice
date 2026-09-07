//! Expanding one edition's identifier into its siblings.
//!
//! The pairing problem this exists for: an audiobook carries an Audible ASIN
//! and usually no ISBN, an EPUB carries an ISBN and no ASIN, and the two
//! numbers never match because they name different editions. Neither is a key
//! for "the same book" — but both upstreams that know about editions can walk
//! from one to the others:
//!
//! - Audnexus `GET /books/{asin}` carries the *print* edition's `isbn` on the
//!   audiobook's record, which is exactly the bridge from an audio ASIN to a
//!   text edition.
//! - Open Library `GET /isbn/{isbn}.json` resolves an edition, whose `works`
//!   key expands through `GET /works/{id}/editions.json` into every ISBN the
//!   work has ever been published under.
//!
//! So `stump_library::editions` asks each configured provider "what else names
//! this book", intersects the answers with the identifiers on the library's own
//! `media_metadata` rows, and pairs on a hit. The trait is deliberately
//! narrower than [`crate::MetadataProvider`]: it returns identifiers, not
//! candidates, because a pairing decision must not depend on how well two
//! titles score against each other.

use async_trait::async_trait;

use crate::{
	error::MetadataProviderError,
	providers::{AudibleClient, OpenLibraryClient},
};

/// An identifier that can name one edition of a work.
///
/// The variants are the carriers `media_metadata` already has columns for
/// (`identifier_isbn`, `identifier_amazon`/`identifier_mobi_asin`) plus the
/// Open Library work key, which is not an edition at all but is how Open
/// Library gets from one edition to the rest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EditionIdentifier {
	/// ISBN-10 or ISBN-13, digits and `X` only.
	Isbn(String),
	/// An Audible product ASIN (`B002V02KPU`).
	Asin(String),
	/// An Open Library work key (`OL45804W`).
	OpenLibraryWork(String),
}

impl EditionIdentifier {
	/// Normalise a raw metadata value: ISBNs lose their hyphens and spaces,
	/// ASINs and keys are upper-cased. An empty or punctuation-only value has
	/// no identity and is dropped.
	#[must_use]
	pub fn normalized(&self) -> Option<Self> {
		match self {
			Self::Isbn(value) => {
				let digits: String = value
					.chars()
					.filter(|c| c.is_ascii_alphanumeric())
					.map(|c| c.to_ascii_uppercase())
					.collect();
				(digits.len() == 10 || digits.len() == 13).then_some(Self::Isbn(digits))
			},
			Self::Asin(value) => {
				let value = value.trim().to_ascii_uppercase();
				(!value.is_empty()).then_some(Self::Asin(value))
			},
			Self::OpenLibraryWork(value) => {
				let value = value
					.rsplit('/')
					.next()
					.unwrap_or(value)
					.trim()
					.to_ascii_uppercase();
				(!value.is_empty()).then_some(Self::OpenLibraryWork(value))
			},
		}
	}
}

/// A provider that can answer "what else names this book".
#[async_trait]
pub trait EditionLookup: Send + Sync {
	/// The `metadata_provider_configs.provider_type` this lookup belongs to,
	/// so a pairing suggestion can name its source.
	fn provider_id(&self) -> &'static str;

	/// Identifiers of other editions of the same work, excluding `identifier`
	/// itself. An identifier this provider cannot resolve is not an error:
	/// most audiobooks have no ISBN and most ebooks have no ASIN, so a miss
	/// is the normal case and answers with an empty list.
	async fn sibling_identifiers(
		&self,
		identifier: &EditionIdentifier,
	) -> Result<Vec<EditionIdentifier>, MetadataProviderError>;
}

/// The edition lookup of a provider type, when it has one.
///
/// `None` for every provider whose upstream has no edition graph — a comic
/// database has issues, not editions — so a caller can hand it the operator's
/// whole enabled-provider list and get back only the ones that can help.
#[must_use]
pub fn create_edition_lookup(
	provider_type: &str,
) -> Option<Box<dyn EditionLookup + Send + Sync>> {
	match provider_type {
		"AUDIBLE" => Some(Box::new(AudibleClient::new())),
		"OPEN_LIBRARY" => Some(Box::new(OpenLibraryClient::new())),
		_ => None,
	}
}

/// The same lookup pointed at a local base URL, for tests in crates that
/// consume this trait but cannot reach the provider clients' private
/// `#[cfg(test)]` overrides.
#[cfg(any(test, feature = "mock"))]
#[must_use]
pub fn create_edition_lookup_at(
	provider_type: &str,
	base_url: &str,
) -> Option<Box<dyn EditionLookup + Send + Sync>> {
	match provider_type {
		"AUDIBLE" => Some(Box::new(AudibleClient::new().pointed_at(base_url))),
		"OPEN_LIBRARY" => Some(Box::new(OpenLibraryClient::new().pointed_at(base_url))),
		_ => None,
	}
}
