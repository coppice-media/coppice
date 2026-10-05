use axum::http::{header, StatusCode};
use chrono::Utc;
use migrations::{Migrator, MigratorTrait};
use models::{
	entity::{
		book_club, book_club_book, book_club_reader_participant as reader_participant,
		media, reading_head,
	},
	shared::enums::FileStatus,
};
use sea_orm::{prelude::*, ActiveModelTrait, EntityTrait, Set};
use serde_json::{json, Value};
use stump_core::config::StumpConfig;
use tests::{fake_data, fixtures};

use super::common::TestApp;

mod discussion;

async fn migrated_database() -> sea_orm::DatabaseConnection {
	let db = sea_orm::Database::connect("sqlite::memory:")
		.await
		.expect("connect in-memory SQLite");
	Migrator::up(&db, None).await.expect("run all migrations");
	db
}

async fn manager_post(
	app: &TestApp,
	path: &str,
	token: &str,
	body: &Value,
) -> axum_test::TestResponse {
	app.server
		.post(path)
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Authorization", format!("Bearer {token}"))
		.json(body)
		.await
}

async fn manager_delete(
	app: &TestApp,
	path: &str,
	token: &str,
) -> axum_test::TestResponse {
	app.server
		.delete(path)
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Authorization", format!("Bearer {token}"))
		.await
}

async fn guest_snapshot(
	app: &TestApp,
	session_id: &str,
	token: &str,
) -> axum_test::TestResponse {
	app.server
		.get(&format!("/api/v2/club-reader/sessions/{session_id}"))
		.add_header("Host", "localhost")
		.add_header("Cookie", format!("coppice_club_reader={token}"))
		.await
}

async fn guest_progress(
	app: &TestApp,
	session_id: &str,
	book_id: &str,
	token: &str,
	participant_id: &str,
	progression: f64,
) -> axum_test::TestResponse {
	app.server
		.put(&format!(
			"/api/v2/club-reader/sessions/{session_id}/books/{book_id}/progress"
		))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token}"))
		.add_header("X-Club-Reader-Participant", participant_id)
		.json(&json!({ "progression": progression, "locator": { "href": "chapter.xhtml", "locations": { "progression": progression } } }))
		.await
}

const CLUB_ID: &str = "guest-reader-club";
const FIRST_BOOK_ID: &str = "guest-reader-queue-first";
const SECOND_BOOK_ID: &str = "guest-reader-queue-next";

/// A private club whose two-book queue (first, then next) points at one ready
/// EPUB fixture.
async fn seed_reader_club(conn: &sea_orm::DatabaseConnection) -> media::Model {
	book_club::ActiveModel {
		id: Set(CLUB_ID.to_string()),
		name: Set("Guest Reader Club".to_string()),
		slug: Set("guest-reader-club".to_string()),
		description: Set(None),
		is_private: Set(true),
		member_role_spec: Set(None),
		created_at: Set(DateTimeWithTimeZone::from(Utc::now())),
		emoji: Set(None),
	}
	.insert(conn)
	.await
	.expect("insert club");

	let library = fake_data::Library {
		id: Some("guest-reader-library".to_string()),
		name: Some("Guest Reader Library".to_string()),
		..Default::default()
	}
	.insert(conn)
	.await;
	let series = fake_data::Series {
		id: Some("guest-reader-series".to_string()),
		name: Some("Guest Reader Series".to_string()),
		library_id: Some(library.id),
		..Default::default()
	}
	.insert(conn)
	.await;
	let stored_media = fake_data::Media {
		id: Some("guest-reader-media".to_string()),
		name: Some("Reader Test Publication".to_string()),
		extension: Some("epub".to_string()),
		series_id: series.id,
		pages: Some(0),
		..Default::default()
	}
	.insert(conn)
	.await;
	let mut stored_media_active: media::ActiveModel = stored_media.clone().into();
	stored_media_active.path = Set(fixtures::get_test_epub_path());
	stored_media_active.status = Set(FileStatus::Ready);
	stored_media_active
		.update(conn)
		.await
		.expect("set EPUB fixture path");

	for (id, position, title) in [
		(FIRST_BOOK_ID, 0, "First queued publication"),
		(SECOND_BOOK_ID, 1, "Next queued publication"),
	] {
		book_club_book::ActiveModel {
			id: Set(id.to_string()),
			position: Set(position),
			completed_at: Set(None),
			title: Set(Some(title.to_string())),
			author: Set(Some("Test Author".to_string())),
			url: Set(None),
			image_url: Set(None),
			book_entity_id: Set(Some(stored_media.id.clone())),
			book_club_id: Set(CLUB_ID.to_string()),
			added_at: Set(DateTimeWithTimeZone::from(Utc::now())),
		}
		.insert(conn)
		.await
		.expect("insert queue book");
	}
	stored_media
}

