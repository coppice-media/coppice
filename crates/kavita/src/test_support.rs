//! Shared in-memory test fixtures for the route tests: an in-memory SQLite
//! database with the Kavita id table materialized and a minimal
//! [`KavitaBackend`] stub over it.

use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use models::entity::{
	collection, collection_series, library, library_config, media, reading_list,
	reading_list_item, series, user::AuthUser,
};
use models::shared::enums::LibraryType;
use models::shared::image::{ImageMetadata, ImageRef};
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend,
	DbBackend, EntityTrait, QueryFilter, Schema, Statement,
};

use crate::errors::{APIError, APIResult};
use crate::ids::CREATE_KAVITA_IDS_SQL;
use crate::progress::CREATE_KAVITA_PROGRESS_SQL;
use crate::routes::{
	KavitaBackend, KavitaBookResource, KavitaBookStructure, KavitaImage,
	KavitaSeriesTarget, ServerFacts,
};

/// An in-memory database with every entity table plus the Kavita side tables
/// (`kavita_ids`, `kavita_progress`, `kavita_on_deck_removals`).
pub(crate) async fn db() -> sea_orm::DatabaseConnection {
	let conn = ::tests::db::test_database().await;
	for sql in [CREATE_KAVITA_IDS_SQL, CREATE_KAVITA_PROGRESS_SQL] {
		conn.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
			.await
			.unwrap();
	}

	// Tables the shared entity fixture does not create: the on-deck removals,
	// the two favourite tables (the want-to-read shelf), and the
	// bookmark/annotation/collection rows the Kavita bookmark, annotation and
	// collection routes project.
	let schema = Schema::new(DbBackend::Sqlite);
	for mut statement in [
		schema.create_table_from_entity(models::entity::kavita_on_deck_removal::Entity),
		schema.create_table_from_entity(models::entity::favorite_series::Entity),
		schema.create_table_from_entity(models::entity::favorite_media::Entity),
		schema.create_table_from_entity(models::entity::bookmark::Entity),
		schema.create_table_from_entity(models::entity::media_annotation::Entity),
		schema.create_table_from_entity(models::entity::collection::Entity),
		schema.create_table_from_entity(models::entity::collection_series::Entity),
		schema.create_table_from_entity(models::entity::tag::Entity),
		schema.create_table_from_entity(models::entity::series_tag::Entity),
		schema.create_table_from_entity(models::entity::media_tag::Entity),
	] {
		// Some of these already ship in the shared entity fixture; creating
		// the rest must not depend on which.
		let statement = statement.if_not_exists();
		conn.execute(conn.get_database_backend().build(statement))
			.await
			.unwrap();
	}
	conn
}

/// An `AuthUser` for a freshly inserted user row.
pub(crate) fn auth_user(user: &models::entity::user::Model) -> AuthUser {
	AuthUser {
		id: user.id.clone(),
		avatar_path: None,
		avatar: ImageRef::default(),
		username: user.username.clone(),
		is_server_owner: user.is_server_owner,
		is_locked: false,
		permissions: Vec::new(),
		age_restriction: None,
		preferences: None,
		device_library_scope: None,
	}
}

/// A library of the given Stump type; `fake_data::Library` always creates the
/// default (`Mixed`) config, so the type is set afterwards.
pub(crate) async fn library_of_type(
	conn: &sea_orm::DatabaseConnection,
	library_type: LibraryType,
) -> library::Model {
	let library_row = ::tests::fake_data::Library::default().insert(conn).await;
	let config = library_config::Entity::find_by_id(library_row.config_id)
		.one(conn)
		.await
		.unwrap()
		.expect("library config");
	library_config::ActiveModel {
		library_type: Set(library_type),
		..config.into()
	}
	.update(conn)
	.await
	.unwrap();
	library_row
}

