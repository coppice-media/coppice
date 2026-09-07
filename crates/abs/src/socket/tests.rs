//! The socket lane, driven the way the official app drives it: a real
//! websocket against a real listener, frames in and frames out.
//!
//! `tokio-tungstenite` is the client here for one reason — it is a websocket
//! client and not a socket.io one, so every byte of the Engine.IO/Socket.IO
//! exchange is written and asserted explicitly. A socket.io client library
//! would hide exactly the framing this module implements. (The same exchange
//! is verified end to end with the real `socket.io-client@4` against a live
//! instance; see `docs/.../abs-compat.mdx`.)

use std::{sync::Arc, time::Duration};

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use models::{entity::user::AuthUser, shared::enums::LibraryType};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
	connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

use crate::{
	auth::mint_tokens,
	socket::AbsEvents,
	test_support::{
		access_token, auth_user, db, library_of_type, metadata, one_track_audio,
		router_with_events, series_with_files, TestBackend, SECRET,
	},
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Fixture {
	address: String,
	user: AuthUser,
	item_id: String,
	backend: Arc<TestBackend>,
	events: AbsEvents,
}

/// One audiobook, one user, and the profile served over a real TCP port.
async fn serve() -> Fixture {
	let conn = db().await;
	let user_row = ::tests::fake_data::User::new("ada").insert(&conn).await;
	let user = auth_user(&user_row);
	let library = library_of_type(&conn, LibraryType::Mixed).await;
	let (_, media_rows) = series_with_files(
		&conn,
		&library.id,
		"Analytical Engine",
		&[("Analytical Engine", "m4b")],
	)
	.await;
	let item = media_rows.into_iter().next().expect("one book");
	metadata(&conn, &item.id, "Analytical Engine", Some("Ada Lovelace")).await;

	let backend = TestBackend::new(conn);
	backend.set_audio(&item.id, one_track_audio(&item.path, 12_000));
	let events = AbsEvents::new();
	backend.set_events(events.clone());

	let app: Router = router_with_events(backend.clone(), &user, events.clone());
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
	let address = listener.local_addr().expect("addr").to_string();
	tokio::spawn(async move {
		let _ = axum::serve(listener, app).await;
	});

	Fixture {
		address,
		user,
		item_id: item.id,
		backend,
		events,
	}
}

/// Connect exactly where socket.io-client connects: the configured path with
/// a trailing slash, `EIO=4`, websocket only.
async fn connect(address: &str) -> Socket {
	let (socket, _) = connect_async(format!(
		"ws://{address}/socket.io/?EIO=4&transport=websocket"
	))
	.await
	.expect("websocket upgrade");
	socket
}

/// The next text frame, or a panic naming what was waited for.
async fn next_frame(socket: &mut Socket, what: &str) -> String {
	loop {
		let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
			.await
			.unwrap_or_else(|_| panic!("timed out waiting for {what}"))
			.unwrap_or_else(|| panic!("socket closed waiting for {what}"))
			.unwrap_or_else(|error| panic!("socket error waiting for {what}: {error}"));
		match message {
			Message::Text(text) => return text.to_string(),
			// Heartbeats at the websocket level are not the lane's frames.
			Message::Ping(_) | Message::Pong(_) => continue,
			other => panic!("unexpected {other:?} waiting for {what}"),
		}
	}
}

/// Wait for the Socket.IO event named `name`, skipping the others.
async fn next_event(socket: &mut Socket, name: &str) -> Value {
	loop {
		let frame = next_frame(socket, name).await;
		let Some(payload) = frame.strip_prefix("42") else {
			continue;
		};
		let value: Value = serde_json::from_str(payload).expect("event array");
		if value[0] == json!(name) {
			return value;
		}
	}
}

async fn send(socket: &mut Socket, frame: &str) {
	socket
		.send(Message::Text(frame.into()))
		.await
		.expect("send frame");
}

/// The Engine.IO open plus the Socket.IO connect, which every client does
/// before it can emit anything.
async fn handshake(socket: &mut Socket) {
	let open = next_frame(socket, "the Engine.IO open").await;
	let payload: Value =
		serde_json::from_str(open.strip_prefix('0').expect("an open packet"))
			.expect("open payload");
	assert!(payload["sid"].is_string(), "the open packet carries a sid");
	assert_eq!(
		payload["upgrades"],
		json!([]),
		"a websocket-only server offers no upgrade"
	);
	assert_eq!(payload["pingInterval"], 25_000);
	assert_eq!(payload["pingTimeout"], 20_000);

	send(socket, "40").await;
	let connect = next_frame(socket, "the Socket.IO connect").await;
	let payload: Value =
		serde_json::from_str(connect.strip_prefix("40").expect("a connect packet"))
			.expect("connect payload");
	assert!(
		payload["sid"].is_string(),
		"the connect reply names a socket"
	);
}

/// The access token `POST /login` would have minted for this user.
fn token(user: &AuthUser) -> String {
	access_token(user)
}

#[tokio::test]
async fn a_valid_token_is_answered_with_init() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;

	send(
		&mut socket,
		&format!("42{}", json!(["auth", token(&fixture.user)])),
	)
	.await;

	let init = next_event(&mut socket, "init").await;
	assert_eq!(init[1]["userId"], fixture.user.id);
	assert_eq!(init[1]["username"], fixture.user.username);
	// The app's socket plugin unhides the bookmark button on `init` alone,
	// but the presence list is what abs-ref answers with and it must be
	// true: this user holds exactly the socket that just authenticated.
	assert_eq!(init[1]["usersOnline"][0]["id"], fixture.user.id);
	assert_eq!(init[1]["usersOnline"][0]["connections"], 1);
}

