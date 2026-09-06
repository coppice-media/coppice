//! `devices.kindle_email`: the Amazon *Send to Kindle* address of a device.
//!
//! A Kindle is reached by mail, not by a sync protocol: Amazon gives every
//! device a `@kindle.com` address and delivers whatever an approved sender
//! mails to it. The address is therefore per-device state exactly like
//! `transform_profile` and `library_scope`, not a second device kind, and it
//! is nullable because only a Kindle ever has one.
//!
//! It is deliberately *not* stored in `registered_email_devices` (the
//! `sendAttachmentEmail` address book): that table is a free-form list of
//! recipients with no owner, no last-seen state, and no protocol, so a send
//! to it could never be recorded as a device sighting.

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
					.add_column(ColumnDef::new(Devices::KindleEmail).text().null())
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(Devices::Table)
					.drop_column(Devices::KindleEmail)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum Devices {
	Table,
	KindleEmail,
}
