//! Write-transaction entry point.
//!
//! SQLite's `BEGIN` is *deferred*: no lock is taken until the first statement
//! runs. A transaction that reads before it writes therefore takes a read
//! snapshot first and then has to **promote** that snapshot to the write lock.
//! SQLite refuses to run the busy handler for a promotion — two connections
//! promoting at once would deadlock — so the promotion fails with `SQLITE_BUSY`
//! ("database is locked") the instant another connection holds the write lock.
//! `busy_timeout` is never consulted. `BEGIN IMMEDIATE` takes the write lock up
//! front, where the busy handler *is* used, so the transaction waits its turn
//! instead of failing.
//!
//! sea-orm 1.1 hard-codes `TransactionManager::begin(conn, None)` for every
//! backend (`sea-orm-1.1.16/src/database/transaction.rs:69-76`) and exposes no
//! hook for a custom begin statement: `begin_with_config` only carries an
//! isolation level and an access mode, and the SQLite driver logs
//! "Setting access mode in a SQLite transaction isn't supported" and drops both
//! (`src/driver/sqlx_sqlite.rs:296-308`). [`begin_write`] therefore opens the
//! transaction sea-orm's way and immediately re-opens it as `IMMEDIATE` on the
//! same pooled connection.

use sea_orm::{
	ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr,
	RuntimeErr, TransactionTrait,
};

/// Take the write lock now, honouring `busy_timeout`.
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";
/// A plain deferred `BEGIN`, used only to restore the state sqlx believes the
/// connection is in if [`begin_write`] fails halfway through the swap.
const BEGIN_DEFERRED: &str = "BEGIN";
/// Closes the deferred transaction sea-orm opened. It has neither read nor
/// written anything at that point, so this can never discard work.
const ROLLBACK: &str = "ROLLBACK";

/// Begins a transaction that is safe to **read from before it writes**.
///
/// On SQLite the transaction is started with `BEGIN IMMEDIATE`; on any other
/// backend, and on mock connections, this is exactly [`TransactionTrait::begin`].
///
/// Use this for every transaction that writes. A write-only transaction is not
/// broken by a deferred `BEGIN` today — its first statement is a write, which
/// acquires the lock through the busy handler — but it is one refactor away from
/// being broken by a read moving to the top, and paying for the lock a single
/// worker round-trip earlier costs nothing. Transactions that only ever read
/// (snapshot consistency for a multi-query response) must keep
/// [`TransactionTrait::begin`]: making those `IMMEDIATE` would serialise
/// readers behind writers.
///
/// # Why this is safe
///
/// The deferred transaction sea-orm hands back has run no statements, so it
/// holds no locks and no read snapshot: rolling it back is a no-op that cannot
/// lose work. sqlx tracks nothing about a SQLite transaction except a *depth*
/// counter, which it only moves in its own begin/commit/rollback paths
/// (`sqlx-sqlite-0.8.6/src/connection/worker.rs:210-252`) — plain statement
/// execution leaves it alone. The counter is therefore 1 both before and after
/// the swap, which is precisely the invariant sqlx maintains for a single-level
/// transaction, so [`DatabaseTransaction::commit`] and
/// [`DatabaseTransaction::rollback`] still emit `COMMIT`/`ROLLBACK` and return
/// the connection to the pool at depth 0.
///
/// This takes `&DatabaseConnection` rather than `impl TransactionTrait` on
/// purpose: `begin()` on an *existing* transaction opens a `SAVEPOINT`, and
/// swapping that for `ROLLBACK` + `BEGIN IMMEDIATE` would discard the enclosing
/// transaction. Nested transactions must keep using
/// [`TransactionTrait::begin`], and the signature makes that a compile error
/// rather than a runtime surprise.
pub async fn begin_write(
	conn: &DatabaseConnection,
) -> Result<DatabaseTransaction, DbErr> {
	let txn = conn.begin().await?;

	// Mock connections record `begin()` calls and reject unexpected statements;
	// non-SQLite backends do not have promotion failures to begin with.
	if conn.get_database_backend() != DbBackend::Sqlite || conn.is_mock_connection() {
		return Ok(txn);
	}

	txn.execute_unprepared(ROLLBACK).await?;

	match txn.execute_unprepared(BEGIN_IMMEDIATE).await {
		Ok(_) => Ok(txn),
		Err(error) => {
			// The connection is in autocommit but sqlx still counts a
			// transaction. Re-open the deferred one so `rollback` below matches
			// reality and the connection goes back to the pool at depth 0.
			match txn.execute_unprepared(BEGIN_DEFERRED).await {
				Ok(_) => {
					let _ = txn.rollback().await;
				},
				Err(restore_error) => {
					tracing::error!(
						?error,
						?restore_error,
						"Failed to restore a pooled SQLite connection after BEGIN IMMEDIATE failed"
					);
				},
			}
			Err(error)
		},
	}
}

