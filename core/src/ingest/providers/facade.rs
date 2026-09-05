use std::{
	collections::{BTreeMap, HashMap},
	sync::Arc,
};

use async_trait::async_trait;
use futures::future::join_all;
use metadata_integrations::{
	ExternalMediaMetadata, ExternalMetadata, ExternalSeriesMetadata, MatchCandidate,
	MediaType, MetadataProvider, MetadataProviderError,
	SearchQuery as IntegrationSearchQuery,
};
use serde_json::{json, Value};
use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::ingest::{
	contract::{
		BookSnapshot, IngestMediaKind, IngestMetadataProvider, MetadataCandidate,
		MetadataField, ProviderCapability, ProviderError, ProviderIdentity, SearchHit,
		SearchQuery,
	},
	quality::filename::parse_filename,
};

/// Version of the adapter contract, independent of each remote provider's API
/// version.  It is persisted with candidates so normalization changes remain
/// auditable.
pub const INTEGRATION_PROVIDER_VERSION: &str = "metadata-integrations-1";

/// Adapt one of the existing `metadata_integrations` clients to the staged
/// ingest provider contract.  The wrapped client remains responsible for HTTP,
/// retries, rate limiting, searching, scoring, and detail fetches.
pub struct IntegrationProvider {
	client: Arc<dyn MetadataProvider + Send + Sync>,
	id: &'static str,
	name: &'static str,
	supported_media_kinds: Vec<IngestMediaKind>,
	capabilities: Vec<ProviderCapability>,
}

impl IntegrationProvider {
	pub fn new(client: Arc<dyn MetadataProvider + Send + Sync>) -> Self {
		let supported_media_kinds = client
			.supported_media_types()
			.into_iter()
			.flat_map(media_kinds)
			.collect();
		Self {
			id: client.id(),
			name: client.name(),
			client,
			supported_media_kinds,
			capabilities: vec![ProviderCapability::Identify, ProviderCapability::Lookup],
		}
	}

	pub fn client(&self) -> &Arc<dyn MetadataProvider + Send + Sync> {
		&self.client
	}

	fn map_error(&self, error: MetadataProviderError) -> ProviderError {
		if error.is_rate_limited() {
			ProviderError::RateLimited {
				provider_id: self.id.to_string(),
			}
		} else {
			ProviderError::Request {
				provider_id: self.id.to_string(),
				message: error.to_string(),
			}
		}
	}
}

