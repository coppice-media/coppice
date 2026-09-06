use serde_json::json;
use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_media::drm::detect_drm;

use crate::contract::{
	BookSnapshot, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{disabled_outcome, enabled_setting, outcome, QUALITY_VERSION};

/// Upper bound on the marker list copied into the report evidence.
const MAX_MARKERS: usize = 8;

/// Rejects files whose content is encrypted, so a book whose pages can never
/// be decoded is never committed to a library.
///
/// This is a blocking gate, not a scored signal: its weight is `0`, so it
/// contributes nothing to the score and instead surfaces through the report's
/// failed-check list. A DRM'd file is not a *low quality* book, it is a file
/// this server cannot read at all.
///
/// Stump neither ships nor invokes any DRM-removal tool. Removing DRM from
/// purchased files is the operator's own responsibility, performed in their own
/// calibre before the file reaches the drop folder; see
/// `docs/content/docs/developer/calibre-tooling.mdx`.
#[derive(Default)]
pub struct DrmProtectedCheck;

impl DrmProtectedCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for DrmProtectedCheck {
	fn id(&self) -> &'static str {
		"drm_protected"
	}

	fn name(&self) -> &'static str {
		"DRM protected"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	/// Blocking gate: it never moves the score, so the weights of the scored
	/// checks keep summing to 100.
	fn weight(&self) -> u16 {
		0
	}

	fn settings(&self) -> &[SettingDefinition] {
		super::enabled_settings()
	}

	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}

		// The detector sniffs the container, so a mislabelled file (a MOBI
		// named `.epub`) is still classified; no media-kind gate here.
		let report = detect_drm(&book.staged_path).map_err(|error| {
			QualityCheckError::Internal {
				check_id: self.id().to_string(),
				message: error.to_string(),
			}
		})?;

		let Some(report) = report else {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Pass,
				1.0,
				json!({ "protected": false }),
			));
		};

		let truncated = report.markers.len() > MAX_MARKERS;
		let evidence = json!({
			"protected": true,
			"blocking": true,
			"container": report.container.as_str(),
			"scheme": report.scheme,
			"scheme_label": report.scheme.label(),
			"markers": report.markers.iter().take(MAX_MARKERS).collect::<Vec<_>>(),
			"markers_truncated": truncated,
			"reason": report.reason,
		});
		Ok(outcome(
			self.id(),
			self.name(),
			QualityStatus::Fail,
			0.0,
			evidence,
		))
	}
}
