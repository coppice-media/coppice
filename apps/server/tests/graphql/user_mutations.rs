use crate::common::{account::CreateTestUser, TestApp};
use graphql::input::user::AgeRestrictionInput;
use models::shared::enums::UserPermission;
use serde_json::json;

const UPDATE_VIEWER: &str = r#"
mutation UpdateViewer($input: UpdateUserInput!) {
    updateViewer(input: $input) {
        id
        username
        maxSessionsAllowed
        permissions
        ageRestriction { age restrictOnUnset }
    }
}
"#;

const UPDATE_USER: &str = r#"
mutation UpdateUser($id: ID!, $input: UpdateUserInput!) {
    updateUser(id: $id, input: $input) {
        id
        username
        maxSessionsAllowed
        permissions
        ageRestriction { age restrictOnUnset }
    }
}
"#;

fn assert_no_errors(response: &serde_json::Value) {
	assert!(
		response.get("errors").is_none(),
		"should not have had errors: {response:#}"
	);
}

fn assert_has_error(response: &serde_json::Value) {
	assert!(
		response.get("errors").is_some(),
		"should have had errors: {response:#}"
	);
}

fn permissions_from(response: &serde_json::Value, mutation: &str) -> Vec<String> {
	response["data"][mutation]["permissions"]
		.as_array()
		.unwrap_or(&vec![])
		.iter()
		.filter_map(|v| v.as_str().map(String::from))
		.collect()
}

/// a regular user cannot update their own permissions via `updateViewer`. it will not
/// error, but the permissions will not be updated
#[tokio::test]
async fn test_regular_user_cannot_change_own_permissions_via_update_viewer() {
	let app = TestApp::new_with_default_user().await;
	let user = CreateTestUser {
		username: "self-update-viewer".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_VIEWER,
			Some(json!({ "input": {
				"username": &user.username,
				"permissions": ["MANAGE_USERS", "MANAGE_LIBRARY", "DOWNLOAD_FILE"],
				"maxSessionsAllowed": null,
				"ageRestriction": null
			}})),
			&user.token,
		)
		.await;

	assert_no_errors(&response);
	let perms = permissions_from(&response, "updateViewer");
	assert!(
		perms.is_empty(),
		"permissions should be unchanged (empty), got: {perms:?}"
	);
}

/// same as above, but via `updateUser` instead of `updateViewer`
#[tokio::test]
async fn test_regular_user_cannot_update_own_permissions_via_update_user() {
	let app = TestApp::new_with_default_user().await;
	let user = CreateTestUser {
		username: "self-update-user".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_USER,
			Some(json!({
				"id": user.id,
				"input": {
					"username": &user.username,
					"permissions": ["MANAGE_USERS"],
					"maxSessionsAllowed": null,
					"ageRestriction": null
				}
			})),
			&user.token,
		)
		.await;

	assert_no_errors(&response);
	let perms = permissions_from(&response, "updateUser");
	assert!(
		perms.is_empty(),
		"permissions should be unchanged (empty), got: {perms:?}"
	);
}

/// a regular user with age restriction must not be able to remove their restriction
#[tokio::test]
async fn test_regular_user_cannot_remove_own_age_restriction() {
	let app = TestApp::new_with_default_user().await;
	let user = CreateTestUser {
		username: "age-restrict-bypass".to_string(),
		age_restriction: Some(AgeRestrictionInput {
			age: 13,
			restrict_on_unset: true,
		}),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_VIEWER,
			Some(json!({ "input": {
				"username": &user.username,
				"permissions": [],
				"maxSessionsAllowed": null,
				"ageRestriction": null   // attempt to clear the restriction
			}})),
			&user.token,
		)
		.await;

	assert_no_errors(&response);
	let ar = &response["data"]["updateViewer"]["ageRestriction"];
	assert!(
		!ar.is_null(),
		"age restriction should NOT have been removed, but got null"
	);
	assert_eq!(
		ar["age"],
		json!(13),
		"age restriction age should still be 13"
	);
}

/// a regular user must not be able to raise their own session cap
#[tokio::test]
async fn test_regular_user_cannot_change_own_max_sessions_allowed() {
	let app = TestApp::new_with_default_user().await;
	let user = CreateTestUser {
		username: "session-cap-bypass".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_VIEWER,
			Some(json!({ "input": {
				"username": &user.username,
				"permissions": [],
				"maxSessionsAllowed": 99,   // attempt to raise the cap
				"ageRestriction": null
			}})),
			&user.token,
		)
		.await;

	assert_no_errors(&response);
	let cap = &response["data"]["updateViewer"]["maxSessionsAllowed"];
	assert!(
		cap.is_null(),
		"maxSessionsAllowed should still be null, got: {cap}"
	);
}

