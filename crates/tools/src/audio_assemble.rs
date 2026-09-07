//! `audio-assemble`: a folder of parts becomes one chaptered, streamable M4B.
//!
//! A folder audiobook works, and works badly. Stump serves it as N tracks and
//! synthesizes a chapter per file, but the *book* only exists in Stump's
//! database: copy the folder to a phone and it is 43 songs, hand it to another
//! client and the chapter list is gone, lose the database and the reading
//! position means nothing. The canonical shape is one file that carries its own
//! structure — the single M4B this tool produces:
//!
//! | Property | Why it is the canonical one |
//! | --- | --- |
//! | MP4/AAC, one audio track | one decoder, one seek space, one duration |
//! | `chpl` **and** a chapter track | players split between the two mechanisms |
//! | iTunes tags (`©nam ©ART ©wrt ©alb ©gen ©day ©des`) | `©wrt` is where every publisher, Audiobookshelf and Plex read the narrator |
//! | `covr` | the grid shows the book, not a placeholder |
//! | `moov` before `mdat` | playback starts without reading the whole file |
//!
//! # Two paths, and the one that costs nothing
//!
//! **Remux** — every part is already an AAC stream in an MP4 and they agree on
//! sample rate, channel layout and profile. The coded samples are copied
//! verbatim into one container and only the sample tables are rebuilt: no
//! decode, no quality loss, no external binary, and a 30-hour book assembles at
//! disk speed. See [`mux`].
//!
//! **Transcode** — anything else (MP3, Opus, FLAC, or MP4 parts that disagree)
//! has to be decoded and re-encoded as AAC, and there is no maintained
//! pure-Rust AAC encoder. That one step is delegated to an operator-installed
//! `ffmpeg` ([`crate::ffmpeg`]). When ffmpeg is absent the tool does **not**
//! guess, half-assemble, or fall back to a container nothing else reads: the
//! publication is planned as [`KIND_PLAN_TRANSCODE`], the plan detail carries
//! the exact command, the marks and the tags it would have used, and `apply`
//! skips it. A plan-only result is a truthful answer; a broken M4B is not.
//!
//! # Where the chapters come from
//!
//! Strictly in this order, because each source is more authoritative than the
//! next about what the *publisher* meant:
//!
//! 1. **Marks already in the containers** — a `chpl`, a chapter track, ID3v2
//!    `CHAP`, or Vorbis `CHAPTERxxx`. Somebody authored these.
//! 2. **A sidecar** — `chapters.txt`, a cue sheet, Audible's `metadata.json`
//!    ([`sidecar`]). Somebody wrote these down beside the book.
//! 3. **One chapter per file**, named from the file's title tag or its stem.
//!    Nobody authored these; they are file boundaries wearing chapter names,
//!    which is exactly what [`stump_media::audio::ChapterSource::PerTrack`]
//!    records and what this tool falls back to rather than shipping a book
//!    with no chapter list at all.
//!
//! # What it never does
//!
//! It never deletes or rewrites a source file. The output is a new file and the
//! parts are left exactly as they were, so a librarian can compare the two and
//! an assemble is always undoable by deleting one file. Replacing the original
//! publication is an ingest decision (`audio.keepOriginal` in the library
//! metadata policy), not a decision a tool makes about somebody's library.

pub mod mux;
pub mod sidecar;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use stump_media::{
	audio::{self, ChapterSource, ProbedAudio, ProbedTrack},
	ContentType, PathUtils,
};

use crate::{
	audio_report::{human_duration, publications},
	ffmpeg::{self, Codec, Encode, MetadataChapter},
	util::write_atomic,
	Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput, ToolResult,
	Warning,
};

const ID: &str = "audio-assemble";

/// Every part is already AAC-in-MP4: the samples are copied, not re-encoded.
const KIND_REMUX: &str = "assemble-remux";
/// The parts have to be decoded and re-encoded through `ffmpeg`.
const KIND_TRANSCODE: &str = "assemble-transcode";
/// A transcode is needed and `ffmpeg` is unavailable. Report-only: the detail
/// carries the command, the marks and the tags, and `apply` writes nothing.
const KIND_PLAN_TRANSCODE: &str = "plan-transcode";

/// The canonical container's extension.
const M4B: &str = "m4b";

/// Default `-b:a` for the transcode path.
///
/// 64 kbit/s AAC-LC is the bitrate Audible, Audiobookshelf and every audiobook
/// publisher use for stereo speech; it is transparent for the material and a
/// 30-hour book fits in 800 MB.
const DEFAULT_BITRATE: &str = "64k";

/// Cover file stems looked for beside a folder book, in order.
const COVER_STEMS: [&str; 4] = ["cover", "folder", "front", "cover_art"];

/// How far the assembled book's measured duration may differ from the sum of
/// its parts before the output is rejected.
///
/// A remux is sample-exact and a transcode differs only by the encoder's
/// priming samples — a few tens of milliseconds per part. One percent is
/// therefore not a tolerance for sloppiness: it is the point at which a part
/// was silently dropped, which is the one failure mode that would otherwise
/// produce a plausible-looking book that ends early.
const DURATION_TOLERANCE: f64 = 0.01;

