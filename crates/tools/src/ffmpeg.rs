//! The crate's `ffmpeg` adapter: the one place a lossy audio *re-encode* is
//! shelled out to an operator-installed binary.
//!
//! Everything else this crate does to an audiobook is pure Rust —
//! [`crate::audio_report`] probes, [`crate::audio_chapters`] writes marks,
//! [`crate::audio_assemble`] remuxes AAC-in-MP4 tracks into one M4B — because
//! a library server must not depend on a C toolchain to read its own files.
//! Transcoding is the exception, and it is a real one: MP3 and Opus cannot be
//! *remuxed* into the canonical M4B, they have to be decoded and re-encoded as
//! AAC, and there is no maintained pure-Rust AAC encoder. Vendoring one would
//! mean either a GPL/LGPL C dependency in an MIT tree or a hand-written
//! encoder nobody should trust with an operator's library.
//!
//! So `ffmpeg` is located the same way calibre and boko are: in the operator's
//! own installation, version-checked once, run as a child process, and absent
//! is a clean [`ToolError::ExternalToolMissing`] rather than a broken output
//! file. It is **never** a build dependency and never linked.
//!
//! # What is delegated, exactly
//!
//! One command per output file, built by [`argv`]:
//!
//! * **concatenation** — several parts become one stream through the `concat`
//!   demuxer, so the parts are decoded in order without a separate join pass;
//! * **encoding** — `-c:a aac` for the canonical container, `-c:a libopus` for
//!   the delivery preset, `-c:a copy` when the caller has already established
//!   the stream needs no conversion;
//! * **metadata and chapters** — a `;FFMETADATA1` file written by
//!   [`write_ffmetadata`] and mapped in with `-map_metadata`, which is the
//!   only way to hand ffmpeg a chapter list;
//! * **cover art** — a still image mapped as an `attached_pic` video stream;
//! * **`-movflags +faststart`** — the `moov` atom is moved ahead of `mdat` in
//!   a second pass so a player can start without reading the whole file.
//!
//! [`argv`] is pure and public precisely so the command line can be asserted
//! in a test on a machine with no ffmpeg at all: the flags are the contract,
//! and a silently changed flag is a silently changed output file.

use std::{
	ffi::OsString,
	fmt,
	io::Write,
	path::{Path, PathBuf},
	time::Duration,
};

use crate::{
	external::{ExternalTool, Install, MinVersion, Output},
	ToolError, ToolResult,
};

/// Name, and the `tool` field of every [`ToolError::ExternalToolMissing`] this
/// module raises.
const FFMPEG: &str = "ffmpeg";

/// `-movflags +faststart` and `;FFMETADATA1` chapters both long predate this;
/// 4.4 (2021) is simply the oldest release still shipped by a maintained
/// distribution, so anything older is an installation to fix rather than a
/// version to work around.
const MIN_VERSION: (u32, u32) = (4, 4);

const INSTALL_HINT: &str = "install ffmpeg (https://ffmpeg.org/download.html) and put it on PATH, or set the tool's bin_dir";

/// `ffmpeg -version` prints `ffmpeg version n9.0.1` or
/// `ffmpeg version 4.4.2-0ubuntu0.22.04.1`; anchoring on the wording keeps a
/// version out of the `built with gcc 16` line that follows it.
static FFMPEG_TOOL: ExternalTool = ExternalTool {
	name: FFMPEG,
	probe: FFMPEG,
	version_arg: "-version",
	anchor: "ffmpeg version ",
	min: MinVersion::MajorMinor(MIN_VERSION.0, MIN_VERSION.1),
	extra_dirs: Vec::new,
	searched: "PATH",
	hint: INSTALL_HINT,
};

/// Find ffmpeg and verify it is at least [`MIN_VERSION`].
///
/// `bin_dir` wins when given, otherwise `PATH` is searched. ffmpeg has no
/// documented per-platform install directory the way calibre does — every
/// package manager and every static build puts it on `PATH` — so there are no
/// extra directories to search, and a hand-unpacked static build is exactly
/// what `bin_dir` is for.
pub fn locate(bin_dir: Option<&Path>) -> ToolResult<Install> {
	FFMPEG_TOOL.locate(bin_dir)
}

