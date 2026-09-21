//! One book Stump mailed to a Kindle: `kindle_deliveries`.
//!
//! This is a delivery *log*, not a success log: a row is written whether the
//! relay accepted the message or refused it, and `error` is the difference.
//! Every column records what the delivery was at the time it happened —
//! `recipient` is the address the book actually went to, so re-pointing a
//! device later never rewrites its history.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kindle_deliveries")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	/// `Some` for legacy/device sends. Destination sends intentionally have no
	/// device and keep their owner/target snapshot below instead.
	#[sea_orm(column_type = "Text", nullable)]
	pub device_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub user_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub destination_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub destination_name: Option<String>,
	/// The Kindle address the book was mailed to, as it was registered on the
	/// device/destination at send time.
	#[sea_orm(column_type = "Text")]
	pub recipient: String,
	/// Extension of the attachment that went out: `azw3` when it was
	/// converted for the Kindle, otherwise the book's own format.
	#[sea_orm(column_type = "Text")]
	pub format: String,
	pub bytes: i64,
	pub converted: bool,
	/// Why the book was *not* converted — usually that the operator has no
	/// `boko` installed. Information, never a failure.
	#[sea_orm(column_type = "Text", nullable)]
	pub note: Option<String>,
	/// Why the delivery failed, when it did. `NULL` on a delivery that left.
	#[sea_orm(column_type = "Text", nullable)]
	pub error: Option<String>,
	pub sent_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
	#[sea_orm(
		belongs_to = "super::device::Entity",
		from = "Column::DeviceId",
		to = "super::device::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Device,
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl Related<super::device::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Device.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
