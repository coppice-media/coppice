use std::{collections::BTreeMap, sync::Arc};

use sea_orm::DatabaseConnection;
use serde::Serialize;

use crate::ingest::contract::{
	BookSnapshot, QualityCheck, QualityCheckError, QualityReport, SettingDefinition,
	SettingValues, QUALITY_ALGORITHM_VERSION,
};

use super::{
	CoverNotPageTwoCheck, CoverPresentCheck, DuplicateExistingCheck,
	EpubTocChaptersCheck, FilenameSeriesParseCheck, ImageDimensionsConsistentCheck,
	PageCountMatchesArchiveEntriesCheck,
};

/// Discovery information for one registered quality check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckDescriptor {
	pub id: String,
	pub name: String,
	pub version: String,
	pub weight: u16,
	pub algorithm_version: String,
	pub settings: Vec<SettingDefinition>,
}

/// Ordered quality-check registry used by the staged analysis coordinator.
pub struct QualityRegistry {
	checks: Vec<Arc<dyn QualityCheck>>,
}

impl QualityRegistry {
	pub fn builtin(conn: Arc<DatabaseConnection>) -> Self {
		Self {
			checks: vec![
				Arc::new(CoverPresentCheck::new()),
				Arc::new(CoverNotPageTwoCheck::new()),
				Arc::new(PageCountMatchesArchiveEntriesCheck::new()),
				Arc::new(ImageDimensionsConsistentCheck::new()),
				Arc::new(EpubTocChaptersCheck::new()),
				Arc::new(DuplicateExistingCheck::new(conn)),
				Arc::new(FilenameSeriesParseCheck::new()),
			],
		}
	}

	pub fn checks(&self) -> &[Arc<dyn QualityCheck>] {
		&self.checks
	}

	pub fn catalog(&self) -> Vec<CheckDescriptor> {
		self.checks
			.iter()
			.map(|check| CheckDescriptor {
				id: check.id().to_string(),
				name: check.name().to_string(),
				version: check.version().to_string(),
				weight: check.weight(),
				algorithm_version: QUALITY_ALGORITHM_VERSION.to_string(),
				settings: check.settings().to_vec(),
			})
			.collect()
	}

	pub async fn run_all(
		&self,
		book: &BookSnapshot,
		settings: &BTreeMap<String, SettingValues>,
	) -> Result<QualityReport, QualityCheckError> {
		let mut outcomes = Vec::with_capacity(self.checks.len());
		let mut settings_snapshot = BTreeMap::new();
		for check in &self.checks {
			let mut effective = check
				.settings()
				.iter()
				.map(|definition| {
					(definition.key.to_string(), definition.default.clone())
				})
				.collect::<SettingValues>();
			if let Some(configured) = settings.get(check.id()) {
				effective.extend(configured.clone());
			}
			settings_snapshot.insert(check.id().to_string(), effective.clone());
			let outcome = check.run(book, &effective).await?;
			outcomes.push((outcome, check.weight()));
		}
		let (score, checks) = crate::ingest::contract::score_report(&outcomes);
		Ok(QualityReport {
			source_sha256: book.source_sha256.clone(),
			algorithm_version: QUALITY_ALGORITHM_VERSION.to_string(),
			score,
			checks,
			settings_snapshot,
		})
	}

	pub fn total_weight(&self) -> u16 {
		self.checks.iter().map(|check| check.weight()).sum()
	}
}
