use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(ReadingSessions::Table)
					.add_column(ColumnDef::new(ReadingSessions::KoboState).json().null())
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(ReadingSessions::Table)
					.drop_column(ReadingSessions::KoboState)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum ReadingSessions {
	Table,
	KoboState,
}
