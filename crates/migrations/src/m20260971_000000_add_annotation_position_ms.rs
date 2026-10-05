//! `media_annotations.position_ms`: a note anchored at a moment in an
//! audiobook.
//!
//! A native annotation is anchored by its Readium `locator`, which addresses
//! text inside a resource and cannot address a moment in a recording.
//! `position_ms` is milliseconds from the start of the *publication* — the
//! same unit as `media_audio.duration_ms`, `reading_heads.position_ms` and
//! `bookmarks.position_ms` — so a time-anchored note, an audio bookmark and a
//! reading head are comparable without a conversion.
//!
//! Nullable with no default: every existing annotation stays exactly as it
//! was, and a text- or page-anchored annotation never writes this column.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(MediaAnnotations::Table)
					.add_column(
						ColumnDef::new(MediaAnnotations::PositionMs)
							.big_integer()
							.null(),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(MediaAnnotations::Table)
					.drop_column(MediaAnnotations::PositionMs)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum MediaAnnotations {
	Table,
	PositionMs,
}

#[cfg(test)]
mod tests {
	use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};

	use super::*;

	async fn position(db: &DatabaseConnection) -> Option<i64> {
		db.query_one(Statement::from_string(
			DbBackend::Sqlite,
			"SELECT position_ms FROM media_annotations WHERE id = 'existing'".to_owned(),
		))
		.await
		.unwrap()
		.unwrap()
		.try_get("", "position_ms")
		.unwrap()
	}

	#[tokio::test]
	async fn existing_annotations_have_no_position() {
		let db = Database::connect("sqlite::memory:").await.unwrap();
		for sql in [
			"CREATE TABLE media_annotations (
				id TEXT PRIMARY KEY NOT NULL, locator TEXT NOT NULL
			)",
			"INSERT INTO media_annotations (id, locator)
			 VALUES ('existing', '{\"href\":\"OPS/chapter.xhtml\"}')",
		] {
			db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
				.await
				.unwrap();
		}

		Migration.up(&SchemaManager::new(&db)).await.unwrap();

		assert_eq!(position(&db).await, None);

		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"UPDATE media_annotations SET position_ms = 754000 WHERE id = 'existing'"
				.to_owned(),
		))
		.await
		.unwrap();
		assert_eq!(position(&db).await, Some(754_000));

		Migration.down(&SchemaManager::new(&db)).await.unwrap();
		let columns = db
			.query_all(Statement::from_string(
				DbBackend::Sqlite,
				"PRAGMA table_info(media_annotations)".to_owned(),
			))
			.await
			.unwrap();
		assert!(columns
			.iter()
			.all(|column| column.try_get::<String>("", "name").unwrap() != "position_ms"));
	}
}
