use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::*;
use crate::{test_support::fake_binary_lock, NoopProgress};

// ---------------------------------------------------------------------------
// Version and format parsing (no boko, no shell)
// ---------------------------------------------------------------------------

#[test]
fn version_is_parsed_from_the_boko_banner() {
	// clap prints `<program> <version>`; 0.5.0 is what the real CLI emits.
	assert_eq!(parse_version("boko 0.5.0"), Some((0, 5, 0)));
	assert_eq!(parse_version("boko 1.2.3\n"), Some((1, 2, 3)));
	assert_eq!(parse_version("boko 0.6"), Some((0, 6, 0)));
	// The banner is the anchor: a versioned path printed first must not win.
	assert_eq!(
		parse_version("/home/user/.cargo/bin/boko-0.4/loader\nboko 0.5.0"),
		Some((0, 5, 0))
	);
	assert_eq!(parse_version("boko"), None);
	assert_eq!(parse_version("version 5"), None);
}

#[test]
fn a_pre_release_patch_is_below_the_minimum() {
	// The whole triple is compared: 0.4.9 is not 0.5.0, and a pre-1.0 crate
	// can move the CLI in a minor release.
	assert!((0, 4, 9) < MIN_VERSION);
	assert!((0, 5, 0) >= MIN_VERSION);
	assert!((1, 0, 0) >= MIN_VERSION);
}

#[test]
fn target_formats_map_to_the_extension_boko_infers_from() {
	for (format, extension) in [
		(TargetFormat::Epub, "epub"),
		(TargetFormat::Azw3, "azw3"),
		(TargetFormat::Kfx, "kfx"),
		(TargetFormat::Mobi, "mobi"),
	] {
		assert_eq!(format.extension(), extension);
	}
	assert_eq!(
		serde_json::from_value::<TargetFormat>(serde_json::json!("kfx")).unwrap(),
		TargetFormat::Kfx
	);
	assert!(serde_json::from_value::<TargetFormat>(serde_json::json!("pdf")).is_err());
	// boko reads MOBI but refuses to write it.
	assert!(!TargetFormat::Mobi.writable());
	for format in [TargetFormat::Epub, TargetFormat::Azw3, TargetFormat::Kfx] {
		assert!(format.writable(), "{format}");
	}
}

#[test]
fn source_format_comes_from_the_extension_case_insensitively() {
	for (name, expected) in [
		("Book.epub", Some(SourceFormat::Epub)),
		("Book.AZW3", Some(SourceFormat::Azw3)),
		("Book.mobi", Some(SourceFormat::Mobi)),
		("Book.kfx", Some(SourceFormat::Kfx)),
		// Write-only for boko, and formats it cannot read at all.
		("Book.md", None),
		("Book.txt", None),
		("Book.pdf", None),
		("Book.cbz", None),
		("Book", None),
	] {
		assert_eq!(SourceFormat::from_path(Path::new(name)), expected, "{name}");
	}
}

// ---------------------------------------------------------------------------
// Fake boko
// ---------------------------------------------------------------------------

// Fake binaries are serialized by the crate-wide `fake_binary_lock`: `execve`
// fails with `ETXTBSY` while any process holds the image open for writing, and
// a per-module lock would only serialize this module against itself while
// `calibre`/`calibre_polish` spawn children on other test threads.

/// A directory holding a fake `boko`. Every invocation appends its argv to
/// `calls.txt`, so tests assert the exact command line without a real boko.
///
/// Holding the crate-wide fake-binary lock for the fake's whole lifetime is
/// what keeps `ETXTBSY` away, so a test needing two fakes must let the first
/// one drop before building the second.
#[cfg(unix)]
struct FakeBoko {
	dir: TempDir,
	_guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(unix)]
impl FakeBoko {
	/// `body` is shell run after the argv log; `$@` holds the arguments, so
	/// `$1` is `convert`, `$2` `--quiet`, `$3` the source and `$4` the target.
	fn new(version_banner: &str, body: &str) -> Self {
		let guard = fake_binary_lock();
		let dir = TempDir::new().expect("temp dir");
		let script = format!(
			"#!/bin/sh\n\
			 if [ \"$1\" = \"--version\" ]; then printf '%s\\n' \
			 '{version_banner}'; exit 0; fi\n\
			 printf '%s\\n' \"$0 $*\" >> \"$(dirname \"$0\")/calls.txt\"\n\
			 {body}\n"
		);
		write_script(&dir.path().join(BOKO), &script);
		Self { dir, _guard: guard }
	}

