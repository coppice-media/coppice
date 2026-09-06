//! The `abs_sessions` table: playback sessions, which Audiobookshelf keeps
//! server-side.
//!
//! A client syncs against the *session* id rather than the book
//! (`POST /api/session/{id}/sync`), so the row has to outlive the play
//! request that created it. It is deliberately not a Stump reading session:
//! `time_listening_ms` is the wall-clock time this one client reported, while
//! the position itself lands on the unified reading state through
//! [`AbsBackend::apply_position`](crate::routes::AbsBackend::apply_position).
//! Closing a session keeps the row (a client may `GET` it afterwards) and
//! stamps `closed_at`.

use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ConnectionTrait, DbErr, Statement, Value};

use crate::routes::AbsSession;

/// `CREATE TABLE` used by the migration and by in-memory tests.
pub const CREATE_ABS_SESSIONS_SQL: &str = "CREATE TABLE IF NOT EXISTS abs_sessions (
    id TEXT NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    media_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    device_id TEXT,
    client_name TEXT,
    client_version TEXT,
    media_player TEXT,
    current_time_ms BIGINT NOT NULL DEFAULT 0,
    time_listening_ms BIGINT NOT NULL DEFAULT 0,
    started_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    closed_at BIGINT
)";

fn statement(conn: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}

fn to_millis(at: DateTime<Utc>) -> i64 {
	at.timestamp_millis()
}

fn from_millis(millis: i64) -> DateTime<Utc> {
	Utc.timestamp_millis_opt(millis)
		.single()
		.unwrap_or_else(Utc::now)
}

/// Accessors over the `abs_sessions` table.
pub struct AbsSessions;

impl AbsSessions {
	pub async fn insert(
		conn: &impl ConnectionTrait,
		session: &AbsSession,
	) -> Result<(), DbErr> {
		conn.execute(statement(
			conn,
			"INSERT INTO abs_sessions (
                id, user_id, media_id, library_id, device_id, client_name,
                client_version, media_player, current_time_ms,
                time_listening_ms, started_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
			vec![
				session.id.as_str().into(),
				session.user_id.as_str().into(),
				session.media_id.as_str().into(),
				session.library_id.as_str().into(),
				session.device_id.clone().into(),
				session.client_name.clone().into(),
				session.client_version.clone().into(),
				session.media_player.clone().into(),
				session.current_time_ms.into(),
				session.time_listening_ms.into(),
				to_millis(session.started_at).into(),
				to_millis(session.updated_at).into(),
			],
		))
		.await?;
		Ok(())
	}

