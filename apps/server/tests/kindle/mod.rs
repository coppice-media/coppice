//! The USB sideload route: `POST /api/v2/media/{id}/kindle-file`.
//!
//! This is the lane with no gateway behind it — the operator saves the
//! response into a mounted Kindle's `documents/` folder — so the assertions
//! are on what a browser and a Kindle actually consume: the status, the
//! `Content-Disposition` filename (which decides the file's extension on
//! disk), the MIME type, and the body bytes. The conversion policy itself is
//! tested in `crates/kindle`.

use axum::http::{header, StatusCode};
use models::entity::media;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, IntoActiveModel};
use tempfile::TempDir;
use tests::fake_data;

use crate::common::{series::setup_single_series_with_n_books, TestApp};

struct Fixture {
	app: TestApp,
	book_id: String,
	/// Held so the book file outlives the test.
	dir: TempDir,
}

/// One book whose file really exists on disk, because the route reads it.
async fn book_on_disk(filename: &str, content: &[u8]) -> Fixture {
	let app = TestApp::new_with_default_user().await;
	let (_series, books) =
		setup_single_series_with_n_books(&app, fake_data::Series::default(), 1).await;

	let dir = TempDir::new().expect("failed to create temp dir");
	let path = dir.path().join(filename);
	std::fs::write(&path, content).expect("failed to write the book");

	let mut active = books[0].clone().into_active_model();
	active.path = Set(path.to_string_lossy().to_string());
	let book: media::Model = active
		.update(app.conn())
		.await
		.expect("failed to point the book at its file");

	Fixture {
		app,
		book_id: book.id,
		dir,
	}
}

/// A book that already is a Kindle format is handed over untouched, named so
/// that saving it onto the device produces a file the Kindle opens.
#[tokio::test]
async fn kindle_file_serves_a_sideloadable_book() {
	let content = b"BOOKMOBI already a kindle file";
	let fixture = book_on_disk("sideload.azw3", content).await;

	let response = fixture
		.app
		.post(&format!("/api/v2/media/{}/kindle-file", fixture.book_id))
		.await;

	response.assert_status_ok();
	assert_eq!(
		response
			.headers()
			.get(header::CONTENT_DISPOSITION)
			.expect("a disposition"),
		"attachment; filename=\"sideload.azw3\""
	);
	assert_eq!(
		response
			.headers()
			.get(header::CONTENT_TYPE)
			.expect("a content type"),
		"application/vnd.amazon.mobi8-ebook"
	);
	// Nothing was converted, and the operator's client can tell.
	assert_eq!(
		response
			.headers()
			.get("x-stump-kindle-converted")
			.expect("the converted header"),
		"false"
	);
	assert_eq!(response.as_bytes().as_ref(), content);
	drop(fixture.dir);
}

/// The invariant of the USB lane: an EPUB is never served as an EPUB. A
/// Kindle mounted as a disk cannot open one, so either the server converted
/// it (operators with `boko`) or the request is refused with the reason.
#[tokio::test]
async fn kindle_file_never_serves_an_epub() {
	let epub = std::fs::read(
		std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../../core/integration-tests/data/book.epub"),
	)
	.expect("the fixture epub");
	let fixture = book_on_disk("book.epub", &epub).await;

	let response = fixture
		.app
		.post(&format!("/api/v2/media/{}/kindle-file", fixture.book_id))
		.await;

	if response.status_code() == StatusCode::OK {
		assert_eq!(
			response
				.headers()
				.get(header::CONTENT_DISPOSITION)
				.expect("a disposition"),
			"attachment; filename=\"book.azw3\"",
			"a converted download must be named as the Kindle format"
		);
		assert_eq!(
			response
				.headers()
				.get("x-stump-kindle-converted")
				.expect("the converted header"),
			"true"
		);
	} else {
		assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
		let body = response.text();
		assert!(
			body.contains("a Kindle opens no .epub over USB"),
			"the refusal must name the reason: {body}"
		);
	}
	drop(fixture.dir);
}

/// A comic is refused: a Kindle cannot open a CBZ, and a download that
/// produced an unreadable file on the device would look like success.
#[tokio::test]
async fn kindle_file_refuses_a_format_a_kindle_cannot_read() {
	let fixture = book_on_disk("book.cbz", b"PK\x03\x04 comic archive").await;

	let response = fixture
		.app
		.post(&format!("/api/v2/media/{}/kindle-file", fixture.book_id))
		.await;

	assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
	let body = response.text();
	assert!(
		body.contains("a Kindle cannot read .cbz"),
		"the refusal must name the format: {body}"
	);
	drop(fixture.dir);
}

/// A book the caller cannot see is a 404, not a conversion of someone else's
/// library file.
#[tokio::test]
async fn kindle_file_is_not_found_for_an_unknown_book() {
	let app = TestApp::new_with_default_user().await;

	let response = app.post("/api/v2/media/not-a-book/kindle-file").await;

	assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}
