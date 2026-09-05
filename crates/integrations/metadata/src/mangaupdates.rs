//! MangaUpdates metadata provider client.
//!
//! Mirrors the behaviour of Komf's `providers/mangaupdates/` module
//! (`MangaUpdatesClient`, `MangaUpdatesMetadataMapper`,
//! `MangaUpdatesMetadataProvider` at komf master 428dac6): the same series
//! search request shape, HTML description stripping, entity unescaping, year
//! range truncation, status parsing, and category-vote tag ranking.
//!
//! Endpoints (https://api.mangaupdates.com/v1):
//! - `POST /series/search` — body `{ search, page, perpage, type? }`
//! - `GET /series/{id}` — full series detail
//!
//! The API requires no credentials. The search request is filtered by series
//! type following Komf's `mediaType` semantics: the default is the manga
//! filter (MANGA excludes novels); `NOVEL`/`WEBTOON` filters can be selected
//! via the `media_type` search-query hint or [`MangaUpdatesClient::with_media_filter`].

use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	error::MetadataProviderError,
	provider::ProviderCredentialVerification,
	types::{
		ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate, MediaType,
		PublicationStatus, SearchQuery,
	},
	ExternalMetadata, MetadataProvider, RateLimiter, SearchOutcome,
};

/// The shared limiter is per-second; MangaUpdates asks for conservative
/// pacing and Komf defaults its global rate limiter similarly.
const MANGA_UPDATES_DEFAULT_RATE_LIMIT: u32 = 2;

/// Search-query hint key selecting the Komf `mediaType` filter
/// (`MANGA` | `NOVEL` | `WEBTOON`; any other value falls back to `MANGA`).
pub const MEDIA_TYPE_HINT: &str = "media_type";

/// Maximum series-search text length accepted by the API (Komf truncates).
const MAX_SEARCH_LENGTH: usize = 400;

/// Maximum `perpage` accepted by the search endpoint.
const MAX_PER_PAGE: u32 = 100;

const USER_AGENT: &str = concat!(
	"stump/",
	env!("CARGO_PKG_VERSION"),
	" (+https://github.com/stumpapp/stump)"
);

// ---------------------------------------------------------------------------
// Series type filter (Komf's `SeriesType` + media-type lists)
// ---------------------------------------------------------------------------

/// Series `type` values understood by the search endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesType {
	Artbook,
	Doujinshi,
	Filipino,
	Indonesian,
	Manga,
	Manhwa,
	Manhua,
	Oel,
	Thai,
	Vietnamese,
	Malaysian,
	Nordic,
	French,
	Spanish,
	Novel,
}

impl SeriesType {
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Artbook => "Artbook",
			Self::Doujinshi => "Doujinshi",
			Self::Filipino => "Filipino",
			Self::Indonesian => "Indonesian",
			Self::Manga => "Manga",
			Self::Manhwa => "Manhwa",
			Self::Manhua => "Manhua",
			Self::Oel => "OEL",
			Self::Thai => "Thai",
			Self::Vietnamese => "Vietnamese",
			Self::Malaysian => "Malaysian",
			Self::Nordic => "Nordic",
			Self::French => "French",
			Self::Spanish => "Spanish",
			Self::Novel => "Novel",
		}
	}
}

/// Komf's `mangaTypes`: manga-family series, novels excluded.
const MANGA_TYPES: &[SeriesType] = &[
	SeriesType::Manga,
	SeriesType::Manhwa,
	SeriesType::Manhua,
	SeriesType::Artbook,
	SeriesType::Doujinshi,
	SeriesType::Filipino,
	SeriesType::Indonesian,
	SeriesType::Thai,
	SeriesType::Vietnamese,
	SeriesType::Malaysian,
	SeriesType::Oel,
	SeriesType::Nordic,
	SeriesType::French,
	SeriesType::Spanish,
];

const NOVEL_TYPES: &[SeriesType] = &[SeriesType::Novel];

/// Komf's `webtoonTypes`: long-strip family, manga/novels excluded.
const WEBTOON_TYPES: &[SeriesType] = &[
	SeriesType::Manhwa,
	SeriesType::Manhua,
	SeriesType::Filipino,
	SeriesType::Indonesian,
	SeriesType::Thai,
	SeriesType::Vietnamese,
	SeriesType::Malaysian,
	SeriesType::Oel,
	SeriesType::Nordic,
	SeriesType::French,
	SeriesType::Spanish,
];

