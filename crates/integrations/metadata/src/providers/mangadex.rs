//! MangaDex metadata provider client (https://api.mangadex.org).
//!
//! MangaDex has no per-volume issues: a `/manga` entry is a series, so both
//! the media and series search paths hit the same `GET /manga` endpoint and
//! differ only in how results are represented (the search payload already
//! carries the full attributes, so no per-hit detail fetch is needed).
//! Behaviour mirrors Komf's `MangaDexMetadataMapper`: prefer the `en` title
//! (falling back to a `ja-ro` alt title, then the first title), map `genre`
//! tag groups to genres, `theme` tag groups plus the publication demographic
//! to tags, and `author`/`artist` relationships to writers/artists.

use std::collections::HashMap;

use reqwest::Url;
use reqwest_middleware::ClientWithMiddleware;
use serde::Deserialize;

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	error::MetadataProviderError,
	provider::ProviderCredentialVerification,
	rate_limit::RateLimiter,
	types::{
		ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate, MediaType,
		PublicationStatus, SearchQuery,
	},
	ExternalMetadata, MetadataProvider, SearchOutcome,
};

/// MangaDex allows 5 req/s; run slightly under it (see provider wiring notes).
const MANGADEX_DEFAULT_RATE_LIMIT: u32 = 4;

/// MangaDex caps `limit` at 100 for the manga search endpoint.
const MANGADEX_MAX_LIMIT: u32 = 100;

/// Komf truncates search titles at 400 characters; the API rejects longer
/// `title` parameters.
const MANGADEX_MAX_TITLE_LEN: usize = 400;

pub struct MangaDexClient {
	client: ClientWithMiddleware,
	api_url: String,
	rate_limiter: RateLimiter,
}

impl Default for MangaDexClient {
	fn default() -> Self {
		Self::new()
	}
}

impl MangaDexClient {
	const API_URL: &'static str = "https://api.mangadex.org";
	const COVER_BASE_URL: &'static str = "https://uploads.mangadex.org/covers";
	const USER_AGENT: &'static str = concat!(
		"stump/",
		env!("CARGO_PKG_VERSION"),
		" (+https://github.com/stumpapp/stump)"
	);

	/// MangaDex requires no API key, so there is nothing to configure.
	pub fn new() -> Self {
		Self::build(
			Self::API_URL.to_string(),
			MANGADEX_DEFAULT_RATE_LIMIT,
			RetryClientConfig::default(),
		)
	}

	fn build(api_url: String, rate_limit: u32, retry: RetryClientConfig) -> Self {
		let inner = reqwest::Client::builder()
			.user_agent(Self::USER_AGENT)
			.build()
			.expect("Failed to build MangaDex HTTP client"); // static config, cannot fail
		Self {
			client: build_client_with_retry(inner, retry),
			api_url,
			rate_limiter: RateLimiter::new(rate_limit),
		}
	}

	/// Test-only override of the API base URL, used to point the client at a
	/// local mock server.
	#[cfg(test)]
	fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
		self.api_url = api_url.into();
		self
	}

	/// Test-only: disable the retry middleware so a canned 429 surfaces
	/// immediately instead of after the exponential backoff schedule.
	fn without_retries(mut self) -> Self {
		self.client = build_client_with_retry(
			reqwest::Client::builder()
				.user_agent(Self::USER_AGENT)
				.build()
				.expect("Failed to build MangaDex HTTP client"),
			RetryClientConfig { max_retries: 0 },
		);
		self.rate_limiter = RateLimiter::new(u32::MAX);
		self
	}

	async fn get<T: serde::de::DeserializeOwned>(
		&self,
		path: &str,
		params: &[(&str, &str)],
	) -> Result<T, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let mut url = Url::parse(&format!("{}{}", self.api_url, path))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;

		for (key, value) in params {
			url.query_pairs_mut().append_pair(key, value);
		}

		let response = self.client.get(url).send().await?.error_for_status()?;
		Ok(response.json().await?)
	}

	/// Search the manga catalog. The response embeds the full attributes
	/// (title, tags, relationships), so the caller maps results directly.
	async fn search_manga(
		&self,
		title: &str,
		limit: u32,
	) -> Result<Vec<MangaDexManga>, MetadataProviderError> {
		let limit = limit.clamp(1, MANGADEX_MAX_LIMIT).to_string();
		let title: String = title.chars().take(MANGADEX_MAX_TITLE_LEN).collect();
		let params = [
			("limit", limit.as_str()),
			("title", title.as_str()),
			("includes[]", "author"),
			("includes[]", "artist"),
			("includes[]", "cover_art"),
			("order[relevance]", "desc"),
			("contentRating[]", "safe"),
			("contentRating[]", "suggestive"),
			("contentRating[]", "erotica"),
			("contentRating[]", "pornographic"),
		];
		let response: MangaDexResponse<Vec<MangaDexManga>> =
			self.get("/manga", &params).await?;
		Ok(response.data)
	}

	/// Fetch one manga by id with the same relationship includes as search.
	async fn fetch_manga(
		&self,
		external_id: &str,
	) -> Result<MangaDexManga, MetadataProviderError> {
		let params = [
			("includes[]", "author"),
			("includes[]", "artist"),
			("includes[]", "cover_art"),
		];
		let path = format!("/manga/{}", external_id);
		let response: MangaDexResponse<MangaDexManga> = self.get(&path, &params).await?;
		Ok(response.data)
	}
}

