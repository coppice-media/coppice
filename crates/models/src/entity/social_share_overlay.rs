//! Immutable, recipient-readable snapshots of explicitly shared annotations.
//!
//! These rows are projections: they are never copied into native annotations,
//! bookmarks, or Liseur CAS state and never live-join the source user's rows.

use chrono::Utc;
use sea_orm::{
	entity::prelude::{async_trait::async_trait, *},
	ActiveValue,
};

pub const HIGHLIGHT: &str = "HIGHLIGHT";
pub const NOTE: &str = "NOTE";
pub const BOOKMARK: &str = "BOOKMARK";
pub const COLOR_YELLOW: &str = "yellow";
pub const COLOR_GREEN: &str = "green";
pub const COLOR_BLUE: &str = "blue";
pub const COLOR_PINK: &str = "pink";
pub const COLOR_PURPLE: &str = "purple";
pub const COLOR_ORANGE: &str = "orange";
pub const COLORS: [&str; 6] = [
	COLOR_YELLOW,
	COLOR_GREEN,
	COLOR_BLUE,
	COLOR_PINK,
	COLOR_PURPLE,
	COLOR_ORANGE,
];

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "SocialShareOverlayModel"))]
#[sea_orm(table_name = "social_share_overlays")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub grant_id: String,
	#[sea_orm(column_type = "Text")]
	pub source_user_id: String,
	#[sea_orm(column_type = "Text")]
	pub target_key: String,
	#[sea_orm(column_type = "Text")]
	pub kind: String,
	#[sea_orm(column_type = "Json", nullable)]
	pub locator: Option<serde_json::Value>,
	#[sea_orm(column_type = "Double", nullable)]
	pub progression: Option<f64>,
	#[sea_orm(column_type = "Integer", nullable)]
	pub percentage: Option<i32>,
	#[sea_orm(column_type = "Text", nullable)]
	pub excerpt: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub body: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub color: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub captured_at: DateTimeWithTimeZone,
	pub revision: i32,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub tombstoned_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::social_share_grant::Entity",
		from = "Column::GrantId",
		to = "super::social_share_grant::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Grant,
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::SourceUserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Source,
}

impl Related<super::social_share_grant::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Grant.def()
	}
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert {
			if self.id.is_not_set() {
				self.id = ActiveValue::Set(uuid::Uuid::new_v4().to_string());
			}
			if self.captured_at.is_not_set() {
				self.captured_at = ActiveValue::Set(Utc::now().into());
			}
			if self.revision.is_not_set() {
				self.revision = ActiveValue::Set(1);
			}
		}
		Ok(self)
	}
}

impl Entity {
	pub fn find_for_recipient(
		_user_id: &str,
		grant_ids: impl IntoIterator<Item = String>,
	) -> Select<Entity> {
		Self::find()
			.filter(Column::GrantId.is_in(grant_ids))
			.filter(Column::TombstonedAt.is_null())
	}

	pub fn deterministic_color(source_user_id: &str, target_key: &str) -> &'static str {
		let mut hash = 2166136261u32;
		for byte in source_user_id
			.bytes()
			.chain([0].into_iter())
			.chain(target_key.bytes())
		{
			hash ^= u32::from(byte);
			hash = hash.wrapping_mul(16777619);
		}
		COLORS[(hash as usize) % COLORS.len()]
	}

	pub fn valid_color(color: &str) -> bool {
		COLORS.contains(&color)
	}
}