#[async_trait]
impl IngestMetadataProvider for IntegrationProvider {
	fn id(&self) -> &'static str {
		self.id
	}

	fn name(&self) -> &'static str {
		self.name
	}

	fn version(&self) -> &'static str {
		INTEGRATION_PROVIDER_VERSION
	}

	fn supported_media_kinds(&self) -> &[IngestMediaKind] {
		&self.supported_media_kinds
	}

	fn capabilities(&self) -> &[ProviderCapability] {
		&self.capabilities
	}

	fn settings(&self) -> &[SettingDefinition] {
		// Credentials are deliberately not represented as ingest plugin settings.
		// The existing metadata_provider_configs row and ProviderClientCache own
		// their encrypted API token and auto-apply policy.
		&[]
	}

	async fn identify(
		&self,
		book: &BookSnapshot,
		_settings: &SettingValues,
	) -> Result<Vec<ProviderIdentity>, ProviderError> {
		let query = composed_query(book);
		let mut identities = Vec::new();
		let mut first_error = None;

		match self.client.search_media(&query).await {
			Ok(outcome) => identities.extend(
				outcome
					.candidates
					.into_iter()
					.map(|candidate| identity_from_match(candidate, "media")),
			),
			Err(error) => first_error = Some(self.map_error(error)),
		}
		match self.client.search_series(&query).await {
			Ok(outcome) => identities.extend(
				outcome
					.candidates
					.into_iter()
					.map(|candidate| identity_from_match(candidate, "series")),
			),
			Err(error) => {
				if first_error.is_none() {
					first_error = Some(self.map_error(error));
				}
			},
		}

		if identities.is_empty() {
			if let Some(error) = first_error {
				return Err(error);
			}
		}

		identities.sort_by(|left, right| {
			right
				.confidence
				.partial_cmp(&left.confidence)
				.unwrap_or(std::cmp::Ordering::Equal)
				.then_with(|| left.external_id.cmp(&right.external_id))
				.then_with(|| left.display.cmp(&right.display))
		});
		Ok(identities)
	}

	async fn lookup(
		&self,
		book: &BookSnapshot,
		identity: &ProviderIdentity,
		_settings: &SettingValues,
	) -> Result<Vec<MetadataCandidate>, ProviderError> {
		if identity.provider_id != self.id {
			return Err(ProviderError::Request {
				provider_id: self.id.to_string(),
				message: format!(
					"identity belongs to provider {}, not {}",
					identity.provider_id, self.id
				),
			});
		}

		let kind = identity
			.factors
			.get("result_kind")
			.and_then(Value::as_str)
			.unwrap_or("media");
		let metadata = if kind == "series" {
			ExternalMetadata::Series(
				self.client
					.fetch_series_metadata(&identity.external_id)
					.await
					.map_err(|error| self.map_error(error))?,
			)
		} else {
			ExternalMetadata::Media(
				self.client
					.fetch_media_metadata(&identity.external_id)
					.await
					.map_err(|error| self.map_error(error))?,
			)
		};
		Ok(vec![normalize_metadata(
			self.id,
			INTEGRATION_PROVIDER_VERSION,
			&book.source_sha256,
			identity.confidence,
			metadata,
			json!({
				"provider": self.id,
				"externalId": identity.external_id,
				"kind": kind,
				"factors": identity.factors,
			}),
		)])
	}

	async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, ProviderError> {
		let integration_query = IntegrationSearchQuery {
			title: query.text.clone(),
			limit: Some(u32::from(query.limit.max(1))),
			..IntegrationSearchQuery::default()
		};
		let outcome = self
			.client
			.search_media(&integration_query)
			.await
			.map_err(|error| self.map_error(error))?;
		let text = query.text.trim().to_string();
		let mut hits = Vec::with_capacity(outcome.candidates.len());
		for candidate in &outcome.candidates {
			if let Some(hit) = search_hit_from_match(self.id, &text, candidate) {
				hits.push(hit);
			}
		}
		hits.sort_by(score_order);
		Ok(hits)
	}
}

fn media_kinds(media_type: MediaType) -> Vec<IngestMediaKind> {
	match media_type {
		MediaType::Comic => vec![
			IngestMediaKind::ComicArchive,
			IngestMediaKind::ComicRarArchive,
		],
		MediaType::Manga
		| MediaType::LightNovel
		| MediaType::Book
		| MediaType::Manhwa
		| MediaType::WebNovel
		| MediaType::Webtoon => vec![IngestMediaKind::Epub, IngestMediaKind::Pdf],
	}
}

fn composed_query(book: &BookSnapshot) -> IntegrationSearchQuery {
	let parsed = parse_filename(&book.source_filename);
	let embedded = book.embedded_metadata.as_ref();
	let title = embedded
		.and_then(|metadata| metadata.title.clone())
		.filter(|value| !value.trim().is_empty())
		.unwrap_or_else(|| parsed.title.clone());
	let author = embedded
		.and_then(|metadata| metadata.writers.as_ref())
		.and_then(|writers| writers.first().cloned());
	let isbn = embedded.and_then(|metadata| metadata.identifier_isbn.clone());
	let year = embedded.and_then(|metadata| metadata.year).or(parsed.year);
	let number = embedded.and_then(|metadata| metadata.number.map(|value| value as f32));
	let mut provider_hints = HashMap::new();
	if let Some(series) = parsed.series {
		provider_hints.insert("filename_series".to_string(), series);
	}

	IntegrationSearchQuery {
		title,
		author,
		isbn,
		year,
		number: number.or_else(|| parsed.number.map(|value| value as f32)),
		limit: Some(10),
		provider_hints,
	}
}