	/// A fake that writes a plausible output file and exits 0.
	fn writing_output() -> Self {
		Self::new("boko 0.5.0", "printf 'converted' > \"$4\"\nexit 0")
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
	std::fs::write(&path, b"not really a book").expect("write book");
	path
}

// ---------------------------------------------------------------------------
// locate()
// ---------------------------------------------------------------------------

#[test]
fn a_missing_binary_is_reported_with_an_install_hint() {
	let empty = TempDir::new().expect("temp dir");
	let error = Boko::locate(Some(empty.path())).expect_err("no boko");
	let ToolError::ExternalToolMissing { tool, reason, hint } = &error else {
		panic!("expected ExternalToolMissing, got {error:?}");
	};
	assert_eq!(tool, "boko");
	assert!(reason.contains("bin_dir"), "{reason}");
	// The hint has to carry the one command that installs it.
	assert!(hint.contains("cargo install boko"), "{hint}");
	let rendered = error.to_string();
	assert!(rendered.starts_with("boko is unavailable:"), "{rendered}");
}

#[cfg(unix)]
#[test]
fn locate_accepts_a_supported_version_and_rejects_an_old_one() {
	{
		let fake = FakeBoko::new("boko 0.5.0", "exit 0");
		let boko = Boko::locate(Some(fake.path())).expect("located");
		assert_eq!(boko.version(), (0, 5, 0));
		assert_eq!(boko.dir(), fake.path());
		assert_eq!(boko.binary(), fake.path().join(BOKO));
	}

	let old = FakeBoko::new("boko 0.4.9", "exit 0");
	let error = Boko::locate(Some(old.path())).expect_err("too old");
	let ToolError::ExternalToolMissing { reason, .. } = &error else {
		panic!("expected ExternalToolMissing, got {error:?}");
	};
	assert!(reason.contains("0.4.9"), "{reason}");
	assert!(reason.contains("0.5.0"), "{reason}");
}

#[cfg(unix)]
#[test]
fn an_unparseable_version_banner_is_rejected() {
	let fake = FakeBoko::new("something else entirely", "exit 0");
	let error = Boko::locate(Some(fake.path())).expect_err("no version");
	assert!(
		matches!(error, ToolError::ExternalToolMissing { .. }),
		"{error:?}"
	);
}

#[cfg(unix)]
#[test]
fn a_non_executable_binary_is_not_accepted() {
	let dir = TempDir::new().expect("temp dir");
	std::fs::write(dir.path().join(BOKO), b"#!/bin/sh\n").expect("write");
	let error = Boko::locate(Some(dir.path())).expect_err("not executable");
	assert!(
		matches!(error, ToolError::ExternalToolMissing { .. }),
		"{error:?}"
	);
}

// ---------------------------------------------------------------------------
// boko-convert: plan
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn plan_lists_source_to_target_with_the_format_pair() {
	let fake = FakeBoko::writing_output();
	let library = TempDir::new().expect("temp dir");
	let azw3 = book(library.path(), "Book.azw3");
	book(library.path(), "Done.epub");
	book(library.path(), "Scan.cbz");
	let directory = library.path().join("sub");
	std::fs::create_dir(&directory).expect("mkdir");

	let input = ToolInput::new(vec![
		azw3.clone(),
		library.path().join("Done.epub"),
		library.path().join("Scan.cbz"),
		directory,
		library.path().join("ghost.kfx"),
	])
	.with_options(serde_json::json!({
		"to": "epub",
		"bin_dir": fake.path(),
	}));
	let plan = BokoConvert.plan(&input).expect("plan");

	assert_eq!(plan.tool, "boko-convert");
	assert_eq!(plan.actions.len(), 1, "{:?}", plan.actions);
	let action = &plan.actions[0];
	assert_eq!(action.kind, "convert");
	assert_eq!(action.source.as_deref(), Some(azw3.as_path()));
	assert_eq!(
		action.target.as_deref(),
		Some(library.path().join("Book.epub").as_path())
	);
	// The format pair travels with the action, so the plan is printable and
	// `apply` replays exactly what was shown.
	assert_eq!(action.detail["from"], serde_json::json!("azw3"));
	assert_eq!(action.detail["to"], serde_json::json!("epub"));

	// A plan is a dry run: nothing was written and nothing but the version
	// probe was invoked.
	assert!(!library.path().join("Book.epub").exists());
	assert!(fake.calls().is_empty());

	let codes = plan
		.warnings
		.iter()
		.map(|warning| warning.code.as_str())
		.collect::<Vec<_>>();
	assert_eq!(
		codes,
		vec![
			"boko-version",
			"already-target-format",
			"unsupported-source",
			"not-a-file",
			"not-a-file"
		]
	);
	assert!(plan.warnings[0].message.contains("boko 0.5.0"));
	// The unreadable format is pointed at the fallback converter.
	let unsupported = &plan.warnings[2];
	assert_eq!(unsupported.severity, Severity::Error);
	assert!(
		unsupported.message.contains("calibre-convert"),
		"{}",
		unsupported.message
	);
}

#[cfg(unix)]
#[test]
fn plan_target_extension_follows_the_requested_format() {
	let fake = FakeBoko::writing_output();
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Book.epub");

	for (to, extension) in [("azw3", "azw3"), ("kfx", "kfx")] {
		let input = ToolInput::new(vec![source.clone()])
			.with_options(serde_json::json!({ "to": to, "bin_dir": fake.path() }));
		let plan = BokoConvert.plan(&input).expect("plan");
		assert_eq!(
			plan.actions[0].target.as_deref(),
			Some(library.path().join(format!("Book.{extension}")).as_path()),
			"to={to}"
		);
	}

	// EPUB out of an AZW3 source, i.e. the other direction.
	let azw3 = book(library.path(), "Kindle.azw3");
	let plan = BokoConvert
		.plan(
			&ToolInput::new(vec![azw3])
				.with_options(serde_json::json!({ "bin_dir": fake.path() })),
		)
		.expect("plan");
	assert_eq!(
		plan.actions[0].target.as_deref(),
		Some(library.path().join("Kindle.epub").as_path())
	);
}

#[test]
fn mobi_output_is_refused_with_a_pointer_to_calibre() {
	// No boko is needed: the request is refused before one is looked for.
	let input = ToolInput::new(vec![PathBuf::from("/books/Book.epub")])
		.with_options(serde_json::json!({ "to": "mobi" }));
	let error = BokoConvert.plan(&input).expect_err("mobi is not writable");
	let ToolError::Invalid(reason) = &error else {
		panic!("expected Invalid, got {error:?}");
	};
	assert!(reason.contains("cannot write"), "{reason}");
	assert!(reason.contains("calibre-convert"), "{reason}");
	assert!(reason.contains("azw3"), "{reason}");
}

#[cfg(unix)]
#[test]
fn plan_refuses_an_existing_target_unless_overwrite_is_set() {
	let fake = FakeBoko::writing_output();
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Book.azw3");
	book(library.path(), "Book.epub");

	let options = serde_json::json!({ "to": "epub", "bin_dir": fake.path() });
	let plan = BokoConvert
		.plan(&ToolInput::new(vec![source.clone()]).with_options(options))
		.expect("plan");
	assert!(plan.is_empty());
	assert!(plan
		.warnings
		.iter()
		.any(|warning| warning.code == "target-exists"));

	let options = serde_json::json!({
		"to": "epub",
		"bin_dir": fake.path(),
		"overwrite": true,
	});
	let plan = BokoConvert
		.plan(&ToolInput::new(vec![source]).with_options(options))
		.expect("plan");
	assert_eq!(plan.actions.len(), 1);
}

// ---------------------------------------------------------------------------
// boko-convert: apply
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn apply_invokes_boko_convert_with_source_target_and_extra_args() {
	let fake = FakeBoko::writing_output();
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Book.epub");
	let output_dir = TempDir::new().expect("temp dir");

	let input = ToolInput::new(vec![source.clone()]).with_options(serde_json::json!({
		"to": "kfx",
		"bin_dir": fake.path(),
		"output_dir": output_dir.path(),
		"extra_args": ["--optimize"],
	}));
	let plan = BokoConvert.plan(&input).expect("plan");
	let target = output_dir.path().join("Book.kfx");
	assert_eq!(plan.actions[0].target.as_deref(), Some(target.as_path()));

	let report = BokoConvert.apply(&plan, &mut NoopProgress).expect("apply");
	assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);
	assert!(report.skipped.is_empty());
	assert!(target.is_file());
	// A silent, successful run reports no captured output at all.
	assert!(
		!report
			.warnings
			.iter()
			.any(|warning| warning.code == "boko-stderr"),
		"{:?}",
		report.warnings
	);

