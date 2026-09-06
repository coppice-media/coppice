//! The cross-source dedupe key of one materialised provider series.
//!
//! Written on every materialisation by
//! `stump_provider::identity::record_identity`. `normalised_title` is the
//! title reduced to comparable form and `external_key` the strongest
//! cross-source id the source carried (`al:<id>`, `mal:<id>`, ...), both
//! indexed so a duplicate check is a lookup.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "ProviderSeriesIdentityModel"))]
#[sea_orm(table_name = "provider_series_identity")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub series_id: String,
	/// The `provider_sources` instance the series was materialised from.
	#[sea_orm(column_type = "Text")]
	pub source_provider: String,
	/// Comparable form of the series title; see
	/// `stump_provider::identity::normalise_title`.
	#[sea_orm(column_type = "Text")]
	pub normalised_title: String,
	/// `<registry>:<id>` for the strongest cross-source id the source
	/// carried, e.g. `al:30002`. `None` when it carried none.
	#[sea_orm(column_type = "Text", nullable)]
	pub external_key: Option<String>,
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
