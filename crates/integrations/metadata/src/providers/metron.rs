//! Metron metadata provider (https://metron.cloud/api/).
//!
//! Metron is a community-run comic book database (publishers, series, issues,
//! creators). Every endpoint requires Basic authentication; the provider's
//! encrypted token holds `username:password`, which is base64-encoded into the
//! `Authorization: Basic …` header. A pre-encoded credential (the token shown
//! on the Metron profile page, which contains no `:`) is passed through
//! unchanged.
//!
//! The published quota is ~30 requests/minute; exceed it and the API answers
//! 429, which the retrying client surfaces as a rate-limit error.
//!
//! Identification is title-first: `GET /issue/?series_name={title}[-&number=n]`
//! for books and `GET /series/?name={title}` for series. List hits already
//! carry the story name, series, cover date and cover image, so media search
//! needs no per-hit detail fetch; `fetch_media_metadata` reads
//! `GET /issue/{id}/` and `fetch_series_metadata` reads `GET /series/{id}/`
//! for the full credits/description/publisher record.

use base64::Engine;
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

/// Metron publishes a 30 requests/minute quota
/// (https://metron-project.github.io/blog/api-best-practices).
const METRON_DEFAULT_RATE_LIMIT: u32 = 30;

/// The list endpoints are DRF-paginated; 100 is the documented ceiling.
const METRON_MAX_LIMIT: u32 = 100;

/// Creator roles Metron uses on issue credits, mapped onto the media fields
/// we can represent (https://metron.cloud/api/creator/ role ids).
const SCRIPT_ROLES: &[&str] = &["script"];
const PENCIL_ROLES: &[&str] = &["pencils", "penciller", "penciler"];
const COLOR_ROLES: &[&str] = &["colors", "colorist", "colourist"];
const LETTER_ROLES: &[&str] = &["letters", "letterer"];
const COVER_ROLES: &[&str] = &["cover", "cover artist", "covers"];

pub struct MetronClient {
	client: ClientWithMiddleware,
	/// Pre-rendered `Authorization` header value, if credentials exist.
	auth_header: Option<String>,
	api_url: String,
	rate_limiter: RateLimiter,
}

