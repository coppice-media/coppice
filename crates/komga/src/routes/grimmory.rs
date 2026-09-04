//! Grimmory's Komga-shaped compatibility profile.
//!
//! Grimmory serves this profile below `/komga`, uses Basic authentication, and
//! lists books with `GET /api/v1/books` instead of Komga's POST list route.
//! The server mounts this router under `/komga`, so the provider's route paths
//! intentionally retain the ordinary Komga `/api/...` shape.

use std::{collections::BTreeSet, sync::Arc};

use axum::{
	body::Body, extract::Query, http::HeaderMap, response::Response, routing::get,
	Extension, Router,
};
use models::entity::{media, user::AuthUser};
use sea_orm::{prelude::*, ColumnTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;

use super::{mapper::map_books, response::cached_json, KomgaBackend};
use crate::{
	errors::{APIError, APIResult},
	KomgaUser, KomgaUserId, Page,
};

const DEFAULT_PAGE_SIZE: i32 = 20;
const MAX_PAGE_SIZE: i32 = 200;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PaginationQuery {
	page: i32,
	size: i32,
}

impl Default for PaginationQuery {
	fn default() -> Self {
		Self {
			page: 0,
			size: DEFAULT_PAGE_SIZE,
		}
	}
}

/// Routes required by Liseur's Grimmory provider.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route("/api/v1/books", get(get_books))
		.route("/api/v2/users/me", get(current_user))
}

async fn current_user(
	Extension(auth): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let user = auth.user();
	let response = KomgaUser {
		id: KomgaUserId::from(user.id),
		email: format!("{}@grimmory.local", user.username),
		roles: BTreeSet::from(["USER".to_owned()]),
		shared_all_libraries: true,
		shared_libraries_ids: BTreeSet::new(),
		labels_allow: BTreeSet::new(),
		labels_exclude: BTreeSet::new(),
		age_restriction: None,
	};
	cached_json(&headers, &response)
}

async fn get_books(
	Extension(backend): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<PaginationQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	if query.page < 0 {
		return Err(APIError::BadRequest(
			"page must be zero-based and non-negative".to_owned(),
		));
	}
	if query.size < 0 {
		return Err(APIError::BadRequest("size must be non-negative".to_owned()));
	}
	let size = query.size.min(MAX_PAGE_SIZE);
	let user: AuthUser = auth.user();
	let books = media::ModelWithMetadata::find_for_user(&user)
		.filter(media::Column::DeletedAt.is_null())
		.order_by_asc(media::Column::Name)
		.order_by_asc(media::Column::Id);
	let total = books.clone().count(backend.conn()).await?;
	let books = books
		.offset((query.page as u64).saturating_mul(size as u64))
		.limit(size as u64)
		.into_model::<media::ModelWithMetadata>()
		.all(backend.conn())
		.await?;
	let books = map_books(backend.conn(), &user, books).await?;
	cached_json(
		&headers,
		&Page::new(
			books,
			query.page,
			size,
			i32::try_from(total)
				.map_err(|error| APIError::InternalServerError(error.to_string()))?,
			false,
		),
	)
}
