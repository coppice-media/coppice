use std::sync::Arc;

use chrono::{Duration, Utc};
use models::{
	entity::{
		api_key, device, device_credential, device_entitlement_delta, liseur_sync_token,
		user::AuthUser,
	},
	shared::{
		api_key::APIKeyPermissions,
		enums::{DeviceCredentialKind, DeviceKind, DeviceProtocol, UserPermission},
	},
};
use parking_lot::Mutex;
use sea_orm::{prelude::*, ActiveValue::Set, DatabaseConnection, QueryOrder};
use serde_json::json;
use stump_api_types::RequestOrigin;
use tests::{db::test_database, fake_data};

use crate::{
	credential::liseur, service::TOUCH_INTERVAL, CredentialRef, DeviceError, DeviceSeen,
	DeviceService, Endpoint, KindleSendSummary, LibraryScope, KINDLE_EMAIL_PROTOCOL,
};

async fn setup() -> (Arc<DatabaseConnection>, AuthUser) {
	let conn = Arc::new(test_database().await);
	let user = fake_data::User::new("al").insert(conn.as_ref()).await;
	let auth_user = AuthUser {
		id: user.id,
		username: user.username,
		is_server_owner: true,
		..Default::default()
	};
	(conn, auth_user)
}

fn reader(id: &str, permissions: Vec<UserPermission>) -> AuthUser {
	AuthUser {
		id: id.to_string(),
		username: "reader".to_string(),
		is_server_owner: false,
		permissions,
		..Default::default()
	}
}

fn origin() -> RequestOrigin {
	RequestOrigin::new("stump.test".to_string(), "https".to_string())
}

#[tokio::test]
async fn create_device_mints_narrowed_api_key_and_default_name() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());

	let (device, issued) = service
		.create_device(&user, DeviceKind::Kobo, None)
		.await
		.expect("device");

	assert_eq!(device.name, "al's Kobo");
	assert_eq!(device.kind, DeviceKind::Kobo);
	assert_eq!(device.user_id, user.id);
	assert!(device.last_seen_at.is_none());
	assert_eq!(issued.kind, DeviceCredentialKind::ApiKey);
	assert_eq!(issued.protocol, DeviceProtocol::Kobo);
	assert!(issued.secret.starts_with("stump_"));
	assert!(issued.secret.contains(&issued.credential_ref));

	let key = api_key::Entity::find()
		.filter(api_key::Column::ShortToken.eq(&issued.credential_ref))
		.one(conn.as_ref())
		.await
		.expect("query")
		.expect("api key row");
	assert_eq!(key.name, "al's Kobo");
	assert_eq!(key.user_id, user.id);
	assert_eq!(
		key.permissions,
		APIKeyPermissions::Custom(vec![
			UserPermission::AccessKoboSync,
			UserPermission::DownloadFile
		])
	);

	let credential = service
		.credential(&device.id)
		.await
		.expect("query")
		.expect("credential row");
	assert_eq!(credential.credential_kind, DeviceCredentialKind::ApiKey);
	assert_eq!(credential.protocol, DeviceProtocol::Kobo);
	assert_eq!(credential.credential_ref, issued.credential_ref);
}

#[tokio::test]
async fn default_names_are_numbered_on_collision() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);

	let first = service
		.create_device(&user, DeviceKind::Koreader, None)
		.await
		.expect("first");
	let second = service
		.create_device(&user, DeviceKind::Koreader, None)
		.await
		.expect("second");
	let third = service
		.create_device(&user, DeviceKind::Koreader, None)
		.await
		.expect("third");

	assert_eq!(first.0.name, "al's KOReader");
	assert_eq!(second.0.name, "al's KOReader 2");
	assert_eq!(third.0.name, "al's KOReader 3");

	let taken = service
		.create_device(&user, DeviceKind::Koreader, Some("al's KOReader".into()))
		.await;
	assert!(matches!(taken, Err(DeviceError::InvalidName(_))));
	let blank = service
		.create_device(&user, DeviceKind::Koreader, Some("   ".into()))
		.await;
	assert!(matches!(blank, Err(DeviceError::InvalidName(_))));
}

