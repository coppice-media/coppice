use std::{collections::HashSet, sync::Arc};

use chrono::{Duration, Utc};
use models::{
	entity::{
		api_key, device, device_credential, session,
		user::{self, AuthUser},
	},
	shared::{
		api_key::API_KEY_PREFIX,
		enums::{DeviceCredentialKind, DeviceKind, DeviceProtocol},
	},
};
use sea_orm::{
	prelude::*, ActiveValue::Set, DatabaseConnection, QueryOrder, QuerySelect,
	TransactionTrait,
};
use serde_json::Value as JsonValue;
use stump_api_types::RequestOrigin;

use crate::{
	credential::{
		api_key_permissions_for, credential_kind_for, liseur, mint_api_key,
		protocol_for, required_permissions, CredentialRef, IssuedCredential,
	},
	endpoint::Endpoint,
	error::{DeviceError, DeviceResult},
	event::DeviceSeen,
};

/// Receives a [`DeviceSeen`] each time [`DeviceService::touch`] updates a device.
pub type SeenListener = Arc<dyn Fn(DeviceSeen) + Send + Sync>;

/// `last_seen_at` is written at most once per interval per device unless a
/// sync summary is reported; protocol clients burst many requests per sync.
const TOUCH_INTERVAL: Duration = Duration::seconds(30);

const MAX_NAME_CHARS: usize = 128;

#[derive(Clone)]
pub struct DeviceService {
	conn: Arc<DatabaseConnection>,
	on_seen: Option<SeenListener>,
}

impl DeviceService {
	pub fn new(conn: Arc<DatabaseConnection>) -> Self {
		Self {
			conn,
			on_seen: None,
		}
	}

	/// Forward every recorded sighting, e.g. onto the core event channel.
	pub fn with_seen_listener(
		mut self,
		listener: impl Fn(DeviceSeen) + Send + Sync + 'static,
	) -> Self {
		self.on_seen = Some(Arc::new(listener));
		self
	}

	pub fn conn(&self) -> &DatabaseConnection {
		self.conn.as_ref()
	}

	/// Every device visible to `user`: their own, or all of them for the server
	/// owner. Revoked devices are included so their history stays visible.
	pub async fn list(&self, user: &AuthUser) -> DeviceResult<Vec<device::Model>> {
		Ok(device::Entity::find_visible_to(user)
			.order_by_asc(device::Column::CreatedAt)
			.all(self.conn())
			.await?)
	}

	/// A device visible to `user`, or [`DeviceError::NotFound`].
	pub async fn get(&self, user: &AuthUser, device_id: &str) -> DeviceResult<device::Model> {
		device::Entity::find_visible_to(user)
			.filter(device::Column::Id.eq(device_id))
			.one(self.conn())
			.await?
			.ok_or(DeviceError::NotFound)
	}

	/// The credential row of a device, if it still has one (revocation removes it).
	pub async fn credential(
		&self,
		device_id: &str,
	) -> DeviceResult<Option<device_credential::Model>> {
		find_credential(self.conn(), device_id).await
	}

	/// Registers a device for `user` and mints its credential. The name defaults
	/// to `"<Username>'s <Kind>"` and is numbered when the user already has a
	/// device by that name.
	pub async fn create_device(
		&self,
		user: &AuthUser,
		kind: DeviceKind,
		name: Option<String>,
	) -> DeviceResult<(device::Model, IssuedCredential)> {
		authorize_creation(user, kind)?;

		let txn = self.conn.begin().await?;
		let name = match name {
			Some(name) => {
				let name = validate_name(&name)?;
				if existing_names(&txn, &user.id).await?.contains(&name) {
					return Err(DeviceError::InvalidName(format!(
						"a device named {name:?} already exists"
					)));
				}
				name
			},
			None => {
				let base = format!("{}'s {}", user.username, kind_label(kind));
				numbered_name(&existing_names(&txn, &user.id).await?, &base)
			},
		};

		let device = device::ActiveModel {
			user_id: Set(user.id.clone()),
			name: Set(name),
			kind: Set(kind),
			..Default::default()
		}
		.insert(&txn)
		.await?;
		let issued = mint_credential(&txn, &device).await?;
		txn.commit().await?;

		Ok((device, issued))
	}

