use sea_orm_migration::prelude::*;

/// Persists live component state and mergeable device telemetry snapshots.
///
/// Component descriptors remain code-owned (compile flags, labels, and
/// dependencies); these rows retain the operator's desired state and the last
/// effective state across restarts. Device telemetry is separate from the raw
/// protocol summary so one protocol cannot erase fields reported by another.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(RuntimeComponents::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(RuntimeComponents::Key)
							.string()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(RuntimeComponents::Label).string().not_null())
					.col(
						ColumnDef::new(RuntimeComponents::Category)
							.string()
							.not_null(),
					)
					.col(
						ColumnDef::new(RuntimeComponents::Compiled)
							.boolean()
							.not_null(),
					)
					.col(
						ColumnDef::new(RuntimeComponents::DesiredEnabled)
							.boolean()
							.not_null(),
					)
					.col(
						ColumnDef::new(RuntimeComponents::EffectiveEnabled)
							.boolean()
							.not_null(),
					)
					.col(
						ColumnDef::new(RuntimeComponents::TransitionMode)
							.string()
							.not_null(),
					)
					.col(
						ColumnDef::new(RuntimeComponents::LastTransitionAt)
							.timestamp_with_time_zone(),
					)
					.col(ColumnDef::new(RuntimeComponents::LastError).string())
					.col(
						ColumnDef::new(RuntimeComponents::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(RuntimeComponents::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(DeviceTelemetry::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(DeviceTelemetry::DeviceId)
							.string()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(DeviceTelemetry::BatteryPercent).integer())
					.col(ColumnDef::new(DeviceTelemetry::Charging).boolean())
					.col(ColumnDef::new(DeviceTelemetry::BatterySource).string())
					.col(
						ColumnDef::new(DeviceTelemetry::BatteryObservedAt)
							.timestamp_with_time_zone(),
					)
					.col(ColumnDef::new(DeviceTelemetry::SyncStatus).string())
					.col(ColumnDef::new(DeviceTelemetry::SyncProtocol).string())
					.col(
						ColumnDef::new(DeviceTelemetry::SyncedAt)
							.timestamp_with_time_zone(),
					)
					.col(ColumnDef::new(DeviceTelemetry::Progress).big_integer())
					.col(ColumnDef::new(DeviceTelemetry::Highlights).big_integer())
					.col(ColumnDef::new(DeviceTelemetry::Notes).big_integer())
					.col(ColumnDef::new(DeviceTelemetry::Bookmarks).big_integer())
					.col(ColumnDef::new(DeviceTelemetry::Sessions).big_integer())
					.col(ColumnDef::new(DeviceTelemetry::Items).big_integer())
					.col(
						ColumnDef::new(DeviceTelemetry::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-device_telemetry-device")
							.from(DeviceTelemetry::Table, DeviceTelemetry::DeviceId)
							.to(Devices::Table, Devices::Id)
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
			.drop_table(Table::drop().table(DeviceTelemetry::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(RuntimeComponents::Table).to_owned())
			.await?;
		Ok(())
	}
}

#[derive(DeriveIden)]
enum RuntimeComponents {
	Table,
	Key,
	Label,
	Category,
	Compiled,
	DesiredEnabled,
	EffectiveEnabled,
	TransitionMode,
	LastTransitionAt,
	LastError,
	CreatedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum DeviceTelemetry {
	Table,
	DeviceId,
	BatteryPercent,
	Charging,
	BatterySource,
	BatteryObservedAt,
	SyncStatus,
	SyncProtocol,
	SyncedAt,
	Progress,
	Highlights,
	Notes,
	Bookmarks,
	Sessions,
	Items,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum Devices {
	Table,
	Id,
}
