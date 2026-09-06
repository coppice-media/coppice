//! `calibre-convert` and `calibre-meta`: licence-safe adapters over the
//! calibre command line.
//!
//! calibre is GPL-3, so this crate never links, vendors, or copies any of it.
//! Both tools shell out to the *operator's own* installation and are inert when
//! it is absent. Nothing here removes DRM: `ebook-convert` refuses protected
//! files, and Stump's ingest rejects them up front through the `drm_protected`
//! quality check.
//!
//! Behaviour and flags are taken from the calibre manual:
//! [`ebook-convert`](https://manual.calibre-ebook.com/generated/en/ebook-convert.html)
//! and [`ebook-meta`](https://manual.calibre-ebook.com/generated/en/ebook-meta.html).
//! Prose, the format matrix, and the licence analysis are in
//! `docs/content/docs/developer/calibre-tooling.mdx`.

use std::{
	ffi::OsString,
	fmt,
	fs::File,
	io::{Read, Seek, SeekFrom},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	time::{Duration, Instant},
};

use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};

use crate::{
	Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput, ToolResult,
	Warning,
};

/// Oldest calibre whose CLI this adapter is written against.
///
/// 7.0 is the floor because the option names and the KEPUB output plugin used
/// here are stable from that release onwards; the manual documents 9.14.0
/// ([CLI index](https://manual.calibre-ebook.com/generated/en/cli-index.html)).
pub const MIN_VERSION: (u32, u32) = (7, 0);

const EBOOK_CONVERT: &str = "ebook-convert";
const EBOOK_META: &str = "ebook-meta";

/// Default per-file wall clock. Converting a large PDF is genuinely slow, so
/// the ceiling is generous but always finite.
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// How often a running child is polled for exit.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Trailing bytes kept from a child's stdout and stderr. It is a *tail*: a
/// conversion log can be arbitrarily long, and the useful part of a calibre
/// failure is always its last lines.
const OUTPUT_TAIL: usize = 4096;

/// On macOS the CLI tools live inside the application bundle
/// ([CLI index](https://manual.calibre-ebook.com/generated/en/cli-index.html)).
#[cfg(target_os = "macos")]
const BUNDLE_DIRS: [&str; 1] = ["/Applications/calibre.app/Contents/MacOS"];
#[cfg(not(target_os = "macos"))]
const BUNDLE_DIRS: [&str; 0] = [];

const INSTALL_HINT: &str = "install calibre 7.0 or newer and put its command \
                            line tools on PATH, or set the `bin_dir` option to \
                            the directory holding `ebook-convert`";

// ---------------------------------------------------------------------------
// Locating calibre
// ---------------------------------------------------------------------------

/// A located, version-checked calibre installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calibre {
	/// Directory holding the CLI binaries.
	dir: PathBuf,
	version: (u32, u32, u32),
}

impl Calibre {
	/// Find calibre and verify it is at least [`MIN_VERSION`].
	///
	/// `bin_dir` wins when given; otherwise `PATH` is searched, then the macOS
	/// bundle location. The version comes from `ebook-convert --version`, which
	/// calibre formats as `%prog (calibre <version>)`
	/// ([`OptionParser`](https://github.com/kovidgoyal/calibre/blob/master/src/calibre/utils/config.py)).
	pub fn locate(bin_dir: Option<&Path>) -> ToolResult<Self> {
		let dir = resolve_dir(bin_dir)?;
		let output = run(
			&binary_in(&dir, EBOOK_CONVERT),
			&[OsString::from("--version")],
			Duration::from_secs(30),
		)?;
		if !output.succeeded() {
			return Err(missing(format!(
				"`{EBOOK_CONVERT} --version` failed ({}): {}",
				output.describe_status(),
				first_line(&output.stderr)
			)));
		}
		let combined = format!("{}\n{}", output.stdout, output.stderr);
		let Some(version) = parse_version(&combined) else {
			return Err(missing(format!(
				"could not parse a version from `{EBOOK_CONVERT} --version` \
				 output {:?}",
				first_line(&combined)
			)));
		};
		if (version.0, version.1) < MIN_VERSION {
			return Err(missing(format!(
				"calibre {}.{}.{} is older than the required {}.{}",
				version.0, version.1, version.2, MIN_VERSION.0, MIN_VERSION.1
			)));
		}
		Ok(Self { dir, version })
	}

