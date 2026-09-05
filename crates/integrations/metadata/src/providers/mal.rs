//! MyAnimeList (MAL) metadata provider.
//!
//! Mirrors the behaviour of Komf's MAL provider
//! (`komf-core/src/commonMain/kotlin/snd/komf/providers/mal/`): the official
//! v2 API is series-level, so `search_media`/`fetch_media_metadata` project
//! the manga record onto media metadata while `search_series`/
//! `fetch_series_metadata` keep the series shape.
//!
//! API: https://api.myanimelist.net/v2 — every request carries an
//! `X-MAL-CLIENT-ID` header; the client id is stored as the provider's
//! encrypted API token.

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	error::MetadataProviderError,
	provider::ProviderCredentialVerification,
	types::{
		ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate, MediaType,
		PublicationStatus, SearchOutcome, SearchQuery,
	},
	ExternalMetadata, MetadataProvider, RateLimiter,
};

/// Conservative client-side rate limit. The official API does not publish a
/// numeric quota; 1 req/sec stays well below the undocumented per-second cap
/// that returns HTTP 429.
const MAL_DEFAULT_RATE_LIMIT: u32 = 1;

/// Fields requested for a full manga lookup.
const DETAIL_FIELDS: &str = "id,title,alternative_titles,synopsis,status,start_date,genres,authors{first_name,last_name},main_picture,media_type,serialization{name}";

/// Fields requested for a search hit.
const SEARCH_FIELDS: &str = "alternative_titles,media_type";

/// Komf filters search hits by media type per configuration:
/// MANGA -> {manga, one_shot, doujinshi, manhwa, manhua, oel},
/// NOVEL -> {novel, light_novel}, WEBTOON -> {manhua, manhwa}.
/// This client serves all three configurations, so the union applies:
/// anything else (or unknown) is filtered out.
fn is_supported_media_type(media_type: Option<&str>) -> bool {
	matches!(
		media_type,
		Some(
			"manga"
				| "one_shot" | "doujinshi"
				| "manhwa" | "manhua"
				| "oel" | "novel"
				| "light_novel"
		)
	)
}

/// Map a raw MAL status string onto the shared publication status.
/// Mirrors Komf's `MalMetadataMapper` status mapping.
fn publication_status(status: Option<&str>) -> Option<PublicationStatus> {
	match status? {
		"finished" => Some(PublicationStatus::Completed),
		"currently_publishing" => Some(PublicationStatus::Ongoing),
		"not_yet_published" => Some(PublicationStatus::Upcoming),
		"on_hiatus" => Some(PublicationStatus::Hiatus),
		"discontinued" => Some(PublicationStatus::Cancelled),
		_ => None,
	}
}

/// Parse a MAL date (`YYYY`, `YYYY-MM`, or `YYYY-MM-DD`) into components.
fn parse_date_parts(date: Option<&str>) -> (Option<i32>, Option<i32>, Option<i32>) {
	let Some(date) = date else {
		return (None, None, None);
	};
	let mut parts = date.split('-');
	let year = parts.next().and_then(|part| part.parse().ok());
	let month = parts.next().and_then(|part| part.parse().ok());
	let day = parts.next().and_then(|part| part.parse().ok());
	(year, month, day)
}

pub struct MalClient {
	client: reqwest_middleware::ClientWithMiddleware,
	client_id: String,
	api_url: String,
	rate_limiter: RateLimiter,
}

impl MalClient {
	const API_URL: &'static str = "https://api.myanimelist.net";

	pub fn new(client_id: String, rate_limit: Option<u32>) -> Self {
		Self::build(
			client_id,
			rate_limit,
			Self::API_URL.to_string(),
			RetryClientConfig::default().max_retries,
		)
	}

	fn build(
		client_id: String,
		rate_limit: Option<u32>,
		api_url: String,
		max_retries: u32,
	) -> Self {
		let inner = reqwest::Client::builder()
			.user_agent(concat!(
				"stump/",
				env!("CARGO_PKG_VERSION"),
				" (+https://github.com/stumpapp/stump)"
			))
			.build()
			.expect("Failed to build MyAnimeList HTTP client");
		Self {
			client: build_client_with_retry(inner, RetryClientConfig { max_retries }),
			client_id,
			api_url,
			rate_limiter: RateLimiter::new(rate_limit.unwrap_or(MAL_DEFAULT_RATE_LIMIT)),
		}
	}

