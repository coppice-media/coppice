//! `UsersController` reads.

use std::sync::Arc;

use crate::{
	errors::APIResult,
	ids::{IdKind, KavitaIds},
};
use axum::{extract::Query, routing::get, Extension, Json, Router};
use models::entity::{library, library_exclusion};
use sea_orm::{prelude::*, ColumnTrait};
use serde::Deserialize;
use stump_auth::AuthContext;

use super::{route_ci, KavitaBackend};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryIdQuery {
	#[serde(default)]
	library_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	route_ci(
		Router::<S>::new(),
		"/api/Users/has-library-access",
		get(has_library_access),
	)
}

/// `UsersController.HasLibraryAccess`: whether the library exists and is
/// visible to the user. Stump has no per-user library grants, only hidden
/// libraries (`library_exclusions`), which is what the query checks.
async fn has_library_access(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LibraryIdQuery>,
) -> APIResult<Json<bool>> {
	let user = auth.user();
	let Some(library_id) = query.library_id else {
		return Ok(Json(false));
	};
	let Some(stump_library_id) =
		KavitaIds::lookup(ctx.conn(), IdKind::Library, library_id).await?
	else {
		return Ok(Json(false));
	};
	let visible = library::Entity::find_by_id(stump_library_id)
		.filter(library::Column::Id.not_in_subquery(
			library_exclusion::Entity::library_hidden_to_user_query(&user),
		))
		.one(ctx.conn())
		.await?
		.is_some();
	Ok(Json(visible))
}