#[tokio::test]
async fn a_bad_token_is_answered_with_auth_failed() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;

	send(&mut socket, &format!("42{}", json!(["auth", "not-a-jwt"]))).await;

	let failed = next_event(&mut socket, "auth_failed").await;
	assert_eq!(failed[1]["message"], "Invalid token");

	// The socket stays open: the app retries `auth` on the same connection
	// once it has refreshed its token.
	send(
		&mut socket,
		&format!("42{}", json!(["auth", token(&fixture.user)])),
	)
	.await;
	let init = next_event(&mut socket, "init").await;
	assert_eq!(init[1]["userId"], fixture.user.id);
}

#[tokio::test]
async fn a_refresh_token_cannot_open_a_live_channel() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;

	let refresh = mint_tokens(SECRET, &fixture.user.id, &fixture.user.username, None)
		.expect("mint")
		.refresh_token;
	send(&mut socket, &format!("42{}", json!(["auth", refresh]))).await;

	let failed = next_event(&mut socket, "auth_failed").await;
	assert_eq!(failed[1]["message"], "Invalid token");
}

#[tokio::test]
async fn a_progress_patch_over_rest_reaches_the_socket() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;
	send(
		&mut socket,
		&format!("42{}", json!(["auth", token(&fixture.user)])),
	)
	.await;
	next_event(&mut socket, "init").await;

	// The same REST route the app calls when a reader marks a position.
	let (status, _) = crate::test_support::request(
		fixture.backend.clone(),
		&fixture.user,
		"PATCH",
		&format!("/api/me/progress/{}", fixture.item_id),
		Some(json!({ "currentTime": 6.5, "duration": 12.0 })),
	)
	.await;
	assert_eq!(status, axum::http::StatusCode::OK);

	let event = next_event(&mut socket, "user_item_progress_updated").await;
	let payload = &event[1];
	assert_eq!(payload["data"]["libraryItemId"], fixture.item_id);
	assert_eq!(payload["data"]["currentTime"], 6.5);
	assert_eq!(payload["data"]["duration"], 12.0);
	assert_eq!(payload["data"]["isFinished"], false);
	// A Stump head is shared by every protocol, so the change cannot be
	// attributed to one ABS playback session.
	assert_eq!(payload["sessionId"], Value::Null);
	assert_eq!(payload["id"], payload["data"]["id"]);

	// abs-ref pushes the whole user object beside it, which is how the app
	// keeps its media-progress list and its bookmarks current.
	let updated = next_event(&mut socket, "user_updated").await;
	assert_eq!(updated[1]["id"], fixture.user.id);
	assert_eq!(updated[1]["mediaProgress"][0]["currentTime"], 6.5);
}

