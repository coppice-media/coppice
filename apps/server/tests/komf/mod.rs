//! Server-level Komf contract, device-permission, and route-collision tests.

use std::collections::BTreeSet;

use axum::http::StatusCode;
use axum_test::TestResponse;
use migrations::{Migrator, MigratorTrait};
use models::{
	entity::{library_config, metadata_provider_config, series_metadata, user::AuthUser},
	shared::enums::{DeviceKind, LibraryType, MetadataProvider, UserPermission},
};
use sea_orm::{
	ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait,
	IntoActiveModel, QueryFilter, Set,
};
use serde_json::{json, Value};
use stump_core::config::StumpConfig;
use stump_devices::{CredentialIssuance, LibraryScope};
use stump_komf::{KomfJob, KomfJobPage, KomfMetadataJobResponse};
use tests::fake_data;

use crate::common::{series::setup_single_series_with_n_books, TestApp};

const LIBRARY_ID: &str = "422a4c5e-2876-487b-9ef8-1c3ca91be73e";
const HIDDEN_LIBRARY_ID: &str = "hidden-library";
const SERIES_ID: &str = "ec35b856-0014-55e7-94ed-cf8261536369";
const CONFIG_SECRET: &str = "komf-config-secret-must-not-be-returned";

struct Fixture {
	app: TestApp,
	admin_token: String,
	library_id: String,
	series_id: String,
	download_key: String,
	edit_key: String,
}

async fn migrated_database() -> DatabaseConnection {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("connect in-memory sqlite");
	Migrator::up(&db, None)
		.await
		.expect("migrate Komf test database");
	db
}

async fn app(abs: bool, kavita: bool) -> TestApp {
	let mut config = StumpConfig::debug();
	config.protocols.enable_abs = abs;
	config.protocols.enable_kavita = kavita;
	config.protocols.enable_komga = true;
	config.protocols.enable_komf = true;
	config.protocols.enable_kobo_sync = false;
	config.protocols.enable_koreader_sync = false;
	config.protocols.enable_webui = false;
	TestApp::with_parts(migrated_database().await, config).await
}

async fn set_library_type(
	conn: &DatabaseConnection,
	library_id: &str,
	library_type: LibraryType,
) {
	let model = library_config::Entity::find()
		.filter(library_config::Column::LibraryId.eq(Some(library_id.to_string())))
		.one(conn)
		.await
		.expect("query library config")
		.expect("library config exists");
	let mut active = model.into_active_model();
	active.library_type = Set(library_type);
	active.update(conn).await.expect("update library type");
}

async fn add_provider(
	conn: &DatabaseConnection,
	provider: MetadataProvider,
	enabled: bool,
	encrypted_api_token: Option<String>,
) {
	metadata_provider_config::ActiveModel {
		provider_type: Set(provider),
		enabled: Set(enabled),
		encrypted_api_token: Set(encrypted_api_token),
		api_token_expires_at: Set(None),
		auto_apply_config: Set(None),
		..Default::default()
	}
	.insert(conn)
	.await
	.expect("insert metadata provider config");
}