/// No guest or unmatched response may expire the normal app session cookie.
fn assert_no_session_clear(response: &axum_test::TestResponse) {
	for value in response.headers().get_all(header::SET_COOKIE) {
		let value = value.to_str().expect("valid Set-Cookie header");
		assert!(
			!value.starts_with("stump_session="),
			"response must not touch the app session cookie: {value}"
		);
	}
}

#[tokio::test]
async fn guest_reader_isolated_capability_with_private_annotations_and_atomic_advance() {
	let app = TestApp::with_parts(migrated_database().await, StumpConfig::debug()).await;
	let owner_token = app.create_initial_account().await;
	*app.access_token.write().await = Some(owner_token.clone());
	let conn = app.conn();

	let club_id = CLUB_ID;
	let first_book_id = FIRST_BOOK_ID;
	let second_book_id = SECOND_BOOK_ID;
	let stored_media = seed_reader_club(conn).await;

	let reader_count_before = reading_head::Entity::find()
		.count(conn)
		.await
		.expect("count native reading heads before guest use");
	assert_eq!(reader_count_before, 0);

	let create_session = manager_post(
		&app,
		&format!("/api/v2/book-clubs/{club_id}/reader-sessions"),
		&owner_token,
		&json!({ "name": "October reading" }),
	)
	.await;
	assert_eq!(create_session.status_code(), StatusCode::CREATED);
	let create_session: Value = create_session.json();
	let session_id = create_session["id"]
		.as_str()
		.expect("session id")
		.to_string();
	assert_eq!(create_session["publishedBookId"], Value::Null);
	assert_eq!(create_session["participants"], json!([]));

	let invite_one = manager_post(
		&app,
		&format!(
			"/api/v2/book-clubs/{club_id}/reader-sessions/{session_id}/participants"
		),
		&owner_token,
		&json!({ "displayName": "Reader One" }),
	)
	.await;
	assert_eq!(invite_one.status_code(), StatusCode::CREATED);
	let invite_one: Value = invite_one.json();
	let participant_one = invite_one["participant"]["id"]
		.as_str()
		.expect("first participant id")
		.to_string();
	let token_one = invite_one["token"]
		.as_str()
		.expect("first token")
		.to_string();
	let participant_record = reader_participant::Entity::find_by_id(&participant_one)
		.one(conn)
		.await
		.expect("read participant")
		.expect("issued participant exists");
	assert_ne!(
		participant_record.token_digest.as_deref(),
		Some(token_one.as_str()),
		"database must retain only a digest of the capability"
	);
	assert_eq!(
		invite_one["readerPath"],
		format!("/app/club-reader/{session_id}")
	);

	let invite_two = manager_post(
		&app,
		&format!(
			"/api/v2/book-clubs/{club_id}/reader-sessions/{session_id}/participants"
		),
		&owner_token,
		&json!({ "displayName": "Reader Two" }),
	)
	.await;
	assert_eq!(invite_two.status_code(), StatusCode::CREATED);
	let invite_two: Value = invite_two.json();
	let participant_two = invite_two["participant"]["id"]
		.as_str()
		.expect("second participant id")
		.to_string();
	let token_two = invite_two["token"]
		.as_str()
		.expect("second token")
		.to_string();

	let publish = manager_post(
		&app,
		&format!("/api/v2/book-clubs/{club_id}/reader-sessions/{session_id}/publish"),
		&owner_token,
		&json!({ "bookId": first_book_id }),
	)
	.await;
	assert_eq!(publish.status_code(), StatusCode::OK);
	let published: Value = publish.json();
	assert_eq!(published["publishedBookId"], first_book_id);

	for token in [&token_one, &token_two] {
		let redeem = app
			.server
			.post("/api/v2/club-reader/redeem")
			.add_header("Host", "localhost")
			.add_header("Origin", "http://localhost")
			.json(&json!({ "sessionId": session_id, "token": token }))
			.await;
		assert_eq!(redeem.status_code(), StatusCode::OK, "{}", redeem.text());
		let set_cookie = redeem
			.headers()
			.get(header::SET_COOKIE)
			.expect("scoped guest cookie")
			.to_str()
			.expect("valid Set-Cookie header");
		assert!(set_cookie
			.contains(&format!("Path=/api/v2/club-reader/sessions/{session_id}")));
		assert!(set_cookie.contains("HttpOnly"));
		assert!(set_cookie.contains("SameSite=Strict"));
		assert!(
			!set_cookie.contains("; Secure"),
			"loopback development cookie must work over HTTP"
		);
	}

	let first_snapshot = guest_snapshot(&app, &session_id, &token_one).await;
	assert_eq!(first_snapshot.status_code(), StatusCode::OK);
	let first_snapshot: Value = first_snapshot.json();
	assert_eq!(first_snapshot["book"]["id"], first_book_id);
	assert_eq!(first_snapshot["viewer"]["id"], participant_one);
	assert_eq!(first_snapshot["viewer"]["shareProgress"], false);
	assert!(!first_snapshot.to_string().contains(&stored_media.id));

	let manifest = app
		.server
		.get(&format!(
			"/api/v2/club-reader/sessions/{session_id}/books/{first_book_id}/manifest.json"
		))
		.add_header("Host", "localhost")
		.add_header("Cookie", format!("coppice_club_reader={token_one}"))
		.await;
	assert_eq!(manifest.status_code(), StatusCode::OK);
	assert_eq!(
		manifest
			.headers()
			.get(header::CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap(),
		"application/webpub+json"
	);

	let stale_identity = app
		.server
		.put(&format!(
			"/api/v2/club-reader/sessions/{session_id}/books/{first_book_id}/progress"
		))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token_one}"))
		.add_header("X-Club-Reader-Participant", &participant_two)
		.json(&json!({ "progression": 0.4, "locator": { "href": "chapter.xhtml", "locations": { "progression": 0.4 } } }))
		.await;
	assert_eq!(stale_identity.status_code(), StatusCode::CONFLICT);

	let progress = app
		.server
		.put(&format!(
			"/api/v2/club-reader/sessions/{session_id}/books/{first_book_id}/progress"
		))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token_one}"))
		.add_header("X-Club-Reader-Participant", &participant_one)
		.json(&json!({ "progression": 0.4, "locator": { "href": "chapter.xhtml", "locations": { "progression": 0.4 } } }))
		.await;
	assert_eq!(progress.status_code(), StatusCode::OK);
	let progress: Value = progress.json();
	assert_eq!(progress["progress"]["progression"], 0.4);

	let private_note = app
		.server
		.post(&format!(
			"/api/v2/club-reader/sessions/{session_id}/books/{first_book_id}/annotations"
		))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token_one}"))
		.add_header("X-Club-Reader-Participant", &participant_one)
		.json(&json!({ "kind": "note", "body": "private thought" }))
		.await;
	assert_eq!(private_note.status_code(), StatusCode::CREATED);
	assert_eq!(private_note.json::<Value>()["shared"], false);

	let guest_two_snapshot = guest_snapshot(&app, &session_id, &token_two).await;
	assert_eq!(guest_two_snapshot.status_code(), StatusCode::OK);
	let guest_two_snapshot: Value = guest_two_snapshot.json();
	assert!(guest_two_snapshot["annotations"]
		.as_array()
		.unwrap()
		.is_empty());
	let guest_one_group_row = guest_two_snapshot["participants"]
		.as_array()
		.unwrap()
		.iter()
		.find(|participant| participant["id"] == participant_one)
		.expect("second participant can see the joined peer");
	assert_eq!(guest_one_group_row["percentage"], Value::Null);
	assert_eq!(guest_one_group_row["isComplete"], Value::Null);

	let shared_note = app
		.server
		.post(&format!(
			"/api/v2/club-reader/sessions/{session_id}/books/{first_book_id}/annotations"
		))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Cookie", format!("coppice_club_reader={token_one}"))
		.add_header("X-Club-Reader-Participant", &participant_one)
		.json(&json!({ "kind": "note", "body": "shared thought", "shared": true }))
		.await;
	assert_eq!(shared_note.status_code(), StatusCode::CREATED);
	let guest_two_snapshot = guest_snapshot(&app, &session_id, &token_two).await;
	let guest_two_snapshot: Value = guest_two_snapshot.json();
	let annotations = guest_two_snapshot["annotations"].as_array().unwrap();
	assert_eq!(annotations.len(), 1);
	assert_eq!(annotations[0]["body"], "shared thought");
	assert_eq!(annotations[0]["authorName"], "Reader One");
	assert_eq!(annotations[0]["editable"], false);

	let normal_identity = app
		.server
		.get("/api/v2/auth/me")
		.add_header("Host", "localhost")
		.add_header("Cookie", format!("coppice_club_reader={token_two}"))
		.await;
	assert_eq!(normal_identity.status_code(), StatusCode::UNAUTHORIZED);
	let rotate = manager_post(
		&app,
		&format!(
			"/api/v2/book-clubs/{club_id}/reader-sessions/{session_id}/participants/{participant_two}/rotate"
		),
		&owner_token,
		&json!({}),
	)
	.await;
	assert_eq!(rotate.status_code(), StatusCode::OK);
	let rotated: Value = rotate.json();
	assert_eq!(rotated["participant"]["id"], participant_two);
	let rotated_token = rotated["token"]
		.as_str()
		.expect("rotated token")
		.to_string();
	assert_ne!(rotated_token, token_two);
	assert_eq!(
		guest_snapshot(&app, &session_id, &token_two)
			.await
			.status_code(),
		StatusCode::UNAUTHORIZED,
		"rotation invalidates the old capability"
	);
	let token_two = rotated_token;

	let advance = app
		.server
		.post(&format!(
			"/api/v2/book-clubs/{club_id}/reader-sessions/{session_id}/advance"
		))
		.add_header("Host", "localhost")
		.add_header("Origin", "http://localhost")
		.add_header("Authorization", format!("Bearer {owner_token}"))
		.await;
	assert_eq!(advance.status_code(), StatusCode::OK);
	let advance: Value = advance.json();
	assert_eq!(advance["publishedBookId"], second_book_id);
	assert_eq!(
		book_club_book::Entity::find_by_id(first_book_id)
			.one(conn)
			.await
			.unwrap()
			.unwrap()
			.completed_at
			.is_some(),
		true
	);
	let old_book_resource = app
		.server
		.get(&format!(
			"/api/v2/club-reader/sessions/{session_id}/books/{first_book_id}/manifest.json"
		))
		.add_header("Host", "localhost")
		.add_header("Cookie", format!("coppice_club_reader={token_one}"))
		.await;
	assert_eq!(old_book_resource.status_code(), StatusCode::NOT_FOUND);
	let next_snapshot = guest_snapshot(&app, &session_id, &token_one).await;
	let next_snapshot: Value = next_snapshot.json();
	assert_eq!(next_snapshot["book"]["id"], second_book_id);
	assert_eq!(next_snapshot["progress"], Value::Null);
	assert!(next_snapshot["annotations"].as_array().unwrap().is_empty());

	let revoked = manager_delete(
		&app,
		&format!(
			"/api/v2/book-clubs/{club_id}/reader-sessions/{session_id}/participants/{participant_one}"
		),
		&owner_token,
	)
	.await;
	assert_eq!(revoked.status_code(), StatusCode::NO_CONTENT);
	let revoked_reader = guest_snapshot(&app, &session_id, &token_one).await;
	assert_eq!(revoked_reader.status_code(), StatusCode::UNAUTHORIZED);
	let remaining_reader = guest_snapshot(&app, &session_id, &token_two).await;
	assert_eq!(remaining_reader.status_code(), StatusCode::OK);
	let remaining_reader: Value = remaining_reader.json();
	assert!(remaining_reader["participants"]
		.as_array()
		.unwrap()
		.is_empty());

	assert_eq!(
		reading_head::Entity::find()
			.count(conn)
			.await
			.expect("count native reading heads after guest use"),
		reader_count_before,
		"guest progress must remain separate from native reading heads"
	);
}

