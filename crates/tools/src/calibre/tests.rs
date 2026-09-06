use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::*;
use crate::NoopProgress;

// ---------------------------------------------------------------------------
// Version parsing (no calibre, no shell)
// ---------------------------------------------------------------------------

#[test]
fn version_is_parsed_from_the_calibre_banner() {
	// calibre's OptionParser renders `%prog (calibre <version>)`.
	assert_eq!(
		parse_version("ebook-convert (calibre 9.14.0)"),
		Some((9, 14, 0))
	);
	assert_eq!(
		parse_version("ebook-convert (calibre 7.0)"),
		Some((7, 0, 0))
	);
	assert_eq!(
		parse_version("ebook-convert (calibre 10.2.1)\n"),
		Some((10, 2, 1))
	);
	// The banner is the anchor: a versioned path printed first must not win.
	assert_eq!(
		parse_version("/usr/lib/calibre-bin-2.4/loader\nebook-convert (calibre 9.14.0)"),
		Some((9, 14, 0))
	);
	// A leading standalone integer must not be mistaken for a version.
	assert_eq!(parse_version("calibre"), None);
	assert_eq!(parse_version("version 5"), None);
}

#[test]
fn target_formats_map_to_the_extension_ebook_convert_dispatches_on() {
	for (format, extension) in [
		(TargetFormat::Epub, "epub"),
		(TargetFormat::Kepub, "kepub"),
		(TargetFormat::Azw3, "azw3"),
		(TargetFormat::Mobi, "mobi"),
		(TargetFormat::Pdf, "pdf"),
	] {
		assert_eq!(format.extension(), extension);
	}
	assert_eq!(
		serde_json::from_value::<TargetFormat>(serde_json::json!("kepub")).unwrap(),
		TargetFormat::Kepub
	);
	assert!(serde_json::from_value::<TargetFormat>(serde_json::json!("cbz")).is_err());
}

// ---------------------------------------------------------------------------
// Fake calibre
// ---------------------------------------------------------------------------

/// Serializes every test that writes a fake binary *and* spawns a child.
///
/// `execve` fails with `ETXTBSY` ("Text file busy") when any process holds the
/// image open for writing. `fs::write` closes its descriptor promptly, but a
/// concurrent `Command::spawn` in another test thread forks a child that
/// inherits that descriptor and keeps it open until its own `exec`. Serializing
/// the write-then-exec tests removes the window; nothing else in this crate
/// spawns a process.
static FAKE_CALIBRE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
fn fake_lock() -> std::sync::MutexGuard<'static, ()> {
	// A failing test poisons the mutex; the guarded data is `()`, so recover.
	FAKE_CALIBRE_LOCK
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A directory holding a fake `ebook-convert` / `ebook-meta` pair. Every
/// invocation appends its argv to `calls.txt`, so tests can assert the exact
/// command line without a real calibre.
///
/// Holding [`FAKE_CALIBRE_LOCK`] for the fake's whole lifetime is what keeps
/// `ETXTBSY` away, so a test needing two fakes must let the first one drop
/// before building the second.
#[cfg(unix)]
struct FakeCalibre {
	dir: TempDir,
	_guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(unix)]
impl FakeCalibre {
	/// `body` is shell run after the argv log; `$@` holds the arguments.
	fn new(version_banner: &str, body: &str) -> Self {
		let guard = fake_lock();
		let dir = TempDir::new().expect("temp dir");
		let script = format!(
			"#!/bin/sh\n\
			 if [ \"$1\" = \"--version\" ]; then printf '%s\\n' \
			 '{version_banner}'; exit 0; fi\n\
			 printf '%s\\n' \"$0 $*\" >> \"$(dirname \"$0\")/calls.txt\"\n\
			 {body}\n"
		);
		for name in [EBOOK_CONVERT, EBOOK_META] {
			write_script(&dir.path().join(name), &script);
		}
		Self { dir, _guard: guard }
	}

