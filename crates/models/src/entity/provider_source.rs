//! A user-enabled remote source instance (one compiled source implementation
//! bound to a language, e.g. `mangadex-en`). Media and series rows created
//! from it reference `id` through their `source_provider` column.

use chrono::Utc;
use sea_orm::{
	entity::prelude::{async_trait::async_trait, *},
	ActiveValue,
};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "ProviderSourceModel"))]
#[sea_orm(table_name = "provider_sources")]
pub struct Model {
	/// Stable instance id, `<implementation>-<lang>`; also used in virtual paths.
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	/// The compiled source implementation, e.g. `mangadex`.
	#[sea_orm(column_type = "Text")]
	pub implementation: String,
	/// Keiyoushi catalog source id this instance was enabled from, when any.
	#[sea_orm(column_type = "Text", nullable)]
	pub catalog_id: Option<String>,
	#[sea_orm(column_type = "Text")]
	pub name: String,
	#[sea_orm(column_type = "Text")]
	pub lang: String,
	#[sea_orm(column_type = "Text")]
	pub base_url: String,
	pub enabled: bool,
	/// The user who enabled the instance.
	#[sea_orm(column_type = "Text", nullable)]
	pub created_by: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub updated_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert {
			self.created_at = ActiveValue::Set(Utc::now().into());
		} else {
			self.updated_at = ActiveValue::Set(Some(Utc::now().into()));
		}
		Ok(self)
	}
}
