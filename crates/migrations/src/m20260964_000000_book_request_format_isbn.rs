//! Adds the requested format and optional ISBN snapshot to book requests.

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
						ColumnDef::new(BookRequests::Format)
							.text()
							.not_null()
							.default("ANY"),
					)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(BookRequests::Table)
					.add_column(ColumnDef::new(BookRequests::Isbn).text().null())
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(BookRequests::Table)
					.drop_column(BookRequests::Isbn)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(BookRequests::Table)
					.drop_column(BookRequests::Format)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum BookRequests {
	#[sea_orm(iden = "book_requests")]
	Table,
	Format,
	Isbn,
}

#[cfg(test)]
mod tests {
	use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

	use super::*;

	#[tokio::test]
	async fn existing_requests_get_any_format_and_no_isbn() {
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
				"SELECT format, isbn FROM book_requests WHERE id = 'existing'".to_owned(),
			))
			.await
			.unwrap()
			.unwrap();
		let format: String = row.try_get("", "format").unwrap();
		let isbn: Option<String> = row.try_get("", "isbn").unwrap();
		assert_eq!(format, "ANY");
		assert_eq!(isbn, None);
	}
}
