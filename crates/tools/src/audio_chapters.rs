//! `audio-chapters`: turn the chapter marks Stump *derived* into marks the
//! container itself carries.
//!
//! A folder audiobook with no embedded marks reads well in Stump — the probe
//! synthesizes one chapter per file and records that as
//! [`ChapterSource::PerTrack`] — but nothing of that survives the library. A
//! listener who copies the files to a phone, or plays them in any other
//! client, is back to a time scrubber. This tool writes those marks into the
//! files, so the chapter list becomes a property of the book instead of a
//! property of Stump's database.
//!
//! # What it writes, and what it refuses to
//!
//! | Container | Mechanism | Library |
//! | --- | --- | --- |
//! | `.m4b`, `.m4a` | Nero `chpl` chapter list in `moov/udta` | [`mp4ameta`] |
//! | `.mp3` | ID3v2 `CHAP` frames plus a top-level `CTOC` | [`id3`] |
//! | anything else | refused | — |
//!
//! Ogg, Opus and FLAC carry their marks in `CHAPTERxxx` comments, and
//! rewriting a comment header means rewriting the container's first page and
//! every following page's granule bookkeeping. A metadata tool that gets that
//! wrong produces a file no player will open, so this one refuses the whole
//! family instead.
//!
//! Two further rules make the tool safe to point at a library:
//!
//! * **Marks a publisher authored are never overwritten** without `force`.
//!   The file is reported as `keep-chapters` and left alone: a synthesized
//!   list must never replace one the publisher shipped.
//! * **Only the `chpl` list is written on MP4**, never the chapter *track*. A
//!   chapter track is a media track; removing or rewriting one is structural
//!   surgery on the container, and the marks written here are the very marks
//!   just read from it, so the two mechanisms cannot disagree. `chpl` is also
//!   the mechanism every player that reads only one of them reads — the same
//!   precedence [`stump_media::audio`] applies when probing.
//!
//! Every time value in a plan is **file-relative**: a probe reports marks in
//! publication milliseconds, but a `CHAP` frame is about its own file, so the
//! marks that fall inside one file are rebased onto it.

use std::{
	fs::File,
	io::{Seek, SeekFrom},
	path::{Path, PathBuf},
	time::Duration,
};

use id3::TagLike;
use serde::{Deserialize, Serialize};
use stump_media::{
	audio::{self, ChapterSource, ProbedAudio, ProbedChapter, ProbedTrack},
	ContentType, PathUtils,
};

use crate::{
	audio_report::publications, util::write_atomic, Action, Plan, ProgressSink, Report,
	Severity, Tool, ToolError, ToolInput, ToolResult, Warning,
};

const ID: &str = "audio-chapters";

/// Marks will be embedded into this file.
const KIND_WRITE: &str = "write-chapters";
/// The file already carries marks a publisher authored; nothing is written.
const KIND_KEEP: &str = "keep-chapters";

/// Nero chapter list (`moov/udta/chpl`), for `.m4b`/`.m4a`.
const CHPL: &str = "chpl";
/// ID3v2 `CHAP`/`CTOC` frames, for `.mp3`.
const ID3: &str = "id3";

/// The `CHAP` element id of chapter `n`, and the one `CTOC` that lists them.
const TOC_ELEMENT: &str = "toc";

/// ID3v2 `CHAP`: "if these bytes are all set to 0xFF they should be ignored",
/// i.e. this chapter is addressed by time only.
const NO_OFFSET: u32 = 0xffff_ffff;

/// Embeds chapter marks into the files of an audiobook.
pub struct AudioChapters;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioChaptersOptions {
	/// Rewrite marks a publisher already authored. Off by default: a
	/// synthesized chapter list must never silently replace a real one.
	pub force: bool,
	/// Name a per-file mark after the file stem even when the file carries a
	/// title tag. With the default (`false`) the tag wins and the stem is the
	/// fallback.
	pub titles_from_filename: bool,
}

/// One mark to write, in **file-relative** milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Mark {
	title: String,
	start_ms: i64,
	#[serde(skip_serializing_if = "Option::is_none")]
	end_ms: Option<i64>,
}