/// Whether a database error is SQLite write-lock contention, i.e. the caller
/// can retry the whole request and reasonably expect it to succeed.
///
/// SQLite reports both `SQLITE_BUSY` (5) and `SQLITE_LOCKED` (6) as
/// "database is locked" / "database table is locked"; sqlx surfaces them as a
/// [`RuntimeErr::SqlxError`] whose `Display` carries the code, and sea-orm
/// flattens some of those into [`DbErr::Custom`] on the way out (for example
/// `Execution Error: error returned from database: (code: 5) database is
/// locked`). Matching the message is the only portable option: sea-orm does not
/// re-export `sqlx::sqlite::SqliteError`, so the native code is unreachable
/// without downcasting through a type we cannot name.
pub fn is_write_lock_contention(error: &DbErr) -> bool {
	let message = match error {
		DbErr::Query(RuntimeErr::SqlxError(inner))
		| DbErr::Exec(RuntimeErr::SqlxError(inner))
		| DbErr::Conn(RuntimeErr::SqlxError(inner)) => inner.to_string(),
		DbErr::Query(RuntimeErr::Internal(message))
		| DbErr::Exec(RuntimeErr::Internal(message))
		| DbErr::Conn(RuntimeErr::Internal(message)) => message.clone(),
		DbErr::Custom(message) => message.clone(),
		_ => return false,
	};

	let message = message.to_ascii_lowercase();
	message.contains("database is locked")
		|| message.contains("database table is locked")
		|| message.contains("(code: 5)")
		|| message.contains("(code: 6)")
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{ConnectOptions, Database, DbConn, Statement};
	use std::{sync::Arc, time::Duration};

	const SCHEMA: &str =
		"CREATE TABLE counters (id TEXT PRIMARY KEY, hits INTEGER NOT NULL)";

	async fn connect(dir: &std::path::Path) -> DbConn {
		let url = format!("sqlite://{}/lock.db?mode=rwc", dir.display());
		// sea-orm caps an unconfigured SQLite pool at a single connection
		// (`sea-orm-1.1.16/src/driver/sqlx_sqlite.rs:85-87`), which would make
		// the contender below block on `pool.acquire()` instead of on the write
		// lock and pass for the wrong reason.
		let mut options = ConnectOptions::new(url);
		options.max_connections(4).sqlx_logging(false);
		let conn = Database::connect(options).await.expect("connect");
		// Persisted in the database file, so setting it once is enough. The
		// 5s `busy_timeout` sqlx applies per connection by default is what
		// `BEGIN IMMEDIATE` waits on.
		conn.execute_unprepared("PRAGMA journal_mode=WAL")
			.await
			.expect("wal");
		conn
	}

	async fn hits(conn: &impl ConnectionTrait, id: &str) -> i64 {
		conn.query_one(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			"SELECT hits FROM counters WHERE id = ?",
			[id.into()],
		))
		.await
		.expect("select")
		.map(|row| row.try_get_by_index::<i64>(0).expect("hits column"))
		.unwrap_or_default()
	}

	/// Opens a write transaction on its own pooled connection and leaves it
	/// holding the write lock until the returned transaction is dropped.
	async fn hold_write_lock(conn: &DbConn) -> DatabaseTransaction {
		let txn = conn.begin().await.expect("begin holder");
		txn.execute_unprepared("INSERT INTO counters (id, hits) VALUES ('holder', 1)")
			.await
			.expect("holder write takes the lock");
		txn
	}

	/// The read-then-write body both variants run: read a row, then update it.
	async fn read_then_write(txn: &DatabaseTransaction) -> Result<(), DbErr> {
		let current = hits(txn, "target").await;
		txn.execute(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			"UPDATE counters SET hits = ? WHERE id = ?",
			[(current + 1).into(), "target".into()],
		))
		.await
		.map(|_| ())
	}

	async fn seeded(dir: &std::path::Path) -> DbConn {
		let conn = connect(dir).await;
		conn.execute_unprepared(SCHEMA).await.expect("schema");
		conn.execute_unprepared("INSERT INTO counters (id, hits) VALUES ('target', 41)")
			.await
			.expect("seed");
		conn
	}

	/// The negative case that proves the positive one bites: sea-orm's deferred
	/// `begin()` cannot promote its read snapshot to the write lock while
	/// another connection holds it, and fails immediately rather than waiting
	/// out `busy_timeout`.
	#[tokio::test]
	async fn deferred_begin_fails_to_promote_a_read_to_a_write() {
		let dir = tempfile::tempdir().expect("tempdir");
		let conn = seeded(dir.path()).await;
		let holder = hold_write_lock(&conn).await;

		let txn = conn.begin().await.expect("deferred begin");
		let error = read_then_write(&txn)
			.await
			.expect_err("promotion must fail while the write lock is held");

		assert!(
			is_write_lock_contention(&error),
			"expected write-lock contention, got {error:?}"
		);
		drop(txn);
		holder.rollback().await.expect("release the write lock");
		assert_eq!(hits(&conn, "target").await, 41, "no write got through");
	}

	/// The regression: the same read-then-write body waits for the write lock
	/// and commits when the transaction is opened with [`begin_write`].
	#[tokio::test]
	async fn begin_write_waits_for_the_write_lock_and_commits() {
		let dir = tempfile::tempdir().expect("tempdir");
		let conn = Arc::new(seeded(dir.path()).await);
		let holder = hold_write_lock(&conn).await;

		let writer = {
			let conn = Arc::clone(&conn);
			tokio::spawn(async move {
				let txn = begin_write(&conn).await?;
				read_then_write(&txn).await?;
				txn.commit().await
			})
		};

		// `BEGIN IMMEDIATE` parks on the sqlite worker thread, so the spawned
		// task is genuinely blocked here rather than merely unpolled.
		tokio::time::sleep(Duration::from_millis(250)).await;
		assert!(!writer.is_finished(), "begin_write must wait, not fail");

		holder.commit().await.expect("release the write lock");
		writer.await.expect("join").expect("write transaction");

		assert_eq!(hits(conn.as_ref(), "target").await, 42);
	}

	/// The `ROLLBACK` + `BEGIN IMMEDIATE` swap must leave sqlx's transaction
	/// depth intact, so rolling a `begin_write` transaction back still discards
	/// its writes instead of leaking them or the connection.
	#[tokio::test]
	async fn begin_write_transactions_still_roll_back() {
		let dir = tempfile::tempdir().expect("tempdir");
		let conn = seeded(dir.path()).await;

		let txn = begin_write(&conn).await.expect("begin_write");
		read_then_write(&txn).await.expect("write");
		assert_eq!(hits(&txn, "target").await, 42, "visible inside the txn");
		txn.rollback().await.expect("rollback");

		assert_eq!(hits(&conn, "target").await, 41, "rolled back");

		// The connection came back to the pool at depth 0: a second
		// transaction commits normally.
		let txn = begin_write(&conn).await.expect("begin_write again");
		read_then_write(&txn).await.expect("write again");
		txn.commit().await.expect("commit");
		assert_eq!(hits(&conn, "target").await, 42);
	}

	#[test]
	fn write_lock_contention_is_recognised_from_sea_orm_error_shapes() {
		assert!(is_write_lock_contention(&DbErr::Custom(
			"Execution Error: error returned from database: (code: 5) database is locked"
				.to_owned()
		)));
		assert!(is_write_lock_contention(&DbErr::Exec(
			RuntimeErr::Internal("database table is locked".to_owned())
		)));
		assert!(!is_write_lock_contention(&DbErr::RecordNotFound(
			"nope".to_owned()
		)));
		assert!(!is_write_lock_contention(&DbErr::Custom(
			"UNIQUE constraint failed".to_owned()
		)));
	}
}