	pub fn dir(&self) -> &Path {
		&self.dir
	}

	pub fn version(&self) -> (u32, u32, u32) {
		self.version
	}

	pub fn binary(&self, name: &str) -> PathBuf {
		binary_in(&self.dir, name)
	}

	fn describe(&self) -> String {
		let (major, minor, patch) = self.version;
		format!("calibre {major}.{minor}.{patch} in {}", self.dir.display())
	}
}

fn missing(reason: String) -> ToolError {
	ToolError::ExternalToolMissing {
		tool: "calibre".to_string(),
		reason,
		hint: INSTALL_HINT.to_string(),
	}
}

fn binary_in(dir: &Path, name: &str) -> PathBuf {
	if cfg!(windows) {
		dir.join(format!("{name}.exe"))
	} else {
		dir.join(name)
	}
}

fn resolve_dir(bin_dir: Option<&Path>) -> ToolResult<PathBuf> {
	if let Some(dir) = bin_dir {
		if is_executable(&binary_in(dir, EBOOK_CONVERT)) {
			return Ok(dir.to_path_buf());
		}
		return Err(missing(format!(
			"no executable `{EBOOK_CONVERT}` in the configured bin_dir {}",
			dir.display()
		)));
	}

	let path = std::env::var_os("PATH").unwrap_or_default();
	let candidates = std::env::split_paths(&path)
		.chain(BUNDLE_DIRS.iter().map(PathBuf::from))
		.filter(|dir| !dir.as_os_str().is_empty());
	for dir in candidates {
		if is_executable(&binary_in(&dir, EBOOK_CONVERT)) {
			return Ok(dir);
		}
	}
	Err(missing(format!("`{EBOOK_CONVERT}` was not found on PATH")))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
	use std::os::unix::fs::PermissionsExt;

	std::fs::metadata(path).is_ok_and(|metadata| {
		metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
	})
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
	std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Version out of an `ebook-convert --version` banner.
///
/// The documented shape is `%prog (calibre <version>)`, so the search is
/// anchored just after the literal `calibre ` when it is present; a wrapper
/// script that prints a versioned path first would otherwise win. Without the
/// anchor it falls back to the first `major.minor[.patch]` triple anywhere.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
	let anchor = text
		.to_ascii_lowercase()
		.find("calibre ")
		.map(|at| at + "calibre ".len())
		.unwrap_or(0);
	first_triple(&text[anchor..]).or_else(|| first_triple(text))
}

fn first_triple(text: &str) -> Option<(u32, u32, u32)> {
	let bytes = text.as_bytes();
	let mut index = 0;
	while index < bytes.len() {
		if !bytes[index].is_ascii_digit() {
			index += 1;
			continue;
		}
		let start = index;
		while index < bytes.len() && bytes[index].is_ascii_digit() {
			index += 1;
		}
		let mut parts = vec![&text[start..index]];
		while index < bytes.len()
			&& bytes[index] == b'.'
			&& bytes.get(index + 1).is_some_and(u8::is_ascii_digit)
		{
			index += 1;
			let start = index;
			while index < bytes.len() && bytes[index].is_ascii_digit() {
				index += 1;
			}
			parts.push(&text[start..index]);
		}
		if parts.len() >= 2 {
			let number = |at: usize| parts.get(at).and_then(|p| p.parse().ok());
			return Some((number(0)?, number(1)?, number(2).unwrap_or(0)));
		}
	}
	None
}