impl MetronClient {
	const API_URL: &'static str = "https://metron.cloud/api";
	const SITE_URL: &'static str = "https://metron.cloud";
	const USER_AGENT: &'static str = concat!(
		"stump/",
		env!("CARGO_PKG_VERSION"),
		" (+https://github.com/stumpapp/stump)"
	);

	/// `api_token` is the Metron credential: either `username:password` (the
	/// recommended storage form) or an already-encoded Basic token.
	pub fn new(api_token: String, rate_limit: Option<u32>) -> Self {
		let inner = reqwest::Client::builder()
			.user_agent(Self::USER_AGENT)
			.build()
			.expect("Failed to build Metron HTTP client"); // static config, cannot fail
		Self {
			client: build_client_with_retry(inner, RetryClientConfig::default()),
			auth_header: basic_auth_header(&api_token),
			api_url: Self::API_URL.to_string(),
			rate_limiter: RateLimiter::per_minute(
				rate_limit.unwrap_or(METRON_DEFAULT_RATE_LIMIT),
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
				.expect("Failed to build Metron HTTP client"),
			RetryClientConfig { max_retries: 0 },
		);
		self.rate_limiter = RateLimiter::new(u32::MAX);
		self
	}

	/// GET a JSON document. A 404 surfaces as [`MetadataProviderError::NotFound`].
	async fn get<T: serde::de::DeserializeOwned>(
		&self,
		path: &str,
		params: &[(&str, String)],
	) -> Result<T, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let mut url = Url::parse(&format!("{}{}", self.api_url, path))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;
		for (key, value) in params {
			url.query_pairs_mut().append_pair(key, value);
		}

		let mut request = self.client.get(url);
		if let Some(auth) = &self.auth_header {
			request = request.header(reqwest::header::AUTHORIZATION, auth);
		}
		let response = request.send().await?;
		if response.status() == reqwest::StatusCode::NOT_FOUND {
			return Err(MetadataProviderError::NotFound(path.to_string()));
		}
		Ok(response.error_for_status()?.json().await?)
	}

	async fn search_issues(
		&self,
		query: &SearchQuery,
	) -> Result<Vec<MetronIssueListItem>, MetadataProviderError> {
		let limit = query.limit.unwrap_or(10).clamp(1, METRON_MAX_LIMIT);
		let mut params = vec![
			("series_name", query.title.trim().to_string()),
			("limit", limit.to_string()),
		];
		if let Some(number) = query.number {
			params.push(("number", format_issue_number(number)));
		}
		if let Some(year) = query.year {
			params.push(("cover_year", year.to_string()));
		}
		let response: MetronListResponse<MetronIssueListItem> =
			self.get("/issue/", &params).await?;
		Ok(response.results)
	}

	async fn search_series_list(
		&self,
		query: &SearchQuery,
	) -> Result<Vec<MetronSeriesListItem>, MetadataProviderError> {
		let limit = query.limit.unwrap_or(10).clamp(1, METRON_MAX_LIMIT);
		let params = vec![
			("name", query.title.trim().to_string()),
			("limit", limit.to_string()),
		];
		let response: MetronListResponse<MetronSeriesListItem> =
			self.get("/series/", &params).await?;
		Ok(response.results)
	}

	fn media_candidate(&self, metadata: ExternalMediaMetadata) -> MatchCandidate {
		MatchCandidate {
			external_id: metadata.external_id.clone(),
			metadata: ExternalMetadata::Media(metadata),
			provider: self.id().to_string(),
			confidence: 0.0,
			confidence_factors: Vec::new(),
		}
	}

	fn series_candidate(&self, metadata: ExternalSeriesMetadata) -> MatchCandidate {
		MatchCandidate {
			external_id: metadata.external_id.clone(),
			metadata: ExternalMetadata::Series(metadata),
			provider: self.id().to_string(),
			confidence: 0.0,
			confidence_factors: Vec::new(),
		}
	}

	fn media_from_list_item(
		&self,
		item: MetronIssueListItem,
	) -> Option<ExternalMediaMetadata> {
		let series = item.series.as_ref()?;
		let number = item.number.clone().filter(|n| !n.trim().is_empty());
		let title = item
			.issue_name
			.clone()
			.filter(|name| !name.trim().is_empty())
			.or_else(|| {
				Some(format!(
					"{} #{}",
					series.name,
					number.as_deref().unwrap_or("?")
				))
			})?;
		let (year, month, day) = parse_date(item.cover_date.as_deref());

		Some(ExternalMediaMetadata {
			provider: self.id().to_string(),
			external_id: item.id.to_string(),
			title: Some(title),
			series_name: Some(series.name.clone()),
			series_external_id: Some(series.id.to_string()),
			number: number.and_then(|n| n.trim().parse().ok()),
			year,
			month,
			day,
			cover_url: non_empty(item.image),
			provider_url: Some(format!("{}/issue/{}/", Self::SITE_URL, item.id)),
			..Default::default()
		})
	}

	fn series_from_list_item(
		&self,
		item: MetronSeriesListItem,
	) -> ExternalSeriesMetadata {
		ExternalSeriesMetadata {
			provider: self.id().to_string(),
			external_id: item.id.to_string(),
			title: item.name,
			year: item.year_began,
			end_year: item.year_end,
			volume_count: item.issue_count,
			..Default::default()
		}
	}

	/// Split issue credits onto the media fields we can represent.
	fn credits_to_people(
		credits: &[MetronCredit],
	) -> (
		Option<Vec<String>>,
		Option<Vec<String>>,
		Option<Vec<String>>,
		Option<Vec<String>>,
		Option<Vec<String>>,
	) {
		let mut writers = Vec::new();
		let mut artists = Vec::new();
		let mut colorists = Vec::new();
		let mut letterers = Vec::new();
		let mut cover_artists = Vec::new();
		for credit in credits {
			for role in &credit.role {
				if role_matches(&role.name, SCRIPT_ROLES) {
					writers.push(credit.creator.clone());
				} else if role_matches(&role.name, PENCIL_ROLES) {
					artists.push(credit.creator.clone());
				} else if role_matches(&role.name, COLOR_ROLES) {
					colorists.push(credit.creator.clone());
				} else if role_matches(&role.name, LETTER_ROLES) {
					letterers.push(credit.creator.clone());
				} else if role_matches(&role.name, COVER_ROLES) {
					cover_artists.push(credit.creator.clone());
				}
			}
		}
		(
			filled_or_none(writers),
			filled_or_none(artists),
			filled_or_none(colorists),
			filled_or_none(letterers),
			filled_or_none(cover_artists),
		)
	}
}

