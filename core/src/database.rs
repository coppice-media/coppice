use std::{env, time::Duration};

use migrations::{Migrator, MigratorTrait};
use schematic::{Config, ConfigLoader};
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sea_orm::{
	self, ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult,
	SqlxSqliteConnector, TransactionTrait,
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
// TODO: expose fn that intakes conn to determine if sqlite v postgres and return diff values

const DB_URL_KEY: &str = "STUMP_DB_URL";

#[derive(Config)]
#[config(env_prefix = "STUMP_DB_")]
struct ConnectionConfig {
	#[setting(default = "localhost")]
	pub host: String,
	#[setting(default = 5432)]
	pub port: u16,
	#[setting(default = "stump")]
	pub name: String,
	#[setting(default = "stump")]
	pub user: String,
	#[setting(validate = schematic::validate::min_length(1))]
	pub password: String,
	// ^ schematic uses default impl if missing by default, which is honestly kinda
	// annoying. so instead ive added a basic validation rule
}

impl ConnectionConfig {
	pub fn to_database_url(&self) -> String {
		// Percent-encode the password so special characters don't break the URL
		let encoded_password = urlencoding::encode(&self.password);
		format!(
			"postgresql://{}:{encoded_password}@{}:{}/{}",
			self.user, self.host, self.port, self.name
		)
	}
}

fn resolve_database_url(config: &StumpConfig) -> String {
	// A full URL takes highest precedence (works for both postgres:// and sqlite://).
	if let Ok(url) = env::var(env_keys::DATABASE_URL_KEY) {
		return url;
	}
	if let Ok(url) = env::var(DB_URL_KEY) {
		return url;
	}

	if let Ok(loaded) = ConfigLoader::<ConnectionConfig>::new().load() {
		return loaded.config.to_database_url();
	}

	// Fall back to SQLite
	let config_dir = config.get_config_dir();
	if let Some(path) = config.database.db_path.clone() {
		format!("sqlite://{path}/stump.db?mode=rwc")
	} else if cfg!(debug_assertions) {
		format!("sqlite://{}/dev.db?mode=rwc", env!("CARGO_MANIFEST_DIR"))
	} else {
		format!("sqlite://{}/stump.db?mode=rwc", config_dir.display())
	}
}

pub fn validate_pool_config(config: &StumpConfig) -> Result<(), CoreError> {
	if config.database.db_max_connections == 0 {
		return Err(CoreError::InitializationError(format!(
			"Invalid database pool configuration: db_max_connections must be greater than zero (got {})",
			config.database.db_max_connections
		)));
	}
	if config.database.db_min_connections > config.database.db_max_connections {
		return Err(CoreError::InitializationError(format!(
			"Invalid database pool configuration: db_min_connections ({}) cannot exceed db_max_connections ({})",
			config.database.db_min_connections, config.database.db_max_connections
		)));
	}

	Ok(())
}

fn sqlite_pool_options(config: &StumpConfig) -> SqlitePoolOptions {
	SqlitePoolOptions::new()
		.max_connections(config.database.db_max_connections)
		.min_connections(config.database.db_min_connections)
		.acquire_timeout(Duration::from_secs(config.database.db_timeout_secs))
}

fn postgres_connect_options(
	connection_url: String,
	config: &StumpConfig,
) -> sea_orm::ConnectOptions {
	sea_orm::ConnectOptions::new(connection_url)
		.max_connections(config.database.db_max_connections)
		.min_connections(config.database.db_min_connections)
		.acquire_timeout(Duration::from_secs(config.database.db_timeout_secs))
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
		.statement_cache_capacity(config.database.sqlite_statement_cache_capacity)
		// TODO(sqlite): do proper eval for NORMAL synchronous mode
		// .synchronous(SqliteSynchronous::Normal)
		.busy_timeout(Duration::from_secs(config.database.db_timeout_secs)))
}

