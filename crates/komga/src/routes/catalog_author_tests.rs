use super::*;
use crate::{
	routes::{KomgaCoreEvent, KomgaImage},
	KomgaBookPage, KomgaBookThumbnail, KomgaLibraryCreateRequest,
	KomgaLibraryUpdateRequest, KomgaSeriesThumbnail,
};
use axum::http::{header, Request, StatusCode};
use models::entity::{age_restriction, collection, library_exclusion};
use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
use serde_json::{json, Value as JsonValue};
use tower::ServiceExt;

struct CatalogBackend {
	conn: Arc<DatabaseConnection>,
}

// The catalog uses a real SQLite database. Any unexpected platform operation
// fails the test instead of fabricating a file, image, job, or mutation result.
macro_rules! reject_non_catalog_operations {
	($($name:ident($($argument:ident: $argument_type:ty),*) -> $result:ty;)*) => {
		#[async_trait::async_trait]
		impl KomgaBackend for CatalogBackend {
			fn conn(&self) -> &DatabaseConnection {
				&self.conn
			}

			fn conn_arc(&self) -> Arc<DatabaseConnection> {
				self.conn.clone()
			}

			fn core_events(&self) -> tokio::sync::broadcast::Receiver<KomgaCoreEvent> {
				unreachable!("catalog search must not subscribe to events")
			}

			fn library_roots(&self) -> Vec<String> {
				unreachable!("catalog search must not browse filesystem roots")
			}

			async fn record_sync(&self, _auth: &AuthContext, _summary: JsonValue) {
				unreachable!("catalog search must not record a sync")
			}

			$(async fn $name(&self, $($argument: $argument_type),*) -> APIResult<$result> {
				unreachable!("catalog search must not call {}", stringify!($name))
			})*
		}
	};
}

reject_non_catalog_operations! {
	enqueue_library_scan(_library_id: String, _path: String, _deep: bool) -> ();
	enqueue_library_analysis(_library_id: String) -> ();
	enqueue_book_analysis(_book_id: String) -> ();
	enqueue_series_analysis(_series_id: String) -> ();
	book_pages(_book: &media::Model) -> Vec<KomgaBookPage>;
	visible_pages(_book: &media::Model) -> Option<Vec<i32>>;
	book_page(_user: &AuthUser, _book_id: String, _page: u32) -> KomgaImage;
	book_page_thumbnail(_user: &AuthUser, _book_id: String, _page: u32) -> KomgaImage;
	book_thumbnail(_user: &AuthUser, _book_id: String) -> KomgaImage;
	book_thumbnails(_user: &AuthUser, _book_id: String) -> Vec<KomgaBookThumbnail>;
	book_thumbnail_by_id(_user: &AuthUser, _book_id: String, _thumbnail_id: String) -> KomgaImage;
	upload_book_thumbnail(_user: &AuthUser, _book_id: String, _bytes: Vec<u8>, _selected: bool) -> KomgaBookThumbnail;
	delete_book_thumbnail(_user: &AuthUser, _book_id: String, _thumbnail_id: String) -> ();
	series_thumbnail(_user: &AuthUser, _series_id: &str) -> KomgaImage;
	series_thumbnails(_user: &AuthUser, _series_id: String) -> Vec<KomgaSeriesThumbnail>;
	series_thumbnail_by_id(_user: &AuthUser, _series_id: String, _thumbnail_id: String) -> KomgaImage;
	upload_series_thumbnail(_user: &AuthUser, _series_id: String, _bytes: Vec<u8>, _selected: bool) -> KomgaSeriesThumbnail;
	delete_series_thumbnail(_user: &AuthUser, _series_id: String, _thumbnail_id: String) -> ();
	serve_book_file(_auth: AuthContext, _headers: HeaderMap, _book_id: String) -> Response<Body>;
	readium_manifest(_path: String, _base_url: String) -> JsonValue;
	readium_positions(_path: String, _base_url: String) -> JsonValue;
	readium_resource(_path: String, _resource_path: std::path::PathBuf) -> KomgaImage;
	create_library(_request: KomgaLibraryCreateRequest) -> library::Model;
	update_library(_user: &AuthUser, _id: &str, _request: KomgaLibraryUpdateRequest) -> ();
	delete_library(_user: &AuthUser, _id: &str) -> ();
	create_collection(_user: &AuthUser, _name: String, _ordered: bool, _series_ids: Vec<String>) -> collection::Model;
	update_collection(_user: &AuthUser, _id: &str, _name: Option<String>, _ordered: Option<bool>, _series_ids: Option<Vec<String>>) -> ();
	delete_collection(_user: &AuthUser, _id: &str) -> ();
	create_read_list(_user: &AuthUser, _name: String, _summary: Option<String>, _ordered: bool, _book_ids: Vec<String>) -> reading_list::Model;
	update_read_list(_user: &AuthUser, _id: &str, _name: Option<String>, _summary: Option<Option<String>>, _ordered: Option<bool>, _book_ids: Option<Vec<String>>) -> ();
	delete_read_list(_user: &AuthUser, _id: &str) -> ();
}

