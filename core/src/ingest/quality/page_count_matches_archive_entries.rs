use crate::ingest::contract::{
	BookSnapshot, IngestMediaKind, QualityCheck, QualityCheckError, QualityStatus,
	SettingDefinition, SettingValues,
};

use super::{
	disabled_outcome, enabled_setting, outcome, page_count, read_archive_entries,
	QUALITY_VERSION,
};

/// Verifies page count matches archive entries.
#[derive(Default)]
pub struct PageCountMatchesArchiveEntriesCheck;

impl PageCountMatchesArchiveEntriesCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for PageCountMatchesArchiveEntriesCheck {
	fn id(&self) -> &'static str {
		"page_count_matches_archive_entries"
	}

	fn name(&self) -> &'static str {
		"Page count matches archive entries"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		20
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
		if !matches!(book.media_kind, IngestMediaKind::ComicArchive) {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({"applicable": false, "format": format!("{:?}", book.media_kind)}),
			));
		}

		let processor_page_count = page_count(book);
		let entries = match read_archive_entries(&book.staged_path) {
			Ok(entries) => entries,
			Err(_) => {
				return Ok(outcome(
					self.id(),
					self.name(),
					QualityStatus::Fail,
					0.0,
					serde_json::json!({
						"entry_count": null,
						"processor_page_count": processor_page_count,
						"difference": null,
					}),
				));
			},
		};
		let entry_count = entries.iter().filter(|entry| entry.is_image).count() as i64;
		let evidence = serde_json::json!({
			"entry_count": entry_count,
			"processor_page_count": processor_page_count,
			"difference": (entry_count - processor_page_count).abs(),
		});

		if processor_page_count <= 0 {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				evidence,
			));
		}
		let difference = (entry_count - processor_page_count).abs();
		let (status, score) = match difference {
			0 => (QualityStatus::Pass, 1.0),
			1 => (QualityStatus::Warn, 0.5),
			_ => (QualityStatus::Fail, 0.0),
		};
		Ok(outcome(self.id(), self.name(), status, score, evidence))
	}
}
