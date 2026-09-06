//! `calibre-polish`: a licence-safe adapter over calibre's `ebook-polish`.
//!
//! calibre is GPL-3, so nothing here links, vendors, or copies any of it: the
//! tool shells out to the *operator's own* installation through
//! [`crate::calibre`] and is inert when that installation is absent or older
//! than [`crate::calibre::MIN_VERSION`]. Polishing removes no DRM and cannot:
//! it only rewrites the internal code of a book calibre can already open, and
//! Stump's ingest rejects protected files up front through the `drm_protected`
//! quality check.
//!
//! Behaviour and every flag come from the calibre manual
//! ([`ebook-polish`](https://manual.calibre-ebook.com/generated/en/ebook-polish.html)),
//! which documents the invocation as
//! `ebook-polish [options] input_file [output_file]` and states that
//! "polishing only works on files in the AZW3 or EPUB or KEPUB formats". The
//! output file is optional there; this adapter always passes it, so a run can
//! never rewrite the input under calibre's control. `in_place` still ends with
//! the polished bytes in the original path, but only through a backup plus a
//! rename of bytes calibre has finished writing beside it.
//!
//! Prose, the format matrix, and the licence analysis are in
//! `docs/content/docs/developer/calibre-tooling.mdx`.

use std::{
	collections::HashSet,
	ffi::OsString,
	path::{Path, PathBuf},
	time::Duration,
};

use serde::{Deserialize, Serialize};
use stump_media::PathUtils;
use tempfile::TempDir;

use crate::{
	calibre::{Calibre, CALIBRE_TOOL},
	external::is_executable,
	util, Action, Plan, ProgressSink, Report, Severity, Tool, ToolError, ToolInput,
	ToolResult, Warning,
};

pub const ID: &str = "calibre-polish";
/// The only action kind this tool plans.
pub const ACTION_POLISH: &str = "polish";

const EBOOK_POLISH: &str = "ebook-polish";

/// Default per-file wall clock, matching `calibre-convert`: subsetting every
/// embedded font or losslessly recompressing every image in a large book is
/// genuinely slow, so the ceiling is generous but always finite.
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Default suffix of a derived output. Parenthesised because
/// [`stump_scanner::clean_name`] strips `(...)`, so `Book (polished).epub`
/// still identifies as the same book during a scan.
const DEFAULT_SUFFIX: &str = " (polished)";

/// What an in-place run appends to the original's file name to keep it.
const BACKUP_EXTENSION: &str = "bak";

const FLAG_COUNT: usize = 7;

/// Every flag this adapter can emit, in the fixed order it emits them, so a
/// planned command line is deterministic and testable. Long forms only: the
/// short forms documented next to them are aliases of the same options
/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#options>).
const FLAGS: [&str; FLAG_COUNT] = [
	"--subset-fonts",
	"--smarten-punctuation",
	"--jacket",
	"--remove-jacket",
	"--upgrade-book",
	"--compress-images",
	"--add-soft-hyphens",
];

