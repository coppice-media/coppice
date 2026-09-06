//! A materialised provider series recorded as the same work as an earlier
//! (canonical) series from another source.
//!
//! The link is advisory: it is written when a series materialises and matches
//! an existing identity, and nothing is merged until an operator calls
//! `mergeProviderSeries`. `reason` records why the two matched
//! (`EXTERNAL_KEY` or `TITLE`).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "ProviderSeriesLinkModel"))]
#[sea_orm(table_name = "provider_series_links")]
pub struct Model {
	/// The newly materialised (duplicate) series.
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub series_id: String,
	/// The series that already existed, from a different source.
	#[sea_orm(column_type = "Text")]
	pub canonical_series_id: String,
	/// `EXTERNAL_KEY` when a cross-source id matched, `TITLE` when only the
	/// normalised titles did.
	#[sea_orm(column_type = "Text")]
	pub reason: String,
	#[sea_orm(column_type = "custom(\"DATETIME\")")]
	pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::series::Entity",
		from = "Column::SeriesId",
		to = "super::series::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Series,
}

impl Related<super::series::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Series.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
