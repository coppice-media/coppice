//! Open Library metadata provider (https://openlibrary.org).
//!
//! Keyless. Identification is ISBN-first: `GET /isbn/{isbn}.json` resolves an
//! edition (`OL…M`), which is expanded through its work (`GET /works/{id}.json`)
//! and authors (`GET /authors/{id}.json`). Without an ISBN hit the client falls
//! back to `GET /search.json`, whose documents already carry enough fields to
//! build a candidate without per-hit detail fetches. Covers come from
//! `https://covers.openlibrary.org/b/id/{cover_id}-L.jpg`.
//!
//! Open Library has no series entity, so the series half of the trait reports
//! `OperationNotSupported`; the ingest facade tolerates that as long as the
//! media search succeeds.
//!
//! External ids are bare Open Library keys: an edition (`OL7353617M`) when the
//! record was resolved by ISBN or a search hit exposes a cover edition, else a
//! work (`OL45804W`).

use reqwest::Url;
use reqwest_middleware::ClientWithMiddleware;
use serde::Deserialize;

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	editions::{EditionIdentifier, EditionLookup},
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

/// Open Library asks clients to identify themselves and stay near one
/// request per second (https://openlibrary.org/developers/api).
const OPEN_LIBRARY_DEFAULT_RATE_LIMIT: u32 = 1;

/// The search endpoint caps `limit` at 100.
const OPEN_LIBRARY_MAX_LIMIT: u32 = 100;

/// Authors expanded per record; keeps a many-author anthology from burning
/// the whole per-second budget on one lookup.
const OPEN_LIBRARY_MAX_AUTHOR_FETCHES: usize = 5;

/// Subjects are unbounded and noisy; keep the leading few as tags.
const OPEN_LIBRARY_MAX_SUBJECTS: usize = 10;

/// Editions fetched per work when expanding an edition list for pairing. An
/// identifier that is not in the first page of a work's editions is not going
/// to decide a pair, and the budget is one request per second.
const OPEN_LIBRARY_MAX_WORK_EDITIONS: u32 = 100;

/// Fields requested from the search endpoint; anything else is dead weight.
const SEARCH_FIELDS: &str = "key,title,subtitle,author_name,author_key,first_publish_year,isbn,cover_i,cover_edition_key,number_of_pages_median,subject";

pub struct OpenLibraryClient {
	client: ClientWithMiddleware,
	api_url: String,
	rate_limiter: RateLimiter,
}

impl Default for OpenLibraryClient {
	fn default() -> Self {
		Self::new()
	}
}

