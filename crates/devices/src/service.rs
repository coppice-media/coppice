use std::{
	collections::{HashMap, HashSet},
	sync::{Arc, Mutex},
	time::Duration as StdDuration,
};

use chrono::Utc;
use models::txn::begin_write;
use models::{
	entity::{
		api_key, device, device_credential, device_entitlement_delta, library,
		library_exclusion, media, series, session,
		user::{self, AuthUser},
	},
	shared::{
		api_key::API_KEY_PREFIX,
		enums::{DeviceCredentialKind, DeviceKind, DeviceProtocol},
	},
};
use sea_orm::{
	prelude::*, sea_query::OnConflict, sea_query::Query, ActiveValue::Set,
	DatabaseConnection, QueryOrder, QuerySelect,
};
use serde_json::Value as JsonValue;
use stump_api_types::RequestOrigin;
use tokio::time::Instant;

use crate::{
	credential::{
		api_key_permissions_for, credential_kind_for, liseur, mint_api_key, protocol_for,
		required_permissions, CredentialRef, IssuedCredential,
	},
	endpoint::Endpoint,
	error::{DeviceError, DeviceResult},
	event::DeviceSeen,
	kindle::{normalize_kindle_email, KindleSendSummary},
	scope::LibraryScope,
};

/// Receives a [`DeviceSeen`] each time [`DeviceService::touch`] updates a device.
pub type SeenListener = Arc<dyn Fn(DeviceSeen) + Send + Sync>;

/// `last_seen_at` is written at most once per interval per credential unless a
/// sync summary is reported: protocol clients burst many requests per sync
/// (page streams, cover fetches), and none of those may cost a SQLite write.
/// The sighting is coalesced in memory, so a coalesced request does not even
/// read the credential row.
pub const TOUCH_INTERVAL: StdDuration = StdDuration::from_secs(60);

const MAX_NAME_CHARS: usize = 128;

/// The credential lookup key of a sighting written within [`TOUCH_INTERVAL`].
type SeenKey = (DeviceCredentialKind, String);

#[derive(Clone)]
pub struct DeviceService {
	conn: Arc<DatabaseConnection>,
	on_seen: Option<SeenListener>,
	/// When each credential's `last_seen_at` was last written. Shared by every
	/// clone so the debounce is process-wide.
	recently_seen: Arc<Mutex<HashMap<SeenKey, Instant>>>,
}

