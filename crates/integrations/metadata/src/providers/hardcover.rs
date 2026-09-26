use chrono::Datelike;
use reqwest_middleware::ClientWithMiddleware;
use serde::Deserialize;

use crate::{
	client::{build_client_with_retry, RetryClientConfig},
	error::MetadataProviderError,
	provider::ProviderCredentialVerification,
	serde_utils::string_or_number,
	types::{
		AudiobookEdition, ExternalMediaMetadata, ExternalSeriesMetadata, MatchCandidate,
		MediaType, SearchOutcome, SearchQuery,
	},
	ExternalMetadata, MetadataProvider, RateLimiter,
};

const HARDCOVER_DEFAULT_RATE_LIMIT: u32 = 5;

/// Audio editions fetched per book. A work rarely has more than a handful of
/// audiobook releases (regional narrators, abridgements); the cap bounds the
/// response rather than paginating.
const AUDIO_EDITIONS_LIMIT: u32 = 25;

pub struct HardcoverClient {
	client: ClientWithMiddleware,
	api_token: Option<String>,
	api_url: String,
	rate_limiter: RateLimiter,
}

/// Object types supported by Hardcover's search API
/// See: https://docs.hardcover.app/api/guides/searching/
#[derive(Debug, Clone, Copy)]
pub enum HardcoverSearchType {
	Book,
	Series,
}

impl HardcoverSearchType {
	fn as_str(&self) -> &'static str {
		match self {
			Self::Book => "Book",
			Self::Series => "Series",
		}
	}
}

/// Hardcover's account page shows the token as `Bearer <token>`; accept it
/// pasted with or without that prefix, since `bearer_auth` adds its own.
fn normalize_token(raw: &str) -> String {
	let trimmed = raw.trim();
	match trimmed.get(..7) {
		Some(prefix) if prefix.eq_ignore_ascii_case("bearer ") => {
			trimmed[7..].trim().to_string()
		},
		_ => trimmed.to_string(),
	}
}

impl HardcoverClient {
	const API_URL: &'static str = "https://api.hardcover.app/v1/graphql";

	pub fn new(api_token: String, rate_limit: Option<u32>) -> Self {
		Self {
			client: build_client_with_retry(
				reqwest::Client::new(),
				RetryClientConfig::default(),
			),
			api_token: Some(normalize_token(&api_token)),
			api_url: Self::API_URL.to_string(),
			rate_limiter: RateLimiter::new(
				rate_limit.unwrap_or(HARDCOVER_DEFAULT_RATE_LIMIT),
			),
		}
	}

	/// Test-only override of the API base URL, used to point the client at a
	/// local mock server.
	#[cfg(test)]
	pub(crate) fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
		self.api_url = api_url.into();
		self
	}

	/// Point the client at a local mock server, for tests in crates that
	/// cannot reach the private override above. `base_url` is the server
	/// root; the GraphQL endpoint path is appended as on the real host.
	#[cfg(any(test, feature = "mock"))]
	#[must_use]
	pub fn pointed_at(mut self, base_url: &str) -> Self {
		self.api_url = format!("{base_url}/v1/graphql");
		self
	}

	pub fn token(&self) -> Result<String, MetadataProviderError> {
		self.api_token
			.clone()
			.ok_or(MetadataProviderError::MissingToken)
	}

	async fn execute_graphql<T: serde::de::DeserializeOwned>(
		&self,
		query: &str,
	) -> Result<T, MetadataProviderError> {
		let token = self.token()?;
		self.rate_limiter.until_ready().await;

		let body = serde_json::json!({ "query": query });

		let response = self
			.client
			.post(&self.api_url)
			.bearer_auth(token)
			.json(&body)
			.send()
			.await?
			.error_for_status()?
			.json::<GraphQLResponse<T>>()
			.await?;

		if let Some(errors) = response.errors {
			if !errors.is_empty() {
				let messages: Vec<_> =
					errors.iter().map(|e| e.message.as_str()).collect();
				return Err(MetadataProviderError::Other(format!(
					"GraphQL errors: {}",
					messages.join("; ")
				)));
			}
		}

		response.data.ok_or(MetadataProviderError::EmptyResponse)
	}

	#[tracing::instrument(skip(self))]
	async fn search(
		&self,
		query: &str,
		query_type: HardcoverSearchType,
		limit: u32,
	) -> Result<SearchResponse, MetadataProviderError> {
		let sanitized_query = query
			.replace('\\', "\\\\")
			.replace('"', "\\\"")
			.replace(['\n', '\r', '\t'], " ");
		let graphql_query = format!(
			r#"query Search {{ search(query: "{}", query_type: "{}", per_page: {}) {{ results }} }}"#,
			sanitized_query,
			query_type.as_str(),
			limit,
		);
		tracing::trace!(?graphql_query, "Searching Hardcover...");

		let data: SearchData = self.execute_graphql(&graphql_query).await?;
		Ok(data.search)
	}

	async fn fetch_series(&self, id: i64) -> Result<SeriesDetail, MetadataProviderError> {
		let graphql_query = format!(
			r#"query GetSeries {{
				series(where: {{ id: {{ _eq: {} }} }}) {{
					id
					slug
					name
					description
					books_count
					author {{
						id
						name
						slug
					}}
				}}
			}}"#,
			id
		);

		let data: SeriesQueryData = self.execute_graphql(&graphql_query).await?;
		data.series
			.into_iter()
			.next()
			.ok_or_else(|| MetadataProviderError::NotFound(format!("Series {}", id)))
	}

	async fn fetch_book(&self, id: i64) -> Result<BookDetail, MetadataProviderError> {
		let graphql_query = format!(
			r#"query GetBook {{
				books(where: {{ id: {{ _eq: {} }} }}) {{
					id
					slug
					title
					subtitle
					description
					release_year
					release_date
					pages
					cached_image
					cached_contributors
					cached_tags
					featured_book_series {{
						position
						series {{
							name
						}}
					}}
					featured_book_series_id
					default_physical_edition {{
						isbn_10
						isbn_13
					}}
				}}
			}}"#,
			id
		);

		let data: BookQueryData = self.execute_graphql(&graphql_query).await?;
		data.books
			.into_iter()
			.next()
			.ok_or_else(|| MetadataProviderError::NotFound(format!("Book {}", id)))
	}

	/// The audiobook editions (`reading_format_id` 2 = Audio, per
	/// https://docs.hardcover.app/api/graphql/schemas/editions/) of one book,
	/// most-shelved first. Narrators come from the edition's `contributions`
	/// and, when those are empty, its `cached_contributors` snapshot.
	async fn fetch_audio_editions(
		&self,
		id: i64,
	) -> Result<Vec<serde_json::Value>, MetadataProviderError> {
		let graphql_query = format!(
			r#"query GetAudiobookEditions {{
				editions(
					where: {{ book_id: {{ _eq: {} }}, reading_format_id: {{ _eq: 2 }} }}
					order_by: {{ users_count: desc }}
					limit: {}
				) {{
					id
					asin
					audio_seconds
					users_count
					language {{
						language
						code2
						code3
					}}
					contributions {{
						contribution
						author {{
							name
						}}
					}}
					cached_contributors
				}}
			}}"#,
			id, AUDIO_EDITIONS_LIMIT
		);

		let data: EditionsQueryData = self.execute_graphql(&graphql_query).await?;
		Ok(data.editions)
	}
}

