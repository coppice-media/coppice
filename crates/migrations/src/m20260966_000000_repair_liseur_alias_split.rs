use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		super::m20260963_000000_repair_liseur_pair_work_identity::repair_alias_splits(
			manager.get_connection(),
		)
		.await
	}

	async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
		// Identity merges are data repairs and cannot be reversed safely.
		Ok(())
	}
}