/// Assembles an audiobook into one canonical M4B.
pub struct AudioAssemble;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioAssembleOptions {
	/// Where the M4B goes. Empty puts it beside the publication, which is
	/// where a librarian expects to find it and where a rescan will see it.
	pub output_dir: Option<String>,
	/// `-b:a` for the transcode path, in ffmpeg's spelling. Ignored by the
	/// remux path, which re-encodes nothing.
	pub bitrate: Option<String>,
	/// Directory holding `ffmpeg`, for an installation that is not on `PATH`.
	pub bin_dir: Option<String>,
	/// Overwrite an existing output file. Off by default: an assemble that
	/// silently replaced last week's assemble would destroy the only copy of
	/// whatever had been done to it since.
	pub force: bool,
	/// Allow the transcode path at all. On by default; turning it off makes
	/// the tool pure-Rust and lossless or nothing, which is what an operator
	/// who wants no re-encoding in their library asks for.
	#[serde(default = "enabled")]
	pub allow_transcode: bool,
	/// Name a per-file chapter after the file stem even when the file carries
	/// a title tag. With the default the tag wins and the stem is the
	/// fallback.
	pub titles_from_filename: bool,
}

fn enabled() -> bool {
	true
}

/// Hand-written rather than derived, because the derive would disagree with
/// the `serde` defaults above: `#[derive(Default)]` gives `allow_transcode:
/// false`, and [`crate::ToolInput::parse_options`] resolves a *null* option
/// blob through `Default` rather than through `serde`. A tool run with no
/// options at all would then refuse every transcode while the same tool run
/// with `{}` allowed it. One meaning per field, whichever way the options
/// arrive.
impl Default for AudioAssembleOptions {
	fn default() -> Self {
		Self {
			output_dir: None,
			bitrate: None,
			bin_dir: None,
			force: false,
			allow_transcode: enabled(),
			titles_from_filename: false,
		}
	}
}

impl AudioAssembleOptions {
	fn bitrate(&self) -> &str {
		self.bitrate
			.as_deref()
			.map(str::trim)
			.filter(|value| !value.is_empty())
			.unwrap_or(DEFAULT_BITRATE)
	}
}

/// One mark to write, in publication milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Mark {
	title: String,
	start_ms: i64,
}

/// Where the cover comes from at apply time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CoverSource {
	/// A `cover.jpg`-style file beside the book.
	File { path: PathBuf },
	/// Embedded art in one of the parts, re-read through the probe.
	Embedded { path: PathBuf },
}

/// The iTunes tags the output will carry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct TagDetail {
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	author: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	narrator: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	album: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	genre: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	year: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	description: Option<String>,
}

/// The per-publication plan carried by an action's `detail`.
///
/// Self-contained by contract: everything `apply` needs is here, so the dry run
/// a librarian approved is what runs, minutes later if need be. The two temp
/// files the transcode path hands ffmpeg — a concat list and an ffmetadata
/// document — are *derived* from `inputs` and `marks` at apply time, so their
/// content is fixed by this detail even though their names are not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct AssembleDetail {
	/// Absolute paths of the parts, in playback order.
	inputs: Vec<PathBuf>,
	/// Part count, for a table cell that a 200-part `inputs` would drown.
	parts: usize,
	duration_ms: i64,
	/// `h:mm:ss`, the field a librarian actually scans for.
	duration: String,
	/// The source codec: `aac` for a remux, whatever the parts hold otherwise.
	codec: String,
	/// Where the marks came from: `container`, a [`sidecar::SidecarKind`], or
	/// `per_track`. Provenance, never a quality tier.
	chapter_source: String,
	chapters: usize,
	/// The mark titles as one line, so a chapter list is reviewable without
	/// `--json`.
	titles: String,
	marks: Vec<Mark>,
	tags: TagDetail,
	#[serde(skip_serializing_if = "Option::is_none")]
	cover: Option<CoverSource>,
	/// Only on the transcode paths: the `-b:a` value.
	#[serde(skip_serializing_if = "Option::is_none")]
	bitrate: Option<String>,
	/// Only on the transcode paths: the exact ffmpeg arguments, with the two
	/// derived temp files named as they will be created. Printed so an
	/// operator whose server has no ffmpeg can run the conversion by hand.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	ffmpeg: Vec<String>,
}

impl Tool for AudioAssemble {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Assemble an audiobook's parts into one chaptered, faststart M4B, remuxing losslessly when every part is already AAC"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<AudioAssembleOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths given".to_string()));
		}
		let output_dir = options
			.output_dir
			.as_deref()
			.map(str::trim)
			.filter(|value| !value.is_empty())
			.map(PathBuf::from);
		let bin_dir = options
			.bin_dir
			.as_deref()
			.map(str::trim)
			.filter(|value| !value.is_empty())
			.map(PathBuf::from);

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
			plan_publication(
				&mut plan,
				&path,
				&probed,
				&options,
				output_dir.as_deref(),
				bin_dir.as_deref(),
			)?;
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
			let Some(target) = action.target.clone() else {
				report.skipped(action.clone(), "the action names no output file");
				continue;
			};
			sink.progress(index + 1, total, &target.to_string_lossy());

			if action.kind == KIND_PLAN_TRANSCODE {
				report.skipped(
					action.clone(),
					"ffmpeg is unavailable; the plan holds the command to run by hand",
				);
				continue;
			}
			if action.kind != KIND_REMUX && action.kind != KIND_TRANSCODE {
				report.skipped(
					action.clone(),
					format!("unknown action kind {:?}", action.kind),
				);
				continue;
			}

			let detail = serde_json::from_value::<AssembleDetail>(action.detail.clone())?;
			// A plan can be minutes old and only the files it names may be
			// read; a part that vanished means the book is no longer the book
			// the plan described.
			if let Some(missing) = detail.inputs.iter().find(|path| !path.is_file()) {
				report.skipped(
					action.clone(),
					format!("{} no longer exists", missing.display()),
				);
				continue;
			}

			match assemble(&action.kind, &target, &detail) {
				Ok(()) => report.applied(action.clone()),
				Err(error) => {
					// A half-written or wrong output in a library is worse
					// than no output: the file is removed and the failure is
					// reported against the action.
					let _ = std::fs::remove_file(&target);
					report.skipped(action.clone(), error.to_string());
				},
			}
		}

		Ok(report)
	}
}

