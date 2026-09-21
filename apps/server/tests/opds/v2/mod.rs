mod browse;
#[cfg(feature = "graphql")]
mod keep_reading;
mod progression;
mod search;

use serde_json::Value;

use crate::common::TestApp;

/// The hrefs of a feed's or group's links carrying `rel`, in feed order.
pub(crate) fn hrefs_with_rel(collection: &Value, rel: &str) -> Vec<String> {
	collection
		.get("links")
		.and_then(Value::as_array)
		.map(|links| {
			links
				.iter()
				.filter(|link| link.get("rel").and_then(Value::as_str) == Some(rel))
				.filter_map(|link| link.get("href").and_then(Value::as_str))
				.map(ToString::to_string)
				.collect()
		})
		.unwrap_or_default()
}

/// The one href carrying `rel`, if the feed emitted it at all.
pub(crate) fn href_with_rel(collection: &Value, rel: &str) -> Option<String> {
	hrefs_with_rel(collection, rel).into_iter().next()
}

/// The pagination metadata of a feed or group as `(numberOfItems,
/// itemsPerPage, currentPage)`.
pub(crate) fn pagination_metadata(collection: &Value) -> (u64, u64, u64) {
	let metadata = collection
		.get("metadata")
		.expect("expected feed metadata")
		.clone();
	let number = |key: &str| {
		metadata
			.get(key)
			.and_then(Value::as_u64)
			.unwrap_or_else(|| panic!("expected {key} in metadata"))
	};

	(
		number("numberOfItems"),
		number("itemsPerPage"),
		number("currentPage"),
	)
}

/// The book ids of a feed's or group's publications, in feed order.
pub(crate) fn publication_ids(collection: &Value) -> Vec<String> {
	collection
		.get("publications")
		.and_then(Value::as_array)
		.expect("expected a publications array")
		.iter()
		.map(|publication| {
			let href = href_with_rel(publication, "self")
				.expect("publication should link to itself");
			href.rsplit("/books/")
				.next()
				.and_then(|tail| tail.split('/').next())
				.expect("publication self link should name a book")
				.to_string()
		})
		.collect()
}

/// The titles of a feed's or group's navigation entries, in feed order.
pub(crate) fn navigation_titles(collection: &Value) -> Vec<String> {
	collection
		.get("navigation")
		.and_then(Value::as_array)
		.expect("expected a navigation array")
		.iter()
		.filter_map(|link| link.get("title").and_then(Value::as_str))
		.map(ToString::to_string)
		.collect()
}

/// The group of a feed whose metadata carries `title`.
pub(crate) fn group_titled(feed: &Value, title: &str) -> Value {
	feed.get("groups")
		.and_then(Value::as_array)
		.expect("expected a groups array")
		.iter()
		.find(|group| {
			group
				.get("metadata")
				.and_then(|metadata| metadata.get("title"))
				.and_then(Value::as_str)
				== Some(title)
		})
		.unwrap_or_else(|| panic!("feed should carry a {title} group"))
		.clone()
}

/// The server path of an absolute OPDS href. Feeds carry absolute URLs built
/// from the request origin; the test client speaks in paths.
pub(crate) fn server_path(href: &str) -> String {
	href.split_once("://")
		.and_then(|(_, rest)| rest.find('/').map(|at| rest[at..].to_string()))
		.unwrap_or_else(|| href.to_string())
}

/// Follow every link the feed offers for navigation — its own, its groups' and
/// its navigation entries' — and assert the server serves it. An OPDS client
/// moves by link, so an href with no route behind it is a dead end it cannot
/// route around. Publication links are excluded: acquisition and thumbnail
/// need bytes on disk, which fixture books do not have.
pub(crate) async fn assert_every_link_resolves(app: &TestApp, feed: &Value) {
	let mut collections = vec![feed.clone()];
	if let Some(groups) = feed.get("groups").and_then(Value::as_array) {
		collections.extend(groups.iter().cloned());
	}

	let mut followed = 0;
	for collection in collections {
		let links = ["links", "navigation"]
			.into_iter()
			.filter_map(|key| collection.get(key).and_then(Value::as_array))
			.flatten()
			.cloned()
			.collect::<Vec<Value>>();

		for link in links {
			if link.get("templated").and_then(Value::as_bool) == Some(true) {
				continue;
			}

			let href = link
				.get("href")
				.and_then(Value::as_str)
				.expect("every link should carry an href");
			let path = server_path(href);
			let response = app.get(&path).await;

			assert!(
				response.status_code().is_success(),
				"{} link {path} returned {}",
				link.get("rel").and_then(Value::as_str).unwrap_or("untyped"),
				response.status_code()
			);
			followed += 1;
		}
	}

	// a walk over nothing would assert nothing
	assert!(followed > 0, "feed offered no followable link");
}
