//! Audible metadata provider, over two unauthenticated upstreams.
//!
//! [Audnexus](https://api.audnex.us) is the record source: `GET /books/{asin}`
//! returns one harmonised audiobook edition (title, subtitle, authors,
//! narrators, series and position, plain-text description, genres and tags,
//! publisher, release date, cover and advertised runtime) and
//! `GET /authors/{asin}` resolves an author a book record names by ASIN
//! alone. It is community-run with a published ceiling of 100 requests per
//! minute, so this client stays under it and identifies itself.
//!
//! The Audible catalog (`GET https://api.audible.com/1.0/catalog/products`) is
//! used for one thing: turning a title into ASINs, because Audnexus has no
//! search endpoint. Its products already carry enough fields to build a
//! candidate, so a search hit needs no per-hit Audnexus fetch;
//! `fetch_media_metadata` reads the richer Audnexus record for whichever ASIN
//! the operator picked.
//!
//! The two upstreams spell the same book differently -- Audnexus is camelCase
//! (`publisherName`, `runtimeLengthMin`), the catalog snake_case
//! (`publisher_name`, `runtime_length_min`) -- and disagree about which fields
//! exist at all, so they get one DTO each instead of a shared struct behind a
//! dozen aliases.
//!
//! External ids are Audible ASINs (`B002V02KPU`), which is exactly what
//! `MetadataField::IdentifierMobiAsin` names, so the identifier needs no new
//! carrier: `external_id` is the ASIN, and the ingest facade folds that into
//! the candidate's identifiers.
//!
//! Audnexus has no series entity, so the series half of the trait reports
//! `OperationNotSupported`; the ingest facade tolerates that as long as the
//! media search succeeds.

use std::collections::HashMap;

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

/// Audnexus documents 100 requests per minute
/// (https://github.com/laxamentumtech/audnexus#-deployment-). It is a
/// community-run service paying its own bills, so this leaves headroom rather
/// than sitting on the published ceiling; the quota is a minute window, so it
/// is expressed as one.
const AUDIBLE_DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 60;

/// `num_results` is capped at 50 by the catalog endpoint.
const CATALOG_MAX_RESULTS: u32 = 50;

/// Authors expanded per record; a record naming twenty contributors by ASIN
/// alone cannot spend the whole rate-limit budget on one lookup.
const AUDIBLE_MAX_AUTHOR_FETCHES: usize = 5;

/// The catalog answers with almost nothing unless response groups are named.
/// These five carry the fields a candidate is built from: `product_desc`
/// (blurb), `product_attrs` (title, subtitle, release date, runtime),
/// `contributors` (authors, narrators), `series`, `media` (cover images).
const CATALOG_RESPONSE_GROUPS: &str =
	"product_desc,product_attrs,contributors,series,media";

/// Cover sizes to ask for; `product_images` only carries the widths named
/// here, and the widest one that comes back wins.
const CATALOG_IMAGE_SIZES: &str = "500,1024";

const CATALOG_SEARCH_PATH: &str = "/1.0/catalog/products";

/// The hint key a caller uses to pass an ASIN it already knows, mirroring
/// `comic_vine_volume_id`.
const ASIN_HINT: &str = "audible_asin";

/// A stable, long-published title used only to prove the API answers.
const REACHABILITY_ASIN: &str = "B002V02KPU";

pub struct AudibleClient {
	client: ClientWithMiddleware,
	audnex_url: String,
	catalog_url: String,
	rate_limiter: RateLimiter,
}

impl Default for AudibleClient {
	fn default() -> Self {
		Self::new()
	}
}