/// Resolve the Komf `mediaType` semantics onto a search type filter.  The
/// default is the manga filter (novels excluded); `NOVEL` and `WEBTOON`
/// select Komf's novel and webtoon lists.
pub fn series_types_for_media_type(media_type: &str) -> &'static [SeriesType] {
	match media_type.trim().to_ascii_uppercase().as_str() {
		"NOVEL" => NOVEL_TYPES,
		"WEBTOON" => WEBTOON_TYPES,
		_ => MANGA_TYPES,
	}
}

// ---------------------------------------------------------------------------
// Wire models
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SearchRequest<'a> {
	search: &'a str,
	page: u32,
	perpage: u32,
	#[serde(rename = "type")]
	series_types: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResultPage {
	#[serde(default)]
	pub total_hits: i64,
	#[serde(default)]
	pub results: Vec<SearchResultHit>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResultHit {
	#[serde(default)]
	pub record: SearchResult,
}

/// A search hit record.  `year` is a string on the wire (`"2005"` or
/// `"1998-2003"`); `description` is unfiltered HTML.
#[derive(Debug, Default, Deserialize)]
pub struct SearchResult {
	#[serde(rename = "series_id", default)]
	pub series_id: i64,
	#[serde(default)]
	pub title: String,
	#[serde(default)]
	pub description: Option<String>,
	#[serde(default)]
	pub image: Option<MangaUpdatesImage>,
	#[serde(default)]
	pub genres: Option<Vec<MangaUpdatesGenre>>,
	#[serde(default, deserialize_with = "string_or_number_or_null")]
	pub year: Option<String>,
	#[serde(default)]
	pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesSeries {
	#[serde(rename = "series_id", default)]
	pub series_id: i64,
	#[serde(default)]
	pub title: String,
	#[serde(default)]
	pub associated: Vec<MangaUpdatesAssociatedName>,
	#[serde(default)]
	pub description: Option<String>,
	#[serde(default)]
	pub image: Option<MangaUpdatesImage>,
	#[serde(default, deserialize_with = "string_or_number_or_null")]
	pub year: Option<String>,
	#[serde(default)]
	pub genres: Vec<MangaUpdatesGenre>,
	#[serde(default)]
	pub categories: Vec<MangaUpdatesCategory>,
	#[serde(default)]
	pub status: Option<String>,
	#[serde(default)]
	pub authors: Vec<MangaUpdatesAuthor>,
	#[serde(default)]
	pub publishers: Vec<MangaUpdatesPublisher>,
	#[serde(default)]
	pub url: Option<String>,
	#[serde(default, deserialize_with = "string_or_number_or_null")]
	pub latest_chapter: Option<String>,
	#[serde(rename = "bayesian_rating", default)]
	pub bayesian_rating: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesImage {
	#[serde(default)]
	pub url: Option<MangaUpdatesImageUrl>,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesImageUrl {
	#[serde(default)]
	pub original: Option<String>,
	#[serde(default)]
	pub thumb: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesGenre {
	#[serde(default)]
	pub genre: String,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesAssociatedName {
	#[serde(default)]
	pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesCategory {
	#[serde(default)]
	pub category: String,
	#[serde(default)]
	pub votes: i64,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesAuthor {
	#[serde(default)]
	pub name: String,
	#[serde(default, rename = "type")]
	pub author_type: String,
}

#[derive(Debug, Deserialize)]
pub struct MangaUpdatesPublisher {
	#[serde(default, rename = "publisher_name")]
	pub name: String,
	#[serde(default, rename = "type")]
	pub publisher_type: String,
}

// ---------------------------------------------------------------------------
// Text helpers (Komf: unescapeEntities, parseDescription, takeLastYear,
// parseStatus)
// ---------------------------------------------------------------------------

/// The API inconsistently types some string fields (`year`, `latest_chapter`)
/// as numbers or nulls depending on the record; tolerate all three shapes.
fn string_or_number_or_null<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let value = Option::<serde_json::Value>::deserialize(deserializer)?;
	match value {
		None | Some(serde_json::Value::Null) => Ok(None),
		Some(serde_json::Value::String(value)) => Ok(Some(value)),
		Some(serde_json::Value::Number(value)) => Ok(Some(value.to_string())),
		Some(_) => Err(serde::de::Error::custom(
			"expected a string, number, or null",
		)),
	}
}

/// Decode the named and numeric HTML entities Komf unescapes on every text
/// field.  Unknown entities are left verbatim.
fn unescape_entities(input: &str) -> String {
	if !input.contains('&') {
		return input.to_string();
	}
	let mut out = String::with_capacity(input.len());
	let mut rest = input;
	while let Some(start) = rest.find('&') {
		out.push_str(&rest[..start]);
		let tail = &rest[start..];
		let Some(end) = tail.find(';') else {
			out.push('&');
			rest = &rest[start + 1..];
			continue;
		};
		let entity = &tail[1..end];
		let decoded = if let Some(digits) = entity
			.strip_prefix('#')
			.and_then(|d| d.strip_prefix(|c: char| c == 'x' || c == 'X'))
		{
			u32::from_str_radix(digits, 16)
				.ok()
				.and_then(char::from_u32)
		} else if let Some(digits) = entity.strip_prefix('#') {
			digits.parse::<u32>().ok().and_then(char::from_u32)
		} else {
			match entity {
				"amp" => Some('&'),
				"lt" => Some('<'),
				"gt" => Some('>'),
				"quot" => Some('"'),
				"apos" => Some('\''),
				"nbsp" => Some('\u{00a0}'),
				_ => None,
			}
		};
		match decoded {
			Some(decoded) => {
				out.push(decoded);
				rest = &tail[end + 1..];
			},
			None => {
				out.push('&');
				rest = &rest[start + 1..];
			},
		}
	}
	out.push_str(rest);
	out
}

/// Render a provider HTML description to plain text: tags are removed,
/// `<br>` becomes a newline, and entities are decoded (Komf's
/// `parseDescription`).
fn parse_description(html: &str) -> String {
	let mut out = String::with_capacity(html.len());
	let mut rest = html;
	while let Some(start) = rest.find('<') {
		out.push_str(&unescape_entities(&rest[..start]));
		let tail = &rest[start..];
		let Some(offset) = tail.find('>') else {
			// Unterminated tag: keep the remainder verbatim.
			out.push_str(&unescape_entities(tail));
			rest = "";
			break;
		};
		let tag = tail[1..offset].trim().to_ascii_lowercase();
		if tag == "br" || tag.starts_with("br/") || tag.starts_with("br ") {
			out.push('\n');
		}
		rest = &tail[offset + 1..];
	}
	out.push_str(&unescape_entities(rest));
	out
}

/// Komf's `takeLastYear`: a `"1998-2003"` range collapses to its first year.
fn take_last_year(year: &str) -> &str {
	if let Some(index) = year.rfind('-') {
		let suffix = &year[index + 1..];
		if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
			return &year[..index];
		}
	}
	year
}

fn parse_year(year: Option<&str>) -> Option<i32> {
	year.map(take_last_year)
		.and_then(|year| year.trim().parse().ok())
}

/// Komf's `parseStatus`: status strings carry a parenthesised marker (e.g.
/// `"2 Volumes (Complete)"`); the marker is only trusted when every
/// parenthesised group contains the first one.
fn parse_status(status: Option<&str>) -> Option<PublicationStatus> {
	let status = status?;
	let mut groups = Vec::new();
	let mut rest = status;
	while let (Some(open), after_open) = (rest.find('('), rest) {
		let tail = &after_open[open + 1..];
		let Some(close) = tail.find(')') else {
			break;
		};
		groups.push(&tail[..close]);
		rest = &tail[close + 1..];
	}
	let first = *groups.first()?;
	if groups.iter().any(|group| !group.contains(first)) {
		return None;
	}
	match first.trim().to_ascii_uppercase().as_str() {
		"COMPLETE" => Some(PublicationStatus::Completed),
		"ONGOING" => Some(PublicationStatus::Ongoing),
		"CANCELLED" => Some(PublicationStatus::Cancelled),
		"HIATUS" => Some(PublicationStatus::Hiatus),
		_ => None,
	}
}

fn strip_novel_suffix(title: &str) -> &str {
	title.strip_suffix(" (Novel)").unwrap_or(title)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct MangaUpdatesClient {
	client: ClientWithMiddleware,
	api_url: String,
	rate_limiter: RateLimiter,
	media_filter: &'static [SeriesType],
}

impl MangaUpdatesClient {
	const API_URL: &'static str = "https://api.mangaupdates.com/v1";

	pub fn new() -> Self {
		Self::with_media_filter(MANGA_TYPES)
	}

	/// Build the client with a fixed Komf `mediaType` search filter.  The
	/// [`MEDIA_TYPE_HINT`] search hint overrides it per request.
	pub fn with_media_filter(media_filter: &'static [SeriesType]) -> Self {
		Self {
			client: build_client_with_retry(
				reqwest::Client::builder()
					.user_agent(USER_AGENT)
					.build()
					.expect("Failed to build MangaUpdates HTTP client"),
				RetryClientConfig::default(),
			),
			api_url: Self::API_URL.to_string(),
			rate_limiter: RateLimiter::new(MANGA_UPDATES_DEFAULT_RATE_LIMIT),
			media_filter,
		}
	}

	/// Test-only override of the API base URL, used to point the client at a
	/// local mock server.
	#[cfg(test)]
	fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
		self.api_url = api_url.into();
		self
	}

	/// Test-only retry override so canned error responses do not burn
	/// exponential-backoff retries.
	#[cfg(test)]
	fn without_retries(mut self) -> Self {
		self.client = build_client_with_retry(
			reqwest::Client::builder()
				.user_agent(USER_AGENT)
				.build()
				.expect("Failed to build MangaUpdates HTTP client"),
			RetryClientConfig { max_retries: 0 },
		);
		self
	}

	fn series_types(&self, query: &SearchQuery) -> Vec<&'static str> {
		let filter = query
			.provider_hints
			.get(MEDIA_TYPE_HINT)
			.map(|hint| series_types_for_media_type(hint))
			.unwrap_or(self.media_filter);
		filter.iter().map(SeriesType::as_str).collect()
	}

	async fn search_series_page(
		&self,
		name: &str,
		series_types: Vec<&'static str>,
		limit: u32,
	) -> Result<SearchResultPage, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let body = SearchRequest {
			search: &name.chars().take(MAX_SEARCH_LENGTH).collect::<String>(),
			page: 1,
			perpage: limit.clamp(1, MAX_PER_PAGE),
			series_types,
		};

		let response = self
			.client
			.post(format!("{}/series/search", self.api_url))
			.json(&body)
			.send()
			.await?
			.error_for_status()?
			.json::<SearchResultPage>()
			.await?;
		Ok(response)
	}

	async fn fetch_series(
		&self,
		series_id: i64,
	) -> Result<MangaUpdatesSeries, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let response = self
			.client
			.get(format!("{}/series/{}", self.api_url, series_id))
			.send()
			.await?
			.error_for_status()?
			.json::<MangaUpdatesSeries>()
			.await?;
		Ok(response)
	}

	fn search_candidate(&self, record: SearchResult) -> MatchCandidate {
		let external_id = record.series_id.to_string();
		let metadata = ExternalMediaMetadata {
			provider: self.id().to_string(),
			external_id: external_id.clone(),
			title: Some(unescape_entities(&record.title)),
			summary: record.description.as_deref().map(parse_description),
			year: parse_year(record.year.as_deref()),
			genres: record.genres.map(|genres| {
				genres
					.into_iter()
					.map(|genre| unescape_entities(&genre.genre))
					.collect()
			}),
			cover_url: record
				.image
				.and_then(|image| image.url)
				.and_then(|url| url.original),
			provider_url: record.url,
			..Default::default()
		};
		MatchCandidate {
			external_id,
			metadata: ExternalMetadata::Media(metadata),
			provider: self.id().to_string(),
			confidence: 0.0,
			confidence_factors: Vec::new(),
		}
	}

	fn series_candidate(&self, record: SearchResult) -> MatchCandidate {
		let external_id = record.series_id.to_string();
		let metadata = ExternalSeriesMetadata {
			provider: self.id().to_string(),
			external_id: external_id.clone(),
			title: unescape_entities(&record.title),
			summary: record.description.as_deref().map(parse_description),
			year: parse_year(record.year.as_deref()),
			genres: record.genres.map(|genres| {
				genres
					.into_iter()
					.map(|genre| unescape_entities(&genre.genre))
					.collect()
			}),
			cover_url: record
				.image
				.and_then(|image| image.url)
				.and_then(|url| url.original),
			..Default::default()
		};
		MatchCandidate {
			external_id,
			metadata: ExternalMetadata::Series(metadata),
			provider: self.id().to_string(),
			confidence: 0.0,
			confidence_factors: Vec::new(),
		}
	}
}

impl Default for MangaUpdatesClient {
	fn default() -> Self {
		Self::new()
	}
}

#[async_trait::async_trait]
impl MetadataProvider for MangaUpdatesClient {
	fn id(&self) -> &'static str {
		"mangaupdates"
	}

	fn name(&self) -> &'static str {
		"MangaUpdates"
	}

	/// Komf's comic media type is unsupported; novels are only reachable via
	/// the `NOVEL` filter, so only the manga-family types are advertised.
	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Manga, MediaType::Manhwa, MediaType::Webtoon]
	}

	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		tracing::trace!(title = %query.title, "Searching for series on MangaUpdates");
		let limit = query.limit.unwrap_or(10);
		let page = self
			.search_series_page(&query.title, self.series_types(query), limit)
			.await?;

		let requested = page.results.len();
		let candidates = page
			.results
			.into_iter()
			.take(limit as usize)
			.map(|hit| self.series_candidate(hit.record))
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		tracing::trace!(title = %query.title, "Searching for media on MangaUpdates");
		let limit = query.limit.unwrap_or(10);
		let page = self
			.search_series_page(&query.title, self.series_types(query), limit)
			.await?;

		let requested = page.results.len();
		let candidates = page
			.results
			.into_iter()
			.take(limit as usize)
			.map(|hit| self.search_candidate(hit.record))
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	/// MangaUpdates is a series-level catalog; there is no per-book record, so
	/// the series detail doubles as the media record (number falls back to the
	/// latest published chapter).
	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let series_id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!("Invalid series ID: {}", external_id))
		})?;
		let series = self.fetch_series(series_id).await?;

		let mut writers = Vec::new();
		let mut artists = Vec::new();
		for author in &series.authors {
			// Komf: type "Author" maps to author roles, everything else to
			// artist roles.
			if author.author_type == "Author" {
				writers.push(unescape_entities(&author.name));
			} else {
				artists.push(unescape_entities(&author.name));
			}
		}

		let genres: Vec<String> = series
			.genres
			.iter()
			.map(|genre| unescape_entities(&genre.genre))
			.collect();
		let tags: Vec<String> = ranked_category_tags(&series.categories);

		Ok(ExternalMediaMetadata {
			provider: self.id().to_string(),
			external_id: series.series_id.to_string(),
			title: Some(
				strip_novel_suffix(&unescape_entities(&series.title)).to_string(),
			),
			summary: series.description.as_deref().map(parse_description),
			year: parse_year(series.year.as_deref()),
			number: series
				.latest_chapter
				.as_deref()
				.and_then(|chapter| chapter.trim().parse().ok()),
			writers: (!writers.is_empty()).then_some(writers),
			artists: (!artists.is_empty()).then_some(artists),
			genres: (!genres.is_empty()).then_some(genres),
			tags: (!tags.is_empty()).then_some(tags),
			cover_url: series
				.image
				.as_ref()
				.and_then(|image| image.url.as_ref())
				.and_then(|url| url.original.clone()),
			provider_url: series.url,
			..Default::default()
		})
	}

	async fn fetch_series_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		let series_id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!("Invalid series ID: {}", external_id))
		})?;
		let series = self.fetch_series(series_id).await?;

		let mut authors = Vec::new();
		let mut artists = Vec::new();
		for author in &series.authors {
			if author.author_type == "Author" {
				authors.push(unescape_entities(&author.name));
			} else {
				artists.push(unescape_entities(&author.name));
			}
		}

		// Komf prefers the original publisher (SeriesMetadataConfig defaults
		// to useOriginalPublisher) and falls back to the English one.
		let publisher = series
			.publishers
			.iter()
			.find(|publisher| publisher.publisher_type == "Original")
			.or_else(|| {
				series
					.publishers
					.iter()
					.find(|publisher| publisher.publisher_type == "English")
			})
			.map(|publisher| unescape_entities(&publisher.name));

		let genres: Vec<String> = series
			.genres
			.iter()
			.map(|genre| unescape_entities(&genre.genre))
			.collect();
		let tags: Vec<String> = ranked_category_tags(&series.categories);

		Ok(ExternalSeriesMetadata {
			provider: self.id().to_string(),
			external_id: series.series_id.to_string(),
			title: strip_novel_suffix(&unescape_entities(&series.title)).to_string(),
			alternative_titles: series
				.associated
				.iter()
				.map(|name| unescape_entities(&name.title))
				.collect(),
			summary: series.description.as_deref().map(parse_description),
			status: parse_status(series.status.as_deref()),
			year: parse_year(series.year.as_deref()),
			end_year: None,
			age_rating: None,
			genres: (!genres.is_empty()).then_some(genres),
			tags: (!tags.is_empty()).then_some(tags),
			authors: (!authors.is_empty()).then_some(authors),
			artists: (!artists.is_empty()).then_some(artists),
			publisher,
			cover_url: series
				.image
				.as_ref()
				.and_then(|image| image.url.as_ref())
				.and_then(|url| url.original.clone()),
			volume_count: None,
		})
	}

	/// The API requires no credentials, so verification is a formality.
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		Ok(ProviderCredentialVerification {
			response_status: 200,
			is_valid: true,
			error: None,
		})
	}
}