	// `--quiet` is not optional: boko writes its progress lines to stderr, so
	// without it every success would carry captured "error output".
	let calls = fake.calls();
	assert_eq!(calls.len(), 1, "{calls:?}");
	assert_eq!(
		calls[0],
		format!(
			"{} convert --quiet {} {} --optimize",
			fake.path().join(BOKO).display(),
			source.display(),
			target.display()
		)
	);
}

#[cfg(unix)]
#[test]
fn apply_captures_stderr_and_removes_a_broken_output() {
	let fake = FakeBoko::new(
		"boko 0.5.0",
		"printf 'half a book' > \"$4\"\n\
		 echo 'error: malformed Epub input: Could not find EOCD' 1>&2\nexit 1",
	);
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Broken.epub");

	let input = ToolInput::new(vec![source])
		.with_options(serde_json::json!({ "to": "azw3", "bin_dir": fake.path() }));
	let plan = BokoConvert.plan(&input).expect("plan");
	let report = BokoConvert.apply(&plan, &mut NoopProgress).expect("apply");

	assert!(report.applied.is_empty());
	assert_eq!(report.skipped.len(), 1);
	let (_, reason) = &report.skipped[0];
	assert!(reason.contains("exit code 1"), "{reason}");
	assert!(reason.contains("Could not find EOCD"), "{reason}");
	// The truncated output must not survive as a library file.
	assert!(!library.path().join("Broken.azw3").exists());
}