async fn fixture() -> Fixture {
	let app = app(false, false).await;
	let admin_token = app.create_initial_account().await;
	let user = fake_data::User::new("komf-device-owner")
		.insert(app.conn())
		.await;
	let owner = AuthUser {
		id: user.id.clone(),
		username: user.username.clone(),
		is_server_owner: true,
		permissions: vec![UserPermission::EditMetadata],
		..Default::default()
	};
	let visible_library = fake_data::Library {
		id: Some(LIBRARY_ID.to_string()),
		name: Some("Synthetic Capture Library".to_string()),
		path: Some("/data/library".to_string()),
	}
	.insert(app.conn())
	.await;
	let hidden_library = fake_data::Library {
		id: Some(HIDDEN_LIBRARY_ID.to_string()),
		name: Some("Hidden Library".to_string()),
		..Default::default()
	}
	.insert(app.conn())
	.await;
	set_library_type(app.conn(), &visible_library.id, LibraryType::Comic).await;
	set_library_type(app.conn(), &hidden_library.id, LibraryType::Comic).await;
	let (series, _) = setup_single_series_with_n_books(
		&app,
		fake_data::Series {
			id: Some(SERIES_ID.to_string()),
			name: Some("Fruits Basket".to_string()),
			library_id: Some(visible_library.id.clone()),
			..Default::default()
		},
		1,
	)
	.await;

	let devices = app.ctx.devices();
	let (download_device, download) = devices
		.create_device(
			&owner,
			CredentialIssuance::InteractiveSession,
			DeviceKind::Komelia,
			Some("Komelia download-only".to_string()),
		)
		.await
		.expect("create download-only Komelia device");
	let (edit_device, edit) = devices
		.create_komelia_device(
			&owner,
			CredentialIssuance::InteractiveSession,
			Some("Komelia metadata editor".to_string()),
			true,
		)
		.await
		.expect("create EditMetadata Komelia device");
	for device_id in [&download_device.id, &edit_device.id] {
		devices
			.set_library_scope(
				&owner,
				device_id,
				LibraryScope::Only(vec![visible_library.id.clone()]),
			)
			.await
			.expect("narrow Komelia device scope");
	}

	// MAL is enabled but lacks its required client id. It is safely excluded
	// from Comic-library search and fails before network I/O in Manga jobs.
	add_provider(app.conn(), MetadataProvider::Mal, true, None).await;
	// A stored provider credential must never leak from the Komf config DTO.
	add_provider(
		app.conn(),
		MetadataProvider::MangaDex,
		false,
		Some(CONFIG_SECRET.to_string()),
	)
	.await;

	Fixture {
		app,
		admin_token,
		library_id: visible_library.id,
		series_id: series.id,
		download_key: download.secret,
		edit_key: edit.secret,
	}
}

fn api_key_path(path: &str, key: &str) -> String {
	let separator = if path.contains('?') { '&' } else { '?' };
	format!("{path}{separator}apiKey={}", urlencoding::encode(key))
}

async fn get_key(app: &TestApp, path: &str, key: &str) -> TestResponse {
	app.server.get(&api_key_path(path, key)).await
}

fn response_fixtures() -> Value {
	serde_json::from_str(include_str!(
		"../../../../crates/komf/tests/fixtures/komf-responses.json"
	))
	.expect("valid Komf route response fixtures")
}

fn object_keys(value: &Value) -> BTreeSet<String> {
	value
		.as_object()
		.expect("JSON object")
		.keys()
		.cloned()
		.collect()
}

