//! `LibraryController`: the library reads the Kavita Tachiyomi extension
//! requests (`GET /api/Library/libraries`, `GET /api/Library`,
//! `GET /api/Library/{id}`) and the scan Kamigura triggers
//! (`POST /api/Library/scan`).

use std::sync::Arc;

use axum::{
	extract::{Path, Query},
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::{get, post},
	Extension, Json, Router,
};
use models::entity::{library, library_config, user::AuthUser};
use sea_orm::{prelude::*, QueryOrder};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::LibraryDto,
	errors::APIResult,
	ids::{IdKind, KavitaIds},
	mapper::map_library,
};

use super::{route_ci, KavitaBackend};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryQuery {
	#[serde(default)]
	library_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScanQuery {
	#[serde(default)]
	library_id: Option<i32>,
	#[serde(default)]
	force: Option<bool>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Library/libraries", get(get_libraries));
	let router = route_ci(router, "/api/Library/scan", post(scan_library));
	let router = route_ci(router, "/api/Library", get(get_library_by_query));
	route_ci(router, "/api/Library/{libraryId}", get(get_library_by_path))
}

async fn get_libraries(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<Vec<LibraryDto>>> {
	let user = auth.user();
	let rows = visible_libraries(ctx.as_ref(), &user).await?;
	Ok(Json(map_libraries(ctx.as_ref(), &rows).await?))
}

/// `LibraryDto`s for loaded libraries, with their Kavita ids resolved once.
pub(crate) async fn map_libraries(
	ctx: &dyn KavitaBackend,
	rows: &[(library::Model, Option<library_config::Model>)],
) -> APIResult<Vec<LibraryDto>> {
	let ids = rows
		.iter()
		.map(|(library, _)| library.id.clone())
		.collect::<Vec<_>>();
	let kavita_ids = KavitaIds::resolve_many(ctx.conn(), IdKind::Library, &ids).await?;
	Ok(rows
		.iter()
		.map(|(library, config)| {
			map_library(kavita_ids[&library.id], library, config.as_ref())
		})
		.collect())
}

/// The libraries a user may see, with their configs.
pub(crate) async fn visible_libraries(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
) -> APIResult<Vec<(library::Model, Option<library_config::Model>)>> {
	Ok(library::Entity::find_for_user(user)
		.order_by_asc(library::Column::Name)
		.find_also_related(library_config::Entity)
		.all(ctx.conn())
		.await?)
}

/// `LibraryController.Scan`: rescan a library. Stump's library-scan job is
/// the same job Komga's `/api/v1/libraries/{id}/scan` enqueues; `force`
/// rebuilds every book instead of only the changed ones.
///
/// Kavita answers `200` for a library that does not exist, so an unknown or
/// inaccessible `libraryId` is a no-op here too.
async fn scan_library(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ScanQuery>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let Some(stump_id) = KavitaIds::lookup(
		ctx.conn(),
		IdKind::Library,
		query.library_id.unwrap_or_default(),
	)
	.await?
	else {
		return Ok(StatusCode::OK);
	};
	let Some(library) = library::Entity::find_for_user(&user)
		.filter(library::Column::Id.eq(stump_id))
		.one(ctx.conn())
		.await?
	else {
		return Ok(StatusCode::OK);
	};
	ctx.enqueue_library_scan(library.id, library.path, query.force.unwrap_or(false))
		.await?;
	Ok(StatusCode::OK)
}

async fn get_library_by_query(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LibraryQuery>,
) -> APIResult<Response> {
	get_library(ctx.as_ref(), &auth, query.library_id.unwrap_or_default()).await
}

/// `/api/Library/{id}` also swallows every unimplemented `LibraryController`
/// sub-path (`scan-folder`, `scan-all`, `refresh-metadata`, …), because
/// `matchit` cannot host a path parameter and a catch-all at the same
/// position. A segment that is not a library id is therefore reported as a
/// missing route (`404`), the way `kavita-ref` reports an unbound one, rather
/// than as a malformed path parameter (`400`).
async fn get_library_by_path(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(library_id): Path<String>,
) -> APIResult<Response> {
	let Ok(library_id) = library_id.parse::<i32>() else {
		tracing::warn!(%library_id, "Unmatched Kavita LibraryController route");
		return Ok(StatusCode::NOT_FOUND.into_response());
	};
	get_library(ctx.as_ref(), &auth, library_id).await
}

/// Kavita answers an unknown or inaccessible library with `204 No Content`
/// (`ActionResult<LibraryDto>` returning null).
async fn get_library(
	ctx: &dyn KavitaBackend,
	auth: &AuthContext,
	library_id: i32,
) -> APIResult<Response> {
	let user = auth.user();
	let Some(stump_id) =
		KavitaIds::lookup(ctx.conn(), IdKind::Library, library_id).await?
	else {
		return Ok(StatusCode::NO_CONTENT.into_response());
	};
	let Some((library, config)) = library::Entity::find_for_user(&user)
		.filter(library::Column::Id.eq(stump_id))
		.find_also_related(library_config::Entity)
		.one(ctx.conn())
		.await?
	else {
		return Ok(StatusCode::NO_CONTENT.into_response());
	};
	Ok(Json(map_library(library_id, &library, config.as_ref())).into_response())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::{
		auth_user, db, library_of_type, request, EnqueuedJob, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	/// `POST /api/Library/scan?libraryId&force` (Kamigura `KavitaApi.kt:24`)
	/// enqueues the same library-scan job Komga's
	/// `/api/v1/libraries/{id}/scan` uses.
	#[tokio::test]
	async fn library_scan_enqueues_the_stump_library_scan_job() {
		let conn = db().await;
		let user_row = fake_data::User::new("scanner").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let library_id = KavitaIds::resolve(backend.conn(), IdKind::Library, &library.id)
			.await
			.unwrap();

		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			&format!("/api/Library/scan?libraryId={library_id}&force=false"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			&format!("/api/library/scan?libraryId={library_id}&force=true"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			backend.enqueued(),
			vec![
				EnqueuedJob::LibraryScan {
					library_id: library.id.clone(),
					path: library.path.clone(),
					force: false,
				},
				EnqueuedJob::LibraryScan {
					library_id: library.id.clone(),
					path: library.path.clone(),
					force: true,
				},
			]
		);

		// An unknown library is `200` with nothing enqueued, like Kavita.
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Library/scan?libraryId=999999",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(backend.enqueued().len(), 2);
	}
}
