//! The per-session discussion and the live event stream, over the real routes.

use std::{future::poll_fn, sync::Arc, time::Duration};

use axum::{
	body::{Body, BodyDataStream},
	http::Request,
	response::Response,
	Router,
};
use axum_test::{TestResponse, TestServer};
use futures_util::StreamExt;
use stump_core::{Ctx, StumpCore};
use stump_server::{config::session::get_session_layer, routers};
use tokio::sync::RwLock;
use tower::Service;

use super::*;

/// A [`TestApp`] that also keeps its router for raw requests: `axum_test`
/// buffers whole response bodies, which never completes for an open stream.
async fn app_with_router() -> (TestApp, Router) {
	let core = StumpCore::from_ctx(Ctx::for_testing_with_config(
		migrated_database().await,
		StumpConfig::debug(),
	));
	core.init_server_config()
		.await
		.expect("failed to init server config");
	core.init_jwt_secrets()
		.await
		.expect("failed to init jwt secrets");
	let ctx = Arc::new(core.get_context());
	let router = routers::mount(ctx.clone())
		.await
		.with_state(ctx.clone())
		.layer(get_session_layer(ctx.clone()));
	let mut server =
		TestServer::new(router.clone()).expect("failed to create test server");
	server.add_header("user-agent", "stump-server-tests");
	let app = TestApp {
		server,
		ctx,
		access_token: RwLock::new(None),
	};
	(app, router)
}

async fn create_session(app: &TestApp, owner_token: &str, name: &str) -> String {
	let response = manager_post(
		app,
		&format!("/api/v2/book-clubs/{CLUB_ID}/reader-sessions"),
		owner_token,
		&json!({ "name": name }),
	)
	.await;
	assert_eq!(response.status_code(), StatusCode::CREATED);
	response.json::<Value>()["id"]
		.as_str()
		.expect("session id")
		.to_string()
}

/// Issue a guest capability: `(participant id, token)`.
async fn invite(
	app: &TestApp,
	owner_token: &str,
	session_id: &str,
	display_name: &str,
) -> (String, String) {
	let response = manager_post(
		app,
		&format!(
			"/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/participants"
		),
		owner_token,
		&json!({ "displayName": display_name }),
	)
	.await;
	assert_eq!(response.status_code(), StatusCode::CREATED);
	let issued: Value = response.json();
	(
		issued["participant"]["id"]
			.as_str()
			.expect("participant id")
			.to_string(),
		issued["token"].as_str().expect("token").to_string(),
	)
}

async fn publish(app: &TestApp, owner_token: &str, session_id: &str, book_id: &str) {
	let response = manager_post(
		app,
		&format!("/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/publish"),
		owner_token,
		&json!({ "bookId": book_id }),
	)
	.await;
	assert_eq!(
		response.status_code(),
		StatusCode::OK,
		"{}",
		response.text()
	);
}

async fn revoke(
	app: &TestApp,
	owner_token: &str,
	session_id: &str,
	participant_id: &str,
) {
	let response = manager_delete(
		app,
		&format!(
			"/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/participants/{participant_id}"
		),
		owner_token,
	)
	.await;
	assert_eq!(response.status_code(), StatusCode::NO_CONTENT);
}

fn messages_path(session_id: &str) -> String {
	format!("/api/v2/club-reader/sessions/{session_id}/messages")
}

async fn guest_messages(
	app: &TestApp,
	session_id: &str,
	token: &str,
	query: &str,
) -> TestResponse {
	app.server
		.get(&format!("{}{query}", messages_path(session_id)))
		.add_header("Host", "localhost")
		.add_header("Cookie", format!("coppice_club_reader={token}"))
		.await
}

async fn post_message(
	app: &TestApp,
	session_id: &str,
	token: &str,
	participant_id: &str,
	body: &Value,
) -> TestResponse {
	app.server
		.post(&messages_path(session_id))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token}"))
		.add_header("X-Club-Reader-Participant", participant_id)
		.json(body)
		.await
}

async fn edit_message(
	app: &TestApp,
	session_id: &str,
	message_id: &str,
	token: &str,
	participant_id: &str,
	body: &str,
) -> TestResponse {
	app.server
		.patch(&format!("{}/{message_id}", messages_path(session_id)))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token}"))
		.add_header("X-Club-Reader-Participant", participant_id)
		.json(&json!({ "body": body }))
		.await
}

