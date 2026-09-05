use sea_orm::entity::prelude::*;

use crate::shared::enums::{DeviceCredentialKind, DeviceProtocol};

/// A credential owned by a device. `credential_ref` points at the row holding
/// the secret hash: the `api_keys.short_token` for
/// [`DeviceCredentialKind::ApiKey`], the `liseur_sync_tokens.id` for
/// [`DeviceCredentialKind::LiseurToken`], or the `sessions.session_id` for
/// [`DeviceCredentialKind::Session`]. The reference is polymorphic, so there
/// is deliberately no foreign key.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "device_credentials")]
pub struct Model {
	#[sea_orm(primary_key)]
	pub id: i32,
	#[sea_orm(column_type = "Text")]
	pub device_id: String,
	/// The protocol this credential was minted for
	#[sea_orm(column_type = "Text")]
	pub protocol: DeviceProtocol,
	#[sea_orm(column_type = "Text")]
	pub credential_kind: DeviceCredentialKind,
	#[sea_orm(column_type = "Text")]
	pub credential_ref: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
	#[sea_orm(
		belongs_to = "super::device::Entity",
		from = "Column::DeviceId",
		to = "super::device::Column::Id",
		on_update = "Cascade",
		on_delete = "Cascade"
	)]
	Device,
}

impl Related<super::device::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Device.def()
	}
}

impl ActiveModelBehavior for ActiveModel {}