fn identity_from_match(candidate: MatchCandidate, result_kind: &str) -> ProviderIdentity {
	let display = match &candidate.metadata {
		ExternalMetadata::Media(metadata) => metadata.title.clone(),
		ExternalMetadata::Series(metadata) => Some(metadata.title.clone()),
	}
	.unwrap_or_else(|| candidate.external_id.clone());
	ProviderIdentity {
		provider_id: candidate.provider,
		external_id: candidate.external_id,
		display,
		confidence: f64::from(candidate.confidence),
		factors: json!({
			"result_kind": result_kind,
			"confidence_factors": candidate.confidence_factors,
		}),
	}
}

/// Map one match candidate onto a media-level search hit.  Series results are
/// skipped: search feeds the editor's "use as candidate" flow, which resolves
/// via `fetch_media_metadata`.  The score comes from the provider's scorer;
/// unscored candidates fall back to title similarity.
fn search_hit_from_match(
	provider_id: &str,
	query_text: &str,
	candidate: &MatchCandidate,
) -> Option<SearchHit> {
	let metadata = candidate.metadata.as_media()?;
	let title = metadata
		.title
		.clone()
		.or_else(|| metadata.series_name.clone())
		.filter(|title| !title.trim().is_empty())?;
	let score = if candidate.confidence > 0.0 {
		candidate.confidence.clamp(0.0, 1.0)
	} else {
		metadata_integrations::title_similarity(query_text, &title) as f32
	};
	Some(SearchHit {
		provider_id: provider_id.to_string(),
		external_id: candidate.external_id.clone(),
		title,
		year: metadata.year,
		cover_url: metadata.cover_url.clone(),
		summary: metadata.summary.clone(),
		score,
	})
}

/// Deterministic hit ordering: score descending, then provider id, then
/// external id.
fn score_order(left: &SearchHit, right: &SearchHit) -> std::cmp::Ordering {
	right
		.score
		.partial_cmp(&left.score)
		.unwrap_or(std::cmp::Ordering::Equal)
		.then_with(|| left.provider_id.cmp(&right.provider_id))
		.then_with(|| left.external_id.cmp(&right.external_id))
}

/// Fan a free-text search out over several providers.  Providers that are not
/// enabled, lack the `Search` capability, or do not support the requested
/// media kind are skipped.  Per-provider rate limiting stays inside the
/// wrapped clients, exactly as it does for `identify`.  A provider failure is
/// logged and isolated; hits are merged and sorted by
/// [`score_order`].
pub async fn search_all(
	providers: &[Arc<dyn IngestMetadataProvider>],
	query: &SearchQuery,
	enabled: &[String],
) -> Vec<SearchHit> {
	let text = query.text.trim().to_string();
	if text.is_empty() {
		return Vec::new();
	}
	let participants = providers
		.iter()
		.filter(|provider| {
			enabled.iter().any(|id| id == provider.id())
				&& provider
					.capabilities()
					.contains(&ProviderCapability::Search)
				&& query
					.media_kind
					.is_none_or(|kind| provider.supported_media_kinds().contains(&kind))
		})
		.map(|provider| provider.search(query))
		.collect::<Vec<_>>();

	let mut hits = Vec::new();
	for result in join_all(participants).await {
		match result {
			Ok(mut found) => hits.append(&mut found),
			Err(error) => tracing::warn!(?error, "Provider search failed"),
		}
	}

	hits.sort_by(score_order);
	hits
}

pub(crate) fn normalize_metadata(
	provider_id: &str,
	provider_version: &str,
	source_sha256: &str,
	confidence: f64,
	metadata: ExternalMetadata,
	provenance: Value,
) -> MetadataCandidate {
	let (external_id, fields) = match metadata {
		ExternalMetadata::Media(metadata) => {
			(metadata.external_id.clone(), media_fields(&metadata))
		},
		ExternalMetadata::Series(metadata) => {
			(metadata.external_id.clone(), series_fields(&metadata))
		},
	};
	let field_confidence = fields
		.keys()
		.map(|field| (*field, confidence))
		.collect::<BTreeMap<_, _>>();
	MetadataCandidate {
		provider_id: provider_id.to_string(),
		provider_version: provider_version.to_string(),
		external_id: Some(external_id),
		source_sha256: source_sha256.to_string(),
		confidence,
		fields,
		field_confidence,
		provenance,
	}
}