struct CatalogFixture {
	conn: Arc<DatabaseConnection>,
	user: AuthUser,
}

impl CatalogFixture {
	async fn new() -> Self {
		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		<migrations::Migrator as migrations::MigratorTrait>::up(&*conn, None)
			.await
			.unwrap();
		let mut user = ::tests::fake_data::User::new("author-search")
			.auth_user(&conn)
			.await;
		user.age_restriction = Some(age_restriction::Model {
			id: 1,
			age: 12,
			restrict_on_unset: false,
			user_id: user.id.clone(),
		});
		for id in ["visible", "hidden", "device-excluded"] {
			::tests::fake_data::Library {
				id: Some(id.to_owned()),
				name: Some(id.to_owned()),
				path: Some(format!("/library/{id}")),
			}
			.insert(&conn)
			.await;
		}
		library_exclusion::ActiveModel {
			user_id: Set(user.id.clone()),
			library_id: Set("hidden".to_owned()),
			..Default::default()
		}
		.insert(&*conn)
		.await
		.unwrap();
		Self { conn, user }
	}

	fn router(&self) -> Router {
		let backend: Arc<dyn KomgaBackend> = Arc::new(CatalogBackend {
			conn: self.conn.clone(),
		});
		super::routes()
			.merge(super::legacy_books())
			.layer(Extension(backend))
			.layer(Extension(AuthContext {
				user: self.user.clone(),
				api_key: None,
				device_id: None,
			}))
	}

	async fn series(
		&self,
		id: &str,
		library_id: &str,
		writers: Option<&str>,
	) -> series::Model {
		let model = series::ActiveModel {
			id: Set(id.to_owned()),
			name: Set(id.to_owned()),
			path: Set(format!("/library/{library_id}/{id}")),
			status: Set(FileStatus::Ready),
			library_id: Set(Some(library_id.to_owned())),
			..Default::default()
		}
		.insert(&*self.conn)
		.await
		.unwrap();
		series_metadata::ActiveModel {
			series_id: Set(id.to_owned()),
			age_rating: Set(Some(0)),
			writers: Set(writers.map(str::to_owned)),
			title_sort_lock: Set(false),
			reading_direction_lock: Set(false),
			language_lock: Set(false),
			alternate_titles_lock: Set(false),
			..Default::default()
		}
		.insert(&*self.conn)
		.await
		.unwrap();
		model
	}

	async fn book(
		&self,
		series_id: &str,
		id: &str,
		metadata: Option<media_metadata::ActiveModel>,
	) -> media::Model {
		let model = media::ActiveModel {
			id: Set(id.to_owned()),
			name: Set(id.to_owned()),
			path: Set(format!("/library/{series_id}/{id}.epub")),
			extension: Set("epub".to_owned()),
			series_id: Set(Some(series_id.to_owned())),
			pages: Set(1),
			size: Set(1),
			status: Set(FileStatus::Ready),
			..Default::default()
		}
		.insert(&*self.conn)
		.await
		.unwrap();
		if let Some(mut metadata) = metadata {
			metadata.media_id = Set(Some(id.to_owned()));
			metadata.insert(&*self.conn).await.unwrap();
		}
		model
	}

