use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Staged ingest directories and progress retention. Flattened into [`super::StumpConfig`].
///
/// The directory defaults resolve relative to the config directory, see
/// [`super::StumpConfig::get_ingest_drop_dir`] and
/// [`super::StumpConfig::get_ingest_staging_dir`]. None of these keys are exposed
/// through GraphQL, so the whole group is skipped there.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IngestConfig {
	/// Optional root for files admitted by the staged ingest drop-folder watcher.
	/// When unset, this resolves to `<config_dir>/ingest/drop`.
	#[default_value(None)]
	#[env_key(INGEST_DROP_DIR_KEY)]
	pub ingest_drop_dir: Option<String>,

	/// Optional root for immutable staged ingest files.
	/// When unset, this resolves to `<config_dir>/ingest/staging`.
	#[default_value(None)]
	#[env_key(INGEST_STAGING_DIR_KEY)]
	pub ingest_staging_dir: Option<String>,

	/// Directory holding the built ingest editor (`editor/build`). When set and
	/// it contains `index.html`, the editor is served under `/editor`.
	#[default_value(None)]
	#[env_key(INGEST_EDITOR_DIR_KEY)]
	pub ingest_editor_dir: Option<String>,

	/// Number of typed ingest progress events retained for replay.
	#[default_value(DEFAULT_INGEST_PROGRESS_RETENTION)]
	#[env_key(INGEST_PROGRESS_RETENTION_KEY)]
	pub ingest_progress_retention: u32,

	/// Optional executable run once per dropped item, before analysis, as
	/// `<command> <absolute staged file path>`. Any executable works; typical
	/// uses are format normalisation and running your own conversion pipeline.
	///
	/// This is an executable path (absolute, or a name resolved on `PATH`), not
	/// a shell line: arguments are not parsed out of it, so wrap multi-step
	/// work in your own script. A command that cannot be resolved to an
	/// executable file fails startup rather than silently skipping every item.
	#[default_value(None)]
	#[env_key(INGEST_PREPROCESS_KEY)]
	pub ingest_preprocess_command: Option<String>,

	/// Wall-clock budget for one preprocess hook run. A hook that outlives it
	/// is killed and its item fails.
	#[default_value(DEFAULT_INGEST_PREPROCESS_TIMEOUT_SECS)]
	#[env_key(INGEST_PREPROCESS_TIMEOUT_KEY)]
	pub ingest_preprocess_timeout_secs: u64,
}