impl AudibleClient {
	const AUDNEX_URL: &'static str = "https://api.audnex.us";
	const CATALOG_URL: &'static str = "https://api.audible.com";
	const USER_AGENT: &'static str = concat!(
		"stump/",
		env!("CARGO_PKG_VERSION"),
		" (+https://github.com/stumpapp/stump)"
	);

	/// Both upstreams are keyless, so there is nothing to configure.
	pub fn new() -> Self {
		Self::build(
			Self::AUDNEX_URL.to_string(),
			Self::CATALOG_URL.to_string(),
			AUDIBLE_DEFAULT_RATE_LIMIT_PER_MINUTE,
			RetryClientConfig::default(),
		)
	}

	fn build(
		audnex_url: String,
		catalog_url: String,
		rate_limit_per_minute: u32,
		retry: RetryClientConfig,
	) -> Self {
		let inner = reqwest::Client::builder()
			.user_agent(Self::USER_AGENT)
			.build()
			.expect("Failed to build Audible HTTP client"); // static config, cannot fail
		Self {
			client: build_client_with_retry(inner, retry),
			audnex_url,
			catalog_url,
			rate_limiter: RateLimiter::per_minute(rate_limit_per_minute),
		}
	}

	/// Test-only override of the Audnexus base URL, used to point the client
	/// at a local mock server. Separate from the catalog base so a test can
	/// aim both at one server and still tell the two endpoints apart by path.
	#[cfg(test)]
	fn with_audnex_url(mut self, audnex_url: impl Into<String>) -> Self {
		self.audnex_url = audnex_url.into();
		self
	}

	/// Test-only override of the Audible catalog base URL.
	#[cfg(test)]
	fn with_catalog_url(mut self, catalog_url: impl Into<String>) -> Self {
		self.catalog_url = catalog_url.into();
		self
	}

	/// Point both bases at one local server, for tests in crates that consume
	/// [`crate::editions::EditionLookup`] and cannot reach the two private
	/// overrides above.
	#[cfg(any(test, feature = "mock"))]
	#[must_use]
	pub fn pointed_at(mut self, base_url: &str) -> Self {
		self.audnex_url = base_url.to_string();
		self.catalog_url = base_url.to_string();
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
				.expect("Failed to build Audible HTTP client"),
			RetryClientConfig { max_retries: 0 },
		);
		self.rate_limiter = RateLimiter::new(u32::MAX);
		self
	}

	/// GET a JSON document from one of the two bases. A 404 surfaces as
	/// [`MetadataProviderError::NotFound`], which is also how Audnexus
	/// answers for an ASIN that exists in another region.
	async fn get<T: serde::de::DeserializeOwned>(
		&self,
		base: &str,
		path: &str,
		params: &[(&str, &str)],
	) -> Result<T, MetadataProviderError> {
		self.rate_limiter.until_ready().await;

		let mut url = Url::parse(&format!("{}{}", base, path))
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

	async fn fetch_book(&self, asin: &str) -> Result<AudnexBook, MetadataProviderError> {
		self.get(&self.audnex_url, &format!("/books/{asin}"), &[])
			.await
	}

	async fn search_catalog(
		&self,
		query: &SearchQuery,
	) -> Result<Vec<CatalogProduct>, MetadataProviderError> {
		let num_results = query
			.limit
			.unwrap_or(10)
			.clamp(1, CATALOG_MAX_RESULTS)
			.to_string();
		let mut params = vec![
			("title", query.title.trim()),
			("num_results", num_results.as_str()),
			("products_sort_by", "Relevance"),
			("response_groups", CATALOG_RESPONSE_GROUPS),
			("image_sizes", CATALOG_IMAGE_SIZES),
		];
		if let Some(author) = query
			.author
			.as_deref()
			.map(str::trim)
			.filter(|author| !author.is_empty())
		{
			params.push(("author", author));
		}
		let response: CatalogResponse = self
			.get(&self.catalog_url, CATALOG_SEARCH_PATH, &params)
			.await?;
		Ok(response.products)
	}

	/// Author display names, expanding only the entries Audnexus names by
	/// ASIN alone. Individual misses are tolerated: a candidate with one
	/// author missing is worth more than no candidate.
	async fn author_names(&self, authors: Vec<AudnexPerson>) -> Vec<String> {
		let mut names = Vec::with_capacity(authors.len());
		let mut fetches = 0usize;
		for author in authors {
			if let Some(name) = non_empty(author.name) {
				names.push(name);
				continue;
			}
			let Some(asin) = non_empty(author.asin) else {
				continue;
			};
			if fetches >= AUDIBLE_MAX_AUTHOR_FETCHES {
				tracing::debug!(
					asin,
					"Audnexus author expansion cap reached; author left unnamed"
				);
				continue;
			}
			fetches += 1;
			match self
				.get::<AudnexAuthor>(&self.audnex_url, &format!("/authors/{asin}"), &[])
				.await
			{
				Ok(fetched) => {
					if let Some(name) = non_empty(fetched.name) {
						names.push(name);
					}
				},
				Err(error) => tracing::warn!(
					asin,
					?error,
					"Audnexus author lookup failed; skipping"
				),
			}
		}
		names
	}

	/// Map an Audnexus book onto media metadata. The only extra requests are
	/// author lookups, and only for authors the record leaves unnamed.
	async fn media_from_book(&self, book: AudnexBook) -> ExternalMediaMetadata {
		let asin = book.asin;
		let writers = self.author_names(book.authors).await;
		let (genres, tags) = split_genres(book.genres);
		let (year, month, day) = parse_release_date(book.release_date.as_deref());
		let (isbn, isbn_13) = split_isbn(book.isbn);
		// Only the primary series is carried: the metadata type has one
		// `series_name`/`number` pair, and the primary series is the one a
		// shelf is built from.
		let (series_name, number) = match book.series_primary {
			Some(series) => (
				non_empty(series.name),
				series.position.as_deref().and_then(parse_series_position),
			),
			None => (None, None),
		};

		ExternalMediaMetadata {
			provider: self.id().to_string(),
			external_id: asin.clone(),
			title: non_empty(book.title),
			subtitle: non_empty(book.subtitle),
			summary: plain_text(book.description),
			// Audiobooks have no pages, and the runtime is not a page count.
			page_count: None,
			series_name,
			number,
			year,
			month,
			day,
			genres,
			tags,
			isbn,
			isbn_13,
			writers: filled_or_none(writers),
			narrators: filled_or_none(
				book.narrators
					.into_iter()
					.filter_map(|person| non_empty(person.name))
					.collect(),
			),
			publisher: non_empty(book.publisher_name),
			runtime_minutes: book.runtime_length_min,
			cover_url: non_empty(book.image),
			provider_url: Some(product_url(&asin)),
			..Default::default()
		}
	}
}