	/// Test-only override of the API base URL and retry budget, used to point
	/// the client at a local mock server.
	#[cfg(test)]
	fn with_test_config(mut self, api_url: impl Into<String>, max_retries: u32) -> Self {
		self.api_url = api_url.into();
		let inner = reqwest::Client::builder()
			.user_agent(concat!(
				"stump/",
				env!("CARGO_PKG_VERSION"),
				" (+https://github.com/stumpapp/stump)"
			))
			.build()
			.expect("Failed to build MyAnimeList HTTP client");
		self.client = build_client_with_retry(inner, RetryClientConfig { max_retries });
		self
	}

	/// Send an authenticated GET request to the MAL v2 API.
	///
	/// - `path` should start with a `/`, e.g. `/v2/manga`.
	/// - `params` are appended as additional query parameters.
	async fn get<T: DeserializeOwned>(
		&self,
		path: &str,
		params: &[(&str, &str)],
	) -> Result<T, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let mut url = reqwest::Url::parse(&format!("{}{}", self.api_url, path))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;
		for (key, value) in params {
			url.query_pairs_mut().append_pair(key, value);
		}

		let response = self
			.client
			.get(url)
			.header("X-MAL-CLIENT-ID", &self.client_id)
			.send()
			.await?
			.error_for_status()?;

		// Parse explicitly so malformed bodies surface as `ParseError`
		// instead of an opaque reqwest decode error.
		let body = response.bytes().await?;
		Ok(serde_json::from_slice(&body)?)
	}

	/// Search manga by title. Mirrors Komf's `searchSeries`: queries shorter
	/// than 3 characters are refused and the query is truncated to 64 chars.
	async fn search_manga(
		&self,
		query: &SearchQuery,
	) -> Result<Vec<MalSearchResult>, MetadataProviderError> {
		let title = query.title.trim();
		if title.chars().count() < 3 {
			tracing::debug!(
				title,
				"Query is shorter than 3 characters, skipping MyAnimeList search"
			);
			return Ok(Vec::new());
		}
		let query_text: String = title.chars().take(64).collect();
		let limit = query.limit.unwrap_or(10);
		let limit_string = limit.to_string();

		let response: MalSearchResponse = self
			.get(
				"/v2/manga",
				&[
					("fields", SEARCH_FIELDS),
					("q", &query_text),
					("limit", &limit_string),
					("nsfw", "true"),
				],
			)
			.await?;

		Ok(response
			.data
			.into_iter()
			.filter(|node| is_supported_media_type(node.node.media_type.as_deref()))
			.take(limit as usize)
			.map(|node| node.node)
			.collect())
	}

	async fn fetch_manga(
		&self,
		external_id: &str,
	) -> Result<MalManga, MetadataProviderError> {
		let id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!("Invalid manga ID: {}", external_id))
		})?;

		self.get(&format!("/v2/manga/{id}"), &[("fields", DETAIL_FIELDS)])
			.await
	}
}

#[async_trait::async_trait]
impl MetadataProvider for MalClient {
	fn id(&self) -> &'static str {
		"mal"
	}

	fn name(&self) -> &'static str {
		"MyAnimeList"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Manga, MediaType::LightNovel, MediaType::Webtoon]
	}

	/// Search for manga (series-level records) and map each hit onto a series
	/// candidate. No per-hit detail fetch: MAL search results already carry
	/// the title/alternative titles needed for matching, matching Komf.
	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let hits = self.search_manga(query).await?;
		let requested = hits.len();

		let candidates = hits
			.into_iter()
			.map(|node| MatchCandidate {
				external_id: node.id.to_string(),
				provider: self.id().to_string(),
				confidence: 0.0,
				confidence_factors: Vec::new(),
				metadata: ExternalMetadata::Series(node.into_series_metadata("mal")),
			})
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	/// Search for manga and map each hit onto a media-level candidate. MAL is
	/// series-level (Komf's `getBookMetadata` is `UnsupportedOperation`), so a
	/// media candidate is the manga record itself.
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let hits = self.search_manga(query).await?;
		let requested = hits.len();

		let candidates = hits
			.into_iter()
			.map(|node| MatchCandidate {
				external_id: node.id.to_string(),
				provider: self.id().to_string(),
				confidence: 0.0,
				confidence_factors: Vec::new(),
				metadata: ExternalMetadata::Media(node.into_media_metadata("mal")),
			})
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	/// Fetch the full manga record and project it onto series metadata.
	async fn fetch_series_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		let manga = self.fetch_manga(external_id).await?;
		Ok(manga.into_series_metadata(self.id()))
	}

	/// Fetch the full manga record and project it onto media-level metadata.
	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let manga = self.fetch_manga(external_id).await?;
		Ok(manga.into_media_metadata(self.id()))
	}

	/// The v2 API has no dedicated credentials endpoint; a 1-result search
	/// that is not rejected (401/403) proves the client id is valid.
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let mut url = reqwest::Url::parse(&format!("{}/v2/manga", self.api_url))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;
		url.query_pairs_mut()
			.append_pair("limit", "1")
			.append_pair("fields", "id");

		let response = self
			.client
			.get(url)
			.header("X-MAL-CLIENT-ID", &self.client_id)
			.send()
			.await?;
		let status = response.status().as_u16();

		if status == 401 || status == 403 {
			return Ok(ProviderCredentialVerification {
				response_status: status,
				is_valid: false,
				error: Some("MyAnimeList rejected the client id".to_string()),
			});
		}

		response.error_for_status()?;
		Ok(ProviderCredentialVerification {
			response_status: status,
			is_valid: true,
			error: None,
		})
	}
}

