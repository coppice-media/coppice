//! `boko-convert`: a licence-safe adapter over the `boko` command line.
//!
//! [boko](https://github.com/zacharydenton/boko) is GPL-3.0-or-later, so this
//! crate never links, vendors, or copies any of it: the tool shells out to the
//! *operator's own* installation and is inert when it is absent. Nothing here
//! removes DRM — boko refuses encrypted input, and Stump's ingest rejects
//! protected files up front through the `drm_protected` quality check.
//!
//! boko is the **preferred** converter for the Kindle formats and
//! [`crate::calibre::CalibreConvert`] is the fallback: boko is the only KFX
//! writer that does not drive Amazon's Kindle Previewer, it needs no plugin,
//! and it converts natively on a headless server (the fixture EPUB becomes
//! AZW3 in ~60 ms). calibre stays for everything boko cannot do at all: MOBI
//! output, PDF, DOCX, FB2, LIT, PDB, CBZ/CBR.
//!
//! Invocation, formats, and flags are taken from `boko --help` /
//! `boko convert --help` of a real **0.5.0** install and from the format matrix
//! in the project README (<https://github.com/zacharydenton/boko#formats>); the
//! exact command line and the verification runs are recorded in
//! `crates/tools/README.md`. Prose and the calibre comparison live in
//! `docs/content/docs/developer/calibre-tooling.mdx`.

use std::{
	ffi::OsString,
	fmt,
	path::{Path, PathBuf},
	time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
	external::{ExternalTool, MinVersion},
	Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput, ToolResult,
	Warning,
};

/// Oldest boko whose CLI this adapter is written against.
///
/// 0.5.0 is the floor because it is the release the `boko convert <INPUT>
/// [OUTPUT]` shape, the `-t/--to` format list and the KFX writer were verified
/// against; a pre-1.0 crate can rename a subcommand in a minor release, so the
/// whole triple is compared, not just `major.minor`.
pub const MIN_VERSION: (u32, u32, u32) = (0, 5, 0);

const BOKO: &str = "boko";

/// The `boko` subcommand this adapter drives (`boko --help`: "convert —
/// Convert between ebook formats").
const CONVERT: &str = "convert";

/// Always passed. boko reports progress (`Converting <in> -> <out>` / `Done.`)
/// on **stderr**, so without `--quiet` every successful conversion would come
/// back carrying captured "error output"; `--quiet` suppresses only those
/// progress lines, and a real `error: ...` diagnostic still arrives on stderr.
/// A non-empty stderr therefore means something actually happened.
const QUIET: &str = "--quiet";

/// Default per-file wall clock. boko is a native converter — the 32 KiB
/// fixture EPUB becomes AZW3 in ~60 ms — so this is a runaway guard, not a
/// budget, but it is always finite.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

const INSTALL_HINT: &str = "install boko 0.5.0 or newer with `cargo install \
                            boko` and put it on PATH, or set the `bin_dir` \
                            option to the directory holding `boko`";

// ---------------------------------------------------------------------------
// Locating boko
// ---------------------------------------------------------------------------

/// boko as the crate knows every external tool ([`crate::external`]): one
/// binary, clap's `boko <version>` banner, the whole [`MIN_VERSION`] triple as
/// the floor, and `PATH` followed by the `cargo install` destination. Sharing
/// that machinery with the calibre adapters is why only boko's own name,
/// banner anchor and install hint live here.
static BOKO_TOOL: ExternalTool = ExternalTool {
	name: BOKO,
	probe: BOKO,
	version_arg: "--version",
	anchor: "boko ",
	min: MinVersion::Triple(MIN_VERSION.0, MIN_VERSION.1, MIN_VERSION.2),
	extra_dirs: cargo_bin_dirs,
	searched: "PATH or in the cargo bin directory",
	hint: INSTALL_HINT,
};

/// A located, version-checked boko installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boko {
	/// Directory holding the `boko` binary.
	dir: PathBuf,
	version: (u32, u32, u32),
}

