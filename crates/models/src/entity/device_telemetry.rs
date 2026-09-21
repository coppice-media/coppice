use sea_orm::entity::prelude::*;

use crate::shared::enums::DeviceProtocol;

/// Typed, mergeable telemetry reported by protocol adapters for a device.
///
/// Every field is nullable because protocol clients report different slices of
/// this snapshot. Counters are cumulative observations and are never replaced
/// by a lower value during a merge.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "device_telemetry")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub device_id: String,
	pub battery_percent: Option<i32>,
	pub charging: Option<bool>,
	#[sea_orm(column_type = "Text", nullable)]
	pub battery_source: Option<String>,
	pub battery_observed_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub sync_status: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub sync_protocol: Option<DeviceProtocol>,
	pub synced_at: Option<DateTimeWithTimeZone>,
	pub progress: Option<i64>,
	pub highlights: Option<i64>,
	pub notes: Option<i64>,
	pub bookmarks: Option<i64>,
	pub sessions: Option<i64>,
	pub items: Option<i64>,
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::device::Entity",
		from = "Column::DeviceId",
		to = "super::device::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Device,
}

impl Related<super::device::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Device.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
