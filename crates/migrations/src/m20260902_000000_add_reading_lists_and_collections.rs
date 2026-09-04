use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum ReadingLists {
	Table,
	Id,
	Name,
	Description,
	UpdatedAt,
	Visibility,
	Ordering,
	CreatingUserId,
}

#[derive(DeriveIden)]
enum ReadingListItems {
	Table,
	Id,
	DisplayOrder,
	MediaId,
	ReadingListId,
}

#[derive(DeriveIden)]
enum ReadingListRules {
	Table,
	Id,
	Role,
	UserId,
	ReadingListId,
}

#[derive(DeriveIden)]
enum Collections {
	Table,
	Id,
	Name,
	Description,
	UpdatedAt,
	Ordered,
	CreatingUserId,
}

#[derive(DeriveIden)]
enum CollectionSeries {
	Table,
	Id,
	DisplayOrder,
	SeriesId,
	CollectionId,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(ReadingLists::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReadingLists::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(ReadingLists::Name).text().not_null())
					.col(ColumnDef::new(ReadingLists::Description).text())
					.col(
						ColumnDef::new(ReadingLists::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(ColumnDef::new(ReadingLists::Visibility).text().not_null())
					.col(
						ColumnDef::new(ReadingLists::Ordering)
							.text()
							.not_null()
							.default("MANUAL"),
					)
					.col(
						ColumnDef::new(ReadingLists::CreatingUserId)
							.text()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-reading_lists-user")
							.from(ReadingLists::Table, ReadingLists::CreatingUserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(Collections::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(Collections::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(Collections::Name).text().not_null())
					.col(ColumnDef::new(Collections::Description).text())
					.col(
						ColumnDef::new(Collections::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(Collections::Ordered)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(Collections::CreatingUserId)
							.text()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-collections-user")
							.from(Collections::Table, Collections::CreatingUserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(ReadingListItems::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReadingListItems::Id)
							.integer()
							.not_null()
							.auto_increment()
							.primary_key(),
					)
					.col(
						ColumnDef::new(ReadingListItems::DisplayOrder)
							.integer()
							.not_null(),
					)
					.col(ColumnDef::new(ReadingListItems::MediaId).text().not_null())
					.col(
						ColumnDef::new(ReadingListItems::ReadingListId)
							.text()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-reading_list_items-media")
							.from(ReadingListItems::Table, ReadingListItems::MediaId)
							.to(Media::Table, Media::Id)
							.on_delete(ForeignKeyAction::Restrict)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-reading_list_items-reading_list")
							.from(
								ReadingListItems::Table,
								ReadingListItems::ReadingListId,
							)
							.to(ReadingLists::Table, ReadingLists::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(ReadingListRules::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReadingListRules::Id)
							.integer()
							.not_null()
							.auto_increment()
							.primary_key(),
					)
					.col(ColumnDef::new(ReadingListRules::Role).integer().not_null())
					.col(ColumnDef::new(ReadingListRules::UserId).text().not_null())
					.col(
						ColumnDef::new(ReadingListRules::ReadingListId)
							.text()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-reading_list_rules-reading_list")
							.from(
								ReadingListRules::Table,
								ReadingListRules::ReadingListId,
							)
							.to(ReadingLists::Table, ReadingLists::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-reading_list_rules-user")
							.from(ReadingListRules::Table, ReadingListRules::UserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(CollectionSeries::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(CollectionSeries::Id)
							.integer()
							.not_null()
							.auto_increment()
							.primary_key(),
					)
					.col(
						ColumnDef::new(CollectionSeries::DisplayOrder)
							.integer()
							.not_null(),
					)
					.col(ColumnDef::new(CollectionSeries::SeriesId).text().not_null())
					.col(
						ColumnDef::new(CollectionSeries::CollectionId)
							.text()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-collection_series-series")
							.from(CollectionSeries::Table, CollectionSeries::SeriesId)
							.to(Series::Table, Series::Id)
							.on_delete(ForeignKeyAction::Restrict)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-collection_series-collection")
							.from(CollectionSeries::Table, CollectionSeries::CollectionId)
							.to(Collections::Table, Collections::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("idx-reading_lists-creating_user_id")
					.table(ReadingLists::Table)
					.col(ReadingLists::CreatingUserId)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-reading_list_items-reading_list_order")
					.table(ReadingListItems::Table)
					.col(ReadingListItems::ReadingListId)
					.col(ReadingListItems::DisplayOrder)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-reading_list_items-reading_list_media")
					.table(ReadingListItems::Table)
					.col(ReadingListItems::ReadingListId)
					.col(ReadingListItems::MediaId)
					.unique()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-reading_list_rules-user_id")
					.table(ReadingListRules::Table)
					.col(ReadingListRules::UserId)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-reading_list_rules-reading_list_user")
					.table(ReadingListRules::Table)
					.col(ReadingListRules::ReadingListId)
					.col(ReadingListRules::UserId)
					.unique()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-collections-creating_user_id")
					.table(Collections::Table)
					.col(Collections::CreatingUserId)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-collection_series-collection_series")
					.table(CollectionSeries::Table)
					.col(CollectionSeries::CollectionId)
					.col(CollectionSeries::SeriesId)
					.unique()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-collection_series-collection_order")
					.table(CollectionSeries::Table)
					.col(CollectionSeries::CollectionId)
					.col(CollectionSeries::DisplayOrder)
					.unique()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-collection_series-series_id")
					.table(CollectionSeries::Table)
					.col(CollectionSeries::SeriesId)
					.to_owned(),
			)
			.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(CollectionSeries::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(ReadingListItems::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(ReadingListRules::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(Collections::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(ReadingLists::Table).to_owned())
			.await?;

		Ok(())
	}
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

#[derive(DeriveIden)]
enum Series {
	Table,
	Id,
}
