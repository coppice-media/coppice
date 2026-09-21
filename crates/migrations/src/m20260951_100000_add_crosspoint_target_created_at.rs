use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

/// Repairs the CrossPoint target table created by `m20260951` so the entity's
/// creation timestamp is present for existing and newly discovered targets.
/// The column is added in one ALTER operation, as required by SQLite.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(CrosspointDeviceTargets::Table)
					.add_column(
						ColumnDef::new(CrosspointDeviceTargets::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							// SQLite only permits constant ADD COLUMN defaults.
							.default("1970-01-01 00:00:00"),
					)
					.to_owned(),
			)
			.await?;

		let conn = manager.get_connection();
		let backend = conn.get_database_backend();
		conn.execute(Statement::from_string(
			backend,
			"UPDATE crosspoint_device_targets SET created_at = CURRENT_TIMESTAMP"
				.to_owned(),
		))
		.await?;
		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(CrosspointDeviceTargets::Table)
					.drop_column(CrosspointDeviceTargets::CreatedAt)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum CrosspointDeviceTargets {
	#[sea_orm(iden = "crosspoint_device_targets")]
	Table,
	CreatedAt,
}