/// Uses GraphQL for user creation and the real queue reorder mutation.
#[cfg(feature = "graphql")]
mod queue_head {
	use models::{
		entity::{
			book_club_member, book_club_reader_progress as reader_progress,
			book_club_reader_session as reader_session, user,
		},
		shared::{book_club::BookClubMemberRole, enums::UserPermission},
	};

	use super::*;
	use crate::common::account::{CreateTestUser, TestUser};

	async fn guest_get(
		app: &TestApp,
		path: &str,
		token: &str,
	) -> axum_test::TestResponse {
		app.server
			.get(path)
			.add_header("Host", "localhost")
			.add_header("Cookie", format!("coppice_club_reader={token}"))
			.await
	}

	async fn create_club_user(
		app: &TestApp,
		username: &str,
		permissions: Vec<UserPermission>,
		role: BookClubMemberRole,
	) -> TestUser {
		let created = CreateTestUser {
			username: username.to_string(),
			password: "password".to_string(),
			permissions,
			age_restriction: None,
		}
		.insert(app)
		.await;
		book_club_member::ActiveModel {
			id: Set(format!("{username}-membership")),
			display_name: Set(None),
			bio: Set(None),
			hide_progress: Set(false),
			role: Set(role),
			joined_at: Set(DateTimeWithTimeZone::from(Utc::now())),
			user_id: Set(created.id.clone()),
			book_club_id: Set(CLUB_ID.to_string()),
		}
		.insert(app.conn())
		.await
		.expect("insert club membership");
		created
	}