/// Alternative titles returned for a manga record.
#[derive(Debug, Deserialize)]
pub struct MalAlternativeTitles {
	#[serde(default)]
	pub synonyms: Vec<String>,
	#[serde(default)]
	pub en: Option<String>,
	#[serde(default)]
	pub ja: Option<String>,
}

impl MalAlternativeTitles {
	/// Flattened alternative titles: synonyms first, then English, then
	/// Japanese. Blank values are skipped.
	fn flattened(&self) -> Vec<String> {
		self.synonyms
			.iter()
			.chain(self.en.iter())
			.chain(self.ja.iter())
			.filter(|title| !title.trim().is_empty())
			.cloned()
			.collect()
	}
}

#[derive(Debug, Deserialize)]
pub struct MalPicture {
	pub large: Option<String>,
	pub medium: Option<String>,
}

impl MalPicture {
	/// Prefer the large variant, falling back to medium.
	fn best_url(&self) -> Option<String> {
		self.large.clone().or_else(|| self.medium.clone())
	}
}

#[derive(Debug, Deserialize)]
pub struct MalNamedRef {
	pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MalAuthor {
	pub node: MalAuthorNode,
	/// Raw MAL role: `Story`, `Art`, or `Story & Art`.
	pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct MalAuthorNode {
	#[serde(default)]
	pub first_name: Option<String>,
	#[serde(default)]
	pub last_name: Option<String>,
}

impl MalAuthorNode {
	fn display_name(&self) -> Option<String> {
		let name = [self.first_name.as_deref(), self.last_name.as_deref()]
			.into_iter()
			.flatten()
			.filter(|part| !part.trim().is_empty())
			.collect::<Vec<_>>()
			.join(" ");
		(!name.is_empty()).then_some(name)
	}
}

#[derive(Debug, Deserialize)]
pub struct MalSerialization {
	pub node: MalSerializationNode,
}

#[derive(Debug, Deserialize)]
pub struct MalSerializationNode {
	pub name: String,
}

/// A manga record as returned by `/v2/manga/{id}` with `DETAIL_FIELDS`.
#[derive(Debug, Deserialize)]
pub struct MalManga {
	pub id: i64,
	pub title: String,
	#[serde(default)]
	pub alternative_titles: Option<MalAlternativeTitles>,
	#[serde(default)]
	pub synopsis: Option<String>,
	#[serde(default)]
	pub status: Option<String>,
	#[serde(rename = "start_date", default)]
	pub start_date: Option<String>,
	#[serde(default)]
	pub genres: Vec<MalNamedRef>,
	#[serde(default)]
	pub authors: Vec<MalAuthor>,
	#[serde(rename = "main_picture")]
	pub main_picture: Option<MalPicture>,
	#[serde(rename = "media_type", default)]
	pub media_type: Option<String>,
	#[serde(default)]
	pub serialization: Vec<MalSerialization>,
}

/// A manga record as returned by `/v2/manga?q=` with `SEARCH_FIELDS`.
#[derive(Debug, Deserialize)]
pub struct MalSearchResult {
	pub id: i64,
	pub title: String,
	#[serde(default)]
	pub alternative_titles: Option<MalAlternativeTitles>,
	#[serde(rename = "media_type", default)]
	pub media_type: Option<String>,
	#[serde(rename = "main_picture")]
	pub main_picture: Option<MalPicture>,
}

#[derive(Debug, Deserialize)]
pub struct MalSearchNode {
	pub node: MalSearchResult,
}

#[derive(Debug, Deserialize)]
pub struct MalSearchResponse {
	#[serde(default)]
	pub data: Vec<MalSearchNode>,
}

impl MalSearchResult {
	fn into_series_metadata(self, provider_id: &str) -> ExternalSeriesMetadata {
		ExternalSeriesMetadata {
			provider: provider_id.to_string(),
			external_id: self.id.to_string(),
			title: self.title,
			alternative_titles: self
				.alternative_titles
				.map(|titles| titles.flattened())
				.unwrap_or_default(),
			..Default::default()
		}
	}

	fn into_media_metadata(self, provider_id: &str) -> ExternalMediaMetadata {
		let external_id = self.id.to_string();
		ExternalMediaMetadata {
			provider: provider_id.to_string(),
			series_name: Some(self.title.clone()),
			series_external_id: Some(external_id.clone()),
			external_id,
			title: Some(self.title),
			cover_url: self.main_picture.and_then(|picture| picture.best_url()),
			provider_url: Some(format!("https://myanimelist.net/manga/{}", self.id)),
			..Default::default()
		}
	}
}

impl MalManga {
	/// Authors split by raw MAL role: `Story` -> writers, `Art` -> artists,
	/// `Story & Art` -> both (mirrors Komf's author-role mapping).
	fn split_authors(&self) -> (Option<Vec<String>>, Option<Vec<String>>) {
		let mut writers = Vec::new();
		let mut artists = Vec::new();
		for author in &self.authors {
			let Some(name) = author.node.display_name() else {
				continue;
			};
			match author.role.as_str() {
				"Story" => writers.push(name),
				"Art" => artists.push(name),
				"Story & Art" => {
					writers.push(name.clone());
					artists.push(name);
				},
				_ => {},
			}
		}
		let filled = |list: Vec<String>| (!list.is_empty()).then_some(list);
		(filled(writers), filled(artists))
	}

	fn cover_url(&self) -> Option<String> {
		self.main_picture
			.as_ref()
			.and_then(|picture| picture.best_url())
	}

	fn into_series_metadata(self, provider_id: &str) -> ExternalSeriesMetadata {
		let (writers, artists) = self.split_authors();
		let (year, _month, _day) = parse_date_parts(self.start_date.as_deref());
		let genres = (!self.genres.is_empty()).then(|| {
			self.genres
				.iter()
				.map(|genre| genre.name.clone())
				.collect::<Vec<_>>()
		});
		let cover_url = self.cover_url();
		let publisher = self
			.serialization
			.first()
			.map(|serialization| serialization.node.name.clone());

		ExternalSeriesMetadata {
			provider: provider_id.to_string(),
			external_id: self.id.to_string(),
			title: self.title,
			alternative_titles: self
				.alternative_titles
				.map(|titles| titles.flattened())
				.unwrap_or_default(),
			summary: self.synopsis,
			status: publication_status(self.status.as_deref()),
			year,
			genres,
			authors: writers,
			artists,
			publisher,
			cover_url,
			..Default::default()
		}
	}

	fn into_media_metadata(self, provider_id: &str) -> ExternalMediaMetadata {
		let (writers, artists) = self.split_authors();
		let (year, month, day) = parse_date_parts(self.start_date.as_deref());
		let cover_url = self.cover_url();
		let title = self.title;
		let summary = self.synopsis;
		let external_id = self.id.to_string();

		ExternalMediaMetadata {
			provider: provider_id.to_string(),
			external_id,
			series_name: Some(title.clone()),
			series_external_id: Some(self.id.to_string()),
			title: Some(title),
			year,
			month,
			day,
			genres: (!self.genres.is_empty()).then(|| {
				self.genres
					.iter()
					.map(|genre| genre.name.clone())
					.collect::<Vec<_>>()
			}),
			writers,
			artists,
			cover_url,
			summary,
			provider_url: Some(format!("https://myanimelist.net/manga/{}", self.id)),
			..Default::default()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::mock_http::{render_ok, MockServer};

	fn test_client(server: &MockServer) -> MalClient {
		MalClient::new("test-client-id".to_string(), Some(u32::MAX))
			.with_test_config(server.url.clone(), 0)
	}

	fn search_response_body() -> String {
		serde_json::json!({
			"data": [
				{
					"node": {
						"id": 1,
						"title": "Berserk",
						"alternative_titles": {
							"synonyms": ["Beruseruku"],
							"en": "Berserk",
							"ja": "ベルセルク"
						},
						"media_type": "manga",
						"main_picture": {
							"large": "https://cdn.myanimelist.net/images/manga/1/l.jpg",
							"medium": "https://cdn.myanimelist.net/images/manga/1/m.jpg"
						}
					}
				},
				{
					"node": {
						"id": 2,
						"title": "Unsupported media type",
						"media_type": "music"
					}
				}
			],
			"paging": {}
		})
		.to_string()
	}

	fn detail_response_body() -> String {
		serde_json::json!({
			"id": 1,
			"title": "Berserk",
			"alternative_titles": {
				"synonyms": ["Beruseruku"],
				"en": "Berserk",
				"ja": "ベルセルク"
			},
			"synopsis": "Guts, a mercenary, travels with the Band of the Hawk.",
			"status": "currently_publishing",
			"start_date": "1989-08-25",
			"genres": [
				{ "id": 1, "name": "Action" },
				{ "id": 14, "name": "Horror" }
			],
			"authors": [
				{
					"node": { "id": 11, "first_name": "Kentaro", "last_name": "Miura" },
					"role": "Story & Art"
				},
				{
					"node": { "id": 12, "first_name": "Story", "last_name": "Only" },
					"role": "Story"
				},
				{
					"node": { "id": 13, "first_name": "Art", "last_name": "Only" },
					"role": "Art"
				}
			],
			"main_picture": {
				"large": "https://cdn.myanimelist.net/images/manga/1/l.jpg",
				"medium": "https://cdn.myanimelist.net/images/manga/1/m.jpg"
			},
			"media_type": "manga",
			"serialization": [
				{ "node": { "id": 2, "name": "Young Animal" } }
			]
		})
		.to_string()
	}

	#[tokio::test]
	async fn search_media_maps_mocked_search_response() {
		let server = MockServer::spawn(vec![render_ok(&search_response_body())]);
		let client = test_client(&server);

		let query = SearchQuery {
			title: "Berserk".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let outcome = client
			.search_media(&query)
			.await
			.expect("search should succeed against the mock");

		// requested counts the post-filter hits this client processed;
		// the unsupported `music` media type was filtered out.
		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "mal");
		assert_eq!(candidate.external_id, "1");

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(media.title.as_deref(), Some("Berserk"));
		assert_eq!(media.series_name.as_deref(), Some("Berserk"));
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://cdn.myanimelist.net/images/manga/1/l.jpg")
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://myanimelist.net/manga/1")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(requests[0].starts_with("GET /v2/manga?"));
		assert!(requests[0].contains("q=Berserk"));
		assert!(requests[0].contains("limit=5"));
		assert!(requests[0].contains("nsfw=true"));
		assert!(requests[0].contains("fields=alternative_titles"));
		assert!(requests[0].contains("x-mal-client-id: test-client-id"));
		assert!(requests[0].contains("user-agent: stump/"));
		assert!(requests[0].contains("(+https://github.com/stumpapp/stump)"));
	}

	#[tokio::test]
	async fn search_series_maps_alternative_titles() {
		let server = MockServer::spawn(vec![render_ok(&search_response_body())]);
		let client = test_client(&server);

		let query = SearchQuery {
			title: "Beruseruku".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let outcome = client
			.search_series(&query)
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.candidates.len(), 1);
		let series = outcome.candidates[0].metadata.as_series().unwrap();
		assert_eq!(series.title, "Berserk");
		assert_eq!(
			series.alternative_titles,
			vec![
				"Beruseruku".to_string(),
				"Berserk".to_string(),
				"ベルセルク".to_string()
			]
		);
	}

	#[tokio::test]
	async fn fetch_media_metadata_maps_mocked_detail_response() {
		let server = MockServer::spawn(vec![render_ok(&detail_response_body())]);
		let client = test_client(&server);

		let media = client
			.fetch_media_metadata("1")
			.await
			.expect("fetch should succeed against the mock");

		assert_eq!(media.external_id, "1");
		assert_eq!(media.title.as_deref(), Some("Berserk"));
		assert_eq!(
			media.summary.as_deref(),
			Some("Guts, a mercenary, travels with the Band of the Hawk.")
		);
		assert_eq!(media.year, Some(1989));
		assert_eq!(media.month, Some(8));
		assert_eq!(media.day, Some(25));
		assert_eq!(
			media.genres.as_deref(),
			Some(["Action".to_string(), "Horror".to_string()].as_slice())
		);
		// "Story & Art" lands in both roles.
		assert_eq!(
			media.writers.as_deref(),
			Some(["Kentaro Miura".to_string(), "Story Only".to_string()].as_slice())
		);
		assert_eq!(
			media.artists.as_deref(),
			Some(["Kentaro Miura".to_string(), "Art Only".to_string()].as_slice())
		);
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://cdn.myanimelist.net/images/manga/1/l.jpg")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(requests[0].starts_with("GET /v2/manga/1?"));
		assert!(requests[0].contains("authors%7Bfirst_name%2Clast_name%7D"));
		assert!(requests[0].contains("serialization%7Bname%7D"));
	}

	#[tokio::test]
	async fn fetch_series_metadata_maps_status_and_publisher() {
		let server = MockServer::spawn(vec![render_ok(&detail_response_body())]);
		let client = test_client(&server);

		let series = client
			.fetch_series_metadata("1")
			.await
			.expect("fetch should succeed against the mock");

		assert_eq!(series.external_id, "1");
		assert_eq!(series.title, "Berserk");
		assert_eq!(series.status, Some(PublicationStatus::Ongoing));
		assert_eq!(series.year, Some(1989));
		assert_eq!(series.publisher.as_deref(), Some("Young Animal"));
		assert_eq!(series.summary.as_deref(), Some(media_synopsis()));
		assert_eq!(
			series.authors.as_deref(),
			Some(["Kentaro Miura".to_string(), "Story Only".to_string()].as_slice())
		);
	}

	fn media_synopsis() -> &'static str {
		"Guts, a mercenary, travels with the Band of the Hawk."
	}

	#[tokio::test]
	async fn short_queries_skip_the_network() {
		let server = MockServer::spawn(vec![render_ok(&search_response_body())]);
		let client = test_client(&server);

		let query = SearchQuery {
			title: "ab".to_string(),
			..Default::default()
		};
		let outcome = client
			.search_media(&query)
			.await
			.expect("short queries should not error");

		assert_eq!(outcome.requested, 0);
		assert!(outcome.candidates.is_empty());
		assert!(server.requests().is_empty());
	}

	/// Render a non-200 response with an empty body.
	fn render_status(status: u16, reason: &str) -> String {
		format!(
			"HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
		)
	}

	#[tokio::test]
	async fn http_429_maps_to_rate_limited() {
		let server = MockServer::spawn(vec![render_status(429, "Too Many Requests")]);
		let client = test_client(&server);

		let error = client
			.search_media(&SearchQuery {
				title: "Berserk".to_string(),
				..Default::default()
			})
			.await
			.expect_err("429 should surface as an error");

		assert!(error.is_rate_limited());
	}

	#[tokio::test]
	async fn malformed_json_maps_to_parse_error() {
		let server = MockServer::spawn(vec![render_ok("this is not json")]);
		let client = test_client(&server);

		let error = client
			.search_media(&SearchQuery {
				title: "Berserk".to_string(),
				..Default::default()
			})
			.await
			.expect_err("malformed JSON should surface as an error");

		assert!(matches!(error, MetadataProviderError::ParseError(_)));
		assert!(!error.is_rate_limited());
	}

	#[tokio::test]
	async fn invalid_external_id_fails_without_network() {
		let server = MockServer::spawn(vec![render_ok(&detail_response_body())]);
		let client = test_client(&server);

		let error = client
			.fetch_media_metadata("not-a-number")
			.await
			.expect_err("invalid ids should be rejected");

		assert!(matches!(
			error,
			MetadataProviderError::Other(message) if message.contains("not-a-number")
		));
		assert!(server.requests().is_empty());
	}

	#[ignore = "Requires MAL_CLIENT_ID env var"]
	#[tokio::test]
	async fn live_search() {
		dotenvy::dotenv().ok();
		let client_id = std::env::var("MAL_CLIENT_ID").expect("MAL_CLIENT_ID not set");
		let client = MalClient::new(client_id, None);

		let outcome = client
			.search_media(&SearchQuery {
				title: "Frieren".to_string(),
				limit: Some(5),
				..Default::default()
			})
			.await
			.expect("live search should succeed");

		assert!(!outcome.candidates.is_empty());
	}
}
