//! The crate's one adapter over an operator-installed command-line tool.
//!
//! Every external binary this crate drives — calibre's `ebook-convert`,
//! `ebook-meta` and `ebook-polish`, and `boko` — is GPL-licensed, so nothing
//! here links, vendors or copies any of it: a tool is *located* in the
//! operator's own installation, version-checked, and run as a child process.
//! An adapter describes its tool once as an [`ExternalTool`] and gets the four
//! hard parts from it:
//!
//! * **locating** it — `bin_dir` first, then `PATH`, then the tool's own
//!   documented install location ([`ExternalTool::extra_dirs`]);
//! * **version-checking** it — one `--version` banner rule, one triple
//!   scanner, one [`MinVersion`] comparison;
//! * **running** it — temp files instead of pipes, a finite timeout, output
//!   tails;
//! * **failing** — every "unavailable" path is one
//!   [`ToolError::ExternalToolMissing`] carrying the tool's own install hint.
//!
//! Duplicating any of those per tool would mean several implementations of the
//! hard part in order to change a name in one error string.

use std::{
	ffi::OsString,
	fmt,
	fs::File,
	io::{Read, Seek, SeekFrom},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	time::{Duration, Instant},
};

use crate::{ToolError, ToolResult};

/// How often a running child is polled for exit.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Trailing bytes kept from a child's stdout and stderr. It is a *tail*: a
/// conversion log can be arbitrarily long, and the useful part of a failure is
/// always its last lines.
const OUTPUT_TAIL: usize = 4096;

/// Wall clock a version probe gets. Printing a banner and exiting is not slow
/// work; a probe that needs longer is a broken installation, not a busy one.
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);

/// The oldest acceptable version of a tool, compared at the precision it is
/// written in.
///
/// calibre's CLI is stable across patch releases, so its floor is a
/// `major.minor`; a pre-1.0 crate like boko can move a subcommand in a minor
/// release, so its floor is the whole triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinVersion {
	/// Compare `major.minor` only; any patch level is accepted.
	MajorMinor(u32, u32),
	/// Compare the whole `major.minor.patch` triple.
	Triple(u32, u32, u32),
}

impl MinVersion {
	fn accepts(self, version: (u32, u32, u32)) -> bool {
		match self {
			Self::MajorMinor(major, minor) => (version.0, version.1) >= (major, minor),
			Self::Triple(major, minor, patch) => version >= (major, minor, patch),
		}
	}
}

impl fmt::Display for MinVersion {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::MajorMinor(major, minor) => write!(f, "{major}.{minor}"),
			Self::Triple(major, minor, patch) => write!(f, "{major}.{minor}.{patch}"),
		}
	}
}

/// One external command-line tool: how it is found, how its version banner
/// reads, and how it says it is unavailable.
///
/// Declared once per adapter as a `static`, so locating and running a tool
/// costs no setup.
#[derive(Debug)]
pub struct ExternalTool {
	/// What the operator knows the installation by, and the `tool` field of
	/// [`ToolError::ExternalToolMissing`]. Not necessarily a binary name:
	/// three binaries belong to one `calibre`.
	pub name: &'static str,
	/// The binary whose presence proves the installation and whose version
	/// banner is read. Sibling binaries are addressed with
	/// [`ExternalTool::binary`].
	pub probe: &'static str,
	/// Argument that makes the probe print its version.
	pub version_arg: &'static str,
	/// Literal the version banner is searched from, **lowercase** — the banner
	/// is matched case-insensitively (`"calibre "`, `"boko "`). A wrapper
	/// script that prints a versioned path first would otherwise win; see
	/// [`ExternalTool::parse_version`].
	pub anchor: &'static str,
	pub min: MinVersion,
	/// Directories searched after `PATH`: the tool's own documented install
	/// location (the macOS application bundle, `cargo install`'s bin dir).
	pub extra_dirs: fn() -> Vec<PathBuf>,
	/// How the searched places are named when nothing was found — `"PATH"`,
	/// `"PATH or in the cargo bin directory"`.
	pub searched: &'static str,
	/// How to install the tool, repeated in every `ExternalToolMissing`.
	pub hint: &'static str,
}

impl ExternalTool {
	/// Find the tool and verify it is at least [`ExternalTool::min`].
	///
	/// `bin_dir` wins when given; otherwise `PATH` is searched, then
	/// [`ExternalTool::extra_dirs`]. Every failure — no binary, a probe that
	/// cannot run, an unreadable banner, a version below the floor — is one
	/// [`ToolError::ExternalToolMissing`] naming the cause and the fix.
	pub fn locate(&self, bin_dir: Option<&Path>) -> ToolResult<Install> {
		let dir = self.resolve_dir(bin_dir)?;
		let output = self.run(
			&self.binary(&dir, self.probe),
			&[OsString::from(self.version_arg)],
			VERSION_TIMEOUT,
		)?;
		if !output.succeeded() {
			return Err(self.missing(format!(
				"`{} {}` failed ({}): {}",
				self.probe,
				self.version_arg,
				output.describe_status(),
				first_line(&output.stderr)
			)));
		}
		// A banner can arrive on either stream; calibre prints to stdout,
		// nothing guarantees the next tool does.
		let combined = format!("{}\n{}", output.stdout, output.stderr);
		let Some(version) = self.parse_version(&combined) else {
			return Err(self.missing(format!(
				"could not parse a version from `{} {}` output {:?}",
				self.probe,
				self.version_arg,
				first_line(&combined)
			)));
		};
		if !self.min.accepts(version) {
			return Err(self.missing(format!(
				"{} {}.{}.{} is older than the required {}",
				self.name, version.0, version.1, version.2, self.min
			)));
		}
		Ok(Install { dir, version })
	}

