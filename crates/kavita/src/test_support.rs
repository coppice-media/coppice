//! Shared in-memory test fixtures for the route tests: an in-memory SQLite
//! database with the Kavita id table materialized and a minimal
//! [`KavitaBackend`] stub over it.

use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use models::shared::image::ImageRef;
use models::entity::user::AuthUser;
use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DbBackend, Schema, Statement};

use crate::errors::{APIError, APIResult};
use crate::ids::CREATE_KAVITA_IDS_SQL;
use crate::routes::{KavitaBackend, KavitaImage, ServerFacts};

/// An in-memory database with every entity table plus `kavita_ids`.
pub(crate) async fn db() -> sea_orm::DatabaseConnection {
	let conn = ::tests::db::test_database().await;
	conn.execute(Statement::from_string(
		DatabaseBackend::Sqlite,
		CREATE_KAVITA_IDS_SQL,
	))
	.await
	.unwrap();

	let removals = Schema::new(DbBackend::Sqlite)
		.create_table_from_entity(models::entity::kavita_on_deck_removal::Entity);
	conn.execute(conn.get_database_backend().build(&removals))
		.await
		.unwrap();
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

/// A [`KavitaBackend`] answering every persistence need from an in-memory
/// database; the file-backed methods are never reached by the route tests.
pub(crate) struct TestBackend {
	pub conn: sea_orm::DatabaseConnection,
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
}