// ---------------------------------------------------------------------------
// Child process handling
// ---------------------------------------------------------------------------

/// What a calibre invocation produced.
#[derive(Debug, Clone)]
pub struct Output {
	pub status: Option<i32>,
	pub timed_out: bool,
	pub stdout: String,
	pub stderr: String,
}

impl Output {
	pub fn succeeded(&self) -> bool {
		!self.timed_out && self.status == Some(0)
	}

	fn describe_status(&self) -> String {
		match (self.timed_out, self.status) {
			(true, _) => "timed out".to_string(),
			(_, Some(code)) => format!("exit code {code}"),
			(_, None) => "killed by a signal".to_string(),
		}
	}
}

/// Run `program` with `args`, killing it after `timeout`.
///
/// stdout and stderr are redirected to temporary files rather than pipes: a
/// polling parent that reads pipes only after exit would deadlock as soon as a
/// chatty conversion filled the pipe buffer.
fn run(program: &Path, args: &[OsString], timeout: Duration) -> ToolResult<Output> {
	let mut stdout = tempfile::tempfile()?;
	let mut stderr = tempfile::tempfile()?;
	let mut child = Command::new(program)
		.args(args)
		.stdin(Stdio::null())
		.stdout(Stdio::from(stdout.try_clone()?))
		.stderr(Stdio::from(stderr.try_clone()?))
		.spawn()
		.map_err(|error| {
			if error.kind() == std::io::ErrorKind::NotFound {
				missing(format!("{} could not be executed", program.display()))
			} else {
				ToolError::Io(error)
			}
		})?;

	let deadline = Instant::now() + timeout;
	let mut timed_out = false;
	let status = loop {
		match child.try_wait()? {
			Some(status) => break status.code(),
			None if Instant::now() >= deadline => {
				timed_out = true;
				let _ = child.kill();
				let _ = child.wait();
				break None;
			},
			None => std::thread::sleep(POLL_INTERVAL),
		}
	};

	Ok(Output {
		status,
		timed_out,
		stdout: read_tail(&mut stdout)?,
		stderr: read_tail(&mut stderr)?,
	})
}

fn read_tail(file: &mut File) -> ToolResult<String> {
	let len = file.seek(SeekFrom::End(0))?;
	let take = len.min(OUTPUT_TAIL as u64);
	file.seek(SeekFrom::Start(len - take))?;
	let mut bytes = Vec::with_capacity(take as usize);
	file.read_to_end(&mut bytes)?;
	Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

fn first_line(text: &str) -> &str {
	text.lines()
		.map(str::trim)
		.find(|line| !line.is_empty())
		.unwrap_or("")
}

// ---------------------------------------------------------------------------
// calibre-convert
// ---------------------------------------------------------------------------

/// Output formats this adapter will ask `ebook-convert` for.
///
/// Deliberately a closed set: every value is a format Stump can serve or a
/// device format an operator asked for, and each maps to one output extension
/// that `ebook-convert` dispatches on (it guesses the output plugin from the
/// extension of the output file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetFormat {
	#[default]
	Epub,
	/// Kobo's EPUB dialect. `ebook-convert` selects it from a `.kepub`
	/// extension; Kobo sideloading then wants the file renamed to
	/// `.kepub.epub`. Prefer the in-process `stump_kepub` converter for
	/// EPUB sources -- this exists for sources it cannot read.
	Kepub,
	Azw3,
	Mobi,
	Pdf,
}

impl TargetFormat {
	pub fn extension(self) -> &'static str {
		match self {
			TargetFormat::Epub => "epub",
			TargetFormat::Kepub => "kepub",
			TargetFormat::Azw3 => "azw3",
			TargetFormat::Mobi => "mobi",
			TargetFormat::Pdf => "pdf",
		}
	}
}