impl DeviceService {
	pub fn new(conn: Arc<DatabaseConnection>) -> Self {
		Self {
			conn,
			on_seen: None,
			recently_seen: Arc::default(),
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
	pub async fn get(
		&self,
		user: &AuthUser,
		device_id: &str,
	) -> DeviceResult<device::Model> {
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

		let txn = begin_write(&self.conn).await?;
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

		let txn = begin_write(&self.conn).await?;
		discard_credentials(&txn, &device).await?;
		let issued = mint_credential(&txn, &device).await?;
		txn.commit().await?;

		Ok(issued)
	}

	/// Invalidates the device's credential and marks the device revoked. The row
	/// is kept so the device's history remains visible; revoking twice is a no-op.
	pub async fn revoke(
		&self,
		user: &AuthUser,
		device_id: &str,
	) -> DeviceResult<device::Model> {
		let device = self.get(user, device_id).await?;
		if device.is_revoked() {
			return Ok(device);
		}

		let txn = begin_write(&self.conn).await?;
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

		let txn = begin_write(&self.conn).await?;
		if let Some(credential) = find_credential(&txn, &device.id).await? {
			match credential.credential_kind {
				DeviceCredentialKind::ApiKey => {
					api_key::Entity::update_many()
						.filter(api_key::Column::UserId.eq(&device.user_id))
						.filter(
							api_key::Column::ShortToken.eq(&credential.credential_ref),
						)
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

	/// Sets the device's Amazon *Send to Kindle* address, or clears it with
	/// `None`. The address is validated by
	/// [`normalize_kindle_email`](crate::kindle::normalize_kindle_email)
	/// before it is stored; a device without one cannot be sent to.
	pub async fn set_kindle_email(
		&self,
		user: &AuthUser,
		device_id: &str,
		email: Option<&str>,
	) -> DeviceResult<device::Model> {
		let device = self.get(user, device_id).await?;
		let email = email.map(normalize_kindle_email).transpose()?;
		let mut active: device::ActiveModel = device.into();
		active.kindle_email = Set(email);
		Ok(active.update(self.conn()).await?)
	}

	/// Records a delivery Stump *initiated*, i.e. a book mailed to a device's
	/// Kindle address.
	///
	/// `last_seen_at` deliberately does not move: nothing authenticated, so
	/// the device was not seen — it was written to. `last_sync_at` and
	/// `last_sync_summary` do, which is exactly what a protocol's completed
	/// sync means, and the summary names the lane
	/// ([`KINDLE_EMAIL_PROTOCOL`](crate::kindle::KINDLE_EMAIL_PROTOCOL))
	/// because no `DeviceProtocol` describes it.
	pub async fn record_delivery(
		&self,
		device_id: &str,
		summary: &KindleSendSummary,
	) -> DeviceResult<()> {
		let now: DateTimeWithTimeZone = Utc::now().into();
		let summary = JsonValue::from(summary);
		device::Entity::update_many()
			.filter(device::Column::Id.eq(device_id))
			.col_expr(device::Column::LastSyncAt, Expr::value(Some(now)))
			.col_expr(device::Column::LastSyncSummary, Expr::value(Some(summary)))
			.exec(self.conn())
			.await?;
		Ok(())
	}

	/// Restricts the device to `scope`'s libraries, or clears the restriction
	/// with [`LibraryScope::Inherit`].
	///
	/// The caller must be the device's user or the server owner (enforced by
	/// [`Self::get`]). Every id must name a library the device's *owner* can
	/// see: the scope is intersected with the owner's visibility on every
	/// query anyway, so an id they cannot see would be silently inert — a
	/// rejection says so instead.
	///
	/// For a Kobo, the books that changed side are recorded as entitlement
	/// deltas so the next sync can hand the device a removal or a new
	/// entitlement (see [`device_entitlement_delta`]). Other protocols
	/// re-query on every request and need no bookkeeping.
	pub async fn set_library_scope(
		&self,
		user: &AuthUser,
		device_id: &str,
		scope: LibraryScope,
	) -> DeviceResult<device::Model> {
		let device = self.get(user, device_id).await?;
		let previous = LibraryScope::of(&device);
		let owner_libraries = visible_library_ids(self.conn(), &device.user_id).await?;

		if let Some(ids) = scope.library_ids() {
			if let Some(unknown) = ids.iter().find(|id| !owner_libraries.contains(*id)) {
				return Err(DeviceError::InvalidScope(format!(
					"library {unknown:?} does not exist or is not visible to the device's user"
				)));
			}
		}

		let transition = ScopeTransition::between(&previous, &scope, &owner_libraries);
		let txn = begin_write(&self.conn).await?;
		let mut active: device::ActiveModel = device.clone().into();
		active.library_scope = Set(scope.to_column_value());
		let device = active.update(&txn).await?;
		if device.kind == DeviceKind::Kobo {
			record_entitlement_deltas(&txn, &device.id, &transition).await?;
		}
		txn.commit().await?;

		// The next request from this device must re-read the row rather than
		// answer from the coalesced sighting, so the new scope is in force
		// immediately.
		self.forget_sightings(&device.id).await?;

		Ok(device)
	}

	/// Drops the in-memory sighting debounce for every credential of the
	/// device. Nothing caches the scope itself, so this is all that stands
	/// between a scope write and the device's next request seeing it.
	async fn forget_sightings(&self, device_id: &str) -> DeviceResult<()> {
		let credentials = device_credential::Entity::find()
			.filter(device_credential::Column::DeviceId.eq(device_id))
			.all(self.conn())
			.await?;
		let mut recently_seen = self.lock_recently_seen();
		for credential in credentials {
			recently_seen
				.remove(&(credential.credential_kind, credential.credential_ref));
		}
		Ok(())
	}

	/// The live (not revoked) device bound to `credential`, or `None` when the
	/// reference is not a device credential or the device was revoked. The
	/// lookup every protocol adapter uses to reach per-device state such as
	/// the transform profile.
	pub async fn device_for_credential(
		&self,
		credential: CredentialRef<'_>,
	) -> DeviceResult<Option<device::Model>> {
		let Some(key) = credential.lookup_key() else {
			return Ok(None);
		};
		self.device_for_key(&key).await
	}

	async fn device_for_key(&self, key: &SeenKey) -> DeviceResult<Option<device::Model>> {
		let found = device_credential::Entity::find()
			.filter(device_credential::Column::CredentialKind.eq(key.0))
			.filter(device_credential::Column::CredentialRef.eq(&key.1))
			.find_also_related(device::Entity)
			.one(self.conn())
			.await?;
		Ok(match found {
			Some((_, Some(device))) if !device.is_revoked() => Some(device),
			_ => None,
		})
	}

	/// Resolves the device behind `credential` and records the sighting in a
	/// single device lookup.
	///
	/// The auth path needs the device's library scope on *every* request, so
	/// the row is always read; only the `last_seen_at` write stays coalesced
	/// to one per [`TOUCH_INTERVAL`], and a failed write is logged rather
	/// than propagated — last-seen bookkeeping must never fail a request.
	/// An `Err` therefore means only one thing: the device behind the
	/// credential could not be determined, so the caller must not guess at
	/// what the device may see. `Ok(None)` is a credential that belongs to no
	/// live device.
	pub async fn authenticate(
		&self,
		credential: CredentialRef<'_>,
		protocol: DeviceProtocol,
	) -> DeviceResult<Option<DeviceAuth>> {
		let Some(key) = credential.lookup_key() else {
			return Ok(None);
		};
		let Some(device) = self.device_for_key(&key).await? else {
			return Ok(None);
		};
		if !self.seen_within_interval(&key) {
			if let Err(error) = self.record_sighting(&device, key, protocol, None).await {
				tracing::warn!(?error, ?protocol, "Failed to record device sighting");
			}
		}
		Ok(Some(DeviceAuth {
			device_id: device.id.clone(),
			library_scope: LibraryScope::of(&device),
		}))
	}

	/// Records that `credential` just authenticated a request over `protocol`.
	///
	/// Updates `last_seen_at` (coalesced in memory to one write per
	/// [`TOUCH_INTERVAL`] per credential) and, when a sync `summary` is
	/// reported, `last_sync_at` + `last_sync_summary`; a summary always lands.
	/// Returns the sighting that was recorded and forwarded to the listener, or
	/// `None` when the credential belongs to no live device or the sighting was
	/// coalesced into the previous one.
	pub async fn touch(
		&self,
		credential: CredentialRef<'_>,
		protocol: DeviceProtocol,
		summary: Option<JsonValue>,
	) -> DeviceResult<Option<DeviceSeen>> {
		let Some(key) = credential.lookup_key() else {
			return Ok(None);
		};
		if summary.is_none() && self.seen_within_interval(&key) {
			return Ok(None);
		}
		let Some(device) = self.device_for_key(&key).await? else {
			return Ok(None);
		};
		Ok(Some(
			self.record_sighting(&device, key, protocol, summary)
				.await?,
		))
	}

	async fn record_sighting(
		&self,
		device: &device::Model,
		key: SeenKey,
		protocol: DeviceProtocol,
		summary: Option<JsonValue>,
	) -> DeviceResult<DeviceSeen> {
		let now: DateTimeWithTimeZone = Utc::now().into();
		let mut update = device::Entity::update_many()
			.filter(device::Column::Id.eq(&device.id))
			.col_expr(device::Column::LastSeenAt, Expr::value(Some(now)));
		if let Some(summary) = summary {
			update = update
				.col_expr(device::Column::LastSyncAt, Expr::value(Some(now)))
				.col_expr(device::Column::LastSyncSummary, Expr::value(Some(summary)));
		}
		update.exec(self.conn()).await?;
		self.mark_seen(key);

		let seen = DeviceSeen {
			device_id: device.id.clone(),
			user_id: device.user_id.clone(),
			protocol,
			first_seen: device.last_seen_at.is_none(),
		};
		if let Some(listener) = &self.on_seen {
			listener(seen.clone());
		}
		Ok(seen)
	}

	/// Whether `key`'s sighting was written less than [`TOUCH_INTERVAL`] ago.
	/// An expired entry is dropped when it is looked up, so the map holds at
	/// most one entry per credential that authenticated since start-up.
	fn seen_within_interval(&self, key: &SeenKey) -> bool {
		let mut recently_seen = self.lock_recently_seen();
		match recently_seen.get(key) {
			Some(written_at) if written_at.elapsed() < TOUCH_INTERVAL => true,
			Some(_) => {
				recently_seen.remove(key);
				false
			},
			None => false,
		}
	}

	fn mark_seen(&self, key: SeenKey) {
		self.lock_recently_seen().insert(key, Instant::now());
	}

	fn lock_recently_seen(&self) -> std::sync::MutexGuard<'_, HashMap<SeenKey, Instant>> {
		self.recently_seen
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
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

/// The device facts one authenticated request needs: which device the
/// credential belongs to and what it may see.
#[derive(Clone, Debug)]
pub struct DeviceAuth {
	pub device_id: String,
	pub library_scope: LibraryScope,
}

/// Which libraries a scope change added to and removed from a device's view,
/// already intersected with the owner's visibility.
#[derive(Debug, Default, PartialEq, Eq)]
struct ScopeTransition {
	entered: Vec<String>,
	left: Vec<String>,
}

impl ScopeTransition {
	/// `owner_libraries` is what the device's user can see; `Inherit` expands
	/// to exactly that, so both sides are compared as concrete library sets
	/// and a widening on one side is never mistaken for the other.
	fn between(
		previous: &LibraryScope,
		next: &LibraryScope,
		owner_libraries: &HashSet<String>,
	) -> Self {
		let before = resolve_scope(previous, owner_libraries);
		let after = resolve_scope(next, owner_libraries);
		Self {
			entered: after.difference(&before).cloned().collect(),
			left: before.difference(&after).cloned().collect(),
		}
	}

	fn is_empty(&self) -> bool {
		self.entered.is_empty() && self.left.is_empty()
	}
}

fn resolve_scope(
	scope: &LibraryScope,
	owner_libraries: &HashSet<String>,
) -> HashSet<String> {
	match scope.library_ids() {
		None => owner_libraries.clone(),
		Some(ids) => ids
			.iter()
			.filter(|id| owner_libraries.contains(*id))
			.cloned()
			.collect(),
	}
}

/// The libraries `user_id` can see, i.e. every library they have not excluded.
async fn visible_library_ids<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> DeviceResult<HashSet<String>> {
	Ok(library::Entity::find()
		.select_only()
		.column(library::Column::Id)
		.filter(
			library::Column::Id.not_in_subquery(
				Query::select()
					.column(library_exclusion::Column::LibraryId)
					.from(library_exclusion::Entity)
					.and_where(library_exclusion::Column::UserId.eq(user_id))
					.to_owned(),
			),
		)
		.into_tuple::<String>()
		.all(conn)
		.await?
		.into_iter()
		.collect())
}

/// SQLite caps the parameters of one statement; a scope change can move a
/// whole library's worth of books, so both the read and the write go in
/// chunks well under that cap.
const DELTA_CHUNK: usize = 256;

/// Records what `transition` did to the device's entitlements: one row per
/// affected book, the latest transition winning, so a book that leaves and
/// rejoins before the next sync collapses to "still there".
async fn record_entitlement_deltas<C: ConnectionTrait>(
	conn: &C,
	device_id: &str,
	transition: &ScopeTransition,
) -> DeviceResult<()> {
	if transition.is_empty() {
		return Ok(());
	}
	let now: DateTimeWithTimeZone = Utc::now().into();
	for (libraries, removed) in [(&transition.left, true), (&transition.entered, false)] {
		for library_ids in libraries.chunks(DELTA_CHUNK) {
			let media_ids = media::Entity::find()
				.select_only()
				.column(media::Column::Id)
				.inner_join(series::Entity)
				.filter(
					series::Column::LibraryId
						.is_in(library_ids.iter().map(String::as_str)),
				)
				.filter(media::Column::DeletedAt.is_null())
				.into_tuple::<String>()
				.all(conn)
				.await?;
			for chunk in media_ids.chunks(DELTA_CHUNK) {
				device_entitlement_delta::Entity::insert_many(chunk.iter().map(
					|media_id| device_entitlement_delta::ActiveModel {
						device_id: Set(device_id.to_owned()),
						media_id: Set(media_id.clone()),
						removed: Set(removed),
						created_at: Set(now),
					},
				))
				.on_conflict(
					OnConflict::columns([
						device_entitlement_delta::Column::DeviceId,
						device_entitlement_delta::Column::MediaId,
					])
					.update_columns([
						device_entitlement_delta::Column::Removed,
						device_entitlement_delta::Column::CreatedAt,
					])
					.to_owned(),
				)
				.exec(conn)
				.await?;
			}
		}
	}
	Ok(())
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
		DeviceKind::Abs => "Audiobookshelf client",
		DeviceKind::Api => "API client",
		DeviceKind::Web => "Browser",
		DeviceKind::Worker => "Worker",
	}
}

fn validate_name(name: &str) -> DeviceResult<String> {
	let name = name.trim();
	if name.is_empty() {
		return Err(DeviceError::InvalidName(
			"name must not be empty".to_string(),
		));
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