#[async_trait::async_trait]
impl MetadataProvider for HardcoverClient {
	fn id(&self) -> &'static str {
		"hardcover"
	}

	fn name(&self) -> &'static str {
		"Hardcover"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Book]
	}

	/// Search for series on Hardcover and fetch full metadata for each result
	/// See: https://docs.hardcover.app/api/guides/searching/#series
	async fn search_series(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		tracing::trace!("Searching for series on Hardcover");
		let response = self
			.search(
				&query.title,
				HardcoverSearchType::Series,
				query.limit.unwrap_or(10),
			)
			.await?;

		let hits = response.parse_series_hits()?;
		let requested = hits.len();

		// TODO: Parallelize these fetches
		let mut candidates = Vec::with_capacity(hits.len());
		for hit in hits {
			let external_id = hit.document.id;
			match self.fetch_series_metadata(&external_id).await {
				Ok(metadata) => {
					tracing::trace!(external_id, "Fetched series metadata successfully");
					candidates.push(MatchCandidate {
						external_id,
						metadata: ExternalMetadata::Series(metadata),
						provider: self.id().to_string(),
						confidence: 0.0,
						confidence_factors: Vec::new(),
					})
				},
				Err(e) => {
					// TODO: Maybe if fetch fails, use naive meta from search?
					// A full skip failure feels wasteful? Idk, it's a complicated feature
					tracing::error!(
						external_id,
						error = ?e,
						"Failed to fetch series metadata for search result"
					);
				},
			}
		}

		if candidates.len() < requested {
			tracing::warn!(
				requested,
				fetched = candidates.len(),
				"Some series search results could not be fetched"
			);
		}

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	/// Search for books on Hardcover and fetch full metadata for each result
	/// See: https://docs.hardcover.app/api/guides/searching/#books
	#[tracing::instrument(skip(self))]
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let response = self
			.search(
				&query.title,
				HardcoverSearchType::Book,
				query.limit.unwrap_or(10),
			)
			.await?;

		let hits = response.parse_book_hits()?;
		let requested = hits.len();

		// TODO: Parallelize these fetches
		let mut candidates = Vec::with_capacity(hits.len());
		for hit in hits {
			let search_writers = hit.document.search_writers();
			let external_id = hit.document.id;
			match self.fetch_media_metadata(&external_id).await {
				Ok(mut metadata) => {
					let detail_has_writers = metadata
						.writers
						.as_ref()
						.is_some_and(|writers| !writers.is_empty());
					if !detail_has_writers && !search_writers.is_empty() {
						metadata.writers = Some(search_writers);
					}
					tracing::trace!(external_id, "Fetched book metadata successfully");
					candidates.push(MatchCandidate {
						external_id,
						metadata: ExternalMetadata::Media(metadata),
						provider: self.id().to_string(),
						confidence: 0.0,
						confidence_factors: Vec::new(),
					})
				},
				Err(e) => {
					// TODO: Maybe if fetch fails, use naive meta from search?
					// A full skip failure feels wasteful? Idk, it's a complicated feature
					tracing::error!(
						external_id,
						error = ?e,
						"Failed to fetch book metadata for search result"
					);
				},
			}
		}

		if candidates.len() < requested {
			tracing::warn!(
				requested,
				fetched = candidates.len(),
				"Some book search results could not be fetched"
			);
		}

		Ok(SearchOutcome {
			candidates: self.score_search(query, candidates),
			requested,
		})
	}

	/// Search for books and build candidates from the search index documents
	/// alone: exactly one request, no per-hit `books` query. Hits keep the
	/// index's own relevance order.
	#[tracing::instrument(skip(self))]
	async fn search_media_brief(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let response = self
			.search(
				&query.title,
				HardcoverSearchType::Book,
				query.limit.unwrap_or(10),
			)
			.await?;

		let hits = response.parse_book_hits()?;
		let requested = hits.len();
		let candidates = hits
			.into_iter()
			.map(|hit| {
				let external_id = hit.document.id.clone();
				MatchCandidate {
					external_id,
					metadata: ExternalMetadata::Media(
						hit.document.into_brief_metadata(self.id()),
					),
					provider: self.id().to_string(),
					confidence: 0.0,
					confidence_factors: Vec::new(),
				}
			})
			.collect();

		Ok(SearchOutcome {
			candidates,
			requested,
		})
	}

	async fn fetch_series_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		let id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!("Invalid series ID: {}", external_id))
		})?;

		let series = self.fetch_series(id).await?;

		let authors: Vec<String> =
			series.author.and_then(|a| a.name).into_iter().collect();

		Ok(ExternalSeriesMetadata {
			provider: self.id().to_string(),
			external_id: series.id.to_string(),
			title: series.name.unwrap_or_default(),
			alternative_titles: vec![],
			summary: series.description,
			authors: Some(authors),
			volume_count: series.books_count,
			..Default::default()
		})
	}

	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!("Invalid book ID: {}", external_id))
		})?;

		let book = self.fetch_book(id).await?;

		let writers: Vec<String> = book
			.cached_contributors
			.as_ref()
			.and_then(|v| v.as_array())
			.map(|arr| {
				arr.iter()
					.filter_map(|c| {
						let contribution = c.get("contribution")?.as_str()?;
						if contribution == "Author" || contribution == "Writer" {
							c.get("author")?
								.get("name")?
								.as_str()
								.map(|s| s.to_string())
						} else {
							None
						}
					})
					.collect()
			})
			.unwrap_or_default();

		let cover_url = book
			.cached_image
			.as_ref()
			.and_then(|img| img.get("url"))
			.and_then(|v| v.as_str())
			.map(|s| s.to_string());

		let number = book
			.featured_book_series
			.as_ref()
			.and_then(|fbs| fbs.position);

		let (isbn, isbn_13) = book
			.default_physical_edition
			.as_ref()
			.map(|ed| (ed.isbn_10.clone(), ed.isbn_13.clone()))
			.unwrap_or((None, None));

		let genres: Option<Vec<String>> = book
			.cached_tags
			.as_ref()
			.and_then(|v| v.get("Genre"))
			.and_then(|v| v.as_array())
			.map(|arr| {
				arr.iter()
					.filter_map(|t| t.get("tag")?.as_str().map(|s| s.to_string()))
					.collect()
			});

		let tags: Option<Vec<String>> = book.cached_tags.as_ref().map(|cached| {
			["Tag", "Mood"]
				.iter()
				.filter_map(|category| cached.get(*category)?.as_array())
				.flatten()
				.filter_map(|t| t.get("tag")?.as_str().map(|s| s.to_string()))
				.collect()
		});

		let (year, month, day) = match book
			.release_date
			.and_then(|d| dateparser::parse_with_timezone(&d, &chrono::Utc).ok())
		{
			Some(date) => (
				Some(date.year()),
				Some(date.month() as i32),
				Some(date.day() as i32),
			),
			None => (book.release_year, None, None),
		};

		let series_name = book
			.featured_book_series
			.as_ref()
			.and_then(|fbs| fbs.series.as_ref().and_then(|s| s.name.clone()));
		let series_external_id = book.featured_book_series_id.map(|id| id.to_string());

		Ok(ExternalMediaMetadata {
			provider: self.id().to_string(),
			external_id: book.id.to_string(),
			series_name,
			series_external_id,
			title: book.title,
			summary: book.description,
			number,
			year,
			month,
			day,
			page_count: book.pages,
			isbn,
			isbn_13,
			writers: Some(writers),
			cover_url,
			provider_url: book
				.slug
				.map(|s| format!("https://hardcover.app/books/{}", s)),
			genres,
			tags,
			..Default::default()
		})
	}

	/// One `editions` query; see [`Self::fetch_audio_editions`]. Editions
	/// naming no narrator still count: their length and ASIN are answers too.
	#[tracing::instrument(skip(self))]
	async fn audiobook_editions(
		&self,
		external_id: &str,
	) -> Result<Vec<AudiobookEdition>, MetadataProviderError> {
		let id: i64 = external_id.parse().map_err(|_| {
			MetadataProviderError::Other(format!("Invalid book ID: {}", external_id))
		})?;
		let editions = self.fetch_audio_editions(id).await?;
		Ok(editions.iter().map(audiobook_edition).collect())
	}

	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		match self.verify_identity().await {
			Ok(identity) => Ok(ProviderCredentialVerification {
				response_status: 200,
				is_valid: identity.remote_user_id.is_some(),
				error: None,
			}),
			Err(error) => Ok(ProviderCredentialVerification {
				response_status: 0,
				is_valid: false,
				error: Some(error.to_string()),
			}),
		}
	}
}