/// The only input formats `ebook-polish` accepts: "polishing only works on
/// files in the AZW3 or EPUB or KEPUB formats"
/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html>).
const SUPPORTED_FORMATS: [&str; 3] = ["azw3", "epub", "kepub"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct PolishOptions {
	/// `--subset-fonts`: reduce every embedded font to the characters the book
	/// actually uses
	/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#cmdoption-ebook-polish-subset-fonts>).
	pub subset_fonts: bool,
	/// `--smarten-punctuation`: turn plain dashes, ellipses and quotes into
	/// their typographic equivalents
	/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#cmdoption-ebook-polish-smarten-punctuation>).
	pub smarten_punctuation: bool,
	/// `--jacket`: insert a book jacket page carrying the book's metadata,
	/// replacing any previous jacket
	/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#cmdoption-ebook-polish-jacket>).
	pub jacket: bool,
	/// `--remove-jacket`: remove a previously inserted jacket page
	/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#cmdoption-ebook-polish-remove-jacket>).
	pub remove_jacket: bool,
	/// `--upgrade-book`: upgrade the book's internal structures, e.g. EPUB 2
	/// to EPUB 3
	/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#cmdoption-ebook-polish-upgrade-book>).
	pub upgrade_book: bool,
	/// `--compress-images`: losslessly recompress the book's images
	/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#cmdoption-ebook-polish-compress-images>).
	pub compress_images: bool,
	/// `--add-soft-hyphens`: add soft hyphens so readers without hyphenation
	/// justify text better
	/// (<https://manual.calibre-ebook.com/generated/en/ebook-polish.html#cmdoption-ebook-polish-add-soft-hyphens>).
	pub add_soft_hyphens: bool,
	/// Where polished copies are written; defaults to each source's own
	/// directory. Mutually exclusive with `in_place`.
	pub output_dir: Option<PathBuf>,
	/// Appended to the stem of a derived output when it lands in the source's
	/// own directory, which is what keeps an output from being its own source.
	pub suffix: String,
	/// Polish the original itself, keeping the previous bytes in a
	/// `<file name>.bak` beside it. `suffix` does not apply, and `output_dir`
	/// is refused: an in-place run has exactly one destination.
	pub in_place: bool,
	/// Descend into subdirectories when a given path is a directory.
	pub recursive: bool,
	/// Directory holding the calibre binaries; `PATH` is searched when unset.
	pub bin_dir: Option<PathBuf>,
	pub timeout_secs: u64,
	/// Replace an existing target — or an existing backup — instead of
	/// skipping the book.
	pub overwrite: bool,
}

impl Default for PolishOptions {
	fn default() -> Self {
		Self {
			subset_fonts: false,
			smarten_punctuation: false,
			jacket: false,
			remove_jacket: false,
			upgrade_book: false,
			compress_images: false,
			add_soft_hyphens: false,
			output_dir: None,
			suffix: DEFAULT_SUFFIX.to_string(),
			in_place: false,
			recursive: false,
			bin_dir: None,
			timeout_secs: DEFAULT_TIMEOUT_SECS,
			overwrite: false,
		}
	}
}

impl PolishOptions {
	/// The selected options as `ebook-polish` arguments, always in [`FLAGS`]
	/// order regardless of the order they were given in.
	fn flags(&self) -> Vec<String> {
		let selected: [bool; FLAG_COUNT] = [
			self.subset_fonts,
			self.smarten_punctuation,
			self.jacket,
			self.remove_jacket,
			self.upgrade_book,
			self.compress_images,
			self.add_soft_hyphens,
		];
		selected
			.into_iter()
			.zip(FLAGS)
			.filter(|(selected, _)| *selected)
			.map(|(_, flag)| flag.to_string())
			.collect()
	}
}

/// Everything `apply` needs to replay one planned polish: a plan is
/// self-contained by contract, and `apply` never sees the options blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PolishDetail {
	/// The selected `--flag` arguments, in [`FLAGS`] order.
	args: Vec<String>,
	#[serde(default)]
	bin_dir: Option<PathBuf>,
	timeout_secs: u64,
	#[serde(default)]
	overwrite: bool,
	#[serde(default)]
	in_place: bool,
	/// Where the original is kept; `Some` exactly when `in_place` is set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	backup: Option<PathBuf>,
	/// Book size before the polish; recorded by `apply` only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	before_bytes: Option<u64>,
	/// Book size after the polish; recorded by `apply` only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	after_bytes: Option<u64>,
}

/// Polish e-books with `ebook-polish`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CalibrePolish;

