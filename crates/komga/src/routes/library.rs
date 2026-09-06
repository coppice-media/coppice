use std::{path::PathBuf, sync::Arc};

use axum::{
	body::Body,
	extract::Path,
	http::{HeaderMap, Response, StatusCode},
	routing::{delete as delete_route, patch, post},
	Extension, Json, Router,
};
use models::{
	entity::{library, library_config, media, series, user::AuthUser},
	services::library::path_within_roots,
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, sea_query::Query, QuerySelect, TransactionTrait};
use stump_auth::AuthContext;
use tokio::fs;

use super::{response::cached_json, KomgaBackend};
use crate::{
	errors::{APIError, APIResult},
	DirectoryListing, DirectoryRequest, KomgaLibrary, KomgaLibraryCreateRequest,
	KomgaLibraryUpdateRequest, Path as KomgaPath,
};

/// Komga library management: `POST`/`PATCH`/`DELETE /api/v1/libraries`, the
/// library task actions, and the owner-only filesystem browser. Persistence is
/// routed through the shared `stump_core::library` service (one path with the
/// GraphQL mutations); authentication is applied by the parent Komga router.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route("/api/v1/libraries", post(create_library))
		.route(
			"/api/v1/libraries/{id}",
			patch(update_library).delete(delete_route(delete_library)),
		)
		.route(
			"/api/v1/libraries/{id}/metadata/refresh",
			post(refresh_library_metadata),
		)
		.route("/api/v1/libraries/{id}/analyze", post(analyze_library))
		.route(
			"/api/v1/libraries/{id}/empty-trash",
			post(empty_library_trash),
		)
		.route("/api/v1/filesystem", post(get_filesystem_listing))
}

async fn find_library(
	conn: &DatabaseConnection,
	user: &AuthUser,
	id: &str,
) -> APIResult<library::Model> {
	library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(id.to_owned()))
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Library not found".to_owned()))
}

fn enforce_manage_library(auth: &AuthContext) -> APIResult<()> {
	auth.enforce_permissions(&[UserPermission::ManageLibrary])
		.map_err(|_| APIError::forbidden_discreet())
}

/// Creates a library from a Komga `LibraryCreationDto` (upstream: `200` with
/// the created `LibraryDto`). Blank required fields mirror upstream's bean
/// validation; path, duplicate, and configured-root validation is shared with
/// the GraphQL mutation via the core library service.
async fn create_library(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<KomgaLibraryCreateRequest>,
) -> APIResult<Json<KomgaLibrary>> {
	enforce_manage_library(&auth)?;

	if request.name.trim().is_empty() {
		return Err(APIError::BadRequest("name must not be blank".to_owned()));
	}
	if request.root.trim().is_empty() {
		return Err(APIError::BadRequest("root must not be blank".to_owned()));
	}

	let created = ctx.create_library(request).await?;

	// Re-fetch with the config row so the response matches the `GET`
	// rendering of the same library.
	let user = auth.user();
	let (library, config) = library::Entity::find_for_user(&user)
		.filter(library::Column::Id.eq(created.id.clone()))
		.find_also_related(library_config::Entity)
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::NotFound("Library not found".to_owned()))?;

	Ok(Json(super::mapper::map_library(library, config, &user)))
}

/// Applies a Komga `LibraryUpdateDto` patch (upstream: `204 No Content`).
/// Omitted fields keep their stored values; the adapter merges the patch onto
/// the existing rows before the shared update service runs.
async fn update_library(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(request): Json<KomgaLibraryUpdateRequest>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;

	// Upstream validates `NullOrNotBlank` on both patchable identity fields.
	if request
		.name
		.as_deref()
		.is_some_and(|name| name.trim().is_empty())
	{
		return Err(APIError::BadRequest("name must not be blank".to_owned()));
	}
	if request
		.root
		.as_deref()
		.is_some_and(|root| root.trim().is_empty())
	{
		return Err(APIError::BadRequest("root must not be blank".to_owned()));
	}

	ctx.update_library(&auth.user(), &id, request).await?;
	Ok(StatusCode::NO_CONTENT)
}

/// Deletes a library and its contents (upstream: `204 No Content`).
async fn delete_library(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;

	ctx.delete_library(&auth.user(), &id).await?;
	Ok(StatusCode::NO_CONTENT)
}

/// Komga metadata refresh is Stump's library scan operation: scanning rereads
/// filesystem metadata and updates the persisted library records.
async fn refresh_library_metadata(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;
	let user = auth.user();
	let library = find_library(ctx.conn(), &user, &id).await?;
	ctx.enqueue_library_scan(library.id, library.path, false)
		.await?;
	Ok(StatusCode::ACCEPTED)
}

async fn analyze_library(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;
	let user = auth.user();
	let library = find_library(ctx.conn(), &user, &id).await?;
	ctx.enqueue_library_analysis(library.id).await?;
	Ok(StatusCode::ACCEPTED)
}

/// Permanently removes only soft-deleted media rows belonging to this library.
/// The operation intentionally leaves active media and series rows untouched.
async fn empty_library_trash(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;
	let user = auth.user();
	let library = find_library(ctx.conn(), &user, &id).await?;

	let txn = ctx.conn().begin().await?;
	let library_series = Query::select()
		.column(series::Column::Id)
		.from(series::Entity)
		.and_where(series::Column::LibraryId.eq(library.id))
		.to_owned();
	let trashed_media_ids = media::Entity::find()
		.select_only()
		.column(media::Column::Id)
		.filter(media::Column::DeletedAt.is_not_null())
		.filter(media::Column::SeriesId.in_subquery(library_series))
		.into_tuple::<String>()
		.all(&txn)
		.await?;

	models::services::lists::remove_memberships_for_media(&txn, &trashed_media_ids)
		.await?;
	media::Entity::delete_many()
		.filter(media::Column::Id.is_in(trashed_media_ids))
		.exec(&txn)
		.await?;
	txn.commit().await?;

	Ok(StatusCode::ACCEPTED)
}

