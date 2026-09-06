//! `single_file`: is the audiobook one file, or 43?
//!
//! This is the heaviest audio check, and the only one whose weight an operator
//! can change, because it is the only one that is a *judgement* rather than a
//! measurement. A folder of parts plays perfectly well in Stump — the probe
//! stitches it into one publication and the player crosses the boundaries — so
//! an operator who is content with folder books can turn the weight down. What
//! they are trading away is everything that happens *outside* Stump: copied to
//! a phone the book is a playlist, opened in another client the chapter list is
//! gone, and the reading position means nothing without the database that
//! defined the part order.
//!
//! The default weight is 30 of the audio family's 100, which is deliberately
//! large enough that a split book cannot score well while still leaving a
//! well-tagged, chaptered, faststart folder book comfortably above a
//! single-file book with no tags and no chapters — because it is, for a
//! listener using Stump, the better book.

use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	FixAction, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{
	audio::{
		human_duration, not_audio, subject, unreadable, AudioSubject, TOOL_ASSEMBLE,
	},
	disabled_outcome, enabled_setting, outcome, QUALITY_VERSION,
};

/// Default weight of a split book, and the value
/// `stump_ingest::policy::AudioPolicy::server_default` carries.
pub const DEFAULT_SINGLE_FILE_WEIGHT: u16 = 30;

/// Verifies the publication is one container rather than a folder of parts.
pub struct SingleFileCheck {
	weight: u16,
}

impl Default for SingleFileCheck {
	fn default() -> Self {
		Self::new()
	}
}

impl SingleFileCheck {
	pub fn new() -> Self {
		Self {
			weight: DEFAULT_SINGLE_FILE_WEIGHT,
		}
	}

	/// The library's own weight for a split book, from
	/// `AudioPolicy::single_file_weight`.
	pub fn with_weight(weight: u16) -> Self {
		Self { weight }
	}
}

#[async_trait::async_trait]
impl QualityCheck for SingleFileCheck {
	fn id(&self) -> &'static str {
		"single_file"
	}

	fn name(&self) -> &'static str {
		"Single file"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		self.weight
	}

	fn settings(&self) -> &[SettingDefinition] {
		super::enabled_settings()
	}

	fn fix(&self) -> Option<FixAction> {
		Some(FixAction::new(
			TOOL_ASSEMBLE,
			"assemble the parts into one chaptered, faststart M4B",
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

		let parts = probed.tracks.len();
		let status = if parts == 1 {
			QualityStatus::Pass
		} else {
			QualityStatus::Fail
		};
		Ok(outcome(
			self.id(),
			self.name(),
			status,
			if parts == 1 { 1.0 } else { 0.0 },
			serde_json::json!({
				"parts": parts,
				"duration": human_duration(probed.duration_ms),
				"codec": probed.codec,
			}),
		))
	}
}