#[tokio::test]
async fn komf_download_key_reads_routes_and_honors_library_scope() {
	let fixture = fixture().await;
	let expected = response_fixtures();

	let providers = get_key(
		&fixture.app,
		"/api/komga/metadata/providers",
		&fixture.download_key,
	)
	.await;
	providers.assert_status_ok();
	let providers: Value = providers.json();
	assert_eq!(providers, expected["providers"]);

	let config_response =
		get_key(&fixture.app, "/api/config", &fixture.download_key).await;
	config_response.assert_status_ok();
	let config: Value = config_response.json();
	assert!(config["metadataProviders"].is_object());
	for (key, value) in expected["configSecretFields"]
		.as_object()
		.expect("redacted config fixture")
	{
		assert_eq!(&config["metadataProviders"][key], value);
	}
	assert_eq!(
		config["metadataProviders"]["defaultProviders"]["mal"]["enabled"],
		true
	);
	assert!(!config.to_string().contains(CONFIG_SECRET));

	let connected = get_key(
		&fixture.app,
		"/api/komga/media-server/connected",
		&fixture.download_key,
	)
	.await;
	connected.assert_status_ok();
	assert_eq!(connected.json::<Value>(), expected["mediaServerConnection"]);

	let libraries = get_key(
		&fixture.app,
		"/api/komga/media-server/libraries",
		&fixture.download_key,
	)
	.await;
	libraries.assert_status_ok();
	assert_eq!(libraries.json::<Value>(), expected["mediaServerLibraries"]);

	let scoped_providers = get_key(
		&fixture.app,
		&format!(
			"/api/komga/metadata/providers?libraryId={}",
			fixture.library_id
		),
		&fixture.download_key,
	)
	.await;
	scoped_providers.assert_status_ok();
	assert_eq!(scoped_providers.json::<Value>(), json!([]));
	let hidden_providers = get_key(
		&fixture.app,
		"/api/komga/metadata/providers?libraryId=hidden-library",
		&fixture.download_key,
	)
	.await;
	hidden_providers.assert_status(StatusCode::NOT_FOUND);

	let search = get_key(
		&fixture.app,
		&format!(
			"/api/komga/metadata/search?name=Fruits%20Basket&libraryId={}",
			fixture.library_id
		),
		&fixture.download_key,
	)
	.await;
	search.assert_status_ok();
	assert_eq!(search.json::<Value>(), json!([]));

	// Comic libraries do not overlap MAL. This is an ordinary missing-cover
	// response, not an authorization failure for the read-only device.
	let cover = get_key(
		&fixture.app,
		&format!(
			"/api/komga/metadata/series-cover?libraryId={}&provider=MAL&providerSeriesId=1",
			fixture.library_id
		),
		&fixture.download_key,
	)
	.await;
	cover.assert_status(StatusCode::NOT_FOUND);

	// The edit credential creates a normal Komf job. The same owner-bound
	// download-only key can read that job and its event stream.
	let start = fixture
		.app
		.server
		.post(&api_key_path(
			&format!(
				"/api/komga/metadata/match/library/{}/series/{}",
				fixture.library_id, fixture.series_id
			),
			&fixture.edit_key,
		))
		.await;
	start.assert_status_ok();
	let start_body: Value = start.json();
	let _: KomfMetadataJobResponse = serde_json::from_value(start_body.clone())
		.expect("match response follows Komf DTO");
	assert_eq!(
		object_keys(&start_body),
		object_keys(&expected["metadataJobResponse"])
	);
	let job_id = start_body["jobId"].as_str().expect("job id");

	let jobs = get_key(&fixture.app, "/api/jobs", &fixture.download_key).await;
	jobs.assert_status_ok();
	let jobs: Value = jobs.json();
	let _: KomfJobPage =
		serde_json::from_value(jobs.clone()).expect("jobs page follows DTO");
	assert_eq!(object_keys(&jobs), object_keys(&expected["jobs"]));
	assert_eq!(
		object_keys(&jobs["content"][0]),
		object_keys(&expected["jobs"]["content"][0])
	);

	let job = get_key(
		&fixture.app,
		&format!("/api/jobs/{job_id}"),
		&fixture.download_key,
	)
	.await;
	job.assert_status_ok();
	let job: Value = job.json();
	let _: KomfJob = serde_json::from_value(job.clone()).expect("job follows Komf DTO");
	assert_eq!(
		object_keys(&job),
		object_keys(&expected["jobs"]["content"][0])
	);

	let events = get_key(
		&fixture.app,
		&format!("/api/jobs/{job_id}/events"),
		&fixture.download_key,
	)
	.await;
	events.assert_status_ok();
	assert!(
		events.text().is_empty(),
		"no provider means no progress events"
	);

	let delete = fixture
		.app
		.server
		.delete(&api_key_path("/api/jobs/all", &fixture.edit_key))
		.await;
	delete.assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn komf_device_permission_matrix_separates_metadata_edits_from_provider_config() {
	let fixture = fixture().await;
	let identify_path = "/api/komga/metadata/identify";
	let identify_body = json!({
		"libraryId": fixture.library_id,
		"seriesId": fixture.series_id,
		"provider": "MAL",
		"providerSeriesId": "42"
	});
	let write_paths = [
		format!(
			"/api/komga/metadata/match/library/{}/series/{}",
			fixture.library_id, fixture.series_id
		),
		format!("/api/komga/metadata/match/library/{}", fixture.library_id),
		format!(
			"/api/komga/metadata/reset/library/{}/series/{}",
			fixture.library_id, fixture.series_id
		),
		format!("/api/komga/metadata/reset/library/{}", fixture.library_id),
	];

	let identify_denied = fixture
		.app
		.server
		.post(&api_key_path(identify_path, &fixture.download_key))
		.json(&identify_body)
		.await;
	identify_denied.assert_status(StatusCode::FORBIDDEN);
	for path in &write_paths {
		let denied = fixture
			.app
			.server
			.post(&api_key_path(path, &fixture.download_key))
			.await;
		denied.assert_status(StatusCode::FORBIDDEN);
	}
	let delete_jobs_denied = fixture
		.app
		.server
		.delete(&api_key_path("/api/jobs/all", &fixture.download_key))
		.await;
	delete_jobs_denied.assert_status(StatusCode::FORBIDDEN);
	let patch_body = json!({});
	let patch_download = fixture
		.app
		.server
		.patch(&api_key_path("/api/config", &fixture.download_key))
		.json(&patch_body)
		.await;
	patch_download.assert_status(StatusCode::FORBIDDEN);
	let patch_edit = fixture
		.app
		.server
		.patch(&api_key_path("/api/config", &fixture.edit_key))
		.json(&patch_body)
		.await;
	patch_edit.assert_status(StatusCode::FORBIDDEN);

	let identify = fixture
		.app
		.server
		.post(&api_key_path(identify_path, &fixture.edit_key))
		.json(&identify_body)
		.await;
	identify.assert_status_ok();
	let identify_body: Value = identify.json();
	let _: KomfMetadataJobResponse = serde_json::from_value(identify_body)
		.expect("identify response follows Komf DTO");

	let match_series = fixture
		.app
		.server
		.post(&api_key_path(&write_paths[0], &fixture.edit_key))
		.await;
	match_series.assert_status_ok();
	let match_series: Value = match_series.json();
	let _: KomfMetadataJobResponse =
		serde_json::from_value(match_series).expect("match response follows Komf DTO");

	let match_library = fixture
		.app
		.server
		.post(&api_key_path(&write_paths[1], &fixture.edit_key))
		.await;
	match_library.assert_status(StatusCode::ACCEPTED);

	series_metadata::ActiveModel {
		series_id: Set(fixture.series_id.clone()),
		title: Set(Some("Metadata before reset".to_string())),
		..Default::default()
	}
	.insert(fixture.app.conn())
	.await
	.expect("insert metadata for series reset");
	let reset_series = fixture
		.app
		.server
		.post(&api_key_path(&write_paths[2], &fixture.edit_key))
		.await;
	reset_series.assert_status(StatusCode::NO_CONTENT);
	assert!(series_metadata::Entity::find_by_id(&fixture.series_id)
		.one(fixture.app.conn())
		.await
		.expect("query reset series metadata")
		.is_none());

	series_metadata::ActiveModel {
		series_id: Set(fixture.series_id.clone()),
		title: Set(Some("Metadata before library reset".to_string())),
		..Default::default()
	}
	.insert(fixture.app.conn())
	.await
	.expect("insert metadata for library reset");
	let reset_library = fixture
		.app
		.server
		.post(&api_key_path(&write_paths[3], &fixture.edit_key))
		.await;
	reset_library.assert_status(StatusCode::NO_CONTENT);
	assert!(series_metadata::Entity::find_by_id(&fixture.series_id)
		.one(fixture.app.conn())
		.await
		.expect("query reset library metadata")
		.is_none());

	let delete_jobs = fixture
		.app
		.server
		.delete(&api_key_path("/api/jobs/all", &fixture.edit_key))
		.await;
	delete_jobs.assert_status(StatusCode::NO_CONTENT);

	let session_admin_patch = fixture
		.app
		.server
		.patch("/api/config")
		.add_header("Authorization", format!("Bearer {}", fixture.admin_token))
		.json(&patch_body)
		.await;
	session_admin_patch.assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn komf_job_sse_replays_provider_progress_and_failure_sequence() {
	let fixture = fixture().await;
	set_library_type(fixture.app.conn(), &fixture.library_id, LibraryType::Manga).await;
	let start = fixture
		.app
		.server
		.post(&api_key_path(
			&format!(
				"/api/komga/metadata/match/library/{}/series/{}",
				fixture.library_id, fixture.series_id
			),
			&fixture.edit_key,
		))
		.await;
	start.assert_status_ok();
	let start_body: Value = start.json();
	let job_id = start_body["jobId"].as_str().expect("job id");

	let events = get_key(
		&fixture.app,
		&format!("/api/jobs/{job_id}/events"),
		&fixture.download_key,
	)
	.await;
	events.assert_status_ok();
	let body = events.text();
	let event_names: Vec<_> = body
		.lines()
		.filter_map(|line| line.strip_prefix("event: "))
		.collect();
	assert_eq!(
		event_names,
		[
			"ProviderSeriesEvent",
			"ProviderErrorEvent",
			"ProcessingErrorEvent"
		]
	);

	let job = get_key(
		&fixture.app,
		&format!("/api/jobs/{job_id}"),
		&fixture.download_key,
	)
	.await;
	job.assert_status_ok();
	let job: Value = job.json();
	assert_eq!(job["status"], "FAILED");
	assert!(job["message"]
		.as_str()
		.unwrap()
		.contains("providers failed"));
}

#[cfg(all(feature = "abs", feature = "kavita", feature = "komga"))]
#[tokio::test]
async fn abs_kavita_komga_and_komf_mount_together_without_route_collisions() {
	let app = app(true, true).await;
	let config_probe = app.server.get("/api/config").await;
	config_probe.assert_status(StatusCode::UNAUTHORIZED);
}
