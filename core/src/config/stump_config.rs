//! Contains the [`StumpConfig`] struct and related functions for loading and saving configuration
//! values for a Stump application.
//!
//! Note: [`StumpConfig`] is constructed _before_ tracing is initializing. This is because the
//! configuration is used to determine the log file path and verbosity level. This means that any
//! logging that occurs during the construction of the [`StumpConfig`] should be done using the
//! standard `println!` or `eprintln!` macros.

use std::{env, path::PathBuf};

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use super::oidc_config::OidcConfig;
use crate::{CoreError, CoreResult};
use stump_config_gen::StumpConfigGenerator;
use stump_media::MediaConfig;

// TODO(env): i think DatabaseConfig enum with e.g. SQLite and Postgres variants would be nice
// TODO(postgres): the vars are not toml-supported atm, not sure if this really matters. i kept it
// like that bc idk what to do about the password, and having all of the config except password in toml
// felt funny? idk ill wait until someone complains maybe >:)
// TODO(env): prefix with STUMP_ for consistency
pub mod env_keys {
	pub const CONFIG_DIR_KEY: &str = "STUMP_CONFIG_DIR";
	pub const IN_DOCKER_KEY: &str = "STUMP_IN_DOCKER";
	pub const PROFILE_KEY: &str = "STUMP_PROFILE";
	pub const IP_KEY: &str = "STUMP_IP";
	pub const PORT_KEY: &str = "STUMP_PORT";
	pub const VERBOSITY_KEY: &str = "STUMP_VERBOSITY";
	pub const PRETTY_LOGS_KEY: &str = "STUMP_PRETTY_LOGS";
	pub const LOG_DIR_KEY: &str = "STUMP_LOG_DIR";
	pub const COLORFUL_LOGS_KEY: &str = "STUMP_COLORFUL_LOGS";
	pub const DB_PATH_KEY: &str = "STUMP_DB_PATH";
	pub const DATABASE_URL_KEY: &str = "STUMP_DATABASE_URL";
	pub const DB_PASSWORD_KEY: &str = "STUMP_DB_PASSWORD";
	pub const DB_HOST_KEY: &str = "STUMP_DB_HOST";
	pub const DB_PORT_KEY: &str = "STUMP_DB_PORT";
	pub const DB_NAME_KEY: &str = "STUMP_DB_NAME";
	pub const DB_USER_KEY: &str = "STUMP_DB_USER";
	pub const DB_TIMEOUT_KEY: &str = "STUMP_DB_TIMEOUT_SECS";
	pub const DB_MAX_CONNECTIONS_KEY: &str = "STUMP_DB_MAX_CONNECTIONS";
	pub const DB_MIN_CONNECTIONS_KEY: &str = "STUMP_DB_MIN_CONNECTIONS";
	pub const SQLITE_STATEMENT_CACHE_CAPACITY_KEY: &str =
		"STUMP_SQLITE_STATEMENT_CACHE_CAPACITY";
	pub const ENABLE_BACKGROUND_JOBS_KEY: &str = "STUMP_ENABLE_BACKGROUND_JOBS";
	pub const CLIENT_KEY: &str = "STUMP_CLIENT_DIR";
	pub const ENABLE_WEBUI_KEY: &str = "STUMP_ENABLE_WEBUI";
	pub const ORIGINS_KEY: &str = "STUMP_ALLOWED_ORIGINS";
	pub const PDFIUM_KEY: &str = "PDFIUM_PATH";
	pub const ENABLE_PLAYGROUND_KEY: &str = "STUMP_ENABLE_PLAYGROUND";
	pub const ENABLE_KOREADER_SYNC_KEY: &str = "ENABLE_KOREADER_SYNC";
	pub const ENABLE_KOBO_SYNC_KEY: &str = "ENABLE_KOBO_SYNC";
	pub const KOBO_KEPUB_CONVERSION_KEY: &str = "KOBO_KEPUB_CONVERSION";
	pub const KOBO_KEPUB_PRECONVERT_KEY: &str = "KOBO_KEPUB_PRECONVERT";
	pub const KOBO_KEPUB_CACHE_MAX_AGE_DAYS_KEY: &str = "KOBO_KEPUB_CACHE_MAX_AGE_DAYS";
	pub const KOBO_KEPUB_DEFLATE_LEVEL_KEY: &str = "KOBO_KEPUB_DEFLATE_LEVEL";
	pub const ENABLE_KOMGA_KEY: &str = "STUMP_ENABLE_KOMGA";