	/// Mirrors GraphQL `reorderBooks` (only mounted with the graphql `web`
	/// feature): rewrite uncompleted queue positions in the given order.
	async fn reorder_queue(conn: &sea_orm::DatabaseConnection, book_ids: [&str; 2]) {
		for (position, book_id) in book_ids.into_iter().enumerate() {
			book_club_book::Entity::update_many()
				.col_expr(
					book_club_book::Column::Position,
					Expr::value(position as i32),
				)
				.filter(book_club_book::Column::Id.eq(book_id))
				.exec(conn)
				.await
				.expect("reorder club queue");
		}
	}

	async fn set_media_deleted(
		conn: &sea_orm::DatabaseConnection,
		media_id: &str,
		deleted: bool,
	) {
		let deleted_at = deleted.then(|| DateTimeWithTimeZone::from(Utc::now()));
		media::Entity::update_many()
			.col_expr(media::Column::DeletedAt, Expr::value(deleted_at))
			.filter(media::Column::Id.eq(media_id))
			.exec(conn)
			.await
			.expect("toggle media soft delete");
	}

	async fn published_book_id(
		conn: &sea_orm::DatabaseConnection,
		session_id: &str,
	) -> Option<String> {
		reader_session::Entity::find_by_id(session_id)
			.one(conn)
			.await
			.expect("read reader session")
			.expect("reader session exists")
			.published_book_id
	}