	/// Discards the device's current credential and mints a replacement.
	pub async fn rotate_credential(
		&self,
		user: &AuthUser,
		device_id: &str,
	) -> DeviceResult<IssuedCredential> {
		let device = self.get(user, device_id).await?;
		if device.is_revoked() {
			return Err(DeviceError::Revoked);
		}

		let txn = self.conn.begin().await?;
		discard_credentials(&txn, &device).await?;
		let issued = mint_credential(&txn, &device).await?;
		txn.commit().await?;

		Ok(issued)
	}

	/// Invalidates the device's credential and marks the device revoked. The row
	/// is kept so the device's history remains visible; revoking twice is a no-op.
	pub async fn revoke(&self, user: &AuthUser, device_id: &str) -> DeviceResult<device::Model> {
		let device = self.get(user, device_id).await?;
		if device.is_revoked() {
			return Ok(device);
		}

		let txn = self.conn.begin().await?;
		discard_credentials(&txn, &device).await?;
		let mut active: device::ActiveModel = device.into();
		active.revoked_at = Set(Some(Utc::now().into()));
		let device = active.update(&txn).await?;
		txn.commit().await?;

		Ok(device)
	}

	/// Renames the device and the credential row that carries its name.
	pub async fn rename(
		&self,
		user: &AuthUser,
		device_id: &str,
		name: &str,
	) -> DeviceResult<device::Model> {
		let device = self.get(user, device_id).await?;
		let name = validate_name(name)?;
		if name == device.name {
			return Ok(device);
		}
		let taken = existing_names(self.conn(), &device.user_id).await?;
		if taken.contains(&name) {
			return Err(DeviceError::InvalidName(format!(
				"a device named {name:?} already exists"
			)));
		}

		let txn = self.conn.begin().await?;
		if let Some(credential) = find_credential(&txn, &device.id).await? {
			match credential.credential_kind {
				DeviceCredentialKind::ApiKey => {
					api_key::Entity::update_many()
						.filter(api_key::Column::UserId.eq(&device.user_id))
						.filter(api_key::Column::ShortToken.eq(&credential.credential_ref))
						.col_expr(api_key::Column::Name, Expr::value(name.clone()))
						.exec(&txn)
						.await?;
				},
				DeviceCredentialKind::LiseurToken => {
					liseur::rename(&txn, &credential.credential_ref, &name).await?;
				},
				DeviceCredentialKind::Session => {},
			}
		}
		let mut active: device::ActiveModel = device.into();
		active.name = Set(name);
		let device = active.update(&txn).await?;
		txn.commit().await?;

		Ok(device)
	}

	pub async fn set_transform_profile(
		&self,
		user: &AuthUser,
		device_id: &str,
		profile: Option<JsonValue>,
	) -> DeviceResult<device::Model> {
		let device = self.get(user, device_id).await?;
		let mut active: device::ActiveModel = device.into();
		active.transform_profile = Set(profile);
		Ok(active.update(self.conn()).await?)
	}

