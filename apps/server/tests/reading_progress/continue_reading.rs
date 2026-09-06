//! `GET /api/v2/reading/continue` — the continue-reading dashboard a device
//! draws in one request.
//!
//! The route reads `reading_heads` (the unified reading state), so the fixture
//! here writes heads through `reading_state::apply` rather than through one
//! protocol's route: the point is that a position projects the same way
//! whichever protocol delivered it.

use chrono::{Duration, Utc};
use models::{
	domain::reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	entity::{library_exclusion, media, user},
	services::reading_state,
};
use sea_orm::{prelude::*, ActiveValue::Set, EntityTrait};
use serde_json::{json, Value};
use tests::fake_data;

use crate::common::TestApp;

struct Fixture {
	app: TestApp,
	user_id: String,
	/// The library the visibility test hides.
	novels: String,
	/// `black_science_1..3`, then `dune_1`.
	books: Vec<media::Model>,
}

/// Two libraries so a visibility test has something to hide, three books in
/// the first so ordering and limits have something to order.
async fn fixture() -> Fixture {
	let app = TestApp::new_with_default_user().await;
	let conn = app.conn();

	let user_id = user::Entity::find()
		.one(conn)
		.await
		.expect("db error")
		.expect("default user exists")
		.id;

	let comics = fake_data::Library {
		id: Some("comics".to_string()),
		name: Some("Comics".to_string()),
		..Default::default()
	}
	.insert(conn)
	.await;
	let novels = fake_data::Library {
		id: Some("novels".to_string()),
		name: Some("Novels".to_string()),
		..Default::default()
	}
	.insert(conn)
	.await;

	let mut books = Vec::new();
	for (library, series_id, series_name, count) in [
		(&comics, "black_science", "Black Science", 3),
		(&novels, "dune", "Dune", 1),
	] {
		let series = fake_data::Series {
			id: Some(series_id.to_string()),
			name: Some(series_name.to_string()),
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(conn)
		.await;
		for index in 1..=count {
			books.push(
				fake_data::Media {
					id: Some(format!("{series_id}_{index}")),
					name: Some(format!("{series_name} #{index}")),
					series_id: series.id.clone(),
					extension: Some("cbz".to_string()),
					pages: Some(100),
					..Default::default()
				}
				.insert(conn)
				.await,
			);
		}
	}

	Fixture {
		app,
		user_id,
		novels: novels.id,
		books,
	}
}

/// Writes a head `minutes_ago` old, as `protocol` would have.
async fn write_head(
	fixture: &Fixture,
	user_id: &str,
	book: &media::Model,
	page: i32,
	minutes_ago: i64,
	completed: bool,
) {
	reading_state::apply(
		fixture.app.conn(),
		user_id,
		Publication::from(book),
		ProtocolUpdate {
			protocol: SourceProtocol::Koreader,
			device_id: None,
			updated_at: Some(Utc::now() - Duration::minutes(minutes_ago)),
			position: Position::Page(page),
			progression: None,
			completed: Some(completed),
			raw_payload: json!({ "page": page }),
		},
	)
	.await
	.expect("head should apply");
}

async fn set_koreader_hash(fixture: &Fixture, book_id: &str, hash: &str) {
	media::Entity::update_many()
		.filter(media::Column::Id.eq(book_id))
		.col_expr(media::Column::KoreaderHash, Expr::value(hash))
		.exec(fixture.app.conn())
		.await
		.expect("hash should update");
}

async fn exclude_library(fixture: &Fixture, library_id: &str) {
	library_exclusion::ActiveModel {
		user_id: Set(fixture.user_id.clone()),
		library_id: Set(library_id.to_string()),
		..Default::default()
	}
	.insert(fixture.app.conn())
	.await
	.expect("exclusion should insert");
}

async fn fetch(app: &TestApp, query: &str) -> Value {
	let response = app.get(&format!("/api/v2/reading/continue{query}")).await;
	assert_eq!(
		response.status_code(),
		200,
		"expected 200, got {}: {}",
		response.status_code(),
		response.text()
	);
	response.json::<Value>()
}

fn media_ids(body: &Value) -> Vec<String> {
	body["items"]
		.as_array()
		.expect("items array")
		.iter()
		.map(|item| {
			item["mediaId"]
				.as_str()
				.expect("mediaId is a string")
				.to_string()
		})
		.collect()
}

/// A user with no heads gets an empty list, not a 404 and not the whole
/// library: "nothing in progress" is a valid answer.
#[tokio::test]
async fn test_continue_reading_is_empty_without_heads() {
	let fixture = fixture().await;

	let body = fetch(&fixture.app, "").await;

	assert_eq!(media_ids(&body), Vec::<String>::new());
}

/// The order is the head order — most recently read first — because that is
/// the only order a "continue reading" row can be in.
#[tokio::test]
async fn test_continue_reading_orders_most_recent_first() {
	let fixture = fixture().await;
	write_head(&fixture, &fixture.user_id, &fixture.books[0], 10, 30, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[1], 20, 5, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[2], 30, 90, false).await;

	let body = fetch(&fixture.app, "").await;

	assert_eq!(
		media_ids(&body),
		vec![
			"black_science_2".to_string(),
			"black_science_1".to_string(),
			"black_science_3".to_string(),
		]
	);
}

/// The row carries the position and the identity a device needs, so a
/// dashboard is one request rather than one request per book (which is what
/// the OPDS `progression` link would cost).
#[tokio::test]
async fn test_continue_reading_row_carries_position_and_links() {
	let fixture = fixture().await;
	set_koreader_hash(&fixture, "black_science_1", "3f2a1b4c5d6e7f80").await;
	write_head(&fixture, &fixture.user_id, &fixture.books[0], 25, 1, false).await;

	let body = fetch(&fixture.app, "").await;
	let item = &body["items"][0];

	assert_eq!(item["mediaId"], "black_science_1");
	assert_eq!(item["name"], "Black Science #1");
	assert_eq!(item["seriesId"], "black_science");
	assert_eq!(item["seriesName"], "Black Science");
	assert_eq!(item["extension"], "cbz");
	assert_eq!(item["page"], 25);
	assert_eq!(item["pages"], 100);
	assert_eq!(item["progression"], 0.25);
	assert_eq!(item["sourceProtocol"], "koreader");
	assert_eq!(item["positionMs"], Value::Null);
	assert_eq!(
		item["downloadUrl"], "/api/v2/media/black_science_1/file",
		"the download rides the same credential as this route"
	);
	assert_eq!(
		item["thumbnailUrl"],
		"/api/v2/media/black_science_1/thumbnail"
	);
	assert!(
		item["updatedAt"]
			.as_str()
			.is_some_and(|value| value.len() > 10),
		"updatedAt should be an RFC 3339 timestamp, got {:?}",
		item["updatedAt"]
	);
}

/// `koreaderHash` is `media.koreader_hash`: the partial MD5 a KOReader-class
/// client already holds for its local file. Without it a device can only
/// guess by title, so it is part of the contract rather than an extra.
#[tokio::test]
async fn test_continue_reading_exposes_koreader_hash() {
	let fixture = fixture().await;
	set_koreader_hash(&fixture, "black_science_1", "abcdef0123456789").await;
	write_head(&fixture, &fixture.user_id, &fixture.books[0], 10, 2, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[1], 10, 1, false).await;

	let body = fetch(&fixture.app, "").await;

	assert_eq!(
		body["items"][1]["koreaderHash"], "abcdef0123456789",
		"a hashed book hands its hash back"
	);
	assert_eq!(
		body["items"][0]["koreaderHash"],
		Value::Null,
		"an unhashed book reports no hash rather than a placeholder"
	);
}

/// A finished book is not something to continue.
#[tokio::test]
async fn test_continue_reading_excludes_completed_heads() {
	let fixture = fixture().await;
	write_head(&fixture, &fixture.user_id, &fixture.books[0], 100, 1, true).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[1], 10, 2, false).await;

	let body = fetch(&fixture.app, "").await;

	assert_eq!(media_ids(&body), vec!["black_science_2".to_string()]);
}

