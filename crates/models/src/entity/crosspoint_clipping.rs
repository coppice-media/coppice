use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "crosspoint_clippings")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub user_id: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub document: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	pub spine: Option<i64>,
	pub start_page: Option<i64>,
	pub end_page: Option<i64>,
	pub pages: Option<i64>,
	pub start_word: Option<i64>,
	pub end_word: Option<i64>,
	pub words: Option<i64>,
	pub para: Option<i64>,
	#[sea_orm(column_type = "Text", nullable)]
	pub chapter: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub text: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub note: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub color: Option<String>,
	pub created_at: Option<i64>,
	pub deleted: bool,
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