	async fn assert_search(
		&self,
		resource: &str,
		condition: JsonValue,
		expected: &[&str],
	) {
		let response = self
			.router()
			.oneshot(
				Request::post(format!("/api/v1/{resource}/list?unpaged=true"))
					.header(header::CONTENT_TYPE, "application/json")
					.body(Body::from(json!({"condition": condition}).to_string()))
					.unwrap(),
			)
			.await
			.unwrap();
		assert_page(response, expected).await;
	}
}

fn credit(role: &str, name: &str) -> media_metadata::ActiveModel {
	let mut metadata: media_metadata::ActiveModel = Default::default();
	let value = Set(Some(name.to_owned()));
	match role {
		"writer" => metadata.writers = value,
		"penciller" => metadata.pencillers = value,
		"inker" => metadata.inkers = value,
		"colorist" => metadata.colorists = value,
		"letterer" => metadata.letterers = value,
		"cover" => metadata.cover_artists = value,
		"editor" => metadata.editors = value,
		_ => panic!("invalid test author role"),
	}
	metadata
}

fn author(operator: &str, value: JsonValue) -> JsonValue {
	json!({"author": {"operator": operator, "value": value}})
}

async fn assert_page(response: Response<Body>, expected: &[&str]) {
	let status = response.status();
	let body = axum::body::to_bytes(response.into_body(), usize::MAX)
		.await
		.unwrap();
	assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
	let page: JsonValue = serde_json::from_slice(&body).unwrap();
	let mut actual: Vec<_> = page["content"]
		.as_array()
		.unwrap()
		.iter()
		.map(|item| item["id"].as_str().unwrap())
		.collect();
	actual.sort_unstable();
	let mut expected = expected.to_vec();
	expected.sort_unstable();
	assert_eq!(actual, expected);
	assert_eq!(page["totalElements"], expected.len());
}

#[tokio::test]
async fn empty_author_is_and_is_not_partition_books_and_child_author_series() {
	let fixture = CatalogFixture::new().await;
	for id in [
		"written",
		"pencilled",
		"bare",
		"blank",
		"empty-metadata",
		"mixed",
	] {
		fixture.series(id, "visible", None).await;
	}
	fixture
		.series("series-writer-only", "visible", Some("Series Writer"))
		.await;
	fixture
		.book("written", "written-book", Some(credit("writer", "Writer")))
		.await;
	fixture
		.book(
			"pencilled",
			"pencilled-book",
			Some(credit("penciller", "Artist")),
		)
		.await;
	fixture.book("bare", "bare-book", None).await;
	fixture
		.book(
			"blank",
			"blank-book",
			Some(credit("writer", " \t, ,\n,\u{a0} ")),
		)
		.await;
	fixture
		.book(
			"empty-metadata",
			"empty-metadata-book",
			Some(Default::default()),
		)
		.await;
	fixture
		.book(
			"mixed",
			"mixed-authored-book",
			Some(credit("writer", "Writer")),
		)
		.await;
	fixture.book("mixed", "mixed-bare-book", None).await;
	fixture
		.book("series-writer-only", "series-writer-only-book", None)
		.await;
	// A NULL book reference must not poison NOT IN for authorless books.
	media_metadata::ActiveModel {
		media_id: Set(None),
		writers: Set(Some("Orphan".to_owned())),
		..Default::default()
	}
	.insert(&*fixture.conn)
	.await
	.unwrap();

	for value in [json!({}), json!({"name": null, "role": null})] {
		fixture
			.assert_search(
				"series",
				author("is", value.clone()),
				&["written", "pencilled", "mixed"],
			)
			.await;
		fixture
			.assert_search(
				"series",
				author("isNot", value.clone()),
				&["bare", "blank", "empty-metadata", "series-writer-only"],
			)
			.await;
		fixture
			.assert_search(
				"books",
				author("is", value.clone()),
				&["written-book", "pencilled-book", "mixed-authored-book"],
			)
			.await;
		fixture
			.assert_search(
				"books",
				author("isNot", value),
				&[
					"bare-book",
					"blank-book",
					"empty-metadata-book",
					"mixed-bare-book",
					"series-writer-only-book",
				],
			)
			.await;
	}
}