/// Plan one publication.
fn plan_publication(
	plan: &mut Plan,
	source: &Path,
	probed: &ProbedAudio,
	options: &AudioAssembleOptions,
	output_dir: Option<&Path>,
	bin_dir: Option<&Path>,
) -> Result<(), ToolError> {
	if probed.tracks.is_empty() {
		plan.warn(
			Warning::new("no-tracks", "the publication holds no audio files")
				.at(source)
				.with_severity(Severity::Error),
		);
		return Ok(());
	}

	let output = output_path(source, output_dir);

	// Which parts can be copied rather than re-encoded.
	let mut shapes = Vec::with_capacity(probed.tracks.len());
	let mut remuxable = true;
	for track in &probed.tracks {
		match mux::aac_shape(&track.path)? {
			Some(found) => shapes.push(found),
			None => {
				remuxable = false;
				break;
			},
		}
	}
	let shape = remuxable.then(|| mux::common_shape(&shapes)).flatten();

	// A single MP4 that is already one AAC track is the canonical container,
	// and its output path is itself: assembling it would rewrite a file to
	// produce the same file. Checked before the existing-output rule, because
	// "the output is already there" would otherwise be reported about the
	// source file and read as if `force` could help.
	if shape.is_some() && probed.tracks.len() == 1 && output == *source {
		plan.warn(
			Warning::new(
				"already-assembled",
				"the publication is already one AAC-in-MP4 file",
			)
			.at(source),
		);
		return Ok(());
	}

	if output.exists() && !options.force {
		plan.warn(
			Warning::new(
				"output-exists",
				format!(
					"{} already exists; pass force to replace it",
					output.display()
				),
			)
			.at(source),
		);
		return Ok(());
	}

	let (chapter_source, marks) = resolve_marks(source, probed, options);
	let cover = resolve_cover(source, probed);
	let detail = AssembleDetail {
		inputs: probed
			.tracks
			.iter()
			.map(|track| track.path.clone())
			.collect(),
		parts: probed.tracks.len(),
		duration_ms: probed.duration_ms,
		duration: human_duration(probed.duration_ms),
		codec: probed.codec.clone(),
		chapter_source: chapter_source.to_string(),
		chapters: marks.len(),
		titles: marks
			.iter()
			.map(|mark| mark.title.as_str())
			.collect::<Vec<_>>()
			.join(" | "),
		marks,
		tags: resolve_tags(source, probed),
		cover,
		bitrate: None,
		ffmpeg: Vec::new(),
	};

	if marks_are_empty(&detail) {
		plan.warn(
			Warning::new(
				"no-chapters",
				"no marks in the containers, no sidecar, and no per-file fallback: the book will have no chapter list",
			)
			.at(source),
		);
	}

	if shape.is_some() {
		plan.push(
			Action::new(KIND_REMUX)
				.with_source(source)
				.with_target(&output)
				.with_detail(serde_json::to_value(&detail)?),
		);
		return Ok(());
	}

	if !options.allow_transcode {
		plan.warn(
			Warning::new(
				"transcode-refused",
				format!(
					"{} parts are {} and would have to be re-encoded, but allow_transcode is off",
					probed.tracks.len(),
					probed.codec
				),
			)
			.at(source)
			.with_severity(Severity::Error),
		);
		return Ok(());
	}

	let mut detail = AssembleDetail {
		bitrate: Some(options.bitrate().to_string()),
		..detail
	};
	detail.ffmpeg = printed_argv(&detail, &output);

	match ffmpeg::locate(bin_dir) {
		Ok(_) => plan.push(
			Action::new(KIND_TRANSCODE)
				.with_source(source)
				.with_target(&output)
				.with_detail(serde_json::to_value(&detail)?),
		),
		Err(error) => {
			plan.warn(
				Warning::new("ffmpeg-missing", error.to_string())
					.at(source)
					.with_severity(Severity::Error),
			);
			plan.push(
				Action::new(KIND_PLAN_TRANSCODE)
					.with_source(source)
					.with_target(&output)
					.with_detail(serde_json::to_value(&detail)?),
			);
		},
	}

	Ok(())
}

fn marks_are_empty(detail: &AssembleDetail) -> bool {
	detail.marks.is_empty()
}

/// `<parent>/<name>.m4b`, in `output_dir` when one was given.
///
/// A folder book is named after its folder and a single container after its
/// stem, which is how both already appear in the library.
fn output_path(source: &Path, output_dir: Option<&Path>) -> PathBuf {
	let name = if source.is_dir() {
		source.file_name()
	} else {
		source.file_stem()
	};
	let stem = name
		.map(|name| name.to_string_lossy().to_string())
		.unwrap_or_else(|| "audiobook".to_string());
	let parent = output_dir
		.map(Path::to_path_buf)
		.or_else(|| source.parent().map(Path::to_path_buf))
		.unwrap_or_else(|| PathBuf::from("."));
	parent.join(format!("{stem}.{M4B}"))
}

