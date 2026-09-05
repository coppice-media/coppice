use sea_orm::entity::prelude::*;

use crate::{
	domain::reading_state::{SourceProtocol, TimestampKind},
	shared::readium::ReadiumLocator,
};

/// Append-only provenance for [`super::reading_head`]: one row per protocol
/// update, whether or not it moved the head. The native payload is stored
/// unchanged beside the projected head fields so a losing or foreign position
/// (a KOReader x-pointer, Kobo statistics, ...) is never reduced to its
/// Readium projection.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "reading_head_events")]
pub struct Model {
	#[sea_orm(primary_key)]
	pub id: i64,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub media_id: String,
	#[sea_orm(column_type = "Text")]
	pub protocol: SourceProtocol,
	#[sea_orm(column_type = "Text", nullable)]
	pub device_id: Option<String>,
	/// The complete native request or record, verbatim.
	#[sea_orm(column_type = "Json")]
	pub raw_payload: Json,
	/// The Readium projection asserted by this update, if any.
	#[sea_orm(column_type = "Json", nullable)]
	pub locator: Option<ReadiumLocator>,
	#[sea_orm(column_type = "Double", nullable)]
	pub progression: Option<f64>,
	pub page: Option<i32>,
	/// The completion asserted by the update; `None` when it did not assert one.
	pub completed: Option<bool>,
	#[sea_orm(column_type = "Text")]
	pub timestamp_kind: TimestampKind,
	/// The effective source time used by the conflict rule.
	pub source_updated_at: DateTimeWithTimeZone,
	pub received_at: DateTimeWithTimeZone,
	/// Whether this event moved the head (`false` = provenance only).
	pub applied: bool,
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
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