#[tokio::test]
async fn create_device_requires_the_kind_permissions() {
	let (conn, owner) = setup().await;
	let service = DeviceService::new(conn.clone());

	let no_key_access = reader(&owner.id, vec![UserPermission::AccessKoboSync]);
	assert!(matches!(
		service
			.create_device(&no_key_access, DeviceKind::Kobo, None)
			.await,
		Err(DeviceError::Forbidden)
	));

	let no_kobo = reader(
		&owner.id,
		vec![UserPermission::AccessApiKeys, UserPermission::DownloadFile],
	);
	assert!(matches!(
		service
			.create_device(&no_kobo, DeviceKind::Kobo, None)
			.await,
		Err(DeviceError::Forbidden)
	));

	let allowed = reader(
		&owner.id,
		vec![
			UserPermission::AccessApiKeys,
			UserPermission::AccessKoboSync,
			UserPermission::DownloadFile,
		],
	);
	assert!(service
		.create_device(&allowed, DeviceKind::Kobo, None)
		.await
		.is_ok());

	// Liseur tokens are not API keys: no ACCESS_API_KEYS needed
	let liseur_only = reader(&owner.id, vec![]);
	assert!(service
		.create_device(&liseur_only, DeviceKind::Liseur, None)
		.await
		.is_ok());
}

#[tokio::test]
async fn liseur_device_mints_bound_device_token() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());

	let (device, issued) = service
		.create_device(
			&user,
			DeviceKind::Liseur,
			Some("Kobo Clara (NickelStump)".into()),
		)
		.await
		.expect("device");

	assert_eq!(issued.kind, DeviceCredentialKind::LiseurToken);
	assert_eq!(issued.protocol, DeviceProtocol::Liseur);
	assert!(issued.secret.starts_with("liseur-"));

	let token = liseur_sync_token::Entity::find_by_id(issued.credential_ref.clone())
		.one(conn.as_ref())
		.await
		.expect("query")
		.expect("token row");
	assert_eq!(token.device_id, device.id);
	assert_eq!(token.user_id, user.id);
	assert_eq!(token.secret_hash, liseur::hash_secret(&issued.secret));
	assert_eq!(token.name.as_deref(), Some("Kobo Clara (NickelStump)"));
	assert_eq!(token.token_kind.as_deref(), Some("device"));
	assert!(token.revoked_at.is_none());
	let scopes: Vec<String> = serde_json::from_str(&token.scopes).expect("scopes json");
	assert_eq!(scopes, ["sync", "read-insights", "library-read"]);
}

#[tokio::test]
async fn rotate_replaces_the_api_key() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());
	let (device, first) = service
		.create_device(&user, DeviceKind::Opds, None)
		.await
		.expect("device");

	let second = service
		.rotate_credential(&user, &device.id)
		.await
		.expect("rotated");
	assert_ne!(first.secret, second.secret);
	assert_ne!(first.credential_ref, second.credential_ref);

	let keys = api_key::Entity::find()
		.filter(api_key::Column::UserId.eq(&user.id))
		.all(conn.as_ref())
		.await
		.expect("keys");
	assert_eq!(keys.len(), 1);
	assert_eq!(keys[0].short_token, second.credential_ref);

	// the old key no longer resolves to the device, the new one does
	assert!(service
		.touch(
			CredentialRef::ApiKey(&first.secret),
			DeviceProtocol::Opds,
			None
		)
		.await
		.expect("touch")
		.is_none());
	assert!(service
		.touch(
			CredentialRef::ApiKey(&second.secret),
			DeviceProtocol::Opds,
			None
		)
		.await
		.expect("touch")
		.is_some());
}