#[tokio::test]
async fn author_roles_and_names_match_child_credits_together() {
	let fixture = CatalogFixture::new().await;
	let roles = [
		"writer",
		"penciller",
		"inker",
		"colorist",
		"letterer",
		"cover",
		"editor",
	];
	for role in roles {
		fixture
			.series(role, "visible", Some("Unrelated Series Writer"))
			.await;
		fixture
			.book(role, role, Some(credit(role, "Shared Author")))
			.await;
	}
	for role in roles {
		for resource in ["books", "series"] {
			fixture
				.assert_search(
					resource,
					author("is", json!({"role": role.to_ascii_uppercase()})),
					&[role],
				)
				.await;
			fixture
				.assert_search(
					resource,
					author("is", json!({"name": "shared author", "role": role})),
					&[role],
				)
				.await;
			let other_roles: Vec<_> =
				roles.into_iter().filter(|other| *other != role).collect();
			fixture
				.assert_search(
					resource,
					author("isNot", json!({"name": "Shared Author", "role": role})),
					&other_roles,
				)
				.await;
			fixture
				.assert_search(
					resource,
					author(
						"is",
						json!({"name": "Unrelated Series Writer", "role": role}),
					),
					&[],
				)
				.await;
		}
	}
	for resource in ["books", "series"] {
		fixture
			.assert_search(
				resource,
				author("is", json!({"name": "Shared Author"})),
				&roles,
			)
			.await;
		fixture
			.assert_search(
				resource,
				author("is", json!({"role": "cover_artist"})),
				&["cover"],
			)
			.await;
		fixture
			.assert_search(resource, author("is", json!({"role": "unknown"})), &[])
			.await;
		fixture
			.assert_search(
				resource,
				author("isNot", json!({"role": "unknown"})),
				&roles,
			)
			.await;
	}

	fixture.series("cross-role", "visible", None).await;
	let mut metadata = credit("writer", "Alice");
	metadata.pencillers = Set(Some("Bob".to_owned()));
	fixture
		.book("cross-role", "cross-role", Some(metadata))
		.await;
	for resource in ["books", "series"] {
		fixture
			.assert_search(
				resource,
				author("is", json!({"name": "Bob", "role": "writer"})),
				&[],
			)
			.await;
		fixture
			.assert_search(
				resource,
				author("is", json!({"name": "Bob", "role": "penciller"})),
				&["cross-role"],
			)
			.await;
	}
	let response = fixture
		.router()
		.oneshot(
			Request::get("/api/v1/series?author=Shared%20Author,penciller&unpaged=true")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_page(response, &["penciller"]).await;
}

#[tokio::test]
async fn author_name_equality_uses_literal_trimmed_csv_tokens() {
	let fixture = CatalogFixture::new().await;
	for id in ["credits", "annette-only", "unicode"] {
		fixture.series(id, "visible", None).await;
	}
	fixture.book("credits", "credits", Some(credit("writer", "Annette,  Ann , O'Brien, \"A\\B\", 50%_Done; Jr., \tTabbed\n,\u{a0}Spaced\u{2003}"))).await;
	fixture
		.book(
			"annette-only",
			"annette-only",
			Some(credit("writer", "Annette")),
		)
		.await;
	fixture
		.book("unicode", "unicode", Some(credit("editor", "José")))
		.await;
	for resource in ["books", "series"] {
		for name in [
			"ann",
			"O'Brien",
			"\"A\\B\"",
			"50%_Done; Jr.",
			"Tabbed",
			"Spaced",
		] {
			fixture
				.assert_search(
					resource,
					author("is", json!({"name": name})),
					&["credits"],
				)
				.await;
		}
		fixture
			.assert_search(
				resource,
				author("isNot", json!({"name": "Ann"})),
				&["annette-only", "unicode"],
			)
			.await;
		for name in ["An", "50%", "50%_", "", " Ann "] {
			fixture
				.assert_search(resource, author("is", json!({"name": name})), &[])
				.await;
		}
		fixture
			.assert_search(resource, author("is", json!({"role": ""})), &[])
			.await;
		fixture
			.assert_search(
				resource,
				author("isNot", json!({"name": ""})),
				&["credits", "annette-only", "unicode"],
			)
			.await;
		fixture
			.assert_search(
				resource,
				author("is", json!({"name": "josé"})),
				&["unicode"],
			)
			.await;
		// SQLite NOCASE is not Komga's ICU PRIMARY/accent-insensitive collation.
		fixture
			.assert_search(resource, author("is", json!({"name": "jose"})), &[])
			.await;
		fixture
			.assert_search(resource, author("is", json!({"name": "JOSÉ"})), &[])
			.await;
	}
}

#[tokio::test]
async fn series_author_matches_ignore_hidden_deleted_missing_and_unsupported_books() {
	let mut fixture = CatalogFixture::new().await;
	fixture.user.device_library_scope =
		Some(vec!["visible".to_owned(), "hidden".to_owned()]);
	for id in [
		"visible-authored",
		"restricted",
		"deleted-child",
		"missing-child",
		"unsupported-child",
		"deleted-series",
	] {
		fixture.series(id, "visible", None).await;
		fixture.book(id, &format!("{id}-bare"), None).await;
	}
	fixture.series("hidden-library", "hidden", None).await;
	fixture
		.series("device-hidden-library", "device-excluded", None)
		.await;
	for id in [
		"visible-authored",
		"deleted-child",
		"missing-child",
		"unsupported-child",
		"deleted-series",
		"hidden-library",
		"device-hidden-library",
	] {
		let mut book: media::ActiveModel = fixture
			.book(
				id,
				&format!("{id}-credit"),
				Some(credit("editor", "Secret")),
			)
			.await
			.into();
		match id {
			"deleted-child" => {
				book.deleted_at = Set(Some(
					chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
				))
			},
			"missing-child" => book.status = Set(FileStatus::Missing),
			"unsupported-child" => book.extension = Set("mp3".to_owned()),
			_ => {},
		}
		book.update(&*fixture.conn).await.unwrap();
	}
	fixture.series("threshold-authored", "visible", None).await;
	let mut threshold_metadata = credit("editor", "Secret");
	threshold_metadata.age_rating = Set(Some(12));
	fixture
		.book(
			"threshold-authored",
			"threshold-credit",
			Some(threshold_metadata),
		)
		.await;
	let mut metadata = credit("editor", "Secret");
	metadata.age_rating = Set(Some(18));
	fixture
		.book("restricted", "restricted-credit", Some(metadata))
		.await;
	let mut deleted_series: series::ActiveModel =
		series::Entity::find_by_id("deleted-series")
			.one(&*fixture.conn)
			.await
			.unwrap()
			.unwrap()
			.into();
	deleted_series.deleted_at = Set(Some(
		chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
	));
	deleted_series.update(&*fixture.conn).await.unwrap();

	let authorless = [
		"restricted",
		"deleted-child",
		"missing-child",
		"unsupported-child",
	];
	for restrict_on_unset in [false, true] {
		fixture
			.user
			.age_restriction
			.as_mut()
			.unwrap()
			.restrict_on_unset = restrict_on_unset;
		for value in [
			json!({}),
			json!({"name": "Secret"}),
			json!({"name": "Secret", "role": "EDITOR"}),
		] {
			fixture
				.assert_search(
					"series",
					author("is", value.clone()),
					&["visible-authored", "threshold-authored"],
				)
				.await;
			fixture
				.assert_search("series", author("isNot", value), &authorless)
				.await;
		}
	}
}

#[tokio::test]
async fn author_search_preserves_threshold_and_unset_age_visibility() {
	let mut fixture = CatalogFixture::new().await;
	fixture.series("rated-series", "visible", None).await;
	fixture.series("unrated-series", "visible", None).await;
	let mut unrated_series_metadata: series_metadata::ActiveModel =
		series_metadata::Entity::find_by_id("unrated-series")
			.one(&*fixture.conn)
			.await
			.unwrap()
			.unwrap()
			.into();
	unrated_series_metadata.age_rating = Set(None);
	unrated_series_metadata
		.update(&*fixture.conn)
		.await
		.unwrap();
	for (id, series_id, rating) in [
		("at-threshold", "rated-series", Some(12)),
		("over-age", "rated-series", Some(18)),
		("unset-in-rated-series", "rated-series", None),
		("unset-in-unrated-series", "unrated-series", None),
	] {
		let mut metadata = credit("editor", "Editor");
		metadata.age_rating = Set(rating);
		fixture.book(series_id, id, Some(metadata)).await;
	}
	for restrict_on_unset in [false, true] {
		fixture
			.user
			.age_restriction
			.as_mut()
			.unwrap()
			.restrict_on_unset = restrict_on_unset;
		let expected = if restrict_on_unset {
			vec!["at-threshold", "unset-in-rated-series"]
		} else {
			vec![
				"at-threshold",
				"unset-in-rated-series",
				"unset-in-unrated-series",
			]
		};
		fixture
			.assert_search("books", author("is", json!({"role": "editor"})), &expected)
			.await;
	}
}

#[tokio::test]
async fn author_is_not_composes_with_nested_all_of_and_any_of() {
	let fixture = CatalogFixture::new().await;
	for id in ["writer", "artist", "both", "bare"] {
		fixture.series(id, "visible", None).await;
	}
	fixture
		.book("writer", "writer", Some(credit("writer", "Writer")))
		.await;
	fixture
		.book("artist", "artist", Some(credit("penciller", "Artist")))
		.await;
	fixture
		.book("both", "both-writer", Some(credit("writer", "Writer")))
		.await;
	fixture
		.book("both", "both-artist", Some(credit("penciller", "Artist")))
		.await;
	fixture.book("bare", "bare", None).await;
	let is_writer = author("is", json!({"name": "Writer", "role": "writer"}));
	let not_writer = author("isNot", json!({"name": "Writer", "role": "writer"}));
	fixture
		.assert_search(
			"series",
			json!({"allOf": [author("is", json!({})), not_writer.clone()]}),
			&["artist"],
		)
		.await;
	fixture
		.assert_search(
			"series",
			json!({"anyOf": [is_writer.clone(), not_writer.clone()]}),
			&["writer", "artist", "both", "bare"],
		)
		.await;
	fixture
		.assert_search(
			"series",
			json!({"allOf": [is_writer.clone(), not_writer.clone()]}),
			&[],
		)
		.await;
	fixture.assert_search("series", json!({"allOf": [{"anyOf": [author("is", json!({"role": "penciller"})), not_writer.clone()]}, is_writer.clone()]}), &["both"]).await;
	fixture
		.assert_search(
			"books",
			json!({"allOf": [author("is", json!({})), not_writer.clone()]}),
			&["artist", "both-artist"],
		)
		.await;
	fixture
		.assert_search(
			"books",
			json!({"anyOf": [is_writer.clone(), not_writer.clone()]}),
			&["writer", "artist", "both-writer", "both-artist", "bare"],
		)
		.await;
	fixture
		.assert_search("books", json!({"allOf": [is_writer, not_writer]}), &[])
		.await;
}

#[tokio::test]
async fn series_author_accepts_tagged_and_deduced_types_without_panicking() {
	let fixture = CatalogFixture::new().await;
	fixture.series("editor", "visible", None).await;
	fixture
		.book("editor", "editor", Some(credit("editor", "Editor")))
		.await;
	for operator in ["is", "isNot"] {
		let expected: &[&str] = if operator == "is" { &["editor"] } else { &[] };
		fixture
			.assert_search(
				"series",
				author(operator, json!({"role": "editor"})),
				expected,
			)
			.await;
		let mut condition = author(operator, json!({"role": "editor"}));
		condition["type"] = json!("Author");
		fixture.assert_search("series", condition, expected).await;
	}
	let response = fixture.router().oneshot(
		Request::post("/api/v1/series/list")
			.header(header::CONTENT_TYPE, "application/json")
			.body(Body::from(json!({"condition": {"type": "LibraryId", "author": {"operator": "is", "value": {}}}}).to_string()))
			.unwrap(),
	).await.unwrap();
	assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
