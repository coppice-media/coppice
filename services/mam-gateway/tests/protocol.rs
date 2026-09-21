use axum::{
	body::Bytes,
	extract::State,
	http::{header, HeaderMap, StatusCode},
	response::IntoResponse,
	routing::{get, post},
	Router,
};
use mam_gateway::{config::GatewayConfig, router, AppState};
use serde::Deserialize;
use serde_json::json;
use std::{
	fs,
	net::SocketAddr,
	sync::{
		atomic::{AtomicBool, AtomicUsize, Ordering},
		Arc,
	},
};
use tempfile::tempdir;
use tokio::net::TcpListener;
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrabWire {
	grab_id: String,
	result_id: String,
	status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusWire {
	grab_id: String,
	result_id: String,
	status: String,
	handoff: Option<HandoffWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HandoffWire {
	relative_path: String,
	filename: String,
	sha256: String,
	byte_size: u64,
	media_kind: String,
}
const SEARCH_FIXTURE: &str = include_str!("fixtures/mam/search.json");

#[derive(Clone)]
struct MamFixtureState {
	cookie_seen: Arc<AtomicBool>,
}

#[derive(Clone)]
struct QbitFixtureState {
	info_calls: Arc<AtomicUsize>,
	add_calls: Arc<AtomicUsize>,
	content_path: String,
}

#[tokio::test]
async fn torznab_and_opaque_grab_flow_hides_tracker_secrets_and_deduplicates() {
	let mam_state = MamFixtureState {
		cookie_seen: Arc::new(AtomicBool::new(false)),
	};
	let handoff_root = tempdir().expect("handoff root");
	let handoff_file = handoff_root.path().join("fixture.epub");
	fs::write(&handoff_file, b"fixture ebook bytes").expect("handoff file");
	let qbit_state = QbitFixtureState {
		info_calls: Arc::new(AtomicUsize::new(0)),
		add_calls: Arc::new(AtomicUsize::new(0)),
		content_path: handoff_file.to_string_lossy().into_owned(),
	};
	let mam_server = Router::new()
		.route("/tor/js/loadSearchJSON.php", post(mam_search))
		.route("/fixture.torrent", get(mam_download))
		.with_state(mam_state.clone());
	let qbit_server = Router::new()
		.route("/api/v2/torrents/info", get(qbit_info))
		.route("/api/v2/torrents/add", post(qbit_add))
		.with_state(qbit_state.clone());
	let (mam_addr, mam_task) = spawn_server(mam_server).await;
	let (qbit_addr, qbit_task) = spawn_server(qbit_server).await;

	let config = GatewayConfig::for_tests(
		"127.0.0.1:0".parse().expect("bind"),
		Url::parse(&format!("http://{mam_addr}")).expect("mam url"),
		Url::parse(&format!("http://127.0.0.1:{}", qbit_addr.port())).expect("qbit url"),
		"gateway-secret",
		"mam-secret-fixture",
	)
	.and_then(|config| config.with_handoff_root(handoff_root.path().to_owned()))
	.expect("config");
	let gateway_listener = TcpListener::bind(config.bind_addr())
		.await
		.expect("gateway bind");
	let gateway_addr = gateway_listener.local_addr().expect("gateway address");
	let gateway_task = tokio::spawn(async move {
		axum::serve(
			gateway_listener,
			router(AppState::new(config).expect("state")),
		)
		.await
		.expect("gateway server");
	});
	let client = reqwest::Client::new();
	let base = format!("http://{gateway_addr}");
	let caps = client
		.get(format!("{base}/torznab/api"))
		.query(&[("t", "caps")])
		.bearer_auth("gateway-secret")
		.send()
		.await
		.expect("caps request");
	assert_eq!(caps.status(), StatusCode::OK);
	let caps_body = caps.text().await.expect("caps body");
	assert!(caps_body.contains("supportedParams=\"q,cat\""));
	assert!(!caps_body.contains("mam-secret-fixture"));

	let health = client
		.get(format!("{base}/v1/health"))
		.send()
		.await
		.expect("health request");
	assert_eq!(health.status(), StatusCode::OK);
	assert_eq!(
		health
			.json::<serde_json::Value>()
			.await
			.expect("health body")["status"],
		"ok"
	);
	let missing_auth = client
		.get(format!("{base}/torznab/api"))
		.query(&[("t", "caps")])
		.send()
		.await
		.expect("missing auth request");
	assert_eq!(missing_auth.status(), StatusCode::UNAUTHORIZED);

	let wrong_auth = client
		.get(format!("{base}/torznab/api"))
		.query(&[("t", "caps")])
		.bearer_auth("wrong-token")
		.send()
		.await
		.expect("wrong auth request");
	assert_eq!(wrong_auth.status(), StatusCode::UNAUTHORIZED);

	let search = client
		.get(format!("{base}/torznab/api"))
		.query(&[("t", "search"), ("q", "fixture")])
		.bearer_auth("gateway-secret")
		.send()
		.await
		.expect("search request");
	assert_eq!(search.status(), StatusCode::OK);
	let search_body = search.text().await.expect("search body");
	assert!(!search_body.contains("fixture-only"));
	assert!(search_body.contains("<author>Ada Lovelace, Grace Hopper</author>"));
	assert!(!search_body.contains("mam-secret-fixture"));
	let result_id = search_body
		.split_once("<guid isPermaLink=\"false\">")
		.and_then(|(_, value)| value.split_once("</guid>"))
		.map(|(value, _)| value.to_owned())
		.expect("opaque result id");
	assert_eq!(result_id.len(), 32);
	assert!(result_id.bytes().all(|byte| byte.is_ascii_hexdigit()));

	let first = client
		.post(format!("{base}/v1/grabs"))
		.bearer_auth("gateway-secret")
		.header("idempotency-key", "request-1")
		.json(&json!({ "result_id": result_id, "idempotency_key": "request-1" }))
		.send()
		.await
		.expect("grab request");
	assert_eq!(first.status(), StatusCode::CREATED);
	let first_body: serde_json::Value = first.json().await.expect("grab json");
	let first_wire: GrabWire =
		serde_json::from_value(first_body.clone()).expect("grab wire");
	assert_eq!(first_wire.grab_id.len(), 32);
	assert_eq!(first_wire.result_id, result_id);
	assert_eq!(first_wire.status, "DOWNLOADING");
	let grab_id = first_wire.grab_id.clone();
	let duplicate = client
		.post(format!("{base}/v1/grabs"))
		.bearer_auth("gateway-secret")
		.header("idempotency-key", "request-1")
		.json(
			&json!({ "result_id": first_body["resultId"], "idempotency_key": "request-1" }),
		)
		.send()
		.await
		.expect("duplicate grab request");
	assert_eq!(duplicate.status(), StatusCode::OK);
	let duplicate_body: serde_json::Value =
		duplicate.json().await.expect("duplicate json");
	assert_eq!(duplicate_body["grabId"].as_str(), Some(grab_id.as_str()));
	assert_eq!(qbit_state.add_calls.load(Ordering::SeqCst), 1);

	let status = client
		.get(format!("{base}/v1/grabs/{grab_id}"))
		.bearer_auth("gateway-secret")
		.send()
		.await
		.expect("status request");
	assert_eq!(status.status(), StatusCode::OK);
	let status_body: serde_json::Value = status.json().await.expect("status json");
	let status_wire: StatusWire =
		serde_json::from_value(status_body.clone()).expect("status wire");
	assert_eq!(status_wire.grab_id, grab_id);
	assert_eq!(status_wire.result_id, result_id);
	assert_eq!(status_wire.status, "COMPLETED");
	let handoff = status_wire.handoff.expect("handoff");
	assert_eq!(handoff.relative_path, "fixture.epub");
	assert_eq!(handoff.filename, "fixture.epub");
	assert_eq!(handoff.media_kind, "EPUB");
	assert_eq!(handoff.byte_size, 19);
	assert_eq!(
		handoff.sha256,
		"91dcac6775710e604b499efda3bab6c07a64937be5930c4a6409850839b6a4be"
	);

	let unauthorized = client
		.get(format!("{base}/v1/grabs/{grab_id}"))
		.bearer_auth("wrong-token")
		.send()
		.await
		.expect("unauthorized request");
	assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

	assert!(mam_state.cookie_seen.load(Ordering::SeqCst));
	gateway_task.abort();
	mam_task.abort();
	qbit_task.abort();
}

async fn mam_search(
	State(state): State<MamFixtureState>,
	headers: HeaderMap,
) -> impl IntoResponse {
	if headers
		.get(header::COOKIE)
		.and_then(|value| value.to_str().ok())
		== Some("mam-secret-fixture")
	{
		state.cookie_seen.store(true, Ordering::SeqCst);
	}
	(
		StatusCode::OK,
		[(header::CONTENT_TYPE, "application/json")],
		SEARCH_FIXTURE,
	)
}

async fn mam_download(headers: HeaderMap) -> impl IntoResponse {
	if headers
		.get(header::COOKIE)
		.and_then(|value| value.to_str().ok())
		!= Some("mam-secret-fixture")
	{
		return (StatusCode::UNAUTHORIZED, Bytes::new());
	}
	(StatusCode::OK, Bytes::from_static(b"fixture-torrent-bytes"))
}

async fn qbit_info(State(state): State<QbitFixtureState>) -> impl IntoResponse {
	let call = state.info_calls.fetch_add(1, Ordering::SeqCst);
	if call == 0 {
		return (StatusCode::OK, "[]").into_response();
	}
	let path = serde_json::to_string(&state.content_path).expect("path json");
	let body =
		format!("[{{\"state\":\"uploading\",\"progress\":1.0,\"content_path\":{path}}}]");
	(
		StatusCode::OK,
		[(header::CONTENT_TYPE, "application/json")],
		body,
	)
		.into_response()
}

async fn qbit_add(
	State(state): State<QbitFixtureState>,
	body: Bytes,
) -> impl IntoResponse {
	let body = String::from_utf8_lossy(&body);
	if !body.contains("coppice:") || !body.contains("fixture-torrent-bytes") {
		return (StatusCode::BAD_REQUEST, "Fails.").into_response();
	}
	state.add_calls.fetch_add(1, Ordering::SeqCst);
	(StatusCode::OK, "Ok.").into_response()
}

async fn spawn_server(app: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
	let listener = TcpListener::bind("127.0.0.1:0")
		.await
		.expect("fixture bind");
	let addr = listener.local_addr().expect("fixture address");
	let task = tokio::spawn(async move {
		axum::serve(listener, app).await.expect("fixture server");
	});
	(addr, task)
}
