//! `audio-report`: what Stump can actually tell a client about an audiobook.
//!
//! An audiobook is not self-describing the way a CBZ is. Two files that look
//! identical in a file manager can differ in the only three things a listener
//! notices: whether the book has real chapter marks, whether it plays through
//! as one codec, and whether it has a cover. None of that is visible without
//! demuxing the container, so a librarian who wants to know which of 400 books
//! need attention has no way to ask — that is the gap this tool fills.
//!
//! The findings are exactly the conditions that change what a client can do:
//!
//! * **No chapters at all** — the player can only offer a time scrubber.
//! * **Chapters synthesized per file** ([`ChapterSource::PerTrack`]) — the
//!   marks exist but the publisher did not write them, so they are file
//!   boundaries wearing chapter names. `audio-chapters` can turn them into
//!   real, portable marks.
//! * **A mixed-codec folder book** — every part plays, but a client that
//!   builds one decoder for the publication may refuse to play it through.
//! * **A zero duration** — nothing about the book is trustworthy.
//! * **No embedded cover** — the library grid will show a placeholder.
//!
//! This tool is report-only, like [`crate::missing_sequence`]: the plan *is*
//! the finding set and [`AudioReport::apply`] writes nothing, so `plan` and
//! `apply` can never disagree. One consequence is deliberate: a broken
//! publication keeps its action row and carries an [`Severity::Error`]
//! warning, because a report whose apply silently dropped the broken books
//! would be worse than no report.

use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
};

use globset::GlobSet;
use serde::{Deserialize, Serialize};
use stump_media::{
	audio::{self, ChapterSource, ProbedAudio},
	PathUtils,
};

use crate::{
	util::sorted_dirs, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError,
	ToolInput, Warning,
};

const ID: &str = "audio-report";

/// One audio publication's facts. A report tool has exactly one action kind:
/// the row *is* the finding, and what varies is the detail and the warnings.
const KIND_REPORT: &str = "report-audio";

/// The publication codec of a folder book whose parts disagree.
const MIXED: &str = "mixed";

/// Reports the audio facts of every publication under the given paths.
pub struct AudioReport;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioReportOptions {
	/// Include the chapter marks and the file list in every report detail.
	/// Off by default because a 200-part book would drown the summary rows.
	pub detailed: bool,
}

/// The per-publication finding carried by an action's `detail`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct AudioDetail {
	duration_ms: i64,
	/// `h:mm:ss`. A CLI table cell holding `27543000` tells a librarian
	/// nothing, and this is the field they scan the report for.
	duration: String,
	codec: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	sample_rate: Option<i32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	channels: Option<i32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	bitrate: Option<i32>,
	/// The snake_case [`ChapterSource`]: provenance, never a quality tier.
	chapter_source: String,
	chapters: usize,
	tracks: usize,
	has_cover: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	author: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	narrator: Option<String>,
	/// Only with `detailed`: `0:00:00 Opening` per mark.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	marks: Vec<String>,
	/// Only with `detailed`: the file names in playback order.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	files: Vec<String>,
}

impl Tool for AudioReport {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Report the duration, codec, chapter provenance, track count and cover of every audiobook under the given paths"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<AudioReportOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}