/// Komga's filesystem browser for picking a library root (upstream:
/// `FileSystemController`, ADMIN-only).
///
/// Divergences from upstream, all deliberate: when `STUMP_LIBRARY_ROOTS` is
/// configured the browser and its root listing are constrained to those roots;
/// `provider://` virtual-library roots are never accepted or listed; and `..`
/// is rejected outright instead of being resolved by canonicalization.
async fn get_filesystem_listing(
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
	body: Option<Json<DirectoryRequest>>,
) -> APIResult<Response<Body>> {
	auth.enforce_server_owner()
		.map_err(|_| APIError::forbidden_discreet())?;

	// Upstream treats an absent body as `DirectoryRequestDto()` and answers
	// with the root directories. The `Option` extractor also folds malformed
	// JSON bodies into this roots listing instead of upstream's `400`.
	let request = body.map(|Json(request)| request).unwrap_or_default();
	let roots = ctx.library_roots();

	if request.path.is_empty() {
		let directories = if roots.is_empty() {
			vec![KomgaPath {
				r#type: "directory".to_string(),
				name: "/".to_string(),
				path: "/".to_string(),
			}]
		} else {
			roots
				.iter()
				.map(|root| KomgaPath {
					r#type: "directory".to_string(),
					name: root_name(root),
					path: root.clone(),
				})
				.collect()
		};
		return cached_json(
			&headers,
			&DirectoryListing {
				parent: None,
				directories,
				files: Vec::new(),
			},
		);
	}

	if request.path.starts_with("provider://") {
		return Err(APIError::BadRequest(
			"Virtual library roots cannot be browsed".to_string(),
		));
	}

	let requested = PathBuf::from(&request.path);
	if requested
		.components()
		.any(|component| component == std::path::Component::ParentDir)
	{
		return Err(APIError::BadRequest(
			"Path must not contain '..'".to_string(),
		));
	}
	if !requested.is_absolute() {
		return Err(APIError::BadRequest("Path must be absolute".to_string()));
	}

	let canonical = fs::canonicalize(&requested)
		.await
		.map_err(|_| APIError::BadRequest("Path does not exist".to_string()))?;
	let canonical_metadata = fs::metadata(&canonical)
		.await
		.map_err(|_| APIError::BadRequest("Path does not exist".to_string()))?;
	// Komga falls back to the parent directory when the picker is pointed at
	// a file, so a previously selected file path still yields a listing.
	let canonical = if canonical_metadata.is_dir() {
		canonical
	} else {
		canonical
			.parent()
			.map(|parent| parent.to_path_buf())
			.ok_or_else(|| {
				APIError::BadRequest("Path must refer to a directory".to_string())
			})?
	};

	let canonical_text = canonical.to_string_lossy();
	if !path_within_roots(&canonical_text, &roots) {
		return Err(APIError::BadRequest(
			"Path is outside of the configured library roots".to_string(),
		));
	}

	let mut directories = Vec::new();
	let mut files = Vec::new();
	let mut directory = fs::read_dir(&canonical)
		.await
		.map_err(|_| APIError::BadRequest("Path could not be listed".to_string()))?;
	while let Some(entry) = directory
		.next_entry()
		.await
		.map_err(|_| APIError::BadRequest("Path could not be listed".to_string()))?
	{
		let name = entry.file_name().to_string_lossy().into_owned();
		if name.starts_with('.') {
			continue;
		}

		let file_type = match entry.file_type().await {
			Ok(file_type) => file_type,
			Err(_) => continue,
		};

		if file_type.is_file() {
			// Komga only includes plain files when the picker asks for them.
			if request.show_files {
				files.push(KomgaPath {
					r#type: "file".to_string(),
					name,
					path: entry.path().to_string_lossy().into_owned(),
				});
			}
			continue;
		}

		if !file_type.is_dir() && !file_type.is_symlink() {
			continue;
		}

		let child = match fs::canonicalize(entry.path()).await {
			Ok(child) => child,
			Err(_) => continue,
		};
		if child == canonical || !child.starts_with(&canonical) {
			continue;
		}
		match fs::metadata(&child).await {
			Ok(metadata) if metadata.is_dir() => {},
			_ => continue,
		}

		directories.push(KomgaPath {
			r#type: "directory".to_string(),
			name,
			path: child.to_string_lossy().into_owned(),
		});
	}

	let name_sort = |left: &KomgaPath, right: &KomgaPath| {
		left.name
			.to_ascii_lowercase()
			.cmp(&right.name.to_ascii_lowercase())
			.then_with(|| left.name.cmp(&right.name))
			.then_with(|| left.path.cmp(&right.path))
	};
	directories.sort_by(name_sort);
	files.sort_by(name_sort);

	let parent = canonical
		.parent()
		.unwrap_or(canonical.as_path())
		.to_string_lossy()
		.into_owned();
	cached_json(
		&headers,
		&DirectoryListing {
			parent: Some(parent),
			directories,
			files,
		},
	)
}

/// The picker-facing label of a configured root (last path component).
fn root_name(root: &str) -> String {
	std::path::Path::new(root)
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_else(|| root.to_owned())
}