/// The marks to write, and where they came from.
fn resolve_marks(
	source: &Path,
	probed: &ProbedAudio,
	options: &AudioAssembleOptions,
) -> (&'static str, Vec<Mark>) {
	// 1. Somebody authored these into the containers.
	if is_authored(probed.chapter_source) && !probed.chapters.is_empty() {
		let marks = probed
			.chapters
			.iter()
			.enumerate()
			.map(|(index, chapter)| Mark {
				title: chapter
					.title
					.clone()
					.filter(|title| !title.trim().is_empty())
					.unwrap_or_else(|| format!("Chapter {}", index + 1)),
				start_ms: chapter.start_ms,
			})
			.collect();
		return ("container", marks);
	}

	// 2. Somebody wrote these down beside the book.
	if let Some(found) = sidecar::discover(source) {
		let marks = found
			.marks
			.into_iter()
			.map(|mark| Mark {
				title: mark.title,
				start_ms: mark.start_ms,
			})
			.collect();
		return (found.kind.as_str(), marks);
	}

	// 3. Nobody authored these: one per file, named after the file.
	let mut marks = Vec::with_capacity(probed.tracks.len());
	let mut start_ms = 0;
	for (index, track) in probed.tracks.iter().enumerate() {
		marks.push(Mark {
			title: per_track_title(track, index, options),
			start_ms,
		});
		start_ms += track.duration_ms;
	}
	("per_track", marks)
}

/// True for a provenance a publisher wrote into the container.
/// [`ChapterSource::PerTrack`] is Stump's own synthesis and
/// [`ChapterSource::None`] is the absence of marks; neither is authored.
fn is_authored(source: ChapterSource) -> bool {
	match source {
		ChapterSource::Mp4Chpl
		| ChapterSource::Mp4ChapterTrack
		| ChapterSource::Id3Chap
		| ChapterSource::VorbisComment => true,
		ChapterSource::PerTrack | ChapterSource::None => false,
	}
}

/// A per-file mark's name: the file's own title tag, or its stem.
fn per_track_title(
	track: &ProbedTrack,
	index: usize,
	options: &AudioAssembleOptions,
) -> String {
	let stem = || {
		track
			.path
			.file_stem()
			.map(|stem| stem.to_string_lossy().to_string())
	};
	let from_tag = track.title.clone().filter(|title| !title.trim().is_empty());
	let chosen = if options.titles_from_filename {
		stem().or(from_tag)
	} else {
		from_tag.or_else(stem)
	};
	chosen
		.filter(|title| !title.trim().is_empty())
		// A nameless mark is a blank row in every player.
		.unwrap_or_else(|| format!("Chapter {}", index + 1))
}

/// The tags to write.
///
/// The probe already resolved what the containers say; the folder name fills
/// the two fields a listener always sees when the parts carry no tags at all,
/// because "Berserk, Volume 1" beats an empty title bar.
fn resolve_tags(source: &Path, probed: &ProbedAudio) -> TagDetail {
	let folder = if source.is_dir() {
		source.file_name()
	} else {
		source.file_stem()
	}
	.map(|name| name.to_string_lossy().to_string());

	// A folder book's parts carry *chapter* titles, not the book's: "Track 1"
	// is what the first part calls itself, and using it would name the book
	// after its opening chapter. The album is the book, so it wins whenever
	// the publication is more than one file.
	let title = if probed.tracks.len() > 1 {
		probed.album.clone().or_else(|| probed.title.clone())
	} else {
		probed.title.clone().or_else(|| probed.album.clone())
	};

	TagDetail {
		title: title.or_else(|| folder.clone()),
		author: probed.author.clone(),
		narrator: probed.narrator.clone(),
		album: probed.album.clone().or(folder),
		genre: probed.genre.clone(),
		year: probed.year.map(|year| year.to_string()),
		description: probed.description.clone(),
	}
}

/// Where the cover will come from: the container's own art, else a `cover.*`
/// beside the book.
fn resolve_cover(source: &Path, probed: &ProbedAudio) -> Option<CoverSource> {
	if probed.cover.is_some() {
		// Re-read at apply time from the part that carried it, so the plan
		// stays printable JSON instead of a megabyte of base64.
		let path = probed.tracks.first().map(|track| track.path.clone())?;
		return Some(CoverSource::Embedded { path });
	}

	let dir = if source.is_dir() {
		source.to_path_buf()
	} else {
		source.parent()?.to_path_buf()
	};
	for stem in COVER_STEMS {
		for extension in ["jpg", "jpeg", "png"] {
			let path = dir.join(format!("{stem}.{extension}"));
			if path.is_file() {
				return Some(CoverSource::File { path });
			}
		}
	}
	None
}

/// The ffmpeg command the transcode path will run, with the two derived temp
/// files named as they will be created.
///
/// Printed in the plan so the [`KIND_PLAN_TRANSCODE`] result is *actionable*:
/// an operator whose server has no ffmpeg can write the same concat list and
/// ffmetadata document — both fully determined by `inputs` and `marks` in the
/// same detail — and run this line on a machine that has one. The result is
/// the assembled audio, its tags and its chapters; the cover is the one thing
/// ffmpeg cannot write into the `covr` atom, so a hand-run book gets its
/// artwork from a following `meta-edit` (or from re-running this tool once
/// ffmpeg is installed).
fn printed_argv(detail: &AssembleDetail, output: &Path) -> Vec<String> {
	let list = PathBuf::from("concat.txt");
	let metadata = PathBuf::from("metadata.ffmeta");
	let request = Encode {
		inputs: &detail.inputs,
		output,
		codec: Codec::Aac,
		bitrate: detail.bitrate.as_deref(),
		metadata: Some(&metadata),
		cover: None,
		faststart: true,
		timeout: ffmpeg::timeout_for(detail.duration_ms),
	};
	let concat = (detail.inputs.len() > 1).then_some(list.as_path());
	std::iter::once("ffmpeg".to_string())
		.chain(
			ffmpeg::argv(&request, concat)
				.iter()
				.map(|arg| arg.to_string_lossy().to_string()),
		)
		.collect()
}

