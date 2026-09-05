use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Comic page transform delivery settings. Flattened into [`super::StumpConfig`].
///
/// When enabled, Kobo devices requesting a CBZ/CBR/PDF book file receive a
/// fixed-layout KEPUB whose pages were resized/toned for their device (see
/// `stump_media::transform`). Files are cached under
/// [`super::StumpConfig::get_transform_cache_dir`] and evicted LRU-style to
/// `transform_cache_max_bytes`.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpTransformConfig"))]
pub struct TransformConfig {
	/// Enable comic transform delivery for Kobo devices (CBZ/CBR/PDF → KEPUB).
	#[default_value(DEFAULT_TRANSFORM_ENABLED)]
	#[env_key(TRANSFORM_ENABLED_KEY)]
	pub transform_enabled: bool,

	/// Named `stump_media::transform::TransformProfile` preset used for Kobo
	/// devices that have no `transform_profile` of their own.
	#[default_value(DEFAULT_TRANSFORM_KOBO_PROFILE.to_string())]
	#[env_key(TRANSFORM_KOBO_PROFILE_KEY)]
	#[validator(validate_transform_kobo_profile)]
	pub transform_kobo_profile: String,

	/// Byte budget for the transform cache; least-recently-used entries are
	/// removed until the directory fits.
	#[default_value(DEFAULT_TRANSFORM_CACHE_MAX_BYTES)]
	#[env_key(TRANSFORM_CACHE_MAX_BYTES_KEY)]
	pub transform_cache_max_bytes: u64,
}

fn validate_transform_kobo_profile(name: &String) -> bool {
	if stump_media::transform::TransformProfile::preset(name).is_some() {
		return true;
	}

	eprintln!(
		"Invalid transform Kobo profile {name}; expected one of: {}",
		stump_media::transform::TransformProfile::preset_names().join(", ")
	);
	false
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_transform_defaults() {
		let config = TransformConfig::new();
		assert!(!config.transform_enabled);
		assert_eq!(config.transform_kobo_profile, "clara");
		assert_eq!(
			config.transform_cache_max_bytes,
			DEFAULT_TRANSFORM_CACHE_MAX_BYTES
		);
		assert_eq!(TRANSFORM_ENABLED_KEY, "STUMP_TRANSFORM_ENABLED");
		assert_eq!(TRANSFORM_KOBO_PROFILE_KEY, "STUMP_TRANSFORM_KOBO_PROFILE");
		assert_eq!(
			TRANSFORM_CACHE_MAX_BYTES_KEY,
			"STUMP_TRANSFORM_CACHE_MAX_BYTES"
		);
	}

	#[test]
	fn test_transform_kobo_profile_validation() {
		assert!(validate_transform_kobo_profile(&"libra".to_string()));
		assert!(validate_transform_kobo_profile(&"phone".to_string()));
		assert!(!validate_transform_kobo_profile(&"kobo".to_string()));
		assert!(!validate_transform_kobo_profile(&String::new()));
	}
}
