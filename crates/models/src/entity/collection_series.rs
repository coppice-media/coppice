use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "collection_series")]
pub struct Model {
	#[sea_orm(primary_key)]
	pub id: i32,
	pub display_order: i32,
	#[sea_orm(column_type = "Text")]
	pub series_id: String,
	#[sea_orm(column_type = "Text")]
	pub collection_id: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::collection::Entity",
		from = "Column::CollectionId",
		to = "super::collection::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Collection,
	#[sea_orm(
		belongs_to = "super::series::Entity",
		from = "Column::SeriesId",
		to = "super::series::Column::Id",
		on_update = "Cascade",
		on_delete = "Restrict"
	)]
	Series,
}

impl Related<super::collection::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Collection.def()
	}
}

impl Related<super::series::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Series.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
