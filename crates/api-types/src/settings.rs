//! The setting schema shared by every pluggable component that exposes
//! user-editable configuration (ingest metadata providers and quality checks,
//! annotation sinks, notification channels). A component declares a static
//! list of [`SettingDefinition`]s; the host stores the user's values as
//! [`SettingValues`] keyed by `SettingDefinition::key` and renders the schema
//! in its editors.

use std::{collections::BTreeMap, fmt};

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
	/// Inclusive lower bound of an `Int`/`Float` setting.
	pub minimum: Option<f64>,
	/// Inclusive upper bound of an `Int`/`Float` setting.
	pub maximum: Option<f64>,
	/// The accepted values of an `Enum` setting; empty for every other kind.
	#[serde(skip_deserializing)]
	pub options: &'static [&'static str],
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

/// A submitted setting value its definition refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingValidationError {
	pub key: String,
	pub message: String,
}

impl fmt::Display for SettingValidationError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "setting `{}`: {}", self.key, self.message)
	}
}

impl std::error::Error for SettingValidationError {}

impl SettingDefinition {
	/// Checks one submitted value against this definition's kind, bounds,
	/// and options. `null` is not a value: callers drop it to fall back to
	/// the default.
	pub fn validate(&self, value: &Value) -> Result<(), SettingValidationError> {
		let refuse = |message: String| SettingValidationError {
			key: self.key.to_owned(),
			message,
		};
		let number = match self.kind {
			SettingKind::Bool => {
				return value
					.is_boolean()
					.then_some(())
					.ok_or_else(|| refuse("expected true or false".to_owned()));
			},
			SettingKind::Int => value
				.as_i64()
				.map(|number| number as f64)
				.ok_or_else(|| refuse("expected a whole number".to_owned()))?,
			SettingKind::Float => value
				.as_f64()
				.filter(|number| number.is_finite())
				.ok_or_else(|| refuse("expected a number".to_owned()))?,
			SettingKind::String => {
				let text = value
					.as_str()
					.ok_or_else(|| refuse("expected text".to_owned()))?;
				if self.required && text.trim().is_empty() {
					return Err(refuse("a value is required".to_owned()));
				}
				return Ok(());
			},
			SettingKind::Enum => {
				let text = value.as_str().ok_or_else(|| {
					refuse("expected one of the listed options".to_owned())
				})?;
				if !self.options.is_empty() && !self.options.contains(&text) {
					return Err(refuse(format!(
						"expected one of {}",
						self.options.join(", ")
					)));
				}
				return Ok(());
			},
			SettingKind::Json => return Ok(()),
		};
		if let Some(minimum) = self.minimum.filter(|minimum| number < *minimum) {
			return Err(refuse(format!("must be at least {minimum}")));
		}
		if let Some(maximum) = self.maximum.filter(|maximum| number > *maximum) {
			return Err(refuse(format!("must be at most {maximum}")));
		}
		Ok(())
	}
}

/// Validates a submitted settings object against `definitions`.
///
/// Unknown keys and values a definition refuses are rejected; `null` entries
/// are dropped so the key falls back to its default. Returns the values to
/// store.
pub fn validate_setting_values(
	definitions: &[SettingDefinition],
	values: &Value,
) -> Result<SettingValues, SettingValidationError> {
	let Value::Object(values) = values else {
		return Err(SettingValidationError {
			key: String::new(),
			message: "settings must be a JSON object".to_owned(),
		});
	};
	let mut accepted = SettingValues::new();
	for (key, value) in values {
		let definition = definitions
			.iter()
			.find(|definition| definition.key == key)
			.ok_or_else(|| SettingValidationError {
				key: key.clone(),
				message: "unknown setting".to_owned(),
			})?;
		if value.is_null() {
			continue;
		}
		definition.validate(value)?;
		accepted.insert(key.clone(), value.clone());
	}
	Ok(accepted)
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn definition(kind: SettingKind) -> SettingDefinition {
		SettingDefinition {
			key: "value",
			label: "Value",
			description: "",
			kind,
			default: Value::Null,
			required: false,
			secret: false,
			help_url: None,
			minimum: None,
			maximum: None,
			options: &[],
		}
	}

	#[test]
	fn int_bounds_and_kind_are_enforced() {
		let bounded = SettingDefinition {
			minimum: Some(1.0),
			maximum: Some(10.0),
			..definition(SettingKind::Int)
		};
		assert!(bounded.validate(&json!(1)).is_ok());
		assert!(bounded.validate(&json!(10)).is_ok());
		assert!(bounded.validate(&json!(0)).is_err());
		assert!(bounded.validate(&json!(11)).is_err());
		assert!(bounded.validate(&json!(2.5)).is_err());
		assert!(bounded.validate(&json!("3")).is_err());
	}

	#[test]
	fn enum_options_bool_and_required_strings_are_enforced() {
		let choice = SettingDefinition {
			options: &["a", "b"],
			..definition(SettingKind::Enum)
		};
		assert!(choice.validate(&json!("a")).is_ok());
		assert!(choice.validate(&json!("c")).is_err());
		assert!(definition(SettingKind::Bool).validate(&json!(1)).is_err());
		let required = SettingDefinition {
			required: true,
			..definition(SettingKind::String)
		};
		assert!(required.validate(&json!("  ")).is_err());
	}

	#[test]
	fn values_reject_unknown_keys_and_drop_nulls() {
		let definitions = [definition(SettingKind::Int)];
		assert!(validate_setting_values(&definitions, &json!({"other": 1})).is_err());
		assert!(validate_setting_values(&definitions, &json!([])).is_err());
		let accepted =
			validate_setting_values(&definitions, &json!({"value": null})).unwrap();
		assert!(accepted.is_empty());
	}
}
