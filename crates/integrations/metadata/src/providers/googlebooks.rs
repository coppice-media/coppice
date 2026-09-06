//! Google Books metadata provider
//! (https://developers.google.com/books/docs/v1/using).
//!
//! Works anonymously; an optional API key (stored as the provider's encrypted
//! token) only raises the per-project quota. Identification is ISBN-first:
//! `GET /volumes?q=isbn:{isbn}`; a miss falls back to
//! `q=intitle:{title}[+inauthor:{author}]`. Volume list entries already carry
//! the full `volumeInfo`, so search hits need no per-hit detail fetch;
//! `fetch_media_metadata` reads `GET /volumes/{id}`.
//!
//! Google Books has no series entity, so the series half of the trait reports
//! `OperationNotSupported`; the ingest facade tolerates that as long as the
//! media search succeeds.

use reqwest::Url;
use reqwest_middleware::ClientWithMiddleware;
use serde::Deserialize;

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	error::MetadataProviderError,
	provider::ProviderCredentialVerification,
	rate_limit::RateLimiter,
	scoring::normalize_isbn,
	types::{
		ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate, MediaType,
		SearchQuery,
	},
	ExternalMetadata, MetadataProvider, SearchOutcome,
};

/// The anonymous quota is small (1,000 requests/day per project); one
/// request per second keeps a scan well inside the per-minute burst window.
const GOOGLE_BOOKS_DEFAULT_RATE_LIMIT: u32 = 1;

/// `maxResults` is capped at 40 by the API.
const GOOGLE_BOOKS_MAX_RESULTS: u32 = 40;

pub struct GoogleBooksClient {
	client: ClientWithMiddleware,
	api_key: Option<String>,
	api_url: String,
	rate_limiter: RateLimiter,
}

impl GoogleBooksClient {
	const API_URL: &'static str = "https://www.googleapis.com/books/v1";
	const USER_AGENT: &'static str = concat!(
		"stump/",
		env!("CARGO_PKG_VERSION"),
		" (+https://github.com/stumpapp/stump)"
	);

	/// `api_key` is optional: an empty or absent key means anonymous access.
	pub fn new(api_key: Option<String>, rate_limit: Option<u32>) -> Self {
		let inner = reqwest::Client::builder()
			.user_agent(Self::USER_AGENT)
			.build()
			.expect("Failed to build Google Books HTTP client"); // static config, cannot fail
		Self {
			client: build_client_with_retry(inner, RetryClientConfig::default()),
			api_key: api_key.filter(|key| !key.trim().is_empty()),
			api_url: Self::API_URL.to_string(),
			rate_limiter: RateLimiter::new(
				rate_limit.unwrap_or(GOOGLE_BOOKS_DEFAULT_RATE_LIMIT),
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

	/// Test-only: disable the retry middleware so a canned 429 surfaces
	/// immediately instead of after the exponential backoff schedule.
	#[cfg(test)]
	fn without_retries(mut self) -> Self {
		self.client = build_client_with_retry(
			reqwest::Client::builder()
				.user_agent(Self::USER_AGENT)
				.build()
				.expect("Failed to build Google Books HTTP client"),
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
		if let Some(api_key) = &self.api_key {
			url.query_pairs_mut().append_pair("key", api_key);
		}

		let response = self.client.get(url).send().await?;
		if response.status() == reqwest::StatusCode::NOT_FOUND {
			return Err(MetadataProviderError::NotFound(path.to_string()));
		}
		Ok(response.error_for_status()?.json().await?)
	}

	async fn search_volumes(
		&self,
		q: &str,
		limit: u32,
	) -> Result<Vec<GoogleBooksVolume>, MetadataProviderError> {
		let limit = limit.clamp(1, GOOGLE_BOOKS_MAX_RESULTS).to_string();
		let params = [
			("q", q),
			("maxResults", limit.as_str()),
			("printType", "books"),
		];
		let response: GoogleBooksVolumeList = self.get("/volumes", &params).await?;
		Ok(response.items)
	}

	fn candidates_from(&self, volumes: Vec<GoogleBooksVolume>) -> Vec<MatchCandidate> {
		volumes
			.into_iter()
			.map(|volume| {
				let metadata = media_metadata(self.id(), volume);
				MatchCandidate {
					external_id: metadata.external_id.clone(),
					metadata: ExternalMetadata::Media(metadata),
					provider: self.id().to_string(),
					confidence: 0.0,
					confidence_factors: Vec::new(),
				}
			})
			.collect()
	}
}

#[async_trait::async_trait]
impl MetadataProvider for GoogleBooksClient {
	fn id(&self) -> &'static str {
		"googlebooks"
	}

	fn name(&self) -> &'static str {
		"Google Books"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Book]
	}

	/// Google Books has no series entity.
	async fn search_series(
		&self,
		_query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		Err(MetadataProviderError::OperationNotSupported)
	}

	#[tracing::instrument(skip(self))]
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let limit = query.limit.unwrap_or(10);

		if let Some(isbn) = query
			.isbn
			.as_deref()
			.map(normalize_isbn)
			.filter(|isbn| !isbn.is_empty())
		{
			let volumes = self.search_volumes(&format!("isbn:{isbn}"), 1).await?;
			if !volumes.is_empty() {
				return Ok(SearchOutcome {
					candidates: self.score_search(query, self.candidates_from(volumes)),
					requested: 1,
				});
			}
			tracing::debug!(
				isbn,
				"No Google Books volume for ISBN; falling back to search"
			);
		}

		let mut q = format!("intitle:{}", query.title.trim());
		if let Some(author) = query.author.as_deref().filter(|a| !a.trim().is_empty()) {
			q.push_str(&format!(" inauthor:{}", author.trim()));
		}
		let volumes = self.search_volumes(&q, limit).await?;
		let requested = volumes.len();

		Ok(SearchOutcome {
			candidates: self.score_search(query, self.candidates_from(volumes)),
			requested,
		})
	}

	async fn fetch_series_metadata(
		&self,
		_external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		Err(MetadataProviderError::OperationNotSupported)
	}

	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let volume: GoogleBooksVolume =
			self.get(&format!("/volumes/{external_id}"), &[]).await?;
		Ok(media_metadata(self.id(), volume))
	}

	/// A one-result search proves the key (if any) is accepted and the quota
	/// is not exhausted.
	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		self.rate_limiter.until_ready().await;
		let mut url = Url::parse(&format!("{}/volumes", self.api_url))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;
		url.query_pairs_mut()
			.append_pair("q", "isbn:9780140328721")
			.append_pair("maxResults", "1");
		if let Some(api_key) = &self.api_key {
			url.query_pairs_mut().append_pair("key", api_key);
		}
		let response = self.client.get(url).send().await?;
		let status = response.status().as_u16();
		let is_valid = response.status().is_success();
		Ok(ProviderCredentialVerification {
			response_status: status,
			is_valid,
			error: (!is_valid).then(|| format!("unexpected status {status}")),
		})
	}
}

