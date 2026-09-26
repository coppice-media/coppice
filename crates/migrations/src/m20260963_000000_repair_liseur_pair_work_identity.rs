//! Merge a pairing-evidenced, alias-less work into the unique alias-backed
//! Liseur identity recognized by a strong alias on the linked media.
//!
//! Source ids, KoReader partial hashes, mapped SHA-256 editions, and
//! server-recorded full-file hashes count as identity evidence; weak
//! title/author aliases do not. Raw CAS ids, revisions and sequences survive.

use std::collections::{HashMap, HashSet};

use sea_orm::{
	prelude::DateTimeWithTimeZone, ConnectionTrait, DbBackend, Statement,
	TransactionTrait, Value as DbValue,
};
use sea_orm_migration::prelude::*;
use serde_json::Value as JsonValue;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CANDIDATES: &str = r#"
SELECT DISTINCT
    link.user_id,
    link.work_id AS losing_work_id,
    alias.work_id AS identity_work_id
FROM liseur_sync_media_links AS link
JOIN media
  ON media.id = link.media_id
JOIN liseur_sync_aliases AS alias
  ON alias.user_id = link.user_id
 AND (
      (alias.kind = 'source'
       AND alias.value IN (media.id, 'komga:' || media.id))
   OR (alias.kind = 'partial-md5'
       AND media.koreader_hash IS NOT NULL
       AND alias.value = media.koreader_hash)
   OR (alias.kind = 'sha256'
       AND EXISTS (
           SELECT 1
           FROM liseur_sync_editions AS edition
           WHERE edition.user_id = alias.user_id
             AND edition.work_id = alias.work_id
             AND edition.media_id = media.id
             AND edition.edition_sha = alias.value
       ))
   OR (alias.kind = 'sha256'
       AND EXISTS (
           SELECT 1
           FROM media_locations AS location
           WHERE location.media_id = media.id
             AND location.sha256 = alias.value
       ))
 )
WHERE alias.work_id <> link.work_id
  AND link.pair_evidence IS NOT NULL
  AND COALESCE(link.pair_status, 'suggested') IN ('suggested', 'confirmed')
  AND NOT EXISTS (
      SELECT 1
      FROM liseur_sync_aliases AS losing_alias
      WHERE losing_alias.user_id = link.user_id
        AND losing_alias.work_id = link.work_id
  )
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		repair_alias_splits(manager.get_connection()).await
	}

	async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
		// Identity merges are data repairs and cannot be reversed without
		// reconstructing the original work ownership of each moved CAS row.
		Ok(())
	}
}

pub(crate) async fn repair_alias_splits<C: ConnectionTrait + TransactionTrait>(
	conn: &C,
) -> Result<(), DbErr> {
	let backend = conn.get_database_backend();
	let candidates = conn
		.query_all(Statement::from_string(backend, CANDIDATES.to_owned()))
		.await?;
	let mut identities: HashMap<(String, String), HashSet<String>> = HashMap::new();
	for row in candidates {
		let user_id = row.try_get("", "user_id")?;
		let losing_work_id = row.try_get("", "losing_work_id")?;
		let identity_work_id = row.try_get("", "identity_work_id")?;
		identities
			.entry((user_id, losing_work_id))
			.or_default()
			.insert(identity_work_id);
	}
	let merges = identities
		.into_iter()
		.filter_map(|((user_id, losing_work_id), identity_work_ids)| {
			let mut identity_work_ids = identity_work_ids.into_iter();
			let identity_work_id = identity_work_ids.next()?;
			identity_work_ids.next().is_none().then_some((
				user_id,
				losing_work_id,
				identity_work_id,
			))
		})
		.collect::<Vec<_>>();
	if merges.is_empty() {
		return Ok(());
	}

	let tx = conn.begin().await?;
	for (user_id, losing_work_id, identity_work_id) in merges {
		merge_liseur_work(&tx, &user_id, &losing_work_id, &identity_work_id).await?;
	}
	tx.commit().await
}