	/// Records that `credential` just authenticated a request over `protocol`.
	///
	/// Updates `last_seen_at` (rate limited to once per [`TOUCH_INTERVAL`]) and,
	/// when a sync `summary` is reported, `last_sync_at` + `last_sync_summary`.
	/// Returns the sighting that was recorded and forwarded to the listener, or
	/// `None` when the credential belongs to no live device or the sighting was
	/// coalesced into the previous one.
	pub async fn touch(
		&self,
		credential: CredentialRef<'_>,
		protocol: DeviceProtocol,
		summary: Option<JsonValue>,
	) -> DeviceResult<Option<DeviceSeen>> {
		let Some((kind, reference)) = credential.lookup_key() else {
			return Ok(None);
		};
		let found = device_credential::Entity::find()
			.filter(device_credential::Column::CredentialKind.eq(kind))
			.filter(device_credential::Column::CredentialRef.eq(reference))
			.find_also_related(device::Entity)
			.one(self.conn())
			.await?;
		let Some((_, Some(device))) = found else {
			return Ok(None);
		};
		if device.is_revoked() {
			return Ok(None);
		}

		let now = Utc::now();
		let recently_seen = device
			.last_seen_at
			.is_some_and(|seen| now - seen.with_timezone(&Utc) < TOUCH_INTERVAL);
		if summary.is_none() && recently_seen {
			return Ok(None);
		}

		let now: DateTimeWithTimeZone = now.into();
		let mut update = device::Entity::update_many()
			.filter(device::Column::Id.eq(&device.id))
			.col_expr(device::Column::LastSeenAt, Expr::value(Some(now)));
		if let Some(summary) = summary {
			update = update
				.col_expr(device::Column::LastSyncAt, Expr::value(Some(now)))
				.col_expr(device::Column::LastSyncSummary, Expr::value(Some(summary)));
		}
		update.exec(self.conn()).await?;

		let seen = DeviceSeen {
			device_id: device.id,
			user_id: device.user_id,
			protocol,
		};
		if let Some(listener) = &self.on_seen {
			listener(seen.clone());
		}
		Ok(Some(seen))
	}

	/// The endpoints to configure on a device, with the secret redacted to a hint.
	pub async fn endpoints(
		&self,
		user: &AuthUser,
		device_id: &str,
		origin: &RequestOrigin,
	) -> DeviceResult<Vec<Endpoint>> {
		let device = self.get(user, device_id).await?;
		let username = self.owner_username(user, &device).await?;
		let hint = match self.credential(&device.id).await? {
			Some(credential) => secret_hint(&credential),
			None => "(no credential)".to_string(),
		};
		Ok(Endpoint::for_kind(device.kind, origin, &username, &hint))
	}

	/// The endpoints for a credential that was just minted, secret included.
	pub async fn endpoints_with_secret(
		&self,
		user: &AuthUser,
		device: &device::Model,
		issued: &IssuedCredential,
		origin: &RequestOrigin,
	) -> DeviceResult<Vec<Endpoint>> {
		let username = self.owner_username(user, device).await?;
		Ok(Endpoint::for_kind(
			device.kind,
			origin,
			&username,
			&issued.secret,
		))
	}

	async fn owner_username(
		&self,
		user: &AuthUser,
		device: &device::Model,
	) -> DeviceResult<String> {
		if device.user_id == user.id {
			return Ok(user.username.clone());
		}
		Ok(user::Entity::find_by_id(&device.user_id)
			.select_only()
			.column(user::Column::Username)
			.into_tuple::<String>()
			.one(self.conn())
			.await?
			.unwrap_or_default())
	}
}

/// The redacted form of a stored credential: the visible part of an API key,
/// nothing of a liseur secret.
pub fn secret_hint(credential: &device_credential::Model) -> String {
	match credential.credential_kind {
		DeviceCredentialKind::ApiKey => {
			format!("{API_KEY_PREFIX}_{}_…", credential.credential_ref)
		},
		DeviceCredentialKind::LiseurToken => "liseur-…".to_string(),
		DeviceCredentialKind::Session => "(session)".to_string(),
	}
}

fn kind_label(kind: DeviceKind) -> &'static str {
	match kind {
		DeviceKind::Kobo => "Kobo",
		DeviceKind::Koreader => "KOReader",
		DeviceKind::Mihon => "Mihon",
		DeviceKind::Komelia => "Komelia",
		DeviceKind::Liseur => "Liseur",
		DeviceKind::Opds => "OPDS reader",
		DeviceKind::Api => "API client",
		DeviceKind::Web => "Browser",
	}
}