fn media_metadata(provider_id: &str, volume: GoogleBooksVolume) -> ExternalMediaMetadata {
	let info = volume.volume_info;
	let (year, month, day) = parse_published_date(info.published_date.as_deref());
	let (isbn, isbn_13) = split_identifiers(&info.industry_identifiers);
	let cover_url = info
		.image_links
		.as_ref()
		.and_then(|links| {
			links
				.thumbnail
				.clone()
				.or_else(|| links.small_thumbnail.clone())
		})
		.map(clean_cover_url);
	let provider_url = info
		.canonical_volume_link
		.or(info.info_link)
		.or_else(|| Some(format!("https://books.google.com/books?id={}", volume.id)));

	ExternalMediaMetadata {
		provider: provider_id.to_string(),
		external_id: volume.id,
		title: info.title.filter(|t| !t.trim().is_empty()),
		summary: info.description.filter(|d| !d.trim().is_empty()),
		page_count: info.page_count,
		year,
		month,
		day,
		genres: filled_or_none(info.categories),
		isbn,
		isbn_13,
		writers: filled_or_none(info.authors),
		cover_url,
		provider_url,
		..Default::default()
	}
}

/// `publishedDate` is `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`.
fn parse_published_date(date: Option<&str>) -> (Option<i32>, Option<i32>, Option<i32>) {
	let Some(date) = date else {
		return (None, None, None);
	};
	let mut parts = date.split('-');
	let year = parts.next().and_then(|part| part.parse().ok());
	let month = parts.next().and_then(|part| part.parse().ok());
	let day = parts.next().and_then(|part| part.parse().ok());
	(year, month, day)
}

fn split_identifiers(
	identifiers: &[GoogleBooksIdentifier],
) -> (Option<String>, Option<String>) {
	let find = |kind: &str| {
		identifiers
			.iter()
			.find(|id| id.kind == kind)
			.map(|id| id.identifier.clone())
	};
	(find("ISBN_10"), find("ISBN_13"))
}

/// Thumbnails are served over plain HTTP with a page-curl overlay baked in;
/// upgrade the scheme and drop the overlay.
fn clean_cover_url(url: String) -> String {
	let url = url
		.strip_prefix("http://")
		.map(|rest| format!("https://{rest}"))
		.unwrap_or(url);
	url.replace("&edge=curl", "")
}

