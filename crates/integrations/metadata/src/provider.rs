use async_trait::async_trait;

use crate::{
	error::MetadataProviderError,
	types::{
		AudiobookEdition, ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate,
		MediaType, SearchOutcome, SearchQuery,
	},
	MatchScorer,
};

#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct ProviderCredentialVerification {
	pub response_status: u16,
	pub is_valid: bool,
	pub error: Option<String>,
}

/// Represents an external metadata source
#[async_trait]
pub trait MetadataProvider: Send + Sync {
	/// Unique identifier for this provider (e.g., "hardcover")
	fn id(&self) -> &'static str;

	/// Human-readable name for display
	fn name(&self) -> &'static str;

	/// Media types supported by this provider
	fn supported_media_types(&self) -> Vec<MediaType>;

	/// Search for series/works matching the query
	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError>;

	/// Search for individual books/issues/volumes/etc
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError>;

	/// Search for books using only what the provider's search index already
	/// returns, without a per-hit detail fetch. Intended for interactive
	/// lookups (type-ahead, request pickers) where latency matters more than
	/// completeness; providers whose index is not self-describing fall back
	/// to [`Self::search_media`].
	async fn search_media_brief(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		self.search_media(query).await
	}

	/// Score and sort search results based on their relevance to the query
	fn score_search(
		&self,
		query: &SearchQuery,
		mut candidates: Vec<MatchCandidate>,
	) -> Vec<MatchCandidate> {
		let scorer = MatchScorer;
		scorer.score_and_sort(query, &mut candidates);
		candidates
	}

	/// Fetch full metadata for a known external ID
	async fn fetch_series_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError>;

	/// Fetch full metadata for a specific book/volume/issue/etc
	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError>;

	/// The audiobook editions of a work this provider already identifies by
	/// `external_id`: narrators, advertised length and ASIN. One request at
	/// most. Providers without an edition graph know nothing and say so with
	/// an empty list rather than an error, so a caller merging several
	/// sources treats "no editions" and "no edition data" alike.
	async fn audiobook_editions(
		&self,
		_external_id: &str,
	) -> Result<Vec<AudiobookEdition>, MetadataProviderError> {
		Ok(Vec::new())
	}

	//// Fetch cover image URL
	// async fn fetch_cover_url(
	// 	&self,
	// 	external_id: &str,
	//  source_type ??? like Series/Media?
	// ) -> Result<Option<String>, MetadataProviderError>;

	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError>;
}