fn media_fields(metadata: &ExternalMediaMetadata) -> BTreeMap<MetadataField, Value> {
	let mut fields = BTreeMap::new();
	insert_string(&mut fields, MetadataField::Title, metadata.title.clone());
	insert_string(
		&mut fields,
		MetadataField::Summary,
		metadata.summary.clone(),
	);
	insert_string(
		&mut fields,
		MetadataField::Series,
		metadata.series_name.clone(),
	);
	insert_number(
		&mut fields,
		MetadataField::SeriesIndex,
		metadata.number.map(f64::from),
	);
	insert_strings(
		&mut fields,
		MetadataField::Authors,
		metadata.writers.clone(),
	);
	insert_strings(&mut fields, MetadataField::Genres, metadata.genres.clone());
	insert_strings(&mut fields, MetadataField::Tags, metadata.tags.clone());
	insert_string(
		&mut fields,
		MetadataField::Isbn,
		metadata.isbn.clone().or_else(|| metadata.isbn_13.clone()),
	);
	insert_date(&mut fields, metadata.year, metadata.month, metadata.day);
	insert_number(
		&mut fields,
		MetadataField::PageCount,
		metadata.page_count.map(f64::from),
	);
	insert_string(
		&mut fields,
		MetadataField::CoverUrl,
		metadata.cover_url.clone(),
	);
	insert_identifiers(
		&mut fields,
		&metadata.provider,
		&metadata.external_id,
		metadata.provider_url.as_deref(),
	);
	fields
}
fn series_fields(metadata: &ExternalSeriesMetadata) -> BTreeMap<MetadataField, Value> {
	let mut fields = BTreeMap::new();
	insert_string(
		&mut fields,
		MetadataField::Title,
		Some(metadata.title.clone()),
	);
	insert_string(
		&mut fields,
		MetadataField::Summary,
		metadata.summary.clone(),
	);
	insert_strings(
		&mut fields,
		MetadataField::Authors,
		metadata.authors.clone(),
	);
	if let Some(age_rating) = metadata.age_rating.as_ref() {
		if let Ok(age_rating) = age_rating.parse::<i64>() {
			fields.insert(MetadataField::AgeRating, json!(age_rating));
		}
	}
	insert_date(&mut fields, metadata.year, None, None);
	insert_string(
		&mut fields,
		MetadataField::CoverUrl,
		metadata.cover_url.clone(),
	);
	insert_identifiers(&mut fields, &metadata.provider, &metadata.external_id, None);
	fields
}

fn insert_string(
	fields: &mut BTreeMap<MetadataField, Value>,
	field: MetadataField,
	value: Option<String>,
) {
	if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
		fields.insert(field, Value::String(value));
	}
}

fn insert_strings(
	fields: &mut BTreeMap<MetadataField, Value>,
	field: MetadataField,
	value: Option<Vec<String>>,
) {
	if let Some(values) = value {
		let values: Vec<_> = values
			.into_iter()
			.map(|value| value.trim().to_string())
			.filter(|value| !value.is_empty())
			.collect();
		if !values.is_empty() {
			fields.insert(field, json!(values));
		}
	}
}

fn insert_number(
	fields: &mut BTreeMap<MetadataField, Value>,
	field: MetadataField,
	value: Option<f64>,
) {
	if let Some(value) = value.filter(|value| value.is_finite()) {
		fields.insert(field, json!(value));
	}
}

fn insert_date(
	fields: &mut BTreeMap<MetadataField, Value>,
	year: Option<i32>,
	month: Option<i32>,
	day: Option<i32>,
) {
	let Some(year) = year else { return };
	let value = match (month, day) {
		(Some(month), Some(day)) => format!("{year:04}-{month:02}-{day:02}"),
		(Some(month), None) => format!("{year:04}-{month:02}"),
		(None, _) => format!("{year:04}"),
	};
	fields.insert(MetadataField::PublishedDate, Value::String(value));
}

