//! `kindle_deliveries`: the history of every book Stump mailed to a Kindle.
//!
//! A send-to-Kindle delivery has two existing homes and neither is a history:
//! `emailer_send_records` records that *an* e-mail went out (it is keyed by
//! emailer, not by book) and `devices.last_sync_summary` keeps only the
//! **last** one. Neither can answer "did this book reach this Kindle, and
//! when?", which is the question an operator asks when Amazon silently drops
//! an attachment.
//!
//! The row therefore keeps what the delivery *was*, not what the device now
//! looks like: `recipient` is the address at send time (a device's address can
//! be re-pointed later), `format`/`bytes` describe the attachment that
//! actually left, `converted` says whether boko produced it, and `note` keeps
//! the reason an EPUB went out unconverted.
//!
//! `error` is what makes this a delivery log rather than a success log: an
//! SMTP refusal is written here with its message, while the emailer history
//! and the device summary stay untouched, because nothing was sent.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(KindleDeliveries::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(KindleDeliveries::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(KindleDeliveries::MediaId).text().not_null())
					.col(ColumnDef::new(KindleDeliveries::DeviceId).text().not_null())
					.col(
						ColumnDef::new(KindleDeliveries::Recipient)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(KindleDeliveries::Format).text().not_null())
					.col(
						ColumnDef::new(KindleDeliveries::Bytes)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(KindleDeliveries::Converted)
							.boolean()
							.not_null(),
					)
					.col(ColumnDef::new(KindleDeliveries::Note).text().null())
					.col(ColumnDef::new(KindleDeliveries::Error).text().null())
					.col(
						ColumnDef::new(KindleDeliveries::SentAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.from(KindleDeliveries::Table, KindleDeliveries::MediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.from(KindleDeliveries::Table, KindleDeliveries::DeviceId)
							.to(Devices::Table, Devices::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		// Both list surfaces are "the deliveries of one device" and "the
		// deliveries of one book", newest first, so each gets its own index.
		manager
			.create_index(
				Index::create()
					.name("kindle_deliveries_device_id_sent_at_idx")
					.table(KindleDeliveries::Table)
					.col(KindleDeliveries::DeviceId)
					.col(KindleDeliveries::SentAt)
					.if_not_exists()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("kindle_deliveries_media_id_sent_at_idx")
					.table(KindleDeliveries::Table)
					.col(KindleDeliveries::MediaId)
					.col(KindleDeliveries::SentAt)
					.if_not_exists()
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(KindleDeliveries::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum KindleDeliveries {
	Table,
	Id,
	MediaId,
	DeviceId,
	Recipient,
	Format,
	Bytes,
	Converted,
	Note,
	Error,
	SentAt,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Devices {
	Table,
	Id,
}
