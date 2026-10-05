use crate::common::TestApp;

use axum::http::StatusCode;
use models::{
	entity::{device, reading_head},
	shared::enums::DeviceKind,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::{json, Value};
use tests::fake_data;

use super::{href_with_rel, server_path};

async fn setup(extension: &str, pages: i32) -> (TestApp, String) {
	let app = TestApp::new_with_default_user().await;
	let db = app.conn();
	let image = fake_data::Library {
		id: Some("image".to_string()),
		name: Some("Image".to_string()),
		..Default::default()
	}
	.insert(db)
	.await;
	let series = fake_data::Series {
		id: Some("black_science".to_string()),
		name: Some("Black Science".to_string()),
		library_id: Some(image.id),
		..Default::default()
	}
	.insert(db)
	.await;
	let book = fake_data::Media {
		id: Some("black_science_1".to_string()),
		name: Some("Black Science #1".to_string()),
		series_id: series.id,
		extension: Some(extension.to_string()),
		pages: Some(pages),
		..Default::default()
	}
	.insert(db)
	.await;
	(app, book.id)
}

fn payload(modified: &str, progression: f64, references: Vec<&str>) -> Value {
	json!({
		"modified": modified,
		"device": { "id": "urn:uuid:opds-device-1", "name": "OPDS Device" },
		"progression": progression,
		"references": references,
	})
}

async fn get_progression(app: &TestApp, book_id: &str) -> Value {
	let response = app
		.get(&format!("/opds/v2.0/books/{book_id}/progression"))
		.await;
	response.assert_status_ok();
	assert_eq!(
		response.headers()["content-type"],
		"application/opds-progression+json"
	);
	response.json()
}

#[tokio::test]
async fn test_progression_is_empty_before_any_reading_head() {
	let (app, book_id) = setup("cbz", 100).await;
	let response = app
		.get(&format!("/opds/v2.0/books/{book_id}/progression"))
		.await;
	response.assert_status_ok();
	assert_eq!(
		response.headers()["content-type"],
		"application/opds-progression+json"
	);
	assert!(
		response.as_bytes().is_empty(),
		"no invented timestamp or locator"
	);
	app.get("/opds/v2.0/books/missing/progression")
		.await
		.assert_status_not_found();
}

#[tokio::test]
async fn test_stable_progression_discovery_and_put_get_receipts() {
	let (app, book_id) = setup("cbz", 100).await;
	let publication: Value = app.get(&format!("/opds/v2.0/books/{book_id}")).await.json();
	let href = href_with_rel(&publication, "http://opds-spec.org/progression").unwrap();
	assert_eq!(
		server_path(&href),
		format!("/opds/v2.0/books/{book_id}/progression")
	);
	let link = publication["links"]
		.as_array()
		.unwrap()
		.iter()
		.find(|link| link["rel"] == "http://opds-spec.org/progression")
		.unwrap();
	assert_eq!(link["type"], "application/opds-progression+json");
	assert!(link["properties"]["authenticate"]["href"]
		.as_str()
		.unwrap()
		.ends_with("/opds/v2.0/auth"));
	assert!(
		href_with_rel(&publication, "http://www.cantook.com/api/progression").is_none()
	);

	for (modified, progression, page, status) in [
		("2026-01-28T11:00:00Z", 0.1, "#page=10", StatusCode::CREATED),
		("2026-01-28T11:01:00Z", 0.23, "#page=20", StatusCode::OK),
	] {
		let input = payload(modified, progression, vec![page]);
		let response = app.put(&server_path(&href), &input).await;
		response.assert_status(status);
		assert_eq!(
			response.headers()["content-type"],
			"application/opds-progression+json"
		);
		let receipt: Value = response.json();
		assert_eq!(receipt["device"], input["device"]);
		assert_eq!(
			receipt["progression"], progression,
			"use the asserted fraction, not page/count"
		);
		assert_eq!(receipt["references"], json!([page]));
		assert!(receipt.get("locator").is_none());
		assert_eq!(get_progression(&app, &book_id).await, receipt);
	}
	let native: Value = app.get("/api/v2/reading/continue").await.json();
	let book = native["items"]
		.as_array()
		.unwrap()
		.iter()
		.find(|book| book["mediaId"] == book_id)
		.unwrap();
	assert_eq!(book["progression"], 0.23);
	assert_eq!(book["page"], 20);
}

#[tokio::test]
async fn test_stale_progression_returns_problem_and_preserves_head() {
	let (app, book_id) = setup("cbz", 100).await;
	let path = format!("/opds/v2.0/books/{book_id}/progression");
	app.put(
		&path,
		&payload("2026-01-28T11:00:00Z", 0.5, vec!["#page=50"]),
	)
	.await
	.assert_status(StatusCode::CREATED);
	let before = get_progression(&app, &book_id).await;
	let response = app
		.put(
			&path,
			&payload("2026-01-28T10:58:00Z", 0.1, vec!["#page=10"]),
		)
		.await;
	response.assert_status(StatusCode::CONFLICT);
	assert_eq!(
		response.headers()["content-type"],
		"application/problem+json"
	);
	let problem: Value = response.json();
	assert_eq!(
		problem["type"],
		"https://registry.opds.io/error#progression-date"
	);
	assert!(problem["title"].is_string());
	assert_eq!(get_progression(&app, &book_id).await, before);
}

#[tokio::test]
async fn test_invalid_stable_payload_and_page_bounds_return_problem_without_writes() {
	let (app, book_id) = setup("cbz", 100).await;
	let path = format!("/opds/v2.0/books/{book_id}/progression");
	let mut bad_timestamp = payload("2026-01-28T11:00:00Z", 0.2, vec![]);
	bad_timestamp["modified"] = json!("not-a-date");
	for input in [
		payload("2026-01-28T11:00:00Z", -0.1, vec![]),
		payload("2026-01-28T11:00:00Z", 1.1, vec![]),
		payload("2026-01-28T11:00:00Z", 0.2, vec!["#page=0"]),
		payload("2026-01-28T11:00:00Z", 0.2, vec!["#page=101"]),
		bad_timestamp,
		json!({
			"modified": "2026-01-28T11:00:00Z",
			"device": { "id": "device", "name": "Reader" },
			"locator": { "href": "chapter.xhtml", "type": "application/xhtml+xml" }
		}),
	] {
		let response = app.put(&path, &input).await;
		response.assert_status_bad_request();
		assert_eq!(
			response.headers()["content-type"],
			"application/problem+json"
		);
		let problem: Value = response.json();
		assert_eq!(
			problem["type"],
			"https://registry.opds.io/error#progression-invalid-payload"
		);
	}
	assert!(reading_head::Entity::find()
		.filter(reading_head::Column::MediaId.eq(&book_id))
		.one(app.conn())
		.await
		.unwrap()
		.is_none());
	assert!(device::Entity::find_by_id("urn:uuid:opds-device-1")
		.one(app.conn())
		.await
		.unwrap()
		.is_none());
}

#[tokio::test]
async fn test_progression_does_not_claim_another_users_device() {
	let (app, book_id) = setup("cbz", 100).await;
	let other = fake_data::User::new("other-reader")
		.insert(app.conn())
		.await;
	device::ActiveModel {
		id: Set("urn:uuid:opds-device-1".into()),
		user_id: Set(other.id),
		name: Set("Other reader's device".into()),
		kind: Set(DeviceKind::Opds),
		created_at: Set(chrono::Utc::now().into()),
		..Default::default()
	}
	.insert(app.conn())
	.await
	.unwrap();
	let response = app
		.put(
			&format!("/opds/v2.0/books/{book_id}/progression"),
			&payload("2026-01-28T11:00:00Z", 0.2, vec!["#page=20"]),
		)
		.await;
	response.assert_status_forbidden();
	let problem: Value = response.json();
	assert_eq!(
		problem["type"],
		"https://registry.opds.io/error#progression-incorrect-user"
	);
	assert!(reading_head::Entity::find()
		.filter(reading_head::Column::MediaId.eq(book_id))
		.one(app.conn())
		.await
		.unwrap()
		.is_none());
}

#[cfg(feature = "graphql")]
#[tokio::test]
async fn test_stable_fraction_only_update_keeps_the_rich_home_native_locator() {
	use crate::common::book::update_progress;
	use graphql::input::media::{EpubProgressInput, MediaProgressInput};
	use models::shared::readium::ReadiumLocator;
	use rust_decimal::Decimal;

	let (app, book_id) = setup("epub", -1).await;
	let locator: ReadiumLocator = serde_json::from_value(json!({
		"href": "OPS/chapter.xhtml", "type": "application/xhtml+xml",
		"chapterTitle": "Chapter 1", "title": "Chapter 1",
		"locations": { "totalProgression": 0.25, "progression": 0.4,
			"fragments": ["paragraph"], "position": 8, "cssSelector": "#paragraph",
			"partialCfi": "/4/2" },
		"text": { "before": "Before", "highlight": "Exact anchor", "after": "After" }
	}))
	.unwrap();
	update_progress(
		&app,
		&book_id,
		MediaProgressInput::Epub(Box::new(EpubProgressInput {
			locator: locator.clone(),
			percentage: Some(Decimal::new(25, 2)),
			is_complete: None,
			elapsed_seconds_delta: None,
			device_id: None,
			reset_elapsed_seconds: None,
		})),
	)
	.await;
	let projected = get_progression(&app, &book_id).await;
	assert_eq!(
		projected["references"],
		json!(["OPS/chapter.xhtml#paragraph"])
	);
	assert_eq!(projected["progression"], 0.25);
	assert!(projected.get("locator").is_none());

	let head = reading_head::Entity::find()
		.filter(reading_head::Column::MediaId.eq(&book_id))
		.one(app.conn())
		.await
		.unwrap()
		.unwrap();
	let mut input = payload(
		&(head.updated_at + chrono::Duration::seconds(1)).to_rfc3339(),
		0.5,
		vec![],
	);
	input.as_object_mut().unwrap().remove("references");
	app.put(&format!("/opds/v2.0/books/{book_id}/progression"), &input)
		.await
		.assert_status_ok();
	let head = reading_head::Entity::find()
		.filter(reading_head::Column::MediaId.eq(&book_id))
		.one(app.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(head.locator, Some(locator));
	assert_eq!(head.progression, 0.5);
	let native = app
		.execute_gql(
			r#"query NativeProgress($id: ID!) {
		mediaById(id: $id) { readProgress { percentageCompleted locator {
			href locations { cssSelector partialCfi progression } text { highlight }
		} } }
	}"#,
			Some(json!({ "id": book_id })),
		)
		.await;
	assert!(native.get("errors").is_none(), "{native}");
	let progress = &native["data"]["mediaById"]["readProgress"];
	assert_eq!(progress["locator"]["text"]["highlight"], "Exact anchor");
	assert_eq!(
		progress["locator"]["locations"]["cssSelector"],
		"#paragraph"
	);
	assert_eq!(get_progression(&app, &book_id).await["progression"], 0.5);
}
