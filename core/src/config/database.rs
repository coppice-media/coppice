use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Database location and connection-pool settings. Flattened into [`super::StumpConfig`].
///
/// The PostgreSQL connection variables (`STUMP_DATABASE_URL`, `STUMP_DB_HOST`, ...) are
/// read directly from the environment by `stump_core::database` and are not TOML keys.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpDatabaseConfig"))]
pub struct DatabaseConfig {
	/// An optional custom path for the database.
	#[default_value(None)]
	#[env_key(DB_PATH_KEY)]
	pub db_path: Option<String>,

	#[default_value(DEFAULT_DB_TIMEOUT_SECS)]
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
}
