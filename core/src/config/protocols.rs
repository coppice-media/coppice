use std::env;

use serde::{Deserialize, Serialize};

use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Runtime switches for the optional protocol routers, the web UI, and uploads.
/// Flattened into [`super::StumpConfig`].
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpProtocolsConfig"))]
pub struct ProtocolsConfig {
	/// Indicates if the KoReader sync feature should be enabled.
	#[default_value(false)]
	#[env_key(ENABLE_KOREADER_SYNC_KEY)]
	#[debug_value(true)]
	pub enable_koreader_sync: bool,

	/// Indicates if the Kobo sync feature should be enabled.
	#[default_value(false)]
	#[env_key(ENABLE_KOBO_SYNC_KEY)]
	#[debug_value(true)]
	pub enable_kobo_sync: bool,

	/// Whether EPUB files should be converted to KEPUB after a library scan.
	#[default_value(false)]
	#[env_key(KOBO_KEPUB_PRECONVERT_KEY)]
	pub kobo_kepub_preconvert: bool,

	/// Number of days an unused KEPUB cache entry is retained.
	#[default_value(DEFAULT_KOBO_KEPUB_CACHE_MAX_AGE_DAYS)]
	#[env_key(KOBO_KEPUB_CACHE_MAX_AGE_DAYS_KEY)]
	pub kobo_kepub_cache_max_age_days: u32,

	/// Deflate level used for generated KEPUB files.
	#[default_value(DEFAULT_KOBO_KEPUB_DEFLATE_LEVEL)]
	#[env_key(KOBO_KEPUB_DEFLATE_LEVEL_KEY)]
	#[validator(validate_kobo_kepub_deflate_level)]
	pub kobo_kepub_deflate_level: u32,

	/// Whether Kobo downloads should be converted to KEPUB with deterministic Kobo spans.
	#[default_value(false)]
	#[env_key(KOBO_KEPUB_CONVERSION_KEY)]
	pub kobo_kepub_conversion: bool,

	/// Indicates if the Komga compatibility feature should be enabled.
	#[default_value(false)]
	#[env_key(ENABLE_KOMGA_KEY)]
	#[debug_value(true)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_komga: bool,

	/// Indicates if the Komf-compatible metadata API should be enabled. This
	/// remains off even in debug builds; the Komf Cargo feature must also be
	/// compiled and Komga compatibility enabled before it can be mounted.
	#[default_value(false)]
	#[env_key(ENABLE_KOMF_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_komf: bool,

	/// Indicates if the Kavita compatibility API (`/api/{Library,Series,Reader,...}`)
	/// should be mounted.
	#[default_value(false)]
	#[env_key(ENABLE_KAVITA_KEY)]
	#[debug_value(true)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_kavita: bool,

	/// Indicates if the Audiobookshelf compatibility API (`/api/{libraries,items,session,...}`)
	/// should be mounted. The routes are compiled in behind the `abs` Cargo
	/// feature and are only mounted when this is true.
	#[default_value(false)]
	#[env_key(ENABLE_ABS_KEY)]
	#[debug_value(true)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_abs: bool,

	/// Indicates if OPDS page access should automatically track reading progression.
	/// When disabled, clients loading/preloading pages won't trigger progress updates.
	#[default_value(false)]
	#[env_key(ENABLE_OPDS_PROGRESSION_KEY)]
	pub enable_opds_progression: bool,

	/// Whether or not the server will allow users with the appropriate permissions to upload books and series.
	#[default_value(DEFAULT_ENABLE_UPLOAD)]
	#[env_key(ENABLE_UPLOAD_KEY)]
	pub enable_upload: bool,

	/// The maximum size, in bytes, of files that can be uploaded to be included in libraries.
	#[default_value(DEFAULT_MAX_FILE_UPLOAD_SIZE)]
	#[env_key(MAX_FILE_UPLOAD_SIZE_KEY)]
	pub max_file_upload_size: usize,

	/// The maximum file size, in bytes, of images that can be uploaded, e.g., as thumbnails for users,
	/// libraries, series, or media.
	#[default_value(DEFAULT_MAX_IMAGE_UPLOAD_SIZE)]
	#[env_key(MAX_IMAGE_UPLOAD_SIZE_KEY)]
	pub max_image_upload_size: usize,

	/// The maximum size, in bytes, of a single annotation attachment accepted
	/// by `PUT /v1/annotations/{id}/attachments/{kind}`.
	#[default_value(DEFAULT_ATTACHMENT_MAX_BYTES)]
	#[env_key(ATTACHMENT_MAX_BYTES_KEY)]
	pub attachment_max_bytes: usize,

	/// Indicates if the web UI redirect routes and GraphQL playground should be enabled.
	#[default_value(DEFAULT_ENABLE_WEBUI)]
	#[env_key(ENABLE_WEBUI_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_webui: bool,

	/// Indicates if the GraphQL playground should be enabled.
	#[default_value(false)]
	#[env_key(ENABLE_PLAYGROUND_KEY)]
	pub enable_playground: bool,
}

fn validate_kobo_kepub_deflate_level(level: &u32) -> bool {
	if (1..=12).contains(level) {
		return true;
	}

	eprintln!("Invalid Kobo KEPUB deflate level {level}; expected 1..=12");
	false
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_kobo_kepub_defaults_and_deflate_bounds() {
		let config = ProtocolsConfig::new();
		assert!(!config.kobo_kepub_preconvert);
		assert_eq!(config.kobo_kepub_cache_max_age_days, 90);
		assert_eq!(config.kobo_kepub_deflate_level, 6);
		assert!(validate_kobo_kepub_deflate_level(&1));
		assert!(validate_kobo_kepub_deflate_level(&12));
		assert!(!validate_kobo_kepub_deflate_level(&0));
		assert!(!validate_kobo_kepub_deflate_level(&13));
		assert_eq!(KOBO_KEPUB_PRECONVERT_KEY, "KOBO_KEPUB_PRECONVERT");
		assert_eq!(
			KOBO_KEPUB_CACHE_MAX_AGE_DAYS_KEY,
			"KOBO_KEPUB_CACHE_MAX_AGE_DAYS"
		);
		assert_eq!(KOBO_KEPUB_DEFLATE_LEVEL_KEY, "KOBO_KEPUB_DEFLATE_LEVEL");
	}

	#[test]
	fn test_komf_is_off_by_default_in_all_profiles() {
		assert!(!ProtocolsConfig::new().enable_komf);
		assert!(!ProtocolsConfig::debug().enable_komf);
		assert_eq!(ENABLE_KOMF_KEY, "STUMP_ENABLE_KOMF");
	}

	#[test]
	fn test_compat_api_defaults() {
		let release = ProtocolsConfig::new();
		assert!(!release.enable_komga);
		assert!(!release.enable_kavita);
		assert!(!release.enable_abs);
		assert!(!release.enable_komf);
		let debug = ProtocolsConfig::debug();
		assert!(debug.enable_komga);
		assert!(debug.enable_kavita);
		assert!(debug.enable_abs);
		assert!(!debug.enable_komf);
	}
}