fn validate_name(name: &str) -> DeviceResult<String> {
	let name = name.trim();
	if name.is_empty() {
		return Err(DeviceError::InvalidName("name must not be empty".to_string()));
	}
	if name.chars().count() > MAX_NAME_CHARS {
		return Err(DeviceError::InvalidName(format!(
			"name must be at most {MAX_NAME_CHARS} characters"
		)));
	}
	Ok(name.to_string())
}

fn numbered_name(taken: &HashSet<String>, base: &str) -> String {
	if !taken.contains(base) {
		return base.to_string();
	}
	(2..)
		.map(|n| format!("{base} {n}"))
		.find(|candidate| !taken.contains(candidate))
		.expect("an unused numbered name exists")
}

fn authorize_creation(user: &AuthUser, kind: DeviceKind) -> DeviceResult<()> {
	if user.is_server_owner {
		return Ok(());
	}
	let missing = required_permissions(kind)
		.into_iter()
		.any(|permission| !user.permissions.contains(&permission));
	if missing {
		return Err(DeviceError::Forbidden);
	}
	Ok(())
}

async fn find_credential<C: ConnectionTrait>(
	conn: &C,
	device_id: &str,
) -> DeviceResult<Option<device_credential::Model>> {
	Ok(device_credential::Entity::find()
		.filter(device_credential::Column::DeviceId.eq(device_id))
		.one(conn)
		.await?)
}

async fn existing_names<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> DeviceResult<HashSet<String>> {
	Ok(device::Entity::find()
		.filter(device::Column::UserId.eq(user_id))
		.select_only()
		.column(device::Column::Name)
		.into_tuple::<String>()
		.all(conn)
		.await?
		.into_iter()
		.collect())
}

/// Mints the credential a device of its kind uses and records it on the device.
async fn mint_credential<C: ConnectionTrait>(
	conn: &C,
	device: &device::Model,
) -> DeviceResult<IssuedCredential> {
	let kind = credential_kind_for(device.kind);
	let protocol = protocol_for(device.kind);
	let (credential_ref, secret) = match kind {
		DeviceCredentialKind::ApiKey => {
			mint_api_key(
				conn,
				&device.user_id,
				&device.name,
				api_key_permissions_for(device.kind),
			)
			.await?
		},
		DeviceCredentialKind::LiseurToken => {
			liseur::mint(conn, &device.user_id, &device.id, &device.name).await?
		},
		DeviceCredentialKind::Session => {
			return Err(DeviceError::Credential(
				"session credentials are bound by a login, not minted".to_string(),
			))
		},
	};

	device_credential::ActiveModel {
		device_id: Set(device.id.clone()),
		protocol: Set(protocol),
		credential_kind: Set(kind),
		credential_ref: Set(credential_ref.clone()),
		..Default::default()
	}
	.insert(conn)
	.await?;

	Ok(IssuedCredential {
		kind,
		protocol,
		credential_ref,
		secret,
	})
}

/// Invalidates every credential of the device at its source and drops the
/// credential rows, so `touch` can no longer resolve them.
async fn discard_credentials<C: ConnectionTrait>(
	conn: &C,
	device: &device::Model,
) -> DeviceResult<()> {
	let credentials = device_credential::Entity::find()
		.filter(device_credential::Column::DeviceId.eq(&device.id))
		.all(conn)
		.await?;
	for credential in credentials {
		match credential.credential_kind {
			DeviceCredentialKind::ApiKey => {
				api_key::Entity::delete_many()
					.filter(api_key::Column::UserId.eq(&device.user_id))
					.filter(api_key::Column::ShortToken.eq(&credential.credential_ref))
					.exec(conn)
					.await?;
			},
			DeviceCredentialKind::LiseurToken => {
				liseur::revoke(conn, &credential.credential_ref).await?;
			},
			DeviceCredentialKind::Session => {
				session::Entity::delete_many()
					.filter(session::Column::SessionId.eq(&credential.credential_ref))
					.exec(conn)
					.await?;
			},
		}
	}
	device_credential::Entity::delete_many()
		.filter(device_credential::Column::DeviceId.eq(&device.id))
		.exec(conn)
		.await?;
	Ok(())
}
