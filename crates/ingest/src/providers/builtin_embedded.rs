use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::{json, Value};

use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_media::media::ProcessedMediaMetadata;

use crate::{
	contract::{
		BookSnapshot, IngestMediaKind, IngestMetadataProvider, MetadataCandidate,
		MetadataField, ProviderCapability, ProviderError, ProviderIdentity,
	},
	quality::filename::parse_filename,
};

pub const EMBEDDED_PROVIDER_ID: &str = "builtin:embedded";
pub const EMBEDDED_PROVIDER_VERSION: &str = "embedded-1";

/// A deterministic, network-free provider backed by the file's embedded
/// metadata and the shared filename parser.  It is enabled by default so a
/// staged item remains useful even when no remote provider credentials exist.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmbeddedProvider;

impl EmbeddedProvider {
	pub fn new() -> Self {
		Self
	}

	fn supported() -> &'static [IngestMediaKind] {
		static KINDS: [IngestMediaKind; 5] = [
			IngestMediaKind::ComicArchive,
			IngestMediaKind::ComicRarArchive,
			IngestMediaKind::Epub,
			IngestMediaKind::Pdf,
			IngestMediaKind::Unknown,
		];
		&KINDS
	}

	fn capabilities() -> &'static [ProviderCapability] {
		static CAPABILITIES: [ProviderCapability; 2] =
			[ProviderCapability::Identify, ProviderCapability::Lookup];
		&CAPABILITIES
	}
}

#[async_trait]
impl IngestMetadataProvider for EmbeddedProvider {
	fn id(&self) -> &'static str {
		EMBEDDED_PROVIDER_ID
	}

	fn name(&self) -> &'static str {
		"Embedded metadata"
	}

	fn version(&self) -> &'static str {
		EMBEDDED_PROVIDER_VERSION
	}

	fn supported_media_kinds(&self) -> &[IngestMediaKind] {
		Self::supported()
	}

	fn capabilities(&self) -> &[ProviderCapability] {
		Self::capabilities()
	}

	fn settings(&self) -> &[SettingDefinition] {
		&[]
	}

	async fn identify(
		&self,
		book: &BookSnapshot,
		_settings: &SettingValues,
	) -> Result<Vec<ProviderIdentity>, ProviderError> {
		let parsed = parse_filename(&book.source_filename);
		let display = book
			.embedded_metadata
			.as_ref()
			.and_then(|metadata| metadata.title.clone())
			.filter(|title| !title.trim().is_empty())
			.unwrap_or(parsed.title);
		Ok(vec![ProviderIdentity {
			provider_id: self.id().to_string(),
			// The source digest is the stable identity for this local provider.
			external_id: book.source_sha256.clone(),
			display,
			confidence: 1.0,
			factors: json!({"source": "embedded-and-filename"}),
		}])
	}

	async fn lookup(
		&self,
		book: &BookSnapshot,
		identity: &ProviderIdentity,
		_settings: &SettingValues,
	) -> Result<Vec<MetadataCandidate>, ProviderError> {
		if identity.provider_id != self.id() || identity.external_id != book.source_sha256
		{
			return Err(ProviderError::Request {
				provider_id: self.id().to_string(),
				message: "embedded identity does not match source digest".to_string(),
			});
		}
		let (fields, field_confidence) = embedded_fields(book);
		Ok(vec![MetadataCandidate {
			provider_id: self.id().to_string(),
			provider_version: self.version().to_string(),
			external_id: Some(book.source_sha256.clone()),
			source_sha256: book.source_sha256.clone(),
			confidence: 1.0,
			fields,
			field_confidence,
			provenance: json!({
				"source": "embedded-and-filename",
				"filename": book.source_filename,
			}),
		}])
	}
}

