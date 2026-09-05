//! The setting schema shared by every pluggable component that exposes
//! user-editable configuration (ingest metadata providers and quality checks,
//! annotation sinks, notification channels). A component declares a static
//! list of [`SettingDefinition`]s; the host stores the user's values as
//! [`SettingValues`] keyed by `SettingDefinition::key` and renders the schema
//! in its editors.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Setting schema entry exposed to the UI for a pluggable component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingDefinition {
	pub key: &'static str,
	pub label: &'static str,
	pub description: &'static str,
	pub kind: SettingKind,
	pub default: Value,
	pub required: bool,
	/// Secret values are stored encrypted and never returned to clients.
	pub secret: bool,
	/// Optional URL where a user can obtain or manage the credential this
	/// setting holds (e.g. the provider's API key page).  Surfaced as
	/// `helpUrl` so the editor can link to it.
	pub help_url: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SettingKind {
	Bool,
	Int,
	Float,
	String,
	Enum,
	Json,
}

/// Effective setting values keyed by `SettingDefinition::key`.
pub type SettingValues = BTreeMap<String, Value>;
