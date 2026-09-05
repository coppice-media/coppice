//! `LibraryController` reads: `GET /api/Library/libraries`, `GET /api/Library`
//! and the `/api/Library/{id}` form the Kavita Tachiyomi extension requests.

use std::sync::Arc;

use axum::{
	extract::{Path, Query},
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::get,
	Extension, Json, Router,
};
use models::entity::{library, library_config};
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

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Library/libraries", get(get_libraries));
	let router = route_ci(router, "/api/Library", get(get_library_by_query));
	route_ci(router, "/api/Library/{libraryId}", get(get_library_by_path))
}

async fn get_libraries(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Json<Vec<LibraryDto>>> {
	let user = auth.user();
	let rows = library::Entity::find_for_user(&user)
		.order_by_asc(library::Column::Name)
		.find_also_related(library_config::Entity)
		.all(ctx.conn())
		.await?;
	let ids = rows
		.iter()
		.map(|(library, _)| library.id.clone())
		.collect::<Vec<_>>();
	let kavita_ids = KavitaIds::resolve_many(ctx.conn(), IdKind::Library, &ids).await?;
	Ok(Json(
		rows.iter()
			.map(|(library, config)| {
				map_library(kavita_ids[&library.id], library, config.as_ref())
			})
			.collect(),
	))
}

async fn get_library_by_query(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LibraryQuery>,
) -> APIResult<Response> {
	get_library(ctx.as_ref(), &auth, query.library_id.unwrap_or_default()).await
}

async fn get_library_by_path(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Path(library_id): Path<i32>,
) -> APIResult<Response> {
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