	/// The path of `name` inside `dir`, with the platform's executable
	/// suffix.
	pub fn binary(&self, dir: &Path, name: &str) -> PathBuf {
		if cfg!(windows) {
			dir.join(format!("{name}.exe"))
		} else {
			dir.join(name)
		}
	}

	/// This tool is unavailable, because `reason`.
	pub fn missing(&self, reason: String) -> ToolError {
		ToolError::ExternalToolMissing {
			tool: self.name.to_string(),
			reason,
			hint: self.hint.to_string(),
		}
	}

	/// Version out of a `--version` banner.
	///
	/// The search is anchored just after [`ExternalTool::anchor`] when the
	/// banner contains it — calibre renders `%prog (calibre <version>)`, clap
	/// renders `boko <version>` — so a versioned path printed first cannot
	/// win. Without the anchor it falls back to the first
	/// `major.minor[.patch]` triple anywhere.
	pub fn parse_version(&self, text: &str) -> Option<(u32, u32, u32)> {
		// `to_ascii_lowercase` is byte-for-byte length preserving, so the
		// offset found in the lowercased copy indexes `text` unchanged; the
		// anchor is already lowercase by contract.
		debug_assert_eq!(self.anchor, self.anchor.to_ascii_lowercase());
		let anchor = text
			.to_ascii_lowercase()
			.find(self.anchor)
			.map(|at| at + self.anchor.len())
			.unwrap_or(0);
		first_triple(&text[anchor..]).or_else(|| first_triple(text))
	}

	/// Run `program` with `args`, killing it after `timeout`.
	///
	/// `program` is this tool's own binary — [`ExternalTool::binary`] of the
	/// probe or of a sibling — so a binary that cannot be executed at all is
	/// reported as *this* tool being unavailable.
	///
	/// stdout and stderr are redirected to temporary files rather than pipes:
	/// a polling parent that reads pipes only after exit would deadlock as
	/// soon as a chatty conversion filled the pipe buffer. stdin is
	/// `/dev/null`, so a child that asks a question is killed by the timeout
	/// instead of blocking on the server's own stdin forever.
	pub fn run(
		&self,
		program: &Path,
		args: &[OsString],
		timeout: Duration,
	) -> ToolResult<Output> {
		let mut stdout = tempfile::tempfile()?;
		let mut stderr = tempfile::tempfile()?;
		let started = Instant::now();
		let mut child = Command::new(program)
			.args(args)
			.stdin(Stdio::null())
			.stdout(Stdio::from(stdout.try_clone()?))
			.stderr(Stdio::from(stderr.try_clone()?))
			.spawn()
			.map_err(|error| {
				if error.kind() == std::io::ErrorKind::NotFound {
					self.missing(format!("{} could not be executed", program.display()))
				} else {
					ToolError::Io(error)
				}
			})?;

		let deadline = started + timeout;
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
			duration: started.elapsed(),
			stdout: read_tail(&mut stdout)?,
			stderr: read_tail(&mut stderr)?,
		})
	}

	fn resolve_dir(&self, bin_dir: Option<&Path>) -> ToolResult<PathBuf> {
		if let Some(dir) = bin_dir {
			if is_executable(&self.binary(dir, self.probe)) {
				return Ok(dir.to_path_buf());
			}
			return Err(self.missing(format!(
				"no executable `{}` in the configured bin_dir {}",
				self.probe,
				dir.display()
			)));
		}

		let path = std::env::var_os("PATH").unwrap_or_default();
		let candidates = std::env::split_paths(&path)
			.chain((self.extra_dirs)())
			.filter(|dir| !dir.as_os_str().is_empty());
		for dir in candidates {
			if is_executable(&self.binary(&dir, self.probe)) {
				return Ok(dir);
			}
		}
		Err(self.missing(format!(
			"`{}` was not found on {}",
			self.probe, self.searched
		)))
	}
}

/// A located, version-checked installation of an [`ExternalTool`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
	/// Directory holding the tool's binaries.
	pub dir: PathBuf,
	pub version: (u32, u32, u32),
}

/// What one invocation of an external tool produced.
#[derive(Debug, Clone)]
pub struct Output {
	/// Exit code, or `None` when the child was killed by a signal or by the
	/// timeout.
	pub status: Option<i32>,
	pub timed_out: bool,
	/// Wall clock the child took, measured around the spawn.
	pub duration: Duration,
	/// Last [`OUTPUT_TAIL`] bytes of stdout, trimmed.
	pub stdout: String,
	/// Last [`OUTPUT_TAIL`] bytes of stderr, trimmed.
	pub stderr: String,
}

impl Output {
	pub fn succeeded(&self) -> bool {
		!self.timed_out && self.status == Some(0)
	}

	/// How the child ended, in the one wording every adapter reports a
	/// failure with.
	pub fn describe_status(&self) -> String {
		match (self.timed_out, self.status) {
			(true, _) => "timed out".to_string(),
			(_, Some(code)) => format!("exit code {code}"),
			(_, None) => "killed by a signal".to_string(),
		}
	}
}

/// Whether `path` is a file this process could execute.
///
/// A directory holding a *name* is not an installation: an adapter that needs
/// a second binary from a located installation (`ebook-polish` beside
/// `ebook-convert`) checks it with this rather than discovering it one failed
/// child per book.
#[cfg(unix)]
pub fn is_executable(path: &Path) -> bool {
	use std::os::unix::fs::PermissionsExt;

	std::fs::metadata(path).is_ok_and(|metadata| {
		metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
	})
}

#[cfg(not(unix))]
pub fn is_executable(path: &Path) -> bool {
	std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// The first `major.minor[.patch]` triple in `text`.
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
