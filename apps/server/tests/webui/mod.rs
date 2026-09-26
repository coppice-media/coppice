use axum::http::StatusCode;
use tempfile::TempDir;
use tests::db::test_database;

use crate::common::TestApp;

#[tokio::test]
async fn webui_redirect_preserves_home_and_api_routes() {
	let home_dir = TempDir::new().expect("home app directory");
	std::fs::write(home_dir.path().join("index.html"), "<h1>Home app</h1>")
		.expect("write Home app index");

	let mut config = stump_core::config::StumpConfig::debug();
	config.protocols.enable_webui = true;
	config.protocols.enable_playground = true;
	config.server.home_app_dir = Some(home_dir.path().to_string_lossy().into_owned());
	let app = TestApp::with_parts(test_database().await, config).await;

	let root = app.server.get("/").await;
	root.assert_status(StatusCode::SEE_OTHER);
	assert_eq!(root.headers()["location"], "/app");

	let unknown_browser_path = app.server.get("/a/browser/deep-link").await;
	unknown_browser_path.assert_status(StatusCode::SEE_OTHER);
	assert_eq!(unknown_browser_path.headers()["location"], "/app");

	for client_path in ["/v1/not-a-route", "/kobo/not-a-route", "/komga/not-a-route"] {
		let response = app.server.get(client_path).await;
		assert_eq!(
			response.status_code(),
			StatusCode::NOT_FOUND,
			"{client_path} must be an API 404, not the browser redirect"
		);
	}

	let home = app.server.get("/app/").await;
	home.assert_status_ok();
	assert_eq!(home.text(), "<h1>Home app</h1>");

	let playground = app.server.get("/api/graphql").await;
	playground.assert_status(StatusCode::SEE_OTHER);
	assert_eq!(
		playground.headers()["location"],
		"/app/login?returnTo=%2Fapi%2Fgraphql"
	);

	let health = app.server.get("/api/v2/health").await;
	health.assert_status_ok();
	let health: serde_json::Value = health.json();
	assert_eq!(health["dependencies"]["database"]["status"], "ok");
	assert!(health["dependencies"].get("spa").is_none());
}
