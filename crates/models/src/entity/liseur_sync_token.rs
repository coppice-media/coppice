use sea_orm::entity::prelude::*;

/// A bearer token accepted by the native liseur-sync protocol.
///
/// Timestamps are RFC 3339 strings with nanosecond precision, exactly as the
/// liseur-sync storage writes and parses them; the table predates the typed
/// timestamp columns used elsewhere.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "liseur_sync_tokens")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	/// Server-generated device identity stamped onto pushed records. For tokens
	/// minted through the device registry this is the `devices.id`.
	#[sea_orm(column_type = "Text")]
	pub device_id: String,
	#[sea_orm(column_type = "Text")]
	pub secret_hash: String,
	/// JSON array of protocol scopes
	#[sea_orm(column_type = "Text")]
	pub scopes: String,
	#[sea_orm(column_type = "Text")]
	pub created_at: String,
	#[sea_orm(column_type = "Text")]
	pub expires_at: String,
	#[sea_orm(column_type = "Text", nullable)]
	pub last_used_at: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub revoked_at: Option<String>,
	#[sea_orm(column_type = "Text", nullable)]
	pub name: Option<String>,
	/// `session` for the short-lived login credential, `device` otherwise
	#[sea_orm(column_type = "Text", nullable)]
	pub token_kind: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::user::Entity",
		from = "Column::UserId",
		to = "super::user::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	User,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
