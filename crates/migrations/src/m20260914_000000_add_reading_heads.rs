use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(ReadingHeadEvents::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReadingHeadEvents::Id)
							.big_integer()
							.not_null()
							.auto_increment()
							.primary_key(),
					)
					.col(ColumnDef::new(ReadingHeadEvents::UserId).text().not_null())
					.col(ColumnDef::new(ReadingHeadEvents::MediaId).text().not_null())
					.col(
						ColumnDef::new(ReadingHeadEvents::Protocol)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ReadingHeadEvents::DeviceId).text())
					.col(
						ColumnDef::new(ReadingHeadEvents::RawPayload)
							.json()
							.not_null(),
					)
					.col(ColumnDef::new(ReadingHeadEvents::Locator).json())
					.col(ColumnDef::new(ReadingHeadEvents::Progression).double())
					.col(ColumnDef::new(ReadingHeadEvents::Page).integer())
					.col(ColumnDef::new(ReadingHeadEvents::Completed).boolean())
					.col(
						ColumnDef::new(ReadingHeadEvents::TimestampKind)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReadingHeadEvents::SourceUpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReadingHeadEvents::ReceivedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReadingHeadEvents::Applied)
							.boolean()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.from(ReadingHeadEvents::Table, ReadingHeadEvents::UserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.from(ReadingHeadEvents::Table, ReadingHeadEvents::MediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_reading_head_events_user_media")
					.table(ReadingHeadEvents::Table)
					.col(ReadingHeadEvents::UserId)
					.col(ReadingHeadEvents::MediaId)
					.col(ReadingHeadEvents::Id)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(ReadingHeads::Table)
					.if_not_exists()
					.col(ColumnDef::new(ReadingHeads::UserId).text().not_null())
					.col(ColumnDef::new(ReadingHeads::MediaId).text().not_null())
					.col(ColumnDef::new(ReadingHeads::Locator).json())
					.col(
						ColumnDef::new(ReadingHeads::Progression)
							.double()
							.not_null(),
					)
					.col(ColumnDef::new(ReadingHeads::Page).integer())
					.col(ColumnDef::new(ReadingHeads::Completed).boolean().not_null())
					.col(
						ColumnDef::new(ReadingHeads::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReadingHeads::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReadingHeads::ChangedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReadingHeads::SourceProtocol)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ReadingHeads::SourceDeviceId).text())
					.col(ColumnDef::new(ReadingHeads::Revision).integer().not_null())
					.col(
						ColumnDef::new(ReadingHeads::EventId)
							.big_integer()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.col(ReadingHeads::UserId)
							.col(ReadingHeads::MediaId),
					)
					.foreign_key(
						ForeignKey::create()
							.from(ReadingHeads::Table, ReadingHeads::UserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.from(ReadingHeads::Table, ReadingHeads::MediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx_reading_heads_user_changed_at")
					.table(ReadingHeads::Table)
					.col(ReadingHeads::UserId)
					.col(ReadingHeads::ChangedAt)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(ReadingHeads::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(ReadingHeadEvents::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum ReadingHeads {
	Table,
	UserId,
	MediaId,
	Locator,
	Progression,
	Page,
	Completed,
	UpdatedAt,
	CreatedAt,
	ChangedAt,
	SourceProtocol,
	SourceDeviceId,
	Revision,
	EventId,
}

#[derive(DeriveIden)]
enum ReadingHeadEvents {
	Table,
	Id,
	UserId,
	MediaId,
	Protocol,
	DeviceId,
	RawPayload,
	Locator,
	Progression,
	Page,
	Completed,
	TimestampKind,
	SourceUpdatedAt,
	ReceivedAt,
	Applied,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}
