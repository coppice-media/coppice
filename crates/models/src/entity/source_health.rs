//! Reachability record for one Keiyoushi catalog source, maintained by the
//! provider health job. Rows are keyed by the catalog source id; sources that
//! share a base URL are probed once per run and updated together.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "SourceHealthModel"))]
#[sea_orm(table_name = "source_health")]
pub struct Model {
	/// Keiyoushi catalog source id.
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub source_id: String,
	#[sea_orm(column_type = "Text")]
	pub name: String,
	#[sea_orm(column_type = "Text")]
	pub lang: String,
	#[sea_orm(column_type = "Text")]
	pub base_url: String,
	/// Detected multi-source theme (`MADARA`, `MANGA_THEMESIA`, `MMRCMS`).
	#[sea_orm(column_type = "Text", nullable)]
	pub theme: Option<String>,
	/// `UNKNOWN`, `OK`, `DEGRADED`, or `DEAD`.
	#[sea_orm(column_type = "Text")]
	pub status: String,
	pub http_status: Option<i32>,
	pub latency_ms: Option<i32>,
	/// Final URL when the base URL redirected elsewhere.
	#[sea_orm(column_type = "Text", nullable)]
	pub redirect_url: Option<String>,
	/// Whether the theme's latest-updates path answered successfully.
	pub latest_path_ok: Option<bool>,
	pub consecutive_failures: i32,
	#[sea_orm(column_type = "Text", nullable)]
	pub error: Option<String>,
	#[sea_orm(column_type = "custom(\"DATETIME\")", nullable)]
	pub checked_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
