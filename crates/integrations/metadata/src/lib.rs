//! Outbound metadata provider clients (Comic Vine, Hardcover, AniList, MAL,
//! MangaDex, MangaUpdates), rate limiting, candidate scoring and field-merge
//! rules. Persistence, credentials and fetch jobs live in `stump_core`.
//! See `crates/integrations/metadata/README.md`.

pub mod client;
pub mod error;
mod mangaupdates;
pub mod merge;
mod mock_http;
mod provider;
mod providers;
pub mod rate_limit;
pub mod scoring;
pub(crate) mod serde_utils;
pub mod types;

pub use client::build_client_with_retry;
pub use error::{MetadataProviderError, MetadataResult};
pub use mangaupdates::MangaUpdatesClient;
pub use merge::{AutoApplyConfig, FieldMerger, MergeStrategy, MetadataFieldOverride};
pub use provider::{MetadataProvider, ProviderCredentialVerification};
pub use rate_limit::RateLimiter;
pub use scoring::{title_similarity, MatchScorer};
pub use types::{
	ConfidenceFactor, ExternalMediaMetadata, ExternalMetadata, ExternalSeriesMetadata,
	MatchCandidate, MediaType, MetadataField, PublicationStatus, SearchOutcome,
	SearchQuery,
};

use providers::{
	AniListClient, ComicVineClient, HardcoverClient, MalClient, MangaDexClient,
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
		_ => Err(MetadataProviderError::UnsupportedProvider(
			provider_type.to_string(),
		)),
	}
}

/// Whether the named provider type requires an API token. Keyless providers
/// (public APIs that work without credentials) return false; add new keyless
/// provider strings here when registering them.
pub fn requires_api_token(provider_type: &str) -> bool {
	!matches!(provider_type, "ANILIST" | "MANGADEX" | "MANGA_UPDATES")
}
