use chrono::Utc;
use sea_orm::entity::prelude::*;
use sea_orm::ActiveValue;

/// What a library-scope change did to one book's entitlement on one device.
///
/// A Kobo holds the entitlements it was handed by earlier syncs, and an
/// incremental sync only looks at `media.created_at` / `media.modified_at`.
/// Neither side of a scope change shows up there: a book that left scope is
/// invisible to the next visibility-filtered query, and a book that entered
/// scope did not change. So both directions are recorded here when the scope
/// is set, and the next sync turns them into a removal
/// (`ChangedEntitlement` with `IsRemoved`) or a `NewEntitlement`.
///
/// The (device, book) pair is the primary key, so a later transition
/// overwrites the earlier one with a plain upsert: a book that leaves and
/// rejoins before the next sync collapses to "still there". Rows are pruned
/// once a sync has carried them to the device.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "device_entitlement_deltas")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub device_id: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub media_id: String,
	/// `true` when the book left the device's scope, `false` when it entered.
	pub removed: bool,
	pub created_at: DateTimeWithTimeZone,
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
	#[sea_orm(
		belongs_to = "super::media::Entity",
		from = "Column::MediaId",
		to = "super::media::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Media,
}

impl Related<super::device::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Device.def()
	}
}

impl Related<super::media::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Media.def()
	}
}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert && self.created_at.is_not_set() {
			self.created_at = ActiveValue::Set(DateTimeWithTimeZone::from(Utc::now()));
		}

		Ok(self)
	}
}