#[async_trait::async_trait]
impl MetadataProvider for MangaDexClient {
	fn id(&self) -> &'static str {
		"mangadex"
	}

	fn name(&self) -> &'static str {
		"MangaDex"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Manga, MediaType::Manhwa]
	}

	#[tracing::instrument(skip(self))]
	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let results = self
			.search_manga(&query.title, query.limit.unwrap_or(10))
			.await?;

		let candidates = results
			.iter()
			.map(|manga| MatchCandidate {
				external_id: manga.id.clone(),
				metadata: ExternalMetadata::Series(series_metadata(self.id(), manga)),
				provider: self.id().to_string(),
				confidence: 0.0,
				confidence_factors: Vec::new(),
			})
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested: 1,
		})
	}

	#[tracing::instrument(skip(self))]
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let results = self
			.search_manga(&query.title, query.limit.unwrap_or(10))
			.await?;
		let requested = results.len();

		// A MangaDex entry is a series, but the editor's search flow resolves
		// media-level candidates only (see `search_hit_from_match`), so each
		// result is represented at media level with the series carried along.
		let candidates = results
			.iter()
			.map(|manga| MatchCandidate {
				external_id: manga.id.clone(),
				metadata: ExternalMetadata::Media(media_metadata(self.id(), manga)),
				provider: self.id().to_string(),
				confidence: 0.0,
				confidence_factors: Vec::new(),
			})
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	async fn fetch_series_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		let manga = self.fetch_manga(external_id).await?;
		Ok(series_metadata(self.id(), &manga))
	}

	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let manga = self.fetch_manga(external_id).await?;
		Ok(media_metadata(self.id(), &manga))
	}

	/// MangaDex has no credentials to verify; a successful ping proves the
	/// API is reachable.
	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		self.rate_limiter.until_ready().await;
		let response = self
			.client
			.get(format!("{}/ping", self.api_url))
			.send()
			.await?;
		let status = response.status().as_u16();
		let is_valid = response.status().is_success();
		Ok(ProviderCredentialVerification {
			response_status: status,
			is_valid,
			error: (!is_valid).then(|| format!("unexpected status {status}")),
		})
	}
}

/// Primary display title: prefer `en`, then a `ja-ro` alt title, then the
/// first declared title, then the first alt title.
fn preferred_title(manga: &MangaDexManga) -> Option<String> {
	let attributes = &manga.attributes;
	attributes
		.title
		.get("en")
		.or_else(|| {
			attributes
				.alt_titles
				.iter()
				.find_map(|title| title.get("ja-ro"))
		})
		.or_else(|| attributes.title.values().next())
		.or_else(|| {
			attributes
				.alt_titles
				.iter()
				.find_map(|title| title.values().next())
		})
		.cloned()
}