#[async_trait::async_trait]
impl MetadataProvider for AudibleClient {
	fn id(&self) -> &'static str {
		"audible"
	}

	fn name(&self) -> &'static str {
		"Audible"
	}

	fn supported_media_types(&self) -> Vec<MediaType> {
		vec![MediaType::Book]
	}

	/// Neither upstream has a series entity: Audnexus exposes books, authors
	/// and chapters only, and the catalog's `series` block is a label on a
	/// product rather than a record of its own.
	async fn search_series(
		&self,
		_query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		Err(MetadataProviderError::OperationNotSupported)
	}

	/// An ASIN the caller already knows is definitive, so it goes straight to
	/// the Audnexus record and the catalog is never touched -- and a miss is
	/// an error rather than an excuse to guess by title, because the title of
	/// an ASIN-shaped query is the ASIN itself.
	#[tracing::instrument(skip(self))]
	async fn search_media(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		if let Some(asin) = query_asin(query) {
			let book = self.fetch_book(&asin).await?;
			let metadata = self.media_from_book(book).await;
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
		}

		let products = self.search_catalog(query).await?;
		let requested = products.len();
		let candidates = products
			.into_iter()
			.filter_map(|product| {
				let metadata = media_from_product(self.id(), product)?;
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

	/// The catalog search alone, in the catalog's own relevance order: no
	/// ASIN shortcut (an ASIN-shaped query is not a type-ahead term) and no
	/// re-scoring, so the caller sees what Audible ranks first.
	#[tracing::instrument(skip(self))]
	async fn search_media_brief(
		&self,
		query: &SearchQuery,
	) -> Result<SearchOutcome, MetadataProviderError> {
		let products = self.search_catalog(query).await?;
		let requested = products.len();
		let candidates = products
			.into_iter()
			.filter_map(|product| {
				let metadata = media_from_product(self.id(), product)?;
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
			candidates,
			requested,
		})
	}

	async fn fetch_series_metadata(
		&self,
		_external_id: &str,
	) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
		Err(MetadataProviderError::OperationNotSupported)
	}

	/// `external_id` is the ASIN, which is what every candidate from this
	/// provider exposes.
	async fn fetch_media_metadata(
		&self,
		external_id: &str,
	) -> Result<ExternalMediaMetadata, MetadataProviderError> {
		let asin = external_id.trim().to_ascii_uppercase();
		let book = self.fetch_book(&asin).await?;
		Ok(self.media_from_book(book).await)
	}

	/// Both upstreams are keyless; there are no credentials to verify, so one
	/// cheap record fetch proves the API is reachable.
	#[tracing::instrument(skip(self))]
	async fn verify_credentials(
		&self,
	) -> Result<ProviderCredentialVerification, MetadataProviderError> {
		self.rate_limiter.until_ready().await;
		let response = self
			.client
			.get(format!("{}/books/{REACHABILITY_ASIN}", self.audnex_url))
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

/// Audnexus' edition graph is one hop wide and only in one direction: an
/// audiobook record carries the print edition's `isbn`, and there is no
/// endpoint that walks back from an ISBN to an ASIN. That one hop is the hop
/// pairing needs, because it is the only place an Audible ASIN and a book
/// ISBN appear in the same document.
#[async_trait::async_trait]
impl EditionLookup for AudibleClient {
	fn provider_id(&self) -> &'static str {
		"AUDIBLE"
	}

	async fn sibling_identifiers(
		&self,
		identifier: &EditionIdentifier,
	) -> Result<Vec<EditionIdentifier>, MetadataProviderError> {
		let EditionIdentifier::Asin(asin) = identifier else {
			return Ok(Vec::new());
		};

		match self.fetch_book(asin).await {
			Ok(book) => Ok(book
				.isbn
				.and_then(|isbn| EditionIdentifier::Isbn(isbn).normalized())
				.into_iter()
				.collect()),
			// An ASIN Audnexus does not carry (a regional title, a podcast)
			// is a miss, not a failure: pairing simply learns nothing from
			// this provider.
			Err(MetadataProviderError::NotFound(_)) => Ok(Vec::new()),
			Err(error) => Err(error),
		}
	}
}

/// Map a catalog product straight onto media metadata. A product with no ASIN
/// has no identity to fetch or store against, so it is dropped -- and counted,
/// through [`SearchOutcome::failed`]. So is anything the catalog labels a
/// podcast, episode or periodical: the store lists them beside audiobooks,
/// but no book on a shelf is one.
fn media_from_product(
	provider_id: &str,
	product: CatalogProduct,
) -> Option<ExternalMediaMetadata> {
	let asin = non_empty(product.asin)?;
	if [&product.content_type, &product.content_delivery_type]
		.into_iter()
		.flatten()
		.any(|kind| is_non_book_content(kind))
	{
		return None;
	}
	let (year, month, day) = parse_release_date(product.release_date.as_deref());
	// The catalog lists every series a product belongs to; the first is the
	// primary one, and the metadata type has room for exactly one.
	let (series_name, number) = match product.series.into_iter().next() {
		Some(series) => (
			non_empty(series.title),
			series.sequence.as_deref().and_then(parse_series_position),
		),
		None => (None, None),
	};

	Some(ExternalMediaMetadata {
		provider: provider_id.to_string(),
		external_id: asin.clone(),
		title: non_empty(product.title),
		subtitle: non_empty(product.subtitle),
		// `publisher_summary` is the long blurb and is HTML on many records;
		// `merchandising_summary` is the short plain-text one. Whichever is
		// prose wins.
		summary: plain_text(product.publisher_summary)
			.or_else(|| plain_text(product.merchandising_summary)),
		// Audiobooks have no pages.
		page_count: None,
		series_name,
		number,
		year,
		month,
		day,
		// The catalog exposes categories only under the `category_ladders`
		// response group, whose nested ladders are a different mapping
		// problem; the Audnexus record an operator fetches next carries
		// genres and tags outright, so search candidates go without.
		genres: None,
		tags: None,
		// The catalog never carries a print ISBN, and inventing one would let
		// the scorer confirm a match on a value nobody supplied.
		isbn: None,
		isbn_13: None,
		writers: filled_or_none(
			product
				.authors
				.into_iter()
				.filter_map(|person| non_empty(person.name))
				.collect(),
		),
		narrators: filled_or_none(
			product
				.narrators
				.into_iter()
				.filter_map(|person| non_empty(person.name))
				.collect(),
		),
		publisher: non_empty(product.publisher_name),
		runtime_minutes: product.runtime_length_min,
		cover_url: largest_image(product.product_images),
		provider_url: Some(product_url(&asin)),
		has_audiobook: Some(true),
		abridged: abridged_from_format_type(product.format_type.as_deref()),
		language: non_empty(product.language).map(|language| language.to_lowercase()),
		..Default::default()
	})
}

/// The catalog's `content_type`/`content_delivery_type` labels for things
/// that are not books: `Podcast`, `PodcastEpisode`, `PodcastParent`,
/// `Periodical`, and the newspaper/magazine editions sold as subscriptions.
fn is_non_book_content(kind: &str) -> bool {
	let kind = kind.trim().to_ascii_lowercase();
	kind.contains("podcast")
		|| kind.contains("periodical")
		|| kind.contains("newspaper")
		|| kind.contains("magazine")
}

/// `format_type` is the catalog's abridgement label: `abridged` or
/// `unabridged` on audiobooks. Anything else (podcasts, unstated) is unknown
/// rather than a guess either way.
fn abridged_from_format_type(format_type: Option<&str>) -> Option<bool> {
	match format_type?.trim().to_ascii_lowercase().as_str() {
		"abridged" => Some(true),
		"unabridged" => Some(false),
		_ => None,
	}
}

/// The ASIN a search should resolve directly, if the caller supplied one.
///
/// `SearchQuery` has no ASIN field, so it arrives either as a provider hint or
/// as the title itself (which is how the ingest facade passes a filename that
/// is already an ASIN). `SearchQuery::isbn` is deliberately never consulted:
/// [`normalize_isbn`] strips every letter, so an ASIN parked there arrives as
/// `02` and would resolve some other book entirely.
fn query_asin(query: &SearchQuery) -> Option<String> {
	[
		query.provider_hints.get(ASIN_HINT).map(String::as_str),
		Some(query.title.as_str()),
	]
	.into_iter()
	.flatten()
	.map(|value| value.trim().to_ascii_uppercase())
	.find(|value| looks_like_asin(value))
}

/// Audible product ASINs are ten upper-case alphanumerics beginning `B0`.
/// Audiobookshelf accepts any ten upper-case alphanumerics
/// (`server/utils/index.js:260`), but that also matches a bare ISBN-10 and any
/// ten-letter shouted title, so the `B0` prefix stays as the guard.
fn looks_like_asin(value: &str) -> bool {
	value.len() == 10
		&& value.starts_with("B0")
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn product_url(asin: &str) -> String {
	format!("https://www.audible.com/pd/{asin}")
}

/// Audnexus labels every entry `genre` or `tag`, so a browse-node tag
/// ("Classics") never lands in a genre column next to a real genre
/// ("Literature & Fiction"). The tags are not discarded -- they are exactly
/// what `tags` is for.
fn split_genres(genres: Vec<AudnexGenre>) -> (Option<Vec<String>>, Option<Vec<String>>) {
	let mut mapped = Vec::new();
	let mut tags = Vec::new();
	for entry in genres {
		let Some(name) = non_empty(entry.name) else {
			continue;
		};
		match entry.kind.as_deref() {
			Some("tag") => tags.push(name),
			// An entry with no type at all is a genre: that is what the field
			// is called, and the only labels Audnexus emits are the two.
			Some("genre") | None => mapped.push(name),
			Some(other) => tracing::debug!(
				kind = other,
				name,
				"Unknown Audnexus genre type; skipping"
			),
		}
	}
	(filled_or_none(mapped), filled_or_none(tags))
}

/// Audnexus carries the print edition's ISBN on some records and nothing on
/// most; nothing is fabricated when it is absent. A value that is neither ten
/// nor thirteen digits long is not an ISBN and is dropped rather than shown to
/// the scorer as one.
fn split_isbn(isbn: Option<String>) -> (Option<String>, Option<String>) {
	let Some(isbn) = non_empty(isbn) else {
		return (None, None);
	};
	let normalized = normalize_isbn(&isbn);
	match normalized.len() {
		13 => (None, Some(normalized)),
		10 => (Some(normalized), None),
		_ => (None, None),
	}
}

/// `product_images` is keyed by pixel width (`"500"`, `"1024"`); take the
/// widest one Audible actually returned.
fn largest_image(images: HashMap<String, String>) -> Option<String> {
	images
		.into_iter()
		.filter_map(|(width, url)| {
			let width = width.parse::<u32>().ok()?;
			non_empty(Some(url)).map(|url| (width, url))
		})
		.max_by_key(|(width, _)| *width)
		.map(|(_, url)| url)
}

/// Audnexus sends an ISO-8601 instant (`2009-09-18T00:00:00.000Z`), the
/// catalog a bare calendar date (`2018-06-14`). Both go through
/// `dateparser::parse_with_timezone(.., &Utc)`, never `dateparser::parse`,
/// which anchors a date-only value at local midnight and so loses a day east
/// of UTC. Anything unparseable degrades to a four-digit year.
fn parse_release_date(date: Option<&str>) -> (Option<i32>, Option<i32>, Option<i32>) {
	use chrono::Datelike;
	let Some(date) = date.map(str::trim).filter(|date| !date.is_empty()) else {
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

/// Series positions are free text: `1`, `Book 1`, `2, Dramatized Adaptation`
/// (advplyr/audiobookshelf#1339 and #2380 are both this field). Take the
/// leading number with an optional fractional part and ignore the prose around
/// it; a position with no number in it at all (`Part One`) has no numeric form
/// and stays empty rather than becoming a wrong one.
fn parse_series_position(position: &str) -> Option<f32> {
	let bytes = position.as_bytes();
	let start = bytes
		.iter()
		.position(|byte| byte.is_ascii_digit() || *byte == b'.')?;
	let mut end = start;
	let mut seen_dot = false;
	while end < bytes.len() {
		match bytes[end] {
			b'0'..=b'9' => end += 1,
			b'.' if !seen_dot => {
				seen_dot = true;
				end += 1;
			},
			_ => break,
		}
	}
	// `start..end` is a run of ASCII digits and at most one `.`, so it is a
	// character boundary and parses as a float or not at all (`.` alone).
	position[start..end].parse().ok()
}

/// A blurb is only used when it is prose. Both upstreams have summary fields
/// that arrive wrapped in HTML on some records, and this crate's only stripper
/// is private to the AniList provider; markup in an operator-facing summary is
/// worse than none, and every record here carries a plain-text blurb in some
/// other field.
fn plain_text(value: Option<String>) -> Option<String> {
	non_empty(value).filter(|value| !value.contains('<'))
}

fn non_empty(value: Option<String>) -> Option<String> {
	value.filter(|value| !value.trim().is_empty())
}

fn filled_or_none(values: Vec<String>) -> Option<Vec<String>> {
	(!values.is_empty()).then(|| {
		let mut values = values;
		values.dedup();
		values
	})
}

/// One Audnexus book. `asin` is required: Audnexus answers an unknown or
/// region-locked ASIN with an error object, and a record without an identity
/// is one this provider could never fetch again.
///
/// Fields deliberately not mapped, all for want of a carrier on
/// [`ExternalMediaMetadata`]: `summary` (the HTML twin of `description`),
/// `formatType` (`abridged`/`unabridged` -- `runtime_minutes` is the tell an
/// operator actually reads), `language`, `literatureType` (fiction or not),
/// `isAdult` and `rating` (no age-rating or rating scalar on a media item),
/// `region` (a request scope, not a fact about the book), and `seriesSecondary`
/// (one series pair exists; see [`AudibleClient::media_from_book`]).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudnexBook {
	asin: String,
	title: Option<String>,
	subtitle: Option<String>,
	/// The plain-text blurb; `summary` is the same text wrapped in `<p>`.
	description: Option<String>,
	#[serde(default)]
	authors: Vec<AudnexPerson>,
	#[serde(default)]
	narrators: Vec<AudnexPerson>,
	#[serde(default)]
	genres: Vec<AudnexGenre>,
	publisher_name: Option<String>,
	release_date: Option<String>,
	runtime_length_min: Option<i32>,
	series_primary: Option<AudnexSeries>,
	image: Option<String>,
	isbn: Option<String>,
}

/// A contributor. Authors carry an ASIN, narrators do not; either may arrive
/// with only one of the two.
#[derive(Debug, Deserialize)]
struct AudnexPerson {
	asin: Option<String>,
	name: Option<String>,
}

/// `type` is `genre` or `tag`. The sibling `asin` is an Audible browse-node
/// id, which has no carrier and nothing to resolve it against.
#[derive(Debug, Deserialize)]
struct AudnexGenre {
	name: Option<String>,
	#[serde(rename = "type")]
	kind: Option<String>,
}

/// `asin` is not mapped onto `series_external_id`: Audnexus has no series
/// endpoint, so an id no call in this trait could resolve would be a dead
/// promise.
#[derive(Debug, Deserialize)]
struct AudnexSeries {
	name: Option<String>,
	position: Option<String>,
}

/// `GET /authors/{asin}`. Only the name is used -- the author's own
/// description, image and genres describe the author, not this book.
#[derive(Debug, Deserialize)]
struct AudnexAuthor {
	name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogResponse {
	#[serde(default)]
	products: Vec<CatalogProduct>,
}

/// One catalog product, as returned for [`CATALOG_RESPONSE_GROUPS`].
///
/// Fields deliberately not mapped: `publication_datetime` and `issue_date`
/// (the same calendar date as `release_date`), `publication_name` (the series
/// title with edition noise baked in, and `series[].title` is the same value
/// from the carrier that owns it), `available_codecs`,
/// `content_delivery_type`, `is_listenable`, `has_children`, `asset_details`,
/// `sku`/`sku_lite` (store and delivery attributes -- the audio facts Stump
/// keeps are measured from the file, not advertised), `social_media_images`
/// (share cards, not covers) and `is_adult_product` (no carrier, as on the
/// Audnexus record). `format_type` is carried only as the abridged flag,
/// `language` lowercased as the edition language, and
/// `content_type`/`content_delivery_type` only decide whether the product is
/// a book at all.
#[derive(Debug, Deserialize)]
struct CatalogProduct {
	asin: Option<String>,
	title: Option<String>,
	subtitle: Option<String>,
	#[serde(default)]
	authors: Vec<CatalogPerson>,
	#[serde(default)]
	narrators: Vec<CatalogPerson>,
	#[serde(default)]
	series: Vec<CatalogSeries>,
	publisher_name: Option<String>,
	release_date: Option<String>,
	runtime_length_min: Option<i32>,
	#[serde(default)]
	product_images: HashMap<String, String>,
	publisher_summary: Option<String>,
	merchandising_summary: Option<String>,
	format_type: Option<String>,
	content_type: Option<String>,
	content_delivery_type: Option<String>,
	language: Option<String>,
}

/// A catalog contributor. The sibling `asin` on an author is not mapped: the
/// catalog always names its contributors, so there is nothing to expand.
#[derive(Debug, Deserialize)]
struct CatalogPerson {
	name: Option<String>,
}

/// The catalog spells a series `title`/`sequence` where Audnexus spells it
/// `name`/`position`. Its `asin` and `url` are not mapped, for the same reason
/// as [`AudnexSeries`].
#[derive(Debug, Deserialize)]
struct CatalogSeries {
	title: Option<String>,
	sequence: Option<String>,
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

	/// Both bases point at the one mock server, so a test can prove which
	/// endpoint was called -- and that the other was not.
	fn client(server: &MockServer) -> AudibleClient {
		AudibleClient::new()
			.with_audnex_url(server.url.clone())
			.with_catalog_url(server.url.clone())
	}

	fn book_response() -> String {
		serde_json::json!({
			"asin": "B002V02KPU",
			"title": "The Long Way to a Small, Angry Planet",
			"subtitle": "Wayfarers, Book 1",
			"description": "Rosemary Harper joins the crew of the Wayfarer.",
			"summary": "<p>Rosemary Harper joins the crew of the Wayfarer.</p>",
			"authors": [{ "asin": "B00LUQWGAU", "name": "Becky Chambers" }],
			"narrators": [{ "name": "Rachel Dulude" }],
			"genres": [
				{ "asin": "18580606011", "name": "Science Fiction & Fantasy", "type": "genre" },
				{ "asin": "18580628011", "name": "Space Opera", "type": "tag" }
			],
			"publisherName": "Hodder & Stoughton",
			"releaseDate": "2016-07-14T00:00:00.000Z",
			"runtimeLengthMin": 862,
			"seriesPrimary": { "asin": "B08RLSPY4J", "name": "Wayfarers", "position": "1" },
			"seriesSecondary": { "asin": "B0BQZ8CXG3", "name": "Booktrack Editions", "position": "1" },
			"image": "https://m.media-amazon.com/images/I/91vS2L5YfEL.jpg",
			"isbn": "978-1-4736-1954-4",
			"formatType": "unabridged",
			"language": "english",
			"region": "us",
			"rating": "4.4"
		})
		.to_string()
	}

	fn catalog_response() -> String {
		serde_json::json!({
			"total_results": 2,
			"products": [
				{
					"asin": "B07DFN5FBP",
					"title": "The Long Way to a Small, Angry Planet",
					"subtitle": "the most hopeful novel to curl up with",
					"authors": [{ "asin": "B00LUQWGAU", "name": "Becky Chambers" }],
					"narrators": [{ "name": "Patricia Rodriguez" }],
					"series": [{
						"asin": "B07KQGBL98",
						"sequence": "1",
						"title": "Wayfarers",
						"url": "/pd/Wayfarers-Audiobook/B07KQGBL98"
					}],
					"publisher_name": "Hodderscape",
					"release_date": "2018-06-14",
					"runtime_length_min": 941,
					"product_images": {
						"500": "https://m.media-amazon.com/images/I/514i3TxHYqL._SL500_.jpg",
						"1024": "https://m.media-amazon.com/images/I/A1audpLofnL._SL1024_.jpg"
					},
					"merchandising_summary": "The beloved debut novel.",
					"publisher_summary": "<p>The beloved debut novel.</p>",
					"issue_date": "2018-06-14",
					"language": "english",
					"format_type": "unabridged"
				},
				{
					"title": "A product the catalog forgot to identify",
					"release_date": "2019-01-01"
				}
			]
		})
		.to_string()
	}

	#[tokio::test]
	async fn fetch_media_metadata_maps_every_audnexus_field() {
		let server = MockServer::spawn(vec![render_ok(&book_response())]);
		let client = client(&server);

		let media = client
			.fetch_media_metadata("b002v02kpu")
			.await
			.expect("book lookup should succeed against the mock");

		assert_eq!(media.provider, "audible");
		assert_eq!(media.external_id, "B002V02KPU");
		assert_eq!(
			media.title.as_deref(),
			Some("The Long Way to a Small, Angry Planet")
		);
		assert_eq!(media.subtitle.as_deref(), Some("Wayfarers, Book 1"));
		assert_eq!(
			media.summary.as_deref(),
			Some("Rosemary Harper joins the crew of the Wayfarer."),
			"the plain-text description wins over the HTML summary"
		);
		assert_eq!(
			media.writers.as_deref(),
			Some(["Becky Chambers".to_string()].as_slice())
		);
		assert_eq!(
			media.narrators.as_deref(),
			Some(["Rachel Dulude".to_string()].as_slice())
		);
		assert_eq!(media.series_name.as_deref(), Some("Wayfarers"));
		assert_eq!(media.number, Some(1.0));
		assert_eq!(
			media.genres.as_deref(),
			Some(["Science Fiction & Fantasy".to_string()].as_slice())
		);
		assert_eq!(
			media.tags.as_deref(),
			Some(["Space Opera".to_string()].as_slice()),
			"a browse-node tag must not land among the genres"
		);
		assert_eq!(media.publisher.as_deref(), Some("Hodder & Stoughton"));
		assert_eq!(
			(media.year, media.month, media.day),
			(Some(2016), Some(7), Some(14))
		);
		assert_eq!(media.runtime_minutes, Some(862));
		assert_eq!(media.isbn_13.as_deref(), Some("9781473619544"));
		assert_eq!(media.isbn, None);
		assert_eq!(media.page_count, None);
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://m.media-amazon.com/images/I/91vS2L5YfEL.jpg")
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://www.audible.com/pd/B002V02KPU")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1, "the named author needs no expansion");
		let request = &requests[0];
		assert!(request.starts_with("GET /books/B002V02KPU"), "{request}");
		assert!(
			request.to_ascii_lowercase().contains("user-agent: stump/"),
			"{request}"
		);
	}

	#[tokio::test]
	async fn search_media_maps_catalog_products_with_search_params() {
		let server = MockServer::spawn(vec![render_ok(&catalog_response())]);
		let client = client(&server);

		let outcome = client
			.search_media(&SearchQuery {
				author: Some("Becky Chambers".to_string()),
				..query("The Long Way to a Small, Angry Planet")
			})
			.await
			.expect("catalog search should succeed against the mock");

		assert_eq!(outcome.requested, 2);
		assert_eq!(outcome.candidates.len(), 1);
		assert_eq!(
			outcome.failed(),
			1,
			"the product without an ASIN is dropped"
		);

		let candidate = &outcome.candidates[0];
		assert_eq!(candidate.provider, "audible");
		assert_eq!(candidate.external_id, "B07DFN5FBP");
		assert!(candidate.confidence >= 0.9, "{}", candidate.confidence);

		let media = candidate.metadata.as_media().unwrap();
		assert_eq!(
			media.title.as_deref(),
			Some("The Long Way to a Small, Angry Planet")
		);
		assert_eq!(
			media.subtitle.as_deref(),
			Some("the most hopeful novel to curl up with")
		);
		assert_eq!(
			media.summary.as_deref(),
			Some("The beloved debut novel."),
			"the HTML publisher summary is declined for the plain-text one"
		);
		assert_eq!(
			media.writers.as_deref(),
			Some(["Becky Chambers".to_string()].as_slice())
		);
		assert_eq!(
			media.narrators.as_deref(),
			Some(["Patricia Rodriguez".to_string()].as_slice())
		);
		assert_eq!(media.series_name.as_deref(), Some("Wayfarers"));
		assert_eq!(media.number, Some(1.0));
		assert_eq!(media.publisher.as_deref(), Some("Hodderscape"));
		assert_eq!(
			(media.year, media.month, media.day),
			(Some(2018), Some(6), Some(14))
		);
		assert_eq!(media.runtime_minutes, Some(941));
		assert_eq!(
			media.cover_url.as_deref(),
			Some("https://m.media-amazon.com/images/I/A1audpLofnL._SL1024_.jpg"),
			"the widest image wins"
		);
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://www.audible.com/pd/B07DFN5FBP")
		);
		assert_eq!(media.has_audiobook, Some(true));
		assert_eq!(media.abridged, Some(false), "format_type unabridged");
		assert_eq!(media.language.as_deref(), Some("english"));

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		let request = &requests[0];
		assert!(
			request.starts_with("GET /1.0/catalog/products?"),
			"{request}"
		);
		assert!(
			request.contains("title=The+Long+Way+to+a+Small%2C+Angry+Planet"),
			"{request}"
		);
		assert!(request.contains("num_results=5"), "{request}");
		assert!(request.contains("products_sort_by=Relevance"), "{request}");
		assert!(
			request.contains(
				"response_groups=product_desc%2Cproduct_attrs%2Ccontributors%2Cseries%2Cmedia"
			),
			"{request}"
		);
		assert!(request.contains("author=Becky+Chambers"), "{request}");
	}

	#[tokio::test]
	async fn brief_search_keeps_catalog_order_and_never_shortcuts_to_audnexus() {
		let mut body: serde_json::Value =
			serde_json::from_str(&catalog_response()).unwrap();
		body["products"][1] = serde_json::json!({
			"asin": "B0ABRIDGED",
			"title": "The Long Way to a Small, Angry Planet",
			"format_type": "Abridged"
		});
		let products = body["products"].as_array_mut().unwrap();
		products.push(serde_json::json!({
			"asin": "B0GTXCMJM4",
			"title": "3/25 Wednesday Hr 1: Harry Potter T...",
			"content_type": "Podcast",
			"content_delivery_type": "PodcastEpisode",
			"runtime_length_min": 40
		}));
		products.push(serde_json::json!({
			"asin": "B0PERIODIC",
			"title": "The New York Times Audio Digest",
			"content_delivery_type": "Periodical"
		}));
		let server = MockServer::spawn(vec![render_ok(&body.to_string())]);
		let client = client(&server);

		let outcome = client
			.search_media_brief(&query("B0ABRIDGED"))
			.await
			.expect("brief search should hit the catalog");

		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(
			requests[0].starts_with("GET /1.0/catalog/products?"),
			"an ASIN-shaped brief query still searches the catalog: {}",
			requests[0]
		);
		assert_eq!(outcome.requested, 4);
		assert_eq!(
			outcome.failed(),
			2,
			"podcast and periodical products are dropped"
		);
		let ids: Vec<_> = outcome
			.candidates
			.iter()
			.map(|candidate| candidate.external_id.as_str())
			.collect();
		assert_eq!(ids, ["B07DFN5FBP", "B0ABRIDGED"], "catalog order is kept");
		assert!(outcome
			.candidates
			.iter()
			.all(|candidate| candidate.confidence == 0.0));
		let abridged = outcome.candidates[1].metadata.as_media().unwrap();
		assert_eq!(abridged.abridged, Some(true));
		assert_eq!(abridged.runtime_minutes, None);
	}

	#[tokio::test]
	async fn asin_shaped_query_skips_the_catalog() {
		let server = MockServer::spawn(vec![render_ok(&book_response())]);
		let client = client(&server);

		let outcome = client
			.search_media(&query("B002V02KPU"))
			.await
			.expect("an ASIN-shaped query should resolve the record directly");

		assert_eq!(outcome.requested, 1);
		assert_eq!(outcome.candidates[0].external_id, "B002V02KPU");
		assert_eq!(
			outcome.candidates[0]
				.metadata
				.as_media()
				.unwrap()
				.title
				.as_deref(),
			Some("The Long Way to a Small, Angry Planet")
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 1, "the catalog must not be consulted");
		assert!(
			requests[0].starts_with("GET /books/B002V02KPU"),
			"{}",
			requests[0]
		);
	}

	#[test]
	fn query_asin_takes_the_hint_first_and_rejects_non_asins() {
		let hinted = SearchQuery {
			provider_hints: [(ASIN_HINT.to_string(), "b0dftwwg3s".to_string())]
				.into_iter()
				.collect(),
			..query("The Long Way to a Small, Angry Planet")
		};
		assert_eq!(query_asin(&hinted).as_deref(), Some("B0DFTWWG3S"));

		// A title that is itself an ASIN is how a filename-derived query
		// arrives.
		assert_eq!(
			query_asin(&query("b002v02kpu")).as_deref(),
			Some("B002V02KPU")
		);
		// An ISBN-10, a ten-letter title, and a real title are all titles.
		assert_eq!(query_asin(&query("0140328726")), None);
		assert_eq!(query_asin(&query("SANDMANXYZ")), None);
		assert_eq!(query_asin(&query("The Martian")), None);
		// An ASIN in the ISBN field is unusable, so it is not consulted.
		assert_eq!(
			query_asin(&SearchQuery {
				isbn: Some("B002V02KPU".to_string()),
				..query("The Martian")
			}),
			None
		);
	}

	#[tokio::test]
	async fn unknown_asin_surfaces_as_not_found() {
		let not_found = "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
		let server = MockServer::spawn(vec![not_found.to_string()]);
		let client = client(&server).without_retries();

		let error = client
			.fetch_media_metadata("B0MISSING1")
			.await
			.expect_err("a 404 should surface as an error");

		let MetadataProviderError::NotFound(path) = &error else {
			panic!("expected NotFound, got {error:?}");
		};
		assert_eq!(path, "/books/B0MISSING1");
	}

	#[tokio::test]
	async fn series_operations_are_unsupported() {
		let client = AudibleClient::new();

		assert!(
			matches!(
				client.search_series(&query("Wayfarers")).await,
				Err(MetadataProviderError::OperationNotSupported)
			),
			"Audnexus has no series search"
		);
		assert!(
			matches!(
				client.fetch_series_metadata("B08RLSPY4J").await,
				Err(MetadataProviderError::OperationNotSupported)
			),
			"Audnexus has no series record"
		);
	}

	#[tokio::test]
	async fn sparse_record_maps_without_panicking() {
		let body = serde_json::json!({ "asin": "B0SPARSE01" }).to_string();
		let server = MockServer::spawn(vec![render_ok(&body)]);
		let client = client(&server);

		let media = client
			.fetch_media_metadata("B0SPARSE01")
			.await
			.expect("a record with only an ASIN should still map");

		assert_eq!(media.external_id, "B0SPARSE01");
		assert_eq!(
			media.provider_url.as_deref(),
			Some("https://www.audible.com/pd/B0SPARSE01")
		);
		assert_eq!(media.title, None);
		assert_eq!(media.subtitle, None);
		assert_eq!(media.summary, None);
		assert_eq!(media.writers, None);
		assert_eq!(media.narrators, None);
		assert_eq!(media.genres, None);
		assert_eq!(media.tags, None);
		assert_eq!(media.publisher, None);
		assert_eq!(media.series_name, None);
		assert_eq!(media.number, None);
		assert_eq!(media.runtime_minutes, None);
		assert_eq!(media.cover_url, None);
		assert_eq!((media.year, media.month, media.day), (None, None, None));
		assert_eq!((media.isbn, media.isbn_13), (None, None));
	}

	#[tokio::test]
	async fn unnamed_author_is_expanded_through_the_author_endpoint() {
		let book = serde_json::json!({
			"asin": "B002V02KPU",
			"title": "Project Hail Mary",
			"authors": [{ "asin": "B002BLLAUS" }, { "name": "A Named Co-Author" }]
		})
		.to_string();
		let author =
			serde_json::json!({ "asin": "B002BLLAUS", "name": "Andy Weir" }).to_string();
		let server = MockServer::spawn(vec![render_ok(&book), render_ok(&author)]);
		let client = client(&server);

		let media = client
			.fetch_media_metadata("B002V02KPU")
			.await
			.expect("book lookup should succeed");

		assert_eq!(
			media.writers.as_deref(),
			Some(["Andy Weir".to_string(), "A Named Co-Author".to_string()].as_slice())
		);

		let requests = server.requests();
		assert_eq!(requests.len(), 2, "only the unnamed author is fetched");
		assert!(
			requests[1].starts_with("GET /authors/B002BLLAUS"),
			"{}",
			requests[1]
		);
	}

	#[test]
	fn release_dates_keep_their_calendar_day() {
		// An Audnexus instant and a catalog date must land on the same day,
		// whatever the host timezone.
		assert_eq!(
			parse_release_date(Some("2009-09-18T00:00:00.000Z")),
			(Some(2009), Some(9), Some(18))
		);
		assert_eq!(
			parse_release_date(Some("2018-06-14")),
			(Some(2018), Some(6), Some(14))
		);
		assert_eq!(parse_release_date(Some("2018")).0, Some(2018));
		assert_eq!(parse_release_date(Some("unknown")), (None, None, None));
		assert_eq!(parse_release_date(Some("   ")), (None, None, None));
		assert_eq!(parse_release_date(None), (None, None, None));
	}

	#[test]
	fn series_positions_take_the_leading_number() {
		assert_eq!(parse_series_position("1"), Some(1.0));
		assert_eq!(parse_series_position("Book 1"), Some(1.0));
		assert_eq!(parse_series_position("2, Dramatized Adaptation"), Some(2.0));
		assert_eq!(parse_series_position("1.5"), Some(1.5));
		assert_eq!(parse_series_position(".5"), Some(0.5));
		assert_eq!(parse_series_position("Part One"), None);
		assert_eq!(parse_series_position(""), None);
	}

	#[ignore = "Requires network access"]
	#[tokio::test]
	async fn live_asin_lookup() {
		let client = AudibleClient::new();
		let media = client
			.fetch_media_metadata(REACHABILITY_ASIN)
			.await
			.expect("live lookup should succeed");
		assert_eq!(media.external_id, REACHABILITY_ASIN);
		assert!(media.runtime_minutes.is_some_and(|minutes| minutes > 0));
	}

	/// The Audnexus probe above cannot see the catalog, and the catalog is the
	/// half whose field names come from an undocumented API.
	#[ignore = "Requires network access"]
	#[tokio::test]
	async fn live_catalog_search() {
		let client = AudibleClient::new();
		let outcome = client
			.search_media(&query("The Long Way to a Small, Angry Planet"))
			.await
			.expect("live catalog search should succeed");
		let media = outcome.candidates[0]
			.metadata
			.as_media()
			.expect("catalog hits are media");
		assert!(looks_like_asin(&media.external_id), "{media:?}");
		assert!(media.title.is_some(), "{media:?}");
		assert!(media.runtime_minutes.is_some(), "{media:?}");
	}
}