async fn delete_message(
	app: &TestApp,
	session_id: &str,
	message_id: &str,
	token: &str,
	participant_id: &str,
) -> TestResponse {
	app.server
		.delete(&format!("{}/{message_id}", messages_path(session_id)))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token}"))
		.add_header("X-Club-Reader-Participant", participant_id)
		.await
}

async fn organizer_messages(
	app: &TestApp,
	owner_token: &str,
	session_id: &str,
	query: &str,
) -> TestResponse {
	app.server
		.get(&format!(
			"/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/messages{query}"
		))
		.add_header("Host", "localhost")
		.add_header("Authorization", format!("Bearer {owner_token}"))
		.await
}

fn message_id(message: &Value) -> String {
	message["id"].as_str().expect("message id").to_string()
}

fn page_ids(page: &Value) -> Vec<&str> {
	page["messages"]
		.as_array()
		.expect("message page")
		.iter()
		.map(|message| message["id"].as_str().expect("message id"))
		.collect()
}

#[tokio::test]
async fn session_discussion_is_private_owned_moderated_and_rate_limited() {
	let app = TestApp::with_parts(migrated_database().await, StumpConfig::debug()).await;
	let owner_token = app.create_initial_account().await;
	seed_reader_club(app.conn()).await;
	let session_id = create_session(&app, &owner_token, "Discussion").await;
	let other_session_id = create_session(&app, &owner_token, "Other table").await;
	let (reader_one, token_one) =
		invite(&app, &owner_token, &session_id, "Reader One").await;
	let (reader_two, token_two) =
		invite(&app, &owner_token, &session_id, "Reader Two").await;
	let (outsider, outsider_token) =
		invite(&app, &owner_token, &other_session_id, "Outsider").await;

	// An unpublished session already has a discussion.
	let hello = post_message(
		&app,
		&session_id,
		&token_one,
		&reader_one,
		&json!({ "body": "  Hello, table!  " }),
	)
	.await;
	assert_eq!(hello.status_code(), StatusCode::CREATED, "{}", hello.text());
	let hello: Value = hello.json();
	assert_eq!(hello["body"], "Hello, table!");
	assert_eq!(hello["authorId"], reader_one.as_str());
	assert_eq!(hello["authorName"], "Reader One");
	assert_eq!(hello["bookId"], Value::Null);
	assert_eq!(hello["editedAt"], Value::Null);
	assert_eq!(hello["mine"], true);
	let hello_id = message_id(&hello);

	// `bookId` may only name the live published book.
	let unpublished_book = post_message(
		&app,
		&session_id,
		&token_one,
		&reader_one,
		&json!({ "body": "Too early", "bookId": FIRST_BOOK_ID }),
	)
	.await;
	assert_eq!(unpublished_book.status_code(), StatusCode::CONFLICT);
	publish(&app, &owner_token, &session_id, FIRST_BOOK_ID).await;
	let not_current = post_message(
		&app,
		&session_id,
		&token_one,
		&reader_one,
		&json!({ "body": "Skipping ahead", "bookId": SECOND_BOOK_ID }),
	)
	.await;
	assert_eq!(not_current.status_code(), StatusCode::CONFLICT);
	let about_book = post_message(
		&app,
		&session_id,
		&token_one,
		&reader_one,
		&json!({ "body": "Chapter one!", "bookId": FIRST_BOOK_ID }),
	)
	.await;
	assert_eq!(about_book.status_code(), StatusCode::CREATED);
	let about_book: Value = about_book.json();
	assert_eq!(about_book["bookId"], FIRST_BOOK_ID);
	let about_book_id = message_id(&about_book);

	for body in [
		json!({ "body": " \n " }),
		json!({ "body": "x".repeat(4_001) }),
	] {
		let rejected =
			post_message(&app, &session_id, &token_one, &reader_one, &body).await;
		assert_eq!(rejected.status_code(), StatusCode::BAD_REQUEST);
	}
	let wrong_identity = post_message(
		&app,
		&session_id,
		&token_one,
		&reader_two,
		&json!({ "body": "Who am I?" }),
	)
	.await;
	assert_eq!(wrong_identity.status_code(), StatusCode::CONFLICT);
	let cross_site = app
		.server
		.post(&messages_path(&session_id))
		.add_header("Host", "localhost")
		.add_header("Origin", "https://elsewhere.example")
		.add_header("Cookie", format!("coppice_club_reader={token_one}"))
		.add_header("X-Club-Reader-Participant", &reader_one)
		.json(&json!({ "body": "forged" }))
		.await;
	assert_eq!(cross_site.status_code(), StatusCode::FORBIDDEN);

	// Another session's participant can neither read nor post here.
	let foreign_read = guest_messages(&app, &session_id, &outsider_token, "").await;
	assert_eq!(foreign_read.status_code(), StatusCode::UNAUTHORIZED);
	assert_no_session_clear(&foreign_read);
	let foreign_post = post_message(
		&app,
		&session_id,
		&outsider_token,
		&outsider,
		&json!({ "body": "Let me in" }),
	)
	.await;
	assert_eq!(foreign_post.status_code(), StatusCode::UNAUTHORIZED);
	let outsider_note = post_message(
		&app,
		&other_session_id,
		&outsider_token,
		&outsider,
		&json!({ "body": "Our own table" }),
	)
	.await;
	assert_eq!(outsider_note.status_code(), StatusCode::CREATED);
	let outsider_note_id = message_id(&outsider_note.json());

	// Newest first, with authorship but no edit rights for others.
	let seen_by_two = guest_messages(&app, &session_id, &token_two, "").await;
	assert_eq!(seen_by_two.status_code(), StatusCode::OK);
	let seen_by_two: Value = seen_by_two.json();
	assert_eq!(
		page_ids(&seen_by_two),
		[about_book_id.as_str(), hello_id.as_str()]
	);
	assert_eq!(seen_by_two["hasMore"], false);
	assert_eq!(seen_by_two["messages"][1]["authorId"], reader_one.as_str());
	assert_eq!(seen_by_two["messages"][1]["mine"], false);

	// Own-only edit and delete.
	let foreign_edit = edit_message(
		&app,
		&session_id,
		&hello_id,
		&token_two,
		&reader_two,
		"Hijacked",
	)
	.await;
	assert_eq!(foreign_edit.status_code(), StatusCode::NOT_FOUND);
	let foreign_delete =
		delete_message(&app, &session_id, &hello_id, &token_two, &reader_two).await;
	assert_eq!(foreign_delete.status_code(), StatusCode::NOT_FOUND);
	let edited = edit_message(
		&app,
		&session_id,
		&hello_id,
		&token_one,
		&reader_one,
		"Hello, everyone!",
	)
	.await;
	assert_eq!(edited.status_code(), StatusCode::OK);
	let edited: Value = edited.json();
	assert_eq!(edited["body"], "Hello, everyone!");
	assert!(edited["editedAt"].is_string());
	let deleted =
		delete_message(&app, &session_id, &about_book_id, &token_one, &reader_one).await;
	assert_eq!(deleted.status_code(), StatusCode::NO_CONTENT);
	let deleted_again =
		delete_message(&app, &session_id, &about_book_id, &token_one, &reader_one).await;
	assert_eq!(deleted_again.status_code(), StatusCode::NOT_FOUND);
	let after_delete: Value = guest_messages(&app, &session_id, &token_two, "")
		.await
		.json();
	assert_eq!(page_ids(&after_delete), [hello_id.as_str()]);
	assert_eq!(after_delete["messages"][0]["body"], "Hello, everyone!");

	// Paging walks back from a cursor; a tombstone remains a valid cursor.
	let mut posted = Vec::new();
	for body in ["one", "two", "three"] {
		let message = post_message(
			&app,
			&session_id,
			&token_two,
			&reader_two,
			&json!({ "body": body }),
		)
		.await;
		assert_eq!(message.status_code(), StatusCode::CREATED);
		posted.push(message_id(&message.json()));
	}
	let newest: Value = guest_messages(&app, &session_id, &token_one, "?limit=2")
		.await
		.json();
	assert_eq!(page_ids(&newest), [posted[2].as_str(), posted[1].as_str()]);
	assert_eq!(newest["hasMore"], true);
	assert_eq!(newest["messages"][0]["mine"], false);
	let older: Value = guest_messages(
		&app,
		&session_id,
		&token_one,
		&format!("?limit=2&before={}", posted[1]),
	)
	.await
	.json();
	assert_eq!(page_ids(&older), [posted[0].as_str(), hello_id.as_str()]);
	assert_eq!(older["hasMore"], false);
	let from_tombstone: Value = guest_messages(
		&app,
		&session_id,
		&token_one,
		&format!("?before={about_book_id}"),
	)
	.await
	.json();
	assert_eq!(page_ids(&from_tombstone), [hello_id.as_str()]);
	for query in ["?limit=0", "?limit=101", "?before=missing-message"] {
		let rejected = guest_messages(&app, &session_id, &token_one, query).await;
		assert_eq!(rejected.status_code(), StatusCode::BAD_REQUEST, "{query}");
	}
	let foreign_cursor = guest_messages(
		&app,
		&session_id,
		&token_one,
		&format!("?before={outsider_note_id}"),
	)
	.await;
	assert_eq!(foreign_cursor.status_code(), StatusCode::BAD_REQUEST);

	// Organizers read every message and moderate within the session only.
	let organizer_view =
		organizer_messages(&app, &owner_token, &session_id, "?limit=10").await;
	assert_eq!(organizer_view.status_code(), StatusCode::OK);
	let organizer_view: Value = organizer_view.json();
	assert_eq!(organizer_view["messages"].as_array().unwrap().len(), 4);
	assert!(organizer_view["messages"]
		.as_array()
		.unwrap()
		.iter()
		.all(|message| message["mine"] == false));
	let moderate_path =
		|session: &str, message: &str| {
			format!("/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session}/messages/{message}")
		};
	let cross_session = manager_delete(
		&app,
		&moderate_path(&session_id, &outsider_note_id),
		&owner_token,
	)
	.await;
	assert_eq!(cross_session.status_code(), StatusCode::NOT_FOUND);
	let moderated =
		manager_delete(&app, &moderate_path(&session_id, &posted[0]), &owner_token).await;
	assert_eq!(moderated.status_code(), StatusCode::NO_CONTENT);
	let after_moderation: Value = guest_messages(&app, &session_id, &token_two, "")
		.await
		.json();
	assert!(!page_ids(&after_moderation).contains(&posted[0].as_str()));
	let moderated_edit = edit_message(
		&app,
		&session_id,
		&posted[0],
		&token_two,
		&reader_two,
		"Undo",
	)
	.await;
	assert_eq!(moderated_edit.status_code(), StatusCode::NOT_FOUND);

	// Revocation ends discussion access and unlinks the author for readers,
	// but organizers still see who wrote what.
	revoke(&app, &owner_token, &session_id, &reader_one).await;
	let revoked_read = guest_messages(&app, &session_id, &token_one, "").await;
	assert_eq!(revoked_read.status_code(), StatusCode::UNAUTHORIZED);
	assert_no_session_clear(&revoked_read);
	let revoked_post = post_message(
		&app,
		&session_id,
		&token_one,
		&reader_one,
		&json!({ "body": "Still here?" }),
	)
	.await;
	assert_eq!(revoked_post.status_code(), StatusCode::UNAUTHORIZED);
	let unlinked: Value = guest_messages(&app, &session_id, &token_two, "")
		.await
		.json();
	let revoked_author = unlinked["messages"]
		.as_array()
		.unwrap()
		.iter()
		.find(|message| message["id"] == hello_id.as_str())
		.expect("a revoked author's message stays visible");
	assert_eq!(revoked_author["authorId"], Value::Null);
	assert_eq!(revoked_author["authorName"], "Reader One");
	let organizer_view: Value = organizer_messages(&app, &owner_token, &session_id, "")
		.await
		.json();
	let organizer_row = organizer_view["messages"]
		.as_array()
		.unwrap()
		.iter()
		.find(|message| message["id"] == hello_id.as_str())
		.expect("organizers see the revoked author's message");
	assert_eq!(organizer_row["authorId"], reader_one.as_str());

	// Twenty messages per participant per minute.
	let (chatty, chatty_token) = invite(&app, &owner_token, &session_id, "Chatty").await;
	for count in 1..=20 {
		let accepted = post_message(
			&app,
			&session_id,
			&chatty_token,
			&chatty,
			&json!({ "body": format!("message {count}") }),
		)
		.await;
		assert_eq!(
			accepted.status_code(),
			StatusCode::CREATED,
			"message {count}"
		);
	}
	let limited = post_message(
		&app,
		&session_id,
		&chatty_token,
		&chatty,
		&json!({ "body": "one too many" }),
	)
	.await;
	assert_eq!(limited.status_code(), StatusCode::TOO_MANY_REQUESTS);
	assert_no_session_clear(&limited);
}