impl OpenLibraryClient {
	const API_URL: &'static str = "https://openlibrary.org";
	const COVER_BASE_URL: &'static str = "https://covers.openlibrary.org/b/id";
	const USER_AGENT: &'static str = concat!(
		"stump/",
		env!("CARGO_PKG_VERSION"),
		" (+https://github.com/stumpapp/stump)"
	);

	/// Open Library requires no API key, so there is nothing to configure.
	pub fn new() -> Self {
		Self::build(
			Self::API_URL.to_string(),
			OPEN_LIBRARY_DEFAULT_RATE_LIMIT,
			RetryClientConfig::default(),
		)
	}

	fn build(api_url: String, rate_limit: u32, retry: RetryClientConfig) -> Self {
		let inner = reqwest::Client::builder()
			.user_agent(Self::USER_AGENT)
			.build()
			.expect("Failed to build Open Library HTTP client"); // static config, cannot fail
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

	/// Point the base at one local server, for tests in crates that consume
	/// [`crate::editions::EditionLookup`] and cannot reach the private
	/// override above.
	#[cfg(any(test, feature = "mock"))]
	#[must_use]
	pub fn pointed_at(mut self, api_url: &str) -> Self {
		self.api_url = api_url.to_string();
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
				.expect("Failed to build Open Library HTTP client"),
			RetryClientConfig { max_retries: 0 },
		);
		self.rate_limiter = RateLimiter::new(u32::MAX);
		self
	}

	/// GET a JSON document. A 404 surfaces as [`MetadataProviderError::NotFound`]
	/// so ISBN misses can fall back to search.
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

		let response = self.client.get(url).send().await?;
		if response.status() == reqwest::StatusCode::NOT_FOUND {
			return Err(MetadataProviderError::NotFound(path.to_string()));
		}
		Ok(response.error_for_status()?.json().await?)
	}

	async fn fetch_edition(
		&self,
		key: &str,
	) -> Result<OpenLibraryEdition, MetadataProviderError> {
		self.get(&format!("/books/{key}.json"), &[]).await
	}

	async fn fetch_edition_by_isbn(
		&self,
		isbn: &str,
	) -> Result<OpenLibraryEdition, MetadataProviderError> {
		self.get(&format!("/isbn/{isbn}.json"), &[]).await
	}

	async fn fetch_work(
		&self,
		key: &str,
	) -> Result<OpenLibraryWork, MetadataProviderError> {
		self.get(&format!("/works/{key}.json"), &[]).await
	}

	/// Every edition catalogued under a work. The endpoint paginates at 50 by
	/// default; one page of [`OPEN_LIBRARY_MAX_WORK_EDITIONS`] is enough to
	/// decide a pairing and keeps a much-reprinted classic from costing a
	/// dozen requests against a one-per-second budget.
	async fn fetch_work_editions(
		&self,
		key: &str,
	) -> Result<Vec<OpenLibraryEdition>, MetadataProviderError> {
		let limit = OPEN_LIBRARY_MAX_WORK_EDITIONS.to_string();
		let response: OpenLibraryWorkEditions = self
			.get(
				&format!("/works/{key}/editions.json"),
				&[("limit", limit.as_str())],
			)
			.await?;
		Ok(response.entries)
	}

	/// Resolve author keys to display names, tolerating individual misses.
	async fn fetch_author_names(&self, keys: &[String]) -> Vec<String> {
		let mut names =
			Vec::with_capacity(keys.len().min(OPEN_LIBRARY_MAX_AUTHOR_FETCHES));
		for key in keys.iter().take(OPEN_LIBRARY_MAX_AUTHOR_FETCHES) {
			let key = strip_prefix(key);
			match self
				.get::<OpenLibraryAuthor>(&format!("/authors/{key}.json"), &[])
				.await
			{
				Ok(author) => {
					if let Some(name) = author.name.filter(|name| !name.trim().is_empty())
					{
						names.push(name);
					}
				},
				Err(error) => tracing::warn!(
					author = key,
					?error,
					"Open Library author lookup failed; skipping"
				),
			}
		}
		names
	}

	async fn search_docs(
		&self,
		query: &SearchQuery,
	) -> Result<Vec<OpenLibrarySearchDoc>, MetadataProviderError> {
		let limit = query
			.limit
			.unwrap_or(10)
			.clamp(1, OPEN_LIBRARY_MAX_LIMIT)
			.to_string();
		let mut params = vec![
			("q", query.title.as_str()),
			("limit", limit.as_str()),
			("fields", SEARCH_FIELDS),
		];
		if let Some(author) = query.author.as_deref().filter(|a| !a.trim().is_empty()) {
			params.push(("author", author));
		}
		let response: OpenLibrarySearchResponse =
			self.get("/search.json", &params).await?;
		Ok(response.docs)
	}

	/// Expand an edition into media metadata: the work supplies the
	/// description and (as a fallback) authors, author records supply names.
	async fn media_from_edition(
		&self,
		edition: OpenLibraryEdition,
		query_isbn: Option<&str>,
	) -> ExternalMediaMetadata {
		let work = match edition.works.first() {
			Some(reference) => {
				match self.fetch_work(strip_prefix(&reference.key)).await {
					Ok(work) => Some(work),
					Err(error) => {
						tracing::warn!(
							work = reference.key,
							?error,
							"Open Library work lookup failed; using edition only"
						);
						None
					},
				}
			},
			None => None,
		};
		let author_keys: Vec<String> = if edition.authors.is_empty() {
			work.as_ref()
				.map(|work| {
					work.authors
						.iter()
						.filter_map(|entry| entry.author.as_ref().map(|a| a.key.clone()))
						.collect()
				})
				.unwrap_or_default()
		} else {
			edition.authors.iter().map(|a| a.key.clone()).collect()
		};
		let writers = self.fetch_author_names(&author_keys).await;

		let external_id = strip_prefix(&edition.key).to_string();
		let (year, month, day) = parse_publish_date(edition.publish_date.as_deref());
		let cover_id = first_cover(&edition.covers)
			.or_else(|| work.as_ref().and_then(|work| first_cover(&work.covers)));
		let summary = edition
			.description
			.as_ref()
			.and_then(OpenLibraryText::text)
			.or_else(|| {
				work.as_ref()
					.and_then(|w| w.description.as_ref())
					.and_then(OpenLibraryText::text)
			});
		let subjects = if edition.subjects.is_empty() {
			work.as_ref()
				.map(|w| w.subjects.clone())
				.unwrap_or_default()
		} else {
			edition.subjects
		};
		let (isbn, isbn_13) = pick_isbns(&edition.isbn_10, &edition.isbn_13, query_isbn);

		ExternalMediaMetadata {
			provider: "openlibrary".to_string(),
			external_id: external_id.clone(),
			title: non_empty(edition.title),
			summary,
			page_count: edition.number_of_pages,
			series_name: edition.series.into_iter().next(),
			year,
			month,
			day,
			tags: filled_or_none(
				subjects
					.into_iter()
					.take(OPEN_LIBRARY_MAX_SUBJECTS)
					.collect(),
			),
			isbn,
			isbn_13,
			writers: filled_or_none(writers),
			cover_url: cover_id.map(cover_url),
			provider_url: Some(format!("https://openlibrary.org/books/{external_id}")),
			..Default::default()
		}
	}

	/// Work-level media metadata for ids that never resolved to an edition.
	async fn media_from_work(&self, work: OpenLibraryWork) -> ExternalMediaMetadata {
		let author_keys: Vec<String> = work
			.authors
			.iter()
			.filter_map(|entry| entry.author.as_ref().map(|a| a.key.clone()))
			.collect();
		let writers = self.fetch_author_names(&author_keys).await;
		let external_id = strip_prefix(&work.key).to_string();
		let (year, _, _) = parse_publish_date(work.first_publish_date.as_deref());

		ExternalMediaMetadata {
			provider: "openlibrary".to_string(),
			external_id: external_id.clone(),
			title: non_empty(work.title),
			summary: work.description.as_ref().and_then(OpenLibraryText::text),
			year,
			tags: filled_or_none(
				work.subjects
					.into_iter()
					.take(OPEN_LIBRARY_MAX_SUBJECTS)
					.collect(),
			),
			writers: filled_or_none(writers),
			cover_url: first_cover(&work.covers).map(cover_url),
			provider_url: Some(format!("https://openlibrary.org/works/{external_id}")),
			..Default::default()
		}
	}
}