#[tokio::test]
async fn a_bookmark_write_pushes_the_user_object() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;
	send(
		&mut socket,
		&format!("42{}", json!(["auth", token(&fixture.user)])),
	)
	.await;
	next_event(&mut socket, "init").await;

	let (status, _) = crate::test_support::request(
		fixture.backend.clone(),
		&fixture.user,
		"POST",
		&format!("/api/me/item/{}/bookmark", fixture.item_id),
		Some(json!({ "time": 4, "title": "The engine" })),
	)
	.await;
	assert_eq!(status, axum::http::StatusCode::OK);

	// Nothing else announces a bookmark: it is the one `/api/me` change no
	// core event carries, and the app's bookmark UI is socket-gated.
	let updated = next_event(&mut socket, "user_updated").await;
	assert_eq!(updated[1]["bookmarks"][0]["title"], "The engine");
	assert_eq!(updated[1]["bookmarks"][0]["time"], 4.0);
}

#[tokio::test]
async fn another_users_position_is_never_pushed() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;
	send(
		&mut socket,
		&format!("42{}", json!(["auth", token(&fixture.user)])),
	)
	.await;
	next_event(&mut socket, "init").await;

	// A head exists for the book; only this user's change may be pushed.
	fixture.backend.set_progress(
		&fixture.user.id,
		&fixture.item_id,
		crate::model::AbsProgress {
			position_ms: 3_000,
			track_index: Some(0),
			is_finished: false,
			started_at: chrono::Utc::now(),
			last_update: chrono::Utc::now(),
			finished_at: None,
		},
	);
	fixture
		.events
		.send(crate::socket::AbsEvent::ProgressChanged {
			user_id: "somebody-else".to_owned(),
			media_id: fixture.item_id.clone(),
		});
	// …and then one for this user, which must be the first thing that
	// arrives: the foreign event is dropped, not queued behind it.
	fixture
		.events
		.send(crate::socket::AbsEvent::ProgressChanged {
			user_id: fixture.user.id.clone(),
			media_id: fixture.item_id.clone(),
		});

	let event = next_event(&mut socket, "user_item_progress_updated").await;
	assert_eq!(event[1]["data"]["userId"], fixture.user.id);
}

#[tokio::test]
async fn nothing_is_pushed_before_auth() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;

	fixture
		.events
		.send(crate::socket::AbsEvent::ProgressChanged {
			user_id: fixture.user.id.clone(),
			media_id: fixture.item_id.clone(),
		});

	// An unauthenticated socket has no user to scope visibility by, so it
	// gets nothing at all until it authenticates.
	let idle = tokio::time::timeout(Duration::from_millis(250), socket.next()).await;
	assert!(idle.is_err(), "an unauthenticated socket received a push");
}

#[tokio::test]
async fn an_unknown_namespace_is_refused_without_dropping_the_socket() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	next_frame(&mut socket, "the Engine.IO open").await;

	send(&mut socket, "40/admin,").await;
	let error = next_event(&mut socket, "connect_error").await;
	assert_eq!(error[1]["message"], "Invalid namespace");

	// The default namespace still connects on the same socket.
	send(&mut socket, "40").await;
	let connect = next_frame(&mut socket, "the Socket.IO connect").await;
	assert!(connect.starts_with("40{"));
}

#[tokio::test]
async fn a_client_ping_is_answered_with_a_pong() {
	let fixture = serve().await;
	let mut socket = connect(&fixture.address).await;
	handshake(&mut socket).await;

	send(&mut socket, "2").await;
	assert_eq!(next_frame(&mut socket, "an Engine.IO pong").await, "3");
}
