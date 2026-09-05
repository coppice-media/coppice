use axum::http::StatusCode;
use models::{entity::user::AuthUser, shared::enums::DeviceKind};
use serde_json::json;
use stump_core::CoreEvent;
use stump_devices::Protocol;
use tests::fake_data;

use crate::common::TestApp;

/// a request authenticated with a device's key marks the device as seen, a
/// library sync records a summary, and a revoked device's key stops working
#[tokio::test]
async fn test_kobo_request_with_device_key_updates_last_seen() {
	let app = TestApp::new().await;
	let user = fake_data::User::new("kobo").insert(app.conn()).await;
	let owner = AuthUser {
		id: user.id.clone(),
		username: user.username.clone(),
		is_server_owner: true,
		..Default::default()
	};

	let devices = app.ctx.devices();
	let (device, issued) = devices
		.create_device(&owner, DeviceKind::Kobo, Some("Clara".to_string()))
		.await
		.expect("failed to create device");
	assert!(device.last_seen_at.is_none());
	let mut events = app.ctx.get_client_receiver();

	app.server
		.post(&format!("/kobo/{}/v1/auth/device", issued.secret))
		.await
		.assert_status_ok();

	let seen = devices.get(&owner, &device.id).await.expect("device");
	assert!(seen.last_seen_at.is_some(), "last_seen_at was not recorded");
	assert!(seen.last_sync_at.is_none());
	match events.try_recv().expect("expected a DeviceSeen event") {
		CoreEvent::DeviceSeen(event) => {
			assert_eq!(event.device_id, device.id);
			assert_eq!(event.user_id, user.id);
			assert_eq!(event.protocol, Protocol::Kobo);
		},
		other => panic!("unexpected event {other:?}"),
	}

	app.server
		.get(&format!("/kobo/{}/v1/library/sync", issued.secret))
		.add_header("x-kobo-deviceid", "kobo-test-device")
		.await
		.assert_status_ok();

	let synced = devices.get(&owner, &device.id).await.expect("device");
	assert!(synced.last_sync_at.is_some(), "last_sync_at was not recorded");
	let summary = synced.last_sync_summary.expect("sync summary");
	assert_eq!(summary["protocol"], json!("kobo"));
	assert_eq!(summary["kobo_device_id"], json!("kobo-test-device"));
	assert_eq!(summary["items"], json!(0));

	devices.revoke(&owner, &device.id).await.expect("revoke");
	app.server
		.post(&format!("/kobo/{}/v1/auth/device", issued.secret))
		.await
		.assert_status(StatusCode::UNAUTHORIZED);
}
