use sea_orm::entity::prelude::*;

/// Persisted desired/effective state for one compiled server component.
///
/// Human-facing labels, dependency metadata, and health are projected by the
/// runtime registry. This row is deliberately small: it stores the operator's
/// desired state and the last effective transition, not an attribution claim
/// about process memory.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "runtime_components")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub key: String,
	#[sea_orm(column_type = "Text")]
	pub label: String,
	#[sea_orm(column_type = "Text")]
	pub category: String,
	pub compiled: bool,
	pub desired_enabled: bool,
	pub effective_enabled: bool,
	#[sea_orm(column_type = "Text")]
	pub transition_mode: String,
	pub last_transition_at: Option<DateTimeWithTimeZone>,
	#[sea_orm(column_type = "Text", nullable)]
	pub last_error: Option<String>,
	pub created_at: DateTimeWithTimeZone,
	pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