#[cfg(unix)]
#[test]
fn apply_times_out_and_reports_it() {
	let fake = FakeBoko::new("boko 0.5.0", "sleep 30");
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Slow.epub");

	let input = ToolInput::new(vec![source]).with_options(serde_json::json!({
		"to": "kfx",
		"bin_dir": fake.path(),
		"timeout_secs": 1,
	}));
	let plan = BokoConvert.plan(&input).expect("plan");
	let started = std::time::Instant::now();
	let report = BokoConvert.apply(&plan, &mut NoopProgress).expect("apply");

	assert!(report.applied.is_empty());
	let (_, reason) = &report.skipped[0];
	assert!(reason.contains("timed out"), "{reason}");
	assert!(
		started.elapsed() < Duration::from_secs(20),
		"the child was not killed"
	);
}

#[cfg(unix)]
#[test]
fn apply_rejects_a_plan_from_another_tool() {
	let error = BokoConvert
		.apply(&Plan::new("calibre-convert"), &mut NoopProgress)
		.expect_err("plan mismatch");
	assert!(
		matches!(&error, ToolError::PlanMismatch { tool, .. } if tool == "boko-convert"),
		"{error:?}"
	);
}

#[test]
fn an_empty_input_is_rejected_before_looking_for_boko() {
	let error = BokoConvert
		.plan(&ToolInput::new(Vec::new()))
		.expect_err("no paths");
	assert!(matches!(error, ToolError::Invalid(_)), "{error:?}");
}

// ---------------------------------------------------------------------------
// KindleExport
// ---------------------------------------------------------------------------

#[test]
fn kindle_export_is_epub_to_azw3() {
	assert!(KindleExport::matches(
		SourceFormat::Epub,
		TargetFormat::Azw3
	));
	// Only that pair: KFX is a different product, and AZW3 -> AZW3 never runs.
	assert!(!KindleExport::matches(
		SourceFormat::Epub,
		TargetFormat::Kfx
	));
	assert!(!KindleExport::matches(
		SourceFormat::Kfx,
		TargetFormat::Azw3
	));
	assert!(!KindleExport::matches(
		SourceFormat::Azw3,
		TargetFormat::Azw3
	));

	let options = KindleExport::options(Some(PathBuf::from("/out")));
	assert_eq!(options.to, TargetFormat::Azw3);
	assert_eq!(options.output_dir.as_deref(), Some(Path::new("/out")));
	assert_eq!(options.timeout_secs, DEFAULT_TIMEOUT_SECS);

	// The convenience input is the same options blob the CLI would pass.
	let input = KindleExport::input(vec![PathBuf::from("/books/Book.epub")], None)
		.expect("input");
	assert_eq!(input.options["to"], serde_json::json!("azw3"));
	assert_eq!(
		input.parse_options::<ConvertOptions>().expect("round trip"),
		KindleExport::options(None)
	);
}

#[cfg(unix)]
#[test]
fn a_planned_kindle_export_is_flagged_and_converts() {
	let fake = FakeBoko::writing_output();
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Book.epub");

	let mut input = KindleExport::input(vec![source.clone()], None).expect("input");
	// The located binary is the one option a caller cannot know up front.
	input.options["bin_dir"] = serde_json::json!(fake.path());
	let plan = BokoConvert.plan(&input).expect("plan");

	let flagged = plan
		.warnings
		.iter()
		.find(|warning| warning.code == KindleExport::WARNING_CODE)
		.expect("kindle-export warning");
	assert_eq!(flagged.severity, Severity::Info);
	assert_eq!(flagged.path.as_deref(), Some(source.as_path()));
	assert!(flagged.message.contains("Book.azw3"), "{}", flagged.message);

	let report = BokoConvert.apply(&plan, &mut NoopProgress).expect("apply");
	assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);
	assert!(library.path().join("Book.azw3").is_file());

	// A second EPUB source with `to: "kfx"` is not the send-to-Kindle path.
	let plan = BokoConvert
		.plan(
			&ToolInput::new(vec![source]).with_options(serde_json::json!({
				"to": "kfx",
				"bin_dir": fake.path(),
			})),
		)
		.expect("plan");
	assert!(!plan
		.warnings
		.iter()
		.any(|warning| warning.code == KindleExport::WARNING_CODE));
}