/// Merge one alias-less pairing work into its unique alias-backed identity.
///
/// The caller owns the transaction so runtime resolution and migrations share
/// the same reference-preserving merge operation.
pub async fn merge_liseur_work<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	losing_work_id: &str,
	identity_work_id: &str,
) -> Result<(), DbErr> {
	let backend = conn.get_database_backend();
	merge_work_metadata(conn, backend, losing_work_id, identity_work_id).await?;
	merge_work_reviews(conn, backend, user_id, losing_work_id, identity_work_id).await?;
	// Move every durable reference before deleting the alias-less work:
	// Liseur edition and CAS rows stay intact, while pair-link ids and
	// verdicts remain unchanged. Native projections are media-keyed.
	for table in [
		"liseur_sync_media_links",
		"liseur_sync_editions",
		"liseur_sync_aliases",
		"liseur_sync_ops",
		"liseur_sync_sessions",
		"liseur_sync_annotations",
	] {
		move_work_reference(
			conn,
			backend,
			table,
			user_id,
			losing_work_id,
			identity_work_id,
		)
		.await?;
	}

	conn.execute(Statement::from_sql_and_values(
		backend,
		"DELETE FROM liseur_sync_works WHERE user_id = $1 AND id = $2",
		[user_id.to_owned().into(), losing_work_id.to_owned().into()],
	))
	.await?;
	Ok(())
}

async fn move_work_reference<C: ConnectionTrait>(
	conn: &C,
	backend: DbBackend,
	table: &str,
	user_id: &str,
	losing_work_id: &str,
	identity_work_id: &str,
) -> Result<(), DbErr> {
	let sql =
		format!("UPDATE {table} SET work_id = $1 WHERE user_id = $2 AND work_id = $3");
	conn.execute(Statement::from_sql_and_values(
		backend,
		&sql,
		[
			identity_work_id.to_owned().into(),
			user_id.to_owned().into(),
			losing_work_id.to_owned().into(),
		],
	))
	.await?;
	Ok(())
}

#[derive(Debug)]
struct WorkMetadata {
	title: Option<String>,
	author: Option<String>,
	metadata: Option<JsonValue>,
	locked_fields: Option<JsonValue>,
	created_at: DateTimeWithTimeZone,
	updated_at: DateTimeWithTimeZone,
}

async fn merge_work_metadata<C: ConnectionTrait>(
	conn: &C,
	backend: DbBackend,
	losing_work_id: &str,
	identity_work_id: &str,
) -> Result<(), DbErr> {
	let rows = conn
		.query_all(Statement::from_sql_and_values(
			backend,
			"SELECT work_id, title, author, metadata, locked_fields, created_at, updated_at \
			 FROM book_work_metadata WHERE work_id IN ($1, $2)",
			[
				losing_work_id.to_owned().into(),
				identity_work_id.to_owned().into(),
			],
		))
		.await?;
	let mut losing = None;
	let mut identity = None;
	for row in rows {
		let work_id: String = row.try_get("", "work_id")?;
		let metadata = WorkMetadata {
			title: row.try_get("", "title")?,
			author: row.try_get("", "author")?,
			metadata: row.try_get("", "metadata")?,
			locked_fields: row.try_get("", "locked_fields")?,
			created_at: row.try_get("", "created_at")?,
			updated_at: row.try_get("", "updated_at")?,
		};
		if work_id == losing_work_id {
			losing = Some(metadata);
		} else if work_id == identity_work_id {
			identity = Some(metadata);
		}
	}
	let Some(losing) = losing else {
		return Ok(());
	};
	let Some(identity) = identity else {
		conn.execute(Statement::from_sql_and_values(
			backend,
			"UPDATE book_work_metadata SET work_id = $1 WHERE work_id = $2",
			[
				identity_work_id.to_owned().into(),
				losing_work_id.to_owned().into(),
			],
		))
		.await?;
		return Ok(());
	};

	let metadata = merge_json(identity.metadata, losing.metadata, false);
	let locked_fields = merge_json(identity.locked_fields, losing.locked_fields, true);
	let created_at = identity.created_at.min(losing.created_at);
	let updated_at = identity.updated_at.max(losing.updated_at);
	conn.execute(Statement::from_sql_and_values(
		backend,
		"UPDATE book_work_metadata SET title = $1, author = $2, metadata = $3, \
		 locked_fields = $4, created_at = $5, updated_at = $6 WHERE work_id = $7",
		[
			identity.title.or(losing.title).into(),
			identity.author.or(losing.author).into(),
			DbValue::Json(metadata.map(Box::new)),
			DbValue::Json(locked_fields.map(Box::new)),
			created_at.into(),
			updated_at.into(),
			identity_work_id.to_owned().into(),
		],
	))
	.await?;
	conn.execute(Statement::from_sql_and_values(
		backend,
		"DELETE FROM book_work_metadata WHERE work_id = $1",
		[losing_work_id.to_owned().into()],
	))
	.await?;
	Ok(())
}

