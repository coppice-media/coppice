use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(DevicePairings::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(DevicePairings::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(DevicePairings::Kind).text().not_null())
					.col(ColumnDef::new(DevicePairings::Name).text())
					.col(ColumnDef::new(DevicePairings::CodeHash).text().not_null())
					.col(ColumnDef::new(DevicePairings::Nonce).text().not_null())
					.col(ColumnDef::new(DevicePairings::RemoteIp).text().not_null())
					.col(ColumnDef::new(DevicePairings::UserId).text())
					.col(
						ColumnDef::new(DevicePairings::Status)
							.text()
							.not_null()
							.default("PENDING"),
					)
					.col(
						ColumnDef::new(DevicePairings::FailedAttempts)
							.integer()
							.not_null()
							.default(0),
					)
					.col(
						ColumnDef::new(DevicePairings::CredentialIssued)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(DevicePairings::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(DevicePairings::ExpiresAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(DevicePairings::ApprovedAt)
							.timestamp_with_time_zone(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-device-pairings-user")
							.from(DevicePairings::Table, DevicePairings::UserId)
							.to(Users::Table, Users::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		// Pending-list and per-IP cap lookups
		manager
			.create_index(
				Index::create()
					.name("idx-device-pairings-status-expires")
					.table(DevicePairings::Table)
					.col(DevicePairings::Status)
					.col(DevicePairings::ExpiresAt)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-device-pairings-remote-ip")
					.table(DevicePairings::Table)
					.col(DevicePairings::RemoteIp)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(DevicePairings::Table).to_owned())
			.await
	}
}

#[derive(Iden)]
enum DevicePairings {
	Table,
	Id,
	Kind,
	Name,
	CodeHash,
	Nonce,
	RemoteIp,
	UserId,
	Status,
	FailedAttempts,
	CredentialIssued,
	CreatedAt,
	ExpiresAt,
	ApprovedAt,
}

#[derive(Iden)]
enum Users {
	Table,
	Id,
}
