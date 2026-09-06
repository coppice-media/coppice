//! `faststart`: is the index at the front of the file?
//!
//! An MP4 keeps its sample tables in a `moov` box that may sit anywhere in the
//! file. A muxer that streams the media first has to put `moov` at the end,
//! because a sample table is not known until the last sample is written — and
//! a player fetching that file over HTTP cannot decode a single frame until it
//! has the index. For a 700 MB audiobook that means downloading the whole book
//! before playback starts, on every device, every time the cache is cold.
//!
//! `-movflags +faststart` is the second pass that moves `moov` ahead of `mdat`,
//! and it is the single cheapest thing that can be done to an audiobook
//! library: it changes no audio, costs one rewrite, and turns "buffering" into
//! "playing".
//!
//! The check reads only the *top-level* box order, which is a handful of seeks
//! regardless of file size, and reports `NOT_APPLICABLE` for a container that
//! has no index to misplace — an MP3 or an Ogg is a stream, not a box tree.

use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	FixAction, QualityCheck, QualityCheckError, QualityCheckOutcome, QualityStatus,
};

use super::{
	audio::{
		moov_before_mdat, not_audio, subject, unreadable, AudioSubject, TOOL_ASSEMBLE,
	},
	disabled_outcome, enabled_setting, outcome, QUALITY_VERSION,
};

/// Verifies an MP4 audiobook's `moov` box precedes its media data.
#[derive(Default)]
pub struct FaststartCheck;

impl FaststartCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for FaststartCheck {
	fn id(&self) -> &'static str {
		"faststart"
	}

	fn name(&self) -> &'static str {
		"Faststart"
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
			"rewrite the book with its moov box ahead of the media data",
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

		// A folder book is judged on the part a player opens first: it is the
		// one whose index has to arrive before anything is audible.
		let first = probed
			.tracks
			.first()
			.map(|track| track.path.clone())
			.unwrap_or_else(|| book.staged_path.clone());

		match moov_before_mdat(&first)? {
			Some(true) => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Pass,
				1.0,
				serde_json::json!({ "container": "mp4", "moov_first": true }),
			)),
			Some(false) => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				serde_json::json!({ "container": "mp4", "moov_first": false }),
			)),
			// Not a box-structured container: there is no index to put first.
			None => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({
					"applicable": false,
					"reason": "not_an_mp4_container",
					"codec": probed.codec,
				}),
			)),
		}
	}
}