#[async_trait::async_trait]
impl MetadataProvider for OpenLibraryClient {
	fn id(&self) -> &'static str {
		"openlibrary"
	}

	fn name(&self) -> &'static str {
		"Open Library"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Book]
	}

	/// Open Library has no series entity.
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
		if let Some(isbn) = query
			.isbn
			.as_deref()
			.map(normalize_isbn)
			.filter(|isbn| !isbn.is_empty())
		{
			match self.fetch_edition_by_isbn(&isbn).await {
				Ok(edition) => {
					let metadata = self.media_from_edition(edition, Some(&isbn)).await;
					let candidate = MatchCandidate {
						external_id: metadata.external_id.clone(),
						metadata: ExternalMetadata::Media(metadata),
						provider: self.id().to_string(),
						confidence: 0.0,
						confidence_factors: Vec::new(),
					};
					return Ok(SearchOutcome {
						candidates: self.score_search(query, vec![candidate]),
						requested: 1,
					});
				},
				Err(MetadataProviderError::NotFound(_)) => {
					tracing::debug!(
						isbn,
						"No Open Library edition for ISBN; falling back to search"
					);
				},
				Err(error) => return Err(error),
			}
		}

		let docs = self.search_docs(query).await?;
		let requested = docs.len();
		let query_isbn = query.isbn.as_deref().map(normalize_isbn);
		let candidates = docs
			.into_iter()
			.filter_map(|doc| {
				let metadata =
					media_from_search_doc(self.id(), doc, query_isbn.as_deref())?;
				Some(MatchCandidate {
					external_id: metadata.external_id.clone(),
					metadata: ExternalMetadata::Media(metadata),
					provider: self.id().to_string(),
					confidence: 0.0,
					confidence_factors: Vec::new(),
				})
			})
			.collect();

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
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
		let key = strip_prefix(external_id);
		if key.ends_with('W') {
			let work = self.fetch_work(key).await?;
			Ok(self.media_from_work(work).await)
		} else {
			let edition = self.fetch_edition(key).await?;
			Ok(self.media_from_edition(edition, None).await)
		}
	}

	/// Open Library has no credentials to verify; a successful search proves
	/// the API is reachable.
	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		self.rate_limiter.until_ready().await;
		let response = self
			.client
			.get(format!(
				"{}/search.json?q=the&limit=1&fields=key",
				self.api_url
			))
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

