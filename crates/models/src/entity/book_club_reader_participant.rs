use sea_orm::entity::prelude::*;

/// A participant's display identity and replaceable, digest-only capability.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "book_club_reader_participants")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub session_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub linked_user_id: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub created_by_user_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub display_name: String,
	pub share_progress: bool,
	#[sea_orm(column_type = "Text", nullable, unique)]
	pub token_digest: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub credential_expires_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub revoked_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::book_club_reader_session::Entity",
		from = "Column::SessionId",
		to = "super::book_club_reader_session::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Session,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::LinkedUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	LinkedUser,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::CreatedByUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "SetNull"
	)]
	CreatedByUser,
}

impl Related<super::book_club_reader_session::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Session.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
