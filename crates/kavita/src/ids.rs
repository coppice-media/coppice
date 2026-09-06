//! Stable integer identities for Kavita clients.
//!
//! Kavita identifies libraries, series, volumes, chapters and users by `int`.
//! Stump uses text uuids, so the `kavita_ids` table (migration
//! `m20260912_000000_add_kavita_compat`) allocates one monotonically
//! increasing id per `(kind, stump_id)` pair; ids are never reused and survive
//! restarts. Every kind draws from the same sequence, so an id names exactly
//! one entity and [`KavitaIds::lookup_any`] can tell which. A media item is
//! both the Kavita volume and its single chapter, so `volumeId == chapterId`
//! for the same file. In Book/LightNovel libraries every media item is also
//! its own Kavita series: that series id is the `book_series` kind keyed by
//! the media id, distinct from the media's volume/chapter id.

use std::collections::HashMap;

use sea_orm::{ConnectionTrait, DbErr, Statement, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdKind {
	Library,
	Series,
	/// A media item of a Book/LightNovel library presented as a Kavita series;
	/// the Stump id is the media id.
	BookSeries,
	Media,
	User,
	/// A Stump reading list presented as a Kavita reading list.
	ReadingList,
}

impl IdKind {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Library => "library",
			Self::Series => "series",
			Self::BookSeries => "book_series",
			Self::Media => "media",
			Self::User => "user",
			Self::ReadingList => "reading_list",
		}
	}

	fn parse(kind: &str) -> Option<Self> {
		match kind {
			"library" => Some(Self::Library),
			"series" => Some(Self::Series),
			"book_series" => Some(Self::BookSeries),
			"media" => Some(Self::Media),
			"user" => Some(Self::User),
			"reading_list" => Some(Self::ReadingList),
			_ => None,
		}
	}
}

/// SQLite's default `SQLITE_MAX_VARIABLE_NUMBER` on older builds is 999.
pub(crate) const LOOKUP_CHUNK: usize = 500;

fn statement(conn: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}

/// Accessors over the `kavita_ids` table.
pub struct KavitaIds;

impl KavitaIds {
	/// Return the Kavita id for a Stump id, allocating one on first use.
	pub async fn resolve(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_id: &str,
	) -> Result<i32, DbErr> {
		if let Some(id) = Self::find(conn, kind, stump_id).await? {
			return Ok(id);
		}
		conn.execute(statement(
			conn,
			"INSERT INTO kavita_ids (kind, stump_id) VALUES ($1, $2)
             ON CONFLICT(kind, stump_id) DO NOTHING",
			vec![kind.as_str().into(), stump_id.into()],
		))
		.await?;
		Self::find(conn, kind, stump_id).await?.ok_or_else(|| {
			DbErr::Custom(format!(
				"kavita_ids row for {}:{stump_id} vanished after insert",
				kind.as_str()
			))
		})
	}

	async fn find(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_id: &str,
	) -> Result<Option<i32>, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT id FROM kavita_ids WHERE kind = $1 AND stump_id = $2",
				vec![kind.as_str().into(), stump_id.into()],
			))
			.await?;
		row.map(|row| row.try_get::<i32>("", "id")).transpose()
	}

	/// Resolve a batch of Stump ids, allocating any that are missing. The
	/// result contains every requested id.
	pub async fn resolve_many(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_ids: &[String],
	) -> Result<HashMap<String, i32>, DbErr> {
		let mut resolved = HashMap::with_capacity(stump_ids.len());
		if stump_ids.is_empty() {
			return Ok(resolved);
		}
		for chunk in stump_ids.chunks(LOOKUP_CHUNK) {
			Self::collect_existing(conn, kind, chunk, &mut resolved).await?;
		}
		let missing = stump_ids
			.iter()
			.filter(|id| !resolved.contains_key(*id))
			.cloned()
			.collect::<Vec<_>>();
		if missing.is_empty() {
			return Ok(resolved);
		}
		for stump_id in &missing {
			conn.execute(statement(
				conn,
				"INSERT INTO kavita_ids (kind, stump_id) VALUES ($1, $2)
                 ON CONFLICT(kind, stump_id) DO NOTHING",
				vec![kind.as_str().into(), stump_id.as_str().into()],
			))
			.await?;
		}
		for chunk in missing.chunks(LOOKUP_CHUNK) {
			Self::collect_existing(conn, kind, chunk, &mut resolved).await?;
		}
		Ok(resolved)
	}

	async fn collect_existing(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_ids: &[String],
		resolved: &mut HashMap<String, i32>,
	) -> Result<(), DbErr> {
		let placeholders = (0..stump_ids.len())
			.map(|index| format!("${}", index + 2))
			.collect::<Vec<_>>()
			.join(", ");
		let mut values: Vec<Value> = Vec::with_capacity(stump_ids.len() + 1);
		values.push(kind.as_str().into());
		values.extend(stump_ids.iter().map(|id| Value::from(id.as_str())));
		let rows = conn
			.query_all(statement(
				conn,
				&format!(
					"SELECT id, stump_id FROM kavita_ids WHERE kind = $1 AND stump_id IN ({placeholders})"
				),
				values,
			))
			.await?;
		for row in rows {
			let id = row.try_get::<i32>("", "id")?;
			let stump_id = row.try_get::<String>("", "stump_id")?;
			resolved.insert(stump_id, id);
		}
		Ok(())
	}

	/// Return the Stump id behind a Kavita id, if one was ever allocated.
	pub async fn lookup(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		id: i32,
	) -> Result<Option<String>, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT stump_id FROM kavita_ids WHERE kind = $1 AND id = $2",
				vec![kind.as_str().into(), id.into()],
			))
			.await?;
		row.map(|row| row.try_get::<String>("", "stump_id"))
			.transpose()
	}

	/// Return the kind and Stump id behind a Kavita id, whatever entity it
	/// names. Callers that accept several kinds for one parameter (a
	/// `seriesId` is a `series` or a `book_series`) resolve with one query.
	pub async fn lookup_any(
		conn: &impl ConnectionTrait,
		id: i32,
	) -> Result<Option<(IdKind, String)>, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT kind, stump_id FROM kavita_ids WHERE id = $1",
				vec![id.into()],
			))
			.await?;
		row.map(|row| {
			let kind = row.try_get::<String>("", "kind")?;
			let stump_id = row.try_get::<String>("", "stump_id")?;
			IdKind::parse(&kind)
				.map(|kind| (kind, stump_id))
				.ok_or_else(|| DbErr::Custom(format!("unknown kavita_ids kind {kind}")))
		})
		.transpose()
	}
}

