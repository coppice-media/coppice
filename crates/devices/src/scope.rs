//! Per-device library scope ("virtual pathing"): the same protocol routes
//! resolve to a different visible library set depending on which device
//! authenticated.
//!
//! A scope is stored on `devices.library_scope` as a JSON array of library
//! ids, or `NULL` for [`LibraryScope::Inherit`]. It is only ever intersected
//! with the user's own visibility (see
//! [`VisibilityScope`](models::shared::visibility::VisibilityScope)), so a
//! scope can hide libraries from a device but never reveal one the user
//! cannot see.

use models::entity::device;
use serde_json::Value as JsonValue;

/// What a device may see.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LibraryScope {
	/// The device sees everything its user sees.
	#[default]
	Inherit,
	/// The device sees only these libraries, intersected with what its user
	/// sees. Empty means the device sees nothing.
	Only(Vec<String>),
}

impl LibraryScope {
	/// The scope stored on `device`.
	pub fn of(device: &device::Model) -> Self {
		match device.library_scope_ids() {
			Some(ids) => Self::Only(ids),
			None => Self::Inherit,
		}
	}

	/// The library ids, or `None` when the device inherits.
	pub fn library_ids(&self) -> Option<&[String]> {
		match self {
			Self::Inherit => None,
			Self::Only(ids) => Some(ids),
		}
	}

	/// The stored column value: `None` clears the scope back to inherit.
	pub fn to_column_value(&self) -> Option<JsonValue> {
		match self {
			Self::Inherit => None,
			Self::Only(ids) => Some(JsonValue::Array(
				ids.iter().map(|id| JsonValue::String(id.clone())).collect(),
			)),
		}
	}

	pub fn is_inherit(&self) -> bool {
		matches!(self, Self::Inherit)
	}
}

impl From<Option<Vec<String>>> for LibraryScope {
	/// The GraphQL/UI shape: `null` is inherit, a list is a restriction.
	/// Duplicates are collapsed so the stored scope is a set.
	fn from(library_ids: Option<Vec<String>>) -> Self {
		match library_ids {
			None => Self::Inherit,
			Some(mut ids) => {
				ids.sort_unstable();
				ids.dedup();
				Self::Only(ids)
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn device_with(library_scope: Option<JsonValue>) -> device::Model {
		device::Model {
			id: "device-1".to_string(),
			user_id: "user-1".to_string(),
			name: "Kobo".to_string(),
			kind: models::shared::enums::DeviceKind::Kobo,
			transform_profile: None,
			library_scope,
			kindle_email: None,
			created_at: chrono::Utc::now().into(),
			last_seen_at: None,
			last_sync_at: None,
			last_sync_summary: None,
			revoked_at: None,
		}
	}

	#[test]
	fn null_column_is_inherit() {
		assert_eq!(LibraryScope::of(&device_with(None)), LibraryScope::Inherit);
		assert_eq!(
			LibraryScope::of(&device_with(Some(JsonValue::Null))),
			LibraryScope::Inherit
		);
	}

	#[test]
	fn array_column_restricts() {
		assert_eq!(
			LibraryScope::of(&device_with(Some(json!(["a", "b"])))),
			LibraryScope::Only(vec!["a".to_string(), "b".to_string()])
		);
	}

	#[test]
	fn empty_array_is_a_restriction_to_nothing() {
		let scope = LibraryScope::of(&device_with(Some(json!([]))));
		assert_eq!(scope, LibraryScope::Only(vec![]));
		assert_eq!(scope.library_ids(), Some(&[][..]));
	}

	#[test]
	fn malformed_column_falls_back_to_inherit() {
		// A corrupt scope must not hide the user's whole library.
		assert_eq!(
			LibraryScope::of(&device_with(Some(json!("everything")))),
			LibraryScope::Inherit
		);
		assert_eq!(
			LibraryScope::of(&device_with(Some(json!(["a", 7])))),
			LibraryScope::Inherit
		);
	}

	#[test]
	fn round_trips_through_the_column() {
		let scope = LibraryScope::Only(vec!["a".to_string()]);
		assert_eq!(
			LibraryScope::of(&device_with(scope.to_column_value())),
			scope
		);
		assert_eq!(LibraryScope::Inherit.to_column_value(), None);
	}

	#[test]
	fn from_library_ids_collapses_duplicates() {
		assert_eq!(
			LibraryScope::from(Some(vec![
				"b".to_string(),
				"a".to_string(),
				"b".to_string()
			])),
			LibraryScope::Only(vec!["a".to_string(), "b".to_string()])
		);
		assert_eq!(LibraryScope::from(None), LibraryScope::Inherit);
	}
}