fn insert_identifiers(
	fields: &mut BTreeMap<MetadataField, Value>,
	provider: &str,
	external_id: &str,
	provider_url: Option<&str>,
) {
	let mut identifiers = serde_json::Map::new();
	identifiers.insert("provider".to_string(), Value::String(provider.to_string()));
	identifiers.insert(
		"externalId".to_string(),
		Value::String(external_id.to_string()),
	);
	if let Some(url) = provider_url {
		identifiers.insert("url".to_string(), Value::String(url.to_string()));
	}
	fields.insert(MetadataField::Identifiers, Value::Object(identifiers));
}

#[cfg(test)]
mod tests {
	use super::*;
	use metadata_integrations::{
		ConfidenceFactor, ExternalMediaMetadata, ExternalMetadata, MatchCandidate,
		SearchOutcome,
	};

	#[test]
	fn maps_match_candidate_with_confidence_factors() {
		let identity = identity_from_match(
			MatchCandidate {
				provider: "hardcover".to_string(),
				external_id: "42".to_string(),
				metadata: ExternalMetadata::Media(ExternalMediaMetadata {
					title: Some("A book".to_string()),
					external_id: "42".to_string(),
					..Default::default()
				}),
				confidence: 0.83,
				confidence_factors: vec![ConfidenceFactor {
					factor: "title_exact".to_string(),
					weight: 0.9,
					matched: true,
				}],
			},
			"media",
		);
		assert_eq!(identity.provider_id, "hardcover");
		assert_eq!(identity.external_id, "42");
		assert_eq!(identity.display, "A book");
		assert!((identity.confidence - 0.83).abs() < 1e-6);
		assert_eq!(identity.factors["result_kind"], "media");
		assert_eq!(
			identity.factors["confidence_factors"][0]["factor"],
			"title_exact"
		);
	}

	#[test]
	fn normalizes_media_fields_and_dates() {
		let candidate = normalize_metadata(
			"comic_vine",
			INTEGRATION_PROVIDER_VERSION,
			"digest",
			0.9,
			ExternalMetadata::Media(ExternalMediaMetadata {
				provider: "comic_vine".to_string(),
				external_id: "17".to_string(),
				title: Some("Issue".to_string()),
				number: Some(1.5),
				writers: Some(vec!["Writer".to_string()]),
				year: Some(2024),
				month: Some(3),
				day: Some(4),
				..Default::default()
			}),
			Value::Null,
		);
		assert_eq!(candidate.source_sha256, "digest");
		assert_eq!(candidate.fields[&MetadataField::Title], "Issue");
		assert_eq!(candidate.fields[&MetadataField::SeriesIndex], 1.5);
		assert_eq!(
			candidate.fields[&MetadataField::PublishedDate],
			"2024-03-04"
		);
		assert_eq!(candidate.fields[&MetadataField::Authors], json!(["Writer"]));
	}

	// -- search -------------------------------------------------------------

	struct StubProvider {
		id: &'static str,
		hits: Vec<SearchHit>,
		fails: bool,
		with_search: bool,
	}

	impl StubProvider {
		fn new(
			id: &'static str,
			hits: Vec<SearchHit>,
		) -> Arc<dyn IngestMetadataProvider> {
			Arc::new(Self {
				id,
				hits,
				fails: false,
				with_search: true,
			})
		}

		fn failing(id: &'static str) -> Arc<dyn IngestMetadataProvider> {
			Arc::new(Self {
				id,
				hits: Vec::new(),
				fails: true,
				with_search: true,
			})
		}

		fn without_search(id: &'static str) -> Arc<dyn IngestMetadataProvider> {
			Arc::new(Self {
				id,
				hits: vec![hit(id, "1", 1.0)],
				fails: false,
				with_search: false,
			})
		}
	}

	#[async_trait]
	impl IngestMetadataProvider for StubProvider {
		fn id(&self) -> &'static str {
			self.id
		}