/// `CREATE TABLE` used by the migration and by in-memory tests.
pub const CREATE_KAVITA_IDS_SQL: &str = "CREATE TABLE IF NOT EXISTS kavita_ids (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    stump_id TEXT NOT NULL,
    UNIQUE (kind, stump_id)
)";

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{Database, DatabaseBackend};

	async fn db() -> sea_orm::DatabaseConnection {
		let conn = Database::connect("sqlite::memory:").await.unwrap();
		conn.execute(Statement::from_string(
			DatabaseBackend::Sqlite,
			CREATE_KAVITA_IDS_SQL,
		))
		.await
		.unwrap();
		conn
	}

	#[tokio::test]
	async fn ids_are_stable_and_monotonic() {
		let conn = db().await;
		let first = KavitaIds::resolve(&conn, IdKind::Series, "a")
			.await
			.unwrap();
		let second = KavitaIds::resolve(&conn, IdKind::Series, "b")
			.await
			.unwrap();
		let again = KavitaIds::resolve(&conn, IdKind::Series, "a")
			.await
			.unwrap();
		assert_eq!(first, 1);
		assert_eq!(second, 2);
		assert_eq!(again, first);
		// Kinds share one sequence, so a media id never collides with a series id.
		let media = KavitaIds::resolve(&conn, IdKind::Media, "a").await.unwrap();
		assert_eq!(media, 3);
		assert_eq!(
			KavitaIds::lookup(&conn, IdKind::Series, first)
				.await
				.unwrap(),
			Some("a".to_owned())
		);
		assert_eq!(
			KavitaIds::lookup(&conn, IdKind::Media, first)
				.await
				.unwrap(),
			None
		);
	}

	#[tokio::test]
	async fn book_series_ids_are_distinct_from_the_media_id_and_reverse_resolve() {
		let conn = db().await;
		let series = KavitaIds::resolve(&conn, IdKind::Series, "s1")
			.await
			.unwrap();
		let media = KavitaIds::resolve(&conn, IdKind::Media, "m1")
			.await
			.unwrap();
		let book = KavitaIds::resolve(&conn, IdKind::BookSeries, "m1")
			.await
			.unwrap();
		assert_ne!(
			book, media,
			"a book's series id is not its volume/chapter id"
		);
		assert_ne!(book, series);
		assert_eq!(
			KavitaIds::resolve(&conn, IdKind::BookSeries, "m1")
				.await
				.unwrap(),
			book
		);
		// One query tells the kind behind any id, so a `seriesId` parameter
		// resolves to a series or a book without probing each kind.
		assert_eq!(
			KavitaIds::lookup_any(&conn, book).await.unwrap(),
			Some((IdKind::BookSeries, "m1".to_owned()))
		);
		assert_eq!(
			KavitaIds::lookup_any(&conn, series).await.unwrap(),
			Some((IdKind::Series, "s1".to_owned()))
		);
		assert_eq!(
			KavitaIds::lookup_any(&conn, media).await.unwrap(),
			Some((IdKind::Media, "m1".to_owned()))
		);
		assert_eq!(KavitaIds::lookup_any(&conn, 99).await.unwrap(), None);
	}

	#[tokio::test]
	async fn resolve_many_allocates_missing_and_keeps_existing() {
		let conn = db().await;
		let existing = KavitaIds::resolve(&conn, IdKind::Media, "m1")
			.await
			.unwrap();
		let ids = ["m1", "m2", "m3"]
			.into_iter()
			.map(str::to_owned)
			.collect::<Vec<_>>();
		let resolved = KavitaIds::resolve_many(&conn, IdKind::Media, &ids)
			.await
			.unwrap();
		assert_eq!(resolved.len(), 3);
		assert_eq!(resolved["m1"], existing);
		assert!(resolved["m2"] > existing && resolved["m3"] > resolved["m2"]);
		let again = KavitaIds::resolve_many(&conn, IdKind::Media, &ids)
			.await
			.unwrap();
		assert_eq!(again, resolved);
		assert!(KavitaIds::resolve_many(&conn, IdKind::Media, &[])
			.await
			.unwrap()
			.is_empty());
	}
}
