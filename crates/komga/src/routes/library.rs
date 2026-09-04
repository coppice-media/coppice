use std::sync::Arc;

use axum::{extract::Path, http::StatusCode, routing::post, Extension, Router};
use models::{
	entity::{library, media, series, user::AuthUser},
	services::lists,
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, sea_query::Query, QuerySelect, TransactionTrait};
use stump_auth::AuthContext;

use super::KomgaBackend;
use crate::errors::{APIError, APIResult};

/// Komga library-management actions backed by Stump's scanner and analysis jobs.
/// Authentication is applied by the parent Komga router.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route(
			"/api/v1/libraries/{id}/metadata/refresh",
			post(refresh_library_metadata),
		)
		.route("/api/v1/libraries/{id}/analyze", post(analyze_library))
		.route(
			"/api/v1/libraries/{id}/empty-trash",
			post(empty_library_trash),
		)
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

	lists::remove_memberships_for_media(&txn, &trashed_media_ids).await?;
	media::Entity::delete_many()
		.filter(media::Column::Id.is_in(trashed_media_ids))
		.exec(&txn)
		.await?;
	txn.commit().await?;

	Ok(StatusCode::ACCEPTED)
}