/// A series holding one media item per `(name, extension, pages)`; the media
/// keep the given order under Kavita's name sort.
pub(crate) async fn series_with_files(
	conn: &sea_orm::DatabaseConnection,
	library_id: &str,
	name: &str,
	files: &[(&str, &str, i32)],
) -> (series::Model, Vec<media::Model>) {
	let series_row = ::tests::fake_data::Series {
		name: Some(name.to_owned()),
		library_id: Some(library_id.to_owned()),
		..Default::default()
	}
	.insert(conn)
	.await;
	let mut media_rows = Vec::with_capacity(files.len());
	for (name, extension, pages) in files {
		media_rows.push(
			::tests::fake_data::Media {
				series_id: series_row.id.clone(),
				name: Some((*name).to_owned()),
				extension: Some((*extension).to_owned()),
				pages: Some(*pages),
				..Default::default()
			}
			.insert(conn)
			.await,
		);
	}
	(series_row, media_rows)
}

/// A job the routes asked the server to enqueue. The server owns the queue;
/// the stub records the request so a route test can assert which Stump job a
/// Kavita client's call maps onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnqueuedJob {
	LibraryScan {
		library_id: String,
		path: String,
		force: bool,
	},
	SeriesScan {
		series_id: String,
		path: String,
		force: bool,
	},
	SeriesAnalysis {
		series_id: String,
	},
}

/// A [`KavitaBackend`] answering every persistence need from an in-memory
/// database. The file-backed methods answer for the media items a test
/// registered with [`TestBackend::store_file`] and are "unreachable" for
/// every other row, which is how the routes that never serve files see them.
pub(crate) struct TestBackend {
	pub conn: sea_orm::DatabaseConnection,
	/// The EPUB structure the `Book` routes see; `None` means "not a book".
	pub book_structure: Option<KavitaBookStructure>,
	/// Jobs the routes enqueued, in order.
	pub jobs: std::sync::Mutex<Vec<EnqueuedJob>>,
	/// Real files behind media ids, for the download and reader routes.
	files: std::sync::Mutex<std::collections::HashMap<String, std::path::PathBuf>>,
	/// Keeps the temporary directory holding those files alive.
	file_root: tempfile::TempDir,
	/// Real page images behind `(media id, 1-based page)`, for the routes that
	/// read a page's bytes rather than just pass them through.
	page_images: std::sync::Mutex<std::collections::HashMap<(String, i32), Vec<u8>>>,
}

impl TestBackend {
	pub fn new(conn: sea_orm::DatabaseConnection) -> Self {
		Self {
			conn,
			book_structure: None,
			jobs: std::sync::Mutex::new(Vec::new()),
			files: std::sync::Mutex::new(std::collections::HashMap::new()),
			file_root: tempfile::tempdir().expect("temp dir"),
			page_images: std::sync::Mutex::new(std::collections::HashMap::new()),
		}
	}

	/// The jobs enqueued so far, in order.
	pub fn enqueued(&self) -> Vec<EnqueuedJob> {
		self.jobs.lock().expect("job log").clone()
	}

	/// Write `contents` to a real file and bind it to `media_id`; returns the
	/// path to store on the media row.
	pub fn store_file(&self, media_id: &str, file_name: &str, contents: &[u8]) -> String {
		let path = self.file_root.path().join(media_id);
		std::fs::create_dir_all(&path).expect("media dir");
		let path = path.join(file_name);
		std::fs::write(&path, contents).expect("media file");
		self.files
			.lock()
			.expect("file map")
			.insert(media_id.to_owned(), path.clone());
		path.to_string_lossy().into_owned()
	}

	/// Bind real image bytes to one page of `media_id` (1-based, Stump
	/// numbering). A provider-backed row has no file to read a page from, and
	/// the reader measures the bytes it is handed — so registering a single
	/// page pins which page a probe is allowed to look at.
	pub fn store_page_image(&self, media_id: &str, page: i32, image: &[u8]) {
		self.page_images
			.lock()
			.expect("page image map")
			.insert((media_id.to_owned(), page), image.to_vec());
	}

	fn file_for(&self, media_id: &str) -> Option<std::path::PathBuf> {
		self.files.lock().expect("file map").get(media_id).cloned()
	}
}

