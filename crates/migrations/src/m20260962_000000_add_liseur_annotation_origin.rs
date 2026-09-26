use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncAnnotations::Table)
					.add_column(
						ColumnDef::new(LiseurSyncAnnotations::OriginDeviceId)
							.text()
							.null(),
					)
					.to_owned(),
			)
			.await?;

		let conn = manager.get_connection();
		conn.execute(Statement::from_string(
			conn.get_database_backend(),
			"UPDATE liseur_sync_annotations
			 SET origin_device_id = device_id
			 WHERE origin_device_id IS NULL"
				.to_owned(),
		))
		.await?;
		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncAnnotations::Table)
					.drop_column(LiseurSyncAnnotations::OriginDeviceId)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum LiseurSyncAnnotations {
	#[sea_orm(iden = "liseur_sync_annotations")]
	Table,
	OriginDeviceId,
}

#[cfg(test)]
mod tests {
	use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

	use super::*;

	#[tokio::test]
	async fn origin_device_backfills_existing_creator_attribution() {
		let db = Database::connect("sqlite::memory:").await.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"CREATE TABLE liseur_sync_annotations (
				annotation_id TEXT NOT NULL, device_id TEXT NOT NULL
			)"
			.to_owned(),
		))
		.await
		.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"INSERT INTO liseur_sync_annotations (annotation_id, device_id)
			 VALUES ('existing', 'koreader-device')"
				.to_owned(),
		))
		.await
		.unwrap();

		Migration.up(&SchemaManager::new(&db)).await.unwrap();

		let row = db
			.query_one(Statement::from_string(
				DbBackend::Sqlite,
				"SELECT origin_device_id FROM liseur_sync_annotations
				 WHERE annotation_id = 'existing'"
					.to_owned(),
			))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			row.try_get::<String>("", "origin_device_id").unwrap(),
			"koreader-device"
		);
	}
}
