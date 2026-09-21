//! Browse feeds used to link to routes that were never mounted and to offer a
//! next page that did not exist, and `/books/browse` rejected the very page
//! parameter its own next link carried.

use crate::common::{series::setup_single_series_with_n_books, TestApp};

use serde_json::Value;
use tests::fake_data;

use super::{
	assert_every_link_resolves, group_titled, href_with_rel, navigation_titles,
	pagination_metadata, publication_ids, server_path,
};

/// Three libraries, one holding a five-book series, so a list and a book feed
/// both have more than one page.
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

/// `/books/browse` should accept the pagination every other feed accepts: its
/// filters are flattened alongside it, which used to make `?page=1` a 400
#[tokio::test]
async fn test_books_browse_accepts_pagination() {
	let app = setup().await;

	app.get("/opds/v2.0/books/browse?page=1")
		.await
		.assert_status_ok();

	let page_two = feed(&app, "/opds/v2.0/books/browse?page=2&page_size=2").await;
	assert_eq!(
		publication_ids(&page_two),
		vec!["black_science_3", "black_science_4"]
	);
	assert_eq!(pagination_metadata(&page_two), (5, 2, 2));
}

/// a filter and a page should coexist: the filter narrows the feed and stays on
/// the links the feed generates for itself
#[tokio::test]
async fn test_books_browse_filters_alongside_pagination() {
	let app = setup().await;

	let filtered = feed(
		&app,
		"/opds/v2.0/books/browse?author=Nobody&page=1&page_size=2",
	)
	.await;

	assert!(publication_ids(&filtered).is_empty());
	assert_eq!(
		href_with_rel(&filtered, "self").map(|href| server_path(&href)),
		Some("/opds/v2.0/books/browse?author=Nobody&page=1&page_size=2".to_string())
	);
}

/// the next link a book feed emits should be followable, and should keep the
/// page size the client asked for rather than silently resetting it
#[tokio::test]
async fn test_book_feed_next_link_is_followable() {
	let app = setup().await;

	let page_one = feed(&app, "/opds/v2.0/books/browse?page=1&page_size=2").await;
	let next = href_with_rel(&page_one, "next").expect("page 1 of 3 should have a next");

	let page_two = feed(&app, &server_path(&next)).await;

	assert_eq!(
		publication_ids(&page_two),
		vec!["black_science_3", "black_science_4"]
	);
	assert_eq!(pagination_metadata(&page_two), (5, 2, 2));
}

/// a final page should not advertise a next page, and a first page should not
/// advertise a previous one: a client following either lands on nothing
#[tokio::test]
async fn test_pagination_links_only_name_pages_that_exist() {
	let app = setup().await;

	let whole_feed = feed(&app, "/opds/v2.0/books/latest?page=1&page_size=5").await;
	assert_eq!(href_with_rel(&whole_feed, "next"), None);
	assert_eq!(href_with_rel(&whole_feed, "previous"), None);
	assert_eq!(href_with_rel(&whole_feed, "first"), None);
	assert_eq!(href_with_rel(&whole_feed, "last"), None);

	let first_page = feed(&app, "/opds/v2.0/books/latest?page=1&page_size=2").await;
	assert!(href_with_rel(&first_page, "next").is_some());
	assert_eq!(href_with_rel(&first_page, "previous"), None);
	assert_eq!(
		href_with_rel(&first_page, "last").map(|href| server_path(&href)),
		Some("/opds/v2.0/books/latest?page=3&page_size=2".to_string())
	);

	let last_page = feed(&app, "/opds/v2.0/books/latest?page=3&page_size=2").await;
	assert_eq!(href_with_rel(&last_page, "next"), None);
	assert!(href_with_rel(&last_page, "previous").is_some());
	assert_eq!(
		href_with_rel(&last_page, "first").map(|href| server_path(&href)),
		Some("/opds/v2.0/books/latest?page=1&page_size=2".to_string())
	);
}

/// the library list should name the route that serves it, page it, and offer
/// only links a client can follow
#[tokio::test]
async fn test_library_list_links_are_mounted_and_paged() {
	let app = setup().await;

	let page_one = feed(&app, "/opds/v2.0/libraries?page=1&page_size=2").await;

	assert_eq!(
		href_with_rel(&page_one, "self").map(|href| server_path(&href)),
		Some("/opds/v2.0/libraries?page=1&page_size=2".to_string())
	);
	assert_eq!(
		navigation_titles(&page_one),
		vec!["Image Classics", "Image Comics"]
	);
	assert_eq!(pagination_metadata(&page_one), (3, 2, 1));
	assert_every_link_resolves(&app, &page_one).await;

	let next = href_with_rel(&page_one, "next").expect("3 libraries should page");
	let page_two = feed(&app, &server_path(&next)).await;

	assert_eq!(navigation_titles(&page_two), vec!["Image Digital"]);
}

/// the catalog is the entry point every client starts from: each of its own
/// links and each of its groups' links must be served
#[tokio::test]
async fn test_catalog_links_are_mounted() {
	let app = setup().await;

	let catalog = feed(&app, "/opds/v2.0/catalog").await;

	assert_every_link_resolves(&app, &catalog).await;
	assert_eq!(
		href_with_rel(&group_titled(&catalog, "Libraries"), "self")
			.map(|href| server_path(&href)),
		Some("/opds/v2.0/libraries?page=1&page_size=10".to_string())
	);
}

/// a library's own feed groups its books and series, and each group must name a
/// route that serves that group's page
#[tokio::test]
async fn test_library_feed_group_links_are_mounted() {
	let app = setup().await;

	let library = feed(&app, "/opds/v2.0/libraries/image_comics").await;

	assert_every_link_resolves(&app, &library).await;
	assert_eq!(
		href_with_rel(&group_titled(&library, "Library Books - All"), "self")
			.map(|href| server_path(&href)),
		Some("/opds/v2.0/libraries/image_comics/books?page=1&page_size=10".to_string())
	);
}