#[tokio::test]
async fn revoke_deletes_credentials_and_blocks_rotation() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());
	let (device, issued) = service
		.create_device(&user, DeviceKind::Mihon, None)
		.await
		.expect("device");
	let (liseur_device, liseur_issued) = service
		.create_device(&user, DeviceKind::Liseur, None)
		.await
		.expect("liseur device");

	let revoked = service.revoke(&user, &device.id).await.expect("revoked");
	assert!(revoked.revoked_at.is_some());
	let revoked_liseur = service
		.revoke(&user, &liseur_device.id)
		.await
		.expect("revoked");
	assert!(revoked_liseur.revoked_at.is_some());

	assert!(api_key::Entity::find()
		.filter(api_key::Column::ShortToken.eq(&issued.credential_ref))
		.one(conn.as_ref())
		.await
		.expect("query")
		.is_none());
	let token =
		liseur_sync_token::Entity::find_by_id(liseur_issued.credential_ref.clone())
			.one(conn.as_ref())
			.await
			.expect("query")
			.expect("token row kept for audit");
	assert!(token.revoked_at.is_some());
	assert!(service
		.credential(&device.id)
		.await
		.expect("query")
		.is_none());
	assert!(service
		.credential(&liseur_device.id)
		.await
		.expect("query")
		.is_none());

	assert!(matches!(
		service.rotate_credential(&user, &device.id).await,
		Err(DeviceError::Revoked)
	));
	// revoking again is a no-op
	let again = service.revoke(&user, &device.id).await.expect("idempotent");
	assert_eq!(again.revoked_at, revoked.revoked_at);
	assert!(service
		.touch(
			CredentialRef::ApiKey(&issued.secret),
			DeviceProtocol::Komga,
			None
		)
		.await
		.expect("touch")
		.is_none());
}

#[tokio::test]
async fn rename_updates_device_and_credential_names() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());
	let (device, issued) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");
	let (liseur_device, liseur_issued) = service
		.create_device(&user, DeviceKind::Liseur, None)
		.await
		.expect("liseur device");

	let renamed = service
		.rename(&user, &device.id, "  Pixel Komelia ")
		.await
		.expect("renamed");
	assert_eq!(renamed.name, "Pixel Komelia");
	let key = api_key::Entity::find()
		.filter(api_key::Column::ShortToken.eq(&issued.credential_ref))
		.one(conn.as_ref())
		.await
		.expect("query")
		.expect("api key");
	assert_eq!(key.name, "Pixel Komelia");

	service
		.rename(&user, &liseur_device.id, "Boox")
		.await
		.expect("renamed");
	let token =
		liseur_sync_token::Entity::find_by_id(liseur_issued.credential_ref.clone())
			.one(conn.as_ref())
			.await
			.expect("query")
			.expect("token");
	assert_eq!(token.name.as_deref(), Some("Boox"));

	assert!(matches!(
		service.rename(&user, &device.id, "Boox").await,
		Err(DeviceError::InvalidName(_))
	));
}

#[tokio::test]
async fn transform_profile_round_trips() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, _) = service
		.create_device(&user, DeviceKind::Kobo, None)
		.await
		.expect("device");

	let profile = json!({ "kepub": { "hyphenate": false }, "max_width": 1264 });
	let updated = service
		.set_transform_profile(&user, &device.id, Some(profile.clone()))
		.await
		.expect("updated");
	assert_eq!(updated.transform_profile, Some(profile));

	let cleared = service
		.set_transform_profile(&user, &device.id, None)
		.await
		.expect("cleared");
	assert_eq!(cleared.transform_profile, None);
}

