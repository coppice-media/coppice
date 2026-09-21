//! Outbound metadata provider clients (Comic Vine, Hardcover, AniList, MAL,
//! MangaDex, MangaUpdates, Open Library, Google Books, Metron), rate limiting,
//! candidate scoring and field-merge rules. Persistence, credentials and
//! fetch jobs live in `stump_core`. See `crates/integrations/metadata/README.md`.

pub mod client;
pub mod editions;
pub mod error;
mod mangaupdates;
pub mod merge;
/// Test-only HTTP mock. Public under `mock` so crates that consume
/// [`editions::EditionLookup`] can point a real provider client at a local
/// server without a network dependency, the same way `stump_provider`
/// exposes its own.
#[cfg(any(test, feature = "mock"))]
pub mod mock_http;
mod provider;
mod providers;
pub mod rate_limit;
pub mod scoring;
pub(crate) mod serde_utils;
pub mod types;

pub use client::build_client_with_retry;
pub use editions::{create_edition_lookup, EditionIdentifier, EditionLookup};
pub use error::{MetadataProviderError, MetadataResult};
pub use mangaupdates::MangaUpdatesClient;
pub use merge::{AutoApplyConfig, FieldMerger, MergeStrategy, MetadataFieldOverride};
pub use provider::{MetadataProvider, ProviderCredentialVerification};
use providers::{
	AniListClient, AudibleClient, ComicVineClient, GoogleBooksClient, MalClient,
	MangaDexClient, MetronClient, OpenLibraryClient,
};
pub use providers::{HardcoverClient, HardcoverIdentity, HardcoverJournalEntry};
pub use rate_limit::RateLimiter;
pub use scoring::{title_similarity, MatchScorer};
pub use types::{
	ConfidenceFactor, ExternalMediaMetadata, ExternalMetadata, ExternalSeriesMetadata,
	MatchCandidate, MediaType, MetadataField, PublicationStatus, SearchOutcome,
	SearchQuery,
};

pub fn create_provider(
	provider_type: &str,
	api_token: String,
) -> MetadataResult<Box<dyn MetadataProvider + Send + Sync>> {
	match provider_type {
		"COMIC_VINE" => Ok(Box::new(ComicVineClient::new(api_token, None))),
		"HARDCOVER" => Ok(Box::new(HardcoverClient::new(api_token, None))),
		// AniList is keyless; the token argument is ignored.
		"ANILIST" => Ok(Box::new(AniListClient::new(None, None))),
		"MANGA_UPDATES" => Ok(Box::new(MangaUpdatesClient::new())),
		// The MAL client id is stored as the provider's API token.
		"MAL" => Ok(Box::new(MalClient::new(api_token, None))),
		// MangaDex is keyless; the token argument is ignored.
		"MANGADEX" => Ok(Box::new(MangaDexClient::new())),
		// Open Library and Google Books are keyless; the token argument is
		// ignored (a Google Books key only raises the quota).
		"OPEN_LIBRARY" => Ok(Box::new(OpenLibraryClient::new())),
		"GOOGLE_BOOKS" => Ok(Box::new(GoogleBooksClient::new(
			api_token.filter_token(),
			None,
		))),
		// Audnexus and the Audible catalogue are both unauthenticated; the
		// token argument is ignored.
		"AUDIBLE" => Ok(Box::new(AudibleClient::new())),
		// Metron stores the Basic credential `username:password` (or a
		// pre-encoded Basic token) as the provider's API token.
		"METRON" => Ok(Box::new(MetronClient::new(api_token, None))),
		_ => Err(MetadataProviderError::UnsupportedProvider(
			provider_type.to_string(),
		)),
	}
}

/// Whether the named provider type requires an API token. Keyless providers
/// (public APIs that work without credentials) return false; add new keyless
/// provider strings here when registering them.
pub fn requires_api_token(provider_type: &str) -> bool {
	!matches!(
		provider_type,
		"ANILIST"
			| "MANGADEX"
			| "MANGA_UPDATES"
			| "OPEN_LIBRARY"
			| "GOOGLE_BOOKS"
			| "AUDIBLE"
	)
}

/// Small helper so `create_provider` can hand the raw token string to a
/// client expecting `Option<String>`: an empty token means "no credential".
trait OptionalToken {
	fn filter_token(self) -> Option<String>;
}

impl OptionalToken for String {
	fn filter_token(self) -> Option<String> {
		(!self.trim().is_empty()).then_some(self)
	}
}