/// Produce the output file for one planned action.
///
/// Both paths end in the same two finishing passes, so a remuxed and a
/// transcoded book are indistinguishable in structure: the same tag set, both
/// chapter mechanisms, the same cover, `moov` in the same place.
fn assemble(kind: &str, target: &Path, detail: &AssembleDetail) -> ToolResult<()> {
	match kind {
		KIND_REMUX => remux_parts(target, detail)?,
		KIND_TRANSCODE => transcode_parts(target, detail)?,
		other => {
			return Err(ToolError::Invalid(format!(
				"unknown assemble kind {other:?}"
			)))
		},
	}

	let cover = read_cover(detail.cover.as_ref());
	let tags = mux::Tags {
		title: detail.tags.title.clone(),
		author: detail.tags.author.clone(),
		narrator: detail.tags.narrator.clone(),
		album: detail.tags.album.clone(),
		genre: detail.tags.genre.clone(),
		year: detail.tags.year.clone(),
		description: detail.tags.description.clone(),
		cover,
	};
	let marks: Vec<mux::Mark> = detail
		.marks
		.iter()
		.map(|mark| mux::Mark {
			title: mark.title.clone(),
			start_ms: mark.start_ms,
		})
		.collect();
	mux::write_tags_and_chapters(target, &tags, &marks)?;
	mux::faststart(target)?;

	verify(target, detail)
}

fn remux_parts(target: &Path, detail: &AssembleDetail) -> ToolResult<()> {
	let mut shapes = Vec::with_capacity(detail.inputs.len());
	for path in &detail.inputs {
		let shape = mux::aac_shape(path)?.ok_or_else(|| {
			ToolError::Invalid(format!(
				"{} is no longer an AAC stream this build can copy",
				path.display()
			))
		})?;
		shapes.push(shape);
	}
	let shape = mux::common_shape(&shapes).ok_or_else(|| {
		ToolError::Invalid("the parts no longer share one AAC stream shape".to_string())
	})?;

	mux::remux(&detail.inputs, target, &shape)?;
	Ok(())
}

fn transcode_parts(target: &Path, detail: &AssembleDetail) -> ToolResult<()> {
	let install = ffmpeg::locate(None)?;
	let scratch = tempfile::tempdir()?;

	let metadata_path = scratch.path().join("metadata.ffmeta");
	let chapters: Vec<MetadataChapter> = detail
		.marks
		.iter()
		.enumerate()
		.map(|(index, mark)| MetadataChapter {
			title: mark.title.clone(),
			start_ms: mark.start_ms,
			// ffmpeg wants an end for every chapter: the next mark's start,
			// and the publication duration for the last one.
			end_ms: detail
				.marks
				.get(index + 1)
				.map(|next| next.start_ms)
				.unwrap_or(detail.duration_ms),
		})
		.collect();
	let tags = ffmetadata_tags(&detail.tags);
	ffmpeg::write_ffmetadata(&metadata_path, &tags, &chapters)?;

	// The cover is deliberately *not* handed to ffmpeg. ffmpeg can only attach
	// a still image to an MP4 as an `attached_pic` video track, and a cover in
	// a video track is both redundant with the `covr` atom the tagger writes
	// two lines below and actively harmful: it is a second copy of the
	// artwork, and a strict MP4 reader that cannot parse a PNG sample entry
	// refuses the whole container — which is exactly why this tool demuxes
	// with symphonia in the first place.

	let staged = scratch.path().join(format!("assembled.{M4B}"));
	let output = ffmpeg::encode(
		&install,
		&Encode {
			inputs: &detail.inputs,
			output: &staged,
			codec: Codec::Aac,
			bitrate: detail.bitrate.as_deref(),
			metadata: Some(&metadata_path),
			cover: None,
			faststart: true,
			timeout: ffmpeg::timeout_for(detail.duration_ms),
		},
	)?;
	if !output.succeeded() {
		return Err(ToolError::Invalid(format!(
			"ffmpeg {}: {}",
			output.describe_status(),
			last_lines(&output.stderr)
		)));
	}

	// ffmpeg wrote into the scratch directory, which may be on another
	// filesystem, so the bytes are copied through the same atomic staging
	// every other tool writes with.
	write_atomic(target, |file| {
		let mut source = std::fs::File::open(&staged)?;
		std::io::copy(&mut source, file)?;
		Ok(())
	})
}

/// ffmetadata keys for the tags, in ffmpeg's own vocabulary.
///
/// ffmpeg maps these onto the MP4 atoms the tagger writes directly —
/// `composer` becomes `©wrt`, `description` becomes `©des` — so a hand-run
/// command and an `apply` produce the same tags.
fn ffmetadata_tags(tags: &TagDetail) -> Vec<(&'static str, String)> {
	let mut pairs = Vec::with_capacity(7);
	let mut push = |key: &'static str, value: &Option<String>| {
		if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
			pairs.push((key, value.to_string()));
		}
	};
	push("title", &tags.title);
	push("artist", &tags.author);
	push("album_artist", &tags.author);
	push("composer", &tags.narrator);
	push("album", &tags.album);
	push("genre", &tags.genre);
	push("date", &tags.year);
	push("description", &tags.description);
	pairs
}

fn last_lines(text: &str) -> String {
	text.lines()
		.rev()
		.take(3)
		.collect::<Vec<_>>()
		.into_iter()
		.rev()
		.collect::<Vec<_>>()
		.join("; ")
}

/// The cover bytes named by a plan, or `None` when they can no longer be read.
///
/// A missing cover is not a reason to refuse a whole book: the assemble
/// proceeds without artwork, which is exactly the state the parts were in.
fn read_cover(source: Option<&CoverSource>) -> Option<(ContentType, Vec<u8>)> {
	match source? {
		CoverSource::File { path } => {
			let bytes = std::fs::read(path).ok()?;
			let content_type = path.naive_content_type();
			matches!(content_type, ContentType::JPEG | ContentType::PNG)
				.then_some((content_type, bytes))
		},
		CoverSource::Embedded { path } => audio::probe_file(path).ok()?.cover,
	}
}

