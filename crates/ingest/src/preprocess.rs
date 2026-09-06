//! Optional ingest preprocess hook: one operator-supplied executable run over
//! every dropped file before analysis.
//!
//! The hook is a plain child process, `<command> <absolute staged file path>`,
//! with the item and library ids in its environment. Any executable works;
//! typical uses are format normalisation and running your own conversion
//! pipeline. Exit `0` means "keep going with whatever bytes are now at that
//! path" — the caller re-hashes the file, so a hook may rewrite it in place or
//! replace it at the same path. A non-zero exit, a timeout, or a failure to
//! spawn fails the item and the stderr tail becomes its failure reason, which
//! is what the editor shows in its failure list.
//!
//! The command is resolved at startup rather than at first use: a typo in
//! `ingest_preprocess_command` must stop the server, not silently skip every
//! item for the rest of its uptime.

use std::{
	path::{Path, PathBuf},
	process::Stdio,
	time::Duration,
};

use tokio::{process::Command, time::timeout};

use crate::{
	config::IngestSettings,
	error::{IngestError, IngestResult},
};

/// The item being preprocessed, exported to the hook process.
pub const ITEM_ID_ENV: &str = "STUMP_INGEST_ITEM_ID";
/// The library the item was dropped into, exported to the hook process.
pub const LIBRARY_ID_ENV: &str = "STUMP_INGEST_LIBRARY_ID";

/// How much of the hook's stderr is kept as the item's failure reason. The
/// reason is read by a human in the editor and stored on the item row, so a
/// hook that dumps a megabyte of diagnostics contributes only its tail.
const STDERR_TAIL_CHARS: usize = 1_000;

/// What one hook run decided about the item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutcome {
	/// Exit `0`: continue with the file as it now stands on disk.
	Continue,
	/// Non-zero exit, timeout, or a process that could not be run at all. The
	/// item fails with this reason.
	Fail(String),
}

/// A validated preprocess hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreprocessHook {
	/// The resolved executable. Resolution happens once, at startup.
	program: PathBuf,
	timeout: Duration,
}

impl PreprocessHook {
	/// The configured hook, or `None` when the feature is unconfigured.
	///
	/// Errors when a command *is* configured but cannot be used, so the
	/// failure surfaces at startup instead of once per dropped file.
	pub fn from_config(config: &IngestSettings) -> IngestResult<Option<Self>> {
		let Some(raw) = config.preprocess_command.as_deref() else {
			return Ok(None);
		};
		let command = raw.trim();
		if command.is_empty() {
			return Err(IngestError::InitializationError(
				"ingest_preprocess_command is set but empty; unset it to disable the preprocess hook".to_string(),
			));
		}
		let timeout_secs = config.preprocess_timeout_secs;
		if timeout_secs == 0 {
			return Err(IngestError::InitializationError(format!(
				"ingest_preprocess_timeout_secs must be greater than zero when ingest_preprocess_command ({command}) is set"
			)));
		}
		Ok(Some(Self {
			program: resolve_program(command)?,
			timeout: Duration::from_secs(timeout_secs),
		}))
	}

