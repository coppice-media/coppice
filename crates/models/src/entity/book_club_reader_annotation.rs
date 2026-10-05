use sea_orm::entity::prelude::*;

/// Shared and private guest annotations, scoped to a club-queue book row.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "book_club_reader_annotations")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub session_id: String,
	#[sea_orm(column_type = "Text")]
	pub participant_id: String,
	#[sea_orm(column_type = "Text")]
	pub book_id: String,
	#[sea_orm(column_type = "Text")]
	pub kind: String,
	#[sea_orm(column_type = "Json", nullable)]
	pub locator: Option<Json>,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	#[sea_orm(column_type = "Text", nullable)]
	pub excerpt: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub body: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub color: Option<String>,
	pub shared: bool,
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
		belongs_to = "super::book_club_reader_participant::Entity",
		from = "Column::ParticipantId",
		to = "super::book_club_reader_participant::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Participant,
}

impl Related<super::book_club_reader_session::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Session.def()
	}
}

impl Related<super::book_club_reader_participant::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Participant.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