impl fmt::Display for TargetFormat {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.extension())
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct ConvertOptions {
	pub to: TargetFormat,
	/// Where converted files are written; defaults to each source's directory.
	pub output_dir: Option<PathBuf>,
	/// Verbatim extra `ebook-convert` arguments, e.g.
	/// `["--smarten-punctuation"]`.
	pub extra_args: Vec<String>,
	/// Directory holding the calibre binaries; `PATH` is searched when unset.
	pub bin_dir: Option<PathBuf>,
	pub timeout_secs: u64,
	/// Replace an existing target instead of skipping it.
	pub overwrite: bool,
}

impl Default for ConvertOptions {
	fn default() -> Self {
		Self {
			to: TargetFormat::default(),
			output_dir: None,
			extra_args: Vec::new(),
			bin_dir: None,
			timeout_secs: DEFAULT_TIMEOUT_SECS,
			overwrite: false,
		}
	}
}

/// Everything `apply` needs to replay one planned conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConvertDetail {
	to: TargetFormat,
	#[serde(default)]
	extra_args: Vec<String>,
	#[serde(default)]
	bin_dir: Option<PathBuf>,
	timeout_secs: u64,
	#[serde(default)]
	overwrite: bool,
}

/// Convert e-books with `ebook-convert`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CalibreConvert;

impl Tool for CalibreConvert {
	fn id(&self) -> &'static str {
		"calibre-convert"
	}

	fn describe(&self) -> &'static str {
		"Convert e-books between formats with the operator's calibre (ebook-convert)"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<ConvertOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths".to_string()));
		}
		let calibre = Calibre::locate(options.bin_dir.as_deref())?;

		let mut plan = Plan::new(self.id());
		plan.warn(
			Warning::new("calibre-version", calibre.describe())
				.with_severity(Severity::Info),
		);
		if let Some(dir) = &options.output_dir {
			if !dir.is_dir() {
				return Err(ToolError::Invalid(format!(
					"output_dir {} is not a directory",
					dir.display()
				)));
			}
		}

		let detail = ConvertDetail {
			to: options.to,
			extra_args: options.extra_args.clone(),
			bin_dir: Some(calibre.dir().to_path_buf()),
			timeout_secs: options.timeout_secs.max(1),
			overwrite: options.overwrite,
		};

		for source in &input.paths {
			if !source.is_file() {
				plan.warn(
					Warning::new("not-a-file", "input is not a readable file")
						.at(source)
						.with_severity(Severity::Error),
				);
				continue;
			}
			let same_format = source
				.extension()
				.is_some_and(|ext| ext.eq_ignore_ascii_case(options.to.extension()));
			if same_format {
				plan.warn(
					Warning::new(
						"already-target-format",
						format!("already a .{} file", options.to),
					)
					.at(source)
					.with_severity(Severity::Info),
				);
				continue;
			}
			let Some(stem) = source.file_stem() else {
				plan.warn(
					Warning::new("no-file-stem", "input has no file name")
						.at(source)
						.with_severity(Severity::Error),
				);
				continue;
			};
			let directory = options
				.output_dir
				.clone()
				.or_else(|| source.parent().map(Path::to_path_buf))
				.unwrap_or_default();
			let mut target = directory.join(stem);
			target.set_extension(options.to.extension());
			if target == *source {
				plan.warn(
					Warning::new(
						"target-is-source",
						"conversion would overwrite the original",
					)
					.at(source)
					.with_severity(Severity::Error),
				);
				continue;
			}
			if target.exists() && !options.overwrite {
				plan.warn(
					Warning::new(
						"target-exists",
						format!("{} already exists", target.display()),
					)
					.at(source),
				);
				continue;
			}
			plan.push(
				Action::new("convert")
					.with_source(source)
					.with_target(&target)
					.with_detail(serde_json::to_value(&detail)?),
			);
		}
		Ok(plan)
	}

	fn apply(
		&self,
		plan: &Plan,
		sink: &mut dyn ProgressSink,
	) -> Result<Report, ToolError> {
		plan.expect_tool(self.id())?;
		let mut report = Report::for_plan(plan);
		let total = plan.actions.len();
		let mut calibre = None;

		for (index, action) in plan.actions.iter().enumerate() {
			let (Some(source), Some(target)) = (&action.source, &action.target) else {
				report.skipped(action.clone(), "action has no source or target");
				continue;
			};
			let detail = serde_json::from_value::<ConvertDetail>(action.detail.clone())
				.map_err(|error| ToolError::Options(error.to_string()))?;

			sink.progress(index, total, &format!("converting {}", source.display()));

			if !source.is_file() {
				report.skipped(action.clone(), "source disappeared");
				continue;
			}
			if target.exists() && !detail.overwrite {
				report.skipped(
					action.clone(),
					format!("{} already exists", target.display()),
				);
				continue;
			}

			let calibre = match &calibre {
				Some(calibre) => calibre,
				None => {
					calibre = Some(Calibre::locate(detail.bin_dir.as_deref())?);
					calibre.as_ref().expect("just located")
				},
			};

			let mut args = vec![source.as_os_str().to_owned(), target.into()];
			args.extend(detail.extra_args.iter().map(OsString::from));
			let output = run(
				&calibre.binary(EBOOK_CONVERT),
				&args,
				Duration::from_secs(detail.timeout_secs),
			)?;

			if output.succeeded() && target.is_file() {
				if !output.stderr.is_empty() {
					report.warn(
						Warning::new("calibre-stderr", output.stderr.clone())
							.at(source)
							.with_severity(Severity::Info),
					);
				}
				report.applied(action.clone());
				continue;
			}

			// A failed or half-finished conversion must not leave a broken
			// book behind; the planner already refused pre-existing targets.
			if target.exists() {
				let _ = std::fs::remove_file(target);
			}
			let reason = if output.succeeded() {
				format!("{EBOOK_CONVERT} reported success but wrote no output")
			} else {
				format!(
					"{EBOOK_CONVERT} {}: {}",
					output.describe_status(),
					// Both streams are already capped to OUTPUT_TAIL.
					if output.stderr.is_empty() {
						&output.stdout
					} else {
						&output.stderr
					}
				)
			};
			report.skipped(action.clone(), reason);
		}
		sink.progress(total, total, "done");
		Ok(report)
	}
}