	pub const ENABLE_OPDS_PROGRESSION_KEY: &str = "ENABLE_OPDS_PROGRESSION";
	pub const HASH_COST_KEY: &str = "HASH_COST";
	pub const SESSION_TTL_KEY: &str = "SESSION_TTL";
	pub const SESSION_EXPIRY_INTERVAL_KEY: &str = "SESSION_EXPIRY_CLEANUP_INTERVAL";
	pub const MAX_IMAGE_UPLOAD_SIZE_KEY: &str = "STUMP_MAX_IMAGE_UPLOAD_SIZE";
	pub const ENABLE_UPLOAD_KEY: &str = "STUMP_ENABLE_UPLOAD";
	pub const MAX_FILE_UPLOAD_SIZE_KEY: &str = "STUMP_MAX_FILE_UPLOAD_SIZE";
	pub const PDF_RENDER_DPI_KEY: &str = "STUMP_PDF_RENDER_DPI";
	pub const PDF_MAX_DIMENSION_KEY: &str = "STUMP_PDF_MAX_DIMENSION";
	pub const PDF_RENDER_FORMAT_KEY: &str = "STUMP_PDF_RENDER_FORMAT";
	pub const PDF_CACHE_PAGES_KEY: &str = "STUMP_PDF_CACHE_PAGES";
	pub const PDF_PRERENDER_RANGE_KEY: &str = "STUMP_PDF_PRERENDER_RANGE";
	pub const PDF_HIGH_QUALITY_KEY: &str = "STUMP_PDF_HIGH_QUALITY";
	pub const OIDC_ENABLED_KEY: &str = "STUMP_OIDC_ENABLED";
	pub const OIDC_CLIENT_ID_KEY: &str = "STUMP_OIDC_CLIENT_ID";
	pub const OIDC_CLIENT_SECRET_KEY: &str = "STUMP_OIDC_CLIENT_SECRET";
	pub const OIDC_ISSUER_URL_KEY: &str = "STUMP_OIDC_ISSUER_URL";
	pub const OIDC_SCOPES_KEY: &str = "STUMP_OIDC_SCOPES";
	pub const OIDC_ALLOW_REGISTRATION_KEY: &str = "STUMP_OIDC_ALLOW_REGISTRATION";
	pub const OIDC_DISABLE_LOCAL_AUTH_KEY: &str = "STUMP_OIDC_DISABLE_LOCAL_AUTH";
	pub const OIDC_EXTRA_AUDIENCES_KEY: &str = "STUMP_OIDC_EXTRA_AUDIENCES";
	pub const OIDC_CA_CERT_FILE_KEY: &str = "STUMP_OIDC_CA_CERT_FILE";
	pub const TRUST_PROXY_HEADERS_KEY: &str = "STUMP_TRUST_PROXY_HEADERS";
	pub const PARALLELISM_MULTIPLIER_KEY: &str = "STUMP_PARALLELISM_MULTIPLIER";
	pub const INGEST_DROP_DIR_KEY: &str = "INGEST_DROP_DIR";
	pub const INGEST_STAGING_DIR_KEY: &str = "INGEST_STAGING_DIR";
	pub const INGEST_EDITOR_DIR_KEY: &str = "INGEST_EDITOR_DIR";
	pub const INGEST_PROGRESS_RETENTION_KEY: &str = "INGEST_PROGRESS_RETENTION";
}
use env_keys::*;