#[tokio::test]
async fn touch_records_sighting_and_sync_summary() {
	let (conn, user) = setup().await;
	let seen: Arc<Mutex<Vec<DeviceSeen>>> = Arc::default();
	let sink = seen.clone();
	let service = DeviceService::new(conn.clone())
		.with_seen_listener(move |event| sink.lock().push(event));
	let (device, issued) = service
		.create_device(&user, DeviceKind::Kobo, None)
		.await
		.expect("device");

	let first = service
		.touch(
			CredentialRef::ApiKey(&issued.secret),
			DeviceProtocol::Kobo,
			None,
		)
		.await
		.expect("touch")
		.expect("recorded");
	assert_eq!(first.device_id, device.id);
	assert_eq!(first.user_id, user.id);
	assert_eq!(first.protocol, DeviceProtocol::Kobo);
	assert!(first.first_seen);

	let stored = service.get(&user, &device.id).await.expect("device");
	let last_seen = stored.last_seen_at.expect("last seen set");
	assert!(Utc::now() - last_seen.with_timezone(&Utc) < Duration::seconds(5));
	assert!(stored.last_sync_at.is_none());

	// a burst of requests within the interval is coalesced
	let coalesced = service
		.touch(
			CredentialRef::ApiKey(&issued.secret),
			DeviceProtocol::Kobo,
			None,
		)
		.await
		.expect("touch");
	assert!(coalesced.is_none());

	// but a sync summary always lands
	let summary = json!({ "protocol": "kobo", "items": 12 });
	let synced = service
		.touch(
			CredentialRef::ApiKey(&issued.secret),
			DeviceProtocol::Kobo,
			Some(summary.clone()),
		)
		.await
		.expect("touch")
		.expect("recorded");
	assert_eq!(synced.device_id, device.id);
	assert!(!synced.first_seen);
	let stored = service.get(&user, &device.id).await.expect("device");
	assert!(stored.last_sync_at.is_some());
	assert_eq!(stored.last_sync_summary, Some(summary));

	assert_eq!(seen.lock().len(), 2);

	// garbage and unknown credentials are ignored
	assert!(service
		.touch(
			CredentialRef::ApiKey("not a key"),
			DeviceProtocol::Api,
			None
		)
		.await
		.expect("touch")
		.is_none());
	assert!(service
		.touch(
			CredentialRef::ApiKey("stump_unknown_unknownunknownunknownunknown"),
			DeviceProtocol::Api,
			None
		)
		.await
		.expect("touch")
		.is_none());
	assert!(service
		.touch(
			CredentialRef::LiseurToken("missing"),
			DeviceProtocol::Liseur,
			None
		)
		.await
		.expect("touch")
		.is_none());
}

#[tokio::test]
async fn touch_coalesces_sightings_per_credential_for_an_interval() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());
	let (kobo, kobo_key) = service
		.create_device(&user, DeviceKind::Kobo, None)
		.await
		.expect("kobo");
	let (reader, reader_key) = service
		.create_device(&user, DeviceKind::Opds, None)
		.await
		.expect("reader");
	let touch = |secret: String| {
		let service = service.clone();
		async move {
			service
				.touch(CredentialRef::ApiKey(&secret), DeviceProtocol::Kobo, None)
				.await
				.expect("touch")
		}
	};

	assert!(touch(kobo_key.secret.clone()).await.is_some());
	assert!(touch(kobo_key.secret.clone()).await.is_none());
	// every credential is coalesced on its own
	assert!(touch(reader_key.secret.clone()).await.is_some());

	// a coalesced sighting reaches neither the credential nor the device row:
	// clear the stored timestamp and prove the next touch leaves it cleared
	device::Entity::update_many()
		.col_expr(
			device::Column::LastSeenAt,
			Expr::value(None::<DateTimeWithTimeZone>),
		)
		.exec(conn.as_ref())
		.await
		.expect("clear");
	assert!(touch(kobo_key.secret.clone()).await.is_none());
	assert!(service
		.get(&user, &kobo.id)
		.await
		.expect("device")
		.last_seen_at
		.is_none());

	// once the interval has elapsed the sighting is written again
	tokio::time::pause();
	tokio::time::advance(TOUCH_INTERVAL).await;
	tokio::time::resume();
	assert!(touch(kobo_key.secret.clone()).await.is_some());
	assert!(service
		.get(&user, &kobo.id)
		.await
		.expect("device")
		.last_seen_at
		.is_some());
	assert!(service
		.get(&user, &reader.id)
		.await
		.expect("device")
		.last_seen_at
		.is_none());
}

