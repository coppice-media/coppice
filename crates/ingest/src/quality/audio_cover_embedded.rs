//! `cover_embedded`: does the container carry its own artwork?
//!
//! Stump can serve a cover from a sidecar image, so a book that fails this
//! check still looks right *in Stump*. What it does not look right in is
//! everywhere else: a phone, a car stereo, another client, or Stump itself
//! after the sidecar is lost in a move. The cover is a property of the
//! publication or it is a property of one server's folder layout, and only one
//! of those survives.
//!
//! The finding is a `WARN`, not a `FAIL`. A book with no artwork is complete;
//! it is just less pleasant, and `meta-edit` fixes it from the sidecar the
//! library already has.

use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	FixAction, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{
	audio::{not_audio, subject, unreadable, AudioSubject, TOOL_META},
	disabled_outcome, enabled_setting, outcome, QUALITY_VERSION,
};

/// Verifies the audiobook container carries embedded cover art.
#[derive(Default)]
pub struct CoverEmbeddedCheck;

impl CoverEmbeddedCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for CoverEmbeddedCheck {
	fn id(&self) -> &'static str {
		"cover_embedded"
	}

	fn name(&self) -> &'static str {
		"Cover embedded"
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

	fn fix(&self) -> Option<FixAction> {
		Some(FixAction::new(
			TOOL_META,
			"embed the library's cover image into the container as covr",
		))
	}

	async fn run(
		&self,
		book: &crate::contract::BookSnapshot,
		settings: &SettingValues,
	) -> Result<QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}
		let probed = match subject(book) {
			AudioSubject::NotAudio => return Ok(not_audio(self.id(), self.name())),
			AudioSubject::Unreadable(error) => {
				return Ok(unreadable(self.id(), self.name(), &error))
			},
			AudioSubject::Probed(probed) => probed,
		};

		match &probed.cover {
			Some((content_type, bytes)) => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Pass,
				1.0,
				serde_json::json!({
					"embedded": true,
					"content_type": content_type.mime_type(),
					"byte_count": bytes.len(),
				}),
			)),
			None => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Warn,
				0.0,
				serde_json::json!({ "embedded": false }),
			)),
		}
	}
}