pub mod defaults {
	pub const DEFAULT_ENABLE_WEBUI: bool = true;
	pub const DEFAULT_PASSWORD_HASH_COST: u32 = 12;
	pub const DEFAULT_SESSION_TTL: i64 = 3600 * 24 * 3; // 3 days
	pub const DEFAULT_ACCESS_TOKEN_TTL: i64 = 3600 * 24; // 1 days
	pub const DEFAULT_REFRESH_TOKEN_TTL: i64 = 3600 * 24 * 30; // 30 days
	pub const DEFAULT_SESSION_EXPIRY_CLEANUP_INTERVAL: u64 = 60 * 60 * 24; // 24 hours
	pub const DEFAULT_MAX_IMAGE_UPLOAD_SIZE: usize = 20 * 1024 * 1024; // 20 MB
	pub const DEFAULT_ENABLE_UPLOAD: bool = false;
	pub const DEFAULT_MAX_FILE_UPLOAD_SIZE: usize = 20 * 1024 * 1024; // 20 MB
	pub const DEFAULT_PDF_RENDER_DPI: u32 = 150; // Good balance of quality and performance
	pub const DEFAULT_PDF_MAX_DIMENSION: u32 = 1200; // Optimized for faster rendering while maintaining quality
	pub const DEFAULT_PDF_RENDER_FORMAT: &str = "webp"; // Default to WebP for better compression
	pub const DEFAULT_PDF_CACHE_PAGES: bool = true; // Enable page caching by default
	pub const DEFAULT_PDF_PRERENDER_RANGE: u32 = 5; // Pre-render 5 pages before/after current
	pub const DEFAULT_PDF_HIGH_QUALITY: bool = true; // Enable high-quality rendering by default
	pub const DEFAULT_BOOK_COMPLETION_DEDUP_TIMEOUT_SECS: i64 = 60 * 60 * 24; // 1 day
	pub const DEFAULT_INGEST_PROGRESS_RETENTION: u32 = 10_000;
	pub const DEFAULT_PARALLELISM_MULTIPLIER: usize = 2;
	pub const DEFAULT_DB_MAX_CONNECTIONS: u32 = 10;
	pub const DEFAULT_DB_MIN_CONNECTIONS: u32 = 0;
	pub const DEFAULT_KOBO_KEPUB_CACHE_MAX_AGE_DAYS: u32 = 90;
	pub const DEFAULT_KOBO_KEPUB_DEFLATE_LEVEL: u32 = 6;
	pub const DEFAULT_SQLITE_STATEMENT_CACHE_CAPACITY: usize = 100;
	pub const DEFAULT_ENABLE_BACKGROUND_JOBS: bool = true;
}
use defaults::*;

