use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Per-user configuration and export state for each annotation sink.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(AnnotationSinkConfigs::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(AnnotationSinkConfigs::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationSinkConfigs::SinkId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(AnnotationSinkConfigs::Settings).json_binary())
					.col(ColumnDef::new(AnnotationSinkConfigs::Enabled).boolean().not_null().default(true))
					.col(
						ColumnDef::new(AnnotationSinkConfigs::LastRunAt)
							.timestamp_with_time_zone(),
					)
					.col(ColumnDef::new(AnnotationSinkConfigs::LastError).text())
					.col(ColumnDef::new(AnnotationSinkConfigs::State).json_binary())
					.col(
						ColumnDef::new(AnnotationSinkConfigs::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.primary_key(
						Index::create()
							.col(AnnotationSinkConfigs::UserId)
							.col(AnnotationSinkConfigs::SinkId),
					)
					.foreign_key(
						ForeignKey::create()
							.from(AnnotationSinkConfigs::Table, AnnotationSinkConfigs::UserId)
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
			.drop_table(Table::drop().table(AnnotationSinkConfigs::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum AnnotationSinkConfigs {
	Table,
	UserId,
	SinkId,
	Settings,
	Enabled,
	LastRunAt,
	LastError,
	State,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
