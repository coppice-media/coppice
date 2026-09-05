use http_cache_reqwest::{Cache, CacheMode, HttpCache, HttpCacheOptions, MokaManager};
use reqwest::Url;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	error::MetadataProviderError,
	provider::ProviderCredentialVerification,
	providers::comic_vine::utils::{
		extract_issue_id, filled_array_or_none, parse_date_parts,
	},
	types::{
		ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate, MediaType,
		SearchQuery,
	},
	ExternalMetadata, MetadataProvider, RateLimiter, SearchOutcome,
};

use super::{
	types::{
		ComicVineIssuesFilter, ComicVinePrefix, ComicVineResponse, IssueDetail,
		IssueResult, VolumeDetail, VolumeResult,
	},
	utils::filter_credits_by_role,
};

/// The official rate limit is 200 req/hour (see https://comicvine.gamespot.com/api/).
/// Honestly that is quite low. I chose 1 req/sec to be a bit more cautious, but
/// that will easily exceed the hourly limit for even small libraries
const COMIC_VINE_DEFAULT_RATE_LIMIT: u32 = 1;

pub struct ComicVineClient {
	client: ClientWithMiddleware,
	api_key: String,
	api_url: String,
	rate_limiter: RateLimiter,
}

impl ComicVineClient {
	const API_URL: &'static str = "https://comicvine.gamespot.com/api";

	pub fn new(api_key: String, rate_limit: Option<u32>) -> Self {
		let inner = reqwest::Client::builder()
			// comic vine requires a user-agent, rejects with a 403. ask me how i know
			.user_agent(concat!(
				env!("CARGO_PKG_NAME"),
				"/",
				env!("CARGO_PKG_VERSION")
			))
			.build()
			.expect("Failed to build ComicVine HTTP client"); // this should never really happen
		let with_retry = build_client_with_retry(inner, RetryClientConfig::default());

		let with_cache = ClientBuilder::from_client(with_retry)
			.with(
				// the rec in their api docs is to cache responses becaues of the low rate limit
				Cache(HttpCache {
					mode: CacheMode::Default,
					manager: MokaManager::default(),
					options: HttpCacheOptions::default(),
				}),
			)
			.build();

		Self {
			client: with_cache,
			api_key,
			api_url: Self::API_URL.to_string(),
			rate_limiter: RateLimiter::new(
				rate_limit.unwrap_or(COMIC_VINE_DEFAULT_RATE_LIMIT),
			),
		}
	}