/// A raw request through the shared router, so the response body streams.
async fn open_events(router: &Router, session_id: &str, token: &str) -> Response {
	let mut service = router.clone();
	poll_fn(|cx| Service::<Request<Body>>::poll_ready(&mut service, cx))
		.await
		.expect("router is always ready");
	let request =
		Request::get(format!("/api/v2/club-reader/sessions/{session_id}/events"))
			.header("Host", "localhost")
			.header("user-agent", "stump-server-tests")
			.header(header::COOKIE, format!("coppice_club_reader={token}"))
			.body(Body::empty())
			.expect("events request");
	service.call(request).await.expect("router is infallible")
}

/// Read the stream until `needle` arrives (or fail after ten seconds).
async fn read_until(stream: &mut BodyDataStream, received: &mut String, needle: &str) {
	let read = tokio::time::timeout(Duration::from_secs(10), async {
		while !received.contains(needle) {
			let chunk = stream
				.next()
				.await
				.expect("event stream ended early")
				.expect("event stream chunk");
			received.push_str(std::str::from_utf8(&chunk).expect("UTF-8 event stream"));
		}
	})
	.await;
	assert!(
		read.is_ok(),
		"no {needle:?} within 10s; received {received:?}"
	);
}

/// Read the stream until the server ends it (or fail after ten seconds).
async fn read_to_end(stream: &mut BodyDataStream, received: &mut String) {
	let read = tokio::time::timeout(Duration::from_secs(10), async {
		while let Some(chunk) = stream.next().await {
			let chunk = chunk.expect("event stream chunk");
			received.push_str(std::str::from_utf8(&chunk).expect("UTF-8 event stream"));
		}
	})
	.await;
	assert!(
		read.is_ok(),
		"stream still open after 10s; received {received:?}"
	);
}

