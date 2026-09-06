//! `tags_complete`: the five fields every audiobook surface renders.
//!
//! Title, author, narrator, series and year. Those five are not an arbitrary
//! set: they are what an Audiobookshelf client, a car stereo, a phone lock
//! screen and Stump's own grid all read, and a book missing them shows blank
//! rows in every one of them at once. Nothing here is inferred from a filename
//! — the point of the check is to find the books whose *containers* are silent,
//! because those are the ones that will look wrong everywhere Stump is not.
//!
//! Where each field is read from is the mapping every publisher and both
//! Audiobookshelf and Plex use, so a book that satisfies this check satisfies
//! them too:
//!
//! | Field | Atom / frame | Probe field |
//! | --- | --- | --- |
//! | title | `©nam` / `TIT2` | `title` |
//! | author | `©ART` / `TPE1` | `author` |
//! | narrator | `©wrt` (composer) / `TCOM` | `narrator` |
//! | series | `©alb` (album) / `TALB` | `album` |
//! | year | `©day` / `TDRC` | `year` |
//!
//! Scoring is fractional rather than pass/fail: five missing fields and one
//! missing field are not the same problem, and a report that cannot tell them
//! apart cannot be triaged. Anything short of all five is a `WARN`, never a
//! `FAIL` — an untagged book still plays, and a library of them is a `meta-edit`
//! run, not a corruption.

use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	FixAction, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{
	audio::{not_audio, subject, unreadable, AudioSubject, TOOL_META},
	disabled_outcome, enabled_setting, outcome, QUALITY_VERSION,
};

/// Verifies the five tags every audiobook surface renders are present.
#[derive(Default)]
pub struct TagsCompleteCheck;

impl TagsCompleteCheck {
	pub fn new() -> Self {
		Self
	}
}

fn present(value: &Option<String>) -> bool {
	value
		.as_deref()
		.map(str::trim)
		.is_some_and(|value| !value.is_empty())
}

#[async_trait::async_trait]
impl QualityCheck for TagsCompleteCheck {
	fn id(&self) -> &'static str {
		"tags_complete"
	}

	fn name(&self) -> &'static str {
		"Tags complete"
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

	fn fix(&self) -> Option<FixAction> {
		Some(FixAction::new(
			TOOL_META,
			"write the missing title, author, narrator, series and year tags into the container",
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

		let fields = [
			("title", present(&probed.title)),
			("author", present(&probed.author)),
			("narrator", present(&probed.narrator)),
			("series", present(&probed.album)),
			("year", probed.year.is_some()),
		];
		let found = fields.iter().filter(|(_, present)| *present).count();
		let missing: Vec<&str> = fields
			.iter()
			.filter(|(_, present)| !*present)
			.map(|(name, _)| *name)
			.collect();

		let status = if missing.is_empty() {
			QualityStatus::Pass
		} else {
			QualityStatus::Warn
		};
		Ok(outcome(
			self.id(),
			self.name(),
			status,
			found as f64 / fields.len() as f64,
			serde_json::json!({
				"present": found,
				"expected": fields.len(),
				"missing": missing,
			}),
		))
	}
}