/// Drive a request through the authenticated Kavita router with `user`
/// already resolved, the way the server's Kavita middleware leaves it. Tests
/// that go through here exercise route registration, casing, query parsing
/// and the serialized DTOs, not just the handler body.
pub(crate) async fn request(
	backend: std::sync::Arc<TestBackend>,
	user: &AuthUser,
	method: &str,
	uri: &str,
	body: Option<serde_json::Value>,
) -> (axum::http::StatusCode, serde_json::Value) {
	let (status, bytes) = request_raw(backend, user, method, uri, body).await;
	let json = if bytes.is_empty() {
		serde_json::Value::Null
	} else {
		serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
	};
	(status, json)
}

/// Drive an authenticated Kavita request and return its response bytes
/// unchanged, for image-cover readback tests.
pub(crate) async fn request_raw(
	backend: std::sync::Arc<TestBackend>,
	user: &AuthUser,
	method: &str,
	uri: &str,
	body: Option<serde_json::Value>,
) -> (axum::http::StatusCode, Vec<u8>) {
	use tower::ServiceExt;
	let router = crate::routes::router::<()>(backend).layer(axum::Extension(
		stump_auth::AuthContext {
			user: user.clone(),
			api_key: None,
			// A test request authenticates as the user, with no device, so
			// visibility is the user's own.
			device_id: None,
		},
	));
	let builder = axum::http::Request::builder().method(method).uri(uri);
	let request = match body {
		Some(json) => {
			let builder =
				builder.header(axum::http::header::CONTENT_TYPE, "application/json");
			builder
				.body(Body::from(serde_json::to_vec(&json).expect("json body")))
				.expect("request")
		},
		None => builder.body(Body::empty()).expect("request"),
	};
	let response = router.oneshot(request).await.expect("router response");
	let status = response.status();
	let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
		.await
		.expect("response body")
		.to_vec();
	(status, bytes)
}

#[async_trait::async_trait]
impl KavitaBackend for TestBackend {
	fn conn(&self) -> &sea_orm::DatabaseConnection {
		&self.conn
	}

	async fn token_secret(&self) -> APIResult<Vec<u8>> {
		Ok(vec![b'x'; 32])
	}

	async fn authenticate_api_key(&self, _api_key: &str) -> APIResult<AuthUser> {
		Err(APIError::Unauthorized)
	}

	async fn authenticate_password(
		&self,
		_username: &str,
		_password: &str,
	) -> APIResult<AuthUser> {
		Err(APIError::Unauthorized)
	}

	fn server_facts(&self) -> ServerFacts {
		ServerFacts {
			is_docker: false,
			first_install_date: None,
		}
	}

	/// The image bytes registered with [`TestBackend::store_page_image`], and
	/// otherwise a recognisable stand-in so a route test can tell "the page
	/// was served" from "the page was refused".
	async fn media_page(
		&self,
		_user: &AuthUser,
		media_id: &str,
		page: i32,
	) -> APIResult<KavitaImage> {
		let image = self
			.page_images
			.lock()
			.expect("page image map")
			.get(&(media_id.to_owned(), page))
			.cloned();
		Ok(KavitaImage::new(
			"image/png".to_owned(),
			image.unwrap_or_else(|| format!("{media_id}:{page}").into_bytes()),
		))
	}