#[tokio::test]
async fn device_for_credential_resolves_live_devices_only() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, issued) = service
		.create_device(&user, DeviceKind::Kobo, None)
		.await
		.expect("device");

	let found = service
		.device_for_credential(CredentialRef::ApiKey(&issued.secret))
		.await
		.expect("lookup")
		.expect("bound device");
	assert_eq!(found.id, device.id);
	assert!(service
		.device_for_credential(CredentialRef::ApiKey("not a key"))
		.await
		.expect("lookup")
		.is_none());

	service.revoke(&user, &device.id).await.expect("revoke");
	assert!(service
		.device_for_credential(CredentialRef::ApiKey(&issued.secret))
		.await
		.expect("lookup")
		.is_none());
}

#[tokio::test]
async fn touch_resolves_liseur_tokens_by_id() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, issued) = service
		.create_device(&user, DeviceKind::Liseur, None)
		.await
		.expect("device");

	let seen = service
		.touch(
			CredentialRef::LiseurToken(&issued.credential_ref),
			DeviceProtocol::Liseur,
			None,
		)
		.await
		.expect("touch")
		.expect("recorded");
	assert_eq!(seen.device_id, device.id);
	assert_eq!(seen.protocol, DeviceProtocol::Liseur);
}

#[tokio::test]
async fn visibility_is_per_user_with_server_owner_override() {
	let (conn, owner) = setup().await;
	let other = fake_data::User::new("bea").insert(conn.as_ref()).await;
	let bea = AuthUser {
		id: other.id,
		username: other.username,
		is_server_owner: false,
		permissions: vec![UserPermission::AccessApiKeys],
		..Default::default()
	};
	let service = DeviceService::new(conn);

	let (owner_device, _) = service
		.create_device(&owner, DeviceKind::Api, None)
		.await
		.expect("owner device");
	let (bea_device, _) = service
		.create_device(&bea, DeviceKind::Web, None)
		.await
		.expect("bea device");

	let bea_sees: Vec<String> = service
		.list(&bea)
		.await
		.expect("list")
		.into_iter()
		.map(|d| d.id)
		.collect();
	assert_eq!(bea_sees, vec![bea_device.id.clone()]);
	assert!(matches!(
		service.get(&bea, &owner_device.id).await,
		Err(DeviceError::NotFound)
	));
	assert!(matches!(
		service.rename(&bea, &owner_device.id, "mine now").await,
		Err(DeviceError::NotFound)
	));

	let owner_sees = service.list(&owner).await.expect("list");
	assert_eq!(owner_sees.len(), 2);
	assert!(service.get(&owner, &bea_device.id).await.is_ok());
}

#[tokio::test]
async fn endpoints_use_secret_once_then_hints() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, issued) = service
		.create_device(&user, DeviceKind::Kobo, None)
		.await
		.expect("device");

	let full = service
		.endpoints_with_secret(&user, &device, &issued, &origin())
		.await
		.expect("endpoints");
	assert_eq!(
		full,
		Endpoint::for_kind(DeviceKind::Kobo, &origin(), "al", &issued.secret)
	);
	assert_eq!(
		full[0].url,
		format!("https://stump.test/kobo/{}", issued.secret)
	);

	let redacted = service
		.endpoints(&user, &device.id, &origin())
		.await
		.expect("endpoints");
	assert_eq!(redacted.len(), 1);
	assert_eq!(
		redacted[0].secret_hint,
		format!("stump_{}_…", issued.credential_ref)
	);
	assert!(!redacted[0].url.contains(&issued.secret));

	service.revoke(&user, &device.id).await.expect("revoked");
	let none = service
		.endpoints(&user, &device.id, &origin())
		.await
		.expect("endpoints");
	assert_eq!(none[0].secret_hint, "(no credential)");
}