/// Every alt title except the chosen primary, in declaration order.
fn alternative_titles(manga: &MangaDexManga) -> Vec<String> {
	let primary = preferred_title(manga);
	let mut titles = Vec::new();
	for title in &manga.attributes.alt_titles {
		for value in title.values() {
			if Some(value) != primary.as_ref() && !titles.contains(value) {
				titles.push(value.clone());
			}
		}
	}
	titles
}

fn localized_tag_names(manga: &MangaDexManga, group: &str) -> Vec<String> {
	manga
		.attributes
		.tags
		.iter()
		.filter(|tag| tag.attributes.group == group)
		.filter_map(|tag| {
			tag.attributes
				.name
				.get("en")
				.or_else(|| tag.attributes.name.values().next())
				.cloned()
		})
		.collect()
}

fn status(manga: &MangaDexManga) -> Option<PublicationStatus> {
	match manga.attributes.status.as_deref() {
		Some("ongoing") => Some(PublicationStatus::Ongoing),
		Some("completed") => Some(PublicationStatus::Completed),
		Some("hiatus") => Some(PublicationStatus::Hiatus),
		Some("cancelled") => Some(PublicationStatus::Cancelled),
		_ => None,
	}
}

fn names_of<'a>(
	manga: &'a MangaDexManga,
	kind: fn(&MangaDexRelationship) -> bool,
) -> Vec<String> {
	manga
		.relationships
		.iter()
		.filter(|relationship| kind(relationship))
		.filter_map(|relationship| match relationship {
			MangaDexRelationship::Author { attributes }
			| MangaDexRelationship::Artist { attributes } => attributes.name.clone(),
			_ => None,
		})
		.collect()
}

fn cover_url(manga: &MangaDexManga) -> Option<String> {
	manga
		.relationships
		.iter()
		.find_map(|relationship| match relationship {
			MangaDexRelationship::CoverArt { attributes } => Some(format!(
				"{}/{}/{}",
				MangaDexClient::COVER_BASE_URL,
				manga.id,
				attributes.file_name
			)),
			_ => None,
		})
}

fn summary(manga: &MangaDexManga) -> Option<String> {
	manga
		.attributes
		.description
		.get("en")
		.or_else(|| manga.attributes.description.values().next())
		.cloned()
}

fn series_metadata(provider_id: &str, manga: &MangaDexManga) -> ExternalSeriesMetadata {
	ExternalSeriesMetadata {
		provider: provider_id.to_string(),
		external_id: manga.id.clone(),
		title: preferred_title(manga).unwrap_or_default(),
		alternative_titles: alternative_titles(manga),
		summary: summary(manga),
		status: status(manga),
		year: manga.attributes.year,
		genres: filled_or_none(localized_tag_names(manga, "genre")),
		tags: {
			let mut tags = localized_tag_names(manga, "theme");
			if let Some(demographic) = &manga.attributes.publication_demographic {
				tags.push(demographic.to_lowercase());
			}
			filled_or_none(tags)
		},
		authors: filled_or_none(names_of(manga, |relationship| {
			matches!(relationship, MangaDexRelationship::Author { .. })
		})),
		artists: filled_or_none(names_of(manga, |relationship| {
			matches!(relationship, MangaDexRelationship::Artist { .. })
		})),
		cover_url: cover_url(manga),
		..Default::default()
	}
}

fn media_metadata(provider_id: &str, manga: &MangaDexManga) -> ExternalMediaMetadata {
	let title = preferred_title(manga);
	ExternalMediaMetadata {
		provider: provider_id.to_string(),
		external_id: manga.id.clone(),
		title: title.clone(),
		summary: summary(manga),
		series_name: title,
		series_external_id: Some(manga.id.clone()),
		year: manga.attributes.year,
		genres: filled_or_none(localized_tag_names(manga, "genre")),
		tags: {
			let mut tags = localized_tag_names(manga, "theme");
			if let Some(demographic) = &manga.attributes.publication_demographic {
				tags.push(demographic.to_lowercase());
			}
			filled_or_none(tags)
		},
		writers: filled_or_none(names_of(manga, |relationship| {
			matches!(relationship, MangaDexRelationship::Author { .. })
		})),
		artists: filled_or_none(names_of(manga, |relationship| {
			matches!(relationship, MangaDexRelationship::Artist { .. })
		})),
		cover_url: cover_url(manga),
		provider_url: Some(format!("https://mangadex.org/title/{}", manga.id)),
		..Default::default()
	}
}