/// Represents the configuration of a Stump application. This struct is generated at startup
/// using a TOML file, environment variables, or both and is input when creating a `StumpCore`
/// instance.
///
/// Example:
/// ```
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
	/// The "release" | "debug" profile with which the application is running.
	#[default_value("release".to_string())]
	#[debug_value("debug".to_string())]
	#[env_key(PROFILE_KEY)]
	#[validator(do_validate_profile)]
	pub profile: String,

	/// The IP address on which to listen on (default: "0.0.0.0").
	#[default_value("0.0.0.0".to_string())]
	#[env_key(IP_KEY)]
	pub ip: String,

	/// The port from which to serve the application (default: 10801).
	#[default_value(10801)]
	#[env_key(PORT_KEY)]
	pub port: u16,

	/// The verbosity with which system logs are visible (default: 1).
	#[default_value(1)]
	#[env_key(VERBOSITY_KEY)]
	pub verbosity: u64,

	/// Whether or not to pretty print logs.
	#[default_value(true)]
	#[env_key(PRETTY_LOGS_KEY)]
	pub pretty_logs: bool,

	/// The directory where the applicaiton logs will be stored
	#[default_value(None)]
	#[env_key(LOG_DIR_KEY)]
	pub log_dir: Option<String>,

	/// Whether or not to include ANSI color codes in log files.
	#[default_value(false)]
	#[env_key(COLORFUL_LOGS_KEY)]
	pub colorful_logs: bool,

	/// An optional custom path for the database.
	#[default_value(None)]
	#[env_key(DB_PATH_KEY)]
	pub db_path: Option<String>,

	#[default_value(30)]
	#[env_key(DB_TIMEOUT_KEY)]
	pub db_timeout_secs: u64,

	/// The maximum number of database connections in the pool.
	#[default_value(DEFAULT_DB_MAX_CONNECTIONS)]
	#[env_key(DB_MAX_CONNECTIONS_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub db_max_connections: u32,

	/// The minimum number of database connections to keep in the pool.
	#[default_value(DEFAULT_DB_MIN_CONNECTIONS)]
	#[env_key(DB_MIN_CONNECTIONS_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub db_min_connections: u32,

	/// The number of prepared statements cached per SQLite connection.
	#[default_value(DEFAULT_SQLITE_STATEMENT_CACHE_CAPACITY)]
	#[env_key(SQLITE_STATEMENT_CACHE_CAPACITY_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub sqlite_statement_cache_capacity: usize,

	/// The client directory.
	#[default_value("./client".to_string())]
	#[debug_value(env!("CARGO_MANIFEST_DIR").to_string() + "/../web/dist")]
	#[env_key(CLIENT_KEY)]
	pub client_dir: String,

	/// Indicates if the web UI and its SPA fallback routes should be served.
	#[default_value(DEFAULT_ENABLE_WEBUI)]
	#[env_key(ENABLE_WEBUI_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_webui: bool,

	/// Whether background jobs such as scheduled scans are enabled.
	#[default_value(DEFAULT_ENABLE_BACKGROUND_JOBS)]
	#[env_key(ENABLE_BACKGROUND_JOBS_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub enable_background_jobs: bool,

	/// Optional root for files admitted by the staged ingest drop-folder watcher.
	/// When unset, this resolves to `<config_dir>/ingest/drop`.
	#[default_value(None)]
	#[env_key(INGEST_DROP_DIR_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub ingest_drop_dir: Option<String>,

	/// Optional root for immutable staged ingest files.
	/// When unset, this resolves to `<config_dir>/ingest/staging`.
	#[default_value(None)]
	#[env_key(INGEST_STAGING_DIR_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub ingest_staging_dir: Option<String>,

	/// Directory holding the built ingest editor (`editor/build`). When set and
	/// it contains `index.html`, the editor is served under `/editor`.
	#[default_value(None)]
	#[env_key(INGEST_EDITOR_DIR_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub ingest_editor_dir: Option<String>,

	/// Number of typed ingest progress events retained for replay.
	#[default_value(DEFAULT_INGEST_PROGRESS_RETENTION)]
	#[env_key(INGEST_PROGRESS_RETENTION_KEY)]
	#[cfg_attr(feature = "graphql", graphql(skip))]
	pub ingest_progress_retention: u32,

	/// The configuration root for the Stump application, contains thumbnails, cache, and logs.
	#[debug_value(super::get_default_config_dir())]
	#[env_key(CONFIG_DIR_KEY)]
	#[required_by_new]
	pub config_dir: String,

	/// A list of origins for CORS.
	#[default_value(vec![])]
	#[env_key(ORIGINS_KEY)]
	pub allowed_origins: Vec<String>,

	/// Path to the PDFium binary for PDF support.
	#[default_value(None)]
	#[env_key(PDFIUM_KEY)]
	pub pdfium_path: Option<String>,

	/// Indicates if the GraphQL playground should be enabled.
	#[default_value(false)]
	#[env_key(ENABLE_PLAYGROUND_KEY)]
	pub enable_playground: bool,

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

	/// Indicates if OPDS page access should automatically track reading progression.
	/// When disabled, clients loading/preloading pages won't trigger progress updates.
	#[default_value(false)]
	#[env_key(ENABLE_OPDS_PROGRESSION_KEY)]
	pub enable_opds_progression: bool,

	/// Password hash cost
	#[default_value(DEFAULT_PASSWORD_HASH_COST)]
	#[env_key(HASH_COST_KEY)]
	pub password_hash_cost: u32,

	/// The time in seconds that a login session will be valid for.
	#[default_value(DEFAULT_SESSION_TTL)]
	#[env_key(SESSION_TTL_KEY)]
	pub session_ttl: i64,

	#[default_value(DEFAULT_ACCESS_TOKEN_TTL)]
	#[env_key("ACCESS_TOKEN_TTL")]
	pub access_token_ttl: i64,

	#[default_value(DEFAULT_REFRESH_TOKEN_TTL)]
	#[env_key("REFRESH_TOKEN_TTL")]
	pub refresh_token_ttl: i64,

	/// The interval at which automatic deleted session cleanup is performed.
	#[default_value(DEFAULT_SESSION_EXPIRY_CLEANUP_INTERVAL)]
	#[env_key(SESSION_EXPIRY_INTERVAL_KEY)]
	pub expired_session_cleanup_interval: u64,

	/// A multiplier applied to the number of logical CPUs to derive the default scanner concurrency
	/// limit. Increasing can speed things up but will increase resource usage
	#[default_value(DEFAULT_PARALLELISM_MULTIPLIER)]
	#[env_key(PARALLELISM_MULTIPLIER_KEY)]
	pub parallelism_multiplier: usize,

	/// The maximum file size, in bytes, of images that can be uploaded, e.g., as thumbnails for users,
	/// libraries, series, or media.
	#[default_value(DEFAULT_MAX_IMAGE_UPLOAD_SIZE)]
	#[env_key(MAX_IMAGE_UPLOAD_SIZE_KEY)]
	pub max_image_upload_size: usize,

	/// Whether or not the server will allow users with the appropriate permissions to upload books and series.
	#[default_value(DEFAULT_ENABLE_UPLOAD)]
	#[env_key(ENABLE_UPLOAD_KEY)]
	pub enable_upload: bool,

	/// The maximum size, in bytes, of files that can be uploaded to be included in libraries.
	#[default_value(DEFAULT_MAX_FILE_UPLOAD_SIZE)]
	#[env_key(MAX_FILE_UPLOAD_SIZE_KEY)]
	pub max_file_upload_size: usize,

	/// The DPI (dots per inch) to use when rendering PDF pages as images.
	#[default_value(DEFAULT_PDF_RENDER_DPI)]
	#[env_key(PDF_RENDER_DPI_KEY)]
	pub pdf_render_dpi: u32,

	/// The maximum width or height dimension for rendered PDF pages.
	#[default_value(DEFAULT_PDF_MAX_DIMENSION)]
	#[env_key(PDF_MAX_DIMENSION_KEY)]
	pub pdf_max_dimension: u32,

	/// The image format to use for rendered PDF pages (webp, png, jpeg).
	#[default_value(DEFAULT_PDF_RENDER_FORMAT.to_string())]
	#[env_key(PDF_RENDER_FORMAT_KEY)]
	pub pdf_render_format: String,

	/// Whether to enable disk caching for rendered PDF pages.
	#[default_value(DEFAULT_PDF_CACHE_PAGES)]
	#[env_key(PDF_CACHE_PAGES_KEY)]
	pub pdf_cache_pages: bool,

	/// Number of pages to pre-render before and after the current page.
	#[default_value(DEFAULT_PDF_PRERENDER_RANGE)]
	#[env_key(PDF_PRERENDER_RANGE_KEY)]
	pub pdf_prerender_range: u32,

	/// Whether to enable high-quality rendering with smoothing (slower but better quality).
	#[default_value(DEFAULT_PDF_HIGH_QUALITY)]
	#[env_key(PDF_HIGH_QUALITY_KEY)]
	pub pdf_high_quality: bool,
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

	/// Whether to trust proxy headers for determining client IP and scheme (e.g., X-Forwarded-For)
	#[default_value(false)]
	#[env_key(TRUST_PROXY_HEADERS_KEY)]
	pub trust_proxy_headers: bool,
}