/// Read the duration libavformat will assign to one concat input.
///
/// `audio-assemble` must use the same clock as ffmpeg's concat demuxer when it
/// writes permanent chapter marks. A cheap MP3 estimate may count trailing
/// tags, while a packet-duration sum may discard per-file delay that the
/// concat demuxer retains between parts. `ffprobe` ships beside ffmpeg and
/// exposes libavformat's exact `format.duration` without decoding.
pub fn probe_duration_ms(install: &Install, path: &Path) -> ToolResult<i64> {
	let program = FFMPEG_TOOL.binary(&install.dir, "ffprobe");
	let args = [
		OsString::from("-v"),
		OsString::from("error"),
		OsString::from("-show_entries"),
		OsString::from("format=duration"),
		OsString::from("-of"),
		OsString::from("default=noprint_wrappers=1:nokey=1"),
		path.as_os_str().to_owned(),
	];
	let output = FFMPEG_TOOL.run(&program, &args, Duration::from_secs(30))?;
	if !output.succeeded() {
		return Err(ToolError::Invalid(format!(
			"ffprobe {}: {}",
			output.describe_status(),
			output.stderr.trim()
		)));
	}
	let seconds = output.stdout.trim().parse::<f64>().map_err(|error| {
		ToolError::Invalid(format!(
			"ffprobe returned an unreadable duration for {}: {error}",
			path.display()
		))
	})?;
	if !seconds.is_finite() || seconds <= 0.0 {
		return Err(ToolError::Invalid(format!(
			"ffprobe returned an invalid duration for {}: {seconds}",
			path.display()
		)));
	}
	Ok((seconds * 1000.0).round().min(i64::MAX as f64) as i64)
}

/// What the output stream is encoded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
	/// AAC-LC through ffmpeg's built-in encoder, the canonical M4B stream.
	/// The native encoder is used rather than `libfdk_aac` because an
	/// operator's distribution build almost never carries the non-free one,
	/// and a tool that only works on a self-compiled ffmpeg works nowhere.
	Aac,
	/// Opus through `libopus`, for the per-device delivery preset. Never a
	/// stored container: Opus is an input Stump reads and an output it hands
	/// to a device, and the canonical archive copy stays AAC-in-MP4.
	Opus,
	/// Pass the coded stream through untouched. Only correct when the caller
	/// has already established the codec is acceptable in the output
	/// container; a wrong `Copy` produces a file that muxes and will not play.
	Copy,
}

impl Codec {
	/// The `-c:a` value.
	fn encoder(self) -> &'static str {
		match self {
			Self::Aac => "aac",
			Self::Opus => "libopus",
			Self::Copy => "copy",
		}
	}

	/// Whether a bitrate applies. `copy` keeps the input's, and passing
	/// `-b:a` alongside it is at best ignored and at worst an error.
	fn takes_bitrate(self) -> bool {
		!matches!(self, Self::Copy)
	}
}

impl fmt::Display for Codec {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.encoder())
	}
}

/// One output file to produce.
#[derive(Debug)]
pub struct Encode<'a> {
	/// The parts, in playback order. One input is read directly; several are
	/// joined through the `concat` demuxer, which needs the list file
	/// [`encode`] writes and [`argv`] is told about.
	pub inputs: &'a [PathBuf],
	pub output: &'a Path,
	pub codec: Codec,
	/// `-b:a` value in ffmpeg's own spelling (`64k`, `128000`). `None` leaves
	/// the encoder's default; validate an operator-supplied value with
	/// [`stump_media::transform::parse_bitrate`] before passing it.
	pub bitrate: Option<&'a str>,
	/// A `;FFMETADATA1` file from [`write_ffmetadata`]: tags and the chapter
	/// list. Mapped with `-map_metadata`, the only channel ffmpeg accepts a
	/// chapter list through.
	pub metadata: Option<&'a Path>,
	/// A still image to attach as cover art.
	pub cover: Option<&'a Path>,
	/// Move `moov` ahead of `mdat`. Always true for a stored M4B; the flag is
	/// MP4-only, so an Opus output leaves it off.
	pub faststart: bool,
	/// Wall clock the child gets. A re-encode is the one genuinely long
	/// operation in this crate — an unabridged audiobook is tens of hours of
	/// audio — so the caller sizes this from the material rather than
	/// inheriting a constant that is either absurd for a short file or too
	/// tight for a long one. See [`timeout_for`].
	pub timeout: Duration,
}

