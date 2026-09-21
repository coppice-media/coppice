//! Recipient-only visibility preferences for shared annotation projections.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(
	feature = "graphql",
	graphql(name = "SocialShareOverlayPreferenceModel")
)]
#[sea_orm(table_name = "social_share_overlay_preferences")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub recipient_user_id: String,
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub overlay_id: String,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub hidden_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub restored_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::social_share_overlay::Entity",
		from = "Column::OverlayId",
		to = "super::social_share_overlay::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Overlay,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::RecipientUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Recipient,
}

impl Related<super::social_share_overlay::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Overlay.def()
	}
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Recipient.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