#[tokio::test]
async fn protocol_registered_devices_are_listed_without_credentials() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());

	// what the KOReader / OPDS adapters insert on sync
	device::ActiveModel {
		id: Set("koreader-device-id".to_string()),
		user_id: Set(user.id.clone()),
		name: Set("Boox Palma".to_string()),
		kind: Set(DeviceKind::Koreader),
		last_seen_at: Set(Some(Utc::now().into())),
		..Default::default()
	}
	.insert(conn.as_ref())
	.await
	.expect("insert");

	let listed = service.list(&user).await.expect("list");
	assert_eq!(listed.len(), 1);
	assert!(service
		.credential("koreader-device-id")
		.await
		.expect("query")
		.is_none());
	assert!(device_credential::Entity::find()
		.all(conn.as_ref())
		.await
		.expect("query")
		.is_empty());

	// rotating gives a protocol-registered device a real credential
	let issued = service
		.rotate_credential(&user, "koreader-device-id")
		.await
		.expect("rotated");
	assert_eq!(issued.protocol, DeviceProtocol::Koreader);
}

/// A scope is stored as a JSON array, `Inherit` clears it back to NULL, and
/// the round trip through the column preserves what was set.
#[tokio::test]
async fn set_library_scope_restricts_and_clears() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());
	let library = fake_data::Library::default().insert(conn.as_ref()).await;
	let other = fake_data::Library::default().insert(conn.as_ref()).await;
	let (device, _) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");

	assert_eq!(LibraryScope::of(&device), LibraryScope::Inherit);

	let scoped = service
		.set_library_scope(
			&user,
			&device.id,
			LibraryScope::Only(vec![library.id.clone()]),
		)
		.await
		.expect("scope");
	assert_eq!(
		LibraryScope::of(&scoped),
		LibraryScope::Only(vec![library.id.clone()])
	);
	assert_eq!(scoped.library_scope, Some(json!([library.id])));

	// An empty scope is a restriction to nothing, not a reset.
	let empty = service
		.set_library_scope(&user, &device.id, LibraryScope::Only(vec![]))
		.await
		.expect("scope");
	assert_eq!(empty.library_scope, Some(json!([])));

	let inherited = service
		.set_library_scope(&user, &device.id, LibraryScope::Inherit)
		.await
		.expect("scope");
	assert_eq!(inherited.library_scope, None);
	assert!(LibraryScope::of(&inherited).is_inherit());

	// A library the owner cannot see would be inert once intersected, so it
	// is rejected at the write instead of silently ignored.
	let error = service
		.set_library_scope(
			&user,
			&device.id,
			LibraryScope::Only(vec![other.id.clone(), "no-such-library".to_string()]),
		)
		.await
		.expect_err("unknown library must be rejected");
	assert!(matches!(error, DeviceError::InvalidScope(_)), "{error:?}");
	assert_eq!(
		service
			.get(&user, &device.id)
			.await
			.expect("device")
			.library_scope,
		None,
		"a rejected scope must not be persisted"
	);
}

