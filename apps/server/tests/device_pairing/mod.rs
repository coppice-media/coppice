//! End-to-end coverage of the device-pairing flow: REST start/status/qr on
//! the device side, GraphQL approve/deny/pending on the user side.

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use models::{
	entity::{api_key, device_pairing},
	shared::{api_key::APIKeyPermissions, enums::UserPermission},
};
use sea_orm::{prelude::*, sea_query::Expr};
use serde_json::{json, Value};

use crate::common::TestApp;

const APPROVE: &str = r#"
	mutation Approve($pairingId: ID!, $code: String, $nonce: String) {
		approveDevicePairing(pairingId: $pairingId, code: $code, nonce: $nonce) {
			id status userId failedAttempts credentialIssued approvedAt
		}
	}
"#;
const DENY: &str = r#"
	mutation Deny($pairingId: ID!) {
		denyDevicePairing(pairingId: $pairingId) { id status userId }
	}
"#;
const PENDING: &str = r#"
	query { pendingDevicePairings { id kind name remoteIp status userId expiresAt } }
"#;

async fn start(app: &TestApp, body: Value) -> Value {
	let response = app
		.server
		.post("/api/v2/devices/pair/start")
		.json(&body)
		.await;
	response.assert_status_ok();
	response.json()
}

fn status_path(started: &Value, nonce: &str) -> String {
	format!(
		"/api/v2/devices/pair/{}/status?nonce={nonce}",
		started["pairing_id"].as_str().unwrap()
	)
}

async fn status(app: &TestApp, started: &Value) -> Value {
	let response = app
		.server
		.get(&status_path(started, started["nonce"].as_str().unwrap()))
		.await;
	response.assert_status_ok();
	response.json()
}

/// Run GraphQL as `token` (any user, not just the default admin) and return the body.
async fn gql_as(app: &TestApp, token: &str, query: &str, variables: Value) -> Value {
	let response = app
		.server
		.post("/api/graphql")
		.add_header("Authorization", format!("Bearer {token}"))
		.json(&json!({ "query": query, "variables": variables }))
		.await;
	response.assert_status_ok();
	response.json()
}

fn first_error(body: &Value) -> String {
	body["errors"][0]["message"]
		.as_str()
		.unwrap_or_else(|| panic!("expected a GraphQL error in {body:#}"))
		.to_string()
}

async fn approve(app: &TestApp, pairing_id: &str, code: &str) -> Value {
	app.execute_gql(
		APPROVE,
		Some(json!({ "pairingId": pairing_id, "code": code })),
	)
	.await
}

async fn admin_id(app: &TestApp) -> String {
	let me = app.get("/api/v2/auth/me").await;
	me.assert_status_ok();
	me.json::<Value>()["id"].as_str().unwrap().to_string()
}

/// Create a non-owner user holding ACCESS_API_KEYS and return their bearer token.
async fn second_user_token(app: &TestApp, username: &str) -> String {
	let created = app
		.execute_gql(
			r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id } }"#,
			Some(json!({ "input": {
				"username": username,
				"password": "password",
				"permissions": ["ACCESS_API_KEYS", "DOWNLOAD_FILE"],
			}})),
		)
		.await;
	assert!(created["errors"].is_null(), "{created:#}");

	let login = app
		.server
		.post("/api/v2/auth/login?generate_token=true")
		.json(&json!({ "username": username, "password": "password" }))
		.await;
	login.assert_status_ok();
	login.json::<Value>()["accessToken"]
		.as_str()
		.unwrap()
		.to_string()
}

async fn expire_now(app: &TestApp, pairing_id: &str) {
	device_pairing::Entity::update_many()
		.col_expr(
			device_pairing::Column::ExpiresAt,
			Expr::value(DateTimeWithTimeZone::from(
				Utc::now() - Duration::seconds(1),
			)),
		)
		.filter(device_pairing::Column::Id.eq(pairing_id))
		.exec(app.conn())
		.await
		.expect("could not expire pairing");
}