	/// A fake that writes a plausible output file and exits 0.
	fn writing_output() -> Self {
		Self::new(
			"ebook-convert (calibre 9.14.0)",
			"printf 'converted' > \"$2\"\nexit 0",
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
	std::fs::write(&path, b"not really a book").expect("write book");
	path
}

// ---------------------------------------------------------------------------
// locate()
// ---------------------------------------------------------------------------

#[test]
fn a_missing_binary_is_reported_with_an_install_hint() {
	let empty = TempDir::new().expect("temp dir");
	let error = Calibre::locate(Some(empty.path())).expect_err("no calibre");
	let ToolError::ExternalToolMissing { tool, reason, hint } = &error else {
		panic!("expected ExternalToolMissing, got {error:?}");
	};
	assert_eq!(tool, "calibre");
	assert!(reason.contains("bin_dir"), "{reason}");
	assert!(hint.contains("ebook-convert"), "{hint}");
	// The rendered message must name the tool, the cause and the fix.
	let rendered = error.to_string();
	assert!(
		rendered.starts_with("calibre is unavailable:"),
		"{rendered}"
	);
}

#[cfg(unix)]
#[test]
fn locate_accepts_a_supported_version_and_rejects_an_old_one() {
	{
		let fake = FakeCalibre::new("ebook-convert (calibre 9.14.0)", "exit 0");
		let calibre = Calibre::locate(Some(fake.path())).expect("located");
		assert_eq!(calibre.version(), (9, 14, 0));
		assert_eq!(calibre.dir(), fake.path());
		assert_eq!(
			calibre.binary(EBOOK_CONVERT),
			fake.path().join(EBOOK_CONVERT)
		);
	}

	let old = FakeCalibre::new("ebook-convert (calibre 6.29.0)", "exit 0");
	let error = Calibre::locate(Some(old.path())).expect_err("too old");
	let ToolError::ExternalToolMissing { reason, .. } = &error else {
		panic!("expected ExternalToolMissing, got {error:?}");
	};
	assert!(reason.contains("6.29.0"), "{reason}");
	assert!(reason.contains("7.0"), "{reason}");
}

#[cfg(unix)]
#[test]
fn an_unparseable_version_banner_is_rejected() {
	let fake = FakeCalibre::new("something else entirely", "exit 0");
	let error = Calibre::locate(Some(fake.path())).expect_err("no version");
	assert!(
		matches!(error, ToolError::ExternalToolMissing { .. }),
		"{error:?}"
	);
}

#[cfg(unix)]
#[test]
fn a_non_executable_binary_is_not_accepted() {
	let dir = TempDir::new().expect("temp dir");
	std::fs::write(dir.path().join(EBOOK_CONVERT), b"#!/bin/sh\n").expect("write");
	let error = Calibre::locate(Some(dir.path())).expect_err("not executable");
	assert!(
		matches!(error, ToolError::ExternalToolMissing { .. }),
		"{error:?}"
	);
}

// ---------------------------------------------------------------------------
// calibre-convert
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn plan_lists_source_to_target_per_file() {
	let fake = FakeCalibre::writing_output();
	let library = TempDir::new().expect("temp dir");
	let mobi = book(library.path(), "Book.mobi");
	let already = book(library.path(), "Done.epub");
	let directory = library.path().join("sub");
	std::fs::create_dir(&directory).expect("mkdir");

	let input = ToolInput::new(vec![
		mobi.clone(),
		already.clone(),
		directory.clone(),
		library.path().join("ghost.azw3"),
	])
	.with_options(serde_json::json!({
		"to": "epub",
		"bin_dir": fake.path(),
	}));
	let plan = CalibreConvert.plan(&input).expect("plan");

	assert_eq!(plan.tool, "calibre-convert");
	assert_eq!(plan.actions.len(), 1, "{:?}", plan.actions);
	let action = &plan.actions[0];
	assert_eq!(action.kind, "convert");
	assert_eq!(action.source.as_deref(), Some(mobi.as_path()));
	assert_eq!(
		action.target.as_deref(),
		Some(library.path().join("Book.epub").as_path())
	);

	// A plan is a dry run: nothing was written and no binary was invoked
	// beyond the version probe.
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
			"calibre-version",
			"already-target-format",
			"not-a-file",
			"not-a-file"
		]
	);
	assert!(plan.warnings[0].message.contains("calibre 9.14.0"));
	let _ = already;
}

#[cfg(unix)]
#[test]
fn plan_target_extension_follows_the_requested_format() {
	let fake = FakeCalibre::writing_output();
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Book.epub");

	for (to, extension) in [
		("kepub", "kepub"),
		("azw3", "azw3"),
		("mobi", "mobi"),
		("pdf", "pdf"),
	] {
		let input = ToolInput::new(vec![source.clone()])
			.with_options(serde_json::json!({ "to": to, "bin_dir": fake.path() }));
		let plan = CalibreConvert.plan(&input).expect("plan");
		assert_eq!(
			plan.actions[0].target.as_deref(),
			Some(library.path().join(format!("Book.{extension}")).as_path()),
			"to={to}"
		);
	}
}