/// `createUser` enforces the same positive session cap as `updateUser`
#[tokio::test]
async fn test_create_user_rejects_non_positive_session_cap() {
	let app = TestApp::new_with_default_user().await;
	for cap in [0_i32, -1] {
		let response = app
			.execute_gql(
				r#"mutation($input: CreateUserInput!) { createUser(input: $input) { id } }"#,
				Some(json!({ "input": {
					"username": format!("zero-cap-{}", cap.abs()),
					"password": "password",
					"permissions": [],
					"maxSessionsAllowed": cap,
				}})),
			)
			.await;
		assert_has_error(&response);
	}
}

/// rewriting library exclusions needs MANAGE_LIBRARY *and* READ_USERS; either
/// alone is refused (the guard used to accept any one of them)
#[tokio::test]
async fn test_library_exclusions_require_manage_library_and_read_users() {
	let app = TestApp::new_with_default_user().await;
	let library = tests::fake_data::Library {
		id: Some("exclusion-library".to_string()),
		name: Some("Exclusion Library".to_string()),
		..Default::default()
	}
	.insert(app.conn())
	.await;
	let target = CreateTestUser {
		username: "exclusion-target".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;
	let exclude = |token: String| {
		let app = &app;
		let library_id = library.id.clone();
		let target_id = target.id.clone();
		async move {
			app.execute_gql_with_token(
				r#"mutation($id: ID!, $users: [String!]!) {
					updateLibraryExcludedUsers(id: $id, userIds: $users) { id }
				}"#,
				Some(json!({ "id": library_id, "users": [target_id] })),
				&token,
			)
			.await
		}
	};

	for (username, permissions) in [
		("reads-users-only", vec![UserPermission::ReadUsers]),
		("manages-library-only", vec![UserPermission::ManageLibrary]),
	] {
		let actor = CreateTestUser {
			username: username.to_string(),
			permissions,
			..Default::default()
		}
		.insert(&app)
		.await;
		assert_has_error(&exclude(actor.token).await);
	}

	let manager = CreateTestUser {
		username: "manages-library-and-reads-users".to_string(),
		permissions: vec![UserPermission::ManageLibrary, UserPermission::ReadUsers],
		..Default::default()
	}
	.insert(&app)
	.await;
	assert_no_errors(&exclude(manager.token).await);
}