#[tokio::test]
async fn happy_path_issues_credential_exactly_once() {
	let app = TestApp::new_with_default_user().await;

	let started = start(&app, json!({ "kind": "kobo", "name": "  Clara  " })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();
	let code = started["code"].as_str().unwrap();
	let nonce = started["nonce"].as_str().unwrap();
	assert_eq!(code.len(), 6);
	assert!(code.bytes().all(|b| b.is_ascii_digit()), "{code}");
	assert_eq!(nonce.len(), 32);
	assert_eq!(started["poll_interval_secs"], 2);
	let qr_payload = started["qr_payload"].as_str().unwrap();
	assert!(
		qr_payload.starts_with("stump://pair?host=http%3A%2F%2F"),
		"{qr_payload}"
	);
	assert!(
		qr_payload.ends_with(&format!("&id={pairing_id}&nonce={nonce}")),
		"{qr_payload}"
	);
	let expires_at =
		chrono::DateTime::parse_from_rfc3339(started["expires_at"].as_str().unwrap())
			.expect("expires_at is RFC 3339");
	let ttl = expires_at.with_timezone(&Utc) - Utc::now();
	assert!(
		ttl > Duration::minutes(4) && ttl <= Duration::minutes(5),
		"{ttl}"
	);

	assert_eq!(status(&app, &started).await, json!({ "status": "pending" }));

	let pending = app.execute_gql(PENDING, None).await;
	let listed = &pending["data"]["pendingDevicePairings"];
	assert_eq!(listed.as_array().map(Vec::len), Some(1), "{pending:#}");
	assert_eq!(listed[0]["id"], pairing_id);
	assert_eq!(listed[0]["kind"], "KOBO");
	assert_eq!(listed[0]["name"], "Clara");
	assert_eq!(listed[0]["remoteIp"], "127.0.0.1");
	assert_eq!(listed[0]["status"], "PENDING");
	assert!(listed[0]["userId"].is_null());

	let approved = approve(&app, pairing_id, code).await;
	let approved = &approved["data"]["approveDevicePairing"];
	assert_eq!(approved["status"], "APPROVED", "{approved:#}");
	assert_eq!(approved["userId"], admin_id(&app).await);
	assert_eq!(approved["credentialIssued"], false);
	assert!(!approved["approvedAt"].is_null());

	// First poll after approval carries the credential
	let issued = status(&app, &started).await;
	assert_eq!(issued["status"], "approved", "{issued:#}");
	assert_eq!(issued["credential_issued"], true);
	assert_eq!(issued["device"]["kind"], "kobo");
	assert_eq!(issued["device"]["name"], "Clara");
	assert_eq!(issued["credential"]["kind"], "api_key");
	assert_eq!(issued["credential"]["protocol"], "kobo");
	let secret = issued["credential"]["secret"].as_str().unwrap();
	assert!(secret.starts_with("stump_"), "{secret}");
	assert!(issued["endpoints"].is_array(), "{issued:#}");

	// Every later poll is approved but carries nothing secret
	let again = status(&app, &started).await;
	assert_eq!(
		again,
		json!({ "status": "approved", "credential_issued": true })
	);

	// The pairing left the pending list
	let pending = app.execute_gql(PENDING, None).await;
	assert_eq!(pending["data"]["pendingDevicePairings"], json!([]));

	// ...and cannot be approved twice
	let twice = approve(&app, pairing_id, code).await;
	assert_eq!(first_error(&twice), "Pairing has already been approved");
}

#[tokio::test]
async fn komelia_pairing_metadata_opt_in_requires_and_mints_edit_permission() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "komelia", "name": "Komelia" })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();
	let code = started["code"].as_str().unwrap();
	let approve_with_komf = r#"
		mutation($pairingId: ID!, $code: String, $allowKomfMetadataEditing: Boolean = false) {
			approveDevicePairing(
				pairingId: $pairingId,
				code: $code,
				allowKomfMetadataEditing: $allowKomfMetadataEditing
			) { id status }
		}
	"#;

	let without_permission = second_user_token(&app, "komf-without-edit").await;
	let rejected = gql_as(
		&app,
		&without_permission,
		approve_with_komf,
		json!({
			"pairingId": pairing_id,
			"code": code,
			"allowKomfMetadataEditing": true,
		}),
	)
	.await;
	assert!(
		first_error(&rejected).contains("requires the Edit metadata permission"),
		"{rejected:#}"
	);

	let editor = app
		.execute_gql(
			r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id } }"#,
			Some(json!({ "input": {
				"username": "komf-editor",
				"password": "password",
				"permissions": ["ACCESS_API_KEYS", "DOWNLOAD_FILE", "EDIT_METADATA"],
			}})),
		)
		.await;
	assert!(editor["errors"].is_null(), "{editor:#}");
	let login = app
		.server
		.post("/api/v2/auth/login?generate_token=true")
		.json(&json!({ "username": "komf-editor", "password": "password" }))
		.await;
	login.assert_status_ok();
	let editor_token = login.json::<Value>()["accessToken"]
		.as_str()
		.unwrap()
		.to_string();

	let approved = gql_as(
		&app,
		&editor_token,
		approve_with_komf,
		json!({
			"pairingId": pairing_id,
			"code": code,
			"allowKomfMetadataEditing": true,
		}),
	)
	.await;
	assert!(approved["errors"].is_null(), "{approved:#}");
	assert_eq!(
		approved["data"]["approveDevicePairing"]["status"], "APPROVED",
		"{approved:#}"
	);

	let issued = status(&app, &started).await;
	let secret = issued["credential"]["secret"].as_str().unwrap();
	let credential_ref = issued["credential"]["credential_ref"].as_str().unwrap();
	assert!(secret.starts_with("stump_"), "{secret}");
	let key = api_key::Entity::find()
		.filter(api_key::Column::ShortToken.eq(credential_ref))
		.one(app.conn())
		.await
		.expect("query Komelia key")
		.expect("Komelia API key exists");
	assert_eq!(
		key.permissions,
		APIKeyPermissions::Custom(vec![
			UserPermission::DownloadFile,
			UserPermission::EditMetadata,
		])
	);
}

