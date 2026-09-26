//! Counts the failed staged-ingest handoffs of a completed MAM grab so the
//! refresh job can retry a stranded download a bounded number of times and
//! then surface the last failure instead of retrying forever.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(MamAcquisitionGrabs::Table)
					.add_column(
						ColumnDef::new(MamAcquisitionGrabs::HandoffAttempts)
							.integer()
							.not_null()
							.default(0),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(MamAcquisitionGrabs::Table)
					.drop_column(MamAcquisitionGrabs::HandoffAttempts)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum MamAcquisitionGrabs {
	#[sea_orm(iden = "mam_acquisition_grabs")]
	Table,
	HandoffAttempts,
}

#[cfg(test)]
mod tests {
	use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

	use super::*;

	#[tokio::test]
	async fn existing_grabs_start_with_no_handoff_attempts() {
		let db = Database::connect("sqlite::memory:").await.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"CREATE TABLE mam_acquisition_grabs (id TEXT PRIMARY KEY NOT NULL)"
				.to_owned(),
		))
		.await
		.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"INSERT INTO mam_acquisition_grabs (id) VALUES ('existing')".to_owned(),
		))
		.await
		.unwrap();

		Migration.up(&SchemaManager::new(&db)).await.unwrap();

		let row = db
			.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT handoff_attempts FROM mam_acquisition_grabs WHERE id = 'existing'"
					.to_owned(),
			))
			.await
			.unwrap()
			.unwrap();
		let attempts: i32 = row.try_get("", "handoff_attempts").unwrap();
		assert_eq!(attempts, 0);

		Migration.down(&SchemaManager::new(&db)).await.unwrap();
		let columns = db
			.query_all(Statement::from_string(
				DbBackend::Sqlite,
				"PRAGMA table_info(mam_acquisition_grabs)".to_owned(),
			))
			.await
			.unwrap();
		assert!(columns
			.iter()
			.all(|column| column.try_get::<String>("", "name").unwrap()
				!= "handoff_attempts"));
	}
}
