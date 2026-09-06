//! `bitrate_sane`: is the book encoded like speech?
//!
//! An audiobook is one or two voices in a treated room. 32–128 kbit/s of AAC or
//! Opus is transparent for that material, and both ends of the range are a
//! problem worth a librarian's attention:
//!
//! * **Below the floor** the voice is audibly damaged — metallic, or swimming
//!   in artefacts — and no amount of re-encoding recovers it. The check can
//!   only report it; the source has to be replaced.
//! * **Above the ceiling** nothing is gained and a great deal is spent: a
//!   30-hour book at 320 kbit/s is 4 GB instead of 800 MB, which is four times
//!   the disk, the backup, the sync and the mobile data for audio nobody can
//!   distinguish. `audio-assemble` re-encodes it down.
//!
//! A mixed-codec folder book fails outright rather than being graded: its parts
//! do not share a bitrate, so there is no single number to judge, and a client
//! that builds one decoder for the publication may refuse to play it through at
//! all. That is a real defect and the assembler is the fix.
//!
//! Both bounds are `WARN`s, not `FAIL`s: the book plays, and whether a
//! low-bitrate rip is acceptable is the librarian's call, not the checker's.

use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	FixAction, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{
	audio::{not_audio, subject, unreadable, AudioSubject, TOOL_ASSEMBLE},
	disabled_outcome, enabled_setting, outcome, QUALITY_VERSION,
};

/// Below this, speech is audibly damaged. Bits per second.
const FLOOR: i32 = 24_000;
/// Above this, nothing is gained for one or two voices. Bits per second.
const CEILING: i32 = 192_000;
/// The publication codec of a folder book whose parts disagree, as
/// `stump_media::audio` reports it.
const MIXED: &str = "mixed";

/// What a book outside the speech range scores.
const OUT_OF_RANGE_SCORE: f64 = 0.5;

/// Verifies the publication's bitrate is in the range speech needs.
#[derive(Default)]
pub struct BitrateSaneCheck;

impl BitrateSaneCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for BitrateSaneCheck {
	fn id(&self) -> &'static str {
		"bitrate_sane"
	}

	fn name(&self) -> &'static str {
		"Bitrate sane"
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
		Some(
			FixAction::new(
				TOOL_ASSEMBLE,
				"re-encode the book to one AAC stream at a speech bitrate",
			)
			.with_options(serde_json::json!({ "bitrate": "64k" })),
		)
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

		if probed.codec == MIXED {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				serde_json::json!({
					"codec": MIXED,
					"reason": "the parts do not share a codec, so there is no publication bitrate",
					"codecs": probed
						.tracks
						.iter()
						.map(|track| track.codec.clone())
						.collect::<std::collections::BTreeSet<_>>(),
				}),
			));
		}

		let Some(bitrate) = probed.bitrate.filter(|bitrate| *bitrate > 0) else {
			// Nothing stated and nothing derivable: `duration_consistent`
			// reports the real problem, so this check declines rather than
			// double-counting it.
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({
					"applicable": false,
					"reason": "no_bitrate_available",
				}),
			));
		};

		let (status, score, verdict) = if bitrate < FLOOR {
			(QualityStatus::Warn, OUT_OF_RANGE_SCORE, "below_floor")
		} else if bitrate > CEILING {
			(QualityStatus::Warn, OUT_OF_RANGE_SCORE, "above_ceiling")
		} else {
			(QualityStatus::Pass, 1.0, "in_range")
		};

		Ok(outcome(
			self.id(),
			self.name(),
			status,
			score,
			serde_json::json!({
				"bitrate": bitrate,
				"codec": probed.codec,
				"floor": FLOOR,
				"ceiling": CEILING,
				"verdict": verdict,
			}),
		))
	}
}
