use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Containers gain the Kobo-shelf projection surface:
///
/// - `kobo_shelf` marks whether a container is projected to Kobo devices as a
///   `Tag` (shelves are `collections` ∪ `reading_lists` with the flag set).
/// - `source_device` records the last writer when a mutation came through the
///   Kobo `tags` write-back endpoints (last-writer-wins on `name`).
/// - `tags.kind` separates `genre` rows from `tag` rows so Komga `genres` and
///   `tags` never conflate across round trips.
/// - `kobo_shelf_tombstones` lets an incremental sync emit `DeletedTag` for
///   shelves deleted between syncs (the container row is gone, so the device
///   needs a durable record of what disappeared).
#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(Collections::Table)
					.add_column(
						ColumnDef::new(Collections::KoboShelf)
							.boolean()
							.not_null()
							.default(true),
					)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Collections::Table)
					.add_column(ColumnDef::new(Collections::SourceDevice).text())
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(ReadingLists::Table)
					.add_column(
						ColumnDef::new(ReadingLists::KoboShelf)
							.boolean()
							.not_null()
							.default(true),
					)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(ReadingLists::Table)
					.add_column(ColumnDef::new(ReadingLists::SourceDevice).text())
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Tags::Table)
					.add_column(
						ColumnDef::new(Tags::Kind)
							.text()
							.not_null()
							.default("tag"),
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_table(
				Table::create()
					.table(KoboShelfTombstones::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(KoboShelfTombstones::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(KoboShelfTombstones::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(KoboShelfTombstones::ShelfId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(KoboShelfTombstones::DeletedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-kobo_shelf_tombstones-user")
							.from(KoboShelfTombstones::Table, KoboShelfTombstones::UserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(KoboShelfTombstones::Table).to_owned())
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Tags::Table)
					.drop_column(Tags::Kind)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(ReadingLists::Table)
					.drop_column(ReadingLists::SourceDevice)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(ReadingLists::Table)
					.drop_column(ReadingLists::KoboShelf)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Collections::Table)
					.drop_column(Collections::SourceDevice)
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Collections::Table)
					.drop_column(Collections::KoboShelf)
					.to_owned(),
			)
			.await?;
		Ok(())
	}
}

#[derive(DeriveIden)]
enum Collections {
	Table,
	KoboShelf,
	SourceDevice,
}

#[derive(DeriveIden)]
enum ReadingLists {
	Table,
	KoboShelf,
	SourceDevice,
}

#[derive(DeriveIden)]
enum Tags {
	Table,
	Kind,
}

#[derive(DeriveIden)]
enum KoboShelfTombstones {
	Table,
	Id,
	UserId,
	ShelfId,
	DeletedAt,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