impl Tool for CalibrePolish {
	fn id(&self) -> &'static str {
		ID
	}

	fn describe(&self) -> &'static str {
		"Polish EPUB/AZW3/KEPUB books with the operator's calibre (ebook-polish)"
	}

	fn plan(&self, input: &ToolInput) -> Result<Plan, ToolError> {
		let options = input.parse_options::<PolishOptions>()?;
		if input.paths.is_empty() {
			return Err(ToolError::Invalid("no input paths".to_string()));
		}

		// Configuration is judged before calibre is looked for: a run that
		// cannot be valid must say so, not blame a missing installation.
		let args = options.flags();
		if args.is_empty() {
			return Err(ToolError::Invalid(format!(
				"no polishing action selected: set at least one of {}",
				FLAGS.join(", ")
			)));
		}
		if let Some(dir) = &options.output_dir {
			if options.in_place {
				return Err(ToolError::Invalid(
					"in_place and output_dir are mutually exclusive".to_string(),
				));
			}
			if !dir.is_dir() {
				return Err(ToolError::Invalid(format!(
					"output_dir {} is not a directory",
					dir.display()
				)));
			}
		}

		let calibre = Calibre::locate(options.bin_dir.as_deref())?;
		// `locate` only proves `ebook-convert` is there. An installation
		// without the polish binary has to fail loudly here rather than skip
		// every single book at apply time.
		if !is_executable(&calibre.binary(EBOOK_POLISH)) {
			return Err(CALIBRE_TOOL.missing(format!(
				"no executable `{EBOOK_POLISH}` in {}",
				calibre.dir().display()
			)));
		}

		let mut plan = Plan::new(ID);
		plan.warn(
			Warning::new("calibre-version", calibre.describe())
				.with_severity(Severity::Info),
		);

		let detail = PolishDetail {
			args,
			bin_dir: Some(calibre.dir().to_path_buf()),
			timeout_secs: options.timeout_secs.max(1),
			overwrite: options.overwrite,
			in_place: options.in_place,
			backup: None,
			before_bytes: None,
			after_bytes: None,
		};

		let sources = collect_books(&input.paths, options.recursive, &mut plan)?;
		if sources.is_empty() {
			plan.warn(
				Warning::new("no-books", "no AZW3, EPUB or KEPUB files found")
					.with_severity(Severity::Error),
			);
			return Ok(plan);
		}

		for source in sources {
			let Some(extension) = polishable_extension(&source) else {
				plan.warn(
					Warning::new(
						"unsupported-format",
						"ebook-polish only reads AZW3, EPUB and KEPUB",
					)
					.at(&source)
					.with_severity(Severity::Error),
				);
				continue;
			};

			let mut detail = detail.clone();
			let target = if options.in_place {
				let backup = backup_path(&source);
				if backup.exists() && !options.overwrite {
					plan.warn(
						Warning::new(
							"backup-exists",
							format!("{} already exists", backup.display()),
						)
						.at(&source),
					);
					continue;
				}
				detail.backup = Some(backup);
				source.clone()
			} else {
				let target = util::derived_target(
					&source,
					options.output_dir.as_deref(),
					&options.suffix,
					&extension,
				)?;
				if target == source {
					plan.warn(
						Warning::new(
							"target-is-source",
							"polishing would overwrite the original: set \
							 in_place, a suffix, or an output_dir",
						)
						.at(&source)
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
						.at(&source),
					);
					continue;
				}
				target
			};

			plan.push(
				Action::new(ACTION_POLISH)
					.with_source(&source)
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
			let mut detail =
				serde_json::from_value::<PolishDetail>(action.detail.clone())
					.map_err(|error| ToolError::Options(error.to_string()))?;

			sink.progress(index, total, &format!("polishing {}", source.display()));

			if !source.is_file() {
				report.skipped(action.clone(), "source disappeared");
				continue;
			}
			// A plan can be minutes old, so every destination is re-checked
			// and `overwrite` is the only licence to replace a file.
			if !detail.in_place && target.exists() && !detail.overwrite {
				report.skipped(
					action.clone(),
					format!("{} already exists", target.display()),
				);
				continue;
			}
			let backup = match (detail.in_place, detail.backup.clone()) {
				(false, _) => None,
				(true, Some(backup)) => Some(backup),
				(true, None) => {
					report.skipped(action.clone(), "in-place action has no backup path");
					continue;
				},
			};
			let backup_existed = match &backup {
				Some(backup) if backup.exists() => {
					if !detail.overwrite {
						report.skipped(
							action.clone(),
							format!("{} already exists", backup.display()),
						);
						continue;
					}
					true
				},
				_ => false,
			};

			let calibre = match &calibre {
				Some(calibre) => calibre,
				None => {
					calibre = Some(Calibre::locate(detail.bin_dir.as_deref())?);
					calibre.as_ref().expect("just located")
				},
			};

			// `ebook-polish` never writes the file it is reading: an in-place
			// run polishes into a temp directory beside the source, on the
			// same filesystem, and the finished bytes are renamed over the
			// original. Dropping the guard takes a half-written output with
			// it, so a failure cannot leave a broken book behind.
			let staged = if detail.in_place {
				match stage(source) {
					Ok(staged) => Some(staged),
					Err(error) => {
						report.skipped(
							action.clone(),
							format!("could not stage the output: {error}"),
						);
						continue;
					},
				}
			} else {
				None
			};
			let destination = match &staged {
				Some((_, path)) => path.as_path(),
				None => target.as_path(),
			};

			if let Some(backup) = &backup {
				if let Err(error) = std::fs::copy(source, backup) {
					report.skipped(
						action.clone(),
						format!(
							"could not write the backup {}: {error}",
							backup.display()
						),
					);
					continue;
				}
			}

			detail.before_bytes = file_len(source);
			let mut args = vec![source.as_os_str().to_owned(), destination.into()];
			args.extend(detail.args.iter().map(OsString::from));
			let output = CALIBRE_TOOL.run(
				&calibre.binary(EBOOK_POLISH),
				&args,
				Duration::from_secs(detail.timeout_secs),
			)?;

			if output.succeeded() && destination.is_file() {
				if staged.is_some() {
					if let Err(error) = std::fs::rename(destination, target) {
						clean_up(target, backup.as_deref(), backup_existed);
						report.skipped(
							action.clone(),
							format!("could not replace {}: {error}", target.display()),
						);
						continue;
					}
				}
				if !output.stderr.is_empty() {
					report.warn(
						Warning::new("calibre-stderr", output.stderr.clone())
							.at(source)
							.with_severity(Severity::Info),
					);
				}
				detail.after_bytes = file_len(target);
				report
					.applied(action.clone().with_detail(serde_json::to_value(&detail)?));
				continue;
			}

			clean_up(target, backup.as_deref(), backup_existed);
			let reason = if output.succeeded() {
				format!("{EBOOK_POLISH} reported success but wrote no output")
			} else {
				format!(
					"{EBOOK_POLISH} {}: {}",
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

/// Expand the given paths into books: a file is taken as-is, whatever its
/// extension, and a directory contributes the polishable files it holds. A
/// path that is neither is an `Error` warning, not a failure, so one stale
/// entry cannot kill a run over a whole library.
fn collect_books(
	paths: &[PathBuf],
	recursive: bool,
	plan: &mut Plan,
) -> ToolResult<Vec<PathBuf>> {
	let mut books = Vec::new();
	let mut seen = HashSet::new();

	for path in paths {
		if path.is_dir() {
			for file in util::sorted_files(path, recursive)? {
				let polishable =
					polishable_extension(&file).is_some() && !file.is_hidden_file();
				if polishable && seen.insert(file.clone()) {
					books.push(file);
				}
			}
		} else if path.is_file() {
			if seen.insert(path.clone()) {
				books.push(path.clone());
			}
		} else {
			plan.warn(
				Warning::new("not-a-file", "input is not a readable file")
					.at(path)
					.with_severity(Severity::Error),
			);
		}
	}

	Ok(books)
}

/// The extension of `path` when `ebook-polish` can read that format, else
/// `None`. A polished book keeps its format, so this is also the extension of
/// a derived output — the one calibre picks its output plugin from.
fn polishable_extension(path: &Path) -> Option<String> {
	let extension = path.extension()?.to_string_lossy().into_owned();
	SUPPORTED_FORMATS
		.iter()
		.any(|format| extension.eq_ignore_ascii_case(format))
		.then_some(extension)
}

/// Where an in-place run keeps the original: the source's own name plus
/// `.bak`, beside it, so a polish can always be undone by hand.
fn backup_path(source: &Path) -> PathBuf {
	let mut path = source.as_os_str().to_owned();
	path.push(".");
	path.push(BACKUP_EXTENSION);
	PathBuf::from(path)
}

/// A temp directory beside `source` plus the path `ebook-polish` writes to
/// inside it: the source's own file name, so calibre selects the same format
/// it read, and a rename into place never crosses a filesystem.
fn stage(source: &Path) -> ToolResult<(TempDir, PathBuf)> {
	let name = source.file_name().ok_or_else(|| {
		ToolError::Invalid(format!("{} has no file name", source.display()))
	})?;
	let parent = match source.parent() {
		Some(parent) if !parent.as_os_str().is_empty() => parent,
		_ => Path::new("."),
	};
	let dir = tempfile::Builder::new()
		.prefix(".stump-tools-polish-")
		.tempdir_in(parent)?;
	let path = dir.path().join(name);
	Ok((dir, path))
}

/// Remove what a failed action left behind.
///
/// A derived target is a half-written book, and the planner already refused
/// pre-existing targets, so anything there is ours to delete. In place the
/// staged output goes with its temp directory and the original was never
/// touched, which leaves only a backup this run created: keeping it would
/// refuse the retry with `backup-exists`. A backup that already existed was
/// replaced under `overwrite` and equals the intact original, so it stays.
fn clean_up(target: &Path, backup: Option<&Path>, backup_existed: bool) {
	match backup {
		None if target.exists() => {
			let _ = std::fs::remove_file(target);
		},
		Some(backup) if !backup_existed => {
			let _ = std::fs::remove_file(backup);
		},
		_ => {},
	}
}

/// Size of `path`, or `None` when it cannot be stat'd. Only ever recorded in
/// an applied action's detail, so a failed stat must not fail the run.
fn file_len(path: &Path) -> Option<u64> {
	std::fs::metadata(path).ok().map(|metadata| metadata.len())
}

#[cfg(test)]
mod tests {
	use pretty_assertions::assert_eq;
	use serde_json::json;
	use tempfile::TempDir;

	use super::*;
	use crate::NoopProgress;

	/// `Calibre::locate` probes this binary, so every fake needs it even when
	/// the test only exercises `ebook-polish`.
	#[cfg(unix)]
	const EBOOK_CONVERT: &str = "ebook-convert";

	#[cfg(unix)]
	const BANNER: &str = "ebook-polish (calibre 9.14.0)";

	/// The bytes every fixture book starts with.
	#[cfg(unix)]
	const ORIGINAL: &[u8] = b"not really a book";

	/// The crate-wide fake-binary lock: writing an executable and spawning it
	/// races with every other module that does the same
	/// ([`crate::test_support::fake_binary_lock`] documents why), so all of
	/// them share one lock.
	#[cfg(unix)]
	use crate::test_support::fake_binary_lock;

	/// A directory holding fake calibre binaries. Each answers `--version`
	/// with a calibre banner and otherwise appends its argv to `calls.txt`, so
	/// a test can assert the exact command line without a real calibre.
	///
	/// Holding the crate-wide fake-binary lock for the fake's whole lifetime
	/// is what keeps `ETXTBSY` away, so a test needing two fakes must let the
	/// first one drop before building the second.
	#[cfg(unix)]
	struct Fake {
		dir: TempDir,
		_guard: std::sync::MutexGuard<'static, ()>,
	}

	#[cfg(unix)]
	impl Fake {
		/// `body` is shell run after the argv log; `$1` is the input file and
		/// `$2` the output file this adapter always passes.
		fn with_binaries(body: &str, names: &[&str]) -> Self {
			let guard = fake_binary_lock();
			let dir = TempDir::new().expect("temp dir");
			let script = format!(
				"#!/bin/sh\n\
				 if [ \"$1\" = \"--version\" ]; then printf '%s\\n' \
				 '{BANNER}'; exit 0; fi\n\
				 printf '%s\\n' \"$0 $*\" >> \"$(dirname \"$0\")/calls.txt\"\n\
				 {body}\n"
			);
			for name in names {
				write_script(&dir.path().join(name), &script);
			}
			Self { dir, _guard: guard }
		}

		fn new(body: &str) -> Self {
			Self::with_binaries(body, &[EBOOK_POLISH, EBOOK_CONVERT])
		}

		/// A fake that writes plausible polished bytes and exits 0, chatting
		/// on stderr the way a real polish reports its progress.
		fn writing_output() -> Self {
			Self::new(
				"printf 'polished' > \"$2\"\n\
				 echo 'Compressing images...' 1>&2\nexit 0",
			)
		}

		fn path(&self) -> &Path {
			self.dir.path()
		}

		fn calls(&self) -> Vec<String> {
			std::fs::read_to_string(self.dir.path().join("calls.txt"))
				.unwrap_or_default()
				.lines()
				.map(str::to_string)
				.collect()
		}
	}

	#[cfg(unix)]
	fn write_script(path: &Path, body: &str) {
		use std::os::unix::fs::PermissionsExt;

		std::fs::write(path, body).expect("write script");
		std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
			.expect("chmod script");
	}

	#[cfg(unix)]
	fn book(dir: &Path, name: &str) -> PathBuf {
		let path = dir.join(name);
		std::fs::write(&path, ORIGINAL).expect("write book");
		path
	}

	/// Every entry name in `dir`, sorted: what proves nothing was left behind.
	#[cfg(unix)]
	fn names(dir: &Path) -> Vec<String> {
		let mut names = std::fs::read_dir(dir)
			.expect("read dir")
			.map(|entry| {
				entry
					.expect("entry")
					.file_name()
					.to_string_lossy()
					.into_owned()
			})
			.collect::<Vec<_>>();
		names.sort();
		names
	}

	#[cfg(unix)]
	#[test]
	fn selected_flags_are_emitted_in_the_documented_order() {
		let fake = Fake::writing_output();
		let library = TempDir::new().expect("temp dir");
		let source = book(library.path(), "Book.epub");

		// Deliberately given out of order: the command line is not.
		let input = ToolInput::new(vec![source.clone()]).with_options(json!({
			"bin_dir": fake.path(),
			"add_soft_hyphens": true,
			"compress_images": true,
			"smarten_punctuation": true,
			"subset_fonts": true,
		}));
		let plan = CalibrePolish.plan(&input).expect("plan");
		let target = library.path().join("Book (polished).epub");
		assert_eq!(plan.actions.len(), 1, "{:?}", plan.warnings);
		assert_eq!(plan.actions[0].kind, ACTION_POLISH);
		assert_eq!(plan.actions[0].target.as_deref(), Some(target.as_path()));

		let report = CalibrePolish
			.apply(&plan, &mut NoopProgress)
			.expect("apply");
		assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);
		assert_eq!(std::fs::read(&target).expect("read target"), b"polished");
		// Whatever calibre said on stderr is relayed as an Info finding.
		let chatter = report
			.warnings
			.iter()
			.find(|warning| warning.code == "calibre-stderr")
			.expect("calibre-stderr warning");
		assert_eq!(chatter.message, "Compressing images...");
		assert_eq!(chatter.path.as_deref(), Some(source.as_path()));
		assert_eq!(chatter.severity, Severity::Info);

		let calls = fake.calls();
		assert_eq!(calls.len(), 1, "{calls:?}");
		assert_eq!(
			calls[0],
			format!(
				"{} {} {} --subset-fonts --smarten-punctuation \
				 --compress-images --add-soft-hyphens",
				fake.path().join(EBOOK_POLISH).display(),
				source.display(),
				target.display()
			)
		);
		// An option that was not set contributes no argument at all.
		assert!(!calls[0].contains("--upgrade-book"), "{}", calls[0]);
		assert!(!calls[0].contains("jacket"), "{}", calls[0]);
	}

	#[cfg(unix)]
	#[test]
	fn a_calibre_without_ebook_polish_is_reported_as_missing() {
		{
			// calibre is installed and answers `--version`; the binary this
			// tool needs is not there.
			let fake = Fake::with_binaries("exit 0", &[EBOOK_CONVERT]);
			let input =
				ToolInput::new(vec![PathBuf::from("Book.epub")]).with_options(json!({
					"bin_dir": fake.path(),
					"subset_fonts": true,
				}));
			let error = CalibrePolish.plan(&input).expect_err("no ebook-polish");
			let ToolError::ExternalToolMissing { tool, reason, .. } = &error else {
				panic!("expected ExternalToolMissing, got {error:?}");
			};
			assert_eq!(tool, "calibre");
			assert!(reason.contains(EBOOK_POLISH), "{reason}");
		}

		// An empty bin_dir fails earlier, on `ebook-convert`.
		let empty = TempDir::new().expect("temp dir");
		let input =
			ToolInput::new(vec![PathBuf::from("Book.epub")]).with_options(json!({
				"bin_dir": empty.path(),
				"subset_fonts": true,
			}));
		let error = CalibrePolish.plan(&input).expect_err("no calibre");
		assert!(
			matches!(error, ToolError::ExternalToolMissing { .. }),
			"{error:?}"
		);
	}

	#[cfg(unix)]
	#[test]
	fn plan_writes_nothing() {
		let fake = Fake::writing_output();
		let library = TempDir::new().expect("temp dir");
		let source = book(library.path(), "Book.epub");

		let input = ToolInput::new(vec![source.clone()]).with_options(json!({
			"bin_dir": fake.path(),
			"upgrade_book": true,
			"in_place": true,
		}));
		let plan = CalibrePolish.plan(&input).expect("plan");

		assert_eq!(plan.actions.len(), 1, "{:?}", plan.warnings);
		assert_eq!(names(library.path()), vec!["Book.epub".to_string()]);
		assert_eq!(std::fs::read(&source).expect("read book"), ORIGINAL);
		// Only the version probe ran, and that one is not logged.
		assert!(fake.calls().is_empty(), "{:?}", fake.calls());
	}

	#[cfg(unix)]
	#[test]
	fn a_format_ebook_polish_cannot_read_is_reported_not_planned() {
		let fake = Fake::writing_output();
		let library = TempDir::new().expect("temp dir");
		let mobi = book(library.path(), "Prose.mobi");
		let epub = book(library.path(), "Comic.epub");

		let input = ToolInput::new(vec![mobi.clone(), epub.clone()])
			.with_options(json!({ "bin_dir": fake.path(), "jacket": true }));
		let plan = CalibrePolish.plan(&input).expect("plan");

		assert_eq!(
			plan.actions
				.iter()
				.filter_map(|action| action.source.as_deref())
				.collect::<Vec<_>>(),
			vec![epub.as_path()],
			"only the EPUB is polishable"
		);
		let unsupported = plan
			.warnings
			.iter()
			.find(|warning| warning.code == "unsupported-format")
			.expect("unsupported-format warning");
		assert_eq!(unsupported.path.as_deref(), Some(mobi.as_path()));
		assert_eq!(unsupported.severity, Severity::Error);
	}

	#[cfg(unix)]
	#[test]
	fn in_place_backs_up_the_original_and_replaces_it_with_the_polish() {
		let fake = Fake::writing_output();
		let library = TempDir::new().expect("temp dir");
		let source = book(library.path(), "Book.epub");

		let input = ToolInput::new(vec![source.clone()]).with_options(json!({
			"bin_dir": fake.path(),
			"in_place": true,
			"subset_fonts": true,
		}));
		let plan = CalibrePolish.plan(&input).expect("plan");
		assert_eq!(plan.actions[0].target.as_deref(), Some(source.as_path()));

		let report = CalibrePolish
			.apply(&plan, &mut NoopProgress)
			.expect("apply");
		assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);
		assert_eq!(std::fs::read(&source).expect("read book"), b"polished");
		assert_eq!(
			std::fs::read(library.path().join("Book.epub.bak")).expect("read bak"),
			ORIGINAL
		);
		// The staging directory is gone; nothing else was left behind.
		assert_eq!(
			names(library.path()),
			vec!["Book.epub".to_string(), "Book.epub.bak".to_string()]
		);

		let detail = &report.applied[0].detail;
		assert_eq!(detail["before_bytes"], json!(ORIGINAL.len()));
		assert_eq!(detail["after_bytes"], json!("polished".len()));
	}

	#[cfg(unix)]
	#[test]
	fn a_failed_in_place_polish_leaves_the_original_byte_identical() {
		let fake = Fake::new(
			"printf 'half a book' > \"$2\"\n\
			 echo 'Failed to parse: unexpected end of file' 1>&2\nexit 1",
		);
		let library = TempDir::new().expect("temp dir");
		let source = book(library.path(), "Book.epub");

		let input = ToolInput::new(vec![source.clone()]).with_options(json!({
			"bin_dir": fake.path(),
			"in_place": true,
			"compress_images": true,
		}));
		let plan = CalibrePolish.plan(&input).expect("plan");
		let report = CalibrePolish
			.apply(&plan, &mut NoopProgress)
			.expect("apply");

		assert!(report.applied.is_empty());
		assert_eq!(report.skipped.len(), 1);
		let (_, reason) = &report.skipped[0];
		assert!(reason.contains("exit code 1"), "{reason}");
		assert!(reason.contains("unexpected end of file"), "{reason}");
		assert_eq!(std::fs::read(&source).expect("read book"), ORIGINAL);
		// No half-written book, and no backup left to refuse the retry.
		assert_eq!(names(library.path()), vec!["Book.epub".to_string()]);
	}

	#[cfg(unix)]
	#[test]
	fn a_failed_polish_removes_the_half_written_target() {
		let fake = Fake::new("printf 'half a book' > \"$2\"\nexit 3");
		let library = TempDir::new().expect("temp dir");
		let source = book(library.path(), "Book.epub");

		let input = ToolInput::new(vec![source.clone()]).with_options(json!({
			"bin_dir": fake.path(),
			"smarten_punctuation": true,
		}));
		let plan = CalibrePolish.plan(&input).expect("plan");
		let report = CalibrePolish
			.apply(&plan, &mut NoopProgress)
			.expect("apply");

		assert!(report.applied.is_empty());
		let (_, reason) = &report.skipped[0];
		assert!(reason.contains("exit code 3"), "{reason}");
		// A truncated book must not survive as a library file.
		assert_eq!(names(library.path()), vec!["Book.epub".to_string()]);
		assert_eq!(std::fs::read(&source).expect("read book"), ORIGINAL);
	}

	#[test]
	fn a_polish_with_no_action_selected_is_refused() {
		let input = ToolInput::new(vec![PathBuf::from("Book.epub")]);
		let error = CalibrePolish.plan(&input).expect_err("no flags");
		let ToolError::Invalid(reason) = &error else {
			panic!("expected Invalid, got {error:?}");
		};
		for flag in FLAGS {
			assert!(reason.contains(flag), "{flag} is not named: {reason}");
		}
	}

	#[test]
	fn in_place_and_output_dir_are_refused_together() {
		let library = TempDir::new().expect("temp dir");
		let input =
			ToolInput::new(vec![PathBuf::from("Book.epub")]).with_options(json!({
				"subset_fonts": true,
				"in_place": true,
				"output_dir": library.path(),
			}));
		let error = CalibrePolish.plan(&input).expect_err("two destinations");
		let ToolError::Invalid(reason) = &error else {
			panic!("expected Invalid, got {error:?}");
		};
		assert!(reason.contains("in_place"), "{reason}");
	}

	#[test]
	fn an_empty_input_is_refused_before_looking_for_calibre() {
		let error = CalibrePolish
			.plan(&ToolInput::new(Vec::new()))
			.expect_err("no paths");
		assert!(matches!(error, ToolError::Invalid(_)), "{error:?}");
	}
}
