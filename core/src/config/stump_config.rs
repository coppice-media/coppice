//! Contains the [`StumpConfig`] struct and related functions for loading and saving configuration
//! values for a Stump application.
//!
//! Note: [`StumpConfig`] is constructed _before_ tracing is initializing. This is because the
//! configuration is used to determine the log file path and verbosity level. This means that any
//! logging that occurs during the construction of the [`StumpConfig`] should be done using the
//! standard `println!` or `eprintln!` macros.

use std::{env, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::{
	annotation_sync::{AnnotationSyncConfig, PartialAnnotationSyncConfig},
	audio::{AudioConfig, PartialAudioConfig},
	auth::{AuthConfig, PartialAuthConfig},
	database::{DatabaseConfig, PartialDatabaseConfig},
	env_keys::*,
	ingest::{IngestConfig, PartialIngestConfig},
	jobs::{JobsConfig, PartialJobsConfig},
	oidc_config::OidcConfig,
	pdf::{PartialPdfConfig, PdfConfig},
	protocols::{PartialProtocolsConfig, ProtocolsConfig},
	providers::{PartialProvidersConfig, ProvidersConfig},
	server::{PartialServerConfig, ServerConfig},
	transform::{PartialTransformConfig, TransformConfig},
};
use crate::{CoreError, CoreResult};
use stump_config_gen::StumpConfigGenerator;
use stump_media::MediaConfig;

/// Represents the configuration of a Stump application. This struct is generated at startup
/// using a TOML file, environment variables, or both and is input when creating a `StumpCore`
/// instance.
///
/// The settings are grouped into sub-structs (`server`, `database`, `jobs`, `protocols`,
/// `ingest`, `providers`, `auth`, `pdf`) that are flattened for serialization, so `Stump.toml` and the
/// environment keep their flat key set.
///
/// Example (boots a real core against the configured directory, so it is
/// not executed as a doctest):
/// ```no_run
/// use stump_core::{config::{self, StumpConfig}, StumpCore};
///
/// #[tokio::main]
/// async fn main() {
///   /// Get config dir from environment variables.
///   let config_dir = config::bootstrap_config_dir();
///
///   // Create a StumpConfig using the config file and environment variables.
///   let config = StumpConfig::new(config_dir)
///     // Load Stump.toml file (if any)
///     .with_config_file().unwrap()
///     // Overlay environment variables
///     .with_environment().unwrap();
///
///   // Ensure that config directory exists and write Stump.toml.
///   config.write_config_dir().unwrap();
///   // Create an instance of the stump core.
///   let core = StumpCore::new(config).await;
/// }
/// ```
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpConfig"))]
#[config_file_location(self.get_config_dir().join("Stump.toml"))]
pub struct StumpConfig {
	/// HTTP listener, logging, and proxy settings.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub server: ServerConfig,

	/// Database location and connection pool settings.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub database: DatabaseConfig,

	/// Background job and scan concurrency settings.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub jobs: JobsConfig,

	/// Runtime switches for optional protocol routers, the web UI, and uploads.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub protocols: ProtocolsConfig,

	/// Staged ingest directories and progress retention.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub ingest: IngestConfig,

	/// Metadata/content provider host settings.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub providers: ProvidersConfig,

	/// Annotation export sink directories and debounce timing.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub annotation_sync: AnnotationSyncConfig,

	/// Password hashing and session/token lifetimes.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub auth: AuthConfig,

	/// PDFium location and PDF rendering settings.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub pdf: PdfConfig,

	/// Comic page transform delivery settings.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub transform: TransformConfig,

	/// Audiobook converter defaults.
	#[nested]
	#[serde(flatten)]
	#[cfg_attr(feature = "graphql", graphql(flatten))]
	pub audio: AudioConfig,

	/// The configuration root for the Stump application, contains thumbnails, cache, and logs.
	#[debug_value(super::get_default_config_dir())]
	#[env_key(CONFIG_DIR_KEY)]
	#[required_by_new]
	pub config_dir: String,

	/// The precomputed media-processing configuration. This is populated when
	/// the complete Stump configuration has been loaded.
	#[serde(skip)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	#[default_value(MediaConfig::default())]
	#[debug_value(MediaConfig::default())]
	pub media: MediaConfig,

	/// OIDC authentication configuration
	#[serde(default)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	#[default_value(None)]
	pub oidc: Option<OidcConfig>,
}

impl StumpConfig {
	/// Computes the derived, read-only snapshots (currently the media-processing
	/// configuration) after all user and environment values have been applied.
	/// Must be called once before the config is shared.
	pub fn finalize(&mut self) {
		self.media = MediaConfig::from_values(
			self.get_config_dir(),
			self.pdf.pdfium_path.as_deref().map(PathBuf::from),
			self.pdf.pdf_cache_pages,
			self.pdf.pdf_prerender_range,
			self.pdf.pdf_render_dpi,
			self.pdf.pdf_max_dimension,
			self.pdf.pdf_high_quality,
			self.pdf.pdf_render_format.clone(),
			self.jobs.cpu_concurrency_limit(),
		);
	}

	/// Ensures that the configuration directory exists and saves the `StumpConfig`'s current values
	/// to Stump.toml in the configuration directory.
	///
	/// This function first checks if `config_dir` exists and creates it if it does not, then does the
	/// same for the thumbnails and cache directories. Finally, a Stump.toml file containing the current
	/// configuration values is written. Returns `Ok` on success and `Err` if paths are misconfigured or
	/// file IO errors are encountered.
	pub fn write_config_dir(&self) -> CoreResult<()> {
		// Check that config directory is configured correctly
		let config_dir = self.get_config_dir();
		if config_dir.is_file() {
			return Err(CoreError::InitializationError(format!(
				"Error writing config directory: {config_dir:?} is a file",
			)));
		}

		// And create directory if it is missing.
		if !config_dir.exists() {
			match std::fs::create_dir_all(config_dir.clone()) {
				Ok(_) => (),
				Err(e) => {
					return Err(CoreError::InitializationError(format!(
						"Failed to create Stump configuration directory at {:?}: {:?}",
						config_dir,
						e.to_string()
					)));
				},
			}
		}

		// Create cache and thumbnail directories if they are missing
		let cache_dir = self.get_cache_dir();
		let thumbs_dir = self.get_thumbnails_dir();
		let avatars_dir = self.get_avatars_dir();
		let emojis_dir = self.get_emojis_dir();
		let pdf_cache_dir = self.get_pdf_cache_dir();
		if !cache_dir.exists() {
			std::fs::create_dir(cache_dir).unwrap();
		}
		if !thumbs_dir.exists() {
			std::fs::create_dir(thumbs_dir).unwrap();
		}
		if !avatars_dir.exists() {
			std::fs::create_dir(avatars_dir).unwrap();
		}
		if !emojis_dir.exists() {
			std::fs::create_dir(emojis_dir).unwrap();
		}
		if !pdf_cache_dir.exists() {
			std::fs::create_dir_all(pdf_cache_dir).unwrap();
		}

		// Save configuration to Stump.toml
		let stump_toml = config_dir.join("Stump.toml");

		std::fs::write(
			stump_toml.as_path(),
			toml::to_string(&self).map_err(|e| {
				eprintln!("Failed to serialize StumpConfig to toml: {e}");
				CoreError::InitializationError(e.to_string())
			})?,
		)?;

		Ok(())
	}

	/// Returns True if the configuration profile is "debug" and False otherwise.
	pub fn is_debug(&self) -> bool {
		self.server.is_debug()
	}

	/// Returns a `PathBuf` to the Stump configuration directory.
	pub fn get_config_dir(&self) -> PathBuf {
		PathBuf::from(&self.config_dir)
	}

	/// Returns the configured ingest drop-folder root, defaulting below the
	/// application configuration directory.
	pub fn get_ingest_drop_dir(&self) -> PathBuf {
		self.ingest
			.ingest_drop_dir
			.as_deref()
			.map(PathBuf::from)
			.unwrap_or_else(|| self.get_config_dir().join("ingest/drop"))
	}

	/// Returns the configured immutable ingest staging root, defaulting below
	/// the application configuration directory.
	pub fn get_ingest_staging_dir(&self) -> PathBuf {
		self.ingest
			.ingest_staging_dir
			.as_deref()
			.map(PathBuf::from)
			.unwrap_or_else(|| self.get_config_dir().join("ingest/staging"))
	}

	/// Returns the configured annotation export root, defaulting below the
	/// application configuration directory.
	pub fn get_annotation_sync_root(&self) -> PathBuf {
		self.annotation_sync
			.annotation_sync_root
			.as_deref()
			.map(PathBuf::from)
			.unwrap_or_else(|| self.get_config_dir().join("annotations"))
	}

	/// Returns a `PathBuf` to the annotation attachment root. Attachment bytes
	/// live at `<root>/<annotation_id>/<sha256>.<ext>`; the database stores
	/// that path relative to this root.
	pub fn get_attachments_dir(&self) -> PathBuf {
		self.get_config_dir().join("attachments")
	}

	pub fn get_log_dir(&self) -> PathBuf {
		match &self.server.log_dir {
			Some(value) => PathBuf::from(value),
			None => self.get_config_dir(),
		}
	}

	/// Returns a `PathBuf` to the Stump cache directory.
	pub fn get_cache_dir(&self) -> PathBuf {
		PathBuf::from(&self.config_dir).join("cache")
	}

	/// Returns a `PathBuf` to the Stump thumbnails directory.
	pub fn get_thumbnails_dir(&self) -> PathBuf {
		PathBuf::from(&self.config_dir).join("thumbnails")
	}

	/// Returns a `PathBuf` to the Stump avatars directory
	pub fn get_avatars_dir(&self) -> PathBuf {
		PathBuf::from(&self.config_dir).join("avatars")
	}

	/// Returns a `PathBuf` to the Stump custom emojis directory
	pub fn get_emojis_dir(&self) -> PathBuf {
		PathBuf::from(&self.config_dir).join("emojis")
	}

	/// Returns a `PathBuf` to the PDF page cache directory
	pub fn get_pdf_cache_dir(&self) -> PathBuf {
		self.get_cache_dir().join("pdf_pages")
	}

	/// Returns a `PathBuf` to the comic transform cache directory
	pub fn get_transform_cache_dir(&self) -> PathBuf {
		self.get_cache_dir().join("transform")
	}

	/// Returns a `PathBuf` to the Stump log file.
	pub fn get_log_file(&self) -> PathBuf {
		self.get_config_dir().join("Stump.log")
	}
}

#[cfg(test)]
mod tests {
	use tempfile;

	use super::super::defaults::*;
	use super::*;

	#[test]
	fn test_writing_to_config_dir() {
		let tempdir = tempfile::tempdir().expect("Failed to create temporary directory");

		// Now we can create a StumpConfig rooted at the temporary directory
		let config_dir = tempdir.path().to_string_lossy().to_string();
		let mut config = StumpConfig::new(config_dir.clone());

		// Apply a partial config to set the values
		let partial_config = PartialStumpConfig {
			server: PartialServerConfig {
				profile: Some("release".to_string()),
				port: Some(1337),
				verbosity: Some(3),
				pretty_logs: Some(true),
				allowed_origins: Some(vec!["origin1".to_string(), "origin2".to_string()]),
				..Default::default()
			},
			database: PartialDatabaseConfig {
				db_path: Some("not_a_real_path".to_string()),
				..Default::default()
			},
			jobs: PartialJobsConfig {
				parallelism_multiplier: Some(DEFAULT_PARALLELISM_MULTIPLIER),
				..Default::default()
			},
			protocols: PartialProtocolsConfig {
				client_dir: Some("not_a_real_dir".to_string()),
				enable_webui: Some(true),
				enable_opds_progression: Some(false),
				enable_playground: Some(false),
				enable_koreader_sync: Some(false),
				enable_kobo_sync: Some(false),
				enable_komga: Some(false),
				enable_kavita: Some(false),
				..Default::default()
			},
			pdf: PartialPdfConfig {
				pdfium_path: Some("not_a_path_to_pdfium".to_string()),
				..Default::default()
			},
			..Default::default()
		};

		partial_config.apply_to_config(&mut config);

		// Write to the config directory
		config.write_config_dir().unwrap();

		// Load the toml that should have been created
		let new_toml_path = tempdir.path().join("Stump.toml");
		let new_toml_content = std::fs::read_to_string(new_toml_path).unwrap();
		let new_toml_vals =
			toml::from_str::<PartialStumpConfig>(&new_toml_content).unwrap();

		// And check its values against what we expect
		assert_eq!(
			new_toml_vals,
			PartialStumpConfig {
				server: PartialServerConfig {
					profile: Some("release".to_string()),
					ip: Some("0.0.0.0".to_string()),
					port: Some(1337),
					verbosity: Some(3),
					pretty_logs: Some(true),
					log_dir: None,
					colorful_logs: Some(false),
					allowed_origins: Some(vec![
						"origin1".to_string(),
						"origin2".to_string()
					]),
					trust_proxy_headers: Some(false),
					home_app_dir: None,
					library_roots: Some(vec![]),
				},
				database: PartialDatabaseConfig {
					db_path: Some("not_a_real_path".to_string()),
					db_timeout_secs: Some(DEFAULT_DB_TIMEOUT_SECS),
					db_max_connections: Some(DEFAULT_DB_MAX_CONNECTIONS),
					db_min_connections: Some(DEFAULT_DB_MIN_CONNECTIONS),
					sqlite_statement_cache_capacity: Some(
						DEFAULT_SQLITE_STATEMENT_CACHE_CAPACITY
					),
				},
				jobs: PartialJobsConfig {
					enable_background_jobs: Some(DEFAULT_ENABLE_BACKGROUND_JOBS),
					parallelism_multiplier: Some(DEFAULT_PARALLELISM_MULTIPLIER),
				},
				protocols: PartialProtocolsConfig {
					enable_koreader_sync: Some(false),
					enable_kobo_sync: Some(false),
					kobo_kepub_preconvert: Some(false),
					kobo_kepub_cache_max_age_days: Some(
						DEFAULT_KOBO_KEPUB_CACHE_MAX_AGE_DAYS
					),
					kobo_kepub_deflate_level: Some(DEFAULT_KOBO_KEPUB_DEFLATE_LEVEL),
					kobo_kepub_conversion: Some(false),
					enable_komga: Some(false),
					enable_kavita: Some(false),
					enable_abs: Some(false),
					enable_opds_progression: Some(false),
					enable_upload: Some(DEFAULT_ENABLE_UPLOAD),
					max_file_upload_size: Some(DEFAULT_MAX_FILE_UPLOAD_SIZE),
					max_image_upload_size: Some(DEFAULT_MAX_IMAGE_UPLOAD_SIZE),
					attachment_max_bytes: Some(DEFAULT_ATTACHMENT_MAX_BYTES),
					enable_webui: Some(true),
					enable_playground: Some(false),
					client_dir: Some("not_a_real_dir".to_string()),
				},
				ingest: PartialIngestConfig {
					ingest_drop_dir: None,
					ingest_staging_dir: None,
					ingest_editor_dir: None,
					ingest_progress_retention: Some(DEFAULT_INGEST_PROGRESS_RETENTION),
					ingest_preprocess_command: None,
					ingest_preprocess_timeout_secs: Some(
						DEFAULT_INGEST_PREPROCESS_TIMEOUT_SECS,
					),
				},
				providers: PartialProvidersConfig {
					enable_providers: Some(DEFAULT_ENABLE_PROVIDERS),
					provider_cache_max_bytes: Some(DEFAULT_PROVIDER_CACHE_MAX_BYTES),
					virtual_series_ttl: Some(DEFAULT_VIRTUAL_SERIES_TTL_SECS),
					provider_gc_days: Some(DEFAULT_PROVIDER_GC_DAYS),
					provider_health_interval_secs: Some(
						DEFAULT_PROVIDER_HEALTH_INTERVAL_SECS
					),
					provider_health_dead_after: Some(DEFAULT_PROVIDER_HEALTH_DEAD_AFTER),
					source_definitions_url: Some(
						DEFAULT_SOURCE_DEFINITIONS_URL.to_string()
					),
				},
				annotation_sync: PartialAnnotationSyncConfig {
					annotation_sync_root: None,
					annotation_sync_debounce_secs: Some(
						DEFAULT_ANNOTATION_SYNC_DEBOUNCE_SECS
					),
				},
				auth: PartialAuthConfig {
					password_hash_cost: Some(DEFAULT_PASSWORD_HASH_COST),
					session_ttl: Some(DEFAULT_SESSION_TTL),
					access_token_ttl: Some(DEFAULT_ACCESS_TOKEN_TTL),
					refresh_token_ttl: Some(DEFAULT_REFRESH_TOKEN_TTL),
					expired_session_cleanup_interval: Some(
						DEFAULT_SESSION_EXPIRY_CLEANUP_INTERVAL
					),
				},
				pdf: PartialPdfConfig {
					pdfium_path: Some("not_a_path_to_pdfium".to_string()),
					pdf_render_dpi: Some(DEFAULT_PDF_RENDER_DPI),
					pdf_max_dimension: Some(DEFAULT_PDF_MAX_DIMENSION),
					pdf_render_format: Some(DEFAULT_PDF_RENDER_FORMAT.to_string()),
					pdf_cache_pages: Some(DEFAULT_PDF_CACHE_PAGES),
					pdf_prerender_range: Some(DEFAULT_PDF_PRERENDER_RANGE),
					pdf_high_quality: Some(DEFAULT_PDF_HIGH_QUALITY),
				},
				transform: PartialTransformConfig {
					transform_enabled: Some(DEFAULT_TRANSFORM_ENABLED),
					transform_kobo_profile: Some(
						DEFAULT_TRANSFORM_KOBO_PROFILE.to_string()
					),
					transform_cache_max_bytes: Some(DEFAULT_TRANSFORM_CACHE_MAX_BYTES),
				},
				audio: PartialAudioConfig {
					audio_canonical: Some(DEFAULT_AUDIO_CANONICAL.to_string()),
					audio_aac_bitrate: Some(DEFAULT_AUDIO_AAC_BITRATE.to_string()),
					audio_ffmpeg: Some(DEFAULT_AUDIO_FFMPEG.to_string()),
				},
				config_dir: Some(config_dir),
				media: None,
				oidc: None,
			}
		);

		// Ensure that the temporary directory is deleted
		tempdir
			.close()
			.expect("Failed to delete temporary directory");
	}

	#[test]
	fn test_simulate_first_boot() {
		temp_env::with_vars(
			[
				(PORT_KEY, Some("1337")),
				(VERBOSITY_KEY, Some("2")),
				(ENABLE_PLAYGROUND_KEY, Some("true")),
				(DB_MAX_CONNECTIONS_KEY, Some("2")),
				(DB_MIN_CONNECTIONS_KEY, Some("1")),
				(SQLITE_STATEMENT_CACHE_CAPACITY_KEY, Some("32")),
				(ENABLE_BACKGROUND_JOBS_KEY, Some("false")),
				(ENABLE_WEBUI_KEY, Some("false")),
				(HASH_COST_KEY, Some("1")),
				(ENABLE_KOMGA_KEY, Some("true")),
				(ENABLE_KAVITA_KEY, Some("true")),
				(ENABLE_ABS_KEY, Some("true")),
			],
			|| {
				let tempdir =
					tempfile::tempdir().expect("Failed to create temporary directory");
				// Now we can create a StumpConfig rooted at the temporary directory
				let config_dir = tempdir.path().to_string_lossy().to_string();
				let generated = StumpConfig::new(config_dir.clone())
					.with_config_file()
					.expect("Failed to generate StumpConfig from Stump.toml")
					.with_environment()
					.expect("Failed to generate StumpConfig from environment");

				assert_eq!(
					generated,
					StumpConfig {
						server: ServerConfig {
							profile: "release".to_string(),
							ip: "0.0.0.0".to_string(),
							port: 1337,
							verbosity: 2,
							pretty_logs: true,
							log_dir: None,
							colorful_logs: false,
							allowed_origins: vec![],
							home_app_dir: None,
							trust_proxy_headers: false,
							library_roots: vec![],
						},
						database: DatabaseConfig {
							db_path: None,
							db_timeout_secs: DEFAULT_DB_TIMEOUT_SECS,
							db_max_connections: 2,
							db_min_connections: 1,
							sqlite_statement_cache_capacity: 32,
						},
						jobs: JobsConfig {
							enable_background_jobs: false,
							parallelism_multiplier: DEFAULT_PARALLELISM_MULTIPLIER,
						},
						protocols: ProtocolsConfig {
							enable_koreader_sync: false,
							enable_kobo_sync: false,
							kobo_kepub_preconvert: false,
							kobo_kepub_cache_max_age_days:
								DEFAULT_KOBO_KEPUB_CACHE_MAX_AGE_DAYS,
							kobo_kepub_deflate_level: DEFAULT_KOBO_KEPUB_DEFLATE_LEVEL,
							kobo_kepub_conversion: false,
							enable_komga: true,
							enable_kavita: true,
							enable_abs: true,
							enable_opds_progression: false,
							enable_upload: DEFAULT_ENABLE_UPLOAD,
							max_file_upload_size: DEFAULT_MAX_FILE_UPLOAD_SIZE,
							max_image_upload_size: DEFAULT_MAX_IMAGE_UPLOAD_SIZE,
							attachment_max_bytes: DEFAULT_ATTACHMENT_MAX_BYTES,
							enable_webui: false,
							enable_playground: true,
							client_dir: "./client".to_string(),
						},
						ingest: IngestConfig {
							ingest_drop_dir: None,
							ingest_staging_dir: None,
							ingest_editor_dir: None,
							ingest_progress_retention: DEFAULT_INGEST_PROGRESS_RETENTION,
							ingest_preprocess_command: None,
							ingest_preprocess_timeout_secs:
								DEFAULT_INGEST_PREPROCESS_TIMEOUT_SECS,
						},
						providers: ProvidersConfig {
							enable_providers: DEFAULT_ENABLE_PROVIDERS,
							provider_cache_max_bytes: DEFAULT_PROVIDER_CACHE_MAX_BYTES,
							virtual_series_ttl: DEFAULT_VIRTUAL_SERIES_TTL_SECS,
							provider_gc_days: DEFAULT_PROVIDER_GC_DAYS,
							provider_health_interval_secs:
								DEFAULT_PROVIDER_HEALTH_INTERVAL_SECS,
							provider_health_dead_after:
								DEFAULT_PROVIDER_HEALTH_DEAD_AFTER,
							source_definitions_url: DEFAULT_SOURCE_DEFINITIONS_URL
								.to_string(),
						},
						annotation_sync: AnnotationSyncConfig {
							annotation_sync_root: None,
							annotation_sync_debounce_secs:
								DEFAULT_ANNOTATION_SYNC_DEBOUNCE_SECS,
						},
						auth: AuthConfig {
							password_hash_cost: 1,
							session_ttl: DEFAULT_SESSION_TTL,
							access_token_ttl: DEFAULT_ACCESS_TOKEN_TTL,
							refresh_token_ttl: DEFAULT_REFRESH_TOKEN_TTL,
							expired_session_cleanup_interval:
								DEFAULT_SESSION_EXPIRY_CLEANUP_INTERVAL,
						},
						pdf: PdfConfig {
							pdfium_path: None,
							pdf_render_dpi: DEFAULT_PDF_RENDER_DPI,
							pdf_max_dimension: DEFAULT_PDF_MAX_DIMENSION,
							pdf_render_format: DEFAULT_PDF_RENDER_FORMAT.to_string(),
							pdf_cache_pages: DEFAULT_PDF_CACHE_PAGES,
							pdf_prerender_range: DEFAULT_PDF_PRERENDER_RANGE,
							pdf_high_quality: DEFAULT_PDF_HIGH_QUALITY,
						},
						transform: TransformConfig {
							transform_enabled: DEFAULT_TRANSFORM_ENABLED,
							transform_kobo_profile: DEFAULT_TRANSFORM_KOBO_PROFILE
								.to_string(),
							transform_cache_max_bytes: DEFAULT_TRANSFORM_CACHE_MAX_BYTES,
						},
						audio: AudioConfig {
							audio_canonical: DEFAULT_AUDIO_CANONICAL.to_string(),
							audio_aac_bitrate: DEFAULT_AUDIO_AAC_BITRATE.to_string(),
							audio_ffmpeg: DEFAULT_AUDIO_FFMPEG.to_string(),
						},
						config_dir,
						media: MediaConfig::default(),
						oidc: None,
					}
				);
			},
		);
	}

	/// A `Stump.toml` written before the config was split into groups (flat keys,
	/// original field order) must load unchanged: the groups are pure structure.
	#[test]
	fn test_pre_split_toml_loads_into_grouped_config() {
		let tempdir = tempfile::tempdir().expect("Failed to create temporary directory");
		let config_dir = tempdir.path().to_string_lossy().to_string();
		std::fs::write(
			tempdir.path().join("Stump.toml"),
			format!(
				r#"profile = "debug"
ip = "127.0.0.1"
port = 1337
verbosity = 3
pretty_logs = false
colorful_logs = true
db_path = "/var/lib/stump"
db_timeout_secs = 45
db_max_connections = 4
db_min_connections = 2
sqlite_statement_cache_capacity = 64
client_dir = "/srv/stump/client"
enable_webui = false
enable_background_jobs = false
ingest_drop_dir = "/ingest/drop"
ingest_progress_retention = 42
config_dir = "{config_dir}"
allowed_origins = ["https://a.example", "https://b.example"]
pdfium_path = "/opt/pdfium/libpdfium.so"
enable_playground = true
enable_koreader_sync = true
enable_kobo_sync = true
kobo_kepub_preconvert = true
kobo_kepub_cache_max_age_days = 7
kobo_kepub_deflate_level = 9
kobo_kepub_conversion = true
enable_komga = true
enable_opds_progression = true
password_hash_cost = 4
session_ttl = 10
access_token_ttl = 20
refresh_token_ttl = 30
expired_session_cleanup_interval = 40
parallelism_multiplier = 3
max_image_upload_size = 1000
enable_upload = true
max_file_upload_size = 2000
pdf_render_dpi = 72
pdf_max_dimension = 800
pdf_render_format = "png"
pdf_cache_pages = false
pdf_prerender_range = 0
pdf_high_quality = false
trust_proxy_headers = true

[oidc]
enabled = true
client_id = "stump"
issuer_url = "https://issuer.example"
client_secret = "secret"
"#
			),
		)
		.unwrap();

		let loaded = StumpConfig::new(config_dir.clone())
			.with_config_file()
			.expect("pre-split Stump.toml must load");

		assert_eq!(
			loaded.server,
			ServerConfig {
				profile: "debug".to_string(),
				ip: "127.0.0.1".to_string(),
				port: 1337,
				verbosity: 3,
				pretty_logs: false,
				log_dir: None,
				colorful_logs: true,
				allowed_origins: vec![
					"https://a.example".to_string(),
					"https://b.example".to_string()
				],
				home_app_dir: None,
				trust_proxy_headers: true,
				library_roots: vec![],
			}
		);
		assert_eq!(
			loaded.database,
			DatabaseConfig {
				db_path: Some("/var/lib/stump".to_string()),
				db_timeout_secs: 45,
				db_max_connections: 4,
				db_min_connections: 2,
				sqlite_statement_cache_capacity: 64,
			}
		);
		assert_eq!(
			loaded.jobs,
			JobsConfig {
				enable_background_jobs: false,
				parallelism_multiplier: 3,
			}
		);
		assert_eq!(
			loaded.protocols,
			ProtocolsConfig {
				enable_koreader_sync: true,
				enable_kobo_sync: true,
				kobo_kepub_preconvert: true,
				kobo_kepub_cache_max_age_days: 7,
				kobo_kepub_deflate_level: 9,
				kobo_kepub_conversion: true,
				enable_komga: true,
				enable_kavita: false,
				enable_abs: false,
				enable_opds_progression: true,
				enable_upload: true,
				max_file_upload_size: 2000,
				max_image_upload_size: 1000,
				attachment_max_bytes: DEFAULT_ATTACHMENT_MAX_BYTES,
				enable_webui: false,
				enable_playground: true,
				client_dir: "/srv/stump/client".to_string(),
			}
		);
		assert_eq!(
			loaded.ingest,
			IngestConfig {
				ingest_drop_dir: Some("/ingest/drop".to_string()),
				ingest_staging_dir: None,
				ingest_editor_dir: None,
				ingest_progress_retention: 42,
				ingest_preprocess_command: None,
				ingest_preprocess_timeout_secs: DEFAULT_INGEST_PREPROCESS_TIMEOUT_SECS,
			}
		);
		assert_eq!(
			loaded.auth,
			AuthConfig {
				password_hash_cost: 4,
				session_ttl: 10,
				access_token_ttl: 20,
				refresh_token_ttl: 30,
				expired_session_cleanup_interval: 40,
			}
		);
		assert_eq!(
			loaded.pdf,
			PdfConfig {
				pdfium_path: Some("/opt/pdfium/libpdfium.so".to_string()),
				pdf_render_dpi: 72,
				pdf_max_dimension: 800,
				pdf_render_format: "png".to_string(),
				pdf_cache_pages: false,
				pdf_prerender_range: 0,
				pdf_high_quality: false,
			}
		);
		assert_eq!(loaded.providers, ProvidersConfig::new());
		assert_eq!(loaded.config_dir, config_dir);
		let oidc = loaded.oidc.as_ref().expect("oidc table must still load");
		assert!(oidc.enabled);
		assert_eq!(oidc.client_id, "stump");

		// And the grouped config still serializes to the same flat key set.
		let written = toml::to_string(&loaded).unwrap();
		for key in [
			"profile = \"debug\"",
			"db_timeout_secs = 45",
			"enable_background_jobs = false",
			"kobo_kepub_deflate_level = 9",
			"ingest_progress_retention = 42",
			"password_hash_cost = 4",
			"pdf_render_format = \"png\"",
			"trust_proxy_headers = true",
			"[oidc]",
		] {
			assert!(written.contains(key), "missing `{key}` in:\n{written}");
		}
		assert!(
			!written.contains("[server]"),
			"groups must stay flat:\n{written}"
		);
	}

	#[test]
	fn test_finalize_snapshots_pdf_and_jobs_settings() {
		let mut config = StumpConfig::new("/tmp/stump-finalize".to_string());
		config.pdf.pdf_render_dpi = 96;
		config.pdf.pdfium_path = Some("/opt/pdfium".to_string());
		config.jobs.parallelism_multiplier = 1;
		config.finalize();

		assert_eq!(config.media.pdf_render_dpi, 96);
		assert_eq!(config.media.pdfium_path, Some(PathBuf::from("/opt/pdfium")));
		assert_eq!(
			config.media.cpu_concurrency_limit(),
			config.jobs.cpu_concurrency_limit()
		);
		assert_eq!(config.media.get_config_dir(), config.get_config_dir());
	}

	#[test]
	fn test_komga_defaults() {
		let release = StumpConfig::new("/tmp/stump-komga-defaults".to_string());
		assert!(!release.protocols.enable_komga);
		assert!(StumpConfig::debug().protocols.enable_komga);
	}
}