/// Komf's tag derivation: categories ranked by vote count, top 15 kept.
fn ranked_category_tags(categories: &[MangaUpdatesCategory]) -> Vec<String> {
	let mut ranked: Vec<&MangaUpdatesCategory> = categories.iter().collect();
	ranked.sort_by(|left, right| right.votes.cmp(&left.votes));
	ranked
		.into_iter()
		.take(15)
		.map(|category| unescape_entities(&category.category))
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::mock_http::{render_ok, MockServer};

	fn test_client(server: &MockServer) -> MangaUpdatesClient {
		MangaUpdatesClient::new()
			.with_api_url(server.url.clone())
			.without_retries()
	}

	fn test_query(title: &str) -> SearchQuery {
		SearchQuery {
			title: title.to_string(),
			limit: Some(5),
			..Default::default()
		}
	}

	#[test]
	fn take_last_year_collapses_ranges() {
		assert_eq!(take_last_year("2005"), "2005");
		assert_eq!(take_last_year("1998-2003"), "1998");
		assert_eq!(take_last_year(""), "");
		assert_eq!(take_last_year("2nd-print-2020"), "2nd-print");
	}

	#[test]
	fn status_marker_is_parsed_like_komf() {
		assert_eq!(
			parse_status(Some("2 Volumes (Complete)")),
			Some(PublicationStatus::Completed)
		);
		assert_eq!(
			parse_status(Some("(Ongoing)")),
			Some(PublicationStatus::Ongoing)
		);
		assert_eq!(
			parse_status(Some("(Hiatus)")),
			Some(PublicationStatus::Hiatus)
		);
		assert_eq!(
			parse_status(Some("(Cancelled)")),
			Some(PublicationStatus::Cancelled)
		);
		// No parenthesised marker -> unmapped, exactly like Komf.
		assert_eq!(parse_status(Some("Complete")), None);
		// Conflicting markers -> unmapped.
		assert_eq!(parse_status(Some("(Ongoing) and (Complete)")), None);
		assert_eq!(parse_status(None), None);
	}

	#[test]
	fn descriptions_are_rendered_as_plain_text() {
		assert_eq!(
			parse_description("<p>A &amp; B<br>cold &#8212; night</p>"),
			"A & B\ncold — night"
		);
		assert_eq!(parse_description("plain text"), "plain text");
	}

	#[test]
	fn media_type_filters_match_komf_lists() {
		assert!(series_types_for_media_type("MANGA").contains(&SeriesType::Manga));
		assert!(!series_types_for_media_type("MANGA").contains(&SeriesType::Novel));
		assert_eq!(series_types_for_media_type("NOVEL"), NOVEL_TYPES);
		assert_eq!(series_types_for_media_type("WEBTOON"), WEBTOON_TYPES);
		assert_eq!(series_types_for_media_type("whatever"), MANGA_TYPES);
	}

	#[tokio::test]
	async fn search_media_maps_mocked_search_response() {
		let body = serde_json::json!({
			"total_hits": 1,
			"page": 1,
			"per_page": 5,
			"results": [
				{
					"hit_title": "berserk",
					"record": {
						"series_id": 1106,
						"title": "Berserk &amp; the Band of the Hawk",
						"description": "<p>A &#8220;dark&#8221; fantasy.<br>Second line.</p>",
						"image": { "url": { "original": "https://www.mangaupdates.comcovers/original.jpg", "thumb": "https://thumb.jpg" } },
						"genres": [{ "genre": "Action" }],
						"year": "1989-2021",
						"url": "https://www.mangaupdates.com/series.html?id=1106"
					}
				}
			]
		})
		.to_string();

		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = test_client(&server);

		let outcome = client
			.search_media(&test_query("Berserk"))
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "mangaupdates");
		assert_eq!(candidate.external_id, "1106");

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(
			media.title.as_deref(),
			Some("Berserk & the Band of the Hawk")
		);
		assert_eq!(
			media.summary.as_deref(),
			Some("A \u{201c}dark\u{201d} fantasy.\nSecond line.")
		);
		assert_eq!(media.year, Some(1989));
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://www.mangaupdates.comcovers/original.jpg")
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://www.mangaupdates.com/series.html?id=1106")
		);
		assert_eq!(
			media.genres.as_deref(),
			Some(["Action".to_string()].as_slice())
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(requests[0].starts_with("POST /series/search"));
		assert!(requests[0].contains("user-agent: stump/"));
		// Default filter is Komf's manga list; novels are excluded.
		assert!(requests[0].contains(r#""type":["Manga""#));
		assert!(!requests[0].contains("\"Novel\""));
		assert!(requests[0].contains(r#""search":"Berserk""#));
		assert!(requests[0].contains(r#""page":1"#));
		assert!(requests[0].contains(r#""perpage":5"#));
	}

	#[tokio::test]
	async fn search_series_media_type_hint_overrides_filter() {
		let body = serde_json::json!({ "results": [] }).to_string();
		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = test_client(&server);

		let mut query = test_query("Ascendance of a Bookworm");
		query
			.provider_hints
			.insert(MEDIA_TYPE_HINT.to_string(), "NOVEL".to_string());
		client
			.search_series(&query)
			.await
			.expect("hinted search should succeed");

		let request = &server.requests()[0];
		assert!(request.contains(r#""type":["Novel"]"#));
	}

	#[tokio::test]
	async fn fetch_series_maps_mocked_detail_response() {
		let body = serde_json::json!({
			"series_id": 1106,
			"title": "Berserk (Novel)",
			"associated": [{ "title": "ベルセルク" }, { "title": "Berserk &#8212; Kanji" }],
			"description": "<p>Guts, a &amp; lone swordsman.<br>Dark fantasy.</p>",
			"image": { "url": { "original": "https://cover.original.jpg", "thumb": "https://cover.thumb.jpg" } },
			"year": "1989-2021",
			"genres": [{ "genre": "Action" }, { "genre": "Horror" }],
			"categories": [
				{ "category": "Low Votes", "votes": 1 },
				{ "category": "Many Votes", "votes": 42 }
			],
			"status": "37 Volumes (Complete)",
			"authors": [
				{ "author_id": 926, "name": "Kentaro Miura", "type": "Author" },
				{ "author_id": 61379, "name": "Studio Gaga", "type": "Artist" }
			],
			"publishers": [
				{ "publisher_id": 2994, "publisher_name": "Hakusensha", "type": "Original", "notes": null },
				{ "publisher_id": 2995, "publisher_name": "Dark Horse", "type": "English", "notes": null }
			],
			"url": "https://www.mangaupdates.com/series.html?id=1106",
			"bayesian_rating": 9.1,
			"latest_chapter": "364",
			"type": "Manga",
			"completed": true
		})
		.to_string();

		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = test_client(&server);

		let series = client
			.fetch_series_metadata("1106")
			.await
			.expect("series fetch should succeed against the mock");

		assert_eq!(series.provider, "mangaupdates");
		assert_eq!(series.external_id, "1106");
		// Komf strips the novel suffix from the canonical title.
		assert_eq!(series.title, "Berserk");
		assert_eq!(
			series.alternative_titles,
			vec!["ベルセルク".to_string(), "Berserk — Kanji".to_string()]
		);
		assert_eq!(
			series.summary.as_deref(),
			Some("Guts, a & lone swordsman.\nDark fantasy.")
		);
		assert_eq!(series.status, Some(PublicationStatus::Completed));
		assert_eq!(series.year, Some(1989));
		assert_eq!(
			series.genres.as_deref(),
			Some(["Action".to_string(), "Horror".to_string()].as_slice())
		);
		// Categories are ranked by votes and truncated to the top 15.
		assert_eq!(
			series.tags.as_deref(),
			Some(["Many Votes".to_string(), "Low Votes".to_string()].as_slice())
		);
		assert_eq!(
			series.authors.as_deref(),
			Some(["Kentaro Miura".to_string()].as_slice())
		);
		assert_eq!(
			series.artists.as_deref(),
			Some(["Studio Gaga".to_string()].as_slice())
		);
		assert_eq!(series.publisher.as_deref(), Some("Hakusensha"));
		assert_eq!(
			series.cover_url.as_deref(),
			Some("https://cover.original.jpg")
		);
	}

	#[tokio::test]
	async fn fetch_media_maps_mocked_detail_response() {
		let body = serde_json::json!({
		"series_id": 1106,
		"title": "Berserk (Novel)",
		"description": "<p>Guts, a &amp; lone swordsman.<br>Dark fantasy.</p>",
		"image": { "url": { "original": "https://cover.original.jpg", "thumb": "https://cover.thumb.jpg" } },
		"year": "1989-2021",
		"genres": [{ "genre": "Action" }],
		"categories": [{ "category": "Many Votes", "votes": 42 }],
		"status": "37 Volumes (Complete)",
		"authors": [
			{ "author_id": 926, "name": "Kentaro Miura", "type": "Author" },
			{ "author_id": 61379, "name": "Studio Gaga", "type": "Artist" }
		],
		"publishers": [
			{ "publisher_id": 2994, "publisher_name": "Hakusensha", "type": "Original", "notes": null }
		],
		"url": "https://www.mangaupdates.com/series.html?id=1106",
		"latest_chapter": "364"
	})
	.to_string();

		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = test_client(&server);

		let media = client
			.fetch_media_metadata("1106")
			.await
			.expect("media fetch should succeed against the mock");

		// The media record is derived from the same series detail endpoint.
		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(requests[0].starts_with("GET /series/1106"));
		assert_eq!(media.provider, "mangaupdates");
		assert_eq!(media.title.as_deref(), Some("Berserk"));
		assert_eq!(media.number, Some(364.0));
		assert_eq!(
			media.writers.as_deref(),
			Some(["Kentaro Miura".to_string()].as_slice())
		);
		assert_eq!(
			media.artists.as_deref(),
			Some(["Studio Gaga".to_string()].as_slice())
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://www.mangaupdates.com/series.html?id=1106")
		);
	}

	#[tokio::test]
	async fn fetch_rejects_non_numeric_ids() {
		let server = MockServer::spawn(vec![]);
		let client = test_client(&server);
		let error = client
			.fetch_series_metadata("not-a-number")
			.await
			.expect_err("non-numeric id should fail before any request");
		assert!(matches!(error, MetadataProviderError::Other(_)));
		assert!(server.requests().is_empty());
	}

	#[tokio::test]
	async fn rate_limited_response_maps_to_rate_limited() {
		let response = "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let server = MockServer::spawn(vec![response.to_string()]);
		let client = test_client(&server);

		let error = client
			.search_media(&test_query("Berserk"))
			.await
			.expect_err("429 should surface as an error");
		assert!(error.is_rate_limited());
	}

	#[tokio::test]
	async fn malformed_json_maps_to_parse_error() {
		let server = MockServer::spawn(vec![render_ok("<html>not json</html>")]);
		let client = test_client(&server);

		let error = client
			.search_media(&test_query("Berserk"))
			.await
			.expect_err("malformed JSON should surface as an error");
		// reqwest surfaces JSON decode failures as ReqwestError (decode); both
		// variants map to ProviderError::Request in the facade, and neither is a
		// rate limit.
		assert!(!error.is_rate_limited());
		assert!(matches!(
			error,
			MetadataProviderError::ParseError(_) | MetadataProviderError::ReqwestError(_)
		));
	}

	#[ignore = "Hits the live MangaUpdates API"]
	#[tokio::test]
	async fn live_search_and_fetch() {
		let client = MangaUpdatesClient::new();
		let outcome = client
			.search_media(&test_query("Berserk"))
			.await
			.expect("live search should succeed");
		assert!(!outcome.candidates.is_empty());

		let external_id = outcome.candidates[0].external_id.clone();
		let media = client
			.fetch_media_metadata(&external_id)
			.await
			.expect("live fetch should succeed");
		assert!(media.title.is_some());
		println!("live media metadata: {media:#?}");
	}
}
