use sea_orm::entity::prelude::*;
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "book_request_releases")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub search_id: String,
	#[sea_orm(column_type = "Text")]
	pub request_id: String,
	#[sea_orm(column_type = "Text")]
	pub source_provider: String,
	#[sea_orm(column_type = "Text")]
	pub remote_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub external_key: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub title: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub authors: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub format: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub language: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub edition: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub quality: Option<String>,
	pub size_bytes: Option<i64>,
	pub seeders: Option<i32>,
	#[sea_orm(column_type = "Text", nullable)]
	pub preview_name: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub preview_mime: Option<String>,
	pub preview_bytes: Option<i64>,
	pub score: i32,
	#[sea_orm(column_type = "Json")]
	pub score_components: JsonValue,
	pub rank: i32,
	pub selected: bool,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::book_request_search::Entity",
		from = "Column::SearchId",
		to = "super::book_request_search::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Search,
	#[sea_orm(
		belongs_to = "super::book_request::Entity",
		from = "Column::RequestId",
		to = "super::book_request::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Request,
}

impl Related<super::book_request_search::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Search.def()
	}
}
impl Related<super::book_request::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Request.def()
	}
}
impl ActiveModelBehavior for ActiveModel {}