/// Open Library is the only keyless upstream with a real edition graph: an
/// ISBN resolves an edition, the edition names its work, and the work lists
/// every edition ever catalogued under it. Two hops, and the second one is
/// paginated, so it is capped — a pairing decision does not improve after
/// the first hundred ISBNs of a classic.
#[async_trait::async_trait]
impl EditionLookup for OpenLibraryClient {
	fn provider_id(&self) -> &'static str {
		"OPEN_LIBRARY"
	}

	async fn sibling_identifiers(
		&self,
		identifier: &EditionIdentifier,
	) -> Result<Vec<EditionIdentifier>, MetadataProviderError> {
		let (work_key, seed) = match identifier {
			EditionIdentifier::OpenLibraryWork(key) => (key.clone(), None),
			EditionIdentifier::Isbn(isbn) => {
				match self.fetch_edition_by_isbn(isbn).await {
					Ok(edition) => {
						let Some(work) = edition.works.first() else {
							return Ok(Vec::new());
						};
						(strip_prefix(&work.key).to_string(), Some(isbn.clone()))
					},
					// An ISBN Open Library has never seen is a miss.
					Err(MetadataProviderError::NotFound(_)) => return Ok(Vec::new()),
					Err(error) => return Err(error),
				}
			},
			// Open Library indexes Amazon ids only as free-form
			// `identifiers.amazon` on some editions, with no lookup route,
			// so there is nothing to walk from an ASIN.
			EditionIdentifier::Asin(_) => return Ok(Vec::new()),
		};

		let editions = match self.fetch_work_editions(&work_key).await {
			Ok(editions) => editions,
			Err(MetadataProviderError::NotFound(_)) => return Ok(Vec::new()),
			Err(error) => return Err(error),
		};

		let mut siblings = vec![EditionIdentifier::OpenLibraryWork(work_key)];
		for edition in editions {
			siblings.extend(
				edition
					.isbn_10
					.into_iter()
					.chain(edition.isbn_13)
					.filter_map(|isbn| EditionIdentifier::Isbn(isbn).normalized()),
			);
		}
		siblings.sort_unstable();
		siblings.dedup();
		if let Some(seed) =
			seed.and_then(|isbn| EditionIdentifier::Isbn(isbn).normalized())
		{
			siblings.retain(|candidate| *candidate != seed);
		}
		Ok(siblings)
	}
}