impl StumpConfig {
	/// Refreshes the media configuration after all user and environment values
	/// have been applied.
	pub fn finalize_media_config(&mut self) {
		self.media = MediaConfig::from_values(
			self.get_config_dir(),
			self.pdfium_path.as_deref().map(PathBuf::from),
			self.pdf_cache_pages,
			self.pdf_prerender_range,
			self.pdf_render_dpi,
			self.pdf_max_dimension,
			self.pdf_high_quality,
			self.pdf_render_format.clone(),
			self.cpu_concurrency_limit(),
		);
	}
	/// returns a sensible default concurrency limit based on the number of logical cpus
	/// available to the process, scaled by `parallelism_multiplier`.
	pub fn cpu_concurrency_limit(&self) -> usize {
		let multiplier = std::cmp::max(self.parallelism_multiplier, 1);
		std::thread::available_parallelism()
			.map(|n| n.get() * multiplier)
			.unwrap_or(multiplier)
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
		self.profile.as_str() == "debug"
	}

	/// Returns a `PathBuf` to the Stump configuration directory.
	pub fn get_config_dir(&self) -> PathBuf {
		PathBuf::from(&self.config_dir)
	}
	/// Returns the configured ingest drop-folder root, defaulting below the
	/// application configuration directory.
	pub fn get_ingest_drop_dir(&self) -> PathBuf {
		self.ingest_drop_dir
			.as_deref()
			.map(PathBuf::from)
			.unwrap_or_else(|| self.get_config_dir().join("ingest/drop"))
	}

