//! Account-scoped key/value settings synchronized with Liseur clients.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(LiseurSyncSettings::Table)
					.if_not_exists()
					.col(ColumnDef::new(LiseurSyncSettings::UserId).text().not_null())
					.col(
						ColumnDef::new(LiseurSyncSettings::SettingKey)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(LiseurSyncSettings::Value).text().not_null())
					.col(
						ColumnDef::new(LiseurSyncSettings::UpdatedAt)
							.text()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.col(LiseurSyncSettings::UserId)
							.col(LiseurSyncSettings::SettingKey),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-liseur-sync-settings-user")
							.from(LiseurSyncSettings::Table, LiseurSyncSettings::UserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(LiseurSyncSettings::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum LiseurSyncSettings {
	#[sea_orm(iden = "liseur_sync_settings")]
	Table,
	UserId,
	SettingKey,
	Value,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