#[tokio::test]
async fn event_stream_announces_changes_and_ends_on_revocation() {
	let (app, router) = app_with_router().await;
	let owner_token = app.create_initial_account().await;
	seed_reader_club(app.conn()).await;
	let session_id = create_session(&app, &owner_token, "Live reading").await;
	publish(&app, &owner_token, &session_id, FIRST_BOOK_ID).await;
	let (reader_one, token_one) =
		invite(&app, &owner_token, &session_id, "Reader One").await;
	let (reader_two, token_two) =
		invite(&app, &owner_token, &session_id, "Reader Two").await;

	let opened = open_events(&router, &session_id, &token_two).await;
	assert_eq!(opened.status(), StatusCode::OK);
	assert!(opened.headers()[header::CONTENT_TYPE]
		.to_str()
		.unwrap()
		.starts_with("text/event-stream"));
	assert!(opened.headers()[header::CACHE_CONTROL]
		.to_str()
		.unwrap()
		.contains("no-store"));
	let mut stream = opened.into_body().into_data_stream();
	let mut received = String::new();

	// Another participant's progress is announced by kind only.
	let progress = guest_progress(
		&app,
		&session_id,
		FIRST_BOOK_ID,
		&token_one,
		&reader_one,
		0.25,
	)
	.await;
	assert_eq!(progress.status_code(), StatusCode::OK);
	read_until(&mut stream, &mut received, r#"{"kinds":["progress"]}"#).await;
	assert!(received.contains("event: changed"), "{received}");
	assert!(received.contains("id: "), "{received}");
	assert!(
		!received.contains("0.25"),
		"events never carry reading state"
	);

	// At most three concurrent streams per participant.
	let more = [
		open_events(&router, &session_id, &token_two).await,
		open_events(&router, &session_id, &token_two).await,
	];
	assert!(more
		.iter()
		.all(|response| response.status() == StatusCode::OK));
	let capped = open_events(&router, &session_id, &token_two).await;
	assert_eq!(capped.status(), StatusCode::TOO_MANY_REQUESTS);
	drop(more);
	assert_eq!(
		open_events(&router, &session_id, &token_one).await.status(),
		StatusCode::OK,
		"each participant has its own stream budget"
	);

	// Revocation ends the open stream and refuses new ones.
	revoke(&app, &owner_token, &session_id, &reader_two).await;
	read_to_end(&mut stream, &mut received).await;
	assert!(received.contains("event: revoked"), "{received}");
	let reconnect = open_events(&router, &session_id, &token_two).await;
	assert_eq!(reconnect.status(), StatusCode::UNAUTHORIZED);
	for value in reconnect.headers().get_all(header::SET_COOKIE) {
		assert!(!value.to_str().unwrap().starts_with("stump_session="));
	}
	let messages = guest_messages(&app, &session_id, &token_two, "").await;
	assert_eq!(messages.status_code(), StatusCode::UNAUTHORIZED);
}