/// The redacted identity returned by Hardcover's `me` capability probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardcoverIdentity {
	pub remote_user_id: Option<String>,
	pub username: Option<String>,
}

impl HardcoverClient {
	/// Verifies this credential and returns only the remote identity. The PAT
	/// never leaves this crate's request boundary.
	pub async fn verify_identity(
		&self,
	) -> Result<HardcoverIdentity, MetadataProviderError> {
		let token = self.token()?;
		let body = serde_json::json!({ "query": "query { me { id username } }" });
		let response = self
			.client
			.post(&self.api_url)
			.bearer_auth(token)
			.json(&body)
			.send()
			.await?
			.error_for_status()?
			.json::<GraphQLResponse<MeResponse>>()
			.await?;
		if let Some(errors) = response.errors {
			if !errors.is_empty() {
				let messages: Vec<_> =
					errors.iter().map(|e| e.message.as_str()).collect();
				return Err(MetadataProviderError::Other(messages.join("; ")));
			}
		}
		let me = response
			.data
			.and_then(|data| data.me.into_iter().next())
			.ok_or(MetadataProviderError::EmptyResponse)?;
		let remote_user_id = (!me.id.is_null()).then(|| match me.id {
			serde_json::Value::String(value) => value,
			value => value.to_string(),
		});
		Ok(HardcoverIdentity {
			remote_user_id,
			username: me.username,
		})
	}