/// A generous ceiling for re-encoding `duration_ms` of audio.
///
/// AAC encoding runs at a large multiple of real time on any machine that can
/// run a library server, so this is not an estimate: it is the point at which
/// something has gone wrong and the child should be killed instead of
/// occupying a worker forever. One tenth of real time plus a ten-minute floor
/// leaves roughly an order of magnitude of headroom.
#[must_use]
pub fn timeout_for(duration_ms: i64) -> Duration {
	let realtime = Duration::from_millis(duration_ms.max(0) as u64);
	(realtime / 10).max(Duration::from_secs(600))
}

/// The exact argument vector for `request`.
///
/// `concat_list` is the list file when [`Encode::inputs`] holds more than one
/// part, and `None` for a single input. Pure and public so the flags can be
/// asserted without ffmpeg installed.
///
/// Input indices are assigned in the order the inputs are added, because
/// `-map` and `-map_metadata` address them positionally: the audio is always
/// input 0, the metadata file and the cover take 1 and 2 in that order when
/// present, and the mapping arguments are built from the same counter rather
/// than from hardcoded numbers that a new optional input would silently
/// invalidate.
#[must_use]
pub fn argv(request: &Encode<'_>, concat_list: Option<&Path>) -> Vec<OsString> {
	let mut args: Vec<OsString> = Vec::with_capacity(24);
	let mut push = |value: &str| args.push(OsString::from(value));

	// `-nostdin` because the parent's stdin is `/dev/null` and ffmpeg would
	// otherwise still put the terminal in raw mode; `-y` because the output is
	// a caller-owned temp file that may already exist.
	push("-nostdin");
	push("-y");
	// Only warnings and errors: the tail of stderr is what a failure is
	// reported with, and the default banner plus per-second progress lines
	// would push the useful part out of it.
	push("-loglevel");
	push("warning");

	let mut next_input = 0u32;
	let audio_input = next_input;
	next_input += 1;
	match concat_list {
		Some(list) => {
			// `-safe 0` permits absolute paths in the list, which library
			// paths always are.
			push("-f");
			push("concat");
			push("-safe");
			push("0");
			push("-i");
			args.push(list.as_os_str().to_owned());
		},
		None => {
			push("-i");
			let single = request.inputs.first().map(PathBuf::as_path);
			args.push(single.unwrap_or(Path::new("")).as_os_str().to_owned());
		},
	}

	let metadata_input = request.metadata.map(|path| {
		let index = next_input;
		next_input += 1;
		args.push(OsString::from("-i"));
		args.push(path.as_os_str().to_owned());
		index
	});
	let cover_input = request.cover.map(|path| {
		let index = next_input;
		next_input += 1;
		args.push(OsString::from("-i"));
		args.push(path.as_os_str().to_owned());
		index
	});

	args.push(OsString::from("-map"));
	args.push(OsString::from(format!("{audio_input}:a")));
	if let Some(index) = cover_input {
		args.push(OsString::from("-map"));
		args.push(OsString::from(format!("{index}:v")));
	}
	if let Some(index) = metadata_input {
		// Without this the output carries the *input's* tags and no chapters;
		// with it, the tags and chapters of the ffmetadata file replace them
		// wholesale, which is what a freshly assembled book wants.
		args.push(OsString::from("-map_metadata"));
		args.push(OsString::from(index.to_string()));
	}

	args.push(OsString::from("-c:a"));
	args.push(OsString::from(request.codec.encoder()));
	if let (true, Some(bitrate)) = (request.codec.takes_bitrate(), request.bitrate) {
		args.push(OsString::from("-b:a"));
		args.push(OsString::from(bitrate));
	}
	if cover_input.is_some() {
		// The cover is already a compressed still; re-encoding it would only
		// lose quality. `attached_pic` is what makes a player treat the video
		// stream as artwork instead of a one-frame movie.
		args.push(OsString::from("-c:v"));
		args.push(OsString::from("copy"));
		args.push(OsString::from("-disposition:v"));
		args.push(OsString::from("attached_pic"));
	}
	if request.faststart {
		args.push(OsString::from("-movflags"));
		args.push(OsString::from("+faststart"));
	}

	args.push(request.output.as_os_str().to_owned());
	args
}