/// The number of connections the SQLite migration pool is allowed to open.
///
/// `sea-orm-migration` only wraps migrations in a transaction for PostgreSQL;
/// MySQL and SQLite get a bare [`DatabaseConnection`]
/// (`sea-orm-migration-1.1.16/src/migrator.rs:261-272`), so every DDL statement
/// of every migration is executed on whichever connection the pool hands out.
/// A migration that rebuilds a table — create the replacement, copy, drop the
/// original, rename — then straddles connections, and the one doing the rename
/// can still be holding the pre-drop schema: SQLite answers
/// `(code: 1) there is already another table or index with this name`. Running
/// the whole migration on a pool of exactly one connection removes the
/// interleaving entirely.
const SQLITE_MIGRATION_POOL_SIZE: u32 = 1;

fn force_reset_requested() -> bool {
	match env::var(FORCE_RESET_KEY) {
		Ok(value) => value == "true",
		Err(error) => {
			tracing::warn!(
				?error,
				"Failed to read `{FORCE_RESET_KEY}` environment variable"
			);
			false
		},
	}
}

async fn migrate(
	connection: &DatabaseConnection,
	force_reset: bool,
) -> Result<(), CoreError> {
	if force_reset && cfg!(debug_assertions) {
		if connection.get_database_backend() == DatabaseBackend::Sqlite {
			tracing::debug!("Forcing database reset");
			Migrator::down(connection, None).await?;
		} else {
			tracing::warn!("Force reset is only supported for SQLite");
			return Err(CoreError::DatabaseResetNotAllowed);
		}
	} else if force_reset {
		tracing::warn!("You can only force a reset in debug mode as a safety measure");
		return Err(CoreError::DatabaseResetNotAllowed);
	}

	if connection.get_database_backend() == DatabaseBackend::Sqlite {
		// sea-orm-migration only wraps migrations in a transaction on Postgres.
		// SQLite DDL is transactional too, so apply one migration per
		// transaction: a crash or error mid-migration rolls back instead of
		// leaving tables behind that make the migration unrepeatable.
		let pending = Migrator::get_pending_migrations(connection).await?.len();
		for _ in 0..pending {
			let txn = connection.begin().await?;
			Migrator::up(&txn, Some(1)).await?;
			txn.commit().await?;
		}
	} else {
		Migrator::up(connection, None).await?;
	}

	Ok(())
}

async fn sqlite_pool(
	connection_url: &str,
	config: &StumpConfig,
	options: SqlitePoolOptions,
) -> Result<DatabaseConnection, CoreError> {
	let pool = options
		.connect_with(sqlite_connect_options(connection_url, config)?)
		.await
		.map_err(|e| {
			CoreError::InternalError(format!("Failed to connect to SQLite: {e}"))
		})?;

	Ok(SqlxSqliteConnector::from_sqlx_sqlite_pool(pool))
}

pub async fn connect(config: &StumpConfig) -> Result<DatabaseConnection, CoreError> {
	validate_pool_config(config)?;
	let connection_url = resolve_database_url(config);
	let force_reset = force_reset_requested();

	if !connection_url.starts_with("sqlite://") {
		let connect_options = postgres_connect_options(connection_url, config);
		let connection = sea_orm::Database::connect(connect_options).await?;
		migrate(&connection, force_reset).await?;
		return Ok(connection);
	}

	// Migrate first, on a pool that cannot interleave (see
	// `SQLITE_MIGRATION_POOL_SIZE`), and hand the connection back before the
	// serving pool opens so the database is never held open twice.
	let migrator = sqlite_pool(
		&connection_url,
		config,
		SqlitePoolOptions::new()
			.max_connections(SQLITE_MIGRATION_POOL_SIZE)
			.acquire_timeout(Duration::from_secs(config.database.db_timeout_secs)),
	)
	.await?;
	let migrated = migrate(&migrator, force_reset).await;
	if let Err(error) = migrator.close().await {
		tracing::warn!(?error, "Failed to close the migration connection");
	}
	migrated?;

	sqlite_pool(&connection_url, config, sqlite_pool_options(config)).await
}

