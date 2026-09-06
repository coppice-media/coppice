//! Stable Audiobookshelf identities for things Stump does not identify.
//!
//! Audiobookshelf identifies everything by uuid, and so does Stump, so
//! libraries, library items (books) and series are exposed under their Stump
//! uuid verbatim — no mapping, no lookup. Three ABS ids have no Stump uuid to
//! borrow and are allocated here, in the `abs_ids` table (migration
//! `m20260935_000000_add_abs_compat`):
//!
//! | kind | Stump key | why |
//! | --- | --- | --- |
//! | [`IdKind::Author`] | the author's name | Stump has no author row; authors are `media_metadata.writers` strings |
//! | [`IdKind::Book`] | the media id | ABS's `media.id` is a second uuid beside `libraryItem.id` (`item.json:2,21`) |
//! | [`IdKind::Folder`] | the library id | ABS's `libraryFolder.id` is a third uuid beside the library's (`library_include_filterdata.json:35,39`) |
//!
//! Ids are allocated on first use, never reused, and survive restarts. The
//! table is keyed by the ABS uuid so `GET /api/authors/{id}` can resolve a
//! uuid back to the author name in one indexed lookup.

use std::collections::HashMap;

use sea_orm::{ConnectionTrait, DbErr, Statement, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdKind {
	/// An author name presented as an ABS author.
	Author,
	/// A Stump media item presented as an ABS `media` (book) object; distinct
	/// from the library item id, which is the media id itself.
	Book,
	/// A Stump library presented as an ABS library folder.
	Folder,
}

impl IdKind {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Author => "author",
			Self::Book => "book",
			Self::Folder => "folder",
		}
	}

	pub fn parse(kind: &str) -> Option<Self> {
		match kind {
			"author" => Some(Self::Author),
			"book" => Some(Self::Book),
			"folder" => Some(Self::Folder),
			_ => None,
		}
	}
}

/// SQLite's default `SQLITE_MAX_VARIABLE_NUMBER` on older builds is 999.
pub(crate) const LOOKUP_CHUNK: usize = 500;

fn statement(conn: &impl ConnectionTrait, sql: &str, values: Vec<Value>) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}

/// Accessors over the `abs_ids` table.
pub struct AbsIds;

impl AbsIds {
	/// The ABS uuid for a Stump key, allocating one on first use.
	pub async fn resolve(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_id: &str,
	) -> Result<String, DbErr> {
		if let Some(id) = Self::find(conn, kind, stump_id).await? {
			return Ok(id);
		}
		Self::insert(conn, kind, stump_id).await?;
		Self::find(conn, kind, stump_id).await?.ok_or_else(|| {
			DbErr::Custom(format!(
				"abs_ids row for {}:{stump_id} vanished after insert",
				kind.as_str()
			))
		})
	}

	/// Resolve a batch of Stump keys, allocating any that are missing. The
	/// result contains every requested key.
	pub async fn resolve_many(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_ids: &[String],
	) -> Result<HashMap<String, String>, DbErr> {
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
			Self::insert(conn, kind, stump_id).await?;
		}
		for chunk in missing.chunks(LOOKUP_CHUNK) {
			Self::collect_existing(conn, kind, chunk, &mut resolved).await?;
		}
		Ok(resolved)
	}

	/// The Stump key an ABS uuid was allocated for, if it names one of
	/// `kind`.
	pub async fn lookup(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		abs_id: &str,
	) -> Result<Option<String>, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT stump_id FROM abs_ids WHERE id = $1 AND kind = $2",
				vec![abs_id.into(), kind.as_str().into()],
			))
			.await?;
		row.map(|row| row.try_get::<String>("", "stump_id"))
			.transpose()
	}

	async fn insert(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_id: &str,
	) -> Result<(), DbErr> {
		conn.execute(statement(
			conn,
			"INSERT INTO abs_ids (id, kind, stump_id) VALUES ($1, $2, $3)
             ON CONFLICT(kind, stump_id) DO NOTHING",
			vec![
				uuid::Uuid::new_v4().to_string().into(),
				kind.as_str().into(),
				stump_id.into(),
			],
		))
		.await?;
		Ok(())
	}

	async fn find(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_id: &str,
	) -> Result<Option<String>, DbErr> {
		let row = conn
			.query_one(statement(
				conn,
				"SELECT id FROM abs_ids WHERE kind = $1 AND stump_id = $2",
				vec![kind.as_str().into(), stump_id.into()],
			))
			.await?;
		row.map(|row| row.try_get::<String>("", "id")).transpose()
	}

	async fn collect_existing(
		conn: &impl ConnectionTrait,
		kind: IdKind,
		stump_ids: &[String],
		resolved: &mut HashMap<String, String>,
	) -> Result<(), DbErr> {
		let placeholders = (0..stump_ids.len())
			.map(|index| format!("${}", index + 2))
			.collect::<Vec<_>>()
			.join(", ");
		let mut values = Vec::with_capacity(stump_ids.len() + 1);
		values.push(Value::from(kind.as_str()));
		values.extend(stump_ids.iter().map(|id| Value::from(id.as_str())));
		let rows = conn
			.query_all(statement(
				conn,
				&format!(
					"SELECT id, stump_id FROM abs_ids
                     WHERE kind = $1 AND stump_id IN ({placeholders})"
				),
				values,
			))
			.await?;
		for row in rows {
			resolved.insert(
				row.try_get::<String>("", "stump_id")?,
				row.try_get::<String>("", "id")?,
			);
		}
		Ok(())
	}
}

