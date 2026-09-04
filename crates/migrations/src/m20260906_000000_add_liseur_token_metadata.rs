use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncTokens::Table)
					.add_column(ColumnDef::new(LiseurSyncTokens::Name).text().null())
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncTokens::Table)
					.add_column(ColumnDef::new(LiseurSyncTokens::TokenKind).text().null())
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncTokens::Table)
					.drop_column(LiseurSyncTokens::TokenKind)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(LiseurSyncTokens::Table)
					.drop_column(LiseurSyncTokens::Name)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum LiseurSyncTokens {
	#[sea_orm(iden = "liseur_sync_tokens")]
	Table,
	#[sea_orm(iden = "name")]
	Name,
	#[sea_orm(iden = "token_kind")]
	TokenKind,
}
