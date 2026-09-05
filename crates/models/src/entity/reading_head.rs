use sea_orm::entity::prelude::*;

use crate::{domain::reading_state::SourceProtocol, shared::readium::ReadiumLocator};

/// The one canonical reading position per `(user, media)`.
///
/// Every protocol projects its native payload onto this row through
/// [`crate::services::reading_state::apply`]; the raw payloads and the
/// updates that lost the conflict rule live in
/// [`super::reading_head_event`]. `reading_sessions` remain the
/// session/statistics history and are not replaced by this row.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "reading_heads")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub user_id: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub media_id: String,
	/// The Readium projection of the position: verbatim for locator-based
	/// protocols, synthesized from `page` for page-based ones.
	#[sea_orm(column_type = "Json", nullable)]
	pub locator: Option<ReadiumLocator>,
	/// Whole-publication progression in `0..=1`.
	#[sea_orm(column_type = "Double")]
	pub progression: f64,
	/// The 1-based page when the position is page-addressed.
	pub page: Option<i32>,
	/// Sticky completion flag; only an explicit un-read clears it.
	pub completed: bool,
	/// The effective source time of the winning update (device time when the
	/// protocol carries one, otherwise server ingestion time).
	pub updated_at: DateTimeWithTimeZone,
	/// Server time the head was first materialized.
	pub created_at: DateTimeWithTimeZone,
	/// Server time of the last head mutation; the cursor for `heads_since`.
	pub changed_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "Text")]
	pub source_protocol: SourceProtocol,
	#[sea_orm(column_type = "Text", nullable)]
	pub source_device_id: Option<String>,
	/// Incremented on every accepted update.
	pub revision: i32,
	/// The [`super::reading_head_event`] that materialized this head.
	pub event_id: i64,
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

impl Entity {
	pub fn find_for_user(user_id: &str) -> Select<Entity> {
		Entity::find().filter(Column::UserId.eq(user_id))
	}
}

impl ActiveModelBehavior for ActiveModel {}