// ---------------------------------------------------------------------------
// calibre-meta
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct MetaOptions {
	pub bin_dir: Option<PathBuf>,
	pub timeout_secs: u64,
}

impl Default for MetaOptions {
	fn default() -> Self {
		Self {
			bin_dir: None,
			timeout_secs: 120,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MetaDetail {
	#[serde(default)]
	bin_dir: Option<PathBuf>,
	timeout_secs: u64,
}

/// Metadata read out of a book, shaped like `ExternalMediaMetadata` so a
/// consumer can merge it with provider candidates without a translation layer.
///
/// `publisher` and `language` have no `ExternalMediaMetadata` counterpart and
/// are additions; every other name matches
/// `crates/integrations/metadata/src/types/metadata.rs`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CalibreMetadata {
	pub provider: String,
	pub external_id: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub summary: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub series_name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub number: Option<f32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub day: Option<i32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub month: Option<i32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub year: Option<i32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tags: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub isbn: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub isbn_13: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub writers: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub publisher: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub language: Option<String>,
}

/// Read embedded metadata with `ebook-meta --to-opf`.
///
/// Read-only by design: this pass never writes metadata back into a book.
#[derive(Debug, Default, Clone, Copy)]
pub struct CalibreMeta;

impl Tool for CalibreMeta {
	fn id(&self) -> &'static str {
		"calibre-meta"
	}

	fn describe(&self) -> &'static str {
		"Read embedded e-book metadata with the operator's calibre (ebook-meta)"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<MetaOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths".to_string()));
		}
		let calibre = Calibre::locate(options.bin_dir.as_deref())?;