// TODO(permissions): server owner going away
#[tokio::test]
async fn test_server_owner_can_set_permissions_on_another_user() {
	let app = TestApp::new_with_default_user().await;
	let target = CreateTestUser {
		username: "owner-perm-target".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql(
			UPDATE_USER,
			Some(json!({
				"id": target.id,
				"input": {
					"username": &target.username,
					"permissions": ["DOWNLOAD_FILE"],
					"maxSessionsAllowed": null,
					"ageRestriction": null
				}
			})),
		)
		.await;

	assert_no_errors(&response);
	let perms = permissions_from(&response, "updateUser");
	assert!(
		perms.iter().any(|p| p == "DOWNLOAD_FILE"),
		"DOWNLOAD_FILE should be present, got: {perms:?}"
	);
}

// TODO(permissions): server owner going away
#[tokio::test]
async fn test_server_owner_can_set_age_restriction_on_another_user() {
	let app = TestApp::new_with_default_user().await;
	let target = CreateTestUser {
		username: "owner-ar-target".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql(
			UPDATE_USER,
			Some(json!({
				"id": target.id,
				"input": {
					"username": &target.username,
					"permissions": [],
					"maxSessionsAllowed": null,
					"ageRestriction": { "age": 16, "restrictOnUnset": true }
				}
			})),
		)
		.await;

	assert_no_errors(&response);
	let ar = &response["data"]["updateUser"]["ageRestriction"];
	assert!(!ar.is_null(), "ageRestriction should have been set");
	assert_eq!(ar["age"], json!(16));
	assert_eq!(ar["restrictOnUnset"], json!(true));
}

// TODO(permissions): server owner going away
#[tokio::test]
async fn test_server_owner_can_set_max_sessions_allowed_on_another_user() {
	let app = TestApp::new_with_default_user().await;
	let target = CreateTestUser {
		username: "owner-sessions-target".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql(
			UPDATE_USER,
			Some(json!({
				"id": target.id,
				"input": {
					"username": &target.username,
					"permissions": [],
					"maxSessionsAllowed": 3,
					"ageRestriction": null
				}
			})),
		)
		.await;

	assert_no_errors(&response);
	assert_eq!(
		response["data"]["updateUser"]["maxSessionsAllowed"],
		json!(3),
		"maxSessionsAllowed should be 3"
	);
}

/// a user with ManageUsers can set another user's permissions
#[tokio::test]
async fn test_manage_users_can_set_permissions_on_another_user() {
	let app = TestApp::new_with_default_user().await;

	let manager = CreateTestUser {
		username: "manager-sets-perms".to_string(),
		permissions: vec![UserPermission::ManageUsers],
		..Default::default()
	}
	.insert(&app)
	.await;

	let target = CreateTestUser {
		username: "managed-perms-target".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_USER,
			Some(json!({
				"id": target.id,
				"input": {
					"username": target.username,
					"permissions": ["DOWNLOAD_FILE"],
					"maxSessionsAllowed": null,
					"ageRestriction": null
				}
			})),
			&manager.token,
		)
		.await;

	assert_no_errors(&response);
	let perms = permissions_from(&response, "updateUser");
	assert!(
		perms.iter().any(|p| p == "DOWNLOAD_FILE"),
		"DOWNLOAD_FILE should be present after manager update, got: {perms:?}"
	);
}

/// a user with ManageUsers can set another user's age restriction
#[tokio::test]
async fn test_manage_users_can_set_age_restriction_on_another_user() {
	let app = TestApp::new_with_default_user().await;

	let manager = CreateTestUser {
		username: "manager-sets-ar".to_string(),
		permissions: vec![UserPermission::ManageUsers],
		..Default::default()
	}
	.insert(&app)
	.await;

	let target = CreateTestUser {
		username: "managed-ar-target".to_string(),
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_USER,
			Some(json!({
				"id": target.id,
				"input": {
					"username": target.username,
					"permissions": [],
					"maxSessionsAllowed": null,
					"ageRestriction": { "age": 18, "restrictOnUnset": false }
				}
			})),
			&manager.token,
		)
		.await;

	assert_no_errors(&response);
	let ar = &response["data"]["updateUser"]["ageRestriction"];
	assert!(
		!ar.is_null(),
		"ageRestriction should have been set by manager"
	);
	assert_eq!(ar["age"], json!(18));
}

// TODO(permissions): server owner going away
/// a user with ManageUsers permission cannot edit the server owner's account
#[tokio::test]
async fn test_manage_users_cannot_edit_server_owner() {
	let app = TestApp::new_with_default_user().await;

	let manager = CreateTestUser {
		username: "manager-vs-owner".to_string(),
		permissions: vec![UserPermission::ManageUsers],
		..Default::default()
	}
	.insert(&app)
	.await;

	let me = app.execute_gql("query { me { id } }", None).await;
	assert_no_errors(&me);
	let owner_id = me["data"]["me"]["id"]
		.as_str()
		.expect("server owner should have id")
		.to_string();

	let response = app
		.execute_gql_with_token(
			UPDATE_USER,
			Some(json!({
				"id": owner_id,
				"input": {
					"username": &manager.username,
					"permissions": [],
					"maxSessionsAllowed": null,
					"ageRestriction": null
				}
			})),
			&manager.token,
		)
		.await;

	assert_has_error(&response);
}

/// a user with ManageUsers permission cannot update their own permissions via `updateUser`
#[tokio::test]
async fn test_manage_users_cannot_update_own_permissions_via_self_update() {
	let app = TestApp::new_with_default_user().await;
	let manager = CreateTestUser {
		username: "manager-self-update".to_string(),
		permissions: vec![UserPermission::ManageUsers],
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_USER,
			Some(json!({
				"id": manager.id,
				"input": {
					"username": &manager.username,
					"permissions": ["MANAGE_USERS", "MANAGE_LIBRARY", "MANAGE_SERVER"],
					"maxSessionsAllowed": null,
					"ageRestriction": null
				}
			})),
			&manager.token,
		)
		.await;

	assert_no_errors(&response);
	let perms = permissions_from(&response, "updateUser");
	assert!(
		!perms.iter().any(|p| p == "MANAGE_LIBRARY"),
		"MANAGE_LIBRARY must not be self-granted, got: {perms:?}"
	);
	assert!(
		!perms.iter().any(|p| p == "MANAGE_SERVER"),
		"MANAGE_SERVER must not be self-granted, got: {perms:?}"
	);
}

/// a user with ManageUsers permission cannot update their own permissions via `updateViewer`
#[tokio::test]
async fn test_manage_users_cannot_update_own_permissions_via_update_viewer() {
	let app = TestApp::new_with_default_user().await;
	let manager = CreateTestUser {
		username: "manager-viewer-update".to_string(),
		permissions: vec![UserPermission::ManageUsers],
		..Default::default()
	}
	.insert(&app)
	.await;

	let response = app
		.execute_gql_with_token(
			UPDATE_VIEWER,
			Some(json!({ "input": {
				"username": &manager.username,
				"permissions": ["MANAGE_USERS", "MANAGE_LIBRARY", "MANAGE_SERVER"],
				"maxSessionsAllowed": null,
				"ageRestriction": null
			}})),
			&manager.token,
		)
		.await;

	assert_no_errors(&response);
	let perms = permissions_from(&response, "updateViewer");
	assert!(
		!perms.iter().any(|p| p == "MANAGE_LIBRARY"),
		"MANAGE_LIBRARY must not be self-granted via updateViewer, got: {perms:?}"
	);
}