impl Boko {
	/// Find boko and verify it is at least [`MIN_VERSION`].
	///
	/// `bin_dir` wins when given; otherwise `PATH` is searched, then the
	/// `cargo install` destination (see [`cargo_bin_dirs`]). The version comes
	/// from `boko --version`, which clap renders as `boko <version>`.
	pub fn locate(bin_dir: Option<&Path>) -> ToolResult<Self> {
		let install = BOKO_TOOL.locate(bin_dir)?;
		Ok(Self {
			dir: install.dir,
			version: install.version,
		})
	}

	pub fn dir(&self) -> &Path {
		&self.dir
	}

	pub fn version(&self) -> (u32, u32, u32) {
		self.version
	}

	pub fn binary(&self) -> PathBuf {
		BOKO_TOOL.binary(&self.dir, BOKO)
	}

	fn describe(&self) -> String {
		let (major, minor, patch) = self.version;
		format!("boko {major}.{minor}.{patch} in {}", self.dir.display())
	}
}

/// Where `cargo install boko` puts the binary: `$CARGO_HOME/bin`, defaulting
/// to `~/.cargo/bin`
/// ([cargo book](https://doc.rust-lang.org/cargo/commands/cargo-install.html)).
///
/// boko has no distribution package, so `cargo install` is the documented way
/// to get it — and a server started by a service manager rarely carries that
/// directory in `PATH`. Probing it turns "works in my shell, missing in the
/// daemon" into a working default instead of a support question.
fn cargo_bin_dirs() -> Vec<PathBuf> {
	let mut dirs = Vec::new();
	if let Some(cargo_home) = std::env::var_os("CARGO_HOME") {
		dirs.push(PathBuf::from(cargo_home).join("bin"));
	}
	let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
	if let Some(home) = home {
		dirs.push(PathBuf::from(home).join(".cargo").join("bin"));
	}
	dirs
}

/// Version out of a `boko --version` banner: the crate-wide banner rule
/// anchored on clap's `boko <version>` wording. Behaviour lives in
/// [`ExternalTool::parse_version`]; this is the name the module's tests pin it
/// through.
#[cfg(test)]
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
	BOKO_TOOL.parse_version(text)
}

// ---------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------

/// Output formats `boko-convert` accepts.
///
/// `boko convert` infers its writer from the output file's extension
/// ("Output format. Inferred from output extension if not specified",
/// `boko convert --help`), so every value is exactly one extension.
///
/// `Mobi` is accepted by the option parser and then refused with an
/// explanation: boko *reads* MOBI but never writes it — `boko convert in.epub
/// out.mobi` exits 1 with `error: MOBI output is not supported; use .azw3
/// instead`, and the project's format matrix says the same
/// (<https://github.com/zacharydenton/boko#formats>). Deserializing it rather
/// than rejecting the word means the operator gets that sentence and the
/// `calibre-convert` pointer instead of a serde error naming three variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetFormat {
	#[default]
	Epub,
	/// KF8, the Kindle sideload format: what `KindleExport` produces.
	Azw3,
	/// The format current Kindles render with hyphenation, kerning and
	/// ligatures, and which no other free tool can write.
	Kfx,
	/// Read-only for boko; planning it is refused, see the type docs.
	Mobi,
}

impl TargetFormat {
	pub fn extension(self) -> &'static str {
		match self {
			TargetFormat::Epub => "epub",
			TargetFormat::Azw3 => "azw3",
			TargetFormat::Kfx => "kfx",
			TargetFormat::Mobi => "mobi",
		}
	}

	/// False for the one format boko refuses as output.
	pub fn writable(self) -> bool {
		!matches!(self, TargetFormat::Mobi)
	}
}

impl fmt::Display for TargetFormat {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.extension())
	}
}

/// The formats boko can read, identified by extension.
///
/// Markdown and plain text are write-only (`boko convert in.md out.epub` exits
/// 1 with `error: Markdown cannot be used as input format`), and boko reads no
/// container Stump would hand it otherwise, so an unlisted extension is
/// reported at plan time instead of costing a failed child per book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceFormat {
	Epub,
	Azw3,
	Mobi,
	Kfx,
}

