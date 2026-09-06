//! `duration_consistent`: is the stated duration the duration that plays?
//!
//! Every progress bar, every "3 hours left", every resume position and every
//! chapter offset is computed against one number: the publication duration.
//! When that number is wrong, nothing about the book is trustworthy, and the
//! failure is silent — the book plays, it just lies about where it is.
//!
//! Three ways it goes wrong, all of them found here:
//!
//! * **Zero duration.** The container states nothing and nothing could be
//!   derived. A book that claims to be 0 ms long cannot be resumed at all.
//! * **A part with no duration.** One file of a folder book contributes
//!   nothing, so every offset after it is short by that part's real length —
//!   the chapter list silently slides.
//! * **The last chapter mark starts after the book ends.** A chapter list
//!   inherited from a different edition, which is what a `chapters.txt` copied
//!   between rips produces. Every mark past the end is a row a listener can
//!   tap and never reach.
//!
//! Zero duration is a `FAIL`; a book whose marks overrun its audio is a `WARN`,
//! because the audio itself is fine and it is the list that needs replacing.

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

/// What a book whose chapter marks overrun its audio scores.
const OVERRUN_SCORE: f64 = 0.5;

/// Verifies the publication duration is usable and internally consistent.
#[derive(Default)]
pub struct DurationConsistentCheck;

impl DurationConsistentCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for DurationConsistentCheck {
	fn id(&self) -> &'static str {
		"duration_consistent"
	}

	fn name(&self) -> &'static str {
		"Duration consistent"
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
			TOOL_ASSEMBLE,
			"rebuild the book so its sample tables and chapter marks agree on one duration",
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

		let silent_parts = probed
			.tracks
			.iter()
			.filter(|track| track.duration_ms <= 0)
			.count();
		let overrunning_marks = probed
			.chapters
			.iter()
			.filter(|chapter| chapter.start_ms >= probed.duration_ms)
			.count();

		let (status, score) = if probed.duration_ms <= 0 || silent_parts > 0 {
			(QualityStatus::Fail, 0.0)
		} else if overrunning_marks > 0 {
			(QualityStatus::Warn, OVERRUN_SCORE)
		} else {
			(QualityStatus::Pass, 1.0)
		};

		Ok(outcome(
			self.id(),
			self.name(),
			status,
			score,
			serde_json::json!({
				"duration_ms": probed.duration_ms,
				"duration": human_duration(probed.duration_ms),
				"parts": probed.tracks.len(),
				"parts_without_duration": silent_parts,
				"marks_past_the_end": overrunning_marks,
			}),
		))
	}
}