/// A user may scope their own device; another non-owner user may not even see
/// it, and the server owner may scope anyone's.
#[tokio::test]
async fn set_library_scope_is_owner_or_device_user_only() {
	let (conn, owner) = setup().await;
	let service = DeviceService::new(conn.clone());
	let library = fake_data::Library::default().insert(conn.as_ref()).await;
	let reader_row = fake_data::User::new("reader").insert(conn.as_ref()).await;
	let reader_user = reader(&reader_row.id, vec![UserPermission::AccessApiKeys]);
	let stranger_row = fake_data::User::new("stranger").insert(conn.as_ref()).await;
	let stranger_user = reader(&stranger_row.id, vec![UserPermission::AccessApiKeys]);

	let (device, _) = service
		.create_device(&reader_user, DeviceKind::Api, None)
		.await
		.expect("device");

	let scope = LibraryScope::Only(vec![library.id.clone()]);
	assert!(service
		.set_library_scope(&reader_user, &device.id, scope.clone())
		.await
		.is_ok());
	assert!(matches!(
		service
			.set_library_scope(&stranger_user, &device.id, scope.clone())
			.await,
		Err(DeviceError::NotFound)
	));
	assert!(service
		.set_library_scope(&owner, &device.id, LibraryScope::Inherit)
		.await
		.is_ok());
}

/// `authenticate` hands the auth path the device and its scope on every
/// request, including the ones whose `last_seen_at` write is coalesced, and a
/// scope write is in force on the very next request.
#[tokio::test]
async fn authenticate_reports_the_current_scope_every_request() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());
	let library = fake_data::Library::default().insert(conn.as_ref()).await;
	let (device, issued) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");

	let first = service
		.authenticate(CredentialRef::ApiKey(&issued.secret), DeviceProtocol::Komga)
		.await
		.expect("authenticate")
		.expect("device credential");
	assert_eq!(first.device_id, device.id);
	assert_eq!(first.library_scope, LibraryScope::Inherit);

	service
		.set_library_scope(
			&user,
			&device.id,
			LibraryScope::Only(vec![library.id.clone()]),
		)
		.await
		.expect("scope");

	// No re-authentication, no TTL wait: the next request sees the new scope.
	let second = service
		.authenticate(CredentialRef::ApiKey(&issued.secret), DeviceProtocol::Komga)
		.await
		.expect("authenticate")
		.expect("device credential");
	assert_eq!(second.library_scope, LibraryScope::Only(vec![library.id]));

	assert!(service
		.authenticate(
			CredentialRef::ApiKey("stump_nope_nope"),
			DeviceProtocol::Api
		)
		.await
		.expect("authenticate")
		.is_none());
}

/// Narrowing a Kobo's scope tombstones the books that left; widening records
/// the ones that entered. Other kinds re-query per request and record nothing.
#[tokio::test]
async fn kobo_scope_changes_record_entitlement_deltas() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn.clone());

	let kept = fake_data::Library::default().insert(conn.as_ref()).await;
	let dropped = fake_data::Library::default().insert(conn.as_ref()).await;
	let kept_book = book_in(conn.as_ref(), &kept.id).await;
	let dropped_book = book_in(conn.as_ref(), &dropped.id).await;

	let (kobo, _) = service
		.create_device(&user, DeviceKind::Kobo, None)
		.await
		.expect("device");

	// Inherit -> only `kept`: the book in `dropped` left the device's view.
	service
		.set_library_scope(&user, &kobo.id, LibraryScope::Only(vec![kept.id.clone()]))
		.await
		.expect("scope");
	assert_eq!(
		deltas(conn.as_ref(), &kobo.id).await,
		vec![(dropped_book.clone(), true)]
	);

	// Back to inherit: the same book entered, so the row flips rather than
	// accumulating a second one.
	service
		.set_library_scope(&user, &kobo.id, LibraryScope::Inherit)
		.await
		.expect("scope");
	assert_eq!(
		deltas(conn.as_ref(), &kobo.id).await,
		vec![(dropped_book, false)]
	);
	assert!(
		!deltas(conn.as_ref(), &kobo.id)
			.await
			.iter()
			.any(|(id, _)| *id == kept_book),
		"a library that never changed side must record nothing"
	);

	let (komelia, _) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");
	service
		.set_library_scope(
			&user,
			&komelia.id,
			LibraryScope::Only(vec![kept.id.clone()]),
		)
		.await
		.expect("scope");
	assert!(
		deltas(conn.as_ref(), &komelia.id).await.is_empty(),
		"only Kobo needs entitlement bookkeeping"
	);
}

