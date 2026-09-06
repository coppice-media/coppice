use sea_orm_migration::prelude::*;

/// Per-user Kavita on-deck removals: `POST /api/Series/remove-from-on-deck`
/// records a Kavita series here and `GET`-time on-deck queries exclude it
/// until the next read event clears the row (Kavita's `AppUserOnDeckRemoval`).
///
/// A Kavita series is a Stump series in Manga/Comic libraries and a single
/// media item in Book/LightNovel libraries, so the row stores the target as
/// `(target_kind, target_id)` with `target_kind` `series` or `media`. Neither
/// target carries a foreign key: the Kavita read-event path deletes the row,
/// and a row whose target has since been deleted hides nothing because the
/// target no longer lists.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(KavitaOnDeckRemovals::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(KavitaOnDeckRemovals::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(KavitaOnDeckRemovals::TargetKind)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(KavitaOnDeckRemovals::TargetId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(KavitaOnDeckRemovals::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.name("pk-kavita_on_deck_removals")
							.col(KavitaOnDeckRemovals::UserId)
							.col(KavitaOnDeckRemovals::TargetKind)
							.col(KavitaOnDeckRemovals::TargetId),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk_kavita_on_deck_removals_user")
							.from(
								KavitaOnDeckRemovals::Table,
								KavitaOnDeckRemovals::UserId,
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
			.drop_table(Table::drop().table(KavitaOnDeckRemovals::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum KavitaOnDeckRemovals {
	Table,
	UserId,
	TargetKind,
	TargetId,
	CreatedAt,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
