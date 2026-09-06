//! Server-side implementation of the Kavita backend contract.

use std::path::PathBuf;

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	response::IntoResponse,
};
use models::entity::{
	collection, library_config, media, reading_list, series, user, user::AuthUser,
};
use prefixed_api_key::PrefixedApiKey;
use sea_orm::{prelude::*, DatabaseConnection, QueryOrder};
use stump_auth::AuthContext;
use stump_core::{
	filesystem::media::analysis::{AnalysisJobConfig, MediaAnalysisJobScope},
	job::stump_job::StumpJob,
};
use stump_kavita::{
	errors::{APIError as KavitaError, APIResult as KavitaResult},
	routes::{
		KavitaBackend, KavitaBookResource, KavitaBookStructure, KavitaImage,
		KavitaNavPoint, KavitaSpineItem, ServerFacts,
	},
	KavitaClaims,
};
use stump_media::{media::get_page_async, EpubNavEntry, EpubProcessor};

use crate::{
	config::{jwt::access_token_secret, state::AppState},
	errors::APIError,
	middleware::auth::validate_api_key,
	routers::api::v2::{media as api_media, series as api_series},
	utils::{serve_media, verify_password},
};

/// A Kavita `force`/`forceUpdate` flag as Stump scan options: a forced scan
/// rebuilds every book, an ordinary one only the changed ones. `None` keeps
/// the scanner's default (`BuildChanged`).
fn scan_options(force: bool) -> Option<stump_scanner::ScanOptions> {
	force.then_some(stump_scanner::ScanOptions {
		config: stump_scanner::ScanConfig::ForceRebuild {
			force_rebuild: true,
		},
	})
}

pub(crate) struct KavitaBackendAdapter {
	ctx: AppState,
}

impl KavitaBackendAdapter {
	pub(crate) fn new(ctx: AppState) -> Self {
		Self { ctx }
	}

	/// The on-disk path of a media item the user may see.
	async fn media_path(&self, user: &AuthUser, media_id: &str) -> KavitaResult<String> {
		media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id.to_owned()))
			.one(self.conn())
			.await?
			.map(|book| book.path)
			.ok_or_else(|| KavitaError::NotFound("Chapter does not exist".to_owned()))
	}

	/// The CBZ a provider-backed chapter downloads as: the pages come through
	/// the provider host's cache and are packed into a stored zip, the same
	/// archive every protocol serves for a virtual row. The chapter's own
	/// name is the archive's file stem.
	#[cfg(feature = "providers")]
	async fn provider_archive(&self, book: &media::Model) -> KavitaResult<Vec<u8>> {
		let host = crate::routers::provider_virtual::provider_host(&self.ctx)
			.ok_or_else(|| {
				KavitaError::NotFound("Provider libraries are disabled".to_owned())
			})?;
		let path = stump_provider::virtual_path::VirtualPath::parse(&book.path)
			.filter(|path| path.remote_chapter_id.is_some())
			.ok_or_else(|| {
				KavitaError::InternalServerError(format!(
					"{} is not a provider chapter path",
					book.path
				))
			})?;
		host.build_archive(
			&path.source_id,
			path.remote_chapter_id.as_deref().unwrap_or_default(),
			&book.name,
		)
		.await
		.map(|archive| archive.bytes)
		.map_err(|error| KavitaError::InternalServerError(error.to_string()))
	}

	/// Provider libraries are compiled out: a `provider://` row cannot exist,
	/// and a row that somehow carries the scheme has no file to serve.
	#[cfg(not(feature = "providers"))]
	async fn provider_archive(&self, book: &media::Model) -> KavitaResult<Vec<u8>> {
		Err(KavitaError::NotFound(format!(
			"{} has no file to download",
			book.path
		)))
	}
}

