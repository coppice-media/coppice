//! The [`Sink`] contract and per-sink export state.
//!
//! A sink is constructed per user at export time with its configured values
//! (already decrypted by the host); [`Sink::export`] receives the canonical
//! batch and the previously persisted [`SinkState`] and returns the state to
//! persist. Sinks must be idempotent: the same batch and state produce the
//! same effect twice.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::error::AnnotationSyncError;
use crate::model::ExportBatch;

/// Durable per-user, per-sink state. The cursors are owned by the host job
/// (which advances them only after a successful export); [`SinkState::data`]
/// is sink-private and round-tripped opaquely.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SinkState {
	/// The liseur CAS annotation seq this sink has successfully exported up
	/// to. `0` means "from the beginning".
	#[serde(default)]
	pub liseur_seq: i64,
	/// Sink-private state; sinks must treat unknown contents as absent.
	#[serde(default, skip_serializing_if = "Value::is_null")]
	pub data: Value,
}

/// A static description of a sink for catalogs and editors.
#[derive(Debug, Clone, PartialEq)]
pub struct SinkDescriptor {
	pub id: &'static str,
	pub name: &'static str,
	pub description: &'static str,
	pub settings: Vec<SettingDefinition>,
}

/// A destination for canonical annotation exports.
#[async_trait]
pub trait Sink: Send + Sync {
	/// The stable sink id stored in `annotation_sink_configs.sink_id`.
	fn id(&self) -> &'static str;

	/// Human-readable name and description for editors.
	fn descriptor(&self) -> SinkDescriptor;

	/// Exports the batch, given the previously persisted state; returns the
	/// state to persist for the next run.
	async fn export(
		&self,
		batch: &ExportBatch,
		state: &SinkState,
	) -> Result<SinkState, AnnotationSyncError>;
}

/// Constructs a sink instance for one user from its configured values.
///
/// `root` is the configured `annotation_sync_root` (or its default); sinks
/// derive their own per-user directory from the batch's user id.
pub type SinkFactory =
	dyn Fn(&std::path::Path, &SettingValues) -> Result<Box<dyn Sink>, AnnotationSyncError>
		+ Send
		+ Sync;

/// Reads a string setting with a default fallback.
pub fn string_setting(
	values: &SettingValues,
	key: &str,
	default: &str,
) -> String {
	values
		.get(key)
		.and_then(Value::as_str)
		.filter(|value| !value.is_empty())
		.unwrap_or(default)
		.to_owned()
}