fn embedded_fields(
	book: &BookSnapshot,
) -> (BTreeMap<MetadataField, Value>, BTreeMap<MetadataField, f64>) {
	let mut fields = BTreeMap::new();
	let mut confidence = BTreeMap::new();
	if let Some(metadata) = book.embedded_metadata.as_ref() {
		add_embedded_metadata(&mut fields, &mut confidence, metadata);
	}

	let parsed = parse_filename(&book.source_filename);
	insert_filename_string(
		&mut fields,
		&mut confidence,
		MetadataField::Title,
		Some(parsed.title),
	);
	insert_filename_string(
		&mut fields,
		&mut confidence,
		MetadataField::Series,
		parsed.series,
	);
	if let Some(number) = parsed.number.filter(|value| value.is_finite()) {
		fields.entry(MetadataField::SeriesIndex).or_insert_with(|| {
			confidence.insert(MetadataField::SeriesIndex, 0.6);
			json!(number)
		});
	}
	if let Some(year) = parsed.year {
		fields
			.entry(MetadataField::PublishedDate)
			.or_insert_with(|| {
				confidence.insert(MetadataField::PublishedDate, 0.6);
				Value::String(format!("{year:04}"))
			});
	}

	fields.insert(
		MetadataField::Identifiers,
		json!({"provider": EMBEDDED_PROVIDER_ID, "sourceSha256": book.source_sha256}),
	);
	confidence.insert(MetadataField::Identifiers, 1.0);
	(fields, confidence)
}

fn add_embedded_metadata(
	fields: &mut BTreeMap<MetadataField, Value>,
	confidence: &mut BTreeMap<MetadataField, f64>,
	metadata: &ProcessedMediaMetadata,
) {
	insert_embedded_string(
		fields,
		confidence,
		MetadataField::Title,
		metadata.title.clone(),
	);
	insert_embedded_string(
		fields,
		confidence,
		MetadataField::SortTitle,
		metadata.title_sort.clone(),
	);
	insert_embedded_string(
		fields,
		confidence,
		MetadataField::Series,
		metadata.series.clone(),
	);
	if let Some(number) = metadata.number.filter(|value| value.is_finite()) {
		fields.insert(MetadataField::SeriesIndex, json!(number));
		confidence.insert(MetadataField::SeriesIndex, 1.0);
	}
	insert_embedded_strings(
		fields,
		confidence,
		MetadataField::Authors,
		metadata.writers.clone(),
	);
	insert_embedded_string(
		fields,
		confidence,
		MetadataField::Publisher,
		metadata.publisher.clone(),
	);
	insert_embedded_string(
		fields,
		confidence,
		MetadataField::Language,
		metadata.language.clone(),
	);
	insert_embedded_string(
		fields,
		confidence,
		MetadataField::Summary,
		metadata.summary.clone(),
	);
	insert_embedded_strings(
		fields,
		confidence,
		MetadataField::Tags,
		metadata.tags.clone(),
	);
	insert_embedded_strings(
		fields,
		confidence,
		MetadataField::Genres,
		metadata.genres.clone(),
	);
	insert_embedded_string(
		fields,
		confidence,
		MetadataField::Isbn,
		metadata.identifier_isbn.clone(),
	);
	if let Some(age_rating) = metadata.age_rating {
		fields.insert(MetadataField::AgeRating, json!(age_rating));
		confidence.insert(MetadataField::AgeRating, 1.0);
	}
	if let Some(page_count) = metadata.page_count {
		fields.insert(MetadataField::PageCount, json!(page_count));
		confidence.insert(MetadataField::PageCount, 1.0);
	}
	if metadata.year.is_some() {
		let value = match (metadata.month, metadata.day) {
			(Some(month), Some(day)) => format!(
				"{:04}-{:02}-{:02}",
				metadata.year.unwrap_or_default(),
				month,
				day
			),
			(Some(month), None) => {
				format!("{:04}-{:02}", metadata.year.unwrap_or_default(), month)
			},
			(None, _) => format!("{:04}", metadata.year.unwrap_or_default()),
		};
		fields.insert(MetadataField::PublishedDate, Value::String(value));
		confidence.insert(MetadataField::PublishedDate, 1.0);
	}
}

fn insert_embedded_string(
	fields: &mut BTreeMap<MetadataField, Value>,
	confidence: &mut BTreeMap<MetadataField, f64>,
	field: MetadataField,
	value: Option<String>,
) {
	if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
		fields.insert(field, Value::String(value));
		confidence.insert(field, 1.0);
	}
}

fn insert_embedded_strings(
	fields: &mut BTreeMap<MetadataField, Value>,
	confidence: &mut BTreeMap<MetadataField, f64>,
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
			confidence.insert(field, 1.0);
		}
	}
}