		let mut plan = Plan::new(self.id());
		plan.warn(
			Warning::new("calibre-version", calibre.describe())
				.with_severity(Severity::Info),
		);
		let detail = MetaDetail {
			bin_dir: Some(calibre.dir().to_path_buf()),
			timeout_secs: options.timeout_secs.max(1),
		};
		for source in &input.paths {
			if !source.is_file() {
				plan.warn(
					Warning::new("not-a-file", "input is not a readable file")
						.at(source)
						.with_severity(Severity::Error),
				);
				continue;
			}
			plan.push(
				Action::new("read-metadata")
					.with_source(source)
					.with_detail(serde_json::to_value(&detail)?),
			);
		}
		Ok(plan)
	}

	fn apply(
		&self,
		plan: &Plan,
		sink: &mut dyn ProgressSink,
	) -> Result<Report, ToolError> {
		plan.expect_tool(self.id())?;
		let mut report = Report::for_plan(plan);
		let total = plan.actions.len();
		let workspace = tempfile::tempdir()?;
		let mut calibre = None;

		for (index, action) in plan.actions.iter().enumerate() {
			let Some(source) = &action.source else {
				report.skipped(action.clone(), "action has no source");
				continue;
			};
			let detail = serde_json::from_value::<MetaDetail>(action.detail.clone())
				.map_err(|error| ToolError::Options(error.to_string()))?;
			sink.progress(index, total, &format!("reading {}", source.display()));

			let calibre = match &calibre {
				Some(calibre) => calibre,
				None => {
					calibre = Some(Calibre::locate(detail.bin_dir.as_deref())?);
					calibre.as_ref().expect("just located")
				},
			};

			let opf = workspace.path().join(format!("{index}.opf"));
			let output = run(
				&calibre.binary(EBOOK_META),
				&[
					source.as_os_str().to_owned(),
					OsString::from("--to-opf"),
					opf.as_os_str().to_owned(),
				],
				Duration::from_secs(detail.timeout_secs),
			)?;
			if !output.succeeded() {
				report.skipped(
					action.clone(),
					format!(
						"{EBOOK_META} {}: {}",
						output.describe_status(),
						output.stderr
					),
				);
				continue;
			}
			let bytes = match std::fs::read(&opf) {
				Ok(bytes) => bytes,
				Err(error) => {
					report.skipped(
						action.clone(),
						format!("{EBOOK_META} wrote no OPF: {error}"),
					);
					continue;
				},
			};
			let metadata = parse_opf(&bytes, source)?;
			report.applied(action.clone().with_detail(serde_json::to_value(&metadata)?));
		}
		sink.progress(total, total, "done");
		Ok(report)
	}
}