#[async_trait::async_trait]
impl MetadataProvider for MetronClient {
	fn id(&self) -> &'static str {
		"metron"
	}

	fn name(&self) -> &'static str {
		"Metron"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Comic]
	}

	#[tracing::instrument(skip(self))]
	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let series = self.search_series_list(query).await?;
		let requested = series.len();
		let candidates = series
			.into_iter()
			.map(|item| self.series_candidate(self.series_from_list_item(item)))
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	#[tracing::instrument(skip(self))]
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let issues = self.search_issues(query).await?;
		let requested = issues.len();
		let candidates = issues
			.into_iter()
			.filter_map(|item| self.media_from_list_item(item))
			.map(|metadata| self.media_candidate(metadata))
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
		let series: MetronSeries =
			self.get(&format!("/series/{external_id}/"), &[]).await?;
		let publisher = series.publisher.map(|publisher| publisher.name);

		Ok(ExternalSeriesMetadata {
			provider: self.id().to_string(),
			external_id: series.id.to_string(),
			title: series.name,
			alternative_titles: series.alt_names,
			summary: non_empty(series.desc),
			status: parse_status(series.status.as_deref()),
			year: series.year_began,
			end_year: series.year_end,
			genres: filled_or_none(series.genres.into_iter().map(|g| g.name).collect()),
			publisher,
			volume_count: series.issue_count,
			cover_url: None, // Metron series records carry no cover image
			..Default::default()
		})
	}

	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let issue: MetronIssue = self.get(&format!("/issue/{external_id}/"), &[]).await?;
		let (writers, artists, colorists, letterers, cover_artists) =
			Self::credits_to_people(&issue.credits);
		let number = issue.number.clone().filter(|n| !n.trim().is_empty());
		let (year, month, day) = parse_date(issue.cover_date.as_deref());
		let title = issue
			.story_titles
			.first()
			.cloned()
			.or_else(|| non_empty(issue.title.clone()))
			.or_else(|| {
				Some(format!(
					"{} #{}",
					issue.series.as_ref().map(|s| s.name.as_str()).unwrap_or(""),
					number.as_deref().unwrap_or("?")
				))
			});

		Ok(ExternalMediaMetadata {
			provider: self.id().to_string(),
			external_id: issue.id.to_string(),
			title,
			summary: non_empty(issue.desc),
			page_count: issue.page,
			series_name: issue.series.as_ref().map(|s| s.name.clone()),
			series_external_id: issue.series.as_ref().map(|s| s.id.to_string()),
			number: number.and_then(|n| n.trim().parse().ok()),
			year,
			month,
			day,
			genres: filled_or_none(
				issue
					.series
					.map(|s| s.genres.into_iter().map(|g| g.name).collect())
					.unwrap_or_default(),
			),
			tags: filled_or_none(issue.arcs.into_iter().map(|a| a.name).collect()),
			isbn: non_empty(issue.isbn),
			writers,
			artists,
			colorists,
			letterers,
			cover_artists,
			cover_url: non_empty(issue.image),
			provider_url: issue.resource_url,
			publisher: issue.publisher.map(|publisher| publisher.name),
			..Default::default()
		})
	}

	/// A one-result publisher query proves the credential is accepted and the
	/// quota is not exhausted.
	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		self.rate_limiter.until_ready().await;
		let mut url = Url::parse(&format!("{}/publisher/", self.api_url))
			.map_err(|e| MetadataProviderError::Other(format!("Invalid URL: {}", e)))?;
		url.query_pairs_mut().append_pair("limit", "1");
		let mut request = self.client.get(url);
		if let Some(auth) = &self.auth_header {
			request = request.header(reqwest::header::AUTHORIZATION, auth);
		}
		let response = request.send().await?;
		let status = response.status().as_u16();
		let is_valid = response.status().is_success();
		Ok(ProviderCredentialVerification {
			response_status: status,
			is_valid,
			error: (!is_valid).then(|| format!("unexpected status {status}")),
		})
	}
}