fn filled_or_none(values: Vec<String>) -> Option<Vec<String>> {
	(!values.is_empty()).then(|| {
		let mut values = values;
		values.dedup();
		values
	})
}

#[derive(Debug, Deserialize)]
struct GoogleBooksVolumeList {
	#[serde(default)]
	items: Vec<GoogleBooksVolume>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleBooksVolume {
	id: String,
	#[serde(default)]
	volume_info: GoogleBooksVolumeInfo,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GoogleBooksVolumeInfo {
	title: Option<String>,
	#[serde(default)]
	authors: Vec<String>,
	published_date: Option<String>,
	description: Option<String>,
	#[serde(default)]
	industry_identifiers: Vec<GoogleBooksIdentifier>,
	page_count: Option<i32>,
	#[serde(default)]
	categories: Vec<String>,
	image_links: Option<GoogleBooksImageLinks>,
	info_link: Option<String>,
	canonical_volume_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleBooksIdentifier {
	#[serde(rename = "type")]
	kind: String,
	identifier: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleBooksImageLinks {
	small_thumbnail: Option<String>,
	thumbnail: Option<String>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::mock_http::{render_ok, MockServer};

	fn query(title: &str) -> SearchQuery {
		SearchQuery {
			title: title.to_string(),
			limit: Some(5),
			..Default::default()
		}
	}

	fn volume_json() -> serde_json::Value {
		serde_json::json!({
			"kind": "books#volume",
			"id": "abc123",
			"selfLink": "https://www.googleapis.com/books/v1/volumes/abc123",
			"volumeInfo": {
				"title": "Fantastic Mr. Fox",
				"authors": ["Roald Dahl"],
				"publisher": "Puffin",
				"publishedDate": "2007-08-16",
				"description": "A clever fox outwits three farmers.",
				"industryIdentifiers": [
					{ "type": "ISBN_10", "identifier": "0140328726" },
					{ "type": "ISBN_13", "identifier": "9780140328721" }
				],
				"pageCount": 96,
				"categories": ["Juvenile Fiction"],
				"imageLinks": {
					"smallThumbnail": "http://books.google.com/books/content?id=abc123&printsec=frontcover&img=1&zoom=5&edge=curl&source=gbs_api",
					"thumbnail": "http://books.google.com/books/content?id=abc123&printsec=frontcover&img=1&zoom=1&edge=curl&source=gbs_api"
				},
				"language": "en",
				"infoLink": "https://play.google.com/store/books/details?id=abc123",
				"canonicalVolumeLink": "https://books.google.com/books/about/Fantastic_Mr_Fox.html?id=abc123"
			}
		})
	}

	fn list_response() -> String {
		serde_json::json!({
			"kind": "books#volumes",
			"totalItems": 1,
			"items": [volume_json()]
		})
		.to_string()
	}

	#[tokio::test]
	async fn search_media_maps_volume_list() {
		let server = MockServer::spawn(vec![render_ok(&list_response())]);
		let client = GoogleBooksClient::new(None, None).with_api_url(server.url.clone());

		let outcome = client
			.search_media(&SearchQuery {
				author: Some("Roald Dahl".to_string()),
				..query("Fantastic Mr. Fox")
			})
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "googlebooks");
		assert_eq!(candidate.external_id, "abc123");
		assert!(candidate.confidence >= 0.9, "{}", candidate.confidence);

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(media.title.as_deref(), Some("Fantastic Mr. Fox"));
		assert_eq!(
			media.summary.as_deref(),
			Some("A clever fox outwits three farmers.")
		);
		assert_eq!(
			(media.year, media.month, media.day),
			(Some(2007), Some(8), Some(16))
		);
		assert_eq!(media.page_count, Some(96));
		assert_eq!(media.isbn.as_deref(), Some("0140328726"));
		assert_eq!(media.isbn_13.as_deref(), Some("9780140328721"));
		assert_eq!(
			media.genres.as_deref(),
			Some(["Juvenile Fiction".to_string()].as_slice())
		);
		assert_eq!(
			media.writers.as_deref(),
			Some(["Roald Dahl".to_string()].as_slice())
		);
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://books.google.com/books/content?id=abc123&printsec=frontcover&img=1&zoom=1&source=gbs_api")
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://books.google.com/books/about/Fantastic_Mr_Fox.html?id=abc123")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		let request = &requests[0];
		assert!(request.starts_with("GET /volumes?"), "{request}");
		assert!(
			request.contains("q=intitle%3AFantastic+Mr.+Fox+inauthor%3ARoald+Dahl"),
			"{request}"
		);
		assert!(request.contains("maxResults=5"), "{request}");
		assert!(!request.contains("key="), "{request}");
		assert!(
			request.to_ascii_lowercase().contains("user-agent: stump/"),
			"{request}"
		);
	}

	#[tokio::test]
	async fn isbn_query_is_tried_first_and_wins() {
		let server = MockServer::spawn(vec![render_ok(&list_response())]);
		let client = GoogleBooksClient::new(Some("secret-key".to_string()), None)
			.with_api_url(server.url.clone());

		let outcome = client
			.search_media(&SearchQuery {
				isbn: Some("978-0-14-032872-1".to_string()),
				..query("Something Else Entirely")
			})
			.await
			.expect("ISBN lookup should succeed");

		assert_eq!(outcome.candidates.len(), 1);
		assert!(
			outcome.candidates[0].confidence >= 0.98,
			"ISBN hit should score as definitive, got {}",
			outcome.candidates[0].confidence
		);
		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(
			requests[0].contains("q=isbn%3A9780140328721"),
			"{}",
			requests[0]
		);
		assert!(requests[0].contains("key=secret-key"), "{}", requests[0]);
	}

	#[tokio::test]
	async fn isbn_miss_falls_back_to_title_search() {
		let empty =
			serde_json::json!({ "kind": "books#volumes", "totalItems": 0 }).to_string();
		let server =
			MockServer::spawn(vec![render_ok(&empty), render_ok(&list_response())]);
		let client = GoogleBooksClient::new(None, None).with_api_url(server.url.clone());

		let outcome = client
			.search_media(&SearchQuery {
				isbn: Some("9780000000000".to_string()),
				..query("Fantastic Mr. Fox")
			})
			.await
			.expect("fallback search should succeed");

		assert_eq!(outcome.candidates.len(), 1);
		let requests = server.requests();
		assert_eq!(requests.len(), 2);
		assert!(
			requests[0].contains("q=isbn%3A9780000000000"),
			"{}",
			requests[0]
		);
		assert!(
			requests[1].contains("q=intitle%3AFantastic+Mr.+Fox"),
			"{}",
			requests[1]
		);
	}

	#[tokio::test]
	async fn fetch_media_metadata_reads_single_volume() {
		let server = MockServer::spawn(vec![render_ok(&volume_json().to_string())]);
		let client = GoogleBooksClient::new(None, None).with_api_url(server.url.clone());

		let media = client
			.fetch_media_metadata("abc123")
			.await
			.expect("volume lookup should succeed");
		assert_eq!(media.external_id, "abc123");
		assert_eq!(media.title.as_deref(), Some("Fantastic Mr. Fox"));

		let requests = server.requests();
		assert!(
			requests[0].starts_with("GET /volumes/abc123"),
			"{}",
			requests[0]
		);
	}

	#[tokio::test]
	async fn rate_limited_response_maps_to_rate_limited_error() {
		let too_many = "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let server = MockServer::spawn(vec![too_many.to_string()]);
		let client = GoogleBooksClient::new(None, None)
			.with_api_url(server.url.clone())
			.without_retries();

		let error = client
			.search_media(&query("Fantastic Mr. Fox"))
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
Content-Length: 13
Connection: close

{"items": [{"#
			.replace('\n', "\r\n");
		let server = MockServer::spawn(vec![body]);
		let client = GoogleBooksClient::new(None, None).with_api_url(server.url.clone());

		let error = client
			.search_media(&query("Fantastic Mr. Fox"))
			.await
			.expect_err("malformed JSON should surface as an error");

		assert!(
			!error.is_rate_limited(),
			"malformed JSON must not surface as rate limited, got {error:?}"
		);
		assert!(
			matches!(&error, MetadataProviderError::ReqwestError(err) if err.is_decode()),
			"expected decode error, got {error:?}"
		);
	}

	#[test]
	fn published_date_accepts_partial_precision() {
		assert_eq!(parse_published_date(Some("2007")), (Some(2007), None, None));
		assert_eq!(
			parse_published_date(Some("2007-08")),
			(Some(2007), Some(8), None)
		);
		assert_eq!(
			parse_published_date(Some("2007-08-16")),
			(Some(2007), Some(8), Some(16))
		);
	}

	#[ignore = "Requires network access"]
	#[tokio::test]
	async fn live_isbn_lookup() {
		let client = GoogleBooksClient::new(None, None);
		let outcome = client
			.search_media(&SearchQuery {
				isbn: Some("9780140328721".to_string()),
				..query("Fantastic Mr Fox")
			})
			.await
			.expect("live lookup should succeed");
		assert!(!outcome.candidates.is_empty());
	}
}
