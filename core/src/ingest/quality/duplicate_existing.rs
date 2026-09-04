use std::sync::Arc;

use metadata_integrations::{
	ExternalMediaMetadata, ExternalMetadata, MatchCandidate, MatchScorer, SearchQuery,
};
use models::entity::{media, media_metadata};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::ingest::contract::{
	BookSnapshot, QualityCheck, QualityCheckError, QualityStatus, SettingDefinition,
	SettingValues,
};

use super::{
	disabled_outcome, enabled_setting, filename::parse_filename, outcome, QUALITY_VERSION,
};

/// Finds exact source/hash duplicates and likely title duplicates among visible media.
pub struct DuplicateExistingCheck {
	conn: Arc<DatabaseConnection>,
}

impl DuplicateExistingCheck {
	pub fn new(conn: Arc<DatabaseConnection>) -> Self {
		Self { conn }
	}
}

#[async_trait::async_trait]
impl QualityCheck for DuplicateExistingCheck {
	fn id(&self) -> &'static str {
		"duplicate_existing"
	}

	fn name(&self) -> &'static str {
		"Duplicate existing media"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		10
	}

	fn settings(&self) -> &[SettingDefinition] {
		super::enabled_settings()
	}

	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<crate::ingest::contract::QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}

		let rows = media::Entity::find()
			.filter(media::Column::DeletedAt.is_null())
			.find_also_related(media_metadata::Entity)
			.all(self.conn.as_ref())
			.await
			.map_err(|error| QualityCheckError::Internal {
				check_id: self.id().to_string(),
				message: error.to_string(),
			})?;

		let mut exact_hash_ids = rows
			.iter()
			.filter(|(media, _)| {
				!book.source_sha256.is_empty()
					&& media.hash.as_deref() == Some(book.source_sha256.as_str())
			})
			.map(|(media, _)| media.id.clone())
			.collect::<Vec<_>>();
		exact_hash_ids.sort();
		if !exact_hash_ids.is_empty() {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				serde_json::json!({
					"exact_hash_match": true,
					"matching_media_ids": exact_hash_ids,
					"matches": [],
				}),
			));
		}

		let parsed = parse_filename(&book.source_filename);
		let query_title = book
			.embedded_metadata
			.as_ref()
			.and_then(|metadata| metadata.title.clone())
			.filter(|title| !title.trim().is_empty())
			.unwrap_or_else(|| parsed.title.clone());
		let query = SearchQuery {
			title: query_title.clone(),
			author: book
				.embedded_metadata
				.as_ref()
				.and_then(|metadata| metadata.writers.as_ref())
				.and_then(|writers| writers.first().cloned()),
			isbn: book
				.embedded_metadata
				.as_ref()
				.and_then(|metadata| metadata.identifier_isbn.clone()),
			year: book
				.embedded_metadata
				.as_ref()
				.and_then(|metadata| metadata.year),
			number: book
				.embedded_metadata
				.as_ref()
				.and_then(|metadata| metadata.number)
				.map(|number| number as f32),
			..Default::default()
		};
		let scorer = MatchScorer;
		let mut matches = rows
			.into_iter()
			.map(|(media, metadata)| {
				let title = metadata
					.as_ref()
					.and_then(|metadata| metadata.title.clone())
					.filter(|title| !title.trim().is_empty())
					.unwrap_or(media.name.clone());
				let writers = metadata
					.as_ref()
					.and_then(|metadata| metadata.writers.clone())
					.map(|writers| {
						writers
							.split(",")
							.map(str::trim)
							.filter(|writer| !writer.is_empty())
							.map(ToOwned::to_owned)
							.collect::<Vec<_>>()
					})
					.unwrap_or_default();
				let mut candidate = MatchCandidate {
					provider: "stump-existing".to_string(),
					external_id: media.id.clone(),
					metadata: ExternalMetadata::Media(ExternalMediaMetadata {
						title: Some(title),
						writers: Some(writers),
						..Default::default()
					}),
					confidence: 0.0,
					confidence_factors: Vec::new(),
				};
				scorer.score_candidate(&query, &mut candidate);
				(media.id, f64::from(candidate.confidence))
			})
			.filter(|(_, score)| *score >= 0.70)
			.collect::<Vec<_>>();
		matches.sort_by(|(left_id, left_score), (right_id, right_score)| {
			right_score
				.partial_cmp(left_score)
				.unwrap_or(std::cmp::Ordering::Equal)
				.then_with(|| left_id.cmp(right_id))
		});
		let match_evidence = matches
			.iter()
			.map(
				|(media_id, score)| serde_json::json!({"media_id": media_id, "score": score}),
			)
			.collect::<Vec<_>>();
		let status = if matches.is_empty() {
			QualityStatus::Pass
		} else {
			QualityStatus::Warn
		};
		let normalized_score = if matches.is_empty() { 1.0 } else { 0.5 };
		Ok(outcome(
			self.id(),
			self.name(),
			status,
			normalized_score,
			serde_json::json!({
				"exact_hash_match": false,
				"query_title": query_title,
				"matching_media_ids": matches.iter().map(|(id, _)| id).collect::<Vec<_>>(),
				"matches": match_evidence,
			}),
		))
	}
}