#[tokio::test]
async fn coppice_pairing_issues_both_credentials_with_stable_metadata() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "coppice" })).await;
	let approved = approve(
		&app,
		started["pairing_id"].as_str().unwrap(),
		started["code"].as_str().unwrap(),
	)
	.await;
	assert_eq!(
		approved["data"]["approveDevicePairing"]["status"], "APPROVED",
		"{approved:#}"
	);

	let issued = status(&app, &started).await;
	assert_eq!(issued["status"], "approved", "{issued:#}");
	assert_eq!(issued["credential_issued"], true);
	assert_eq!(issued["username"], "initial-server-admin");

	let credentials = issued["credentials"]
		.as_array()
		.expect("Coppice credentials array");
	assert_eq!(credentials.len(), 2, "{issued:#}");
	let api = credentials
		.iter()
		.find(|credential| {
			credential["kind"] == "api_key" && credential["protocol"] == "koreader"
		})
		.expect("Coppice API credential");
	let liseur = credentials
		.iter()
		.find(|credential| {
			credential["kind"] == "liseur_token" && credential["protocol"] == "liseur"
		})
		.expect("Coppice liseur credential");
	assert!(api["secret"].as_str().unwrap().starts_with("stump_"));
	assert!(liseur["secret"].as_str().unwrap().starts_with("liseur-"));
	assert_eq!(&issued["credential"], api);
	assert!(issued["endpoints"].as_array().is_some());

	let again = status(&app, &started).await;
	assert_eq!(
		again,
		json!({ "status": "approved", "credential_issued": true })
	);
}

#[tokio::test]
async fn qr_nonce_approves_without_the_code() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "liseur" })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();

	let wrong = app
		.execute_gql(
			APPROVE,
			Some(json!({ "pairingId": pairing_id, "nonce": "0".repeat(32) })),
		)
		.await;
	assert_eq!(
		first_error(&wrong),
		"Incorrect code or nonce (4 attempts left)"
	);

	let both = app
		.execute_gql(
			APPROVE,
			Some(json!({
				"pairingId": pairing_id,
				"code": started["code"],
				"nonce": started["nonce"],
			})),
		)
		.await;
	assert!(first_error(&both).starts_with("Provide exactly one of"));

	let approved = app
		.execute_gql(
			APPROVE,
			Some(json!({ "pairingId": pairing_id, "nonce": started["nonce"] })),
		)
		.await;
	assert_eq!(
		approved["data"]["approveDevicePairing"]["status"], "APPROVED",
		"{approved:#}"
	);

	let issued = status(&app, &started).await;
	assert_eq!(issued["credential"]["kind"], "liseur_token", "{issued:#}");
	assert_eq!(issued["credential"]["protocol"], "liseur");
	assert!(!issued["credential"]["secret"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn five_wrong_codes_deny_the_pairing() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "mihon" })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();
	let code = started["code"].as_str().unwrap();
	let wrong_code = if code == "000000" { "000001" } else { "000000" };

	for attempt in 1..5 {
		let rejected = approve(&app, pairing_id, wrong_code).await;
		assert_eq!(
			first_error(&rejected),
			format!(
				"Incorrect code or nonce ({} attempt{} left)",
				5 - attempt,
				if attempt == 4 { "" } else { "s" }
			)
		);
		assert_eq!(status(&app, &started).await, json!({ "status": "pending" }));
	}

	let denied = approve(&app, pairing_id, wrong_code).await;
	assert_eq!(
		first_error(&denied),
		"Too many failed attempts; the pairing has been denied"
	);
	assert_eq!(status(&app, &started).await, json!({ "status": "denied" }));

	// Even the right code is useless now
	let late = approve(&app, pairing_id, code).await;
	assert_eq!(first_error(&late), "Pairing has been denied");

	let row = device_pairing::Entity::find_by_id(pairing_id)
		.one(app.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(row.failed_attempts, 5);
	assert_eq!(row.user_id, Some(admin_id(&app).await));
	assert!(!row.credential_issued);
}