/// Parse a calibre OPF 2.0 metadata document.
///
/// Series lives in `<meta name="calibre:series">` / `calibre:series_index`,
/// which is how calibre serializes it
/// ([`opf2.py`](https://github.com/kovidgoyal/calibre/blob/master/src/calibre/ebooks/metadata/opf2.py));
/// everything else is Dublin Core.
fn parse_opf(bytes: &[u8], source: &Path) -> ToolResult<CalibreMetadata> {
	let mut metadata = CalibreMetadata {
		provider: "calibre".to_string(),
		external_id: source
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_default(),
		..Default::default()
	};
	let mut tags = Vec::new();
	let mut writers = Vec::new();

	let mut reader = Reader::from_reader(bytes);
	let mut buffer = Vec::new();
	// The element currently open, plus its `scheme`/`opf:scheme` attribute.
	let mut open: Option<(Vec<u8>, Option<String>)> = None;
	// Still-escaped text of the open element. quick-xml reports every entity
	// reference as its own `GeneralRef` event, so `<dc:description>a &amp;
	// b</dc:description>` arrives as three events; assigning the first
	// `Text` alone would silently truncate the value at the ampersand.
	let mut raw = Vec::new();
	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(event) => {
				let name = event.local_name().as_ref().to_vec();
				let scheme = attribute(&event, b"scheme")?;
				open = Some((name, scheme));
				raw.clear();
			},
			Event::Empty(event) => {
				if event.local_name().as_ref() == b"meta" {
					let key = attribute(&event, b"name")?.unwrap_or_default();
					let value = attribute(&event, b"content")?.unwrap_or_default();
					match key.as_str() {
						"calibre:series" if !value.is_empty() => {
							metadata.series_name = Some(value);
						},
						"calibre:series_index" => {
							metadata.number = value.parse().ok();
						},
						_ => {},
					}
				}
			},
			Event::Text(text) if open.is_some() => {
				raw.extend_from_slice(&text.into_inner());
			},
			Event::GeneralRef(reference) if open.is_some() => {
				// Put the reference back together so the single unescape
				// pass below resolves it, numeric (`&#39;`) included.
				raw.push(b'&');
				raw.extend_from_slice(&reference.into_inner());
				raw.push(b';');
			},
			Event::End(_) => {
				let Some((name, scheme)) = open.take() else {
					continue;
				};
				let decoded = String::from_utf8_lossy(&raw);
				let value = quick_xml::escape::unescape(&decoded)
					.map(|unescaped| unescaped.into_owned())
					.unwrap_or_else(|_| decoded.into_owned());
				let value = value.trim().to_string();
				raw.clear();
				if value.is_empty() {
					continue;
				}
				match name.as_slice() {
					b"title" => metadata.title.get_or_insert(value),
					b"creator" => {
						writers.push(value);
						continue;
					},
					b"description" => metadata.summary.get_or_insert(value),
					b"publisher" => metadata.publisher.get_or_insert(value),
					b"language" => metadata.language.get_or_insert(value),
					b"subject" => {
						tags.push(value);
						continue;
					},
					b"date" => {
						let (year, month, day) = parse_date(&value);
						metadata.year = year;
						metadata.month = month;
						metadata.day = day;
						continue;
					},
					b"identifier" => {
						let scheme =
							scheme.as_deref().unwrap_or_default().to_ascii_lowercase();
						if scheme == "isbn" {
							let digits = value
								.chars()
								.filter(char::is_ascii_alphanumeric)
								.collect::<String>();
							if digits.len() == 13 {
								metadata.isbn_13 = Some(digits);
							} else {
								metadata.isbn = Some(digits);
							}
						}
						continue;
					},
					_ => continue,
				};
			},
			Event::Eof => break,
			_ => {},
		}
		buffer.clear();
	}

	if !tags.is_empty() {
		metadata.tags = Some(tags);
	}
	if !writers.is_empty() {
		metadata.writers = Some(writers);
	}
	Ok(metadata)
}

/// `scheme` matched on the local name, so both `scheme` and `opf:scheme` hit.
fn attribute(
	event: &quick_xml::events::BytesStart<'_>,
	name: &[u8],
) -> ToolResult<Option<String>> {
	for attribute in event.attributes() {
		let attribute = attribute.map_err(quick_xml::Error::from)?;
		if attribute.key.local_name().as_ref() == name {
			return Ok(Some(attribute.unescape_value()?.trim().to_string()));
		}
	}
	Ok(None)
}

/// Leading `YYYY[-MM[-DD]]` of an OPF date.
fn parse_date(value: &str) -> (Option<i32>, Option<i32>, Option<i32>) {
	let mut parts = value
		.split(['-', 'T', ' '])
		.map(|part| part.parse::<i32>().ok());
	let year = parts.next().flatten();
	if year.is_none() {
		return (None, None, None);
	}
	(year, parts.next().flatten(), parts.next().flatten())
}

#[cfg(test)]
mod tests;
