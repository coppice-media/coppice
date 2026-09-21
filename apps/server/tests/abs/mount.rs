use migrations::{Migrator, MigratorTrait};
use sea_orm::{Database, DatabaseConnection};
use serde_json::{json, Value};
use stump_core::config::StumpConfig;

use crate::common::TestApp;

/// The profile's own tables (`abs_ids`, `abs_sessions`) have no SeaORM
/// entities, so the entity-built schema cannot serve them: this suite runs
/// the real migrations, as the liseur-sync suite does.
async fn migrated_database() -> DatabaseConnection {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("failed to connect to sqlite");
	Migrator::up(&db, None)
		.await
		.expect("failed to run migrations");
	db
}

async fn app() -> TestApp {
	TestApp::with_parts(migrated_database().await, StumpConfig::debug()).await
}

/// A server with the profile turned off, to prove the switch is the switch.
async fn app_without_abs() -> TestApp {
	let mut config = StumpConfig::debug();
	config.protocols.enable_abs = false;
	TestApp::with_parts(migrated_database().await, config).await
}

/// The profile's probes answer at the server root, unauthenticated, because
/// an Audiobookshelf client builds absolute paths from a bare host.
#[tokio::test]
async fn abs_probes_are_mounted_at_the_root() {
	let app = app().await;

	let status = app.server.get("/status").await;
	status.assert_status_ok();
	let body: Value = status.json();
	assert_eq!(body["app"], "audiobookshelf");
	assert_eq!(body["authMethods"], json!(["local"]));

	let ping = app.server.get("/ping").await;
	ping.assert_status_ok();
	assert_eq!(ping.json::<Value>(), json!({ "success": true }));
}

/// `STUMP_ENABLE_ABS=false` keeps the compiled route surface behind the
/// runtime component gate. This permits a HOT transition without rebuilding
/// the Axum router while returning a stable, machine-readable 503.
#[tokio::test]
async fn abs_routes_are_gated_when_the_profile_is_disabled() {
	let app = app_without_abs().await;

	for path in ["/status", "/socket.io/?EIO=4&transport=websocket"] {
		let response = app.server.get(path).await;
		assert_eq!(
			response.status_code(),
			axum::http::StatusCode::SERVICE_UNAVAILABLE,
			"{path}"
		);
		assert_eq!(
			response.json::<Value>(),
			json!({"component": "abs", "error": "component_disabled"})
		);
	}
}

/// The socket endpoint exists and takes no credential on the upgrade: the
/// app's socket.io client cannot set a header there, so the token arrives in
/// an `auth` event instead. A plain GET carries no upgrade headers, so the
/// extractor rejects it with `400` — which is the proof that the route is
/// mounted (not `404`) *and* that the auth middleware is not in front of it
/// (not `401`).
#[tokio::test]
async fn abs_socket_io_is_mounted_without_authentication() {
	let app = app().await;

	for path in ["/socket.io", "/socket.io/?EIO=4&transport=websocket"] {
		let response = app.server.get(path).await;
		assert_eq!(
			response.status_code(),
			axum::http::StatusCode::BAD_REQUEST,
			"{path}"
		);
	}
}

/// The authenticated surface is behind the profile's own credential rules,
/// and the routes the official app calls on launch are all there.
#[tokio::test]
async fn abs_app_routes_answer_for_a_logged_in_user() {
	let app = app().await;
	let token = app.create_initial_account().await;
	*app.access_token.write().await = Some(token);

	// A Stump access token authenticates the profile too.
	let me = app.get("/api/me").await;
	me.assert_status_ok();
	let me: Value = me.json();
	assert_eq!(me["username"], "initial-server-admin");
	assert!(me["mediaProgress"].is_array());
	assert!(me["bookmarks"].is_array());

	// The launch screens: an empty library must answer empties, never 404s.
	let in_progress = app.get("/api/me/items-in-progress").await;
	in_progress.assert_status_ok();
	assert_eq!(in_progress.json::<Value>()["libraryItems"], json!([]));

	let sessions = app.get("/api/me/listening-sessions").await;
	sessions.assert_status_ok();
	let sessions: Value = sessions.json();
	assert_eq!(sessions["total"], 0);
	assert_eq!(sessions["sessions"], json!([]));

	let stats = app.get("/api/me/listening-stats").await;
	stats.assert_status_ok();
	let stats: Value = stats.json();
	assert_eq!(stats["totalTime"], 0.0);
	assert_eq!(stats["recentSessions"], json!([]));

	let libraries = app.get("/api/libraries").await;
	libraries.assert_status_ok();
	assert!(libraries.json::<Value>()["libraries"].is_array());
}

