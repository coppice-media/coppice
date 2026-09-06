//! Per-device library scope across the protocol surfaces: two devices of the
//! *same* user, authenticating with their own credentials, resolve the same
//! routes to different visible library sets.

use axum::http::header;
use base64::{engine::general_purpose::STANDARD, Engine};
use migrations::{Migrator, MigratorTrait};
use models::{entity::user::AuthUser, shared::enums::DeviceKind};
use sea_orm::{Database, DatabaseConnection};
use stump_core::config::StumpConfig;
use stump_devices::LibraryScope;
use tests::fake_data;

use crate::common::{series::setup_single_series_with_n_books, TestApp};

/// The suite runs against the real migrated schema: the Komga series list
/// joins `series_tags` and the Kavita profile reads `kavita_ids`, neither of
/// which the entity-built schema of `tests::db::test_database` provides.
async fn migrated_database() -> DatabaseConnection {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("failed to connect to sqlite");
	Migrator::up(&db, None)
		.await
		.expect("failed to run migrations");
	db
}

/// Two libraries, one series each, and two devices of one user: `narrow` sees
/// only the first library, `wide` inherits and sees both.
struct Fixture {
	app: TestApp,
	username: String,
	narrow_secret: String,
	wide_secret: String,
	narrow_device: String,
	comics: String,
	novels: String,
}

async fn fixture(narrow_kind: DeviceKind, wide_kind: DeviceKind) -> Fixture {
	let app = TestApp::with_parts(migrated_database().await, StumpConfig::debug()).await;
	let user = fake_data::User::new("reader").insert(app.conn()).await;
	let owner = AuthUser {
		id: user.id.clone(),
		username: user.username.clone(),
		is_server_owner: true,
		..Default::default()
	};

	let comics = fake_data::Library {
		id: Some("comics".to_string()),
		name: Some("Comics".to_string()),
		..Default::default()
	}
	.insert(app.conn())
	.await;
	let novels = fake_data::Library {
		id: Some("novels".to_string()),
		name: Some("Novels".to_string()),
		..Default::default()
	}
	.insert(app.conn())
	.await;
	setup_single_series_with_n_books(
		&app,
		fake_data::Series {
			id: Some("saga".to_string()),
			name: Some("Saga".to_string()),
			library_id: Some(comics.id.clone()),
			..Default::default()
		},
		1,
	)
	.await;
	setup_single_series_with_n_books(
		&app,
		fake_data::Series {
			id: Some("dune".to_string()),
			name: Some("Dune".to_string()),
			library_id: Some(novels.id.clone()),
			..Default::default()
		},
		1,
	)
	.await;

	let devices = app.ctx.devices();
	let (narrow, narrow_issued) = devices
		.create_device(&owner, narrow_kind, Some("Narrow".to_string()))
		.await
		.expect("narrow device");
	let (_, wide_issued) = devices
		.create_device(&owner, wide_kind, Some("Wide".to_string()))
		.await
		.expect("wide device");
	devices
		.set_library_scope(
			&owner,
			&narrow.id,
			LibraryScope::Only(vec![comics.id.clone()]),
		)
		.await
		.expect("scope");

	Fixture {
		app,
		username: user.username,
		narrow_secret: narrow_issued.secret,
		wide_secret: wide_issued.secret,
		narrow_device: narrow.id,
		comics: comics.id,
		novels: novels.id,
	}
}

fn basic(username: &str, secret: &str) -> String {
	format!("Basic {}", STANDARD.encode(format!("{username}:{secret}")))
}

/// Komga's library and series lists are filtered by the authenticating
/// device's scope, not just by the user's exclusions.
#[cfg(feature = "komga")]
#[tokio::test]
async fn komga_library_and_series_lists_follow_the_devices_scope() {
	let fx = fixture(DeviceKind::Komelia, DeviceKind::Komelia).await;

	let ids = |body: serde_json::Value| -> Vec<String> {
		body.as_array()
			.unwrap_or_else(|| panic!("library array: {body}"))
			.iter()
			.map(|library| library["id"].as_str().expect("id").to_string())
			.collect()
	};

	let narrow: serde_json::Value = fx
		.app
		.server
		.get("/api/v1/libraries")
		.add_header(
			header::AUTHORIZATION,
			basic(&fx.username, &fx.narrow_secret),
		)
		.await
		.json();
	assert_eq!(ids(narrow), vec![fx.comics.clone()]);

	let wide: serde_json::Value = fx
		.app
		.server
		.get("/api/v1/libraries")
		.add_header(header::AUTHORIZATION, basic(&fx.username, &fx.wide_secret))
		.await
		.json();
	let mut wide = ids(wide);
	wide.sort();
	assert_eq!(wide, vec![fx.comics.clone(), fx.novels.clone()]);

	let series_names = |body: serde_json::Value| -> Vec<String> {
		body["content"]
			.as_array()
			.unwrap_or_else(|| panic!("series page: {body}"))
			.iter()
			.map(|series| series["name"].as_str().expect("name").to_string())
			.collect()
	};

	let narrow_series: serde_json::Value = fx
		.app
		.server
		.get("/api/v1/series")
		.add_header(
			header::AUTHORIZATION,
			basic(&fx.username, &fx.narrow_secret),
		)
		.await
		.json();
	assert_eq!(series_names(narrow_series), vec!["Saga".to_string()]);

	let wide_series: serde_json::Value = fx
		.app
		.server
		.get("/api/v1/series")
		.add_header(header::AUTHORIZATION, basic(&fx.username, &fx.wide_secret))
		.await
		.json();
	let mut wide_series = series_names(wide_series);
	wide_series.sort();
	assert_eq!(wide_series, vec!["Dune".to_string(), "Saga".to_string()]);
}