	/// Inspects the provider's public GraphQL schema without reading any
	/// personal reading data. The result is only a list of advertised query
	/// capabilities and is safe to persist as redacted account state.
	pub async fn inspect_capabilities(
		&self,
	) -> Result<Vec<String>, MetadataProviderError> {
		let value: serde_json::Value = self
			.execute_graphql(
				"query Capabilities { __schema { queryType { fields { name } } } }",
			)
			.await?;
		Ok(value
			.get("__schema")
			.and_then(|schema| schema.get("queryType"))
			.and_then(|query_type| query_type.get("fields"))
			.and_then(serde_json::Value::as_array)
			.map(|fields| {
				fields
					.iter()
					.filter_map(|field| field.get("name").and_then(|name| name.as_str()))
					.map(ToOwned::to_owned)
					.collect()
			})
			.unwrap_or_default())
	}

	/// Reads the current user's journal/quote records when the provider
	/// advertises the `user_books` query. Fields are intentionally optional:
	/// an absent locator remains an unresolved provenance record instead of a
	/// fabricated annotation.
	pub async fn fetch_journal_entries(
		&self,
	) -> Result<Vec<HardcoverJournalEntry>, MetadataProviderError> {
		let value: serde_json::Value = self
			.execute_graphql(
				"query Journal { me { user_books { id book_id title quote note notes page pages current_page progression } } }",
			)
			.await?;
		let values = value
			.get("me")
			.and_then(serde_json::Value::as_array)
			.and_then(|users| users.first())
			.and_then(|user| user.get("user_books"))
			.and_then(serde_json::Value::as_array)
			.cloned()
			.unwrap_or_default();
		Ok(values
			.into_iter()
			.filter_map(HardcoverJournalEntry::from_value)
			.collect())
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct HardcoverJournalEntry {
	pub remote_entry_id: String,
	pub remote_book_id: Option<String>,
	pub quote: Option<String>,
	pub note: Option<String>,
	pub page: Option<i32>,
	pub progression: Option<f64>,
	pub raw: serde_json::Value,
}

impl HardcoverJournalEntry {
	fn from_value(value: serde_json::Value) -> Option<Self> {
		let remote_entry_id = value
			.get("id")
			.and_then(value_as_string)
			.or_else(|| value.get("user_book_id").and_then(value_as_string))?;
		let remote_book_id =
			value.get("book_id").and_then(value_as_string).or_else(|| {
				value
					.get("book")
					.and_then(|book| book.get("id"))
					.and_then(value_as_string)
			});
		let quote = first_string(&value, &["quote", "highlight", "excerpt"]);
		let note = first_string(&value, &["note", "notes", "review"]);
		let page = first_i32(&value, &["page", "pages", "current_page"]);
		let progression = value.get("progression").and_then(serde_json::Value::as_f64);
		Some(Self {
			remote_entry_id,
			remote_book_id,
			quote,
			note,
			page,
			progression,
			raw: value,
		})
	}
}

fn value_as_string(value: &serde_json::Value) -> Option<String> {
	match value {
		serde_json::Value::String(value) if !value.trim().is_empty() => {
			Some(value.clone())
		},
		serde_json::Value::Number(value) => Some(value.to_string()),
		_ => None,
	}
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
	keys.iter()
		.find_map(|key| value.get(*key).and_then(value_as_string))
}

fn value_as_i32(value: &serde_json::Value) -> Option<i32> {
	value
		.as_i64()
		.or_else(|| value.as_f64().map(|number| number.trunc() as i64))
		.and_then(|number| i32::try_from(number).ok())
		.or_else(|| value.as_str()?.trim().parse().ok())
}

fn value_as_f32(value: &serde_json::Value) -> Option<f32> {
	value
		.as_f64()
		.map(|number| number as f32)
		.or_else(|| value.as_str()?.trim().parse().ok())
}

fn value_as_bool(value: &serde_json::Value) -> Option<bool> {
	match value {
		serde_json::Value::Bool(flag) => Some(*flag),
		serde_json::Value::Number(number) => number.as_i64().map(|n| n != 0),
		serde_json::Value::String(text) => match text.trim() {
			"true" | "1" => Some(true),
			"false" | "0" => Some(false),
			_ => None,
		},
		_ => None,
	}
}

fn first_i32(value: &serde_json::Value, keys: &[&str]) -> Option<i32> {
	keys.iter()
		.find_map(|key| value.get(*key).and_then(value_as_i32))
}

#[derive(Debug, Deserialize)]
pub struct MeResponse {
	pub me: Vec<Me>,
}

#[derive(Debug, Deserialize)]
pub struct Me {
	#[serde(default)]
	pub id: serde_json::Value,
	#[serde(default)]
	pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQLResponse<T> {
	pub data: Option<T>,
	pub errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQLError {
	pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchData {
	pub search: SearchResponse,
}

#[derive(Debug, Deserialize)]
pub struct SearchResponse {
	pub results: serde_json::Value,
}

impl SearchResponse {
	pub fn parse_book_hits(&self) -> Result<Vec<BookHit>, MetadataProviderError> {
		let container: HitsContainer<BookDocument> =
			serde_json::from_value(self.results.clone())?;
		Ok(container.hits.unwrap_or_default())
	}

	pub fn parse_series_hits(&self) -> Result<Vec<SeriesHit>, MetadataProviderError> {
		let container: HitsContainer<SeriesDocument> =
			serde_json::from_value(self.results.clone())?;
		Ok(container.hits.unwrap_or_default())
	}
}

#[derive(Debug, Deserialize)]
pub struct HitsContainer<T> {
	pub hits: Option<Vec<Hit<T>>>,
}

#[derive(Debug, Deserialize)]
pub struct Hit<T> {
	pub document: T,
}

pub type BookHit = Hit<BookDocument>;
pub type SeriesHit = Hit<SeriesDocument>;

/// Document returned from book search
/// Fields from: https://docs.hardcover.app/api/guides/searching/#books
#[derive(Debug, Deserialize)]
pub struct BookDocument {
	#[serde(deserialize_with = "string_or_number")]
	pub id: String,
	#[serde(default)]
	pub author_names: Option<serde_json::Value>,
	#[serde(default)]
	pub cached_contributors: Option<serde_json::Value>,
	#[serde(default)]
	pub title: Option<String>,
	#[serde(default)]
	pub slug: Option<String>,
	#[serde(default)]
	pub description: Option<String>,
	#[serde(default)]
	pub release_year: Option<serde_json::Value>,
	#[serde(default)]
	pub pages: Option<serde_json::Value>,
	#[serde(default)]
	pub isbns: Option<serde_json::Value>,
	#[serde(default)]
	pub series_names: Option<serde_json::Value>,
	#[serde(default)]
	pub featured_series: Option<serde_json::Value>,
	#[serde(default)]
	pub featured_series_position: Option<serde_json::Value>,
	#[serde(default)]
	pub image: Option<serde_json::Value>,
	#[serde(default)]
	pub has_audiobook: Option<serde_json::Value>,
	#[serde(default)]
	pub has_ebook: Option<serde_json::Value>,
	/// Documented as the default audiobook edition's length, but absent from
	/// live index documents (observed 2026-09-25); parsed leniently in case
	/// Hardcover starts sending it. Lengths come from audio editions.
	#[serde(default)]
	pub audio_seconds: Option<serde_json::Value>,
}

impl BookDocument {
	fn search_writers(&self) -> Vec<String> {
		let author_names: Vec<String> = self
			.author_names
			.as_ref()
			.map(|value| match value {
				serde_json::Value::Array(values) => {
					values.iter().filter_map(search_author_name).collect()
				},
				value => search_author_name(value).into_iter().collect(),
			})
			.unwrap_or_default();
		if !author_names.is_empty() {
			return author_names;
		}

		self.cached_contributors
			.as_ref()
			.and_then(serde_json::Value::as_array)
			.map(|contributors| {
				contributors
					.iter()
					.filter_map(|contributor| {
						let role = contributor.get("contribution")?.as_str()?;
						if !matches!(role, "Author" | "Writer") {
							return None;
						}
						contributor.get("author").and_then(search_author_name)
					})
					.collect()
			})
			.unwrap_or_default()
	}

	/// Build metadata from the indexed document only. ISBNs are classified
	/// by length after stripping separators; the series comes from
	/// `featured_series` and falls back to the first `series_names` entry.
	fn into_brief_metadata(self, provider: &str) -> ExternalMediaMetadata {
		let writers = self.search_writers();
		let (isbn, isbn_13) = self
			.isbns
			.as_ref()
			.and_then(serde_json::Value::as_array)
			.map(|values| {
				values.iter().filter_map(value_as_string).fold(
					(None, None),
					|(isbn10, isbn13), raw| {
						let digits: String =
							raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
						match digits.len() {
							10 => (isbn10.or(Some(digits)), isbn13),
							13 => (isbn10, isbn13.or(Some(digits))),
							_ => (isbn10, isbn13),
						}
					},
				)
			})
			.unwrap_or((None, None));
		let featured = self.featured_series.as_ref();
		let series_name = featured
			.and_then(|series| {
				first_string(series, &["name", "series_name"]).or_else(|| {
					series
						.get("series")
						.and_then(|inner| first_string(inner, &["name"]))
				})
			})
			.or_else(|| {
				self.series_names
					.as_ref()
					.and_then(serde_json::Value::as_array)
					.and_then(|names| names.iter().find_map(value_as_string))
			});
		let series_external_id = featured.and_then(|series| {
			first_string(series, &["id", "series_id"]).or_else(|| {
				series
					.get("series")
					.and_then(|inner| first_string(inner, &["id"]))
			})
		});
		let number = self
			.featured_series_position
			.as_ref()
			.and_then(value_as_f32)
			.or_else(|| {
				featured
					.and_then(|series| series.get("position"))
					.and_then(value_as_f32)
			});
		let cover_url = self.image.as_ref().and_then(|image| {
			first_string(image, &["url"]).or_else(|| value_as_string(image))
		});

		let has_audiobook = self.has_audiobook.as_ref().and_then(value_as_bool);
		let audio_seconds = self
			.audio_seconds
			.as_ref()
			.and_then(value_as_i32)
			.filter(|seconds| *seconds > 0);

		ExternalMediaMetadata {
			provider: provider.to_string(),
			external_id: self.id,
			series_name,
			series_external_id,
			title: self.title.filter(|title| !title.trim().is_empty()),
			summary: self.description,
			number,
			year: self.release_year.as_ref().and_then(value_as_i32),
			page_count: self.pages.as_ref().and_then(value_as_i32),
			isbn,
			isbn_13,
			writers: Some(writers),
			cover_url,
			provider_url: self
				.slug
				.map(|slug| format!("https://hardcover.app/books/{slug}")),
			has_ebook: self.has_ebook.as_ref().and_then(value_as_bool),
			has_audiobook,
			audio_seconds,
			..Default::default()
		}
	}
}

fn search_author_name(value: &serde_json::Value) -> Option<String> {
	value
		.as_str()
		.or_else(|| value.get("name").and_then(serde_json::Value::as_str))
		.map(str::trim)
		.filter(|name| !name.is_empty())
		.map(str::to_owned)
}

/// Document returned from series search
/// Fields from: https://docs.hardcover.app/api/guides/searching/#series
#[derive(Debug, Deserialize)]
pub struct SeriesDocument {
	#[serde(deserialize_with = "string_or_number")]
	pub id: String,
}

// TODO(author-entity): collect id of author eventually
#[derive(Debug, Deserialize)]
pub struct AuthorRef {
	pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SeriesQueryData {
	pub series: Vec<SeriesDetail>,
}

/// See https://docs.hardcover.app/api/graphql/schemas/series/
#[derive(Debug, Deserialize)]
pub struct SeriesDetail {
	pub id: i64,
	pub name: Option<String>,
	pub description: Option<String>,
	pub books_count: Option<i32>,
	pub author: Option<AuthorRef>,
}

#[derive(Debug, Deserialize)]
pub struct BookQueryData {
	pub books: Vec<BookDetail>,
}

/// See https://docs.hardcover.app/api/graphql/schemas/books
#[derive(Debug, Deserialize)]
pub struct BookDetail {
	pub id: i64,
	pub slug: Option<String>,
	pub title: Option<String>,
	pub description: Option<String>,
	pub release_year: Option<i32>,
	pub release_date: Option<String>,
	pub pages: Option<i32>,
	pub cached_image: Option<serde_json::Value>,
	pub cached_contributors: Option<serde_json::Value>,
	pub cached_tags: Option<serde_json::Value>,
	pub featured_book_series: Option<FeaturedBookSeries>,
	pub featured_book_series_id: Option<i64>,
	pub default_physical_edition: Option<EditionRef>,
}

#[derive(Debug, Deserialize)]
pub struct FeaturedBookSeries {
	pub position: Option<f32>,
	pub series: Option<SeriesNameRef>,
}

#[derive(Debug, Deserialize)]
pub struct SeriesNameRef {
	pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EditionRef {
	pub isbn_10: Option<String>,
	pub isbn_13: Option<String>,
}

/// Editions come back as raw documents: every field the query names is
/// optional in practice, and the contributor shape has already drifted once
/// (`contributions` rows versus the `cached_contributors` snapshot).
#[derive(Debug, Deserialize)]
pub struct EditionsQueryData {
	#[serde(default)]
	pub editions: Vec<serde_json::Value>,
}

/// Contributors credited with a role that reads as narration. Roles are
/// free text on Hardcover, so the match is on the word rather than the
/// exact "Narrator" the docs list.
fn narrator_names(contributors: &serde_json::Value) -> Vec<String> {
	contributors
		.as_array()
		.map(|rows| {
			rows.iter()
				.filter_map(|row| {
					let role = row.get("contribution")?.as_str()?;
					if !role.to_ascii_lowercase().contains("narrat") {
						return None;
					}
					row.get("author").and_then(search_author_name)
				})
				.collect()
		})
		.unwrap_or_default()
}

/// See [`HardcoverClient::fetch_audio_editions`] for the shape queried.
fn audiobook_edition(edition: &serde_json::Value) -> AudiobookEdition {
	let narrators = edition
		.get("contributions")
		.map(narrator_names)
		.filter(|names| !names.is_empty())
		.or_else(|| edition.get("cached_contributors").map(narrator_names))
		.unwrap_or_default();
	AudiobookEdition {
		external_id: edition.get("id").and_then(value_as_string),
		narrators,
		audio_seconds: edition
			.get("audio_seconds")
			.and_then(value_as_i32)
			.filter(|seconds| *seconds > 0),
		abridged: None,
		asin: edition.get("asin").and_then(value_as_string),
		language: edition.get("language").and_then(|language| {
			first_string(language, &["code2", "code3", "language"])
				.map(|value| value.trim().to_lowercase())
		}),
		users_count: edition.get("users_count").and_then(value_as_i32),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn get_test_client() -> HardcoverClient {
		dotenvy::dotenv().ok();
		let api_token =
			std::env::var("HARDCOVER_API_TOKEN").expect("HARDCOVER_API_TOKEN not set");
		HardcoverClient::new(api_token, None)
	}

	#[ignore = "Requires HARDCOVER_API_TOKEN env var"]
	#[tokio::test]
	async fn test_search_series() {
		let client = get_test_client();
		let query = SearchQuery {
			title: "Wayfarers".to_string(),
			limit: Some(5),
			..Default::default()
		};

		let results = client.search_series(&query).await;
		println!("search_series results: {:#?}", results);
		assert!(results.is_ok());

		let candidates = results.unwrap().candidates;
		assert!(!candidates.is_empty());
		println!("Found {} series candidates", candidates.len());
		for candidate in &candidates {
			println!("{:?}", candidate);
		}
	}

	#[ignore = "Requires HARDCOVER_API_TOKEN env var"]
	#[tokio::test]
	async fn test_search_media() {
		let client = get_test_client();
		let query = SearchQuery {
			title: "The Long Way to a Small, Angry Planet".to_string(),
			limit: Some(5),
			..Default::default()
		};

		let results = client.search_media(&query).await;
		println!("search_media results: {:#?}", results);
		assert!(results.is_ok());

		let candidates = results.unwrap().candidates;
		assert!(!candidates.is_empty());
		println!("Found {} book candidates", candidates.len());
		for candidate in &candidates {
			println!("{:?}", candidate);
		}
	}

	#[ignore = "Requires HARDCOVER_API_TOKEN env var"]
	#[tokio::test]
	async fn test_fetch_series_metadata() {
		let client = get_test_client();

		let query = SearchQuery {
			title: "Wayfarers".to_string(),
			limit: Some(1),
			..Default::default()
		};

		let search_results = client.search_series(&query).await.unwrap().candidates;
		assert!(!search_results.is_empty());

		let series_id = &search_results[0].external_id;
		println!("Fetching series metadata for ID: {}", series_id);

		let metadata = client.fetch_series_metadata(series_id).await;
		println!("fetch_series_metadata result: {:#?}", metadata);
		assert!(metadata.is_ok());

		let meta = metadata.unwrap();
		println!("Series: {} (volumes: {:?})", meta.title, meta.volume_count);
	}

	#[ignore = "Requires HARDCOVER_API_TOKEN env var"]
	#[tokio::test]
	async fn test_fetch_media_metadata() {
		let client = get_test_client();

		let query = SearchQuery {
			title: "The Long Way to a Small, Angry Planet".to_string(),
			limit: Some(1),
			..Default::default()
		};

		let search_results = client.search_media(&query).await.unwrap().candidates;
		assert!(!search_results.is_empty());

		let book_id = &search_results[0].external_id;
		println!("Fetching book metadata for ID: {}", book_id);

		let metadata = client.fetch_media_metadata(book_id).await;
		println!("fetch_media_metadata result: {:#?}", metadata);
		assert!(metadata.is_ok());

		let meta = metadata.unwrap();
		println!(
			"Book: {:?} by {:?} ({:?} pages)",
			meta.title, meta.writers, meta.page_count
		);
	}

	#[tokio::test]
	async fn search_media_maps_mocked_book_response() {
		use crate::mock_http::{render_ok, MockServer};

		let search_body = serde_json::json!({
			"data": {
				"search": {
					"results": {
						"hits": [{
							"document": {
								"id": 52709,
								"author_names": ["Becky Chambers"],
								"cached_contributors": [{
									"contribution": "Author",
									"author": { "name": "Becky Chambers" }
								}]
							}
						}]
					}
				}
			}
		})
		.to_string();
		let book_body = serde_json::json!({
			"data": {
				"books": [
					{
						"id": 52709,
						"slug": "the-long-way-to-a-small-angry-planet",
						"title": "The Long Way to a Small, Angry Planet",
						"description": "<p>A cozy science fiction novel.</p>",
						"release_year": 2014,
						"release_date": null,
						"pages": 518,
						"cached_image": { "url": "https://hardcover.app/cover.jpg" },
						"cached_contributors": [],
						"cached_tags": {
							"Genre": [{ "tag": "Science Fiction" }]
						},
						"featured_book_series": {
							"position": 1.0,
							"series": { "name": "Wayfarers" }
						},
						"featured_book_series_id": 123,
						"default_physical_edition": {
							"isbn_10": "1477818541",
							"isbn_13": "9781477818542"
						}
					}
				]
			}
		})
		.to_string();

		let server =
			MockServer::spawn(vec![render_ok(&search_body), render_ok(&book_body)]);
		let client = HardcoverClient::new("test-token".to_string(), Some(u32::MAX))
			.with_api_url(format!("{}/v1/graphql", server.url));

		let query = SearchQuery {
			title: "The Long Way to a Small, Angry Planet".to_string(),
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
		assert_eq!(candidate.provider, "hardcover");
		assert_eq!(candidate.external_id, "52709");

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(
			media.title.as_deref(),
			Some("The Long Way to a Small, Angry Planet")
		);
		assert_eq!(media.year, Some(2014));
		assert_eq!(media.page_count, Some(518));
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://hardcover.app/cover.jpg")
		);
		assert_eq!(
			media.writers.as_deref(),
			Some(["Becky Chambers".to_string()].as_slice())
		);
		assert_eq!(media.number, Some(1.0));
		assert_eq!(media.series_name.as_deref(), Some("Wayfarers"));
		assert_eq!(media.isbn.as_deref(), Some("1477818541"));
		assert_eq!(
			media.genres.as_deref(),
			Some(["Science Fiction".to_string()].as_slice())
		);

		// The search POSTs the sanitized GraphQL query with the bearer token,
		// then the per-hit book detail fetch queries by id.
		let requests = server.requests();
		assert_eq!(requests.len(), 2);
		assert!(requests[0].starts_with("POST /v1/graphql"));
		assert!(requests[0]
			.to_lowercase()
			.contains("authorization: bearer test-token"));
		assert!(requests[0].contains("search(query:"));
		assert!(requests[0].contains(r#"query_type: \"Book\""#));
		assert!(requests[0].contains("per_page: 5"));
		assert!(requests[0].contains("The Long Way"));
		assert!(requests[1].contains("books(where:"));
		assert!(requests[1].contains("_eq: 52709"));
	}

	#[tokio::test]
	async fn search_media_brief_maps_index_document_with_one_request() {
		use crate::mock_http::{render_ok, MockServer};

		let search_body = serde_json::json!({
			"data": { "search": { "results": { "hits": [
				{
					"document": {
						"id": 52709,
						"title": "The Long Way to a Small, Angry Planet",
						"slug": "the-long-way-to-a-small-angry-planet",
						"author_names": ["Becky Chambers"],
						"release_year": 2014,
						"pages": "518",
						"isbns": ["978-1477818542", "1477818541", "bogus"],
						"series_names": ["Wayfarers"],
						"featured_series": { "id": 123, "name": "Wayfarers" },
						"featured_series_position": 1,
						"image": { "url": "https://hardcover.app/cover.jpg" },
						"has_audiobook": true,
						"has_ebook": false,
						"audio_seconds": 52380
					}
				},
				{
					"document": {
						"id": "99",
						"title": "Record of a Spaceborn Few",
						"cached_contributors": [{
							"contribution": "Author",
							"author": { "name": "Becky Chambers" }
						}],
						"release_year": "2018",
						"series_names": ["Wayfarers"],
						"featured_series": { "series": { "name": "Wayfarers" }, "position": "3" },
						"image": "https://hardcover.app/spaceborn.jpg",
						"has_audiobook": "true",
						"audio_seconds": 0
					}
				}
			] } } }
		})
		.to_string();

		let server = MockServer::spawn(vec![render_ok(&search_body)]);
		let client = HardcoverClient::new("test-token".to_string(), Some(u32::MAX))
			.with_api_url(format!("{}/v1/graphql", server.url));

		let outcome = client
			.search_media_brief(&SearchQuery {
				title: "Wayfarers".to_string(),
				limit: Some(5),
				..Default::default()
			})
			.await
			.expect("brief search should succeed against the mock");

		let requests = server.requests();
		assert_eq!(
			requests.len(),
			1,
			"brief search must not fetch per-hit detail"
		);
		assert!(requests[0].contains("search(query:"));
		assert!(requests[0].contains(r#"query_type: \"Book\""#));
		assert!(requests[0].contains("per_page: 5"));

		assert_eq!(outcome.requested, 2);
		assert_eq!(outcome.candidates.len(), 2);

		let first = outcome.candidates[0].metadata.as_media().unwrap();
		assert_eq!(outcome.candidates[0].external_id, "52709");
		assert_eq!(
			first.title.as_deref(),
			Some("The Long Way to a Small, Angry Planet")
		);
		assert_eq!(
			first.writers.as_deref(),
			Some(["Becky Chambers".to_string()].as_slice())
		);
		assert_eq!(first.year, Some(2014));
		assert_eq!(first.page_count, Some(518));
		assert_eq!(first.isbn_13.as_deref(), Some("9781477818542"));
		assert_eq!(first.isbn.as_deref(), Some("1477818541"));
		assert_eq!(
			first.cover_url.as_deref(),
			Some("https://hardcover.app/cover.jpg")
		);
		assert_eq!(first.series_name.as_deref(), Some("Wayfarers"));
		assert_eq!(first.series_external_id.as_deref(), Some("123"));
		assert_eq!(first.number, Some(1.0));
		assert_eq!(
			first.provider_url.as_deref(),
			Some("https://hardcover.app/books/the-long-way-to-a-small-angry-planet")
		);
		assert_eq!(first.has_audiobook, Some(true));
		assert_eq!(first.has_ebook, Some(false));
		assert_eq!(first.audio_seconds, Some(52380));

		let second = outcome.candidates[1].metadata.as_media().unwrap();
		assert_eq!(outcome.candidates[1].external_id, "99");
		assert_eq!(
			second.writers.as_deref(),
			Some(["Becky Chambers".to_string()].as_slice())
		);
		assert_eq!(second.year, Some(2018));
		assert_eq!(second.series_name.as_deref(), Some("Wayfarers"));
		assert_eq!(second.number, Some(3.0));
		assert_eq!(
			second.cover_url.as_deref(),
			Some("https://hardcover.app/spaceborn.jpg")
		);
		assert!(second.isbn.is_none() && second.isbn_13.is_none());
		assert_eq!(
			second.has_audiobook,
			Some(true),
			"string flags are accepted"
		);
		assert_eq!(second.has_ebook, None, "an absent flag stays unknown");
		assert_eq!(second.audio_seconds, None, "a zero length is no length");
	}

	#[tokio::test]
	async fn audiobook_editions_use_one_query_and_read_narrators_from_either_shape() {
		use crate::mock_http::{render_ok, MockServer};

		let body = serde_json::json!({
			"data": { "editions": [
				{
					"id": 1,
					"asin": "B08G9PRS1K",
					"audio_seconds": 57000.0,
					"users_count": 812,
					"language": { "language": "English", "code2": "en", "code3": "eng" },
					"contributions": [
						{ "contribution": "Author", "author": { "name": "Andy Weir" } },
						{ "contribution": "narrator", "author": { "name": "Ray Porter" } }
					],
					"cached_contributors": []
				},
				{
					"id": "2",
					"asin": null,
					"audio_seconds": "0",
					"language": { "language": "Finnish", "code2": null, "code3": "fin" },
					"contributions": [],
					"cached_contributors": [
						{ "contribution": "Narrator", "author": { "name": "Someone Else" } }
					]
				},
				{ "id": 3 }
			] }
		})
		.to_string();
		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = HardcoverClient::new("test-token".to_string(), Some(u32::MAX))
			.pointed_at(&server.url);

		let editions = client.audiobook_editions("52709").await.unwrap();

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(requests[0].contains("book_id: { _eq: 52709 }"));
		assert!(requests[0].contains("reading_format_id: { _eq: 2 }"));
		assert_eq!(
			editions,
			vec![
				AudiobookEdition {
					external_id: Some("1".to_string()),
					narrators: vec!["Ray Porter".to_string()],
					audio_seconds: Some(57000),
					abridged: None,
					asin: Some("B08G9PRS1K".to_string()),
					language: Some("en".to_string()),
					users_count: Some(812),
				},
				AudiobookEdition {
					external_id: Some("2".to_string()),
					narrators: vec!["Someone Else".to_string()],
					audio_seconds: None,
					abridged: None,
					asin: None,
					language: Some("fin".to_string()),
					users_count: None,
				},
				AudiobookEdition {
					external_id: Some("3".to_string()),
					..Default::default()
				},
			]
		);

		assert!(client.audiobook_editions("not-a-number").await.is_err());
	}

	#[test]
	fn book_search_document_extracts_cached_contributors() {
		let document: BookDocument = serde_json::from_value(serde_json::json!({
			"id": 52709,
			"cached_contributors": [
				{
					"contribution": "Author",
					"author": { "name": "Becky Chambers" }
				},
				{
					"contribution": "Translator",
					"author": { "name": "Other Contributor" }
				}
			]
		}))
		.unwrap();

		assert_eq!(document.search_writers(), vec!["Becky Chambers"]);
	}

	#[test]
	fn pasted_bearer_prefix_is_not_sent_twice() {
		for raw in ["tok", "Bearer tok", "bearer  tok ", " BEARER tok\n"] {
			assert_eq!(super::normalize_token(raw), "tok", "input {raw:?}");
		}
	}
}