/// Build the `Authorization: Basic …` header value. A `username:password`
/// token is encoded; a token without a `:` is assumed to already be the
/// encoded credential Metron shows on the profile page.
fn basic_auth_header(token: &str) -> Option<String> {
	let token = token.trim();
	if token.is_empty() {
		return None;
	}
	if token.contains(':') {
		let encoded = base64::engine::general_purpose::STANDARD.encode(token);
		Some(format!("Basic {encoded}"))
	} else if let Some(encoded) = token.strip_prefix("Basic ") {
		Some(format!("Basic {}", encoded.trim()))
	} else {
		Some(format!("Basic {token}"))
	}
}

/// Issue numbers arrive as strings (`1`, `1.5`); one decimal place covers the
/// half-step issues comics actually use.
fn format_issue_number(number: f32) -> String {
	if number.fract() == 0.0 {
		format!("{}", number as i64)
	} else {
		format!("{number}")
	}
}

fn role_matches(role: &str, expected: &[&str]) -> bool {
	let role = role.trim().to_ascii_lowercase();
	expected.iter().any(|candidate| role == *candidate)
}

/// Metron series status strings map onto the shared publication status.
fn parse_status(status: Option<&str>) -> Option<PublicationStatus> {
	match status?.trim().to_ascii_lowercase().as_str() {
		"ongoing" => Some(PublicationStatus::Ongoing),
		"completed" => Some(PublicationStatus::Completed),
		"hiatus" => Some(PublicationStatus::Hiatus),
		"cancelled" | "canceled" => Some(PublicationStatus::Cancelled),
		"upcoming" | "announced" => Some(PublicationStatus::Upcoming),
		_ => None,
	}
}

/// Dates are `YYYY-MM-DD`, `YYYY-MM`, or `YYYY`.
fn parse_date(date: Option<&str>) -> (Option<i32>, Option<i32>, Option<i32>) {
	let Some(date) = date else {
		return (None, None, None);
	};
	let mut parts = date.split('-');
	let year = parts.next().and_then(|part| part.parse().ok());
	let month = parts.next().and_then(|part| part.parse().ok());
	let day = parts.next().and_then(|part| part.parse().ok());
	(year, month, day)
}

fn non_empty(value: Option<String>) -> Option<String> {
	value.filter(|v| !v.trim().is_empty())
}

fn filled_or_none(values: Vec<String>) -> Option<Vec<String>> {
	(!values.is_empty()).then(|| {
		let mut values = values;
		values.dedup();
		values
	})
}

#[derive(Debug, Deserialize)]
struct MetronListResponse<T> {
	results: Vec<T>,
}

/// `{ "id": 1, "name": "Marvel" }` shape used across Metron payloads; only the
/// name is consumed.
#[derive(Debug, Deserialize)]
struct MetronIdName {
	name: String,
}