	async fn media_thumbnail(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> APIResult<KavitaImage> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id.to_owned()))
			.filter(media::Column::DeletedAt.is_null())
			.one(&self.conn)
			.await?
			.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
		let path = book.thumbnail_path.ok_or_else(|| {
			APIError::NotFound("Chapter cover does not exist".to_owned())
		})?;
		let bytes = tokio::fs::read(&path)
			.await
			.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		Ok(KavitaImage::new("image/png", bytes))
	}

	async fn series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: &str,
	) -> APIResult<KavitaImage> {
		let series = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id.to_owned()))
			.filter(series::Column::DeletedAt.is_null())
			.one(&self.conn)
			.await?
			.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?;
		let path = series.thumbnail_path.ok_or_else(|| {
			APIError::NotFound("Series cover does not exist".to_owned())
		})?;
		let bytes = tokio::fs::read(&path)
			.await
			.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		Ok(KavitaImage::new("image/png", bytes))
	}

	async fn upload_series_cover(
		&self,
		user: &AuthUser,
		target: KavitaSeriesTarget,
		bytes: Vec<u8>,
		lock_cover: bool,
	) -> APIResult<()> {
		let thumbnail_id = match &target {
			KavitaSeriesTarget::Series(series_id) => series::Entity::find_for_user(user)
				.filter(series::Column::Id.eq(series_id.to_owned()))
				.filter(series::Column::DeletedAt.is_null())
				.one(&self.conn)
				.await?
				.map(|row| row.id)
				.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))?,
			KavitaSeriesTarget::Book(media_id) => media::Entity::find_for_user(user)
				.filter(media::Column::Id.eq(media_id.to_owned()))
				.filter(media::Column::DeletedAt.is_null())
				.one(&self.conn)
				.await?
				.map(|row| row.id)
				.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?,
		};
		let path = self
			.file_root
			.path()
			.join(format!("cover-{thumbnail_id}.png"));
		tokio::fs::write(&path, bytes)
			.await
			.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		self.persist_series_cover(
			user,
			target,
			path.to_string_lossy().into_owned(),
			ImageMetadata::default(),
			lock_cover,
		)
		.await
	}

	async fn upload_media_cover(
		&self,
		user: &AuthUser,
		media_id: String,
		bytes: Vec<u8>,
		lock_cover: bool,
	) -> APIResult<()> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.filter(media::Column::DeletedAt.is_null())
			.one(&self.conn)
			.await?
			.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
		let path = self.file_root.path().join(format!("cover-{}.png", book.id));
		tokio::fs::write(&path, bytes)
			.await
			.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		self.persist_media_cover(
			user,
			book.id,
			path.to_string_lossy().into_owned(),
			ImageMetadata::default(),
			lock_cover,
		)
		.await
	}

	/// Serves a registered file through the very service the server uses
	/// (`tower_http::services::ServeFile`), so a route test sees the real
	/// `Range`/`Content-Length` behaviour of a download.
	async fn serve_media_file(
		&self,
		_auth: stump_auth::AuthContext,
		headers: HeaderMap,
		media_id: &str,
	) -> APIResult<Response<Body>> {
		let path = self.file_for(media_id).ok_or_else(|| {
			APIError::NotFound(format!("unreachable in tests: {media_id}"))
		})?;
		let mut request = axum::http::Request::new(Body::empty());
		*request.headers_mut() = headers;
		tower_http::services::ServeFile::new(path)
			.try_call(request)
			.await
			.map(|response| response.map(Body::new))
			.map_err(|error| APIError::InternalServerError(error.to_string()))
	}

	/// The registered file's bytes; a media item without one (a
	/// provider-backed row) gets deterministic stand-in bytes, so a route
	/// test can tell "the generated archive was served" from "the archive
	/// was refused".
	async fn media_bytes(&self, _user: &AuthUser, media_id: &str) -> APIResult<Vec<u8>> {
		match self.file_for(media_id) {
			Some(path) => std::fs::read(path)
				.map_err(|error| APIError::InternalServerError(error.to_string())),
			None => Ok(format!("archive:{media_id}").into_bytes()),
		}
	}

	async fn book_structure(
		&self,
		_user: &AuthUser,
		media_id: &str,
	) -> APIResult<KavitaBookStructure> {
		self.book_structure
			.clone()
			.ok_or_else(|| APIError::NotFound(format!("no book fixture: {media_id}")))
	}

	async fn book_page(
		&self,
		_user: &AuthUser,
		media_id: &str,
		spine_index: usize,
	) -> APIResult<KavitaBookResource> {
		let structure = self
			.book_structure
			.as_ref()
			.ok_or_else(|| APIError::NotFound(format!("no book fixture: {media_id}")))?;
		if spine_index >= structure.spine.len() {
			return Err(APIError::BadRequest(format!(
				"spine item {spine_index} does not exist"
			)));
		}
		Ok(KavitaBookResource {
			content_type: "application/xhtml+xml".to_owned(),
			data: format!(
				"<p>spine {spine_index}</p><img src=\"epub://OEBPS/i{spine_index}.png\"/>"
			)
			.into_bytes(),
		})
	}

	async fn book_resource(
		&self,
		_user: &AuthUser,
		_media_id: &str,
		file: &str,
	) -> APIResult<KavitaBookResource> {
		Ok(KavitaBookResource {
			content_type: "image/png".to_owned(),
			data: file.as_bytes().to_vec(),
		})
	}

	/// The canonical container service lives in `stump_core`, which this crate
	/// does not depend on; the stub writes the rows that service would.
	async fn create_read_list(
		&self,
		user: &AuthUser,
		name: String,
	) -> APIResult<reading_list::Model> {
		let model = reading_list::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			name: Set(name),
			description: Set(None),
			updated_at: Set(chrono::Utc::now().into()),
			visibility: Set("PRIVATE".to_owned()),
			ordering: Set("MANUAL".to_owned()),
			kobo_shelf: Set(true),
			source_device: Set(None),
			creating_user_id: Set(user.id.clone()),
		}
		.insert(&self.conn)
		.await?;
		Ok(model)
	}

	async fn set_read_list_items(
		&self,
		_user: &AuthUser,
		id: &str,
		book_ids: Vec<String>,
	) -> APIResult<()> {
		reading_list_item::Entity::delete_many()
			.filter(reading_list_item::Column::ReadingListId.eq(id.to_owned()))
			.exec(&self.conn)
			.await?;
		for (index, media_id) in book_ids.into_iter().enumerate() {
			reading_list_item::ActiveModel {
				display_order: Set(i32::try_from(index).unwrap_or(i32::MAX)),
				media_id: Set(media_id),
				reading_list_id: Set(id.to_owned()),
				..Default::default()
			}
			.insert(&self.conn)
			.await?;
		}
		Ok(())
	}

	async fn delete_read_list(&self, _user: &AuthUser, id: &str) -> APIResult<()> {
		reading_list_item::Entity::delete_many()
			.filter(reading_list_item::Column::ReadingListId.eq(id.to_owned()))
			.exec(&self.conn)
			.await?;
		reading_list::Entity::delete_by_id(id.to_owned())
			.exec(&self.conn)
			.await?;
		Ok(())
	}

	async fn create_collection(
		&self,
		user: &AuthUser,
		name: String,
		series_ids: Vec<String>,
	) -> APIResult<collection::Model> {
		let model = collection::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			name: Set(name),
			description: Set(None),
			updated_at: Set(chrono::Utc::now().into()),
			ordered: Set(false),
			kobo_shelf: Set(true),
			source_device: Set(None),
			creating_user_id: Set(user.id.clone()),
		}
		.insert(&self.conn)
		.await?;
		self.set_collection_series(user, &model.id, series_ids)
			.await?;
		Ok(model)
	}

	async fn set_collection_series(
		&self,
		_user: &AuthUser,
		id: &str,
		series_ids: Vec<String>,
	) -> APIResult<()> {
		collection_series::Entity::delete_many()
			.filter(collection_series::Column::CollectionId.eq(id.to_owned()))
			.exec(&self.conn)
			.await?;
		for (index, series_id) in series_ids.into_iter().enumerate() {
			collection_series::ActiveModel {
				collection_id: Set(id.to_owned()),
				series_id: Set(series_id),
				display_order: Set(i32::try_from(index).unwrap_or(i32::MAX)),
				..Default::default()
			}
			.insert(&self.conn)
			.await?;
		}
		Ok(())
	}

	/// The job queue lives in the server; the stub records what was enqueued
	/// so the route tests can assert the job a client's request maps onto.
	async fn enqueue_library_scan(
		&self,
		library_id: String,
		path: String,
		force: bool,
	) -> APIResult<()> {
		self.jobs
			.lock()
			.expect("job log")
			.push(EnqueuedJob::LibraryScan {
				library_id,
				path,
				force,
			});
		Ok(())
	}

	async fn enqueue_series_scan(
		&self,
		series_id: String,
		path: String,
		force: bool,
	) -> APIResult<()> {
		self.jobs
			.lock()
			.expect("job log")
			.push(EnqueuedJob::SeriesScan {
				series_id,
				path,
				force,
			});
		Ok(())
	}

	async fn enqueue_series_analysis(&self, series_id: String) -> APIResult<()> {
		self.jobs
			.lock()
			.expect("job log")
			.push(EnqueuedJob::SeriesAnalysis { series_id });
		Ok(())
	}
}