/// Run one [`Encode`].
///
/// The `concat` list file lives in a temp directory that is dropped when this
/// returns, so a killed or failed child leaves nothing behind. A non-zero exit
/// is returned as an [`Output`], not an error: the caller decides whether one
/// failed book aborts a run or is reported as a skip.
pub fn encode(install: &Install, request: &Encode<'_>) -> ToolResult<Output> {
	if request.inputs.is_empty() {
		return Err(ToolError::Invalid(
			"ffmpeg was given no input files".to_string(),
		));
	}

	let scratch = tempfile::tempdir()?;
	let list_path = scratch.path().join("concat.txt");
	let concat = if request.inputs.len() > 1 {
		write_concat_list(&list_path, request.inputs)?;
		Some(list_path.as_path())
	} else {
		None
	};

	let args = argv(request, concat);
	let program = FFMPEG_TOOL.binary(&install.dir, FFMPEG);
	FFMPEG_TOOL.run(&program, &args, request.timeout)
}

/// A `concat` demuxer list: one `file '<path>'` line per part.
///
/// Single quotes are the demuxer's own quoting, and a literal quote inside a
/// path is escaped by closing, emitting an escaped quote, and reopening
/// (`'\''`) — the same rule a POSIX shell uses, and the only escape the
/// demuxer's parser implements.
fn write_concat_list(path: &Path, inputs: &[PathBuf]) -> ToolResult<()> {
	let mut file = std::fs::File::create(path)?;
	for input in inputs {
		let quoted = input.to_string_lossy().replace('\'', r"'\''");
		writeln!(file, "file '{quoted}'")?;
	}
	file.sync_all()?;
	Ok(())
}

/// One chapter in a `;FFMETADATA1` file, in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataChapter {
	pub title: String,
	pub start_ms: i64,
	/// ffmpeg requires an end; a caller with only start marks passes the next
	/// mark's start, and the last chapter passes the publication duration.
	pub end_ms: i64,
}

/// Write a `;FFMETADATA1` document: `key=value` tags, then one `[CHAPTER]`
/// block per mark.
///
/// `TIMEBASE=1/1000` makes every `START`/`END` a millisecond, matching the
/// unit every audio value in Stump is expressed in, so nothing has to be
/// rescaled on the way in or out.
pub fn write_ffmetadata(
	path: &Path,
	tags: &[(&str, String)],
	chapters: &[MetadataChapter],
) -> ToolResult<()> {
	let mut file = std::fs::File::create(path)?;
	writeln!(file, ";FFMETADATA1")?;
	for (key, value) in tags {
		if value.trim().is_empty() {
			continue;
		}
		writeln!(file, "{}={}", escape_metadata(key), escape_metadata(value))?;
	}
	for chapter in chapters {
		writeln!(file, "[CHAPTER]")?;
		writeln!(file, "TIMEBASE=1/1000")?;
		writeln!(file, "START={}", chapter.start_ms.max(0))?;
		writeln!(file, "END={}", chapter.end_ms.max(chapter.start_ms.max(0)))?;
		writeln!(file, "title={}", escape_metadata(&chapter.title))?;
	}
	file.sync_all()?;
	Ok(())
}