	#[tokio::test]
	async fn guest_reader_follows_true_queue_head_and_never_resurrects_capabilities() {
		let app =
			TestApp::with_parts(migrated_database().await, StumpConfig::debug()).await;
		let owner_token = app.create_initial_account().await;
		*app.access_token.write().await = Some(owner_token);
		let conn = app.conn();
		let stored_media = seed_reader_club(conn).await;

		let manager = create_club_user(
			&app,
			"club-manager",
			vec![
				UserPermission::AccessBookClub,
				UserPermission::ShareBookClubReader,
			],
			BookClubMemberRole::Admin,
		)
		.await;
		let member = create_club_user(
			&app,
			"club-member",
			vec![UserPermission::AccessBookClub],
			BookClubMemberRole::Member,
		)
		.await;

		let create_session = manager_post(
			&app,
			&format!("/api/v2/book-clubs/{CLUB_ID}/reader-sessions"),
			&manager.token,
			&json!({ "name": "Queue head checks" }),
		)
		.await;
		assert_eq!(create_session.status_code(), StatusCode::CREATED);
		let session_id = create_session.json::<Value>()["id"]
			.as_str()
			.expect("session id")
			.to_string();
		let publish_path =
			format!("/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/publish");

		let non_head = manager_post(
			&app,
			&publish_path,
			&manager.token,
			&json!({ "bookId": SECOND_BOOK_ID }),
		)
		.await;
		assert_eq!(
			non_head.status_code(),
			StatusCode::CONFLICT,
			"an uncompleted book behind the queue head is not publishable"
		);
		assert_eq!(published_book_id(conn, &session_id).await, None);

		set_media_deleted(conn, &stored_media.id, true).await;
		let deleted_head = manager_post(
			&app,
			&publish_path,
			&manager.token,
			&json!({ "bookId": FIRST_BOOK_ID }),
		)
		.await;
		assert_eq!(
			deleted_head.status_code(),
			StatusCode::NOT_FOUND,
			"a soft-deleted media item is not publishable"
		);
		assert_eq!(published_book_id(conn, &session_id).await, None);
		set_media_deleted(conn, &stored_media.id, false).await;

		let publish = manager_post(
			&app,
			&publish_path,
			&manager.token,
			&json!({ "bookId": FIRST_BOOK_ID }),
		)
		.await;
		assert_eq!(publish.status_code(), StatusCode::OK, "{}", publish.text());

		let invite = manager_post(
			&app,
			&format!(
				"/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/participants"
			),
			&manager.token,
			&json!({ "displayName": "Guest" }),
		)
		.await;
		assert_eq!(invite.status_code(), StatusCode::CREATED);
		let invite: Value = invite.json();
		let guest_id = invite["participant"]["id"]
			.as_str()
			.expect("guest participant id")
			.to_string();
		let guest_token = invite["token"].as_str().expect("guest token").to_string();

		let manifest_path = format!(
		"/api/v2/club-reader/sessions/{session_id}/books/{FIRST_BOOK_ID}/manifest.json"
	);
		let positions_path = format!(
		"/api/v2/club-reader/sessions/{session_id}/books/{FIRST_BOOK_ID}/positions.json"
	);
		assert_eq!(
			guest_get(&app, &manifest_path, &guest_token)
				.await
				.status_code(),
			StatusCode::OK
		);
		let saved = guest_progress(
			&app,
			&session_id,
			FIRST_BOOK_ID,
			&guest_token,
			&guest_id,
			0.25,
		)
		.await;
		assert_eq!(saved.status_code(), StatusCode::OK, "{}", saved.text());

		// Another book moves ahead of the published one: the session pauses.
		reorder_queue(conn, [SECOND_BOOK_ID, FIRST_BOOK_ID]).await;
		let paused_snapshot = guest_snapshot(&app, &session_id, &guest_token).await;
		assert_eq!(paused_snapshot.status_code(), StatusCode::OK);
		assert_eq!(paused_snapshot.json::<Value>()["book"], Value::Null);
		for path in [&manifest_path, &positions_path] {
			assert_eq!(
				guest_get(&app, path, &guest_token).await.status_code(),
				StatusCode::CONFLICT,
				"a reordered-away book must stop serving {path}"
			);
		}
		let paused_write = guest_progress(
			&app,
			&session_id,
			FIRST_BOOK_ID,
			&guest_token,
			&guest_id,
			0.75,
		)
		.await;
		assert_eq!(paused_write.status_code(), StatusCode::CONFLICT);
		let stored_progress = reader_progress::Entity::find()
			.filter(reader_progress::Column::ParticipantId.eq(&guest_id))
			.one(conn)
			.await
			.expect("read guest progress")
			.expect("baseline guest progress");
		assert_eq!(
			stored_progress.progression, 0.25,
			"paused writes must not land"
		);

		reorder_queue(conn, [FIRST_BOOK_ID, SECOND_BOOK_ID]).await;
		assert_eq!(
			guest_get(&app, &manifest_path, &guest_token)
				.await
				.status_code(),
			StatusCode::OK,
			"restoring the queue head resumes the session"
		);

		set_media_deleted(conn, &stored_media.id, true).await;
		assert_eq!(
			guest_get(&app, &manifest_path, &guest_token)
				.await
				.status_code(),
			StatusCode::NOT_FOUND,
			"soft-deleted media must not be served to guests"
		);
		set_media_deleted(conn, &stored_media.id, false).await;

		// Guest failures are route-local JSON and never sign the app user out.
		let invalid = guest_snapshot(&app, &session_id, &"A".repeat(43)).await;
		assert_eq!(invalid.status_code(), StatusCode::UNAUTHORIZED);
		assert_no_session_clear(&invalid);
		assert!(invalid
			.headers()
			.get(header::CACHE_CONTROL)
			.and_then(|value| value.to_str().ok())
			.is_some_and(|value| value.contains("no-store")));
		let rejected_page = guest_get(
		&app,
		&format!("/api/v2/club-reader/sessions/{session_id}/books/{FIRST_BOOK_ID}/page/not-a-number"),
		&guest_token,
	)
	.await;
		assert_eq!(rejected_page.status_code(), StatusCode::BAD_REQUEST);
		assert_eq!(rejected_page.json::<Value>()["status"], 400);
		let unknown = guest_get(&app, "/api/v2/club-reader/unknown", &guest_token).await;
		assert_eq!(unknown.status_code(), StatusCode::NOT_FOUND);
		assert_eq!(unknown.json::<Value>()["status"], 404);

		// A revoked linked member cannot restore access by joining again.
		let join_path =
			format!("/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/join");
		let joined = manager_post(
			&app,
			&join_path,
			&member.token,
			&json!({ "displayName": "Member" }),
		)
		.await;
		assert_eq!(
			joined.status_code(),
			StatusCode::CREATED,
			"{}",
			joined.text()
		);
		let member_participant = joined.json::<Value>()["participant"]["id"]
			.as_str()
			.expect("member participant id")
			.to_string();
		let revoke = manager_delete(
		&app,
		&format!(
			"/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/participants/{member_participant}"
		),
		&manager.token,
	)
	.await;
		assert!(revoke.status_code().is_success(), "{}", revoke.text());
		let rejoin = manager_post(
			&app,
			&join_path,
			&member.token,
			&json!({ "displayName": "Member" }),
		)
		.await;
		assert_eq!(rejoin.status_code(), StatusCode::CONFLICT);
		assert!(rejoin.json::<Value>().get("token").is_none());
		let rotate = manager_post(
		&app,
		&format!(
			"/api/v2/book-clubs/{CLUB_ID}/reader-sessions/{session_id}/participants/{member_participant}/rotate"
		),
		&manager.token,
		&json!({}),
	)
	.await;
		assert_eq!(rotate.status_code(), StatusCode::CONFLICT);
		let revoked = reader_participant::Entity::find_by_id(&member_participant)
			.one(conn)
			.await
			.expect("read revoked participant")
			.expect("revoked participant exists");
		assert!(revoked.revoked_at.is_some());
		assert_eq!(revoked.token_digest, None);

		// Locking the issuing manager stops every capability it delegated.
		user::Entity::update_many()
			.col_expr(user::Column::IsLocked, Expr::value(true))
			.filter(user::Column::Id.eq(&manager.id))
			.exec(conn)
			.await
			.expect("lock issuing manager");
		let locked = guest_snapshot(&app, &session_id, &guest_token).await;
		assert_eq!(locked.status_code(), StatusCode::UNAUTHORIZED);
		assert_no_session_clear(&locked);
	}
}