/// The per-file plan carried by an action's `detail`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct ChapterDetail {
	chapters: usize,
	/// The snake_case [`ChapterSource`] the marks came from, i.e. what the
	/// embedded list will be *derived* from.
	source: String,
	/// The file's own provenance before the write.
	existing: String,
	/// The mark titles as one line. The CLI table can only render scalar
	/// detail fields, and a chapter tool whose titles are invisible without
	/// `--json` is a tool nobody can review before applying.
	titles: String,
	/// Which mechanism [`AudioChapters::apply`] writes: [`CHPL`] or [`ID3`].
	#[serde(skip_serializing_if = "String::is_empty")]
	container: String,
	/// Exactly what is written, file-relative. A plan is self-contained: the
	/// dry run the user approved is what runs, minutes later if need be.
	marks: Vec<Mark>,
}

impl Tool for AudioChapters {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Embed an audiobook's chapter marks into its own files as an MP4 chpl list or ID3v2 CHAP/CTOC frames"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<AudioChaptersOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}

		let mut plan = Plan::new(ID);
		let targets = publications(&input.paths, &mut plan)?;
		if targets.is_empty() {
			plan.warn(
				Warning::new("no-audio", "no audio publications in the given paths")
					.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		for path in targets {
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
			plan_publication(&mut plan, &probed, &options)?;
		}

		Ok(plan)
	}

	fn apply(
		&self,
		plan: &Plan,
		sink: &mut dyn ProgressSink,
	) -> Result<Report, ToolError> {
		plan.expect_tool(ID)?;

		let mut report = Report::for_plan(plan);
		let total = plan.actions.len();
		for (index, action) in plan.actions.iter().enumerate() {
			let Some(target) = action.source.clone() else {
				report.skipped(action.clone(), "the action names no file");
				continue;
			};
			sink.progress(index + 1, total, &target.to_string_lossy());

			if action.kind == KIND_KEEP {
				report.skipped(
					action.clone(),
					"the file already carries publisher chapter marks",
				);
				continue;
			}
			if action.kind != KIND_WRITE {
				report.skipped(
					action.clone(),
					format!("unknown action kind {:?}", action.kind),
				);
				continue;
			}
			// A plan can be minutes old, and only the files it names may be
			// touched — so the file it names is re-checked, and nothing else
			// is ever opened for writing.
			if !target.is_file() {
				report.skipped(action.clone(), "the file no longer exists");
				continue;
			}

			let detail = serde_json::from_value::<ChapterDetail>(action.detail.clone())?;
			match write_marks(&target, &detail) {
				Ok(()) => report.applied(action.clone()),
				Err(error) => report.skipped(action.clone(), error.to_string()),
			}
		}

		Ok(report)
	}
}

/// Plan one publication, file by file.
///
/// The unit is the *file*, not the book: a folder audiobook is many
/// containers, each of which can only carry the marks that fall inside it.
fn plan_publication(
	plan: &mut Plan,
	probed: &ProbedAudio,
	options: &AudioChaptersOptions,
) -> Result<(), ToolError> {
	let per_track = probed.chapter_source == ChapterSource::PerTrack;
	let mut offset = 0_i64;

	for track in &probed.tracks {
		let start = offset;
		offset += track.duration_ms;
		let marks = marks_in(probed, track, start, offset, per_track, options);

		if marks.is_empty() {
			plan.warn(
				Warning::new("no-chapters", "no chapter marks to embed").at(&track.path),
			);
			continue;
		}

		let existing = existing_source(probed, track)?;
		let titles = marks
			.iter()
			.map(|mark| mark.title.as_str())
			.collect::<Vec<_>>()
			.join(", ");

		if is_authored(existing) && !options.force {
			plan.push(Action::new(KIND_KEEP).with_source(&track.path).with_detail(
				serde_json::to_value(ChapterDetail {
					chapters: marks.len(),
					source: existing.to_string(),
					existing: existing.to_string(),
					titles,
					container: String::new(),
					marks: Vec::new(),
				})?,
			));
			continue;
		}

		let Some(container) = container_of(&track.path) else {
			plan.warn(
				Warning::new(
					"unsupported-container",
					"chapter marks can only be embedded in .m4b/.m4a (Nero chpl) and .mp3 (ID3v2 CHAP/CTOC); refusing to rewrite this container",
				)
				.at(&track.path)
				.with_severity(Severity::Error),
			);
			continue;
		};

		plan.push(
			Action::new(KIND_WRITE)
				.with_source(&track.path)
				.with_detail(serde_json::to_value(ChapterDetail {
					chapters: marks.len(),
					source: probed.chapter_source.to_string(),
					existing: existing.to_string(),
					titles,
					container: container.to_string(),
					marks,
				})?),
		);
	}

	Ok(())
}

/// The publication marks that fall inside one file, rebased onto it.
///
/// A mark belongs to the file its *start* is in; an end that runs past the
/// file is clamped to it. A chapter can only be written into the file
/// playback actually reaches it in.
fn marks_in(
	probed: &ProbedAudio,
	track: &ProbedTrack,
	start: i64,
	end: i64,
	per_track: bool,
	options: &AudioChaptersOptions,
) -> Vec<Mark> {
	let inside = probed
		.chapters
		.iter()
		.filter(|chapter| chapter.start_ms >= start && chapter.start_ms < end)
		.collect::<Vec<_>>();

	inside
		.iter()
		.enumerate()
		.map(|(index, chapter)| Mark {
			title: title_for(chapter, track, index, per_track, options),
			start_ms: chapter.start_ms - start,
			end_ms: chapter
				.end_ms
				.map(|end_ms| (end_ms - start).min(track.duration_ms)),
		})
		.collect()
}

/// A mark's title.
///
/// A mark the probe synthesized from the file list is named after its file:
/// the file's own title tag by default (which is what the probe already
/// resolved, falling back to the stem), or the stem when
/// `titles_from_filename` is set. A mark that came out of a container keeps
/// the publisher's title, and an untitled one becomes `Chapter N` rather than
/// an empty string, because a nameless mark is a blank row in every player.
fn title_for(
	chapter: &ProbedChapter,
	track: &ProbedTrack,
	index: usize,
	per_track: bool,
	options: &AudioChaptersOptions,
) -> String {
	let stem = track
		.path
		.file_stem()
		.map(|stem| stem.to_string_lossy().to_string())
		.unwrap_or_default();
	let title = chapter
		.title
		.clone()
		.filter(|title| !title.trim().is_empty());

	if per_track {
		if options.titles_from_filename {
			return stem;
		}
		return title.unwrap_or(stem);
	}
	title.unwrap_or_else(|| format!("Chapter {}", index + 1))
}

/// The file's *own* chapter provenance, which is what decides whether a write
/// would overwrite a publisher's work.
///
/// A single-container publication *is* the file, so the publication's
/// provenance is the file's. For a folder book the aggregate is not enough —
/// it names the mechanism of whichever part carried marks first — except in
/// the one case where it is exact: [`ChapterSource::PerTrack`] is recorded
/// only when no part carried a mark at all, so every file's own provenance is
/// [`ChapterSource::None`] and no re-probe can say otherwise.
fn existing_source(
	probed: &ProbedAudio,
	track: &ProbedTrack,
) -> Result<ChapterSource, ToolError> {
	if probed.tracks.len() == 1 {
		return Ok(probed.chapter_source);
	}
	if probed.chapter_source == ChapterSource::PerTrack {
		return Ok(ChapterSource::None);
	}
	Ok(audio::probe_file(&track.path)?.chapter_source)
}

/// True for a provenance a publisher wrote into the container.
/// [`ChapterSource::PerTrack`] is Stump's own synthesis and
/// [`ChapterSource::None`] is the absence of marks: neither is authored, so
/// neither is protected from a write.
fn is_authored(source: ChapterSource) -> bool {
	matches!(
		source,
		ChapterSource::Mp4Chpl
			| ChapterSource::Mp4ChapterTrack
			| ChapterSource::Id3Chap
			| ChapterSource::VorbisComment
	)
}

/// The mechanism a file's extension can carry, or `None` for a container this
/// tool refuses to rewrite. Extensions are read through
/// [`PathUtils::naive_content_type`], so the tool accepts exactly the audio
/// types the scanner recognises.
fn container_of(path: &Path) -> Option<&'static str> {
	match path.naive_content_type() {
		ContentType::M4B | ContentType::M4A => Some(CHPL),
		ContentType::MP3 => Some(ID3),
		_ => None,
	}
}

