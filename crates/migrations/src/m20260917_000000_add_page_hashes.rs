use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(PageHashes::Table)
					.if_not_exists()
					.col(ColumnDef::new(PageHashes::MediaId).text().not_null())
					.col(ColumnDef::new(PageHashes::Page).integer().not_null())
					.col(ColumnDef::new(PageHashes::Dhash).big_integer().not_null())
					.col(
						ColumnDef::new(PageHashes::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.col(PageHashes::MediaId)
							.col(PageHashes::Page),
					)
					.foreign_key(
						ForeignKey::create()
							.from(PageHashes::Table, PageHashes::MediaId)
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
					.name("idx_page_hashes_dhash")
					.table(PageHashes::Table)
					.col(PageHashes::Dhash)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(KnownDuplicatePages::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(KnownDuplicatePages::LibraryId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(KnownDuplicatePages::Dhash)
							.big_integer()
							.not_null(),
					)
					.col(ColumnDef::new(KnownDuplicatePages::Action).text().not_null())
					.col(ColumnDef::new(KnownDuplicatePages::CreatedBy).text())
					.col(
						ColumnDef::new(KnownDuplicatePages::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.col(KnownDuplicatePages::LibraryId)
							.col(KnownDuplicatePages::Dhash),
					)
					.foreign_key(
						ForeignKey::create()
							.from(KnownDuplicatePages::Table, KnownDuplicatePages::LibraryId)
							.to(Libraries::Table, Libraries::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.from(KnownDuplicatePages::Table, KnownDuplicatePages::CreatedBy)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::SetNull),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(KnownDuplicatePages::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(Table::drop().table(PageHashes::Table).if_exists().to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum PageHashes {
	Table,
	MediaId,
	Page,
	Dhash,
	CreatedAt,
}

#[derive(DeriveIden)]
enum KnownDuplicatePages {
	Table,
	LibraryId,
	Dhash,
	Action,
	CreatedBy,
	CreatedAt,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Libraries {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