	/// Returns the configured immutable ingest staging root, defaulting below
	/// the application configuration directory.
	pub fn get_ingest_staging_dir(&self) -> PathBuf {
		self.ingest_staging_dir
			.as_deref()
			.map(PathBuf::from)
			.unwrap_or_else(|| self.get_config_dir().join("ingest/staging"))
	}

	pub fn get_log_dir(&self) -> PathBuf {
		match &self.log_dir {
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

	/// Returns a `PathBuf` to the Stump log file.
	pub fn get_log_file(&self) -> PathBuf {
		self.get_config_dir().join("Stump.log")
	}

	/// Parse the configured PDF render format into a SupportedImageFormat.
	/// Falls back to WebP if the configured format is invalid.
	pub fn get_pdf_render_format(
		&self,
	) -> models::shared::image_processor_options::SupportedImageFormat {
		use models::shared::image_processor_options::SupportedImageFormat;

		match self.pdf_render_format.to_lowercase().as_str() {
			"webp" => SupportedImageFormat::Webp,
			"jpeg" | "jpg" => SupportedImageFormat::Jpeg,
			"png" => SupportedImageFormat::Png,
			_ => {
				tracing::warn!(
					format = self.pdf_render_format,
					"Invalid PDF render format, falling back to WebP"
				);
				SupportedImageFormat::Webp
			},
		}
	}
}
fn validate_kobo_kepub_deflate_level(level: &u32) -> bool {
	if (1..=12).contains(level) {
		return true;
	}

	eprintln!("Invalid Kobo KEPUB deflate level {level}; expected 1..=12");
	false
}

fn do_validate_profile(profile: &String) -> bool {
	if profile == "release" || profile == "debug" {
		return true;
	}

	eprintln!("Invalid profile value: {profile}");
	false
}

#[cfg(test)]
mod tests {
	use tempfile;

	use super::*;

	#[test]
	fn test_writing_to_config_dir() {
		let tempdir = tempfile::tempdir().expect("Failed to create temporary directory");

		// Now we can create a StumpConfig rooted at the temporary directory
		let config_dir = tempdir.path().to_string_lossy().to_string();
		let mut config = StumpConfig::new(config_dir.clone());

		// Apply a partial config to set the values
		let partial_config = PartialStumpConfig {
			profile: Some("release".to_string()),
			port: Some(1337),
			verbosity: Some(3),
			pretty_logs: Some(true),
			log_dir: None,
			colorful_logs: None,
			db_path: Some("not_a_real_path".to_string()),
			client_dir: Some("not_a_real_dir".to_string()),
			enable_webui: Some(true),
			parallelism_multiplier: Some(DEFAULT_PARALLELISM_MULTIPLIER),
			enable_opds_progression: Some(false),
			ip: None,
			config_dir: None,
			allowed_origins: Some(vec!["origin1".to_string(), "origin2".to_string()]),
			pdfium_path: Some("not_a_path_to_pdfium".to_string()),
			enable_playground: Some(false),
			enable_koreader_sync: Some(false),
			enable_kobo_sync: Some(false),
			kobo_kepub_preconvert: None,
			kobo_kepub_cache_max_age_days: None,
			kobo_kepub_deflate_level: None,
			kobo_kepub_conversion: None,
			enable_komga: Some(false),
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
				profile: Some("release".to_string()),
				ip: Some("0.0.0.0".to_string()),
				port: Some(1337),
				verbosity: Some(3),
				pretty_logs: Some(true),
				log_dir: None,
				colorful_logs: Some(false),
				db_path: Some("not_a_real_path".to_string()),
				db_timeout_secs: Some(30),
				client_dir: Some("not_a_real_dir".to_string()),
				db_max_connections: Some(DEFAULT_DB_MAX_CONNECTIONS),
				db_min_connections: Some(DEFAULT_DB_MIN_CONNECTIONS),
				sqlite_statement_cache_capacity: Some(
					DEFAULT_SQLITE_STATEMENT_CACHE_CAPACITY
				),
				enable_background_jobs: Some(DEFAULT_ENABLE_BACKGROUND_JOBS),
				ingest_drop_dir: None,
				ingest_staging_dir: None,
				ingest_editor_dir: None,
				ingest_progress_retention: Some(DEFAULT_INGEST_PROGRESS_RETENTION),
				enable_webui: Some(true),
				config_dir: Some(config_dir),
				parallelism_multiplier: Some(DEFAULT_PARALLELISM_MULTIPLIER),
				allowed_origins: Some(vec!["origin1".to_string(), "origin2".to_string()]),
				pdfium_path: Some("not_a_path_to_pdfium".to_string()),
				enable_opds_progression: Some(false),
				enable_playground: Some(false),
				enable_koreader_sync: Some(false),
				enable_kobo_sync: Some(false),
				kobo_kepub_preconvert: Some(false),
				kobo_kepub_cache_max_age_days: Some(
					DEFAULT_KOBO_KEPUB_CACHE_MAX_AGE_DAYS
				),
				kobo_kepub_deflate_level: Some(DEFAULT_KOBO_KEPUB_DEFLATE_LEVEL),
				kobo_kepub_conversion: Some(false),
				enable_komga: Some(false),
				password_hash_cost: Some(DEFAULT_PASSWORD_HASH_COST),
				session_ttl: Some(DEFAULT_SESSION_TTL),
				access_token_ttl: Some(DEFAULT_ACCESS_TOKEN_TTL),
				refresh_token_ttl: Some(DEFAULT_REFRESH_TOKEN_TTL),
				expired_session_cleanup_interval: Some(
					DEFAULT_SESSION_EXPIRY_CLEANUP_INTERVAL
				),
				max_image_upload_size: Some(DEFAULT_MAX_IMAGE_UPLOAD_SIZE),
				enable_upload: Some(DEFAULT_ENABLE_UPLOAD),
				max_file_upload_size: Some(DEFAULT_MAX_FILE_UPLOAD_SIZE),
				pdf_render_dpi: Some(DEFAULT_PDF_RENDER_DPI),
				pdf_max_dimension: Some(DEFAULT_PDF_MAX_DIMENSION),
				pdf_render_format: Some(DEFAULT_PDF_RENDER_FORMAT.to_string()),
				pdf_cache_pages: Some(DEFAULT_PDF_CACHE_PAGES),
				pdf_prerender_range: Some(DEFAULT_PDF_PRERENDER_RANGE),
				pdf_high_quality: Some(DEFAULT_PDF_HIGH_QUALITY),
				oidc: None,
				media: None,
				trust_proxy_headers: Some(false),
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
						profile: "release".to_string(),
						ip: "0.0.0.0".to_string(),
						port: 1337,
						verbosity: 2,
						pretty_logs: true,
						log_dir: None,
						colorful_logs: false,
						db_path: None,
						db_timeout_secs: 30,
						client_dir: "./client".to_string(),
						enable_webui: false,
						config_dir,
						db_max_connections: 2,
						ingest_drop_dir: None,
						ingest_staging_dir: None,
						ingest_editor_dir: None,
						ingest_progress_retention: DEFAULT_INGEST_PROGRESS_RETENTION,
						db_min_connections: 1,
						sqlite_statement_cache_capacity: 32,
						enable_background_jobs: false,
						allowed_origins: vec![],
						pdfium_path: None,
						enable_playground: true,
						enable_koreader_sync: false,
						enable_kobo_sync: false,
						kobo_kepub_preconvert: false,
						kobo_kepub_cache_max_age_days:
							DEFAULT_KOBO_KEPUB_CACHE_MAX_AGE_DAYS,
						kobo_kepub_deflate_level: DEFAULT_KOBO_KEPUB_DEFLATE_LEVEL,
						kobo_kepub_conversion: false,
						enable_komga: true,
						enable_opds_progression: false,
						password_hash_cost: 1,
						session_ttl: DEFAULT_SESSION_TTL,
						access_token_ttl: DEFAULT_ACCESS_TOKEN_TTL,
						refresh_token_ttl: DEFAULT_REFRESH_TOKEN_TTL,
						expired_session_cleanup_interval:
							DEFAULT_SESSION_EXPIRY_CLEANUP_INTERVAL,
						parallelism_multiplier: DEFAULT_PARALLELISM_MULTIPLIER,

						max_image_upload_size: DEFAULT_MAX_IMAGE_UPLOAD_SIZE,
						enable_upload: DEFAULT_ENABLE_UPLOAD,
						max_file_upload_size: DEFAULT_MAX_FILE_UPLOAD_SIZE,
						pdf_render_dpi: DEFAULT_PDF_RENDER_DPI,
						pdf_max_dimension: DEFAULT_PDF_MAX_DIMENSION,
						pdf_render_format: DEFAULT_PDF_RENDER_FORMAT.to_string(),
						pdf_cache_pages: DEFAULT_PDF_CACHE_PAGES,
						pdf_prerender_range: DEFAULT_PDF_PRERENDER_RANGE,
						pdf_high_quality: DEFAULT_PDF_HIGH_QUALITY,
						oidc: None,
						media: MediaConfig::default(),
						trust_proxy_headers: false,
					}
				);
			},
		);
	}
	#[test]
	fn test_komga_defaults() {
		let release = StumpConfig::new("/tmp/stump-komga-defaults".to_string());
		assert!(!release.enable_komga);
		assert!(StumpConfig::debug().enable_komga);
	}
	#[test]
	fn test_kobo_kepub_defaults_and_deflate_bounds() {
		let config = StumpConfig::new("/tmp/stump-kobo-kepub-defaults".to_string());
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
}
