use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Annotation export sink configuration. Flattened into
/// [`super::StumpConfig`].
///
/// Sink-specific settings (git remote, tokens, ...) are per-user rows in the
/// `annotation_sink_configs` table, not environment keys; this group only
/// carries the server-side defaults for the feature. The export root is an
/// administrator-controlled mount (`STUMP_ANNOTATION_SYNC_ROOT`); users can
/// select only contained relative destinations within their own directory.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AnnotationSyncConfig {
	/// Administrator-controlled root directory the markdown/git sinks export
	/// into, one subdirectory per user. When unset, this resolves to
	/// `<config_dir>/annotations`, see [`super::StumpConfig::get_annotation_sync_root`].
	#[default_value(None)]
	#[env_key(ANNOTATION_SYNC_ROOT_KEY)]
	pub annotation_sync_root: Option<String>,

	/// How long to wait after the last annotation or reading-head change
	/// before the debounced export runs for that user.
	#[default_value(DEFAULT_ANNOTATION_SYNC_DEBOUNCE_SECS)]
	#[env_key(ANNOTATION_SYNC_DEBOUNCE_SECS_KEY)]
	pub annotation_sync_debounce_secs: u64,
}
