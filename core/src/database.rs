use std::{env, time::Duration};

use migrations::{Migrator, MigratorTrait};
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sea_orm::{
	self, ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult,
	SqlxSqliteConnector,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::{
	config::{env_keys, StumpConfig},
	CoreError,
};

pub const FORCE_RESET_KEY: &str = "FORCE_DB_RESET";

/// A slightly lower max number of binding params for SQL queries, I believe
/// the default is 999
pub const SQLITE_BIND_LIMIT: usize = 900;

fn resolve_database_url(config: &StumpConfig) -> String {
	// A full DATABASE_URL takes highest precedence (works for both postgres:// and sqlite://)
	if let Ok(url) = env::var(env_keys::DATABASE_URL_KEY) {
		return url;
	}

	// A DB_PASSWORD env var signals PostgreSQL; compose the URL from individual components
	if let Ok(password) = env::var(env_keys::DB_PASSWORD_KEY) {
		let host =
			env::var(env_keys::DB_HOST_KEY).unwrap_or_else(|_| "localhost".to_string());
		let port = env::var(env_keys::DB_PORT_KEY).unwrap_or_else(|_| "5432".to_string());
		let name =
			env::var(env_keys::DB_NAME_KEY).unwrap_or_else(|_| "stump".to_string());
		let user =
			env::var(env_keys::DB_USER_KEY).unwrap_or_else(|_| "stump".to_string());
		// Percent-encode the password so special characters don't break the URL
		let encoded_password = urlencoding::encode(&password);
		return format!("postgresql://{user}:{encoded_password}@{host}:{port}/{name}");
	}

	// Fall back to SQLite
	let config_dir = config.get_config_dir();
	if let Some(path) = config.db_path.clone() {
		format!("sqlite://{path}/stump.db?mode=rwc")
	} else if cfg!(debug_assertions) {
		format!("sqlite://{}/dev.db?mode=rwc", env!("CARGO_MANIFEST_DIR"))
	} else {
		format!("sqlite://{}/stump.db?mode=rwc", config_dir.display())
	}
}

pub fn validate_pool_config(config: &StumpConfig) -> Result<(), CoreError> {
	if config.db_max_connections == 0 {
		return Err(CoreError::InitializationError(format!(
			"Invalid database pool configuration: db_max_connections must be greater than zero (got {})",
			config.db_max_connections
		)));
	}
	if config.db_min_connections > config.db_max_connections {
		return Err(CoreError::InitializationError(format!(
			"Invalid database pool configuration: db_min_connections ({}) cannot exceed db_max_connections ({})",
			config.db_min_connections, config.db_max_connections
		)));
	}

	Ok(())
}

fn sqlite_pool_options(config: &StumpConfig) -> SqlitePoolOptions {
	SqlitePoolOptions::new()
		.max_connections(config.db_max_connections)
		.min_connections(config.db_min_connections)
		.acquire_timeout(Duration::from_secs(config.db_timeout_secs))
}

fn postgres_connect_options(
	connection_url: String,
	config: &StumpConfig,
) -> sea_orm::ConnectOptions {
	sea_orm::ConnectOptions::new(connection_url)
		.max_connections(config.db_max_connections)
		.min_connections(config.db_min_connections)
		.acquire_timeout(Duration::from_secs(config.db_timeout_secs))
		.to_owned()
}

fn sqlite_connect_options(
	connection_url: &str,
	config: &StumpConfig,
) -> Result<SqliteConnectOptions, CoreError> {
	Ok(SqliteConnectOptions::from_str(connection_url)
		.map_err(|e| {
			CoreError::InternalError(format!("Invalid SQLite connection string: {e}"))
		})?
		// TODO(482): support this:
		// - add indexes (e.g., create index media_name on media (name collate NATURALSORT))
		// - maybe some sql magic (e.g., update sqlite_master set sql = replace(sql, 'collate NOCASE', 'collate NATURALSORT') WHERE type = 'table' AND name IN (...))
		// - will need to verify ^ doesn't break comparisons where case matters, though
		.collation("NATURALSORT", natord::compare)
		.statement_cache_capacity(config.sqlite_statement_cache_capacity)
		// TODO(sqlite): do proper eval for NORMAL synchronous mode
		// .synchronous(SqliteSynchronous::Normal)
		.busy_timeout(Duration::from_secs(config.db_timeout_secs)))
}

pub async fn connect(config: &StumpConfig) -> Result<DatabaseConnection, CoreError> {
	validate_pool_config(config)?;
	let connection_url = resolve_database_url(config);

	let connection = if connection_url.starts_with("sqlite://") {
		let options = sqlite_connect_options(&connection_url, config)?;
		let pool = sqlite_pool_options(config)
			.connect_with(options)
			.await
			.map_err(|e| {
				CoreError::InternalError(format!("Failed to connect to SQLite: {e}"))
			})?;
		SqlxSqliteConnector::from_sqlx_sqlite_pool(pool)
	} else {
		let connect_options = postgres_connect_options(connection_url, config);
		sea_orm::Database::connect(connect_options).await?
	};

	let force_reset = match env::var(FORCE_RESET_KEY) {
		Ok(value) => value == "true",
		Err(error) => {
			tracing::warn!(
				?error,
				"Failed to read `{FORCE_RESET_KEY}` environment variable"
			);
			false
		},
	};

	if force_reset && cfg!(debug_assertions) {
		if connection.get_database_backend() == DatabaseBackend::Sqlite {
			tracing::debug!("Forcing database reset");
			Migrator::down(&connection, None).await?;
		} else {
			tracing::warn!("Force reset is only supported for SQLite");
			return Err(CoreError::DatabaseResetNotAllowed);
		}
	} else if force_reset {
		tracing::warn!("You can only force a reset in debug mode as a safety measure");
		return Err(CoreError::DatabaseResetNotAllowed);
	}

	Migrator::up(&connection, None).await?;

	Ok(connection)
}

pub async fn connect_at(path: &str) -> Result<DatabaseConnection, CoreError> {
	let connection = sea_orm::Database::connect(path).await?;
	Migrator::up(&connection, None).await?;
	Ok(connection)
}

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CountQueryReturn {
	pub count: i64,
}

// TODO: Use strum, maybe move to models::shared::enums?

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum JournalMode {
	#[serde(alias = "wal")]
	#[default]
	WAL,
	#[serde(alias = "delete")]
	DELETE,
}

impl AsRef<str> for JournalMode {
	fn as_ref(&self) -> &str {
		match self {
			Self::WAL => "WAL",
			Self::DELETE => "DELETE",
		}
	}
}

impl FromStr for JournalMode {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_uppercase().as_str() {
			"WAL" => Ok(Self::WAL),
			"DELETE" => Ok(Self::DELETE),
			_ => Err(format!("Invalid or unsupported journal mode: {s}")),
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JournalModeQueryResult {
	pub journal_mode: JournalMode,
}

impl FromQueryResult for JournalModeQueryResult {
	fn from_query_result(
		res: &sea_orm::QueryResult,
		_pre: &str,
	) -> Result<Self, sea_orm::DbErr> {
		let journal_mode = match res.try_get::<String>("", "journal_mode") {
			Ok(value) => JournalMode::from_str(value.as_str()).unwrap_or_default(),
			_ => {
				tracing::warn!("No journal mode found! Defaulting to WAL assumption");
				JournalMode::default()
			},
		};

		Ok(Self { journal_mode })
	}
}

/// Splits a vector of items into chunks of at most [`SQLITE_BIND_LIMIT`]
pub fn chunk_vec_into<T, F, R>(items: Vec<T>, map_fn: F) -> Vec<R>
where
	F: Fn(Vec<T>) -> R,
	T: Clone,
{
	if items.is_empty() {
		return vec![];
	}

	items
		.chunks(SQLITE_BIND_LIMIT)
		.map(|chunk| map_fn(chunk.to_vec()))
		.collect()
}

/// Return an estimated batch size for inserts based on the number of parameters per row.
/// This is to reduce query complexity and avoid shit like "too many SQL variables"
pub fn get_insert_batch_size(param_count: usize) -> usize {
	SQLITE_BIND_LIMIT / param_count
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_config() -> StumpConfig {
		StumpConfig::new("/tmp/stump-database-test".to_string())
	}

	#[test]
	fn rejects_zero_max_connections_before_connecting() {
		let mut config = test_config();
		config.db_max_connections = 0;

		let error = validate_pool_config(&config).expect_err("zero max must be rejected");
		assert!(matches!(
			error,
			CoreError::InitializationError(message)
				if message.contains("db_max_connections")
					&& message.contains("greater than zero")
		));
	}

	#[test]
	fn rejects_min_connections_above_max() {
		let mut config = test_config();
		config.db_max_connections = 2;
		config.db_min_connections = 3;

		let error =
			validate_pool_config(&config).expect_err("min above max must be rejected");
		assert!(matches!(
			error,
			CoreError::InitializationError(message)
				if message.contains("db_min_connections")
					&& message.contains("db_max_connections")
		));
	}

	#[test]
	fn applies_pool_limits_to_sqlite_and_postgres_options() {
		let mut config = test_config();
		config.db_max_connections = 2;
		config.db_min_connections = 1;
		config.sqlite_statement_cache_capacity = 32;

		let sqlite_pool = sqlite_pool_options(&config);
		assert_eq!(sqlite_pool.get_max_connections(), 2);
		assert_eq!(sqlite_pool.get_min_connections(), 1);

		let postgres =
			postgres_connect_options("postgresql://localhost/stump".to_string(), &config);
		assert_eq!(postgres.get_max_connections(), Some(2));
		assert_eq!(postgres.get_min_connections(), Some(1));

		let sqlite = sqlite_connect_options("sqlite::memory:", &config)
			.expect("in-memory SQLite URL should parse");
		assert!(format!("{sqlite:?}").contains("statement_cache_capacity: 32"));
	}
}