fn write_marks(target: &Path, detail: &ChapterDetail) -> ToolResult<()> {
	match detail.container.as_str() {
		CHPL => write_chpl(target, &detail.marks),
		ID3 => write_id3(target, &detail.marks),
		other => Err(ToolError::Invalid(format!(
			"unknown chapter container {other:?}"
		))),
	}
}

/// Write a Nero `chpl` chapter list, leaving the metadata item list and any
/// chapter track exactly as they are (`WriteConfig::NONE` plus the one flag).
fn write_chpl(target: &Path, marks: &[Mark]) -> ToolResult<()> {
	let read = mp4ameta::ReadConfig {
		read_chapter_list: true,
		..mp4ameta::ReadConfig::NONE
	};
	let mut tag = mp4ameta::Tag::read_with_path(target, &read)?;

	let list = tag.chapter_list_mut();
	list.clear();
	list.extend(marks.iter().map(|mark| {
		mp4ameta::Chapter::new(
			Duration::from_millis(mark.start_ms.max(0) as u64),
			mark.title.clone(),
		)
	}));

	let write = mp4ameta::WriteConfig {
		write_chapter_list: true,
		..mp4ameta::WriteConfig::NONE
	};
	edit_atomic(target, |file| Ok(tag.write_with(file, &write)?))
}