	/// Test-only override of the API base URL, used to point the client at a
	/// local mock server.
	#[cfg(test)]
	fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
		self.api_url = api_url.into();
		self
	}

	/// Send a GET request to the ComicVine API
	///
	/// - `path` should start with a `/`, e.g. `/volumes/`.
	/// - `params` are appended as additional query parameters
	async fn get<T: serde::de::DeserializeOwned>(
		&self,
		path: &str,
		params: &[(&str, &str)],
	) -> Result<T, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let mut url = Url::parse(&format!("{}{}", self.api_url, path))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;

		url.query_pairs_mut()
			.append_pair("api_key", &self.api_key)
			.append_pair("format", "json");

		for (k, v) in params {
			url.query_pairs_mut().append_pair(k, v);
		}

		let response = self.client.get(url).send().await?.error_for_status()?;
		let data: ComicVineResponse<T> = response.json().await?;

		if data.status_code != 1 {
			return Err(MetadataProviderError::Other(format!(
				"ComicVine API error (code {}): {}",
				data.status_code, data.error
			)));
		}

		data.results.ok_or(MetadataProviderError::EmptyResponse)
	}

	#[tracing::instrument(skip(self))]
	async fn search_volumes(
		&self,
		query: &str,
		limit: u32,
	) -> Result<Vec<VolumeResult>, MetadataProviderError> {
		let limit_str = limit.to_string();
		let params = [
			("query", query),
			("resources", "volume"),
			(
				"field_list",
				"id,name,description,start_year,publisher,image,count_of_issues,deck",
			),
			("limit", limit_str.as_str()),
		];
		self.get::<Vec<VolumeResult>>("/search/", &params).await
	}

	#[tracing::instrument(skip(self))]
	async fn search_issues(
		&self,
		query: &str,
		limit: u32,
	) -> Result<Vec<IssueResult>, MetadataProviderError> {
		let limit_str = limit.to_string();
		let params = vec![
			("query", query),
			("resources", "issue"),
			("field_list", "id,name,description,issue_number,volume,cover_date,image,person_credits,character_credits"),
			("limit", limit_str.as_str()),
		];

		self.get::<Vec<IssueResult>>("/search/", &params).await
	}

	/// queries the `/issues/` endpoint with filters, which is more precise than search when you know
	/// specific criteria like volume ID or issue number
	#[tracing::instrument(skip(self))]
	async fn filter_issues(
		&self,
		filter: &str,
		limit: u32,
	) -> Result<Vec<IssueResult>, MetadataProviderError> {
		let limit_str = limit.to_string();
		let params = vec![
			("field_list", "id,name,description,issue_number,volume,cover_date,image,person_credits,character_credits"),
			("limit", limit_str.as_str()),
			("filter", filter),
		];

		self.get::<Vec<IssueResult>>("/issues/", &params).await
	}

	async fn fetch_volume(
		&self,
		id: &str,
	) -> Result<VolumeDetail, MetadataProviderError> {
		let params = [(
			"field_list",
			"id,name,description,start_year,publisher,image,count_of_issues,people,issues",
		)];
		self.get::<VolumeDetail>(
			&format!("/volume/{}-{}/", u32::from(ComicVinePrefix::Volume), id),
			&params,
		)
		.await
	}

	async fn fetch_issue(&self, id: &str) -> Result<IssueDetail, MetadataProviderError> {
		let params = [(
			"field_list",
			"id,name,description,issue_number,volume,cover_date,image,person_credits,character_credits",
		)];
		self.get::<IssueDetail>(
			&format!("/issue/{}-{}/", u32::from(ComicVinePrefix::Issue), id),
			&params,
		)
		.await
	}

	/// Attempt a direct issue lookup using the `comic_vine_volume_id` provider hint, if present
	async fn try_hint_issue_lookup(&self, query: &SearchQuery) -> Option<MatchCandidate> {
		let volume_id = query.provider_hints.get("comic_vine_volume_id")?;
		let number = query.number?;

		let volume = match self.fetch_volume(volume_id).await {
			Ok(v) => v,
			Err(e) => {
				tracing::warn!(
					volume_id,
					error = ?e,
					"Volume fetch via hint failed, falling back to search"
				);
				return None;
			},
		};

		let issue_id = extract_issue_id(&volume.issues?, number)?;

		match self.fetch_media_metadata(&issue_id).await {
			Ok(metadata) => Some(MatchCandidate {
				external_id: issue_id,
				metadata: ExternalMetadata::Media(metadata),
				provider: self.id().to_string(),
				confidence: 1.0,
				confidence_factors: Vec::new(),
			}),
			Err(e) => {
				tracing::warn!(
					issue_id = issue_id,
					error = ?e,
					"Direct issue fetch via hint failed, falling back to search"
				);
				None
			},
		}
	}
}

