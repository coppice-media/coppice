use sea_orm_migration::prelude::*;

/// Per-user Kavita on-deck removals: `POST /api/Series/remove-from-on-deck`
/// records a series here and `GET`-time on-deck queries exclude it until the
/// next read event clears the row (Kavita's `AppUserOnDeckRemoval`).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(OnDeckRemovals::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(OnDeckRemovals::UserId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(OnDeckRemovals::SeriesId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(OnDeckRemovals::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_on_deck_removals_user")
							.from(OnDeckRemovals::Table, OnDeckRemovals::UserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_on_deck_removals_series")
							.from(OnDeckRemovals::Table, OnDeckRemovals::SeriesId)
							.to(Series::Table, Series::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(OnDeckRemovals::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum OnDeckRemovals {
	Table,
	UserId,
	SeriesId,
	CreatedAt,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Series {
	Table,
	Id,
}