/// Write ID3v2 `CHAP` frames plus the one top-level `CTOC` that lists them.
///
/// A `CHAP` frame without a `CTOC` is a chapter no reader has been told about,
/// and the existing frames are removed rather than added to: two chapter lists
/// in one file is a file two readers disagree about.
fn write_id3(target: &Path, marks: &[Mark]) -> ToolResult<()> {
	let mut tag = match id3::Tag::read_from_path(target) {
		Ok(tag) => tag,
		// A part with no tag at all is the common case for a folder book.
		Err(error) if matches!(error.kind, id3::ErrorKind::NoTag) => id3::Tag::new(),
		Err(error) => return Err(error.into()),
	};
	tag.remove("CHAP");
	tag.remove("CTOC");

	let mut elements = Vec::with_capacity(marks.len());
	for (index, mark) in marks.iter().enumerate() {
		let element_id = format!("chp{:03}", index + 1);
		let mut chapter = id3::frame::Chapter {
			element_id: element_id.clone(),
			start_time: milliseconds(mark.start_ms),
			end_time: milliseconds(mark.end_ms.unwrap_or(mark.start_ms)),
			start_offset: NO_OFFSET,
			end_offset: NO_OFFSET,
			frames: Vec::new(),
		};
		// `TIT2` inside a `CHAP` is the chapter name; that is what every
		// reader, including the demuxer behind `stump_media::audio`, reads.
		chapter.set_title(mark.title.clone());
		tag.add_frame(chapter);
		elements.push(element_id);
	}

	tag.add_frame(id3::frame::TableOfContents {
		element_id: TOC_ELEMENT.to_string(),
		top_level: true,
		ordered: true,
		elements,
		frames: Vec::new(),
	});

	edit_atomic(target, |file| {
		Ok(tag.write_to_file(file, id3::Version::Id3v24)?)
	})
}