pub async fn connect_at(path: &str) -> Result<DatabaseConnection, CoreError> {
	let connection = sea_orm::Database::connect(path).await?;
	migrate(&connection, false).await?;
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
		config.database.db_max_connections = 0;

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
		config.database.db_max_connections = 2;
		config.database.db_min_connections = 3;

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
		config.database.db_max_connections = 2;
		config.database.db_min_connections = 1;
		config.database.sqlite_statement_cache_capacity = 32;

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

	/// `connect` used to run `Migrator::up` on the serving pool. Because
	/// `sea-orm-migration` does not transact SQLite migrations, the DDL of a
	/// single migration was spread over several pooled connections, and
	/// `m20260909_000000_add_ingest_media_targets` (create replacement, copy,
	/// drop original, rename) failed with
	/// `(code: 1) there is already another table or index with this name:
	/// ingest_analysis_jobs` on a fresh database. Migrating on a one-connection
	/// pool has to keep a multi-connection serving pool working.
	#[tokio::test]
	async fn migrates_a_fresh_database_behind_a_multi_connection_pool() {
		let dir = tempfile::tempdir().expect("tempdir");
		let mut config = StumpConfig::debug();
		config.database.db_path = Some(dir.path().display().to_string());
		config.database.db_max_connections = 4;
		config.database.db_min_connections = 0;

		// `resolve_database_url` prefers full URL aliases/PG credentials, and
		// `FORCE_DB_RESET` would turn this into a down-migration.
		let connection = temp_env::async_with_vars(
			[
				(env_keys::DATABASE_URL_KEY, None::<&str>),
				(DB_URL_KEY, None),
				(env_keys::DB_PASSWORD_KEY, None),
				(FORCE_RESET_KEY, None),
			],
			connect(&config),
		)
		.await
		.expect("a fresh SQLite database must migrate");

		let applied = migrations::Migrator::get_applied_migrations(&connection)
			.await
			.expect("migration status");
		assert!(
			applied.len() >= 2,
			"the whole migration chain ran, not just the first"
		);

		// The rebuilt table survived the drop-and-rename and is queryable from
		// a connection the migration never touched.
		connection
			.execute_unprepared("SELECT \"media_id\" FROM \"ingest_analysis_jobs\"")
			.await
			.expect("the rebuilt ingest_analysis_jobs table exists");
	}

	/// `sea-orm-migration` runs SQLite migrations outside a transaction, so a
	/// migration that failed halfway (crash, disk full, a broken precondition)
	/// left its first DDL statements behind without being recorded; the next
	/// boot then died with `table "..." already exists` and the database was
	/// unrecoverable without hand surgery. Each migration must be atomic.
	#[tokio::test]
	async fn a_failing_sqlite_migration_leaves_nothing_behind() {
		const TARGET: &str = "m20260909_000000_add_ingest_media_targets";
		let dir = tempfile::tempdir().expect("tempdir");
		let connection = sea_orm::Database::connect(format!(
			"sqlite://{}/stump.db?mode=rwc",
			dir.path().display()
		))
		.await
		.expect("open");
		let before = Migrator::migrations()
			.iter()
			.position(|m| m.name() == TARGET)
			.expect("target migration is registered") as u32;
		Migrator::up(&connection, Some(before))
			.await
			.expect("prefix");

		// Break the migration after its first statement: it creates the
		// replacement table, then copies from `ingest_analysis_jobs`.
		connection
			.execute_unprepared(
				"ALTER TABLE \"ingest_analysis_jobs\" RENAME TO \"ingest_analysis_jobs_gone\"",
			)
			.await
			.expect("rename");
		migrate(&connection, false)
			.await
			.expect_err("the copy step must fail");

		let leftover = connection
			.execute_unprepared("SELECT 1 FROM \"ingest_analysis_jobs_media_targets\"")
			.await;
		assert!(
			leftover.is_err(),
			"the failed migration's table was rolled back"
		);
		let applied = Migrator::get_applied_migrations(&connection)
			.await
			.expect("status");
		assert_eq!(
			applied.len() as u32,
			before,
			"the failed migration is not recorded"
		);

		// Restoring the precondition makes the same database migrate to the end.
		connection
			.execute_unprepared(
				"ALTER TABLE \"ingest_analysis_jobs_gone\" RENAME TO \"ingest_analysis_jobs\"",
			)
			.await
			.expect("rename back");
		migrate(&connection, false).await.expect("recovers");
		assert!(Migrator::get_pending_migrations(&connection)
			.await
			.expect("status")
			.is_empty());
	}
}
