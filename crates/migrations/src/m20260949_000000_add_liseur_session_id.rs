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
					.add_column(ColumnDef::new(ReadingSessions::LiseurSessionId).text())
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("uq-reading_sessions-user-liseur-session")
					.table(ReadingSessions::Table)
					.col(ReadingSessions::UserId)
					.col(ReadingSessions::LiseurSessionId)
					.unique()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_index(
				Index::drop()
					.name("uq-reading_sessions-user-liseur-session")
					.table(ReadingSessions::Table)
					.to_owned(),
			)
			.await?;

		manager
			.alter_table(
				Table::alter()
					.table(ReadingSessions::Table)
					.drop_column(ReadingSessions::LiseurSessionId)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum ReadingSessions {
	#[sea_orm(iden = "reading_sessions")]
	Table,
	UserId,
	LiseurSessionId,
}