/// Mutate `target` through the sibling temp file [`write_atomic`] renames into
/// place: the original bytes are copied in first, so the tagger edits the copy
/// and the original is untouched until the new bytes are complete.
///
/// Both taggers rewrite a container *in place* — they need the media bytes
/// they are not changing — which is why this stages a copy instead of writing
/// a fresh file.
fn edit_atomic<F>(target: &Path, edit: F) -> ToolResult<()>
where
	F: FnOnce(&mut File) -> ToolResult<()>,
{
	let permissions = std::fs::metadata(target)
		.map(|meta| meta.permissions())
		.ok();

	write_atomic(target, |file| {
		let mut original = File::open(target)?;
		std::io::copy(&mut original, file)?;
		file.seek(SeekFrom::Start(0))?;
		edit(file)
	})?;

	// `write_atomic` stages through a 0600 temp file, so an in-place rewrite
	// has to put the library file's own mode back.
	if let Some(permissions) = permissions {
		let _ = std::fs::set_permissions(target, permissions);
	}

	Ok(())
}

fn milliseconds(value: i64) -> u32 {
	u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NoopProgress;
	use std::fs;
	use tempfile::TempDir;

	fn fixture(name: &str) -> PathBuf {
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("../media/integration-tests/data/audio")
			.join(name)
	}

	/// Copy a fixture file or folder into a temp dir. The fixtures are shared
	/// by the whole workspace: a tool that mutates one would poison every
	/// other test in the tree.
	fn staged(name: &str) -> (TempDir, PathBuf) {
		let dir = TempDir::new().expect("temp dir");
		let source = fixture(name);
		let target = dir.path().join(name);

		if source.is_dir() {
			fs::create_dir_all(&target).expect("create folder");
			for entry in fs::read_dir(&source).expect("read fixture folder") {
				let entry = entry.expect("fixture entry").path();
				let name = entry.file_name().expect("entry name");
				fs::copy(&entry, target.join(name)).expect("copy part");
			}
		} else {
			fs::copy(&source, &target).expect("copy fixture");
		}

		(dir, target)
	}

	fn plan_for(path: &Path, options: serde_json::Value) -> Plan {
		AudioChapters
			.plan(&ToolInput::new(vec![path.to_path_buf()]).with_options(options))
			.expect("plan succeeds")
	}

	fn detail_of(action: &Action) -> ChapterDetail {
		serde_json::from_value(action.detail.clone()).expect("decode detail")
	}

	fn titles(probed: &ProbedAudio) -> Vec<String> {
		probed
			.chapters
			.iter()
			.map(|chapter| chapter.title.clone().unwrap_or_default())
			.collect()
	}

	/// A publisher's chapter list is never replaced by a derived one: the
	/// file is reported as `keep-chapters` and only `force` turns it into a
	/// write. Planning writes nothing either way.
	#[test]
	fn publisher_marks_are_kept_unless_force_is_set() {
		let path = fixture("chapters-chpl.m4b");
		let before = fs::read(&path).expect("read fixture");

		let plan = plan_for(&path, serde_json::json!({}));
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.actions[0].kind, KIND_KEEP);
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.existing, "mp4_chpl");
		assert_eq!(detail.titles, "Opening, Middle, Closing");
		assert_eq!(detail.chapters, 3);
		assert!(detail.marks.is_empty(), "a keep writes nothing");

		let plan = plan_for(&path, serde_json::json!({"force": true}));
		assert_eq!(plan.actions.len(), 1);
		assert_eq!(plan.actions[0].kind, KIND_WRITE);
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.container, CHPL);
		assert_eq!(detail.chapters, 3);
		assert_eq!(
			detail
				.marks
				.iter()
				.map(|mark| mark.start_ms)
				.collect::<Vec<_>>(),
			vec![0, 2000, 4000]
		);

		assert_eq!(
			fs::read(&path).expect("re-read fixture"),
			before,
			"plan must not write a byte"
		);
	}

	/// A QuickTime chapter track is not portable; a `chpl` list is. Forcing
	/// the write leaves a file whose marks and titles read back through the
	/// probe as [`ChapterSource::Mp4Chpl`].
	#[test]
	fn a_chapter_track_becomes_a_portable_chpl_list() {
		let (_dir, path) = staged("chapters-track.m4b");
		let probed = audio::probe_file(&path).expect("probe the copy");
		assert_eq!(probed.chapter_source, ChapterSource::Mp4ChapterTrack);

		let plan = plan_for(&path, serde_json::json!({"force": true}));
		let report = AudioChapters
			.apply(&plan, &mut NoopProgress)
			.expect("apply succeeds");
		assert_eq!(report.applied.len(), 1);
		assert!(report.skipped.is_empty(), "{:?}", report.skipped);

		let probed = audio::probe_file(&path).expect("re-probe the copy");
		assert_eq!(probed.chapter_source, ChapterSource::Mp4Chpl);
		assert_eq!(
			titles(&probed),
			vec!["Opening", "Middle", "Closing"],
			"the publisher's titles survive the mechanism change"
		);
		assert_eq!(
			probed
				.chapters
				.iter()
				.map(|chapter| chapter.start_ms)
				.collect::<Vec<_>>(),
			vec![0, 2000, 4000]
		);
		assert_eq!(
			probed.title.as_deref(),
			Some("Silent Test Book"),
			"writing only the chapter list leaves the metadata items alone"
		);
	}

	/// The flagship case: a folder book whose marks Stump only synthesized
	/// gains real ID3v2 chapters, so the chapter list becomes a property of
	/// the files instead of a property of Stump's database.
	#[test]
	fn synthesized_folder_marks_become_real_id3_chapters() {
		let (_dir, folder) = staged("folder-filename");
		let probed = audio::probe_folder(&folder).expect("probe the copy");
		assert_eq!(probed.chapter_source, ChapterSource::PerTrack);

		let plan = plan_for(&folder, serde_json::json!({}));
		assert_eq!(plan.actions.len(), 3, "one action per file");
		assert!(plan.actions.iter().all(|action| action.kind == KIND_WRITE));
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.container, ID3);
		assert_eq!(detail.source, "per_track");
		assert_eq!(detail.existing, "none");
		assert_eq!(detail.titles, "01 - Part 1");
		assert_eq!(
			detail.marks[0].start_ms, 0,
			"a mark is written file-relative, not publication-relative"
		);

		let report = AudioChapters
			.apply(&plan, &mut NoopProgress)
			.expect("apply succeeds");
		assert_eq!(report.applied.len(), 3);
		assert!(report.skipped.is_empty(), "{:?}", report.skipped);

		let part = audio::probe_file(&folder.join("02 - Part 2.mp3"))
			.expect("re-probe part two");
		assert_eq!(part.chapter_source, ChapterSource::Id3Chap);
		assert_eq!(titles(&part), vec!["02 - Part 2"]);
		assert_eq!(part.chapters[0].start_ms, 0);

		// The publication's own provenance is no longer a synthesis.
		let probed = audio::probe_folder(&folder).expect("re-probe the book");
		assert_eq!(probed.chapter_source, ChapterSource::Id3Chap);
		assert_eq!(
			titles(&probed),
			vec!["01 - Part 1", "02 - Part 2", "03 - Part 3"]
		);
		assert_eq!(
			probed
				.chapters
				.iter()
				.map(|chapter| chapter.start_ms)
				.collect::<Vec<_>>(),
			vec![0, 2063, 4126],
			"the re-probed marks are publication-relative again"
		);
	}

	/// A per-file mark is named after the file's title tag by default and
	/// after the file stem on request — the two disagree exactly when a
	/// publisher tagged parts whose filenames sort differently.
	#[test]
	fn titles_from_filename_overrides_a_track_title_tag() {
		let folder = fixture("folder-tracknum");

		let plan = plan_for(&folder, serde_json::json!({}));
		let tagged = plan
			.actions
			.iter()
			.map(|action| detail_of(action).titles)
			.collect::<Vec<_>>();
		assert_eq!(tagged, vec!["Opening", "Middle", "Closing"]);

		let plan = plan_for(&folder, serde_json::json!({"titles_from_filename": true}));
		let stems = plan
			.actions
			.iter()
			.map(|action| detail_of(action).titles)
			.collect::<Vec<_>>();
		assert_eq!(
			stems,
			vec!["zulu", "mike", "alpha"],
			"TRCK order beats filename order, so the stems are not sorted"
		);
	}

	/// Rewriting an Ogg comment header means rewriting every following page's
	/// granule bookkeeping. The tool refuses the whole family instead of
	/// producing a file no player opens.
	#[test]
	fn an_unsupported_container_is_refused_with_an_error() {
		let (_dir, path) = staged("chapters-vorbis.opus");
		let before = fs::read(&path).expect("read the copy");

		// Without `force` the publisher's marks are simply kept.
		let plan = plan_for(&path, serde_json::json!({}));
		assert_eq!(plan.actions[0].kind, KIND_KEEP);

		let plan = plan_for(&path, serde_json::json!({"force": true}));
		assert!(plan.actions.is_empty(), "{:?}", plan.actions);
		let warning = plan
			.warnings
			.iter()
			.find(|warning| warning.code == "unsupported-container")
			.expect("unsupported-container warning");
		assert_eq!(warning.severity, Severity::Error);
		assert_eq!(warning.path.as_deref(), Some(path.as_path()));

		AudioChapters
			.apply(&plan, &mut NoopProgress)
			.expect("apply succeeds");
		assert_eq!(
			fs::read(&path).expect("re-read the copy"),
			before,
			"a refused container must not be rewritten"
		);
	}

	/// `apply` performs exactly the actions of the plan it was handed, so a
	/// caller who applies one action of a three-part book must find the other
	/// two parts byte-identical afterwards. A file with no chapters of its
	/// own is reported and never written.
	#[test]
	fn apply_never_touches_a_file_the_plan_did_not_name() {
		let (_dir, folder) = staged("folder-filename");
		let target = folder.join("01 - Part 1.mp3");
		let siblings = ["02 - Part 2.mp3", "03 - Part 3.mp3"]
			.map(|name| folder.join(name))
			.map(|path| (path.clone(), fs::read(path).expect("read a sibling")));

		let mut plan = plan_for(&folder, serde_json::json!({}));
		assert_eq!(plan.actions.len(), 3);
		plan.actions
			.retain(|action| action.source.as_deref() == Some(&target));

		let report = AudioChapters
			.apply(&plan, &mut NoopProgress)
			.expect("apply succeeds");
		assert_eq!(report.applied.len(), 1);
		for (path, before) in &siblings {
			assert_eq!(
				&fs::read(path).expect("re-read a sibling"),
				before,
				"a part the plan never named must not move"
			);
		}

		let probed = audio::probe_file(&target).expect("re-probe the target");
		assert_eq!(probed.chapter_source, ChapterSource::Id3Chap);

		// That part now carries marks of its own, so planning the book again
		// is a keep for it, while its siblings still have none to embed.
		let plan = plan_for(&folder, serde_json::json!({}));
		let kinds = plan
			.actions
			.iter()
			.map(|action| action.kind.as_str())
			.collect::<Vec<_>>();
		assert_eq!(kinds, vec![KIND_KEEP]);
		assert_eq!(
			plan.warnings
				.iter()
				.filter(|warning| warning.code == "no-chapters")
				.count(),
			2,
			"the two untouched parts have no marks to embed"
		);
	}

	/// A plan built by another tool can never be applied by this one, and an
	/// action naming a file that has since gone is skipped with the reason
	/// rather than aborting the run.
	#[test]
	fn apply_refuses_a_foreign_plan_and_skips_a_vanished_file() {
		let error = AudioChapters
			.apply(&Plan::new("missing-sequence"), &mut NoopProgress)
			.expect_err("a foreign plan is refused");
		assert!(matches!(error, ToolError::PlanMismatch { .. }), "{error}");

		let (dir, path) = staged("chapters-track.m4b");
		let plan = plan_for(&path, serde_json::json!({"force": true}));
		fs::remove_file(&path).expect("remove the target");

		let report = AudioChapters
			.apply(&plan, &mut NoopProgress)
			.expect("apply succeeds");
		assert!(report.applied.is_empty());
		assert_eq!(report.skipped.len(), 1);
		assert_eq!(report.skipped[0].1, "the file no longer exists");
		drop(dir);
	}
}