#[tokio::test]
async fn unmatched_api_paths_are_not_captured_by_reader_management() {
	let app = TestApp::with_parts(migrated_database().await, StumpConfig::debug()).await;
	let missing = app
		.server
		.get("/api/v2/definitely-missing")
		.add_header("Host", "localhost")
		.await;
	assert_eq!(missing.status_code(), StatusCode::NOT_FOUND);
	assert_no_session_clear(&missing);
}

async fn redeem_with_origin(
	app: &TestApp,
	origin: &str,
	forwarded: bool,
) -> axum_test::TestResponse {
	let mut request = app
		.server
		.post("/api/v2/club-reader/redeem")
		.add_header("Host", "coppice:10801")
		.add_header("Origin", origin);
	if forwarded {
		request = request
			.add_header("X-Forwarded-Host", "books.example")
			.add_header("X-Forwarded-Proto", "https");
	}
	request
		.json(&json!({ "sessionId": "missing", "token": "invalid" }))
		.await
}

#[tokio::test]
async fn guest_origin_check_honours_forwarded_host_only_behind_trusted_proxy() {
	let mut trusted = StumpConfig::debug();
	trusted.server.trust_proxy_headers = true;
	let app = TestApp::with_parts(migrated_database().await, trusted).await;
	// Passing the origin guard reaches the credential check (401).
	assert_eq!(
		redeem_with_origin(&app, "https://books.example", true)
			.await
			.status_code(),
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		redeem_with_origin(&app, "http://coppice:10801", true)
			.await
			.status_code(),
		StatusCode::FORBIDDEN
	);

	let app = TestApp::with_parts(migrated_database().await, StumpConfig::debug()).await;
	assert_eq!(
		redeem_with_origin(&app, "https://books.example", true)
			.await
			.status_code(),
		StatusCode::FORBIDDEN,
		"forwarded headers are ignored unless the proxy is trusted"
	);
	assert_eq!(
		redeem_with_origin(&app, "http://coppice:10801", true)
			.await
			.status_code(),
		StatusCode::UNAUTHORIZED
	);
}