/// A Kindle address round-trips through the column, is trimmed on the way in,
/// and `None` clears it again.
#[tokio::test]
async fn kindle_email_round_trips_and_clears() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, _) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");
	assert_eq!(device.kindle_email, None);

	let stored = service
		.set_kindle_email(&user, &device.id, Some("  al@kindle.com  "))
		.await
		.expect("stored");
	assert_eq!(stored.kindle_email.as_deref(), Some("al@kindle.com"));

	let cleared = service
		.set_kindle_email(&user, &device.id, None)
		.await
		.expect("cleared");
	assert_eq!(cleared.kindle_email, None);
}

/// A malformed address is refused before it is stored, so a send never fails
/// at the SMTP layer for a typo the operator could have been told about.
#[tokio::test]
async fn kindle_email_rejects_malformed_addresses() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, _) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");

	for bad in [
		"",
		"   ",
		"al",
		"al@kindle",
		"@kindle.com",
		"al@@kindle.com",
		"al kindle@kindle.com",
		"al@.kindle.com",
	] {
		let error = service
			.set_kindle_email(&user, &device.id, Some(bad))
			.await
			.expect_err("refused");
		assert!(
			matches!(error, DeviceError::InvalidEmail(_)),
			"{bad:?} was accepted as {error:?}"
		);
	}

	let stored = service.get(&user, &device.id).await.expect("device");
	assert_eq!(stored.kindle_email, None);
}

/// Only the device's user (or the server owner) may set its address; another
/// user cannot even see the device.
#[tokio::test]
async fn kindle_email_is_owner_or_device_user_only() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, _) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");

	let stranger = reader("someone-else", vec![UserPermission::AccessApiKeys]);
	let error = service
		.set_kindle_email(&stranger, &device.id, Some("al@kindle.com"))
		.await
		.expect_err("hidden");
	assert!(matches!(error, DeviceError::NotFound), "{error:?}");
}

/// A send-to-Kindle delivery is a sync summary, not a sighting: Stump mailed
/// the book out, so `last_sync_at` advances and `last_seen_at` must not.
#[tokio::test]
async fn record_delivery_writes_a_sync_summary_without_a_sighting() {
	let (conn, user) = setup().await;
	let service = DeviceService::new(conn);
	let (device, _) = service
		.create_device(&user, DeviceKind::Komelia, None)
		.await
		.expect("device");

	let summary = KindleSendSummary::new(4096, "azw3");
	service
		.record_delivery(&device.id, &summary)
		.await
		.expect("recorded");

	let stored = service.get(&user, &device.id).await.expect("device");
	assert!(stored.last_seen_at.is_none(), "nothing authenticated");
	let synced = stored.last_sync_at.expect("last sync set");
	assert!(Utc::now() - synced.with_timezone(&Utc) < Duration::seconds(5));
	assert_eq!(
		stored.last_sync_summary,
		Some(json!({
			"protocol": KINDLE_EMAIL_PROTOCOL,
			"bytes": 4096,
			"format": "azw3",
		}))
	);
}

async fn book_in(conn: &DatabaseConnection, library_id: &str) -> String {
	let series = fake_data::Series {
		library_id: Some(library_id.to_string()),
		..Default::default()
	}
	.insert(conn)
	.await;
	fake_data::Media {
		series_id: series.id,
		..Default::default()
	}
	.insert(conn)
	.await
	.id
}

async fn deltas(conn: &DatabaseConnection, device_id: &str) -> Vec<(String, bool)> {
	device_entitlement_delta::Entity::find()
		.filter(device_entitlement_delta::Column::DeviceId.eq(device_id))
		.order_by_asc(device_entitlement_delta::Column::MediaId)
		.all(conn)
		.await
		.expect("deltas")
		.into_iter()
		.map(|row| (row.media_id, row.removed))
		.collect()
}