/// A canonical container-service failure, mapped onto its Kavita status.
fn map_core_error(error: stump_core::CoreError) -> KavitaError {
	match error {
		stump_core::CoreError::NotFound(message) => KavitaError::NotFound(message),
		stump_core::CoreError::BadRequest(message) => KavitaError::BadRequest(message),
		stump_core::CoreError::Forbidden(message) => KavitaError::Forbidden(message),
		other => KavitaError::InternalServerError(other.to_string()),
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
	let stump_id =
		stump_kavita::KavitaIds::lookup(conn, stump_kavita::IdKind::User, kavita_id)
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
		let pak = PrefixedApiKey::from_string(api_key)
			.map_err(|_| KavitaError::Unauthorized)?;
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
		let (content_type, data) =
			get_page_async(&book.path, page, &self.ctx.config.media)
				.await
				.map_err(|error| map_server_error(APIError::from(error)))?;
		Ok(KavitaImage::new(content_type.to_string(), data))
	}

	async fn media_thumbnail(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> KavitaResult<KavitaImage> {
		let image = api_media::get_media_thumbnail_by_id(
			self.ctx.as_ref(),
			user,
			media_id.to_owned(),
		)
		.await
		.map_err(map_server_error)?;
		Ok(KavitaImage::new(image.content_type.to_string(), image.data))
	}

	async fn series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: &str,
	) -> KavitaResult<KavitaImage> {
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
		let format =
			config.and_then(|config| config.thumbnail_config.map(|config| config.format));
		let (content_type, data) = api_series::get_series_thumbnail(
			&series,
			first_book,
			format,
			self.ctx.config.as_ref(),
		)
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

	async fn media_bytes(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> KavitaResult<Vec<u8>> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id.to_owned()))
			.one(self.conn())
			.await?
			.ok_or_else(|| KavitaError::NotFound("Chapter does not exist".to_owned()))?;
		if stump_media::virtual_media::is_virtual_path(&book.path) {
			return self.provider_archive(&book).await;
		}
		tokio::fs::read(&book.path).await.map_err(|error| {
			KavitaError::InternalServerError(format!(
				"Failed to read {}: {error}",
				book.path
			))
		})
	}
	async fn book_structure(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> KavitaResult<KavitaBookStructure> {
		let path = self.media_path(user, media_id).await?;
		tokio::task::spawn_blocking(move || book_structure(&path))
			.await
			.map_err(|error| KavitaError::InternalServerError(error.to_string()))?
	}

	async fn book_page(
		&self,
		user: &AuthUser,
		media_id: &str,
		spine_index: usize,
	) -> KavitaResult<KavitaBookResource> {
		let path = self.media_path(user, media_id).await?;
		let (content_type, data) = tokio::task::spawn_blocking(move || {
			EpubProcessor::get_chapter(&path, spine_index)
		})
		.await
		.map_err(|error| KavitaError::InternalServerError(error.to_string()))?
		.map_err(|error| map_server_error(APIError::from(error)))?;
		Ok(KavitaBookResource {
			content_type: content_type.to_string(),
			data,
		})
	}

	async fn book_resource(
		&self,
		user: &AuthUser,
		media_id: &str,
		file: &str,
	) -> KavitaResult<KavitaBookResource> {
		let path = self.media_path(user, media_id).await?;
		let file = PathBuf::from(file);
		let (content_type, data) = tokio::task::spawn_blocking(move || {
			EpubProcessor::get_resource_by_path(&path, "", file)
		})
		.await
		.map_err(|error| KavitaError::InternalServerError(error.to_string()))?
		.map_err(|error| map_server_error(APIError::from(error)))?;
		Ok(KavitaBookResource {
			content_type: content_type.to_string(),
			data,
		})
	}

	async fn create_read_list(
		&self,
		user: &AuthUser,
		name: String,
	) -> KavitaResult<reading_list::Model> {
		stump_collections::create_read_list(
			&self.ctx,
			user,
			stump_collections::ReadListCreate {
				name,
				summary: None,
				ordered: true,
				book_ids: Vec::new(),
			},
		)
		.await
		.map_err(map_core_error)
	}

	async fn set_read_list_items(
		&self,
		user: &AuthUser,
		id: &str,
		book_ids: Vec<String>,
	) -> KavitaResult<()> {
		stump_collections::update_read_list(
			&self.ctx,
			user,
			id,
			stump_collections::ReadListUpdate {
				name: None,
				summary: None,
				ordered: None,
				book_ids: Some(book_ids),
			},
		)
		.await
		.map(|_| ())
		.map_err(map_core_error)
	}

	async fn delete_read_list(&self, user: &AuthUser, id: &str) -> KavitaResult<()> {
		stump_collections::delete_read_list(&self.ctx, user, id)
			.await
			.map_err(map_core_error)
	}

	async fn create_collection(
		&self,
		user: &AuthUser,
		name: String,
		series_ids: Vec<String>,
	) -> KavitaResult<collection::Model> {
		stump_collections::create_collection(
			&self.ctx,
			user,
			stump_collections::CollectionCreate {
				name,
				ordered: false,
				series_ids,
			},
		)
		.await
		.map_err(map_core_error)
	}

	async fn set_collection_series(
		&self,
		user: &AuthUser,
		id: &str,
		series_ids: Vec<String>,
	) -> KavitaResult<()> {
		stump_collections::update_collection(
			&self.ctx,
			user,
			id,
			stump_collections::CollectionUpdate {
				name: None,
				ordered: None,
				series_ids: Some(series_ids),
			},
		)
		.await
		.map(|_| ())
		.map_err(map_core_error)
	}

	async fn enqueue_library_scan(
		&self,
		library_id: String,
		path: String,
		force: bool,
	) -> KavitaResult<()> {
		self.ctx
			.enqueue(StumpJob::library_scan(
				library_id,
				path,
				scan_options(force),
			))
			.await
			.map_err(map_core_error)
	}

	async fn enqueue_series_scan(
		&self,
		series_id: String,
		path: String,
		force: bool,
	) -> KavitaResult<()> {
		self.ctx
			.enqueue(StumpJob::series_scan(series_id, path, scan_options(force)))
			.await
			.map_err(map_core_error)
	}

	async fn enqueue_series_analysis(&self, series_id: String) -> KavitaResult<()> {
		self.ctx
			.enqueue(StumpJob::analyze_media(AnalysisJobConfig {
				force_reanalysis: true,
				scope: MediaAnalysisJobScope::Series(series_id),
			}))
			.await
			.map_err(map_core_error)
	}
}

/// The EPUB spine page budget and navigation tree behind `BookController`.
/// Runs on a blocking thread: it opens and reads the archive.
fn book_structure(path: &str) -> KavitaResult<KavitaBookStructure> {
	let structure = EpubProcessor::structure(path)
		.map_err(|error| map_server_error(APIError::from(error)))?;
	Ok(KavitaBookStructure {
		spine: structure
			.spine_pages
			.into_iter()
			.map(|pages| KavitaSpineItem { pages })
			.collect(),
		navigation: map_nav_entries(&structure.navigation),
	})
}

fn map_nav_entries(entries: &[EpubNavEntry]) -> Vec<KavitaNavPoint> {
	entries
		.iter()
		.map(|entry| KavitaNavPoint {
			title: entry.title.clone(),
			fragment: entry.fragment.clone(),
			spine_index: entry.spine_index,
			children: map_nav_entries(&entry.children),
		})
		.collect()
}
