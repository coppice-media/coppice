//! Feed positions of the Liseur CAS annotation records.
//!
//! `liseur_sync_annotations.seq` is the per-account clock every consumer of
//! the annotation lane reads by: the `/v1/annotations/changes` cursor of a
//! device, `projected_annotation_seq` of the native projection writer, and
//! the `liseur_from_seq` cursor of the annotation-sync exporter. A record
//! only moves in that clock when its content changes, so a change to *where*
//! a work's records belong — a media link that is new, or that now names the
//! edition a record was made on — was invisible to all three: the projection
//! stayed on the old book, the export never re-read the work, and devices
//! never re-resolved it.
//!
//! [`resequence_work_annotations`] is how a link writer reports that. It gives
//! every live record of the work a fresh position above the account's high
//! water and changes nothing else: `rev`, the timestamps, the writer and the
//! payload stay, so a device replays an unchanged record (its CAS state is the
//! same `rev` with the same signature, which every pinned client treats as a
//! no-op) rather than a phantom edit.

use sea_orm::{ConnectionTrait, DbErr, Statement};

/// Move every live CAS record of `work_id` above the account's annotation
/// high water, in their existing feed order, and return how many moved.
///
/// Call this in the transaction that inserted or re-homed a media link of the
/// work, and only then: a no-op link upsert must not replay the work.
pub async fn resequence_work_annotations<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	work_id: &str,
) -> Result<u64, DbErr> {
	let backend = conn.get_database_backend();
	let rows = conn
		.query_all(Statement::from_sql_and_values(
			backend,
			"SELECT row_id FROM liseur_sync_annotations
             WHERE user_id = $1 AND work_id = $2 AND deleted = FALSE
             ORDER BY seq ASC, row_id ASC",
			[user_id.to_owned().into(), work_id.to_owned().into()],
		))
		.await?;
	if rows.is_empty() {
		return Ok(0);
	}
	let moved = rows.len() as i64;
	conn.execute(Statement::from_sql_and_values(
		backend,
		"INSERT INTO liseur_sync_counters (user_id, op_seq, annotation_seq)
         VALUES ($1, 0, 0) ON CONFLICT(user_id) DO NOTHING",
		[user_id.to_owned().into()],
	))
	.await?;
	conn.execute(Statement::from_sql_and_values(
		backend,
		"UPDATE liseur_sync_counters SET annotation_seq = annotation_seq + $1
         WHERE user_id = $2",
		[moved.into(), user_id.to_owned().into()],
	))
	.await?;
	let high_water: i64 = conn
		.query_one(Statement::from_sql_and_values(
			backend,
			"SELECT annotation_seq FROM liseur_sync_counters WHERE user_id = $1",
			[user_id.to_owned().into()],
		))
		.await?
		.ok_or_else(|| DbErr::Custom("liseur-sync counter disappeared".into()))?
		.try_get("", "annotation_seq")?;
	let first = high_water - moved + 1;
	for (offset, row) in rows.iter().enumerate() {
		let row_id: i64 = row.try_get("", "row_id")?;
		conn.execute(Statement::from_sql_and_values(
			backend,
			"UPDATE liseur_sync_annotations SET seq = $1 WHERE row_id = $2",
			[(first + offset as i64).into(), row_id.into()],
		))
		.await?;
	}
	Ok(moved as u64)
}

#[cfg(test)]
mod tests {
	use sea_orm::{Database, DatabaseConnection, DbBackend};

	use super::*;

	async fn database() -> DatabaseConnection {
		let conn = Database::connect("sqlite::memory:").await.unwrap();
		for sql in [
			"CREATE TABLE liseur_sync_counters (
                user_id TEXT PRIMARY KEY, op_seq BIGINT NOT NULL DEFAULT 0,
                annotation_seq BIGINT NOT NULL DEFAULT 0,
                projected_annotation_seq BIGINT NOT NULL DEFAULT 0
            )",
			"CREATE TABLE liseur_sync_annotations (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT NOT NULL,
                annotation_id TEXT NOT NULL, rev BIGINT NOT NULL, seq BIGINT NOT NULL,
                work_id TEXT NOT NULL, deleted BOOLEAN NOT NULL DEFAULT FALSE,
                updated_at TEXT NOT NULL, device_id TEXT NOT NULL, payload TEXT NOT NULL
            )",
		] {
			conn.execute(Statement::from_string(DbBackend::Sqlite, sql))
				.await
				.unwrap();
		}
		conn
	}

	async fn insert(
		conn: &DatabaseConnection,
		id: &str,
		rev: i64,
		seq: i64,
		work: &str,
		deleted: bool,
	) {
		conn.execute(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			"INSERT INTO liseur_sync_annotations
             (user_id, annotation_id, rev, seq, work_id, deleted, updated_at, device_id, payload)
             VALUES ('u1', $1, $2, $3, $4, $5, '2026-09-11T12:00:00Z', 'kobo', '{}')",
			[id.into(), rev.into(), seq.into(), work.into(), deleted.into()],
		))
		.await
		.unwrap();
	}

	async fn rows(conn: &DatabaseConnection) -> Vec<(String, i64, i64, String)> {
		conn.query_all(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT annotation_id, rev, seq, updated_at FROM liseur_sync_annotations
             ORDER BY annotation_id",
		))
		.await
		.unwrap()
		.iter()
		.map(|row| {
			(
				row.try_get("", "annotation_id").unwrap(),
				row.try_get("", "rev").unwrap(),
				row.try_get("", "seq").unwrap(),
				row.try_get("", "updated_at").unwrap(),
			)
		})
		.collect()
	}

	#[tokio::test]
	async fn live_records_of_the_work_move_above_the_high_water_in_feed_order() {
		let conn = database().await;
		conn.execute(Statement::from_string(
			DbBackend::Sqlite,
			"INSERT INTO liseur_sync_counters (user_id, annotation_seq) VALUES ('u1', 9)",
		))
		.await
		.unwrap();
		insert(&conn, "late", 3, 7, "work-a", false).await;
		insert(&conn, "early", 2, 4, "work-a", false).await;
		insert(&conn, "gone", 5, 8, "work-a", true).await;
		insert(&conn, "other", 1, 9, "work-b", false).await;

		assert_eq!(
			resequence_work_annotations(&conn, "u1", "work-a")
				.await
				.unwrap(),
			2
		);

		// Order among the moved records is kept, tombstones and other works
		// stay where they were, and only `seq` changes.
		assert_eq!(
			rows(&conn).await,
			[
				("early".into(), 2, 10, "2026-09-11T12:00:00Z".into()),
				("gone".into(), 5, 8, "2026-09-11T12:00:00Z".into()),
				("late".into(), 3, 11, "2026-09-11T12:00:00Z".into()),
				("other".into(), 1, 9, "2026-09-11T12:00:00Z".into()),
			]
		);
		let high_water: i64 = conn
			.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT annotation_seq FROM liseur_sync_counters WHERE user_id = 'u1'",
			))
			.await
			.unwrap()
			.unwrap()
			.try_get("", "annotation_seq")
			.unwrap();
		assert_eq!(high_water, 11);
	}

	#[tokio::test]
	async fn a_work_without_live_records_leaves_the_counter_alone() {
		let conn = database().await;
		insert(&conn, "gone", 1, 1, "work-a", true).await;
		assert_eq!(
			resequence_work_annotations(&conn, "u1", "work-a")
				.await
				.unwrap(),
			0
		);
		assert!(conn
			.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT annotation_seq FROM liseur_sync_counters WHERE user_id = 'u1'",
			))
			.await
			.unwrap()
			.is_none());
	}
}
