use models::{
	entity::{device, device_credential, device_pairing, user},
	shared::{api_key::APIKeyPermissions, enums::UserPermission},
};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::{json, Value};

use crate::common::{api_key::create_api_key_for_user, TestApp};

const CREATE_DEVICE: &str = r#"
	mutation CreateDevice($kind: DeviceKind!) {
		createDevice(kind: $kind) { device { id } credential { secret } }
	}
"#;
const ROTATE_DEVICE: &str = r#"
	mutation RotateDevice($id: String!) {
		rotateDeviceCredential(id: $id) { device { id } credential { secret } }
	}
"#;
const APPROVE_PAIRING: &str = r#"
	mutation ApprovePairing($pairingId: ID!, $code: String!) {
		approveDevicePairing(pairingId: $pairingId, code: $code) { id status }
	}
"#;

fn first_error(body: &Value) -> &str {
	body["errors"][0]["message"]
		.as_str()
		.unwrap_or_else(|| panic!("expected GraphQL error in {body:#}"))
}

#[tokio::test]
async fn delegated_api_key_cannot_issue_inherited_device_credentials() {
	let app = TestApp::new_with_default_user().await;
	let owner = user::Entity::find()
		.one(app.conn())
		.await
		.expect("user query")
		.expect("server owner");
	let (delegated_key, _) = create_api_key_for_user(
		&app,
		&owner,
		Some(APIKeyPermissions::Custom(vec![
			UserPermission::AccessApiKeys,
		])),
	)
	.await;

	let rejected = app
		.execute_gql_with_token(
			CREATE_DEVICE,
			Some(json!({ "kind": "API" })),
			&delegated_key,
		)
		.await;
	assert_eq!(
		first_error(&rejected),
		"an API key cannot issue another API key with inherited permissions"
	);
	assert_eq!(device::Entity::find().count(app.conn()).await.unwrap(), 0);

	let created = app
		.execute_gql(CREATE_DEVICE, Some(json!({ "kind": "API" })))
		.await;
	assert!(created["errors"].is_null(), "{created:#}");
	let device_id = created["data"]["createDevice"]["device"]["id"]
		.as_str()
		.expect("device id")
		.to_string();
	let original_ref = device_credential::Entity::find()
		.filter(device_credential::Column::DeviceId.eq(&device_id))
		.one(app.conn())
		.await
		.expect("credential query")
		.expect("device credential")
		.credential_ref;

	let rejected = app
		.execute_gql_with_token(
			ROTATE_DEVICE,
			Some(json!({ "id": device_id })),
			&delegated_key,
		)
		.await;
	assert_eq!(
		first_error(&rejected),
		"an API key cannot issue another API key with inherited permissions"
	);
	let current_ref = device_credential::Entity::find()
		.filter(device_credential::Column::DeviceId.eq(&device_id))
		.one(app.conn())
		.await
		.expect("credential query")
		.expect("device credential")
		.credential_ref;
	assert_eq!(current_ref, original_ref);

	let started = app
		.server
		.post("/api/v2/devices/pair/start")
		.json(&json!({ "kind": "api" }))
		.await;
	started.assert_status_ok();
	let started: Value = started.json();
	let pairing_id = started["pairing_id"].as_str().expect("pairing id");
	let code = started["code"].as_str().expect("pairing code");
	let rejected = app
		.execute_gql_with_token(
			APPROVE_PAIRING,
			Some(json!({ "pairingId": pairing_id, "code": code })),
			&delegated_key,
		)
		.await;
	assert_eq!(
		first_error(&rejected),
		"an API key cannot issue another API key with inherited permissions"
	);
	let pending = device_pairing::Entity::find_by_id(pairing_id)
		.one(app.conn())
		.await
		.expect("pairing query")
		.expect("pairing row");
	assert_eq!(pending.status, device_pairing::DevicePairingStatus::Pending);
	assert!(pending.user_id.is_none());

	let approved = app
		.execute_gql(
			APPROVE_PAIRING,
			Some(json!({ "pairingId": pairing_id, "code": code })),
		)
		.await;
	assert!(approved["errors"].is_null(), "{approved:#}");
	assert_eq!(
		approved["data"]["approveDevicePairing"]["status"],
		"APPROVED"
	);
}
