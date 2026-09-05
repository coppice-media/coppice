//! Server-side implementation of the Kavita backend contract.

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	response::IntoResponse,
};
use models::entity::{library_config, media, series, user::AuthUser, user};
use prefixed_api_key::PrefixedApiKey;
use sea_orm::{prelude::*, DatabaseConnection, QueryOrder};
use stump_auth::AuthContext;
use stump_kavita::{
	errors::{APIError as KavitaError, APIResult as KavitaResult},
	routes::{KavitaBackend, KavitaImage, ServerFacts},
	KavitaClaims,
};
use stump_media::media::get_page_async;

use crate::{
	config::{jwt::access_token_secret, state::AppState},
	errors::APIError,
	middleware::auth::validate_api_key,
	routers::api::v2::{media as api_media, series as api_series},
	utils::{serve_media, verify_password},
};

pub(crate) struct KavitaBackendAdapter {
	ctx: AppState,
}

impl KavitaBackendAdapter {
	pub(crate) fn new(ctx: AppState) -> Self {
		Self { ctx }
	}
}

fn map_server_error(error: APIError) -> KavitaError {
	match error {
		APIError::NotFound(message) => KavitaError::NotFound(message),
		APIError::BadRequest(message) => KavitaError::BadRequest(message),
		APIError::Unauthorized => KavitaError::Unauthorized,
		APIError::Forbidden(message) => KavitaError::Forbidden(message),
		other => KavitaError::InternalServerError(other.to_string()),
	}
}

/// Resolve the Stump user behind a Kavita token's `nameid`.
pub(crate) async fn user_by_kavita_id(
	conn: &DatabaseConnection,
	claims: &KavitaClaims,
) -> KavitaResult<AuthUser> {
	let kavita_id = claims.user_id().ok_or(KavitaError::Unauthorized)?;
	let stump_id = stump_kavita::KavitaIds::lookup(conn, stump_kavita::IdKind::User, kavita_id)
		.await?
		.ok_or(KavitaError::Unauthorized)?;
	let user = user::LoginUser::find()
		.filter(user::Column::Id.eq(stump_id))
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<user::LoginUser>()
		.one(conn)
		.await?
		.ok_or(KavitaError::Unauthorized)?;
	if user.is_locked {
		return Err(KavitaError::Forbidden(
			"Your account has been locked by an administrator".to_owned(),
		));
	}
	Ok(AuthUser::from(user))
}

#[async_trait]
impl KavitaBackend for KavitaBackendAdapter {
	fn conn(&self) -> &DatabaseConnection {
		self.ctx.conn.as_ref()
	}

	async fn token_secret(&self) -> KavitaResult<Vec<u8>> {
		access_token_secret(self.conn())
			.await
			.map(String::into_bytes)
			.map_err(map_server_error)
	}

	async fn authenticate_api_key(&self, api_key: &str) -> KavitaResult<AuthUser> {
		let pak = PrefixedApiKey::from_string(api_key).map_err(|_| KavitaError::Unauthorized)?;
		validate_api_key(pak, self.conn())
			.await
			.map_err(map_server_error)
	}

	async fn authenticate_password(
		&self,
		username: &str,
		password: &str,
	) -> KavitaResult<AuthUser> {
		let user = user::LoginUser::find()
			.filter(user::Column::Username.eq(username))
			.filter(user::Column::DeletedAt.is_null())
			.into_model::<user::LoginUser>()
			.one(self.conn())
			.await?
			.ok_or(KavitaError::Unauthorized)?;
		let verified = verify_password(&user.hashed_password, password)
			.map_err(|error| KavitaError::InternalServerError(error.to_string()))?;
		if !verified {
			return Err(KavitaError::Unauthorized);
		}
		if user.is_locked {
			return Err(KavitaError::Forbidden(
				"Your account has been locked by an administrator".to_owned(),
			));
		}
		Ok(AuthUser::from(user))
	}

	fn server_facts(&self) -> ServerFacts {
		ServerFacts {
			is_docker: std::path::Path::new("/.dockerenv").exists(),
			first_install_date: None,
		}
	}

	async fn media_page(
		&self,
		user: &AuthUser,
		media_id: &str,
		page: i32,
	) -> KavitaResult<KavitaImage> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| KavitaError::NotFound("Chapter does not exist".to_owned()))?;
		let (content_type, data) = get_page_async(&book.path, page, &self.ctx.config.media)
			.await
			.map_err(|error| map_server_error(APIError::from(error)))?;
		Ok(KavitaImage::new(content_type.to_string(), data))
	}

	async fn media_thumbnail(&self, user: &AuthUser, media_id: &str) -> KavitaResult<KavitaImage> {
		let image = api_media::get_media_thumbnail_by_id(
			self.ctx.as_ref(),
			user,
			media_id.to_owned(),
		)
		.await
		.map_err(map_server_error)?;
		Ok(KavitaImage::new(image.content_type.to_string(), image.data))
	}

	async fn series_thumbnail(&self, user: &AuthUser, series_id: &str) -> KavitaResult<KavitaImage> {
		let series = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id.to_owned()))
			.into_model::<series::SeriesThumbSelect>()
			.one(self.conn())
			.await?
			.ok_or_else(|| KavitaError::NotFound("Series does not exist".to_owned()))?;
		let first_book = media::Entity::find_for_user(user)
			.filter(media::Column::SeriesId.eq(series.id.clone()))
			.order_by_asc(media::Column::Name)
			.into_model::<media::MediaThumbSelect>()
			.one(self.conn())
			.await?;
		let config = library_config::Entity::find()
			.filter(library_config::Column::LibraryId.eq(series.library_id.clone()))
			.one(self.conn())
			.await?;
		let format = config.and_then(|config| config.thumbnail_config.map(|config| config.format));
		let (content_type, data) =
			api_series::get_series_thumbnail(&series, first_book, format, self.ctx.config.as_ref())
				.await
				.map_err(map_server_error)?;
		Ok(KavitaImage::new(content_type.to_string(), data))
	}

	async fn serve_media_file(
		&self,
		auth: AuthContext,
		headers: HeaderMap,
		media_id: &str,
	) -> KavitaResult<Response<Body>> {
		serve_media::serve_media_file(auth, headers, self.conn(), media_id.to_owned())
			.await
			.map(IntoResponse::into_response)
			.map_err(map_server_error)
	}
}