/// The Kavita profile resolves `Library/libraries` through the same funnel.
#[cfg(feature = "kavita")]
#[tokio::test]
async fn kavita_library_list_follows_the_devices_scope() {
	let fx = fixture(DeviceKind::Api, DeviceKind::Api).await;

	let names = |body: serde_json::Value| -> Vec<String> {
		body.as_array()
			.unwrap_or_else(|| panic!("library array: {body}"))
			.iter()
			.map(|library| library["name"].as_str().expect("name").to_string())
			.collect()
	};

	let narrow: serde_json::Value = fx
		.app
		.server
		.get("/api/Library/libraries")
		.add_header("x-api-key", fx.narrow_secret.clone())
		.await
		.json();
	assert_eq!(names(narrow), vec!["Comics".to_string()]);

	let wide: serde_json::Value = fx
		.app
		.server
		.get("/api/Library/libraries")
		.add_header("x-api-key", fx.wide_secret.clone())
		.await
		.json();
	let mut wide = names(wide);
	wide.sort();
	assert_eq!(wide, vec!["Comics".to_string(), "Novels".to_string()]);
}

/// The OPDS v1.2 library feed, reached with the device key in the path, is
/// scoped the same way.
#[cfg(feature = "opds")]
#[tokio::test]
async fn opds_v1_2_library_feed_follows_the_devices_scope() {
	let fx = fixture(DeviceKind::Opds, DeviceKind::Opds).await;

	let narrow = fx
		.app
		.server
		.get(&format!("/opds/{}/v1.2/libraries", fx.narrow_secret))
		.await;
	narrow.assert_status_ok();
	let narrow = narrow.text();
	assert!(narrow.contains("Comics"), "{narrow}");
	assert!(
		!narrow.contains("Novels"),
		"a scoped device must not see the other library: {narrow}"
	);

	let wide = fx
		.app
		.server
		.get(&format!("/opds/{}/v1.2/libraries", fx.wide_secret))
		.await;
	wide.assert_status_ok();
	let wide = wide.text();
	assert!(wide.contains("Comics"), "{wide}");
	assert!(wide.contains("Novels"), "{wide}");
}

/// Nothing caches an authorization decision across a scope write: the very
/// next request on the same Basic credential — which the Basic cache would
/// otherwise short-circuit for its whole TTL — sees the new scope.
#[cfg(feature = "komga")]
#[tokio::test]
async fn a_scope_change_is_in_force_on_the_next_request() {
	let fx = fixture(DeviceKind::Komelia, DeviceKind::Komelia).await;
	let header_value = basic(&fx.username, &fx.narrow_secret);

	let count = |body: serde_json::Value| body.as_array().expect("libraries").len();

	let before: serde_json::Value = fx
		.app
		.server
		.get("/api/v1/libraries")
		.add_header(header::AUTHORIZATION, header_value.clone())
		.await
		.json();
	assert_eq!(count(before), 1);

	// Widen the device with the server owner's authority, then repeat the
	// identical request.
	let owner = AuthUser {
		id: "unused".to_string(),
		is_server_owner: true,
		..Default::default()
	};
	fx.app
		.ctx
		.devices()
		.set_library_scope(&owner, &fx.narrow_device, LibraryScope::Inherit)
		.await
		.expect("scope");

	let after: serde_json::Value = fx
		.app
		.server
		.get("/api/v1/libraries")
		.add_header(header::AUTHORIZATION, header_value)
		.await
		.json();
	assert_eq!(
		count(after),
		2,
		"the same credential must not be served a pre-change scope"
	);
}