/// List hits expose the series as `{ id, name, volume, year_began }`.
#[derive(Debug, Deserialize)]
struct MetronBasicSeries {
	id: i64,
	name: String,
	#[serde(default)]
	genres: Vec<MetronIdName>,
}

#[derive(Debug, Deserialize)]
struct MetronIssueListItem {
	id: i64,
	#[serde(default)]
	number: Option<String>,
	#[serde(default)]
	cover_date: Option<String>,
	#[serde(default)]
	image: Option<String>,
	/// The story name arrives under the `issue` key in list responses.
	#[serde(alias = "issue", default)]
	issue_name: Option<String>,
	#[serde(default)]
	series: Option<MetronBasicSeries>,
}

#[derive(Debug, Deserialize)]
struct MetronIssue {
	id: i64,
	#[serde(default)]
	number: Option<String>,
	#[serde(default)]
	cover_date: Option<String>,
	#[serde(default)]
	image: Option<String>,
	#[serde(default)]
	publisher: Option<MetronIdName>,
	#[serde(default)]
	series: Option<MetronBasicSeries>,
	/// Collection title (`title` key) — distinct from the story titles.
	#[serde(default)]
	title: Option<String>,
	/// Story titles arrive under the `name` key.
	#[serde(alias = "name", default)]
	story_titles: Vec<String>,
	/// Page count arrives under the `page` key.
	#[serde(default)]
	page: Option<i32>,
	#[serde(default)]
	desc: Option<String>,
	#[serde(default)]
	isbn: Option<String>,
	#[serde(default)]
	credits: Vec<MetronCredit>,
	#[serde(default)]
	arcs: Vec<MetronIdName>,
	#[serde(default)]
	resource_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetronCredit {
	#[serde(default)]
	creator: String,
	#[serde(default)]
	role: Vec<MetronIdName>,
}

#[derive(Debug, Deserialize)]
struct MetronSeriesListItem {
	id: i64,
	/// List hits name the field `series`; detail uses `name`.
	#[serde(alias = "series")]
	name: String,
	#[serde(default)]
	year_began: Option<i32>,
	#[serde(default)]
	year_end: Option<i32>,
	#[serde(default)]
	issue_count: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct MetronSeries {
	id: i64,
	name: String,
	#[serde(default)]
	alt_names: Vec<String>,
	#[serde(default)]
	status: Option<String>,
	#[serde(default)]
	year_began: Option<i32>,
	#[serde(default)]
	year_end: Option<i32>,
	#[serde(default)]
	issue_count: Option<i32>,
	#[serde(default)]
	publisher: Option<MetronIdName>,
	#[serde(default)]
	desc: Option<String>,
	#[serde(default)]
	genres: Vec<MetronIdName>,
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

	fn issue_list_response() -> String {
		serde_json::json!({
			"count": 1,
			"next": null,
			"previous": null,
			"results": [{
				"id": 406984,
				"number": "1",
				"cover_date": "1970-11-01",
				"store_date": null,
				"image": "https://metron.cloud/media/issue/406984/cover.jpg",
				"cover_hash": "abcd1234",
				"modified": "2023-01-01T00:00:00Z",
				"issue": "Fantastic Mr Fox",
				"series": {
					"id": 51229,
					"name": "Fantastic Mr Fox",
					"volume": 1,
					"year_began": 1970
				}
			}]
		})
		.to_string()
	}

	fn issue_detail_response() -> String {
		serde_json::json!({
			"id": 406984,
			"number": "1",
			"cover_date": "1970-11-01",
			"store_date": "1970-10-15",
			"image": "https://metron.cloud/media/issue/406984/cover.jpg",
			"cover_hash": "abcd1234",
			"modified": "2023-01-01T00:00:00Z",
			"publisher": { "id": 1, "name": "Puffin Comics" },
			"series": {
				"id": 51229,
				"name": "Fantastic Mr Fox",
				"sort_name": "Fantastic Mr Fox",
				"volume": 1,
				"year_began": 1970,
				"series_type": { "id": 1, "name": "Single Issue" },
				"genres": [{ "id": 3, "name": "Anthology" }]
			},
			"title": "The Classic Tale",
			"name": ["Fantastic Mr Fox"],
			"page": 96,
			"desc": "A clever fox outwits three farmers.",
			"isbn": "",
			"credits": [
				{ "id": 1, "creator": "Roald Dahl", "role": [{ "id": 1, "name": "Script" }] },
				{ "id": 2, "creator": "Donald Chaffin", "role": [{ "id": 2, "name": "Pencils" }, { "id": 6, "name": "Cover" }] },
				{ "id": 3, "creator": "Some Inker", "role": [{ "id": 3, "name": "Inks" }] },
				{ "id": 4, "creator": "A Colorist", "role": [{ "id": 4, "name": "Colors" }] },
				{ "id": 5, "creator": "A Letterer", "role": [{ "id": 5, "name": "Letters" }] }
			],
			"arcs": [{ "id": 9, "name": "The Fox Arc" }],
			"resource_url": "https://metron.cloud/issue/406984/"
		})
		.to_string()
	}

	fn series_list_response() -> String {
		serde_json::json!({
			"count": 1,
			"next": null,
			"previous": null,
			"results": [{
				"id": 51229,
				"series": "Fantastic Mr Fox",
				"year_began": 1970,
				"year_end": 1972,
				"issue_count": 30,
				"volume": 1,
				"modified": "2023-01-01T00:00:00Z"
			}]
		})
		.to_string()
	}

	fn series_detail_response() -> String {
		serde_json::json!({
			"id": 51229,
			"series": "Fantastic Mr Fox",
			"name": "Fantastic Mr Fox",
			"sort_name": "Fantastic Mr Fox",
			"alt_names": ["Mr Fox le magnifique"],
			"year_began": 1970,
			"year_end": 1972,
			"issue_count": 30,
			"volume": 1,
			"series_type": { "id": 1, "name": "Single Issue" },
			"status": "Completed",
			"publisher": { "id": 1, "name": "Puffin Comics" },
			"desc": "A comic adaptation.",
			"genres": [{ "id": 3, "name": "Anthology" }],
			"resource_url": "https://metron.cloud/series/51229/"
		})
		.to_string()
	}

	fn client_at(server: &MockServer) -> MetronClient {
		MetronClient::new("user:pass".to_string(), None).with_api_url(server.url.clone())
	}

	#[tokio::test]
	async fn search_media_maps_issue_hits_without_detail_fetches() {
		let server = MockServer::spawn(vec![render_ok(&issue_list_response())]);
		let client = client_at(&server);

		let outcome = client
			.search_media(&query("Fantastic Mr Fox"))
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "metron");
		assert_eq!(candidate.external_id, "406984");
		assert!(candidate.confidence >= 0.9, "{}", candidate.confidence);

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(media.title.as_deref(), Some("Fantastic Mr Fox"));
		assert_eq!(media.series_name.as_deref(), Some("Fantastic Mr Fox"));
		assert_eq!(media.series_external_id.as_deref(), Some("51229"));
		assert_eq!(media.number, Some(1.0));
		assert_eq!(
			(media.year, media.month, media.day),
			(Some(1970), Some(11), Some(1))
		);
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://metron.cloud/media/issue/406984/cover.jpg")
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://metron.cloud/issue/406984/")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		let request = &requests[0];
		assert!(request.starts_with("GET /issue/"), "{request}");
		assert!(
			request.contains("series_name=Fantastic+Mr+Fox"),
			"{request}"
		);
		assert!(request.contains("limit=5"), "{request}");
		// Basic auth must ride along on every request
		assert!(
			request
				.to_ascii_lowercase()
				.contains("authorization: basic dxnlcjpwyxnz"),
			"{request}"
		);
	}