/// Every one of those needs a credential; the profile answers `401`, not a
/// silent empty page, so a client knows to log in.
#[tokio::test]
async fn abs_app_routes_reject_an_anonymous_request() {
	let app = app().await;
	let token = app.create_initial_account().await;
	*app.access_token.write().await = Some(token);

	for path in [
		"/api/me",
		"/api/me/items-in-progress",
		"/api/me/listening-sessions",
		"/api/me/listening-stats",
	] {
		let response = app.server.get(path).await;
		assert_eq!(
			response.status_code(),
			axum::http::StatusCode::UNAUTHORIZED,
			"{path}"
		);
	}
}

/// An unknown `/api/...` path is a JSON `404`, never the web UI shell.
///
/// On a `full` build the SPA's `fallback` becomes the *server's* fallback, so
/// before `NON_SPA_PREFIXES` every unmatched `/api/...` request answered
/// `200 text/html`. On a real phone that made `GET /api/items/{id}/download`
/// and `GET /api/items/{id}/ebook` look like working routes that returned the
/// Nuxt shell instead of a file.
#[cfg(feature = "webui")]
#[tokio::test]
async fn an_unknown_api_path_is_a_json_404_and_not_the_spa() {
	let app = app().await;
	let token = app.create_initial_account().await;
	*app.access_token.write().await = Some(token.clone());

	for path in [
		"/api/x",
		"/api/items/does-not-exist/nonsense",
		"/api/definitely/not/a/route",
	] {
		let response = app
			.server
			.get(path)
			.add_header("authorization", format!("Bearer {token}"))
			.await;
		assert_eq!(
			response.status_code(),
			axum::http::StatusCode::NOT_FOUND,
			"{path}"
		);
		let content_type = response
			.headers()
			.get("content-type")
			.and_then(|value| value.to_str().ok())
			.unwrap_or_default()
			.to_owned();
		assert!(
			content_type.starts_with("application/json"),
			"{path} answered {content_type}, not JSON"
		);
		assert!(
			!String::from_utf8_lossy(&response.as_bytes().to_vec())
				.to_lowercase()
				.contains("<html"),
			"{path} answered the SPA shell"
		);
	}
}

/// The routes the official app needs that the SPA fallback used to swallow
/// are mounted, and the public track lane takes no credential.
#[tokio::test]
async fn abs_download_ebook_and_public_track_routes_are_mounted() {
	let app = app().await;
	let token = app.create_initial_account().await;
	*app.access_token.write().await = Some(token.clone());

	// A real item id is not needed: what is asserted is that the router
	// matches the path at all, i.e. answers the profile's `404`, not the SPA.
	for path in [
		"/api/items/missing/download",
		"/api/items/missing/ebook",
		"/api/items/missing/ebook/some-file",
		"/api/items/missing/file/0/download",
	] {
		let response = app
			.server
			.get(path)
			.add_header("authorization", format!("Bearer {token}"))
			.await;
		assert_eq!(
			response.status_code(),
			axum::http::StatusCode::NOT_FOUND,
			"{path}"
		);
	}

	// `/public/session/{id}/track/{index}` is outside the auth middleware:
	// an anonymous request reaches the handler and 404s on the session id
	// rather than being refused with a 401.
	let response = app
		.server
		.get("/public/session/00000000-0000-0000-0000-000000000000/track/1")
		.await;
	assert_eq!(response.status_code(), axum::http::StatusCode::NOT_FOUND);

	// So is the cover route, from Audiobookshelf 2.17.0 on.
	let response = app.server.get("/api/items/missing/cover").await;
	assert_eq!(response.status_code(), axum::http::StatusCode::NOT_FOUND);
}

/// Playlists are mounted and answer the app's envelope.
#[tokio::test]
async fn abs_playlist_routes_are_mounted() {
	let app = app().await;
	let token = app.create_initial_account().await;
	*app.access_token.write().await = Some(token.clone());

	let response = app
		.server
		.get("/api/playlists")
		.add_header("authorization", format!("Bearer {token}"))
		.await;
	response.assert_status_ok();
	assert!(response.json::<Value>()["results"].is_array());

	// `POST /api/playlists` was a 405 before this batch.
	let response = app
		.server
		.post("/api/playlists")
		.add_header("authorization", format!("Bearer {token}"))
		.json(&json!({ "items": [], "name": "Commute" }))
		.await;
	response.assert_status_ok();
	assert_eq!(response.json::<Value>()["name"], "Commute");
}
