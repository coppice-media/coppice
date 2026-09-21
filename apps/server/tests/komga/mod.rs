//! Komga compatibility requests authenticated with a registered device's key.

use axum::http::{header, StatusCode};
use base64::{engine::general_purpose::STANDARD, Engine};
use models::{entity::user::AuthUser, shared::enums::DeviceKind};
use serde_json::json;
use stump_core::CoreEvent;
use stump_devices::Protocol;
use tests::fake_data;

use crate::common::{series::setup_single_series_with_n_books, TestApp};

fn basic(username: &str, secret: &str) -> String {
	format!("Basic {}", STANDARD.encode(format!("{username}:{secret}")))
}

/// a Basic request whose password is a device key marks the device as seen
/// without opening a session, and a read-progress patch records a sync summary
#[tokio::test]
async fn test_komga_basic_request_with_device_key_updates_last_seen() {
	let app = TestApp::new().await;
	let user = fake_data::User::new("komelia").insert(app.conn()).await;
	let owner = AuthUser {
		id: user.id.clone(),
		username: user.username.clone(),
		is_server_owner: true,
		..Default::default()
	};
	let library = fake_data::Library {
		id: Some("comics".to_string()),
		name: Some("Comics".to_string()),
		..Default::default()
	}
	.insert(app.conn())
	.await;
	let (_, books) = setup_single_series_with_n_books(
		&app,
		fake_data::Series {
			id: Some("saga".to_string()),
			name: Some("Saga".to_string()),
			library_id: Some(library.id.clone()),
			..Default::default()
		},
		1,
	)
	.await;
	let book = books.into_iter().next().expect("one book");

	let devices = app.ctx.devices();
	let (device, issued) = devices
		.create_device(
			&owner,
			stump_devices::CredentialIssuance::InteractiveSession,
			DeviceKind::Komelia,
			Some("Pixel".to_string()),
		)
		.await
		.expect("failed to create device");
	let mut events = app.ctx.get_client_receiver();

	let response = app
		.server
		.get("/api/v1/libraries")
		.add_header(header::AUTHORIZATION, basic(&user.username, &issued.secret))
		.await;
	response.assert_status_ok();
	assert!(
		response.maybe_header(header::SET_COOKIE).is_none(),
		"a device key must not open a session"
	);

	let seen = devices.get(&owner, &device.id).await.expect("device");
	assert!(seen.last_seen_at.is_some(), "last_seen_at was not recorded");
	assert!(seen.last_sync_at.is_none());
	match events.try_recv().expect("expected a DeviceSeen event") {
		CoreEvent::DeviceSeen(event) => {
			assert_eq!(event.device_id, device.id);
			assert_eq!(event.protocol, Protocol::Komga);
			assert!(event.first_seen);
		},
		other => panic!("unexpected event {other:?}"),
	}

	app.server
		.patch(&format!("/api/v1/books/{}/read-progress", book.id))
		.add_header(header::AUTHORIZATION, basic(&user.username, &issued.secret))
		.json(&json!({ "page": 1 }))
		.await
		.assert_status(StatusCode::NO_CONTENT);

	let synced = devices.get(&owner, &device.id).await.expect("device");
	assert!(
		synced.last_sync_at.is_some(),
		"last_sync_at was not recorded"
	);
	let summary = synced.last_sync_summary.expect("sync summary");
	assert_eq!(summary["protocol"], json!("komga"));
	assert_eq!(summary["book_id"], json!(book.id));
	assert_eq!(summary["page"], json!(1));

	// the key only authenticates as its owner
	app.server
		.get("/api/v1/libraries")
		.add_header(header::AUTHORIZATION, basic("nobody", &issued.secret))
		.await
		.assert_status(StatusCode::UNAUTHORIZED);
}