	#[tokio::test]
	async fn search_media_with_number_and_year_adds_filters() {
		let server = MockServer::spawn(vec![render_ok(&issue_list_response())]);
		let client = client_at(&server);

		client
			.search_media(&SearchQuery {
				number: Some(1.5),
				year: Some(1970),
				..query("Fantastic Mr Fox")
			})
			.await
			.expect("search should succeed");

		let request = &server.requests()[0];
		assert!(request.contains("number=1.5"), "{request}");
		assert!(request.contains("cover_year=1970"), "{request}");
	}

	#[tokio::test]
	async fn search_series_maps_list_results() {
		let server = MockServer::spawn(vec![render_ok(&series_list_response())]);
		let client = client_at(&server);

		let outcome = client
			.search_series(&query("Fantastic Mr Fox"))
			.await
			.expect("series search should succeed");

		assert_eq!(outcome.candidates.len(), 1);
		let series = outcome.candidates[0]
			.metadata
			.as_series()
			.expect("series candidate");
		assert_eq!(series.provider, "metron");
		assert_eq!(series.title, "Fantastic Mr Fox");
		assert_eq!(series.year, Some(1970));
		assert_eq!(series.end_year, Some(1972));
		assert_eq!(series.volume_count, Some(30));
		let requests = server.requests();
		assert!(requests.iter().all(|request| request
			.to_ascii_lowercase()
			.contains("authorization: basic dxnlcjpwyxnz")));
	}