#[cfg(unix)]
#[test]
fn plan_refuses_an_existing_target_unless_overwrite_is_set() {
	let fake = FakeCalibre::writing_output();
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Book.mobi");
	book(library.path(), "Book.epub");

	let options = serde_json::json!({ "to": "epub", "bin_dir": fake.path() });
	let plan = CalibreConvert
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
	let plan = CalibreConvert
		.plan(&ToolInput::new(vec![source]).with_options(options))
		.expect("plan");
	assert_eq!(plan.actions.len(), 1);
}

#[cfg(unix)]
#[test]
fn apply_invokes_ebook_convert_with_source_target_and_extra_args() {
	let fake = FakeCalibre::writing_output();
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Book.mobi");
	let output_dir = TempDir::new().expect("temp dir");

	let input = ToolInput::new(vec![source.clone()]).with_options(serde_json::json!({
		"to": "azw3",
		"bin_dir": fake.path(),
		"output_dir": output_dir.path(),
		"extra_args": ["--smarten-punctuation", "--authors", "A & B"],
	}));
	let plan = CalibreConvert.plan(&input).expect("plan");
	let target = output_dir.path().join("Book.azw3");
	assert_eq!(plan.actions[0].target.as_deref(), Some(target.as_path()));

	let report = CalibreConvert
		.apply(&plan, &mut NoopProgress)
		.expect("apply");
	assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);
	assert!(report.skipped.is_empty());
	assert!(target.is_file());

	let calls = fake.calls();
	assert_eq!(calls.len(), 1, "{calls:?}");
	assert_eq!(
		calls[0],
		format!(
			"{} {} {} --smarten-punctuation --authors A & B",
			fake.path().join(EBOOK_CONVERT).display(),
			source.display(),
			target.display()
		)
	);
}

#[cfg(unix)]
#[test]
fn apply_captures_stderr_and_removes_a_broken_output() {
	let fake = FakeCalibre::new(
		"ebook-convert (calibre 9.14.0)",
		"printf 'half a book' > \"$2\"\n\
		 echo 'Failed to parse: unexpected end of file' 1>&2\nexit 1",
	);
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Broken.mobi");

	let input = ToolInput::new(vec![source])
		.with_options(serde_json::json!({ "to": "epub", "bin_dir": fake.path() }));
	let plan = CalibreConvert.plan(&input).expect("plan");
	let report = CalibreConvert
		.apply(&plan, &mut NoopProgress)
		.expect("apply");

	assert!(report.applied.is_empty());
	assert_eq!(report.skipped.len(), 1);
	let (_, reason) = &report.skipped[0];
	assert!(reason.contains("exit code 1"), "{reason}");
	assert!(reason.contains("unexpected end of file"), "{reason}");
	// The truncated output must not survive as a library file.
	assert!(!library.path().join("Broken.epub").exists());
}

#[cfg(unix)]
#[test]
fn apply_times_out_and_reports_it() {
	let fake = FakeCalibre::new("ebook-convert (calibre 9.14.0)", "sleep 30");
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Slow.mobi");

	let input = ToolInput::new(vec![source]).with_options(serde_json::json!({
		"to": "epub",
		"bin_dir": fake.path(),
		"timeout_secs": 1,
	}));
	let plan = CalibreConvert.plan(&input).expect("plan");
	let started = std::time::Instant::now();
	let report = CalibreConvert
		.apply(&plan, &mut NoopProgress)
		.expect("apply");

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
	let error = CalibreConvert
		.apply(&Plan::new("epub2cbz"), &mut NoopProgress)
		.expect_err("plan mismatch");
	assert!(
		matches!(&error, ToolError::PlanMismatch { tool, .. } if tool == "calibre-convert"),
		"{error:?}"
	);
}

// ---------------------------------------------------------------------------
// calibre-meta
// ---------------------------------------------------------------------------

const OPF: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>Mistborn: The Final Empire</dc:title>
    <dc:creator opf:role="aut">Brandon Sanderson</dc:creator>
    <dc:creator opf:role="aut">Someone Else</dc:creator>
    <dc:description>Once, a hero arose &amp; failed.</dc:description>
    <dc:publisher>Tor Books</dc:publisher>
    <dc:language>eng</dc:language>
    <dc:date>2006-07-17T04:00:00+00:00</dc:date>
    <dc:subject>Fantasy</dc:subject>
    <dc:subject>Epic</dc:subject>
    <dc:identifier opf:scheme="ISBN">978-0-7653-1178-8</dc:identifier>
    <meta name="calibre:series" content="Mistborn"/>
    <meta name="calibre:series_index" content="1.0"/>
  </metadata>
</package>
"#;