fn merge_json(
	identity: Option<JsonValue>,
	losing: Option<JsonValue>,
	merge_array: bool,
) -> Option<JsonValue> {
	match (identity, losing) {
		(Some(mut identity), Some(losing)) => {
			match (&mut identity, losing) {
				(JsonValue::Object(identity), JsonValue::Object(losing)) => {
					for (key, value) in losing {
						identity.entry(key).or_insert(value);
					}
				},
				(JsonValue::Array(identity), JsonValue::Array(losing)) if merge_array => {
					for value in losing {
						if !identity.contains(&value) {
							identity.push(value);
						}
					}
				},
				_ => {},
			}
			Some(identity)
		},
		(identity, losing) => identity.or(losing),
	}
}

#[derive(Debug)]
struct WorkReview {
	id: String,
	rating: i32,
	content: Option<String>,
	is_private: bool,
	created_at: DateTimeWithTimeZone,
	updated_at: DateTimeWithTimeZone,
}

async fn merge_work_reviews<C: ConnectionTrait>(
	conn: &C,
	backend: DbBackend,
	user_id: &str,
	losing_work_id: &str,
	identity_work_id: &str,
) -> Result<(), DbErr> {
	let losing_rows = conn
		.query_all(Statement::from_sql_and_values(
			backend,
			"SELECT id, rating, content, is_private, created_at, updated_at \
			 FROM book_reviews WHERE user_id = $1 AND work_id = $2",
			[user_id.to_owned().into(), losing_work_id.to_owned().into()],
		))
		.await?;
	for row in losing_rows {
		let losing = WorkReview {
			id: row.try_get("", "id")?,
			rating: row.try_get("", "rating")?,
			content: row.try_get("", "content")?,
			is_private: row.try_get("", "is_private")?,
			created_at: row.try_get("", "created_at")?,
			updated_at: row.try_get("", "updated_at")?,
		};
		let identity_row = conn
			.query_one(Statement::from_sql_and_values(
				backend,
				"SELECT id, rating, content, is_private, created_at, updated_at \
				 FROM book_reviews WHERE user_id = $1 AND work_id = $2",
				[
					user_id.to_owned().into(),
					identity_work_id.to_owned().into(),
				],
			))
			.await?;
		let Some(row) = identity_row else {
			conn.execute(Statement::from_sql_and_values(
				backend,
				"UPDATE book_reviews SET work_id = $1 WHERE id = $2",
				[identity_work_id.to_owned().into(), losing.id.into()],
			))
			.await?;
			continue;
		};
		let identity = WorkReview {
			id: row.try_get("", "id")?,
			rating: row.try_get("", "rating")?,
			content: row.try_get("", "content")?,
			is_private: row.try_get("", "is_private")?,
			created_at: row.try_get("", "created_at")?,
			updated_at: row.try_get("", "updated_at")?,
		};
		let losing_is_newer = losing.updated_at > identity.updated_at
			|| losing.updated_at == identity.updated_at && losing.id > identity.id;
		let rating = if losing_is_newer {
			losing.rating
		} else {
			identity.rating
		};
		let content = merge_review_content(identity.content, losing.content);
		let is_private = identity.is_private || losing.is_private;
		let created_at = identity.created_at.min(losing.created_at);
		let updated_at = identity.updated_at.max(losing.updated_at);
		conn.execute(Statement::from_sql_and_values(
			backend,
			"UPDATE book_reviews SET rating = $1, content = $2, is_private = $3, \
			 created_at = $4, updated_at = $5 WHERE id = $6",
			[
				rating.into(),
				content.into(),
				is_private.into(),
				created_at.into(),
				updated_at.into(),
				identity.id.into(),
			],
		))
		.await?;
		conn.execute(Statement::from_sql_and_values(
			backend,
			"DELETE FROM book_reviews WHERE id = $1",
			[losing.id.into()],
		))
		.await?;
	}
	Ok(())
}

fn merge_review_content(
	identity: Option<String>,
	losing: Option<String>,
) -> Option<String> {
	match (identity, losing) {
		(Some(identity), Some(losing)) if identity != losing => {
			Some(format!("{identity}\n\n{losing}"))
		},
		(Some(identity), _) => Some(identity),
		(None, losing) => losing,
	}
}