/// Map a search document straight onto media metadata. Prefers the cover
/// edition as the external id so a later lookup resolves an edition with
/// ISBNs and page counts; falls back to the work.
fn media_from_search_doc(
	provider_id: &str,
	doc: OpenLibrarySearchDoc,
	query_isbn: Option<&str>,
) -> Option<ExternalMediaMetadata> {
	let title = non_empty(doc.title)?;
	let work_key = strip_prefix(&doc.key).to_string();
	let (external_id, provider_url) = match doc.cover_edition_key {
		Some(edition) => {
			let url = format!("https://openlibrary.org/books/{edition}");
			(edition, url)
		},
		None => {
			let url = format!("https://openlibrary.org/works/{work_key}");
			(work_key, url)
		},
	};
	// The document's ISBN list spans every edition of the work; only echo an
	// ISBN back when it is the one the caller asked about.
	let matched_isbn = query_isbn.and_then(|wanted| {
		doc.isbn
			.iter()
			.find(|candidate| normalize_isbn(candidate) == wanted)
			.cloned()
	});
	let (isbn, isbn_13) = match matched_isbn {
		Some(value) if value.len() == 13 => (None, Some(value)),
		Some(value) => (Some(value), None),
		None => (None, None),
	};

	Some(ExternalMediaMetadata {
		provider: provider_id.to_string(),
		external_id,
		title: Some(title),
		page_count: doc.number_of_pages_median,
		year: doc.first_publish_year,
		tags: filled_or_none(
			doc.subject
				.into_iter()
				.take(OPEN_LIBRARY_MAX_SUBJECTS)
				.collect(),
		),
		isbn,
		isbn_13,
		writers: filled_or_none(doc.author_name),
		cover_url: doc.cover_i.filter(|id| *id > 0).map(cover_url),
		provider_url: Some(provider_url),
		..Default::default()
	})
}

/// Choose the ISBN-10/13 pair to expose. When the caller searched by ISBN the
/// matching value wins so the scorer can confirm it; otherwise the first of
/// each list.
fn pick_isbns(
	isbn_10: &[String],
	isbn_13: &[String],
	query_isbn: Option<&str>,
) -> (Option<String>, Option<String>) {
	let pick = |values: &[String]| {
		query_isbn
			.and_then(|wanted| {
				values
					.iter()
					.find(|candidate| normalize_isbn(candidate) == wanted)
			})
			.or_else(|| values.first())
			.cloned()
	};
	(pick(isbn_10), pick(isbn_13))
}

/// Keys arrive as `/books/OL1M`, `/works/OL1W`, `/authors/OL1A`; strip to the
/// bare identifier.
fn strip_prefix(key: &str) -> &str {
	key.rsplit('/').next().unwrap_or(key)
}

fn cover_url(cover_id: i64) -> String {
	format!("{}/{}-L.jpg", OpenLibraryClient::COVER_BASE_URL, cover_id)
}

/// Cover lists may contain `-1` placeholders for missing images.
fn first_cover(covers: &[i64]) -> Option<i64> {
	covers.iter().copied().find(|id| *id > 0)
}