#[test]
fn opf_is_mapped_onto_the_external_metadata_shape() {
	let metadata =
		parse_opf(OPF.as_bytes(), Path::new("/books/Mistborn.epub")).expect("parse opf");
	assert_eq!(metadata.provider, "calibre");
	assert_eq!(metadata.external_id, "Mistborn.epub");
	assert_eq!(
		metadata.title.as_deref(),
		Some("Mistborn: The Final Empire")
	);
	assert_eq!(
		metadata.writers,
		Some(vec![
			"Brandon Sanderson".to_string(),
			"Someone Else".to_string()
		])
	);
	assert_eq!(
		metadata.summary.as_deref(),
		Some("Once, a hero arose & failed.")
	);
	assert_eq!(metadata.publisher.as_deref(), Some("Tor Books"));
	assert_eq!(metadata.language.as_deref(), Some("eng"));
	assert_eq!(
		(metadata.year, metadata.month, metadata.day),
		(Some(2006), Some(7), Some(17))
	);
	assert_eq!(
		metadata.tags,
		Some(vec!["Fantasy".to_string(), "Epic".to_string()])
	);
	assert_eq!(metadata.isbn_13.as_deref(), Some("9780765311788"));
	assert_eq!(metadata.isbn, None);
	assert_eq!(metadata.series_name.as_deref(), Some("Mistborn"));
	assert_eq!(metadata.number, Some(1.0));

	// Only populated fields are serialized, so a merge never clears a field.
	let json = serde_json::to_value(&metadata).unwrap();
	assert!(json.get("cover_url").is_none());
	assert!(json.get("isbn").is_none());
}

#[test]
fn an_opf_without_metadata_still_parses() {
	let metadata = parse_opf(b"<package><metadata/></package>", Path::new("x.epub"))
		.expect("parse opf");
	assert_eq!(metadata.title, None);
	assert_eq!(metadata.writers, None);
	assert_eq!(metadata.external_id, "x.epub");
}

#[cfg(unix)]
#[test]
fn calibre_meta_reads_metadata_into_the_report() {
	// `ebook-meta <file> --to-opf <path>`: the OPF path is argument 3.
	let fake = FakeCalibre::new(
		"ebook-meta (calibre 9.14.0)",
		&format!("cat > \"$3\" <<'OPF_EOF'\n{OPF}OPF_EOF\nexit 0"),
	);
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Mistborn.epub");

	let input = ToolInput::new(vec![source.clone()])
		.with_options(serde_json::json!({ "bin_dir": fake.path() }));
	let plan = CalibreMeta.plan(&input).expect("plan");
	assert_eq!(plan.actions.len(), 1);
	assert_eq!(plan.actions[0].kind, "read-metadata");
	assert_eq!(plan.actions[0].target, None);

	let report = CalibreMeta.apply(&plan, &mut NoopProgress).expect("apply");
	assert_eq!(report.applied.len(), 1, "{:?}", report.skipped);
	let detail = &report.applied[0].detail;
	assert_eq!(detail["title"], "Mistborn: The Final Empire");
	assert_eq!(detail["series_name"], "Mistborn");
	assert_eq!(detail["provider"], "calibre");

	let calls = fake.calls();
	assert_eq!(calls.len(), 1, "{calls:?}");
	assert!(
		calls[0].contains(&format!("{} --to-opf ", source.display())),
		"{calls:?}"
	);
	// Reading metadata never touches the book.
	assert_eq!(
		std::fs::read(&source).unwrap(),
		b"not really a book".to_vec()
	);
}

#[cfg(unix)]
#[test]
fn calibre_meta_skips_a_file_ebook_meta_cannot_read() {
	let fake = FakeCalibre::new(
		"ebook-meta (calibre 9.14.0)",
		"echo 'Unknown format' 1>&2\nexit 1",
	);
	let library = TempDir::new().expect("temp dir");
	let source = book(library.path(), "Weird.xyz");

	let input = ToolInput::new(vec![source])
		.with_options(serde_json::json!({ "bin_dir": fake.path() }));
	let plan = CalibreMeta.plan(&input).expect("plan");
	let report = CalibreMeta.apply(&plan, &mut NoopProgress).expect("apply");
	assert!(report.applied.is_empty());
	let (_, reason) = &report.skipped[0];
	assert!(reason.contains("Unknown format"), "{reason}");
}

#[test]
fn both_tools_reject_an_empty_input_before_looking_for_calibre() {
	for tool in [
		Box::new(CalibreConvert) as Box<dyn Tool>,
		Box::new(CalibreMeta) as Box<dyn Tool>,
	] {
		let error = tool
			.plan(&ToolInput::new(Vec::new()))
			.expect_err("no paths");
		assert!(matches!(error, ToolError::Invalid(_)), "{error:?}");
	}
}
