use async_graphql::{Error, ErrorExtensions};
use stump_core::CoreError;

/// Convert a core error into the GraphQL error representation.
///
/// Disabled optional services are actionable availability failures, so expose their
/// machine-readable code and feature name while preserving the core error message.
pub(crate) fn map_core_error(error: CoreError) -> Error {
	let message = error.to_string();

	match error {
		CoreError::FeatureDisabled(feature) => {
			Error::new(message).extend_with(|_, extensions| {
				extensions.set("code", "SERVICE_UNAVAILABLE");
				extensions.set("feature", feature);
			})
		},
		other => other.into(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use async_graphql::Value;

	#[test]
	fn feature_disabled_errors_include_service_unavailable_extensions() {
		let error = map_core_error(CoreError::FeatureDisabled("background jobs"));
		let extensions = error.extensions.as_ref().expect("extensions should be set");

		assert_eq!(error.message, "Feature disabled: background jobs");
		assert_eq!(
			extensions.get("code"),
			Some(&Value::from("SERVICE_UNAVAILABLE"))
		);
		assert_eq!(
			extensions.get("feature"),
			Some(&Value::from("background jobs"))
		);
	}

	#[test]
	fn ordinary_core_errors_remain_plain_graphql_errors() {
		let error =
			map_core_error(CoreError::InternalError("database unavailable".to_string()));

		assert_eq!(error.message, "database unavailable");
		assert!(error.extensions.is_none());
	}
}