/// Escape the four characters the ffmetadata parser treats as syntax, plus a
/// newline inside a value.
///
/// A book blurb routinely contains `=` and `#`, and a title can contain `;`;
/// without escaping, the first of them truncates the value or turns the rest
/// of it into a comment.
fn escape_metadata(value: &str) -> String {
	let mut escaped = String::with_capacity(value.len());
	for character in value.chars() {
		match character {
			'=' | ';' | '#' | '\\' => {
				escaped.push('\\');
				escaped.push(character);
			},
			'\n' => escaped.push_str("\\\n"),
			other => escaped.push(other),
		}
	}
	escaped
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fs;
	use tempfile::TempDir;

	fn strings(args: &[OsString]) -> Vec<String> {
		args.iter()
			.map(|arg| arg.to_string_lossy().to_string())
			.collect()
	}

	fn request<'a>(inputs: &'a [PathBuf], output: &'a Path, codec: Codec) -> Encode<'a> {
		Encode {
			inputs,
			output,
			codec,
			bitrate: Some("64k"),
			metadata: None,
			cover: None,
			faststart: true,
			timeout: Duration::from_secs(60),
		}
	}

	/// The canonical single-input command: one AAC stream, faststart on.
	#[test]
	fn single_input_reads_the_file_directly() {
		let inputs = vec![PathBuf::from("/books/part.mp3")];
		let output = PathBuf::from("/books/out.m4b");
		let args = strings(&argv(&request(&inputs, &output, Codec::Aac), None));

		assert_eq!(
			args,
			vec![
				"-nostdin",
				"-y",
				"-loglevel",
				"warning",
				"-i",
				"/books/part.mp3",
				"-map",
				"0:a",
				"-c:a",
				"aac",
				"-b:a",
				"64k",
				"-movflags",
				"+faststart",
				"/books/out.m4b",
			]
		);
	}

	/// Several parts are joined by the demuxer, not by a separate pass.
	#[test]
	fn multiple_inputs_use_the_concat_demuxer() {
		let inputs = vec![PathBuf::from("/books/a.mp3"), PathBuf::from("/books/b.mp3")];
		let output = PathBuf::from("/books/out.m4b");
		let list = PathBuf::from("/tmp/concat.txt");
		let args = strings(&argv(&request(&inputs, &output, Codec::Aac), Some(&list)));

		let joined = args.join(" ");
		assert!(
			joined.contains("-f concat -safe 0 -i /tmp/concat.txt"),
			"{joined}"
		);
		// The parts are never named on the command line: the list file is.
		assert!(!joined.contains("/books/a.mp3"), "{joined}");
	}

	/// Metadata and cover inputs are addressed by their real positions, and
	/// the cover becomes artwork rather than a video track.
	#[test]
	fn metadata_and_cover_are_mapped_by_index() {
		let inputs = vec![PathBuf::from("/books/part.m4a")];
		let output = PathBuf::from("/books/out.m4b");
		let metadata = PathBuf::from("/tmp/meta.txt");
		let cover = PathBuf::from("/tmp/cover.jpg");
		let args = strings(&argv(
			&Encode {
				metadata: Some(&metadata),
				cover: Some(&cover),
				..request(&inputs, &output, Codec::Aac)
			},
			None,
		));

		let joined = args.join(" ");
		assert!(
			joined.contains("-i /tmp/meta.txt -i /tmp/cover.jpg"),
			"{joined}"
		);
		assert!(joined.contains("-map 0:a -map 2:v"), "{joined}");
		assert!(joined.contains("-map_metadata 1"), "{joined}");
		assert!(
			joined.contains("-c:v copy -disposition:v attached_pic"),
			"{joined}"
		);
	}

	/// A cover with no metadata file must not inherit the metadata input's
	/// index, which is the bug a hardcoded `-map 1:v` would be.
	#[test]
	fn cover_without_metadata_takes_the_first_free_index() {
		let inputs = vec![PathBuf::from("/books/part.m4a")];
		let output = PathBuf::from("/books/out.m4b");
		let cover = PathBuf::from("/tmp/cover.jpg");
		let args = strings(&argv(
			&Encode {
				cover: Some(&cover),
				..request(&inputs, &output, Codec::Aac)
			},
			None,
		));

		let joined = args.join(" ");
		assert!(joined.contains("-map 0:a -map 1:v"), "{joined}");
		assert!(!joined.contains("-map_metadata"), "{joined}");
	}

	/// `copy` takes no bitrate: ffmpeg would either ignore `-b:a` or refuse.
	#[test]
	fn copy_drops_the_bitrate() {
		let inputs = vec![PathBuf::from("/books/part.m4a")];
		let output = PathBuf::from("/books/out.m4b");
		let args = strings(&argv(&request(&inputs, &output, Codec::Copy), None));

		assert!(args.contains(&"copy".to_string()));
		assert!(!args.contains(&"-b:a".to_string()), "{args:?}");
	}

	/// The Opus delivery preset is not an MP4, so the MP4-only flag is off.
	#[test]
	fn opus_output_leaves_faststart_off() {
		let inputs = vec![PathBuf::from("/books/part.m4b")];
		let output = PathBuf::from("/books/out.opus");
		let args = strings(&argv(
			&Encode {
				faststart: false,
				..request(&inputs, &output, Codec::Opus)
			},
			None,
		));

		assert!(args.contains(&"libopus".to_string()));
		assert!(!args.contains(&"-movflags".to_string()), "{args:?}");
	}

	#[test]
	fn concat_list_quotes_every_part() {
		let dir = TempDir::new().expect("temp dir");
		let list = dir.path().join("concat.txt");
		write_concat_list(
			&list,
			&[
				PathBuf::from("/books/one.mp3"),
				PathBuf::from("/books/it's two.mp3"),
			],
		)
		.expect("write list");

		assert_eq!(
			fs::read_to_string(&list).expect("read list"),
			"file '/books/one.mp3'\nfile '/books/it'\\''s two.mp3'\n"
		);
	}

	/// A blurb with an `=` in it must not truncate, and a chapter block must
	/// carry the millisecond timebase.
	#[test]
	fn ffmetadata_escapes_syntax_and_writes_chapters() {
		let dir = TempDir::new().expect("temp dir");
		let path = dir.path().join("meta.txt");
		write_ffmetadata(
			&path,
			&[
				("title", "E=mc2; a #1 story".to_string()),
				("artist", "  ".to_string()),
			],
			&[MetadataChapter {
				title: "One".to_string(),
				start_ms: 0,
				end_ms: 1500,
			}],
		)
		.expect("write metadata");

		let written = fs::read_to_string(&path).expect("read metadata");
		assert_eq!(
			written,
			";FFMETADATA1\n\
			 title=E\\=mc2\\; a \\#1 story\n\
			 [CHAPTER]\n\
			 TIMEBASE=1/1000\n\
			 START=0\n\
			 END=1500\n\
			 title=One\n"
		);
	}

	/// The version floor is read out of the banner ffmpeg actually prints,
	/// including the `n`-prefixed one every recent release uses.
	#[test]
	fn version_is_parsed_from_the_real_banner() {
		assert_eq!(
			FFMPEG_TOOL.parse_version("ffmpeg version n9.0.1 Copyright (c) 2000-2026"),
			Some((9, 0, 1))
		);
		assert_eq!(
			FFMPEG_TOOL.parse_version(
				"ffmpeg version 4.4.2-0ubuntu0.22.04.1\nbuilt with gcc 11"
			),
			Some((4, 4, 2))
		);
	}

	/// An absent binary is one actionable error, not an I/O failure.
	#[test]
	fn missing_ffmpeg_names_the_tool_and_the_fix() {
		let empty = TempDir::new().expect("temp dir");
		let error = locate(Some(empty.path())).expect_err("no ffmpeg in an empty dir");

		match error {
			ToolError::ExternalToolMissing { tool, hint, .. } => {
				assert_eq!(tool, "ffmpeg");
				assert!(hint.contains("ffmpeg.org"), "{hint}");
			},
			other => panic!("expected ExternalToolMissing, got {other:?}"),
		}
	}

	#[test]
	fn timeout_scales_with_the_material_but_never_below_the_floor() {
		assert_eq!(timeout_for(0), Duration::from_secs(600));
		// Ten hours of audio: one hour of wall clock.
		assert_eq!(timeout_for(36_000_000), Duration::from_secs(3600));
	}
}