	#[tokio::test]
	async fn fetch_media_metadata_maps_credits_and_dates() {
		let server = MockServer::spawn(vec![render_ok(&issue_detail_response())]);
		let client = client_at(&server);

		let media = client
			.fetch_media_metadata("406984")
			.await
			.expect("issue lookup should succeed");

		assert_eq!(media.external_id, "406984");
		assert_eq!(media.title.as_deref(), Some("Fantastic Mr Fox"));
		assert_eq!(
			media.summary.as_deref(),
			Some("A clever fox outwits three farmers.")
		);
		assert_eq!(media.page_count, Some(96));
		assert_eq!(media.number, Some(1.0));
		assert_eq!(
			(media.year, media.month, media.day),
			(Some(1970), Some(11), Some(1))
		);
		assert_eq!(
			media.genres.as_deref(),
			Some(["Anthology".to_string()].as_slice())
		);
		assert_eq!(
			media.tags.as_deref(),
			Some(["The Fox Arc".to_string()].as_slice())
		);
		assert_eq!(
			media.writers.as_deref(),
			Some(["Roald Dahl".to_string()].as_slice())
		);
		assert_eq!(
			media.artists.as_deref(),
			Some(["Donald Chaffin".to_string()].as_slice())
		);
		// Inks have no dedicated media field; colors/letters/cover do.
		assert_eq!(
			media.colorists.as_deref(),
			Some(["A Colorist".to_string()].as_slice())
		);
		assert_eq!(
			media.letterers.as_deref(),
			Some(["A Letterer".to_string()].as_slice())
		);
		assert_eq!(
			media.cover_artists.as_deref(),
			Some(["Donald Chaffin".to_string()].as_slice())
		);
		// Metron stores an empty string for missing ISBNs; that must not leak.
		assert_eq!(media.isbn, None);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://metron.cloud/issue/406984/")
		);
		assert_eq!(media.publisher.as_deref(), Some("Puffin Comics"));

		let request = &server.requests()[0];
		assert!(request.starts_with("GET /issue/406984/"), "{request}");
		assert!(
			request
				.to_ascii_lowercase()
				.contains("authorization: basic dxnlcjpwyxnz"),
			"{request}"
		);
	}

