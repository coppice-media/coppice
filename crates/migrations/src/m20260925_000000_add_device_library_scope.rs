//! Per-device library scope ("virtual pathing") and the Kobo entitlement
//! deltas a scope change produces.
//!
//! `devices.library_scope` is a JSON array of library ids the device may see.
//! `NULL` means the device inherits its user's visibility; an empty array is a
//! device that sees nothing. The scope is only ever intersected with the
//! user's own visibility, so it can narrow but never widen.
//!
//! `device_entitlement_deltas` records what a scope change did to a Kobo
//! device's entitlements, because neither direction is derivable after the
//! fact: a book that left scope is invisible to the next query, and a book
//! that entered scope has no `created_at`/`modified_at` change for the
//! incremental sync window to notice. One row per (device, book) with the
//! latest transition, so a leave-then-rejoin collapses to nothing new.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(Devices::Table)
					.add_column(ColumnDef::new(Devices::LibraryScope).json().null())
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(DeviceEntitlementDeltas::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(DeviceEntitlementDeltas::DeviceId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(DeviceEntitlementDeltas::MediaId)
							.text()
							.not_null(),
					)
					// The (device, book) pair is the identity, so a later
					// transition upserts over the earlier one and no surrogate
					// key can let the same pair appear twice. It also indexes
					// the sync's `WHERE device_id = ?` lookup on its leading
					// column.
					.primary_key(
						Index::create()
							.col(DeviceEntitlementDeltas::DeviceId)
							.col(DeviceEntitlementDeltas::MediaId),
					)
					.col(
						ColumnDef::new(DeviceEntitlementDeltas::Removed)
							.boolean()
							.not_null(),
					)
					.col(
						ColumnDef::new(DeviceEntitlementDeltas::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-device_entitlement_deltas-device")
							.from(
								DeviceEntitlementDeltas::Table,
								DeviceEntitlementDeltas::DeviceId,
							)
							.to(Devices::Table, Devices::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-device_entitlement_deltas-media")
							.from(
								DeviceEntitlementDeltas::Table,
								DeviceEntitlementDeltas::MediaId,
							)
							.to(Media::Table, Media::Id)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(DeviceEntitlementDeltas::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.alter_table(
				Table::alter()
					.table(Devices::Table)
					.drop_column(Devices::LibraryScope)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum Devices {
	Table,
	Id,
	LibraryScope,
}

#[derive(DeriveIden)]
enum DeviceEntitlementDeltas {
	Table,
	DeviceId,
	MediaId,
	Removed,
	CreatedAt,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}