#[async_trait::async_trait]
impl MetadataProvider for ComicVineClient {
	fn id(&self) -> &'static str {
		"comic_vine"
	}

	fn name(&self) -> &'static str {
		"ComicVine"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Comic]
	}

	#[tracing::instrument(skip(self))]
	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		// happy path
		if let Some(volume_id) = query.provider_hints.get("comic_vine_volume_id") {
			tracing::debug!(
				volume_id,
				"Using comic_vine_volume_id hint for direct fetch"
			);
			match self.fetch_series_metadata(volume_id).await {
				Ok(metadata) => {
					return Ok(SearchOutcome {
						candidates: self.score_search(
							query,
							vec![MatchCandidate {
								external_id: volume_id.clone(),
								metadata: ExternalMetadata::Series(metadata),
								provider: self.id().to_string(),
								confidence: 1.0,
								confidence_factors: Vec::new(),
							}],
						),
						requested: 1,
					});
				},
				Err(e) => {
					// TODO: i think this would warrant a persisted log, since unless the request was just
					// rate limited or a spurious network error happened, it would indicate the id stored is
					// not valid
					tracing::warn!(
						volume_id,
						error = ?e,
						"Direct volume fetch via hint failed, falling back to search"
					);
				},
			}
		}

		tracing::trace!("Searching for volumes on ComicVine");
		let results = self
			.search_volumes(&query.title, query.limit.unwrap_or(10))
			.await?;

		let mut candidates = Vec::with_capacity(results.len());
		for result in results {
			let external_id = result.id.to_string();
			match self.fetch_series_metadata(&external_id).await {
				Ok(metadata) => {
					tracing::trace!(external_id, "Fetched volume metadata successfully");
					candidates.push(MatchCandidate {
						external_id,
						metadata: ExternalMetadata::Series(metadata),
						provider: self.id().to_string(),
						confidence: 0.0,
						confidence_factors: Vec::new(),
					});
				},
				Err(e) => {
					tracing::error!(
						external_id,
						error = ?e,
						"Failed to fetch volume metadata for search result"
					);
				},
			}
		}

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
		if let Some(candidate) = self.try_hint_issue_lookup(query).await {
			return Ok(SearchOutcome {
				candidates: self.score_search(query, vec![candidate]),
				requested: 1,
			});
		}

		tracing::trace!("Searching for issues on ComicVine");

		let filter = ComicVineIssuesFilter::from(query);
		let filter_str = filter.to_filter_string();

		let results = if let Some(ref filter) = filter_str {
			tracing::debug!(?filter, "Using ComicVine /issues/ endpoint with filter");
			self.filter_issues(filter, query.limit.unwrap_or(10))
				.await?
		} else {
			tracing::debug!("Using ComicVine /search/ endpoint");
			self.search_issues(&query.title, query.limit.unwrap_or(10))
				.await?
		};

		let requested = results.len();

		let mut candidates = Vec::with_capacity(results.len());
		for result in results {
			let external_id = result.id.to_string();
			match self.fetch_media_metadata(&external_id).await {
				Ok(metadata) => {
					tracing::trace!(external_id, "Fetched issue metadata successfully");
					candidates.push(MatchCandidate {
						external_id,
						metadata: ExternalMetadata::Media(metadata),
						provider: self.id().to_string(),
						confidence: 0.0,
						confidence_factors: Vec::new(),
					});
				},
				Err(e) => {
					// TODO: persisted log?
					tracing::error!(
						external_id,
						error = ?e,
						"Failed to fetch issue metadata for search result"
					);
				},
			}
		}

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	async fn fetch_series_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		let volume = self.fetch_volume(external_id).await?;

		let people = volume.people.unwrap_or_default();
		let authors = filter_credits_by_role(&people, &["writer"]);
		// i took the largest one, but perhaps that isn't ideal. i think it's probably fine
		let cover_url = volume.image.and_then(|img| img.super_url);

		Ok(ExternalSeriesMetadata {
			provider: self.id().to_string(),
			external_id: volume.id.to_string(),
			title: volume.name.unwrap_or_default(),
			alternative_titles: vec![],
			summary: volume.deck.or(volume.description),
			year: volume.start_year.and_then(|y| y.parse().ok()),
			publisher: volume.publisher.and_then(|p| p.name),
			authors: filled_array_or_none(authors),
			volume_count: volume.count_of_issues,
			cover_url,
			..Default::default()
		})
	}

	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let issue = self.fetch_issue(external_id).await?;

		let cover_url = issue.image.and_then(|img| img.super_url);

		let number = issue
			.issue_number
			.as_deref()
			.and_then(|n| n.parse::<f32>().ok());

		let (year, month, day) = issue
			.cover_date
			.as_deref()
			.map(parse_date_parts)
			.unwrap_or((None, None, None));

		let credits = issue.person_credits.unwrap_or_default();
		let writers =
			filter_credits_by_role(&credits, &["writer", "plotter", "scripter"]);
		let artists = filter_credits_by_role(
			&credits,
			&["penciler", "penciller", "breakdowns", "inker", "finishes"],
		);
		let colorists = filter_credits_by_role(
			&credits,
			&["colorist", "colourist", "colorer", "colourer"],
		);
		let letterers = filter_credits_by_role(&credits, &["letterer"]);
		let cover_artists =
			filter_credits_by_role(&credits, &["cover", "coverartist", "cover artist"]);

		let series_name = issue.volume.as_ref().and_then(|v| v.name.clone());
		let series_external_id = issue.volume.as_ref().map(|v| v.id.to_string());

		Ok(ExternalMediaMetadata {
			provider: self.id().to_string(),
			external_id: issue.id.to_string(),
			title: issue.name,
			summary: issue.description,
			number,
			series_name,
			series_external_id,
			year,
			month,
			day,
			writers: filled_array_or_none(writers),
			artists: filled_array_or_none(artists),
			colorists: filled_array_or_none(colorists),
			letterers: filled_array_or_none(letterers),
			cover_artists: filled_array_or_none(cover_artists),
			cover_url,
			provider_url: Some(format!(
				"https://comicvine.gamespot.com/issue/{}-{}/",
				u32::from(ComicVinePrefix::Issue),
				external_id
			)),
			..Default::default()
		})
	}

	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		let mut url = Url::parse(&format!("{}{}", self.api_url, "/characters/"))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;

		url.query_pairs_mut()
			.append_pair("api_key", &self.api_key)
			.append_pair("format", "json");

		let response = self.client.get(url).send().await?.error_for_status()?;
		let response_status = response.status().as_u16();
		// don't care about actual data, so used serde_json::Value as catch-all
		let data: ComicVineResponse<serde_json::Value> = response.json().await?;

		// sometimes data.error would be "OK" which is kinda annoying, so we only check status code here
		if data.status_code != 1 {
			return Ok(ProviderCredentialVerification {
				is_valid: false,
				response_status,
				error: Some(data.error),
			});
		}

		Ok(ProviderCredentialVerification {
			is_valid: true,
			response_status,
			error: None,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn get_test_client() -> ComicVineClient {
		dotenvy::dotenv().ok();
		let api_key =
			std::env::var("COMIC_VINE_API_KEY").expect("COMIC_VINE_API_KEY not set");
		ComicVineClient::new(api_key, None)
	}

	#[ignore = "Requires COMIC_VINE_API_KEY env var"]
	#[tokio::test]
	async fn test_search_series() {
		let client = get_test_client();
		let query = SearchQuery {
			title: "Superior Spider-Man".to_string(),
			limit: Some(5),
			..Default::default()
		};

		let results = client.search_series(&query).await;
		assert!(results.is_ok(), "should have been ok: {:?}", results);

		let outcome = results.unwrap();
		assert!(
			!outcome.candidates.is_empty(),
			"should have gotten at least one: {:?}",
			outcome
		);
	}

	#[ignore = "Requires COMIC_VINE_API_KEY env var"]
	#[tokio::test]
	async fn test_search_media() {
		let client = get_test_client();
		let query = SearchQuery {
			title: "Superior Spider-Man 001".to_string(),
			limit: Some(5),
			..Default::default()
		};

		let results = client.search_media(&query).await;
		println!("search_media results: {:#?}", results);
		assert!(results.is_ok());
	}

	#[ignore = "Requires COMIC_VINE_API_KEY env var"]
	#[tokio::test]
	async fn test_verify_credentials() {
		let client = get_test_client();
		let verification = client.verify_credentials().await;
		assert!(verification.is_ok());
		assert!(verification.unwrap().is_valid);
	}

	#[tokio::test]
	async fn search_media_maps_mocked_issue_response() {
		use crate::mock_http::{render_ok, MockServer};

		let search_body = serde_json::json!({
			"error": "OK",
			"status_code": 1,
			"results": [{ "id": 2340 }]
		})
		.to_string();
		let issue_body = serde_json::json!({
			"error": "OK",
			"status_code": 1,
			"results": {
				"id": 2340,
				"name": "The Case of the Chemical Syndicate",
				"description": "<p>First appearance of Batman.</p>",
				"issue_number": "1",
				"cover_date": "1939-05-01",
				"volume": { "id": 796, "name": "Detective Comics" },
				"image": {
					"super_url": "https://comicvine.gamespot.com/cover.jpg"
				},
				"person_credits": [
					{ "name": "Bill Finger", "role": "writer" },
					{ "name": "Bob Kane", "role": "penciler" }
				]
			}
		})
		.to_string();

		let server =
			MockServer::spawn(vec![render_ok(&search_body), render_ok(&issue_body)]);
		let client = ComicVineClient::new("test-key".to_string(), Some(u32::MAX))
			.with_api_url(server.url.clone());

		let query = SearchQuery {
			title: "Batman".to_string(),
			limit: Some(5),
			..Default::default()
		};
		let outcome = client
			.search_media(&query)
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "comic_vine");
		assert_eq!(candidate.external_id, "2340");

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(
			media.title.as_deref(),
			Some("The Case of the Chemical Syndicate")
		);
		assert_eq!(media.series_name.as_deref(), Some("Detective Comics"));
		assert_eq!(media.year, Some(1939));
		assert_eq!(media.month, Some(5));
		assert_eq!(media.day, Some(1));
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://comicvine.gamespot.com/cover.jpg")
		);
		assert_eq!(
			media.writers.as_deref(),
			Some(["Bill Finger".to_string()].as_slice())
		);
		assert_eq!(
			media.artists.as_deref(),
			Some(["Bob Kane".to_string()].as_slice())
		);

		// The search hits /search/ with the issue resource and key, then the
		// per-hit issue detail fetch uses the 4000 issue prefix.
		let requests = server.requests();
		assert_eq!(requests.len(), 2);
		assert!(requests[0].starts_with("GET /search/?"));
		assert!(requests[0].contains("api_key=test-key"));
		assert!(requests[0].contains("format=json"));
		assert!(requests[0].contains("query=Batman"));
		assert!(requests[0].contains("resources=issue"));
		assert!(requests[1].starts_with("GET /issue/4000-2340/"));
	}
}
