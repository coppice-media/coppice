use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	BookSnapshot, IngestMediaKind, QualityCheck, QualityCheckError, QualityStatus,
};

use super::{
	disabled_outcome, enabled_setting, first_decodable_page, outcome, QUALITY_VERSION,
};

/// Verifies that the format adapter can select and decode a cover image.
#[derive(Default)]
pub struct CoverPresentCheck;

impl CoverPresentCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for CoverPresentCheck {
	fn id(&self) -> &'static str {
		"cover_present"
	}

	fn name(&self) -> &'static str {
		"Cover present"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		15
	}

	fn settings(&self) -> &[SettingDefinition] {
		super::enabled_settings()
	}

	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<crate::contract::QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}
		if matches!(book.media_kind, IngestMediaKind::Unknown) {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({"applicable": false, "reason": "unsupported_format"}),
			));
		}

		match first_decodable_page(book) {
			Ok(Some((path, bytes))) => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Pass,
				1.0,
				serde_json::json!({
					"selected_path": path,
					"decoded": true,
					"byte_count": bytes.len(),
				}),
			)),
			Ok(None) | Err(_) => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				serde_json::json!({"selected_path": null, "decoded": false}),
			)),
		}
	}
}