#[tokio::test]
async fn expired_pairing_cannot_be_approved() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "koreader" })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();
	expire_now(&app, pairing_id).await;

	let pending = app.execute_gql(PENDING, None).await;
	assert_eq!(pending["data"]["pendingDevicePairings"], json!([]));

	let rejected = approve(&app, pairing_id, started["code"].as_str().unwrap()).await;
	assert_eq!(first_error(&rejected), "Pairing has expired");

	assert_eq!(status(&app, &started).await, json!({ "status": "expired" }));

	let row = device_pairing::Entity::find_by_id(pairing_id)
		.one(app.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(row.status, device_pairing::DevicePairingStatus::Expired);
}

#[tokio::test]
async fn nonce_mismatch_reads_as_not_found() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "opds" })).await;

	app.server
		.get(&status_path(&started, &"f".repeat(32)))
		.await
		.assert_status(StatusCode::NOT_FOUND);
	app.server
		.get(&status_path(&started, ""))
		.await
		.assert_status(StatusCode::NOT_FOUND);
	app.server
		.get(&format!(
			"/api/v2/devices/pair/{}/status",
			started["pairing_id"].as_str().unwrap()
		))
		.await
		.assert_status(StatusCode::BAD_REQUEST);
	app.server
		.get(&format!(
			"/api/v2/devices/pair/00000000-0000-4000-8000-000000000000/status?nonce={}",
			started["nonce"].as_str().unwrap()
		))
		.await
		.assert_status(StatusCode::NOT_FOUND);

	// The right nonce still works
	assert_eq!(status(&app, &started).await, json!({ "status": "pending" }));
}

#[tokio::test]
async fn approval_binds_the_pairing_to_the_approver() {
	let app = TestApp::new_with_default_user().await;
	let other_token = second_user_token(&app, "second-approver").await;

	let started = start(&app, json!({ "kind": "komelia" })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();
	let code = started["code"].as_str().unwrap();

	// Anyone allowed to create device credentials sees the pending request
	let seen_by_other = gql_as(&app, &other_token, PENDING, json!({})).await;
	assert_eq!(
		seen_by_other["data"]["pendingDevicePairings"][0]["id"], pairing_id,
		"{seen_by_other:#}"
	);

	let approved = gql_as(
		&app,
		&other_token,
		APPROVE,
		json!({ "pairingId": pairing_id, "code": code }),
	)
	.await;
	let approved = &approved["data"]["approveDevicePairing"];
	assert_eq!(approved["status"], "APPROVED", "{approved:#}");
	let other_id = approved["userId"].as_str().unwrap().to_string();
	assert_ne!(other_id, admin_id(&app).await);

	// The admin cannot re-approve or deny somebody else's settled pairing
	let hijack = approve(&app, pairing_id, code).await;
	assert_eq!(first_error(&hijack), "Pairing has already been approved");
	let deny = app
		.execute_gql(DENY, Some(json!({ "pairingId": pairing_id })))
		.await;
	assert_eq!(first_error(&deny), "Pairing has already been approved");

	// The device is minted for the approver, not the server owner
	let issued = status(&app, &started).await;
	assert_eq!(issued["status"], "approved", "{issued:#}");
	let row = device_pairing::Entity::find_by_id(pairing_id)
		.one(app.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(row.user_id.as_deref(), Some(other_id.as_str()));
	assert!(row.credential_issued);
}

#[tokio::test]
async fn approval_requires_the_api_key_permission() {
	let app = TestApp::new_with_default_user().await;
	let created = app
		.execute_gql(
			r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id } }"#,
			Some(json!({ "input": {
				"username": "no-keys",
				"password": "password",
				"permissions": [],
			}})),
		)
		.await;
	assert!(created["errors"].is_null(), "{created:#}");
	let login = app
		.server
		.post("/api/v2/auth/login?generate_token=true")
		.json(&json!({ "username": "no-keys", "password": "password" }))
		.await;
	let token = login.json::<Value>()["accessToken"]
		.as_str()
		.unwrap()
		.to_string();

	let started = start(&app, json!({ "kind": "api" })).await;
	let forbidden = gql_as(
		&app,
		&token,
		APPROVE,
		json!({ "pairingId": started["pairing_id"], "code": started["code"] }),
	)
	.await;
	assert_eq!(
		first_error(&forbidden),
		"You do not have permission to perform this action."
	);
	assert_eq!(status(&app, &started).await, json!({ "status": "pending" }));

	// ACCESS_API_KEYS alone is not enough for a kind whose key carries more
	let created = app
		.execute_gql(
			r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id } }"#,
			Some(json!({ "input": {
				"username": "keys-only",
				"password": "password",
				"permissions": ["ACCESS_API_KEYS"],
			}})),
		)
		.await;
	assert!(created["errors"].is_null(), "{created:#}");
	let login = app
		.server
		.post("/api/v2/auth/login?generate_token=true")
		.json(&json!({ "username": "keys-only", "password": "password" }))
		.await;
	let token = login.json::<Value>()["accessToken"]
		.as_str()
		.unwrap()
		.to_string();
	let started = start(&app, json!({ "kind": "komelia" })).await;
	let refused = gql_as(
		&app,
		&token,
		APPROVE,
		json!({ "pairingId": started["pairing_id"], "code": started["code"] }),
	)
	.await;
	assert_eq!(
		first_error(&refused),
		"Approving a komelia device requires the DOWNLOAD_FILE permission"
	);
	assert_eq!(status(&app, &started).await, json!({ "status": "pending" }));
}

