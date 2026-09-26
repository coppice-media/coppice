use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(DevicePairings::Table)
					.add_column(
						ColumnDef::new(DevicePairings::AllowKomfMetadataEditing)
							.boolean()
							.not_null()
							.default(false),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(DevicePairings::Table)
					.drop_column(DevicePairings::AllowKomfMetadataEditing)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum DevicePairings {
	Table,
	AllowKomfMetadataEditing,
}
