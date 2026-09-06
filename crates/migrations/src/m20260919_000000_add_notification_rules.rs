use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(NotificationRules::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(NotificationRules::Id)
							.integer()
							.not_null()
							.auto_increment()
							.primary_key(),
					)
					.col(ColumnDef::new(NotificationRules::UserId).text().not_null())
					.col(
						ColumnDef::new(NotificationRules::EventKind)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(NotificationRules::ChannelId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(NotificationRules::Enabled)
							.boolean()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.from(NotificationRules::Table, NotificationRules::UserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_notification_rules_user_event_channel")
					.table(NotificationRules::Table)
					.col(NotificationRules::UserId)
					.col(NotificationRules::EventKind)
					.col(NotificationRules::ChannelId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(NotificationChannelSettings::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(NotificationChannelSettings::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(NotificationChannelSettings::ChannelId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(NotificationChannelSettings::Settings)
							.json()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.col(NotificationChannelSettings::UserId)
							.col(NotificationChannelSettings::ChannelId),
					)
					.foreign_key(
						ForeignKey::create()
							.from(
								NotificationChannelSettings::Table,
								NotificationChannelSettings::UserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(NotificationChannelSettings::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(NotificationRules::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum NotificationRules {
	Table,
	Id,
	UserId,
	EventKind,
	ChannelId,
	Enabled,
}

#[derive(DeriveIden)]
enum NotificationChannelSettings {
	Table,
	UserId,
	ChannelId,
	Settings,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