#[tokio::test]
async fn deny_settles_the_pairing() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "kobo" })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();

	let denied = app
		.execute_gql(DENY, Some(json!({ "pairingId": pairing_id })))
		.await;
	assert_eq!(
		denied["data"]["denyDevicePairing"]["status"], "DENIED",
		"{denied:#}"
	);
	assert_eq!(
		denied["data"]["denyDevicePairing"]["userId"],
		admin_id(&app).await
	);

	assert_eq!(status(&app, &started).await, json!({ "status": "denied" }));
	let late = approve(&app, pairing_id, started["code"].as_str().unwrap()).await;
	assert_eq!(first_error(&late), "Pairing has been denied");
	let twice = app
		.execute_gql(DENY, Some(json!({ "pairingId": pairing_id })))
		.await;
	assert_eq!(first_error(&twice), "Pairing has been denied");
}

#[tokio::test]
async fn start_validates_input_and_caps_open_pairings_per_ip() {
	let app = TestApp::new_with_default_user().await;

	app.server
		.post("/api/v2/devices/pair/start")
		.json(&json!({ "kind": "nickel" }))
		.await
		.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
	app.server
		.post("/api/v2/devices/pair/start")
		.json(&json!({ "kind": "kobo", "name": "x".repeat(101) }))
		.await
		.assert_status(StatusCode::BAD_REQUEST);

	for _ in 0..10 {
		start(&app, json!({ "kind": "kobo" })).await;
	}
	app.server
		.post("/api/v2/devices/pair/start")
		.json(&json!({ "kind": "kobo" }))
		.await
		.assert_status(StatusCode::TOO_MANY_REQUESTS);

	// Settling one frees a slot
	let pending = app.execute_gql(PENDING, None).await;
	let oldest = pending["data"]["pendingDevicePairings"][0]["id"]
		.as_str()
		.unwrap()
		.to_string();
	app.execute_gql(DENY, Some(json!({ "pairingId": oldest })))
		.await;
	start(&app, json!({ "kind": "kobo" })).await;
}

#[cfg(feature = "qr")]
#[tokio::test]
async fn qr_png_is_served_only_with_the_nonce() {
	let app = TestApp::new_with_default_user().await;
	let started = start(&app, json!({ "kind": "kobo" })).await;
	let pairing_id = started["pairing_id"].as_str().unwrap();

	let png = app
		.server
		.get(&format!(
			"/api/v2/devices/pair/{pairing_id}/qr.png?nonce={}",
			started["nonce"].as_str().unwrap()
		))
		.await;
	png.assert_status_ok();
	assert_eq!(png.header("content-type"), "image/png");
	assert_eq!(png.header("cache-control"), "no-store");
	assert_eq!(&png.as_bytes()[..8], b"\x89PNG\r\n\x1a\n");

	app.server
		.get(&format!(
			"/api/v2/devices/pair/{pairing_id}/qr.png?nonce={}",
			"0".repeat(32)
		))
		.await
		.assert_status(StatusCode::NOT_FOUND);
}