/// Publish dates are free text (`October 1, 1988`, `Sep 2001`, `1988`).
/// Parse what dateparser accepts, otherwise settle for a four-digit year.
fn parse_publish_date(date: Option<&str>) -> (Option<i32>, Option<i32>, Option<i32>) {
	use chrono::Datelike;
	let Some(date) = date.map(str::trim).filter(|d| !d.is_empty()) else {
		return (None, None, None);
	};
	if let Ok(parsed) = dateparser::parse_with_timezone(date, &chrono::Utc) {
		return (
			Some(parsed.year()),
			Some(parsed.month() as i32),
			Some(parsed.day() as i32),
		);
	}
	let year = date
		.split(|c: char| !c.is_ascii_digit())
		.find(|part| part.len() == 4)
		.and_then(|part| part.parse().ok());
	(year, None, None)
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
struct OpenLibrarySearchResponse {
	#[serde(default)]
	docs: Vec<OpenLibrarySearchDoc>,
}

#[derive(Debug, Deserialize)]
struct OpenLibrarySearchDoc {
	key: String,
	title: Option<String>,
	#[serde(default)]
	author_name: Vec<String>,
	first_publish_year: Option<i32>,
	#[serde(default)]
	isbn: Vec<String>,
	cover_i: Option<i64>,
	cover_edition_key: Option<String>,
	number_of_pages_median: Option<i32>,
	#[serde(default)]
	subject: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenLibraryKeyRef {
	key: String,
}

#[derive(Debug, Deserialize)]
struct OpenLibraryEdition {
	key: String,
	title: Option<String>,
	#[serde(default)]
	authors: Vec<OpenLibraryKeyRef>,
	#[serde(default)]
	works: Vec<OpenLibraryKeyRef>,
	publish_date: Option<String>,
	number_of_pages: Option<i32>,
	#[serde(default)]
	isbn_10: Vec<String>,
	#[serde(default)]
	isbn_13: Vec<String>,
	#[serde(default)]
	covers: Vec<i64>,
	description: Option<OpenLibraryText>,
	#[serde(default)]
	series: Vec<String>,
	#[serde(default)]
	subjects: Vec<String>,
}

/// `GET /works/{key}/editions.json`. Only `entries` matters; the sibling
/// `links` and `size` describe the pagination this deliberately does not
/// follow.
#[derive(Debug, Deserialize)]
struct OpenLibraryWorkEditions {
	#[serde(default)]
	entries: Vec<OpenLibraryEdition>,
}

#[derive(Debug, Deserialize)]
struct OpenLibraryWorkAuthor {
	author: Option<OpenLibraryKeyRef>,
}

#[derive(Debug, Deserialize)]
struct OpenLibraryWork {
	key: String,
	title: Option<String>,
	description: Option<OpenLibraryText>,
	#[serde(default)]
	authors: Vec<OpenLibraryWorkAuthor>,
	#[serde(default)]
	covers: Vec<i64>,
	#[serde(default)]
	subjects: Vec<String>,
	first_publish_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenLibraryAuthor {
	name: Option<String>,
}

/// Descriptions are either a bare string or `{ "type": "/type/text",
/// "value": "…" }`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenLibraryText {
	Plain(String),
	Typed { value: String },
}

impl OpenLibraryText {
	fn text(&self) -> Option<String> {
		let value = match self {
			Self::Plain(value) | Self::Typed { value } => value,
		};
		non_empty(Some(value.clone()))
	}
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

	fn search_response() -> String {
		serde_json::json!({
			"numFound": 1,
			"start": 0,
			"docs": [{
				"key": "/works/OL45804W",
				"title": "Fantastic Mr Fox",
				"author_name": ["Roald Dahl"],
				"author_key": ["OL34184A"],
				"first_publish_year": 1970,
				"isbn": ["0140328726", "9780140328721"],
				"cover_i": 6498519,
				"cover_edition_key": "OL7353617M",
				"number_of_pages_median": 96,
				"subject": ["Foxes", "Juvenile fiction", "Farmers"]
			}]
		})
		.to_string()
	}

	fn edition_response() -> String {
		serde_json::json!({
			"key": "/books/OL7353617M",
			"title": "Fantastic Mr. Fox",
			"authors": [{ "key": "/authors/OL34184A" }],
			"works": [{ "key": "/works/OL45804W" }],
			"publishers": ["Puffin"],
			"publish_date": "October 1, 1988",
			"number_of_pages": 96,
			"isbn_10": ["0140328726"],
			"isbn_13": ["9780140328721"],
			"covers": [-1, 15152634, 8739161],
			"series": ["Puffin Books"]
		})
		.to_string()
	}

	fn work_response() -> String {
		serde_json::json!({
			"key": "/works/OL45804W",
			"title": "Fantastic Mr Fox",
			"description": { "type": "/type/text", "value": "A clever fox outwits three farmers." },
			"authors": [{ "author": { "key": "/authors/OL34184A" }, "type": { "key": "/type/author_role" } }],
			"covers": [6498519],
			"subjects": ["Foxes", "Juvenile fiction"]
		})
		.to_string()
	}

	fn author_response() -> String {
		serde_json::json!({ "key": "/authors/OL34184A", "name": "Roald Dahl" })
			.to_string()
	}

	#[tokio::test]
	async fn search_media_maps_search_documents_without_detail_fetches() {
		let server = MockServer::spawn(vec![render_ok(&search_response())]);
		let client = OpenLibraryClient::new().with_api_url(server.url.clone());

		let outcome = client
			.search_media(&query("Fantastic Mr Fox"))
			.await
			.expect("search should succeed against the mock");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "openlibrary");
		assert_eq!(candidate.external_id, "OL7353617M");
		assert!(candidate.confidence >= 0.9, "{}", candidate.confidence);

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(media.title.as_deref(), Some("Fantastic Mr Fox"));
		assert_eq!(media.year, Some(1970));
		assert_eq!(media.page_count, Some(96));
		assert_eq!(
			media.writers.as_deref(),
			Some(["Roald Dahl".to_string()].as_slice())
		);
		assert_eq!(
			media.tags.as_deref(),
			Some(
				[
					"Foxes".to_string(),
					"Juvenile fiction".to_string(),
					"Farmers".to_string()
				]
				.as_slice()
			)
		);
		// no ISBN in the query, so none of the work-wide ISBNs is echoed back
		assert_eq!(media.isbn, None);
		assert_eq!(media.isbn_13, None);
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://covers.openlibrary.org/b/id/6498519-L.jpg")
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://openlibrary.org/books/OL7353617M")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		let request = &requests[0];
		assert!(request.starts_with("GET /search.json?"), "{request}");
		assert!(request.contains("q=Fantastic+Mr+Fox"), "{request}");
		assert!(request.contains("limit=5"), "{request}");
		assert!(request.contains("fields=key%2Ctitle"), "{request}");
		assert!(!request.contains("author="), "{request}");
		assert!(
			request.to_ascii_lowercase().contains("user-agent: stump/"),
			"{request}"
		);
	}

	#[tokio::test]
	async fn search_media_with_author_adds_author_filter() {
		let server = MockServer::spawn(vec![render_ok(&search_response())]);
		let client = OpenLibraryClient::new().with_api_url(server.url.clone());

		client
			.search_media(&SearchQuery {
				author: Some("Roald Dahl".to_string()),
				..query("Fantastic Mr Fox")
			})
			.await
			.expect("search should succeed");

		let request = &server.requests()[0];
		assert!(request.contains("author=Roald+Dahl"), "{request}");
	}

	#[tokio::test]
	async fn isbn_query_resolves_edition_work_and_author_first() {
		let server = MockServer::spawn(vec![
			render_ok(&edition_response()),
			render_ok(&work_response()),
			render_ok(&author_response()),
		]);
		let client = OpenLibraryClient::new().with_api_url(server.url.clone());

		let outcome = client
			.search_media(&SearchQuery {
				isbn: Some("978-0-14-032872-1".to_string()),
				..query("Something Else Entirely")
			})
			.await
			.expect("ISBN lookup should succeed");

		assert_eq!(outcome.candidates.len(), 1);
		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.external_id, "OL7353617M");
		assert!(
			candidate.confidence >= 0.98,
			"ISBN hit should score as definitive, got {}",
			candidate.confidence
		);
		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(media.title.as_deref(), Some("Fantastic Mr. Fox"));
		assert_eq!(
			media.summary.as_deref(),
			Some("A clever fox outwits three farmers.")
		);
		assert_eq!(media.isbn.as_deref(), Some("0140328726"));
		assert_eq!(media.isbn_13.as_deref(), Some("9780140328721"));
		assert_eq!(
			(media.year, media.month, media.day),
			(Some(1988), Some(10), Some(1))
		);
		assert_eq!(media.series_name.as_deref(), Some("Puffin Books"));
		assert_eq!(
			media.writers.as_deref(),
			Some(["Roald Dahl".to_string()].as_slice())
		);
		// the -1 placeholder is skipped
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://covers.openlibrary.org/b/id/15152634-L.jpg")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 3);
		assert!(
			requests[0].starts_with("GET /isbn/9780140328721.json"),
			"{}",
			requests[0]
		);
		assert!(
			requests[1].starts_with("GET /works/OL45804W.json"),
			"{}",
			requests[1]
		);
		assert!(
			requests[2].starts_with("GET /authors/OL34184A.json"),
			"{}",
			requests[2]
		);
	}

	#[tokio::test]
	async fn isbn_miss_falls_back_to_title_search() {
		let not_found = "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
		let server =
			MockServer::spawn(vec![not_found.to_string(), render_ok(&search_response())]);
		let client = OpenLibraryClient::new().with_api_url(server.url.clone());

		let outcome = client
			.search_media(&SearchQuery {
				isbn: Some("9780000000000".to_string()),
				..query("Fantastic Mr Fox")
			})
			.await
			.expect("fallback search should succeed");

		assert_eq!(outcome.candidates.len(), 1);
		let requests = server.requests();
		assert_eq!(requests.len(), 2);
		assert!(requests[0].starts_with("GET /isbn/9780000000000.json"));
		assert!(requests[1].starts_with("GET /search.json?"));
	}

	#[tokio::test]
	async fn fetch_media_metadata_dispatches_on_edition_or_work_key() {
		let server = MockServer::spawn(vec![
			render_ok(&edition_response()),
			render_ok(&work_response()),
			render_ok(&author_response()),
			render_ok(&work_response()),
			render_ok(&author_response()),
		]);
		let client = OpenLibraryClient::new().with_api_url(server.url.clone());

		let edition = client
			.fetch_media_metadata("OL7353617M")
			.await
			.expect("edition lookup should succeed");
		assert_eq!(edition.external_id, "OL7353617M");
		assert_eq!(edition.page_count, Some(96));

		let work = client
			.fetch_media_metadata("OL45804W")
			.await
			.expect("work lookup should succeed");
		assert_eq!(work.external_id, "OL45804W");
		assert_eq!(work.title.as_deref(), Some("Fantastic Mr Fox"));
		assert_eq!(
			work.provider_url.as_deref(),
			Some("https://openlibrary.org/works/OL45804W")
		);

		let requests = server.requests();
		assert!(requests[0].starts_with("GET /books/OL7353617M.json"));
		assert!(requests[3].starts_with("GET /works/OL45804W.json"));
	}

	#[tokio::test]
	async fn rate_limited_response_maps_to_rate_limited_error() {
		let too_many = "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let server = MockServer::spawn(vec![too_many.to_string()]);
		let client = OpenLibraryClient::new()
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
Content-Length: 12
Connection: close

{"docs": [{"#
			.replace('\n', "\r\n");
		let server = MockServer::spawn(vec![body]);
		let client = OpenLibraryClient::new().with_api_url(server.url.clone());

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
	fn publish_dates_degrade_to_a_bare_year() {
		assert_eq!(
			parse_publish_date(Some("October 1, 1988")),
			(Some(1988), Some(10), Some(1))
		);
		assert_eq!(parse_publish_date(Some("1988")).0, Some(1988));
		assert_eq!(parse_publish_date(Some("n.d.")), (None, None, None));
		assert_eq!(parse_publish_date(None), (None, None, None));
	}

	#[ignore = "Requires network access"]
	#[tokio::test]
	async fn live_isbn_lookup() {
		let client = OpenLibraryClient::new();
		let outcome = client
			.search_media(&SearchQuery {
				isbn: Some("9780140328721".to_string()),
				..query("Fantastic Mr Fox")
			})
			.await
			.expect("live lookup should succeed");
		assert_eq!(outcome.candidates[0].external_id, "OL7353617M");
	}
}
