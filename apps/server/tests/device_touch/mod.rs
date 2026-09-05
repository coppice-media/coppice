//! `POST /api/v2/devices/me/touch`: a paired liseur device stores a free-form
//! sync summary on its device row.

use axum::http::StatusCode;
use models::entity::device;
use sea_orm::prelude::*;
use serde_json::{json, Value};

use crate::common::TestApp;

const APPROVE: &str = r#"
	mutation Approve($pairingId: ID!, $code: String) {
		approveDevicePairing(pairingId: $pairingId, code: $code) { id status }
	}
"#;

/// Pair a liseur device end to end and return `(device id, bearer secret)`.
async fn pair_liseur_device(app: &TestApp) -> (String, String) {
	let started = app
		.server
		.post("/api/v2/devices/pair/start")
		.json(&json!({ "kind": "liseur", "name": "Kobo Clara HD (NickelStump)" }))
		.await;
	started.assert_status_ok();
	let started: Value = started.json();
	let pairing_id = started["pairing_id"].as_str().unwrap().to_string();
	let code = started["code"].as_str().unwrap().to_string();
	let nonce = started["nonce"].as_str().unwrap().to_string();

	let approved = app
		.execute_gql(APPROVE, Some(json!({ "pairingId": pairing_id, "code": code })))
		.await;
	assert_eq!(
		approved["data"]["approveDevicePairing"]["status"], "APPROVED",
		"{approved:#}"
	);

	let issued = app
		.server
		.get(&format!("/api/v2/devices/pair/{pairing_id}/status?nonce={nonce}"))
		.await;
	issued.assert_status_ok();
	let issued: Value = issued.json();
	assert_eq!(issued["credential"]["kind"], "liseur_token", "{issued:#}");
	(
		issued["device"]["id"].as_str().unwrap().to_string(),
		issued["credential"]["secret"].as_str().unwrap().to_string(),
	)
}

async fn touch(app: &TestApp, bearer: Option<&str>, body: &Value) -> StatusCode {
	let mut request = app.server.post("/api/v2/devices/me/touch").json(body);
	if let Some(bearer) = bearer {
		request = request.add_header("Authorization", format!("Bearer {bearer}"));
	}
	request.await.status_code()
}

async fn stored_summary(app: &TestApp, device_id: &str) -> Option<Value> {
	device::Entity::find_by_id(device_id.to_string())
		.one(app.conn())
		.await
		.expect("device query")
		.expect("device row")
		.last_sync_summary
}

#[tokio::test]
async fn paired_liseur_device_stores_its_summary_verbatim() {
	let app = TestApp::new_with_default_user().await;
	let (device_id, secret) = pair_liseur_device(&app).await;

	let summary = json!({
		"plugin": "nickelstump",
		"firmware_version": "4.38.23697",
		"battery_percent": 87,
		"book": { "title": "Leaves", "progress_percent": 42 }
	});
	assert_eq!(
		touch(&app, Some(&secret), &summary).await,
		StatusCode::NO_CONTENT
	);
	assert_eq!(stored_summary(&app, &device_id).await, Some(summary));

	// The last writer wins; the summary is replaced, not merged.
	let later = json!({ "plugin": "nickelstump", "battery_percent": 12 });
	assert_eq!(
		touch(&app, Some(&secret), &later).await,
		StatusCode::NO_CONTENT
	);
	assert_eq!(stored_summary(&app, &device_id).await, Some(later));
}

#[tokio::test]
async fn touch_rejects_missing_or_unknown_credentials_and_bad_bodies() {
	let app = TestApp::new_with_default_user().await;
	let (_, secret) = pair_liseur_device(&app).await;
	let body = json!({ "battery_percent": 50 });

	assert_eq!(touch(&app, None, &body).await, StatusCode::UNAUTHORIZED);
	assert_eq!(
		touch(&app, Some("liseur-not-a-real-secret"), &body).await,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		touch(&app, Some(&secret), &json!([1, 2, 3])).await,
		StatusCode::BAD_REQUEST
	);
	let too_large = json!({ "blob": "x".repeat(17 * 1024) });
	assert_eq!(
		touch(&app, Some(&secret), &too_large).await,
		StatusCode::BAD_REQUEST
	);
}