fn insert_filename_string(
	fields: &mut BTreeMap<MetadataField, Value>,
	confidence: &mut BTreeMap<MetadataField, f64>,
	field: MetadataField,
	value: Option<String>,
) {
	if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
		fields.entry(field).or_insert_with(|| {
			confidence.insert(field, 0.6);
			Value::String(value)
		});
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;
	use crate::contract::IngestMediaKind;
	use stump_media::media::process_metadata;

	fn snapshot(
		path: &str,
		filename: &str,
		metadata: Option<ProcessedMediaMetadata>,
	) -> BookSnapshot {
		BookSnapshot {
			drop_item_id: "drop".to_string(),
			library_id: "library".to_string(),
			staged_path: PathBuf::from(path),
			source_sha256: "digest".to_string(),
			byte_size: 1,
			source_filename: filename.to_string(),
			relative_path: String::new(),
			media_kind: IngestMediaKind::Epub,
			embedded_metadata: metadata,
			pages: vec![],
			analysis: None,
		}
	}

	#[tokio::test]
	async fn embedded_epub_yields_metadata_candidate() {
		// The embedded provider only reshapes what `stump_media` extracted, so
		// these two tests read that crate's own metadata fixtures instead of
		// keeping a second copy of the same bytes here.
		let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("../media/integration-tests/data/book.epub");
		let metadata = process_metadata(&path).expect("process EPUB metadata");
		let provider = EmbeddedProvider::new();
		let book = snapshot(path.to_str().unwrap_or_default(), "book.epub", metadata);
		let identity = provider
			.identify(&book, &BTreeMap::new())
			.await
			.unwrap()
			.remove(0);
		let candidate = provider
			.lookup(&book, &identity, &BTreeMap::new())
			.await
			.unwrap()
			.remove(0);
		assert_eq!(candidate.provider_id, EMBEDDED_PROVIDER_ID);
		assert_eq!(candidate.source_sha256, "digest");
		assert!(candidate.fields.contains_key(&MetadataField::Title));
		assert_eq!(candidate.field_confidence[&MetadataField::Title], 1.0);
	}

	#[tokio::test]
	async fn embedded_filename_values_are_lower_confidence() {
		let provider = EmbeddedProvider::new();
		let book = snapshot("unused", "Saga - Chapter One 003 (2024).epub", None);
		let identity = provider
			.identify(&book, &BTreeMap::new())
			.await
			.unwrap()
			.remove(0);
		let candidate = provider
			.lookup(&book, &identity, &BTreeMap::new())
			.await
			.unwrap()
			.remove(0);
		assert_eq!(candidate.field_confidence[&MetadataField::Title], 0.6);
		assert_eq!(candidate.field_confidence[&MetadataField::Series], 0.6);
	}
	#[tokio::test]
	async fn embedded_comicinfo_archive_yields_canonical_fields() {
		let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("../media/integration-tests/data/book.zip");
		let metadata = process_metadata(&path).expect("process ComicInfo metadata");
		let provider = EmbeddedProvider::new();
		let book = BookSnapshot {
			drop_item_id: "drop".to_string(),
			library_id: "library".to_string(),
			staged_path: path,
			source_sha256: "comic-digest".to_string(),
			byte_size: 1,
			source_filename: "Miles Morales 004.cbz".to_string(),
			relative_path: String::new(),
			media_kind: IngestMediaKind::ComicArchive,
			embedded_metadata: metadata,
			pages: vec![],
			analysis: None,
		};
		let identity = provider
			.identify(&book, &BTreeMap::new())
			.await
			.unwrap()
			.remove(0);
		let candidate = provider
			.lookup(&book, &identity, &BTreeMap::new())
			.await
			.unwrap()
			.remove(0);
		assert_eq!(
			candidate.fields[&MetadataField::Series],
			"Miles Morales: Spider-Man"
		);
		assert_eq!(candidate.fields[&MetadataField::SeriesIndex], 4.0);
		assert_eq!(candidate.fields[&MetadataField::Publisher], "Marvel");
		assert_eq!(
			candidate.fields[&MetadataField::Genres],
			json!(["Superhero"])
		);
		assert_eq!(candidate.fields[&MetadataField::PageCount], 21);
		assert_eq!(candidate.field_confidence[&MetadataField::Publisher], 1.0);
	}
}