		let mut plan = Plan::new(ID);
		let publications = publications(&input.paths, &mut plan)?;
		if publications.is_empty() {
			plan.warn(
				Warning::new("no-audio", "no audio publications in the given paths")
					.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		for path in publications {
			// One unreadable book must not cost the report for the other 399.
			let probed = match audio::probe(&path) {
				Ok(probed) => probed,
				Err(error) => {
					plan.warn(
						Warning::new("probe-failed", error.to_string())
							.at(&path)
							.with_severity(Severity::Error),
					);
					continue;
				},
			};

			warn_about(&mut plan, &path, &probed);
			plan.push(
				Action::new(KIND_REPORT)
					.with_source(&path)
					.with_detail(serde_json::to_value(detail(&probed, &options))?),
			);
		}

		Ok(plan)
	}

	fn apply(
		&self,
		plan: &Plan,
		sink: &mut dyn ProgressSink,
	) -> Result<Report, ToolError> {
		plan.expect_tool(ID)?;
		// Report-only: the plan is the finding set, so nothing is written and
		// every finding is reported as applied.
		let mut report = Report::for_plan(plan);
		let total = plan.actions.len();
		for (index, action) in plan.actions.iter().enumerate() {
			let publication = action
				.source
				.as_deref()
				.unwrap_or(Path::new(""))
				.to_string_lossy();
			sink.progress(index + 1, total, &publication);
			report.applied(action.clone());
		}
		Ok(report)
	}
}

/// Every audio publication under `paths`, in natural path order.
///
/// A publication is one container file *or* one folder of parts, and the same
/// three shapes a scan sees are accepted here: a file, an audiobook folder,
/// and a *library* folder holding both loose containers and audiobook folders.
/// The library case is walked exactly one level, because
/// [`PathUtils::dir_is_audio_book`] already refuses a directory that holds
/// subdirectories — a deeper walk could only find folders that are not books.
///
/// The ignore rules are empty: this crate takes paths plus a JSON options blob
/// and has no library configuration to read. `is_hidden_file` and
/// `is_default_ignored` still apply inside the media helpers.
///
/// [`crate::audio_chapters`] resolves its targets through this same function,
/// so it can never name a file `audio-report` would not have listed.
pub(crate) fn publications(
	paths: &[PathBuf],
	plan: &mut Plan,
) -> Result<Vec<PathBuf>, ToolError> {
	let ignore = GlobSet::empty();
	let mut found = Vec::new();

	for path in paths {
		if path.is_file() {
			if path.as_path().is_audio() {
				found.push(path.clone());
			} else {
				plan.warn(
					Warning::new("not-audio", "not an audio file")
						.at(path)
						.with_severity(Severity::Error),
				);
			}
		} else if path.is_dir() {
			if path.as_path().dir_is_audio_book(&ignore) {
				found.push(path.clone());
				continue;
			}

			let before = found.len();
			found.extend(audio::audio_files_in(path)?);
			for dir in sorted_dirs(path)? {
				if dir.as_path().dir_is_audio_book(&ignore) {
					found.push(dir);
				}
			}
			if found.len() == before {
				plan.warn(
					Warning::new(
						"no-audio",
						"no audio files or audiobook folders in this directory",
					)
					.at(path)
					.with_severity(Severity::Error),
				);
			}
		} else {
			plan.warn(
				Warning::new("missing-path", "path does not exist")
					.at(path)
					.with_severity(Severity::Error),
			);
		}
	}

	// A caller is free to pass a folder and one of its own books; a
	// publication must still be probed and reported exactly once.
	alphanumeric_sort::sort_path_slice(&mut found);
	found.dedup();
	Ok(found)
}

/// The conditions a librarian has to act on, and nothing else.
fn warn_about(plan: &mut Plan, path: &Path, probed: &ProbedAudio) {
	if probed.duration_ms == 0 {
		plan.warn(
			Warning::new("zero-duration", "duration could not be determined")
				.at(path)
				.with_severity(Severity::Error),
		);
	}

	match probed.chapter_source {
		ChapterSource::None => plan.warn(
			Warning::new(
				"no-chapters",
				"no chapter marks: clients can only seek by time",
			)
			.at(path),
		),
		ChapterSource::PerTrack => plan.warn(
			Warning::new(
				"synthesized-chapters",
				"chapters are synthesized one per file; the publisher shipped no marks",
			)
			.at(path)
			.with_severity(Severity::Info),
		),
		_ => {},
	}

	if probed.codec == MIXED {
		let codecs = probed
			.tracks
			.iter()
			.map(|track| track.codec.as_str())
			.collect::<BTreeSet<_>>()
			.into_iter()
			.collect::<Vec<_>>()
			.join(", ");
		plan.warn(
			Warning::new(
				"mixed-codec",
				format!("parts do not share one codec ({codecs})"),
			)
			.at(path),
		);
	}

	if probed.cover.is_none() {
		plan.warn(
			Warning::new("no-cover", "no embedded cover art")
				.at(path)
				.with_severity(Severity::Info),
		);
	}
}

fn detail(probed: &ProbedAudio, options: &AudioReportOptions) -> AudioDetail {
	AudioDetail {
		duration_ms: probed.duration_ms,
		duration: human_duration(probed.duration_ms),
		codec: probed.codec.clone(),
		sample_rate: probed.sample_rate,
		channels: probed.channels,
		bitrate: probed.bitrate,
		chapter_source: probed.chapter_source.to_string(),
		chapters: probed.chapters.len(),
		tracks: probed.tracks.len(),
		has_cover: probed.cover.is_some(),
		title: probed.title.clone(),
		author: probed.author.clone(),
		narrator: probed.narrator.clone(),
		marks: if options.detailed {
			probed
				.chapters
				.iter()
				.map(|chapter| {
					let start = human_duration(chapter.start_ms);
					match chapter.title.as_deref() {
						Some(title) => format!("{start} {title}"),
						None => start,
					}
				})
				.collect()
		} else {
			Vec::new()
		},
		files: if options.detailed {
			probed
				.tracks
				.iter()
				.map(|track| file_name(&track.path).to_string())
				.collect()
		} else {
			Vec::new()
		},
	}
}

/// `h:mm:ss`. Hours are never zero-padded: a book is 9 or 27 hours long, and
/// the field is read, not sorted.
fn human_duration(duration_ms: i64) -> String {
	let seconds = duration_ms.max(0) / 1_000;
	format!(
		"{}:{:02}:{:02}",
		seconds / 3_600,
		(seconds % 3_600) / 60,
		seconds % 60
	)
}

fn file_name(path: &Path) -> &str {
	path.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NoopProgress;

	fn fixture(name: &str) -> PathBuf {
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("../media/integration-tests/data/audio")
			.join(name)
	}

	fn plan_for(path: &Path, options: serde_json::Value) -> Plan {
		AudioReport
			.plan(&ToolInput::new(vec![path.to_path_buf()]).with_options(options))
			.expect("plan succeeds")
	}

	fn detail_of(action: &Action) -> AudioDetail {
		serde_json::from_value(action.detail.clone()).expect("decode detail")
	}

	fn codes(plan: &Plan) -> Vec<&str> {
		plan.warnings
			.iter()
			.map(|warning| warning.code.as_str())
			.collect()
	}

	/// A single container is reported with its own tags, its cover and the
	/// container mechanism its marks came from — and a chapterless book is
	/// flagged, because a client can then only offer a time scrubber.
	#[test]
	fn a_single_container_is_reported_with_its_cover_and_provenance() {
		let plan = plan_for(&fixture("plain.m4b"), serde_json::json!({}));
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.actions[0].kind, KIND_REPORT);

		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.duration_ms, 6000);
		assert_eq!(detail.duration, "0:00:06");
		assert_eq!(detail.codec, "aac");
		assert_eq!(detail.sample_rate, Some(22050));
		assert_eq!(detail.channels, Some(1));
		assert_eq!(detail.chapter_source, "none");
		assert_eq!(detail.chapters, 0);
		assert_eq!(detail.tracks, 1);
		assert!(detail.has_cover, "plain.m4b carries a PNG covr atom");
		assert_eq!(detail.title.as_deref(), Some("Chapterless Book"));
		assert_eq!(detail.author.as_deref(), Some("Test Narrator"));
		assert!(codes(&plan).contains(&"no-chapters"));
		assert!(!codes(&plan).contains(&"no-cover"));

