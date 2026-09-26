//! Adds the optional preferred narrator to book requests. A request stays
//! "this work in this format"; the narrator only biases audiobook release
//! ranking and never filters it.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(BookRequests::Table)
					.add_column(
						ColumnDef::new(BookRequests::PreferredNarrator)
							.text()
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
					.table(BookRequests::Table)
					.drop_column(BookRequests::PreferredNarrator)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum BookRequests {
	#[sea_orm(iden = "book_requests")]
	Table,
	PreferredNarrator,
}

#[cfg(test)]
mod tests {
	use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

	use super::*;

	#[tokio::test]
	async fn existing_requests_have_no_preferred_narrator() {
		let db = Database::connect("sqlite::memory:").await.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"CREATE TABLE book_requests (id TEXT PRIMARY KEY NOT NULL)".to_owned(),
		))
		.await
		.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"INSERT INTO book_requests (id) VALUES ('existing')".to_owned(),
		))
		.await
		.unwrap();

		Migration.up(&SchemaManager::new(&db)).await.unwrap();

		let row = db
			.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT preferred_narrator FROM book_requests WHERE id = 'existing'"
					.to_owned(),
			))
			.await
			.unwrap()
			.unwrap();
		let narrator: Option<String> = row.try_get("", "preferred_narrator").unwrap();
		assert_eq!(narrator, None);

		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"UPDATE book_requests SET preferred_narrator = 'Ray Porter' WHERE id = 'existing'"
				.to_owned(),
		))
		.await
		.unwrap();
		Migration.down(&SchemaManager::new(&db)).await.unwrap();
		let columns = db
			.query_all(Statement::from_string(
				DbBackend::Sqlite,
				"PRAGMA table_info(book_requests)".to_owned(),
			))
			.await
			.unwrap();
		assert!(columns
			.iter()
			.all(|column| column.try_get::<String>("", "name").unwrap()
				!= "preferred_narrator"));
	}
}
