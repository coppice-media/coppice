//! The pipeline's view of host configuration.
//!
//! The crate never sees `StumpConfig`: the host resolves the ingest directory
//! defaults (which hang off its config directory) and hands the pipeline the
//! resolved values. `stump_core` builds this from `StumpConfig` in
//! `core/src/ingest_host.rs`, which is the only place the two spellings meet.

use std::path::PathBuf;

use stump_media::MediaConfig;

/// Resolved ingest configuration.
#[derive(Debug, Clone)]
pub struct IngestSettings {
	/// Root the drop-folder watcher admits files from, one subdirectory per
	/// library id.
	pub drop_dir: PathBuf,
	/// Root of the immutable staged copies.
	pub staging_dir: PathBuf,
	/// Number of typed progress events retained for replay.
	pub progress_retention: u32,
	/// Executable run once per dropped item before analysis, if configured.
	pub preprocess_command: Option<String>,
	/// Wall-clock budget for one preprocess hook run. Zero is a
	/// configuration error when a command is set, not "no timeout".
	pub preprocess_timeout_secs: u64,
	/// Media processing options, needed for the page counts of formats whose
	/// pages are not archive entries (PDF, RAR).
	pub media: MediaConfig,
}

impl IngestSettings {
	/// Settings with `<root>/drop` and `<root>/staging` and every other value
	/// at its default. The host passes explicitly resolved directories
	/// instead; this is for callers that own a scratch root (tests).
	pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
		let root = root.into();
		Self {
			drop_dir: root.join("drop"),
			staging_dir: root.join("staging"),
			progress_retention: 500,
			preprocess_command: None,
			preprocess_timeout_secs: 300,
			media: MediaConfig::default(),
		}
	}

	/// Scratch settings rooted under the OS temp directory. For tests and
	/// one-off tooling that has no host configuration to resolve; the server
	/// always builds these from `StumpConfig`.
	pub fn debug() -> Self {
		Self::rooted_at(std::env::temp_dir().join("stump-ingest"))
	}
}