		// The same probe on a book with publisher marks and no artwork: the
		// provenance is the concrete MP4 mechanism, and the missing cover is
		// the finding.
		let plan = plan_for(&fixture("chapters-chpl.m4b"), serde_json::json!({}));
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.chapter_source, "mp4_chpl");
		assert_eq!(detail.chapters, 3);
		assert!(!detail.has_cover, "chapters-chpl.m4b has no covr atom");
		assert!(codes(&plan).contains(&"no-cover"));
		assert!(!codes(&plan).contains(&"no-chapters"));
	}

	/// A folder book whose files carry no marks at all gets one chapter per
	/// file, and that has to be reported as synthesized: the marks are file
	/// boundaries, not something the publisher authored.
	#[test]
	fn a_folder_book_reports_synthesized_chapters_per_track() {
		let plan = plan_for(
			&fixture("folder-filename"),
			serde_json::json!({"detailed": true}),
		);
		assert_eq!(plan.actions.len(), 1);

		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.chapter_source, "per_track");
		assert_eq!(detail.chapters, 3);
		assert_eq!(detail.tracks, 3);
		assert_eq!(
			detail.files,
			vec!["01 - Part 1.mp3", "02 - Part 2.mp3", "03 - Part 3.mp3"]
		);
		assert_eq!(
			detail.marks,
			vec![
				"0:00:00 01 - Part 1",
				"0:00:02 02 - Part 2",
				"0:00:04 03 - Part 3"
			]
		);
		assert!(codes(&plan).contains(&"synthesized-chapters"));
		assert!(
			!codes(&plan).contains(&"no-chapters"),
			"the book has marks, they are just not the publisher's"
		);
	}

	/// Parts that do not share a codec still play one by one, but a client
	/// that builds one decoder for the publication may refuse the book, so
	/// the disagreeing codecs are named.
	#[test]
	fn a_mixed_codec_folder_book_names_the_codecs() {
		let plan = plan_for(&fixture("folder-mixed"), serde_json::json!({}));
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.codec, MIXED);
		assert_eq!(detail.tracks, 2);

		let warning = plan
			.warnings
			.iter()
			.find(|warning| warning.code == "mixed-codec")
			.expect("mixed-codec warning");
		assert_eq!(warning.message, "parts do not share one codec (mp3, opus)");
		assert_eq!(warning.severity, Severity::Warn);
	}

	/// A library folder holds loose containers *and* folder books; both are
	/// publications, and a folder that only contains books is never itself
	/// probed as one.
	#[test]
	fn a_library_folder_is_walked_one_level_into_its_publications() {
		let plan = plan_for(&fixture(""), serde_json::json!({}));

		let sources = plan
			.actions
			.iter()
			.map(|action| {
				file_name(action.source.as_deref().expect("source")).to_string()
			})
			.collect::<Vec<_>>();
		assert_eq!(
			sources,
			vec![
				"chapters-chpl.m4b",
				"chapters-id3.mp3",
				"chapters-track.m4b",
				"chapters-vorbis.opus",
				"folder-filename",
				"folder-mixed",
				"folder-tracknum",
				"plain.m4b",
			]
		);
	}

	/// The plan is the deliverable: `apply` writes nothing and re-emits every
	/// finding, so a report tool's dry run can never disagree with its run.
	#[test]
	fn apply_writes_nothing_and_re_emits_the_findings() {
		let path = fixture("chapters-vorbis.opus");
		let before = std::fs::read(&path).expect("read fixture");

		let plan = plan_for(&path, serde_json::json!({}));
		let report = AudioReport
			.apply(&plan, &mut NoopProgress)
			.expect("apply succeeds");

		assert_eq!(report.applied, plan.actions);
		assert!(report.skipped.is_empty());
		assert_eq!(report.warnings, plan.warnings);
		assert_eq!(
			std::fs::read(&path).expect("re-read fixture"),
			before,
			"a report-only apply must not touch a byte"
		);
	}

	/// A path that is not audio at all, and a path that does not exist, are
	/// findings rather than a failed run: a bulk report over a library must
	/// not die on one stray file.
	#[test]
	fn unusable_paths_are_reported_and_never_abort_the_run() {
		let plan = AudioReport
			.plan(&ToolInput::new(vec![
				fixture("plain.m4b"),
				fixture("nope.m4b"),
				PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
			]))
			.expect("plan succeeds");

		assert_eq!(plan.actions.len(), 1);
		assert!(codes(&plan).contains(&"missing-path"));
		assert!(codes(&plan).contains(&"not-audio"));

		let error = AudioReport
			.plan(&ToolInput::new(Vec::new()))
			.expect_err("no paths is a caller error");
		assert!(matches!(error, ToolError::Invalid(_)), "{error}");
	}
}