	/// Run the hook against `path`, which must be the absolute staged file.
	///
	/// Never returns an error: everything that can go wrong with the child is
	/// the item's failure, not the server's.
	pub async fn run(&self, item_id: &str, library_id: &str, path: &Path) -> HookOutcome {
		let child = match Command::new(&self.program)
			.arg(path)
			.env(ITEM_ID_ENV, item_id)
			.env(LIBRARY_ID_ENV, library_id)
			.stdin(Stdio::null())
			// The hook's stdout is its own business; only stderr is kept, as
			// the failure reason.
			.stdout(Stdio::null())
			.stderr(Stdio::piped())
			// A hook that outlives its budget is killed when the timed-out
			// future below drops the child.
			.kill_on_drop(true)
			.spawn()
		{
			Ok(child) => child,
			Err(error) => {
				return HookOutcome::Fail(format!(
					"preprocess hook {} could not be started: {error}",
					self.program.display()
				));
			},
		};

		let output = match timeout(self.timeout, child.wait_with_output()).await {
			Ok(Ok(output)) => output,
			Ok(Err(error)) => {
				return HookOutcome::Fail(format!(
					"preprocess hook {} failed while running: {error}",
					self.program.display()
				));
			},
			Err(_) => {
				return HookOutcome::Fail(format!(
					"preprocess hook {} timed out after {} seconds",
					self.program.display(),
					self.timeout.as_secs()
				));
			},
		};

		if output.status.success() {
			HookOutcome::Continue
		} else {
			let code = output
				.status
				.code()
				.map_or_else(|| "signal".to_string(), |code| code.to_string());
			HookOutcome::Fail(format!(
				"preprocess hook {} exited with {code}: {}",
				self.program.display(),
				stderr_tail(&output.stderr)
			))
		}
	}
}

/// Fail startup when the ingest preprocess hook is configured but unusable.
/// Called from `StumpCore::init_config` alongside the other config gates.
pub fn validate_config(config: &IngestSettings) -> IngestResult<()> {
	PreprocessHook::from_config(config).map(|_| ())
}

/// An absolute or relative path is used as given; a bare name is looked up on
/// `PATH`, the same way the child spawn would. Either way the target must
/// exist and be executable now, or the configuration is wrong.
fn resolve_program(command: &str) -> IngestResult<PathBuf> {
	let candidate = Path::new(command);
	if candidate.components().count() > 1 || candidate.is_absolute() {
		return if is_executable_file(candidate) {
			Ok(candidate.to_path_buf())
		} else {
			Err(IngestError::InitializationError(format!(
				"ingest_preprocess_command ({command}) is not an executable file"
			)))
		};
	}

	let path_var = std::env::var_os("PATH").unwrap_or_default();
	std::env::split_paths(&path_var)
		.map(|dir| dir.join(command))
		.find(|candidate| is_executable_file(candidate))
		.ok_or_else(|| {
			IngestError::InitializationError(format!(
				"ingest_preprocess_command ({command}) was not found as an executable on PATH"
			))
		})
}

fn is_executable_file(path: &Path) -> bool {
	let Ok(metadata) = std::fs::metadata(path) else {
		return false;
	};
	if !metadata.is_file() {
		return false;
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		metadata.permissions().mode() & 0o111 != 0
	}
	#[cfg(not(unix))]
	{
		true
	}
}

/// The last [`STDERR_TAIL_CHARS`] characters of the hook's stderr, as a
/// single-line-friendly reason. Lossy on purpose: a hook is free to emit
/// whatever bytes it likes.
fn stderr_tail(stderr: &[u8]) -> String {
	let text = String::from_utf8_lossy(stderr);
	let trimmed = text.trim();
	if trimmed.is_empty() {
		return "no stderr output".to_string();
	}
	let total = trimmed.chars().count();
	if total <= STDERR_TAIL_CHARS {
		return trimmed.to_string();
	}
	let start = trimmed
		.char_indices()
		.nth(total - STDERR_TAIL_CHARS)
		.map_or(0, |(index, _)| index);
	format!("...{}", &trimmed[start..])
}

#[cfg(test)]
mod tests {
	use super::*;

	fn config_with(command: Option<&str>, timeout_secs: u64) -> IngestSettings {
		let mut config = IngestSettings::debug();
		config.preprocess_command = command.map(str::to_owned);
		config.preprocess_timeout_secs = timeout_secs;
		config
	}

	#[test]
	fn unconfigured_hook_is_not_an_error() {
		assert_eq!(
			PreprocessHook::from_config(&config_with(None, 600)).unwrap(),
			None
		);
	}