		fn name(&self) -> &'static str {
			self.id
		}

		fn version(&self) -> &'static str {
			"stub"
		}

		fn supported_media_kinds(&self) -> &[IngestMediaKind] {
			static KINDS: [IngestMediaKind; 2] =
				[IngestMediaKind::ComicArchive, IngestMediaKind::Epub];
			&KINDS
		}

		fn capabilities(&self) -> &[ProviderCapability] {
			if self.with_search {
				&[ProviderCapability::Search]
			} else {
				&[]
			}
		}

		fn settings(&self) -> &[SettingDefinition] {
			&[]
		}

		async fn identify(
			&self,
			_book: &BookSnapshot,
			_settings: &SettingValues,
		) -> Result<Vec<ProviderIdentity>, ProviderError> {
			Ok(Vec::new())
		}

		async fn lookup(
			&self,
			_book: &BookSnapshot,
			_identity: &ProviderIdentity,
			_settings: &SettingValues,
		) -> Result<Vec<MetadataCandidate>, ProviderError> {
			Ok(Vec::new())
		}

		async fn search(
			&self,
			_query: &SearchQuery,
		) -> Result<Vec<SearchHit>, ProviderError> {
			if self.fails {
				Err(ProviderError::RateLimited {
					provider_id: self.id.to_string(),
				})
			} else {
				Ok(self.hits.clone())
			}
		}
	}

	fn hit(provider: &str, external: &str, score: f32) -> SearchHit {
		SearchHit {
			provider_id: provider.to_string(),
			external_id: external.to_string(),
			title: "Title".to_string(),
			year: None,
			cover_url: None,
			summary: None,
			score,
		}
	}

	fn search_query(media_kind: Option<IngestMediaKind>) -> SearchQuery {
		SearchQuery {
			text: "batman".to_string(),
			media_kind,
			limit: 10,
		}
	}

	#[tokio::test]
	async fn search_all_filters_disabled_and_sorts_by_score() {
		let providers: Vec<Arc<dyn IngestMetadataProvider>> = vec![
			StubProvider::new("a", vec![hit("a", "1", 0.4), hit("a", "2", 0.9)]),
			StubProvider::new("b", vec![hit("b", "1", 0.7)]),
			StubProvider::new("c", vec![hit("c", "1", 1.0)]),
		];
		let enabled = vec!["a".to_string(), "b".to_string()];
		let hits = search_all(&providers, &search_query(None), &enabled).await;
		let ids: Vec<String> = hits
			.iter()
			.map(|hit| format!("{}:{}", hit.provider_id, hit.external_id))
			.collect();
		assert_eq!(ids, vec!["a:2", "b:1", "a:1"]);
	}

	#[tokio::test]
	async fn search_all_isolates_provider_errors_and_skips_capability() {
		let providers: Vec<Arc<dyn IngestMetadataProvider>> = vec![
			StubProvider::new("ok", vec![hit("ok", "1", 0.5)]),
			StubProvider::failing("rate-limited"),
			StubProvider::without_search("no-search"),
		];
		let enabled = vec![
			"ok".to_string(),
			"rate-limited".to_string(),
			"no-search".to_string(),
		];
		let hits = search_all(&providers, &search_query(None), &enabled).await;
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].provider_id, "ok");
	}

	#[tokio::test]
	async fn search_all_filters_by_requested_media_kind() {
		let providers: Vec<Arc<dyn IngestMetadataProvider>> =
			vec![StubProvider::new("a", vec![hit("a", "1", 0.5)])];
		let enabled = vec!["a".to_string()];
		let hits = search_all(
			&providers,
			&search_query(Some(IngestMediaKind::Epub)),
			&enabled,
		)
		.await;
		assert_eq!(hits.len(), 1);
		// ComicArchive is in the stub's supported kinds, but Pdf is not.
		let hits = search_all(
			&providers,
			&search_query(Some(IngestMediaKind::Pdf)),
			&enabled,
		)
		.await;
		assert!(hits.is_empty());
	}

	#[tokio::test]
	async fn search_all_ignores_blank_queries() {
		let providers: Vec<Arc<dyn IngestMetadataProvider>> =
			vec![StubProvider::new("a", vec![hit("a", "1", 0.5)])];
		let enabled = vec!["a".to_string()];
		let hits = search_all(
			&providers,
			&SearchQuery {
				text: "   ".to_string(),
				media_kind: None,
				limit: 10,
			},
			&enabled,
		)
		.await;
		assert!(hits.is_empty());
	}

	// -- IntegrationProvider::search ---------------------------------------

	struct FakeIntegrationClient {
		/// Last observed search limit, encoded as limit+1 (0 = none seen).
		seen_limit: std::sync::atomic::AtomicU32,
	}

	#[async_trait::async_trait]
	impl MetadataProvider for FakeIntegrationClient {
		fn id(&self) -> &'static str {
			"fake"
		}

		fn name(&self) -> &'static str {
			"Fake"
		}

		fn supported_media_types(&self) -> Vec<MediaType> {
			vec![MediaType::Comic]
		}

		async fn search_series(
			&self,
			_query: &IntegrationSearchQuery,
		) -> Result<SearchOutcome, MetadataProviderError> {
			Ok(SearchOutcome::default())
		}

		async fn search_media(
			&self,
			query: &IntegrationSearchQuery,
		) -> Result<SearchOutcome, MetadataProviderError> {
			self.seen_limit.store(
				query.limit.unwrap_or(0) + 1,
				std::sync::atomic::Ordering::Relaxed,
			);
			Ok(SearchOutcome {
				candidates: vec![
					MatchCandidate {
						provider: "fake".to_string(),
						external_id: "9".to_string(),
						metadata: ExternalMetadata::Media(ExternalMediaMetadata {
							external_id: "9".to_string(),
							title: Some("Batman".to_string()),
							year: Some(1940),
							cover_url: Some("https://example.test/cover.png".to_string()),
							summary: Some("The Dark Knight".to_string()),
							..Default::default()
						}),
						// Unscored: exercises the title-similarity fallback.
						confidence: 0.0,
						confidence_factors: Vec::new(),
					},
					MatchCandidate {
						provider: "fake".to_string(),
						external_id: "8".to_string(),
						metadata: ExternalMetadata::Media(ExternalMediaMetadata {
							external_id: "8".to_string(),
							title: Some("Detective Comics".to_string()),
							..Default::default()
						}),
						confidence: 0.55,
						confidence_factors: Vec::new(),
					},
				],
				requested: 2,
			})
		}

		async fn fetch_series_metadata(
			&self,
			_external_id: &str,
		) -> Result<ExternalSeriesMetadata, MetadataProviderError> {
			Err(MetadataProviderError::Other("unused".to_string()))
		}

		async fn fetch_media_metadata(
			&self,
			_external_id: &str,
		) -> Result<ExternalMediaMetadata, MetadataProviderError> {
			Err(MetadataProviderError::Other("unused".to_string()))
		}

		async fn verify_credentials(
			&self,
		) -> Result<
			metadata_integrations::ProviderCredentialVerification,
			MetadataProviderError,
		> {
			Ok(metadata_integrations::ProviderCredentialVerification {
				response_status: 200,
				is_valid: true,
				error: None,
			})
		}
	}

	#[tokio::test]
	async fn integration_search_maps_hits_and_falls_back_to_title_similarity() {
		let client = Arc::new(FakeIntegrationClient {
			seen_limit: std::sync::atomic::AtomicU32::new(0),
		});
		let provider = IntegrationProvider::new(client.clone());
		let hits = provider
			.search(&SearchQuery {
				text: "Batman".to_string(),
				media_kind: None,
				limit: 7,
			})
			.await
			.unwrap();

		assert_eq!(
			client.seen_limit.load(std::sync::atomic::Ordering::Relaxed),
			8
		);
		assert_eq!(hits.len(), 2);
		assert_eq!(hits[0].provider_id, "fake");
		assert_eq!(hits[0].external_id, "9");
		assert_eq!(hits[0].title, "Batman");
		assert_eq!(hits[0].year, Some(1940));
		assert_eq!(
			hits[0].cover_url.as_deref(),
			Some("https://example.test/cover.png")
		);
		assert!(
			hits[0].score > 0.9,
			"exact title should score high: {}",
			hits[0].score
		);
		assert!((hits[1].score - 0.55).abs() < 1e-6);
		assert_eq!(hits[1].external_id, "8");
	}
}
