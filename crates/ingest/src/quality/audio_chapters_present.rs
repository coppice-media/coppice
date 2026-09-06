//! `chapters_present`: does the book carry a chapter list a publisher wrote?
//!
//! Without marks a client can offer a time scrubber and nothing else, which for
//! a 27-hour book is the difference between "go to chapter 14" and "guess".
//!
//! The check grades three states rather than two, because
//! [`ChapterSource`](stump_media::audio::ChapterSource) is provenance and the
//! middle state is real:
//!
//! * **Pass** — the marks came out of the container (`chpl`, a chapter track,
//!   ID3v2 `CHAP`, Vorbis `CHAPTERxxx`). Somebody authored them.
//! * **Warn** — `PER_TRACK`: Stump synthesized one chapter per file because
//!   the publisher shipped none. The list exists inside Stump and nowhere
//!   else; `audio-chapters` makes it a property of the files.
//! * **Fail** — no marks at all.
//!
//! A warn scores 0.5 and not 0.0 on purpose: a folder book with file
//! boundaries named after its parts is genuinely half way there, and scoring it
//! as harshly as a book with nothing would make the two indistinguishable in a
//! report a librarian is trying to triage.

use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_media::audio::ChapterSource;

use crate::contract::{
	FixAction, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{
	audio::{not_audio, subject, unreadable, AudioSubject, TOOL_CHAPTERS},
	disabled_outcome, enabled_setting, outcome, QUALITY_VERSION,
};

/// What a synthesized chapter list is worth: half of a real one.
const SYNTHESIZED_SCORE: f64 = 0.5;

/// Verifies the publication carries chapter marks, and says who authored them.
#[derive(Default)]
pub struct ChaptersPresentCheck;

impl ChaptersPresentCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for ChaptersPresentCheck {
	fn id(&self) -> &'static str {
		"chapters_present"
	}

	fn name(&self) -> &'static str {
		"Chapters present"
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
			TOOL_CHAPTERS,
			"embed the derived chapter marks into the files as chpl or ID3v2 CHAP frames",
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

		let (status, score) = match probed.chapter_source {
			ChapterSource::Mp4Chpl
			| ChapterSource::Mp4ChapterTrack
			| ChapterSource::Id3Chap
			| ChapterSource::VorbisComment => (QualityStatus::Pass, 1.0),
			ChapterSource::PerTrack => (QualityStatus::Warn, SYNTHESIZED_SCORE),
			ChapterSource::None => (QualityStatus::Fail, 0.0),
		};

		Ok(outcome(
			self.id(),
			self.name(),
			status,
			score,
			serde_json::json!({
				"chapters": probed.chapters.len(),
				"source": probed.chapter_source,
				"authored": status == QualityStatus::Pass,
			}),
		))
	}
}