	/// One session of `user_id`. Scoping the read by user is what stops a
	/// client syncing progress onto somebody else's book with a guessed id.
	pub async fn get(
		conn: &impl ConnectionTrait,
		user_id: &str,
		session_id: &str,
	) -> Result<Option<AbsSession>, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT id, user_id, media_id, library_id, device_id, client_name,
                        client_version, media_player, current_time_ms,
                        time_listening_ms, started_at, updated_at
                 FROM abs_sessions WHERE id = $1 AND user_id = $2",
				vec![session_id.into(), user_id.into()],
			))
			.await?;
		let Some(row) = row else {
			return Ok(None);
		};
		Ok(Some(AbsSession {
			id: row.try_get("", "id")?,
			user_id: row.try_get("", "user_id")?,
			media_id: row.try_get("", "media_id")?,
			library_id: row.try_get("", "library_id")?,
			device_id: row.try_get("", "device_id")?,
			client_name: row.try_get("", "client_name")?,
			client_version: row.try_get("", "client_version")?,
			media_player: row.try_get("", "media_player")?,
			current_time_ms: row.try_get("", "current_time_ms")?,
			time_listening_ms: row.try_get("", "time_listening_ms")?,
			started_at: from_millis(row.try_get("", "started_at")?),
			updated_at: from_millis(row.try_get("", "updated_at")?),
		}))
	}

	pub async fn update(
		conn: &impl ConnectionTrait,
		session_id: &str,
		current_time_ms: i64,
		time_listening_ms: i64,
	) -> Result<(), DbErr> {
		conn.execute(statement(
			conn,
			"UPDATE abs_sessions
             SET current_time_ms = $1, time_listening_ms = $2, updated_at = $3
             WHERE id = $4",
			vec![
				current_time_ms.into(),
				time_listening_ms.into(),
				to_millis(Utc::now()).into(),
				session_id.into(),
			],
		))
		.await?;
		Ok(())
	}

	pub async fn close(
		conn: &impl ConnectionTrait,
		session_id: &str,
	) -> Result<(), DbErr> {
		conn.execute(statement(
			conn,
			"UPDATE abs_sessions SET closed_at = $1 WHERE id = $2",
			vec![to_millis(Utc::now()).into(), session_id.into()],
		))
		.await?;
		Ok(())
	}

	/// Whether a session has been closed, for the tests and for a client that
	/// re-syncs a session it already closed.
	pub async fn is_closed(
		conn: &impl ConnectionTrait,
		session_id: &str,
	) -> Result<bool, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT closed_at FROM abs_sessions WHERE id = $1",
				vec![session_id.into()],
			))
			.await?;
		Ok(row
			.map(|row| row.try_get::<Option<i64>>("", "closed_at"))
			.transpose()?
			.flatten()
			.is_some())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::DatabaseBackend;

	async fn db() -> sea_orm::DatabaseConnection {
		let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
		conn.execute(Statement::from_string(
			DatabaseBackend::Sqlite,
			CREATE_ABS_SESSIONS_SQL,
		))
		.await
		.unwrap();
		conn
	}

	fn session(id: &str, user: &str) -> AbsSession {
		AbsSession {
			id: id.to_owned(),
			user_id: user.to_owned(),
			media_id: "media".to_owned(),
			library_id: "library".to_owned(),
			device_id: Some("device".to_owned()),
			client_name: Some("Lissen".to_owned()),
			client_version: None,
			media_player: Some("exo-player".to_owned()),
			current_time_ms: 4_500,
			time_listening_ms: 0,
			started_at: Utc::now(),
			updated_at: Utc::now(),
		}
	}

	#[tokio::test]
	async fn a_session_round_trips_with_its_nullable_columns() {
		let conn = db().await;
		let stored = session("session-1", "user-1");
		AbsSessions::insert(&conn, &stored).await.unwrap();

		let loaded = AbsSessions::get(&conn, "user-1", "session-1")
			.await
			.unwrap()
			.expect("session");
		assert_eq!(loaded.media_id, "media");
		assert_eq!(loaded.client_name.as_deref(), Some("Lissen"));
		assert_eq!(loaded.client_version, None);
		assert_eq!(loaded.current_time_ms, 4_500);
		// Millisecond timestamps survive the round trip.
		assert_eq!(
			loaded.started_at.timestamp_millis(),
			stored.started_at.timestamp_millis()
		);
	}

	#[tokio::test]
	async fn a_session_is_invisible_to_another_user() {
		let conn = db().await;
		AbsSessions::insert(&conn, &session("session-1", "user-1"))
			.await
			.unwrap();
		assert!(AbsSessions::get(&conn, "user-2", "session-1")
			.await
			.unwrap()
			.is_none());
	}

	#[tokio::test]
	async fn sync_accumulates_listening_time_and_close_stamps_the_row() {
		let conn = db().await;
		AbsSessions::insert(&conn, &session("session-1", "user-1"))
			.await
			.unwrap();

		AbsSessions::update(&conn, "session-1", 9_000, 4_500)
			.await
			.unwrap();
		let loaded = AbsSessions::get(&conn, "user-1", "session-1")
			.await
			.unwrap()
			.expect("session");
		assert_eq!(loaded.current_time_ms, 9_000);
		assert_eq!(loaded.time_listening_ms, 4_500);

		assert!(!AbsSessions::is_closed(&conn, "session-1").await.unwrap());
		AbsSessions::close(&conn, "session-1").await.unwrap();
		assert!(AbsSessions::is_closed(&conn, "session-1").await.unwrap());
		// A closed session is still readable: a client may GET it after the
		// close, and abs-ref keeps the row too.
		assert!(AbsSessions::get(&conn, "user-1", "session-1")
			.await
			.unwrap()
			.is_some());
	}
}