impl SourceFormat {
	/// Every readable format, in the order the project README lists them.
	pub const ALL: [SourceFormat; 4] = [
		SourceFormat::Kfx,
		SourceFormat::Azw3,
		SourceFormat::Epub,
		SourceFormat::Mobi,
	];

	pub fn extension(self) -> &'static str {
		match self {
			SourceFormat::Epub => "epub",
			SourceFormat::Azw3 => "azw3",
			SourceFormat::Mobi => "mobi",
			SourceFormat::Kfx => "kfx",
		}
	}

	/// The format of `path` by extension, case-insensitively; `None` when boko
	/// cannot read it.
	pub fn from_path(path: &Path) -> Option<Self> {
		let extension = path.extension()?.to_str()?;
		Self::ALL
			.into_iter()
			.find(|format| extension.eq_ignore_ascii_case(format.extension()))
	}

	fn readable_list() -> String {
		Self::ALL
			.iter()
			.map(|format| format!(".{format}"))
			.collect::<Vec<_>>()
			.join(", ")
	}
}

impl fmt::Display for SourceFormat {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.extension())
	}
}

// ---------------------------------------------------------------------------
// boko-convert
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct ConvertOptions {
	pub to: TargetFormat,
	/// Where converted files are written; defaults to each source's directory.
	pub output_dir: Option<PathBuf>,
	/// Verbatim extra `boko convert` arguments, e.g. `["--optimize"]`.
	pub extra_args: Vec<String>,
	/// Directory holding the `boko` binary; `PATH` and the cargo bin directory
	/// are searched when unset.
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

/// Everything `apply` needs to replay one planned conversion, including the
/// format pair the plan showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConvertDetail {
	from: SourceFormat,
	to: TargetFormat,
	#[serde(default)]
	extra_args: Vec<String>,
	#[serde(default)]
	bin_dir: Option<PathBuf>,
	timeout_secs: u64,
	#[serde(default)]
	overwrite: bool,
}

/// The send-to-Kindle path: an EPUB in, an AZW3 out.
///
/// AZW3 (KF8) is what a Kindle accepts as a sideloaded file over USB and what
/// Amazon's Send to Kindle service takes as an attachment, so `to: "azw3"` from
/// an EPUB source is the one conversion an operator asks for by device rather
/// than by format. It is not a separate tool: it is [`BokoConvert`] with the
/// pair pinned, and the planner flags every such action with an informational
/// [`KindleExport::WARNING_CODE`] warning so the CLI can say so out loud.
///
/// `to: "kfx"` is the better *reading* experience on a current Kindle
/// (hyphenation, kerning, ligatures); AZW3 is the safer sideload, because every
/// Kindle since the Paperwhite 1 accepts it.
#[derive(Debug, Default, Clone, Copy)]
pub struct KindleExport;

impl KindleExport {
	/// Warning code the planner attaches to a send-to-Kindle action.
	pub const WARNING_CODE: &'static str = "kindle-export";

	/// [`ConvertOptions`] for the send-to-Kindle path.
	pub fn options(output_dir: Option<PathBuf>) -> ConvertOptions {
		ConvertOptions {
			to: TargetFormat::Azw3,
			output_dir,
			..ConvertOptions::default()
		}
	}

	/// A ready-made [`BokoConvert`] input for `paths`, so a caller never has to
	/// hand-write the options blob.
	pub fn input(
		paths: Vec<PathBuf>,
		output_dir: Option<PathBuf>,
	) -> ToolResult<ToolInput> {
		let options = serde_json::to_value(Self::options(output_dir))?;
		Ok(ToolInput::new(paths).with_options(options))
	}

	/// True when this format pair is the send-to-Kindle path.
	pub fn matches(from: SourceFormat, to: TargetFormat) -> bool {
		from == SourceFormat::Epub && to == TargetFormat::Azw3
	}
}

/// Convert e-books to and from the Kindle formats with `boko`.
#[derive(Debug, Default, Clone, Copy)]
pub struct BokoConvert;

