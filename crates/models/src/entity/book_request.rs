use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue::Set};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_requests")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub requester_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub internal_media_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub internal_work_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub source_provider: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub remote_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub external_key: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub title: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub authors: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub cover_url: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub destination_shelf_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub destination_device_id: Option<String>,
	/// May contain historical acquisition states; active request writes use
	/// `PENDING`, `APPROVED`, and `REJECTED`.
	#[sea_orm(column_type = "Text")]
	pub status: String,
	#[sea_orm(column_type = "Text")]
	pub approval_policy: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub approved_by: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub rejected_by: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub failure_code: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub failure_message: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub approved_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub completed_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::RequesterId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Requester,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Requester.def()
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