fn filled_or_none(values: Vec<String>) -> Option<Vec<String>> {
	(!values.is_empty()).then(|| {
		let mut values = values;
		values.dedup();
		values
	})
}

#[derive(Debug, Deserialize)]
struct MangaDexResponse<T> {
	data: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MangaDexManga {
	id: String,
	#[serde(default)]
	attributes: MangaDexAttributes,
	#[serde(default)]
	relationships: Vec<MangaDexRelationship>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MangaDexAttributes {
	#[serde(default)]
	title: HashMap<String, String>,
	#[serde(default)]
	alt_titles: Vec<HashMap<String, String>>,
	#[serde(default)]
	description: HashMap<String, String>,
	#[serde(default)]
	links: Option<HashMap<String, String>>,
	original_language: Option<String>,
	publication_demographic: Option<String>,
	status: Option<String>,
	year: Option<i32>,
	#[serde(default)]
	tags: Vec<MangaDexTag>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum MangaDexRelationship {
	#[serde(rename = "author")]
	Author {
		attributes: MangaDexPersonAttributes,
	},
	#[serde(rename = "artist")]
	Artist {
		attributes: MangaDexPersonAttributes,
	},
	#[serde(rename = "cover_art")]
	CoverArt {
		attributes: MangaDexCoverArtAttributes,
	},
	#[serde(other)]
	Unknown,
}

#[derive(Debug, Deserialize)]
struct MangaDexPersonAttributes {
	name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MangaDexCoverArtAttributes {
	file_name: String,
	#[serde(default)]
	volume: Option<String>,
	#[serde(default)]
	locale: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MangaDexTag {
	attributes: MangaDexTagAttributes,
}

#[derive(Debug, Deserialize)]
struct MangaDexTagAttributes {
	name: HashMap<String, String>,
	group: String,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::mock_http::{render_ok, MockServer};

	fn search_response() -> String {
		serde_json::json!({
			"result": "ok",
			"response": "collection",
			"data": [{
				"id": "a96676e5-8ae2-425e-b549-7f15dd34a6d8",
				"type": "manga",
				"attributes": {
					"title": { "en": "Solo Leveling" },
					"altTitles": [
						{ "ja-ro": "Ore dake Level Up" },
						{ "en": "Solo Leveling (Official)" }
					],
					"description": { "en": "E-rank hunter Sung Jin-Woo..." },
					"links": { "al": "78101", "mal": "107573" },
					"originalLanguage": "ko",
					"publicationDemographic": "shounen",
					"status": "ongoing",
					"year": 2018,
					"tags": [
						{ "id": "t1", "type": "tag", "attributes": { "name": { "en": "Action" }, "group": "genre" } },
						{ "id": "t2", "type": "tag", "attributes": { "name": { "en": "Monsters" }, "group": "theme" } }
					]
				},
				"relationships": [
					{ "id": "r1", "type": "author", "attributes": { "name": "Chugong" } },
					{ "id": "r2", "type": "artist", "attributes": { "name": "DUBU" } },
					{ "id": "r3", "type": "cover_art", "attributes": { "fileName": "a96676e5.jpg", "volume": "1", "locale": "en" } }
				]
			}],
			"limit": 10,
			"offset": 0,
			"total": 1
		})
		.to_string()
	}

	fn query(title: &str) -> SearchQuery {
		SearchQuery {
			title: title.to_string(),
			limit: Some(10),
			..Default::default()
		}
	}

	#[tokio::test]
	async fn search_media_maps_mocked_search_response() {
		let server = MockServer::spawn(vec![render_ok(&search_response())]);
		let client = MangaDexClient::new().with_api_url(server.url.clone());

		let outcome = client
			.search_media(&query("Solo Leveling"))
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "mangadex");
		assert_eq!(
			candidate.external_id,
			"a96676e5-8ae2-425e-b549-7f15dd34a6d8"
		);

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(media.title.as_deref(), Some("Solo Leveling"));
		assert_eq!(
			media.summary.as_deref(),
			Some("E-rank hunter Sung Jin-Woo...")
		);
		assert_eq!(media.year, Some(2018));
		assert_eq!(
			media.writers.as_deref(),
			Some(["Chugong".to_string()].as_slice())
		);
		assert_eq!(
			media.artists.as_deref(),
			Some(["DUBU".to_string()].as_slice())
		);
		assert_eq!(
			media.genres.as_deref(),
			Some(["Action".to_string()].as_slice())
		);
		assert_eq!(
			media.tags.as_deref(),
			Some(["Monsters".to_string(), "shounen".to_string()].as_slice())
		);
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://uploads.mangadex.org/covers/a96676e5-8ae2-425e-b549-7f15dd34a6d8/a96676e5.jpg")
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://mangadex.org/title/a96676e5-8ae2-425e-b549-7f15dd34a6d8")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		let request = &requests[0];
		assert!(request.starts_with("GET /manga?"), "{request}");
		assert!(request.contains("limit=10"), "{request}");
		assert!(request.contains("title=Solo+Leveling"), "{request}");
		assert!(request.contains("includes%5B%5D=cover_art"), "{request}");
		assert!(request.contains("includes%5B%5D=author"), "{request}");
		assert!(request.contains("includes%5B%5D=artist"), "{request}");
		assert!(request.contains("order%5Brelevance%5D=desc"), "{request}");
		assert!(request.contains("contentRating%5B%5D=safe"), "{request}");
	}

	#[tokio::test]
	async fn search_series_maps_results_as_series_candidates() {
		let server = MockServer::spawn(vec![render_ok(&search_response())]);
		let client = MangaDexClient::new().with_api_url(server.url.clone());

		let outcome = client
			.search_series(&query("Solo Leveling"))
			.await
			.expect("series search should succeed against the mock");

		assert_eq!(outcome.candidates.len(), 1);
		let series = outcome.candidates[0].metadata.as_series().unwrap();
		assert_eq!(series.title, "Solo Leveling");
		// primary title excluded from alternatives; the remaining two stay
		assert_eq!(
			series.alternative_titles,
			vec![
				"Ore dake Level Up".to_string(),
				"Solo Leveling (Official)".to_string()
			]
		);
		assert_eq!(series.status, Some(PublicationStatus::Ongoing));
		assert_eq!(series.year, Some(2018));
		assert_eq!(series.cover_url.as_deref(), media_cover_url());
	}

	fn media_cover_url() -> Option<&'static str> {
		Some(
			"https://uploads.mangadex.org/covers/a96676e5-8ae2-425e-b549-7f15dd34a6d8/a96676e5.jpg",
		)
	}

	fn detail_response() -> String {
		serde_json::json!({
			"result": "ok",
			"response": "entity",
			"data": {
				"id": "a96676e5-8ae2-425e-b549-7f15dd34a6d8",
				"type": "manga",
				"attributes": {
					"title": { "en": "Solo Leveling" },
					"altTitles": [ { "ja-ro": "Ore dake Level Up" } ],
					"description": { "en": "E-rank hunter Sung Jin-Woo..." },
					"originalLanguage": "ko",
					"publicationDemographic": "shounen",
					"status": "completed",
					"year": 2018,
					"tags": [
						{ "id": "t1", "type": "tag", "attributes": { "name": { "en": "Action" }, "group": "genre" } },
						{ "id": "t2", "type": "tag", "attributes": { "name": { "en": "Monsters" }, "group": "theme" } }
					]
				},
				"relationships": [
					{ "id": "r1", "type": "author", "attributes": { "name": "Chugong" } },
					{ "id": "r2", "type": "artist", "attributes": { "name": "DUBU" } },
					{ "id": "r3", "type": "cover_art", "attributes": { "fileName": "a96676e5.jpg", "volume": "1", "locale": "en" } }
				]
			}
		})
		.to_string()
	}

	#[tokio::test]
	async fn fetch_metadata_maps_mocked_detail_response() {
		let server = MockServer::spawn(vec![
			render_ok(&detail_response()),
			render_ok(&detail_response()),
		]);
		let client = MangaDexClient::new().with_api_url(server.url.clone());
		let id = "a96676e5-8ae2-425e-b549-7f15dd34a6d8";

		let series = client
			.fetch_series_metadata(id)
			.await
			.expect("series lookup should succeed against the mock");
		assert_eq!(series.external_id, id);
		assert_eq!(series.title, "Solo Leveling");

		let media = client
			.fetch_media_metadata(id)
			.await
			.expect("media lookup should succeed against the mock");
		assert_eq!(media.series_name.as_deref(), Some("Solo Leveling"));

		let requests = server.requests();
		assert_eq!(requests.len(), 2);
		assert!(
			requests[0].starts_with(&format!("GET /manga/{id}?")),
			"{}",
			requests[0]
		);
		assert!(
			requests[0].contains("includes%5B%5D=cover_art"),
			"{}",
			requests[0]
		);
		assert!(
			requests[1].starts_with(&format!("GET /manga/{id}?")),
			"{}",
			requests[1]
		);
	}

	#[tokio::test]
	async fn missing_title_falls_back_to_ja_ro_alt_title() {
		let body = serde_json::json!({
			"result": "ok",
			"data": {
				"id": "id-1",
				"attributes": {
					"title": { "ja": "元タイトル" },
					"altTitles": [{ "ja-ro": "Gen Title" }]
				},
				"relationships": []
			}
		})
		.to_string();
		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = MangaDexClient::new().with_api_url(server.url.clone());

		let series = client
			.fetch_series_metadata("id-1")
			.await
			.expect("lookup should succeed");
		assert_eq!(series.title, "Gen Title");
		assert_eq!(series.alternative_titles, Vec::<String>::new());
	}

	#[tokio::test]
	async fn rate_limited_response_maps_to_rate_limited_error() {
		let too_many = "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let server = MockServer::spawn(vec![too_many.to_string()]);
		let client = MangaDexClient::new()
			.with_api_url(server.url.clone())
			.without_retries();

		let error = client
			.search_media(&query("Solo Leveling"))
			.await
			.expect_err("429 should surface as an error");

		assert!(
			error.is_rate_limited(),
			"expected rate limited, got {error:?}"
		);
	}

	#[tokio::test]
	async fn malformed_json_surfaces_as_non_rate_limited_request_error() {
		let body = r#"HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: 15
Connection: close

{"result": "ok""#
			.replace('\n', "\r\n");
		let server = MockServer::spawn(vec![body]);
		let client = MangaDexClient::new().with_api_url(server.url.clone());

		let error = client
			.search_media(&query("Solo Leveling"))
			.await
			.expect_err("malformed JSON should surface as an error");

		// reqwest wraps JSON decode failures in a Decode error; the ingest
		// facade maps every non-rate-limit failure to ProviderError::Request.
		assert!(
			!error.is_rate_limited(),
			"malformed JSON must not surface as rate limited, got {error:?}"
		);
		assert!(
			matches!(&error, MetadataProviderError::ReqwestError(err) if err.is_decode()),
			"expected decode error, got {error:?}"
		);
	}

	#[tokio::test]
	async fn search_results_are_scored_against_the_query() {
		let server = MockServer::spawn(vec![render_ok(&search_response())]);
		let client = MangaDexClient::new().with_api_url(server.url.clone());

		let outcome = client
			.search_media(&query("Solo Leveling"))
			.await
			.expect("search should succeed");

		assert!(
			outcome.candidates[0].confidence > 0.0,
			"scorer should have assigned a positive confidence"
		);
	}

	#[ignore = "Requires network access"]
	#[tokio::test]
	async fn live_search_and_lookup() {
		let client = MangaDexClient::new();
		let outcome = client
			.search_media(&query("Solo Leveling"))
			.await
			.expect("live search should succeed");
		assert!(!outcome.candidates.is_empty());

		let id = outcome.candidates[0].external_id.clone();
		let media = client
			.fetch_media_metadata(&id)
			.await
			.expect("live lookup should succeed");
		assert!(!media.title.as_deref().unwrap_or_default().is_empty());
	}
}
