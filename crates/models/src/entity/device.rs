use chrono::Utc;
use sea_orm::{entity::prelude::*, prelude::async_trait::async_trait, ActiveValue};

use crate::shared::enums::DeviceKind;

use super::user::AuthUser;

/// A registered client: a Kobo, a KOReader install, Mihon, Komelia, Liseur, an
/// OPDS reader, a script, or a browser. A device owns credentials (see
/// [`super::device_credential`]) and accumulates last-seen / last-sync state
/// each time one of those credentials authenticates a request.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "DeviceModel"))]
#[sea_orm(table_name = "devices")]
pub struct Model {
	#[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
	pub id: String,
	#[sea_orm(column_type = "Text")]
	pub user_id: String,
	#[sea_orm(column_type = "Text")]
	pub name: String,
	#[sea_orm(column_type = "Text")]
	pub kind: DeviceKind,
	/// A free-form, client-specific transform profile (e.g. KEPUB options for a
	/// Kobo, image sizing for a phone). Interpreted by the protocol adapters.
	#[sea_orm(column_type = "Json", nullable)]
	pub transform_profile: Option<Json>,
	/// The library ids this device may see, as a JSON array. `NULL` means the
	/// device inherits its user's visibility; `[]` is a device that sees
	/// nothing. Only ever intersected with the user's visibility, so it
	/// narrows and never widens — see
	/// [`VisibilityScope`](crate::shared::visibility::VisibilityScope).
	///
	/// Exposed to GraphQL as the typed `Device.libraryScope: [String!]`
	/// (see `graphql::object::device`), not as the raw JSON column.
	#[cfg_attr(feature = "graphql", graphql(skip))]
	#[sea_orm(column_type = "Json", nullable)]
	pub library_scope: Option<Json>,
	/// The Amazon *Send to Kindle* address of this device, or `NULL` when it
	/// has none. Only a Kindle is reachable this way: Amazon delivers what an
	/// approved sender mails to the address, so this is the whole transport
	/// for a device that speaks no sync protocol.
	#[sea_orm(column_type = "Text", nullable)]
	pub kindle_email: Option<String>,
	pub created_at: DateTimeWithTimeZone,
	/// The last time any credential of this device authenticated a request
	pub last_seen_at: Option<DateTimeWithTimeZone>,
	/// The last time a protocol reported a completed sync for this device
	pub last_sync_at: Option<DateTimeWithTimeZone>,
	/// The protocol-specific summary of the last sync, stored verbatim
	#[sea_orm(column_type = "Json", nullable)]
	pub last_sync_summary: Option<Json>,
	pub revoked_at: Option<DateTimeWithTimeZone>,
}

impl Model {
	pub fn is_revoked(&self) -> bool {
		self.revoked_at.is_some()
	}

	/// The parsed [`Self::library_scope`], or `None` when the device inherits
	/// its user's visibility. A stored value that is not an array of strings
	/// is treated as inherit and logged: a corrupt scope must not silently
	/// hide a user's whole library.
	pub fn library_scope_ids(&self) -> Option<Vec<String>> {
		let value = self.library_scope.as_ref()?;
		if value.is_null() {
			return None;
		}
		match value.as_array() {
			Some(entries) => entries
				.iter()
				.map(|entry| entry.as_str().map(str::to_owned))
				.collect::<Option<Vec<String>>>()
				.or_else(|| {
					tracing::error!(
						device_id = %self.id,
						"device library_scope holds a non-string entry; treating as inherit"
					);
					None
				}),
			None => {
				tracing::error!(
					device_id = %self.id,
					"device library_scope is not an array; treating as inherit"
				);
				None
			},
		}
	}
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
	#[sea_orm(has_many = "super::device_credential::Entity")]
	Credentials,
}

impl Related<super::user::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::User.def()
	}
}

impl Related<super::device_credential::Entity> for Entity {
	fn to() -> RelationDef {
		Relation::Credentials.def()
	}
}

impl Entity {
	/// Devices visible to `user`: their own, or every device for the server owner
	pub fn find_visible_to(user: &AuthUser) -> Select<Entity> {
		if user.is_server_owner {
			Entity::find()
		} else {
			Entity::find().filter(Column::UserId.eq(user.id.clone()))
		}
	}
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
	async fn before_save<C>(mut self, _db: &C, insert: bool) -> Result<Self, DbErr>
	where
		C: ConnectionTrait,
	{
		if insert {
			self.created_at = ActiveValue::Set(DateTimeWithTimeZone::from(Utc::now()));
			if self.id.is_not_set() {
				self.id = ActiveValue::Set(Uuid::new_v4().to_string());
			}
		}

		Ok(self)
	}
}
