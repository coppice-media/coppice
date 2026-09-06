//! Shared in-memory test fixtures for the route tests: an in-memory SQLite
//! database with the Kavita id table materialized and a minimal
//! [`KavitaBackend`] stub over it.

use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use models::entity::{
	library, library_config, media, reading_list, reading_list_item, series,
	user::AuthUser,
};
use models::shared::enums::LibraryType;
use models::shared::image::ImageRef;
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend,
	DbBackend, EntityTrait, QueryFilter, Schema, Statement,
};

use crate::errors::{APIError, APIResult};
use crate::ids::CREATE_KAVITA_IDS_SQL;
use crate::progress::CREATE_KAVITA_PROGRESS_SQL;
use crate::routes::{
	KavitaBackend, KavitaBookResource, KavitaBookStructure, KavitaImage, ServerFacts,
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

	// `kavita_on_deck_removals` and the two favourite tables (the want-to-read
	// shelf) are not part of the shared entity fixture.
	let schema = Schema::new(DbBackend::Sqlite);
	for statement in [
		schema.create_table_from_entity(models::entity::kavita_on_deck_removal::Entity),
		schema.create_table_from_entity(models::entity::favorite_series::Entity),
		schema.create_table_from_entity(models::entity::favorite_media::Entity),
	] {
		conn.execute(conn.get_database_backend().build(&statement))
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

/// A [`KavitaBackend`] answering every persistence need from an in-memory
/// database; the file-backed methods are never reached by the route tests.
pub(crate) struct TestBackend {
	pub conn: sea_orm::DatabaseConnection,
	/// The EPUB structure the `Book` routes see; `None` means "not a book".
	pub book_structure: Option<KavitaBookStructure>,
}

impl TestBackend {
	pub fn new(conn: sea_orm::DatabaseConnection) -> Self {
		Self {
			conn,
			book_structure: None,
		}
	}
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

	async fn media_page(
		&self,
		_user: &AuthUser,
		media_id: &str,
		_page: i32,
	) -> APIResult<KavitaImage> {
		Err(APIError::NotFound(format!(
			"unreachable in tests: {media_id}"
		)))
	}

	async fn media_thumbnail(
		&self,
		_user: &AuthUser,
		media_id: &str,
	) -> APIResult<KavitaImage> {
		Err(APIError::NotFound(format!(
			"unreachable in tests: {media_id}"
		)))
	}

	async fn series_thumbnail(
		&self,
		_user: &AuthUser,
		series_id: &str,
	) -> APIResult<KavitaImage> {
		Err(APIError::NotFound(format!(
			"unreachable in tests: {series_id}"
		)))
	}

	async fn serve_media_file(
		&self,
		_auth: stump_auth::AuthContext,
		_headers: HeaderMap,
		media_id: &str,
	) -> APIResult<Response<Body>> {
		Err(APIError::NotFound(format!(
			"unreachable in tests: {media_id}"
		)))
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
}
