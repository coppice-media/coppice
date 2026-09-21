//! Search used to serve the first ten of everything and link each group to a
//! route that was never mounted, so a client could neither page a search nor
//! follow one.

use crate::common::{series::setup_single_series_with_n_books, TestApp};

use serde_json::Value;
use tests::fake_data;

use super::{
	assert_every_link_resolves, group_titled, href_with_rel, navigation_titles,
	pagination_metadata, publication_ids, server_path,
};

/// Three libraries whose names all match `Image`, one of them holding a
/// five-book series, so every kind of match can be paged.
async fn setup() -> TestApp {
	let app = TestApp::new_with_default_user().await;
	let db = app.conn();

	for (id, name) in [
		("image_classics", "Image Classics"),
		("image_comics", "Image Comics"),
		("image_digital", "Image Digital"),
	] {
		fake_data::Library {
			id: Some(id.to_string()),
			name: Some(name.to_string()),
			..Default::default()
		}
		.insert(db)
		.await;
	}

	setup_single_series_with_n_books(
		&app,
		fake_data::Series {
			id: Some("black_science".to_string()),
			name: Some("Black Science".to_string()),
			library_id: Some("image_comics".to_string()),
			..Default::default()
		},
		5,
	)
	.await;

	app
}

async fn feed(app: &TestApp, path: &str) -> Value {
	let response = app.get(path).await;
	response.assert_status_ok();
	response.json()
}

/// the mixed search feed should serve the page the client asked for, in every
/// group, rather than the first ten of each kind
#[tokio::test]
async fn test_search_groups_serve_the_requested_page() {
	let app = setup().await;

	let feed = feed(&app, "/opds/v2.0/search?query=Black&page=2&page_size=2").await;
	let books = group_titled(&feed, "Books");

	assert_eq!(
		publication_ids(&books),
		vec!["black_science_3", "black_science_4"]
	);
	assert_eq!(pagination_metadata(&books), (5, 2, 2));
}

/// each group of the mixed feed should link to the route that serves that page
/// of that kind, and every link in the feed should be mounted
#[tokio::test]
async fn test_search_group_links_are_mounted_and_paged() {
	let app = setup().await;

	let feed = feed(&app, "/opds/v2.0/search?query=Black&page=2&page_size=2").await;
	let books = group_titled(&feed, "Books");

	assert_eq!(
		href_with_rel(&books, "self").map(|href| server_path(&href)),
		Some("/opds/v2.0/books/search?query=Black&page=2&page_size=2".to_string())
	);
	assert_eq!(
		href_with_rel(&books, "next").map(|href| server_path(&href)),
		Some("/opds/v2.0/books/search?query=Black&page=3&page_size=2".to_string())
	);

	assert_every_link_resolves(&app, &feed).await;
}

/// a per-kind search route should page one kind on its own, and the last page
/// should not offer a next page
#[tokio::test]
async fn test_book_search_pages_to_the_end() {
	let app = setup().await;

	let page_two = feed(
		&app,
		"/opds/v2.0/books/search?query=Black&page=2&page_size=2",
	)
	.await;
	assert_eq!(
		publication_ids(&page_two),
		vec!["black_science_3", "black_science_4"]
	);

	let next = href_with_rel(&page_two, "next").expect("page 2 of 3 should have a next");
	let page_three = feed(&app, &server_path(&next)).await;

	assert_eq!(publication_ids(&page_three), vec!["black_science_5"]);
	assert_eq!(pagination_metadata(&page_three), (5, 2, 3));
	assert_eq!(href_with_rel(&page_three, "next"), None);
	assert!(href_with_rel(&page_three, "previous").is_some());
}

/// the library and series search routes should page their own kind, ordered so
/// that a page is reproducible
#[tokio::test]
async fn test_library_and_series_search_routes_serve_their_kind() {
	let app = setup().await;

	let libraries = feed(
		&app,
		"/opds/v2.0/libraries/search?query=Image&page=2&page_size=1",
	)
	.await;
	assert_eq!(navigation_titles(&libraries), vec!["Image Comics"]);
	assert_eq!(pagination_metadata(&libraries), (3, 1, 2));
	assert_every_link_resolves(&app, &libraries).await;

	let series = feed(&app, "/opds/v2.0/series/search?query=Black").await;
	assert_eq!(navigation_titles(&series), vec!["Black Science"]);
	assert_every_link_resolves(&app, &series).await;
}

/// every search route needs a query to run: without one the request is a 400,
/// not an unfiltered feed of the whole library
#[tokio::test]
async fn test_search_requires_a_query() {
	let app = setup().await;

	for path in [
		"/opds/v2.0/search",
		"/opds/v2.0/libraries/search",
		"/opds/v2.0/series/search",
		"/opds/v2.0/books/search",
	] {
		app.get(path).await.assert_status_bad_request();
	}
}