/// One user's position is never another user's business.
#[tokio::test]
async fn test_continue_reading_is_scoped_to_the_caller() {
	let fixture = fixture().await;
	let other = fake_data::User::new("someone-else")
		.insert(fixture.app.conn())
		.await;
	write_head(&fixture, &other.id, &fixture.books[0], 10, 1, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[1], 20, 2, false).await;

	let body = fetch(&fixture.app, "").await;

	assert_eq!(media_ids(&body), vec!["black_science_2".to_string()]);
}

/// A book the caller cannot see is absent, and — because the limit is applied
/// to the already-filtered set — an invisible book does not consume a row.
#[tokio::test]
async fn test_continue_reading_hides_excluded_libraries_without_eating_rows() {
	let fixture = fixture().await;
	// Newest head is in the library the user has hidden.
	write_head(&fixture, &fixture.user_id, &fixture.books[3], 10, 1, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[0], 20, 2, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[1], 30, 3, false).await;
	exclude_library(&fixture, &fixture.novels).await;

	let body = fetch(&fixture.app, "?limit=2").await;

	assert_eq!(
		media_ids(&body),
		vec!["black_science_1".to_string(), "black_science_2".to_string(),],
		"the hidden book is gone and the two visible ones still fill the page"
	);
}

/// `limit` bounds the page, and a nonsense limit is clamped rather than
/// producing an empty page or an unbounded scan.
#[tokio::test]
async fn test_continue_reading_limit_is_clamped() {
	let fixture = fixture().await;
	write_head(&fixture, &fixture.user_id, &fixture.books[0], 10, 3, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[1], 20, 2, false).await;
	write_head(&fixture, &fixture.user_id, &fixture.books[2], 30, 1, false).await;

	assert_eq!(
		media_ids(&fetch(&fixture.app, "?limit=1").await),
		vec!["black_science_3".to_string()]
	);
	assert_eq!(
		media_ids(&fetch(&fixture.app, "?limit=0").await),
		vec!["black_science_3".to_string()],
		"limit=0 clamps to one row instead of returning nothing"
	);
	assert_eq!(
		media_ids(&fetch(&fixture.app, "?limit=9999").await).len(),
		3,
		"an over-large limit is capped but still returns everything available"
	);
}

/// The route is behind the same authentication as every other `/api/v2`
/// route: an anonymous request is rejected, not answered with an empty list.
#[tokio::test]
async fn test_continue_reading_requires_authentication() {
	let fixture = fixture().await;

	let response = fixture.app.server.get("/api/v2/reading/continue").await;

	assert_eq!(response.status_code(), 401);
}