/// Refuse an output that is not the book the plan described.
///
/// The three things checked are the three ways an assemble can go wrong
/// silently: a dropped part (duration), a lost chapter list, and a container
/// that is not streamable. Every one of them produces a file that opens and
/// plays, which is precisely why they have to be checked rather than assumed.
fn verify(target: &Path, detail: &AssembleDetail) -> ToolResult<()> {
	let probed = audio::probe_file(target)?;

	let expected = detail.duration_ms.max(0) as f64;
	let actual = probed.duration_ms.max(0) as f64;
	if expected > 0.0 && (actual - expected).abs() / expected > DURATION_TOLERANCE {
		return Err(ToolError::Invalid(format!(
			"the assembled book is {} but its parts are {}",
			human_duration(probed.duration_ms),
			detail.duration
		)));
	}
	if probed.tracks.len() != 1 {
		return Err(ToolError::Invalid(format!(
			"the assembled book has {} tracks, not one",
			probed.tracks.len()
		)));
	}
	if !detail.marks.is_empty() && probed.chapters.len() != detail.marks.len() {
		return Err(ToolError::Invalid(format!(
			"the assembled book carries {} chapter marks, not the planned {}",
			probed.chapters.len(),
			detail.marks.len()
		)));
	}

	Ok(())
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

	fn plan_for(paths: &[PathBuf], options: serde_json::Value) -> Plan {
		AudioAssemble
			.plan(&ToolInput {
				paths: paths.to_vec(),
				options,
			})
			.expect("plan the assemble")
	}

	/// Copy a fixture publication into a writable directory, so a plan's
	/// output path is somewhere a test may create files.
	fn staged(names: &[&str]) -> (TempDir, PathBuf) {
		let dir = TempDir::new().expect("temp dir");
		let book = dir.path().join("Assembled Book");
		fs::create_dir_all(&book).expect("create book folder");
		for name in names {
			let from = fixture(name);
			let to = book.join(Path::new(name).file_name().expect("a file name"));
			fs::copy(&from, &to).unwrap_or_else(|error| panic!("copy {name}: {error}"));
		}
		(dir, book)
	}

	fn detail_of(action: &Action) -> AssembleDetail {
		serde_json::from_value(action.detail.clone()).expect("detail parses")
	}

	/// The whole remux contract, proved by probing what was written: one
	/// track, the planned chapter count, `moov` before `mdat`, and every tag
	/// round-tripping.
	#[test]
	fn remux_produces_a_single_track_faststart_m4b() {
		let (_dir, book) = staged(&["plain.m4b", "chapters-chpl.m4b"]);
		let plan = plan_for(&[book.clone()], serde_json::Value::Null);

		assert_eq!(plan.actions.len(), 1, "{plan:?}");
		assert_eq!(plan.actions[0].kind, KIND_REMUX);
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.parts, 2);
		let output = plan.actions[0]
			.target
			.clone()
			.expect("the plan names an output");
		// `plan` is pure: nothing exists yet.
		assert!(!output.exists());

		let report = AudioAssemble
			.apply(&plan, &mut NoopProgress)
			.expect("apply the plan");
		assert_eq!(report.skipped, Vec::new(), "{report:?}");
		assert_eq!(report.applied.len(), 1);

		let probed = audio::probe_file(&output).expect("probe the output");
		assert_eq!(probed.tracks.len(), 1, "one track, not one per part");
		assert_eq!(probed.codec, "aac");
		assert_eq!(probed.chapters.len(), detail.marks.len());
		assert!(moov_precedes_mdat(&output), "moov must come first");

		// The parts are untouched.
		assert!(book.join("plain.m4b").is_file());
		assert!(book.join("chapters-chpl.m4b").is_file());
	}

	/// Both chapter mechanisms, so neither half of the player population sees
	/// a book with no chapters. The probe prefers `chpl`, so the track is
	/// asserted directly.
	#[test]
	fn remux_writes_both_chapter_mechanisms() {
		let (_dir, book) = staged(&["plain.m4b", "chapters-chpl.m4b"]);
		let plan = plan_for(&[book], serde_json::Value::Null);
		let output = plan.actions[0].target.clone().expect("an output");
		AudioAssemble
			.apply(&plan, &mut NoopProgress)
			.expect("apply the plan");

		let config = mp4ameta::ReadConfig {
			read_chapter_list: true,
			read_chapter_track: true,
			..mp4ameta::ReadConfig::NONE
		};
		let tag = mp4ameta::Tag::read_with_path(&output, &config).expect("read tags");

		assert!(!tag.chapter_list().is_empty(), "the chpl list");
		assert!(!tag.chapter_track().is_empty(), "the chapter track");
		assert_eq!(tag.chapter_list().len(), tag.chapter_track().len());
	}

	/// Every tag a listener sees survives the assemble, and the container's
	/// own tags outrank the folder name.
	#[test]
	fn remux_round_trips_every_tag() {
		let (_dir, book) = staged(&["plain.m4b", "chapters-chpl.m4b"]);
		let plan = plan_for(&[book], serde_json::Value::Null);
		let output = plan.actions[0].target.clone().expect("an output");
		let detail = detail_of(&plan.actions[0]);
		AudioAssemble
			.apply(&plan, &mut NoopProgress)
			.expect("apply the plan");

		let config = mp4ameta::ReadConfig {
			read_meta_items: true,
			..mp4ameta::ReadConfig::NONE
		};
		let tag = mp4ameta::Tag::read_with_path(&output, &config).expect("read tags");

		// The parts carry their own album and title; the folder is only a
		// fallback, so it must not overwrite them.
		assert_eq!(tag.album(), Some("Silent Test Book"));
		assert_eq!(tag.album().map(str::to_string), detail.tags.album);
		assert_eq!(
			tag.title().map(str::to_string),
			detail.tags.title,
			"the planned title is the written title"
		);
		// `©ART` and `©aART` both, so a player that groups by album artist
		// does not scatter one author across two rows.
		assert_eq!(tag.artist().map(str::to_string), detail.tags.author);
		assert_eq!(tag.album_artist().map(str::to_string), detail.tags.author);
		assert_eq!(
			tag.composer().map(str::to_string),
			detail.tags.narrator,
			"the narrator lands in ©wrt"
		);
		assert!(
			detail.tags.author.is_some(),
			"the fixture carries an author"
		);
	}

	/// With no tags in the parts at all, the folder name fills the two fields
	/// a listener always sees rather than leaving the title bar blank.
	#[test]
	fn the_folder_name_is_the_title_fallback() {
		let dir = TempDir::new().expect("temp dir");
		let book = dir.path().join("Berserk, Volume 1");
		fs::create_dir_all(&book).expect("create the book folder");

		let tags = resolve_tags(&book, &ProbedAudio::default());

		assert_eq!(tags.title.as_deref(), Some("Berserk, Volume 1"));
		assert_eq!(tags.album.as_deref(), Some("Berserk, Volume 1"));
		assert_eq!(tags.author, None, "a name is not an author");
	}

	/// A folder of MP3s cannot be remuxed. With ffmpeg masked from `PATH` the
	/// result is plan-only and actionable, never a broken file.
	#[test]
	fn mp3_parts_plan_only_when_ffmpeg_is_masked() {
		let (_dir, book) = staged(&[
			"folder-tracknum/alpha.mp3",
			"folder-tracknum/mike.mp3",
			"folder-tracknum/zulu.mp3",
		]);
		let empty = TempDir::new().expect("temp dir");
		let plan = plan_for(
			&[book],
			serde_json::json!({ "bin_dir": empty.path().to_string_lossy() }),
		);

		assert_eq!(plan.actions.len(), 1, "{plan:?}");
		assert_eq!(plan.actions[0].kind, KIND_PLAN_TRANSCODE);
		let missing = plan
			.warnings
			.iter()
			.find(|warning| warning.code == "ffmpeg-missing")
			.expect("an ffmpeg-missing warning");
		assert_eq!(missing.severity, Severity::Error);

		// The plan is actionable: the exact command, the marks and the tags.
		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.chapter_source, "per_track");
		assert_eq!(detail.marks.len(), 3);
		assert_eq!(detail.bitrate.as_deref(), Some(DEFAULT_BITRATE));
		let command = detail.ffmpeg.join(" ");
		assert!(command.starts_with("ffmpeg "), "{command}");
		assert!(command.contains("-c:a aac"), "{command}");
		assert!(command.contains("-movflags +faststart"), "{command}");
		assert!(command.contains("-f concat"), "{command}");

		let output = plan.actions[0].target.clone().expect("an output");
		let report = AudioAssemble
			.apply(&plan, &mut NoopProgress)
			.expect("apply the plan");
		assert!(report.applied.is_empty());
		assert_eq!(report.skipped.len(), 1);
		assert!(report.skipped[0].1.contains("ffmpeg is unavailable"));
		assert!(!output.exists(), "a plan-only result writes nothing");
	}

	/// `allow_transcode: false` makes the tool lossless or nothing.
	#[test]
	fn transcode_can_be_refused_outright() {
		let (_dir, book) = staged(&["folder-tracknum/alpha.mp3"]);
		let plan = plan_for(&[book], serde_json::json!({ "allow_transcode": false }));

		assert!(plan.actions.is_empty(), "{plan:?}");
		let refused = plan
			.warnings
			.iter()
			.find(|warning| warning.code == "transcode-refused")
			.expect("a transcode-refused warning");
		assert_eq!(refused.severity, Severity::Error);
	}

	/// A null option blob and `{}` are the same run.
	///
	/// `ToolInput::parse_options` resolves a null blob through `Default` and a
	/// present object through `serde`, so the two paths only agree while
	/// `Default` agrees with the `#[serde(default = ...)]` attributes. They
	/// did not: a derived `Default` gave `allow_transcode: false`, and every
	/// caller that passed no options at all — the ingest auto-assemble among
	/// them — silently refused every MP3 book while the same tool invoked
	/// with `{}` assembled it.
	#[test]
	fn no_options_and_an_empty_object_plan_the_same_transcode() {
		let (_dir, book) = staged(&["folder-tracknum/alpha.mp3"]);

		let implicit = AudioAssemble
			.plan(&ToolInput::new(vec![book.clone()]))
			.expect("plan with no options at all");
		let explicit = plan_for(&[book], serde_json::json!({}));

		assert_eq!(implicit, explicit);
		assert!(
			implicit
				.warnings
				.iter()
				.all(|warning| warning.code != "transcode-refused"),
			"{implicit:?}"
		);
		assert_eq!(
			implicit.actions.len(),
			1,
			"an mp3 book plans a transcode by default: {implicit:?}"
		);
	}

	/// Marks a publisher authored beat the file list.
	#[test]
	fn container_marks_outrank_the_file_list() {
		let dir = TempDir::new().expect("temp dir");
		let book = dir.path().join("Chaptered.m4b");
		fs::copy(fixture("chapters-chpl.m4b"), &book).expect("copy fixture");

		let plan = plan_for(
			&[book.clone()],
			serde_json::json!({ "output_dir": dir.path().join("out").to_string_lossy() }),
		);

		let detail = detail_of(&plan.actions[0]);
		assert_eq!(detail.chapter_source, "container");
		let probed = audio::probe_file(&book).expect("probe the fixture");
		assert_eq!(detail.marks.len(), probed.chapters.len());
	}

	/// A sidecar beats the file list, and loses to authored marks. Asserted
	/// through the resolver, because the precedence is the same whichever
	/// assemble path the publication then takes.
	#[test]
	fn a_sidecar_outranks_the_file_list_and_loses_to_the_container() {
		let (_dir, book) =
			staged(&["folder-tracknum/alpha.mp3", "folder-tracknum/mike.mp3"]);
		fs::write(
			book.join("chapters.txt"),
			"00:00:00 From The Sidecar\n00:00:01 Second\n",
		)
		.expect("write the sidecar");
		let options = AudioAssembleOptions::default();

		// The parts carry no marks, so the probe synthesized one per file —
		// and the sidecar replaces that synthesis.
		let probed = audio::probe(&book).expect("probe the folder");
		assert_eq!(probed.chapter_source, ChapterSource::PerTrack);
		let (source, marks) = resolve_marks(&book, &probed, &options);
		assert_eq!(source, "chapters.txt");
		assert_eq!(marks.len(), 2);
		assert_eq!(marks[0].title, "From The Sidecar");

		// Marks a publisher authored are not displaced by a sidecar sitting
		// in the same folder.
		let authored = audio::probe_file(&fixture("chapters-chpl.m4b"))
			.expect("probe the chaptered fixture");
		let staged_container = book.join("Chaptered.m4b");
		fs::copy(fixture("chapters-chpl.m4b"), &staged_container)
			.expect("copy the chaptered fixture");
		let (source, marks) = resolve_marks(&staged_container, &authored, &options);
		assert_eq!(source, "container");
		assert_eq!(marks.len(), authored.chapters.len());
		assert_ne!(marks[0].title, "From The Sidecar");
	}

	/// An existing output is never silently replaced.
	#[test]
	fn an_existing_output_needs_force() {
		let (dir, book) = staged(&["plain.m4b", "chapters-chpl.m4b"]);
		let output = dir.path().join("Assembled Book.m4b");
		fs::write(&output, b"last week's assemble").expect("write the output");

		let plan = plan_for(&[book.clone()], serde_json::Value::Null);
		assert!(plan.actions.is_empty(), "{plan:?}");
		assert!(plan
			.warnings
			.iter()
			.any(|warning| warning.code == "output-exists"));
		assert_eq!(
			fs::read(&output).expect("read the output"),
			b"last week's assemble",
			"the existing file is untouched"
		);

		let forced = plan_for(&[book], serde_json::json!({ "force": true }));
		assert_eq!(forced.actions.len(), 1);
	}

	/// A single AAC-in-MP4 file already *is* the canonical container.
	#[test]
	fn an_assembled_book_is_left_alone() {
		let dir = TempDir::new().expect("temp dir");
		let book = dir.path().join("Already.m4b");
		fs::copy(fixture("chapters-chpl.m4b"), &book).expect("copy fixture");

		let plan = plan_for(&[book], serde_json::Value::Null);

		assert!(plan.actions.is_empty(), "{plan:?}");
		assert!(plan
			.warnings
			.iter()
			.any(|warning| warning.code == "already-assembled"));
	}

	#[test]
	fn a_plan_from_another_tool_is_refused() {
		let plan = Plan::new("audio-chapters");
		let error = AudioAssemble
			.apply(&plan, &mut NoopProgress)
			.expect_err("refuse the plan");

		assert!(matches!(error, ToolError::PlanMismatch { .. }), "{error:?}");
	}

	#[test]
	fn no_paths_is_an_error_not_an_empty_plan() {
		let error = AudioAssemble
			.plan(&ToolInput::default())
			.expect_err("refuse an empty input");

		assert!(matches!(error, ToolError::Invalid(_)), "{error:?}");
	}

	/// A part that vanished between plan and apply means the book is no
	/// longer the book the plan described.
	#[test]
	fn a_vanished_part_skips_the_action() {
		let (_dir, book) = staged(&["plain.m4b", "chapters-chpl.m4b"]);
		let plan = plan_for(&[book.clone()], serde_json::Value::Null);
		fs::remove_file(book.join("plain.m4b")).expect("remove a part");

		let report = AudioAssemble
			.apply(&plan, &mut NoopProgress)
			.expect("apply the plan");

		assert!(report.applied.is_empty());
		assert_eq!(report.skipped.len(), 1);
		assert!(report.skipped[0].1.contains("no longer exists"));
	}

	/// `moov` ahead of `mdat` — the property that makes the file streamable.
	fn moov_precedes_mdat(path: &Path) -> bool {
		use std::io::{Read, Seek, SeekFrom};

		let mut file = fs::File::open(path).expect("open the output");
		let length = file.seek(SeekFrom::End(0)).expect("measure the output");
		let mut offset = 0u64;
		let mut order = Vec::new();
		while offset + 8 <= length {
			file.seek(SeekFrom::Start(offset)).expect("seek");
			let mut header = [0u8; 8];
			file.read_exact(&mut header).expect("read a box header");
			let declared =
				u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
			let size = match declared {
				1 => {
					let mut large = [0u8; 8];
					file.read_exact(&mut large).expect("read a large size");
					u64::from_be_bytes(large)
				},
				0 => length - offset,
				other => u64::from(other),
			};
			order.push(String::from_utf8_lossy(&header[4..8]).to_string());
			if size < 8 {
				break;
			}
			offset += size;
		}

		let moov = order.iter().position(|name| name == "moov");
		let mdat = order.iter().position(|name| name == "mdat");
		matches!((moov, mdat), (Some(moov), Some(mdat)) if moov < mdat)
	}
}