/// `CREATE TABLE` used by the migration and by in-memory tests.
pub const CREATE_ABS_IDS_SQL: &str = "CREATE TABLE IF NOT EXISTS abs_ids (
    id TEXT NOT NULL PRIMARY KEY,
    kind TEXT NOT NULL,
    stump_id TEXT NOT NULL,
    UNIQUE (kind, stump_id)
)";

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{DatabaseBackend, DatabaseConnection};

	async fn db() -> DatabaseConnection {
		let conn = sea_orm::Database::connect("sqlite::memory:")
			.await
			.expect("in-memory sqlite");
		conn.execute(Statement::from_string(
			DatabaseBackend::Sqlite,
			CREATE_ABS_IDS_SQL,
		))
		.await
		.expect("abs_ids");
		conn
	}

	#[tokio::test]
	async fn ids_are_stable_per_kind_and_key() {
		let conn = db().await;
		let first = AbsIds::resolve(&conn, IdKind::Author, "Ada Lovelace")
			.await
			.expect("allocate");
		let again = AbsIds::resolve(&conn, IdKind::Author, "Ada Lovelace")
			.await
			.expect("reuse");
		assert_eq!(first, again, "the same key keeps its id");
		assert_eq!(
			uuid::Uuid::parse_str(&first).map(|id| id.get_version_num()),
			Ok(4),
			"ABS ids are uuids"
		);

		// The same string under another kind is a different entity.
		let book = AbsIds::resolve(&conn, IdKind::Book, "Ada Lovelace")
			.await
			.expect("allocate");
		assert_ne!(book, first);

		assert_eq!(
			AbsIds::lookup(&conn, IdKind::Author, &first)
				.await
				.expect("lookup"),
			Some("Ada Lovelace".to_owned())
		);
		assert_eq!(
			AbsIds::lookup(&conn, IdKind::Book, &first)
				.await
				.expect("lookup"),
			None,
			"an author id does not name a book"
		);
		assert_eq!(
			AbsIds::lookup(&conn, IdKind::Author, "not-an-id")
				.await
				.expect("lookup"),
			None
		);
	}

	#[tokio::test]
	async fn batches_allocate_only_what_is_missing() {
		let conn = db().await;
		let ada = AbsIds::resolve(&conn, IdKind::Author, "Ada Lovelace")
			.await
			.expect("allocate");
		let keys = vec![
			"Ada Lovelace".to_owned(),
			"Grace Hopper".to_owned(),
			"Various Narrators".to_owned(),
		];
		let resolved = AbsIds::resolve_many(&conn, IdKind::Author, &keys)
			.await
			.expect("resolve many");
		assert_eq!(resolved.len(), 3);
		assert_eq!(resolved.get("Ada Lovelace"), Some(&ada));
		let ids = resolved.values().collect::<std::collections::HashSet<_>>();
		assert_eq!(ids.len(), 3, "every key gets its own id");

		assert!(AbsIds::resolve_many(&conn, IdKind::Author, &[])
			.await
			.expect("empty")
			.is_empty());
	}

	#[test]
	fn kinds_round_trip_through_their_stored_names() {
		for kind in [IdKind::Author, IdKind::Book, IdKind::Folder] {
			assert_eq!(IdKind::parse(kind.as_str()), Some(kind));
		}
		assert_eq!(IdKind::parse("series"), None);
	}
}
