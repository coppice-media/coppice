use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

/// One explicit MAM Bridge grab attached to a generic book request.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "mam_acquisition_grabs")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub request_id: String,
	pub torrent_id: i64,
	#[sea_orm(column_type = "Text", nullable)]
	pub bridge_grab_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub title: String,
	#[sea_orm(column_type = "Text")]
	pub phase: String,
	pub progress: f64,
	#[sea_orm(column_type = "Text", nullable)]
	pub error: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub ingest_item_id: Option<String>,
	/// Failed staged-ingest handoffs of a `completed` grab; the refresh job
	/// retries until the bound in `stump_core::mam_acquisition` and then marks
	/// the grab `error` with the last reason.
	pub handoff_attempts: i32,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::book_request::Entity",
		from = "Column::RequestId",
		to = "super::book_request::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Request,
	#[sea_orm(
		belongs_to = "super::ingest_drop_item::Entity",
		from = "Column::IngestItemId",
		to = "super::ingest_drop_item::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	IngestItem,
}

impl Related<super::book_request::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Request.def()
	}
}

impl Related<super::ingest_drop_item::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::IngestItem.def()
	}
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		let now = DateTimeWithTimeZone::from(Utc::now());
		if insert && self.created_at.is_not_set() {
			self.created_at = Set(now);
		}
		self.updated_at = Set(now);
		Ok(self)
	}
}