	#[tokio::test]
	async fn fetch_series_metadata_maps_status_and_publisher() {
		let server = MockServer::spawn(vec![render_ok(&series_detail_response())]);
		let client = client_at(&server);

		let series = client
			.fetch_series_metadata("51229")
			.await
			.expect("series lookup should succeed");

		assert_eq!(series.external_id, "51229");
		assert_eq!(series.title, "Fantastic Mr Fox");
		assert_eq!(
			series.alternative_titles,
			vec!["Mr Fox le magnifique".to_string()]
		);
		assert_eq!(series.summary.as_deref(), Some("A comic adaptation."));
		assert_eq!(series.status, Some(PublicationStatus::Completed));
		assert_eq!(series.publisher.as_deref(), Some("Puffin Comics"));
		assert_eq!(series.volume_count, Some(30));
		assert_eq!(series.year, Some(1970));
		assert_eq!(series.end_year, Some(1972));
		assert_eq!(
			series.genres.as_deref(),
			Some(["Anthology".to_string()].as_slice())
		);
	}

	#[tokio::test]
	async fn unauthorized_credentials_report_invalid() {
		let unauthorized = "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
		let server = MockServer::spawn(vec![unauthorized.to_string()]);
		let client = MetronClient::new("user:wrong".to_string(), None)
			.with_api_url(server.url.clone())
			.without_retries();

		let verification = client
			.verify_credentials()
			.await
			.expect("verification should not error");

		assert_eq!(verification.response_status, 401);
		assert!(!verification.is_valid);
		assert!(verification.error.is_some());
	}

	#[tokio::test]
	async fn rate_limited_response_maps_to_rate_limited_error() {
		let too_many = "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let server = MockServer::spawn(vec![too_many.to_string()]);
		let client = MetronClient::new("user:pass".to_string(), None)
			.with_api_url(server.url.clone())
			.without_retries();

		let error = client
			.search_media(&query("Fantastic Mr Fox"))
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
Content-Length: 14
Connection: close

{"results": [{"#
			.replace('\n', "\r\n");
		let server = MockServer::spawn(vec![body]);
		let client = client_at(&server);

		let error = client
			.search_media(&query("Fantastic Mr Fox"))
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
	fn empty_token_sends_no_auth_header() {
		assert_eq!(basic_auth_header(""), None);
		assert_eq!(basic_auth_header("   "), None);
	}

	#[test]
	fn preencoded_credential_is_passed_through() {
		assert_eq!(
			basic_auth_header("dXNlcjpwYXNz"),
			Some("Basic dXNlcjpwYXNz".to_string())
		);
		assert_eq!(
			basic_auth_header("Basic dXNlcjpwYXNz"),
			Some("Basic dXNlcjpwYXNz".to_string())
		);
		assert_eq!(
			basic_auth_header("user:pass"),
			Some("Basic dXNlcjpwYXNz".to_string())
		);
	}

	#[test]
	fn status_strings_map_onto_publication_status() {
		assert_eq!(
			parse_status(Some("Ongoing")),
			Some(PublicationStatus::Ongoing)
		);
		assert_eq!(
			parse_status(Some("completed")),
			Some(PublicationStatus::Completed)
		);
		assert_eq!(
			parse_status(Some("Hiatus")),
			Some(PublicationStatus::Hiatus)
		);
		assert_eq!(
			parse_status(Some("Cancelled")),
			Some(PublicationStatus::Cancelled)
		);
		assert_eq!(parse_status(Some("Something Else")), None);
		assert_eq!(parse_status(None), None);
	}

	#[ignore = "Requires network access and Metron credentials"]
	#[tokio::test]
	async fn live_publisher_query() {
		let token = std::env::var("METRON_TOKEN").expect("METRON_TOKEN must be set");
		let client = MetronClient::new(token, None);
		let verification = client.verify_credentials().await.unwrap();
		assert!(verification.is_valid, "{:?}", verification.response_status);
	}
}