	/// A command that does not resolve to an executable is a configuration
	/// error at startup, not a per-item surprise.
	#[test]
	fn unresolvable_command_fails_validation() {
		let error = validate_config(&config_with(
			Some("/definitely/not/here/stump-preprocess"),
			600,
		))
		.expect_err("a missing command must fail startup");
		assert!(
			matches!(&error, IngestError::InitializationError(message) if message.contains("not an executable file")),
			"unexpected error: {error}"
		);

		let error = validate_config(&config_with(Some("   "), 600))
			.expect_err("an empty command must fail startup");
		assert!(
			matches!(&error, IngestError::InitializationError(message) if message.contains("set but empty")),
			"unexpected error: {error}"
		);
	}

	/// A directory is not a hook, even though it exists.
	#[test]
	fn directory_is_not_an_executable() {
		let dir = tempfile::tempdir().unwrap();
		let error =
			validate_config(&config_with(Some(&dir.path().to_string_lossy()), 600))
				.expect_err("a directory must fail startup");
		assert!(
			matches!(&error, IngestError::InitializationError(message) if message.contains("not an executable file")),
			"unexpected error: {error}"
		);
	}

	/// The budget is checked before the command is resolved: a hook that can
	/// never finish is a configuration error in its own right.
	#[test]
	fn zero_timeout_is_rejected_when_a_command_is_set() {
		let error = validate_config(&config_with(Some("/bin/true"), 0))
			.expect_err("a zero timeout must fail startup");
		assert!(
			matches!(&error, IngestError::InitializationError(message) if message.contains("greater than zero")),
			"unexpected error: {error}"
		);
	}

	/// Write an executable script and return its tempdir guard plus path.
	#[cfg(unix)]
	fn script(body: &str) -> (tempfile::TempDir, String) {
		use std::os::unix::fs::PermissionsExt;

		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("hook.sh");
		std::fs::write(&path, body).unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
		let display = path.to_string_lossy().into_owned();
		(dir, display)
	}

	/// A hook that outlives its budget is killed and the item is told why,
	/// rather than the analysis job hanging on it forever.
	#[cfg(unix)]
	#[tokio::test]
	async fn hook_that_outlives_its_timeout_fails() {
		let (_dir, command) = script("#!/bin/sh\nsleep 30\n");
		let hook = PreprocessHook::from_config(&config_with(Some(&command), 1))
			.unwrap()
			.unwrap();
		let outcome = hook
			.run("item", "library", Path::new("/tmp/does-not-matter"))
			.await;
		let HookOutcome::Fail(reason) = outcome else {
			panic!("a hook that never exits must fail the item");
		};
		assert!(reason.contains("timed out after 1 seconds"), "{reason}");
	}

	/// The hook receives the file path as its only argument, plus the item and
	/// library ids in the environment.
	#[cfg(unix)]
	#[tokio::test]
	async fn hook_receives_path_and_ids() {
		let (_dir, command) = script(
			"#!/bin/sh\nprintf '%s %s %s' \"$1\" \"$STUMP_INGEST_ITEM_ID\" \"$STUMP_INGEST_LIBRARY_ID\" > \"$1\"\n",
		);
		let target = tempfile::NamedTempFile::new().unwrap();
		let hook = PreprocessHook::from_config(&config_with(Some(&command), 600))
			.unwrap()
			.unwrap();
		assert_eq!(
			hook.run("item-1", "library-1", target.path()).await,
			HookOutcome::Continue
		);
		assert_eq!(
			std::fs::read_to_string(target.path()).unwrap(),
			format!("{} item-1 library-1", target.path().display())
		);
	}

	#[test]
	fn stderr_tail_keeps_the_end_and_reports_silence() {
		assert_eq!(stderr_tail(b"  boom  \n"), "boom");
		assert_eq!(stderr_tail(b"   \n"), "no stderr output");
		let long = "x".repeat(STDERR_TAIL_CHARS + 10);
		let tail = stderr_tail(long.as_bytes());
		assert_eq!(tail.chars().count(), STDERR_TAIL_CHARS + 3);
		assert!(tail.starts_with("..."), "{tail}");
	}
}