impl Tool for BokoConvert {
	fn id(&self) -> &'static str {
		"boko-convert"
	}

	fn describe(&self) -> &'static str {
		"Convert e-books between EPUB, AZW3/KF8 and KFX with the operator's boko"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<ConvertOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths".to_string()));
		}
		// Refused before boko is even looked for: it is a property of the
		// request, not of the installation, and it is the whole run.
		if !options.to.writable() {
			return Err(ToolError::Invalid(format!(
				"boko reads .{format} but cannot write it (`boko convert` \
				 exits with \"MOBI output is not supported; use .azw3 \
				 instead\"); use `to: \"azw3\"` for a Kindle, or the \
				 `calibre-convert` tool when .{format} really is required",
				format = options.to
			)));
		}
		let boko = Boko::locate(options.bin_dir.as_deref())?;

		let mut plan = Plan::new(self.id());
		plan.warn(
			Warning::new("boko-version", boko.describe()).with_severity(Severity::Info),
		);
		if let Some(dir) = &options.output_dir {
			if !dir.is_dir() {
				return Err(ToolError::Invalid(format!(
					"output_dir {} is not a directory",
					dir.display()
				)));
			}
		}

		for source in &input.paths {
			if !source.is_file() {
				plan.warn(
					Warning::new("not-a-file", "input is not a readable file")
						.at(source)
						.with_severity(Severity::Error),
				);
				continue;
			}
			let Some(from) = SourceFormat::from_path(source) else {
				plan.warn(
					Warning::new(
						"unsupported-source",
						format!(
							"boko reads only {}; use the `calibre-convert` \
							 tool for anything else",
							SourceFormat::readable_list()
						),
					)
					.at(source)
					.with_severity(Severity::Error),
				);
				continue;
			};
			if from.extension() == options.to.extension() {
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
			if KindleExport::matches(from, options.to) {
				plan.warn(
					Warning::new(
						KindleExport::WARNING_CODE,
						format!(
							"{} is the send-to-Kindle format: sideload it \
							 over USB or mail it to a Kindle address",
							target.display()
						),
					)
					.at(source)
					.with_severity(Severity::Info),
				);
			}
			let detail = ConvertDetail {
				from,
				to: options.to,
				extra_args: options.extra_args.clone(),
				bin_dir: Some(boko.dir().to_path_buf()),
				timeout_secs: options.timeout_secs.max(1),
				overwrite: options.overwrite,
			};
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
		let mut boko = None;

		for (index, action) in plan.actions.iter().enumerate() {
			let (Some(source), Some(target)) = (&action.source, &action.target) else {
				report.skipped(action.clone(), "action has no source or target");
				continue;
			};
			let detail = serde_json::from_value::<ConvertDetail>(action.detail.clone())
				.map_err(|error| ToolError::Options(error.to_string()))?;

			sink.progress(
				index,
				total,
				&format!("{} -> {}: {}", detail.from, detail.to, source.display()),
			);

			if !source.is_file() {
				report.skipped(action.clone(), "source disappeared");
				continue;
			}
			// A plan can be minutes old, and `overwrite` is the only licence
			// to replace a book that exists now.
			if target.exists() && !detail.overwrite {
				report.skipped(
					action.clone(),
					format!("{} already exists", target.display()),
				);
				continue;
			}

			let boko = match &boko {
				Some(boko) => boko,
				None => {
					boko = Some(Boko::locate(detail.bin_dir.as_deref())?);
					boko.as_ref().expect("just located")
				},
			};

			let mut args = vec![
				OsString::from(CONVERT),
				OsString::from(QUIET),
				source.as_os_str().to_owned(),
				target.into(),
			];
			args.extend(detail.extra_args.iter().map(OsString::from));
			let output = BOKO_TOOL.run(
				&boko.binary(),
				&args,
				Duration::from_secs(detail.timeout_secs),
			)?;

			if output.succeeded() && target.is_file() {
				if !output.stderr.is_empty() {
					report.warn(
						Warning::new("boko-stderr", output.stderr.clone())
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
				format!("{BOKO} {CONVERT} reported success but wrote no output")
			} else {
				format!(
					"{BOKO} {CONVERT} {}: {}",
					output.describe_status(),
					// Both streams are already capped to a tail.
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

#[cfg(test)]
mod tests;
