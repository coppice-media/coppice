use sea_orm::entity::prelude::*;

/// A plain-text message in one reader session's private discussion.
///
/// `book_id` is the club queue row that was published when the message was
/// posted. Deletes are tombstones (`deleted_at` set, body cleared) so paging
/// cursors and the per-participant posting window survive them.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "book_club_reader_messages")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub session_id: String,
	#[sea_orm(column_type = "Text")]
	pub participant_id: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub book_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub body: String,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub edited_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub deleted_at: Option<DateTimeWithTimeZone>,
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
