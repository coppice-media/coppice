use std::{
	collections::{BTreeMap, HashMap},
	sync::Arc,
};

use async_trait::async_trait;
use metadata_integrations::{
	ExternalMediaMetadata, ExternalMetadata, ExternalSeriesMetadata, MatchCandidate,
	MediaType, MetadataProvider, MetadataProviderError, SearchQuery,
};
use serde_json::{json, Value};

use crate::ingest::{
	contract::{
		BookSnapshot, IngestMediaKind, IngestMetadataProvider, MetadataCandidate,
		MetadataField, ProviderCapability, ProviderError, ProviderIdentity,
		SettingDefinition, SettingValues,
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

fn composed_query(book: &BookSnapshot) -> SearchQuery {
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

	SearchQuery {
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
}
