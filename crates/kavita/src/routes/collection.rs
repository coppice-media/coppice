//! `CollectionController`: Stump collections as Kavita `AppUserCollection`s.
//!
//! Kavita's collections are per-user (`AppUserCollection`) and its clients
//! either list them all or restrict to the ones they own; Stump collections
//! carry a `creating_user_id` and no visibility flag, so `ownedOnly=true` is
//! "created by this user" and `ownedOnly=false` is every collection — the
//! projection of Kavita's "mine plus promoted" onto a server where nothing is
//! promoted. Writes go through [`KavitaBackend::create_collection`] and
//! [`KavitaBackend::set_collection_series`] so they land in the same
//! canonical container service the native, Komga and Kobo surfaces use.
//!
//! A Kavita series in a Book/LightNovel library is one file, but a Stump
//! collection can only hold series, so adding such a series adds the Stump
//! series the file is filed under — the same compromise the Kobo shelf
//! write-back makes.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
	extract::Query,
	http::StatusCode,
	routing::{get, post},
	Extension, Json, Router,
};
use models::entity::{collection, collection_series, media, user, user::AuthUser};
use sea_orm::{prelude::*, QueryOrder};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{AppUserCollectionDto, CollectionTagBulkAddDto, UpdateSeriesForTagDto},
	errors::{APIError, APIResult},
	ids::{IdKind, KavitaIds, LOOKUP_CHUNK},
	mapper::map_collection,
};

use super::{
	query::{resolve_series_key, SeriesKey},
	route_ci, KavitaBackend,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollectionQuery {
	#[serde(default)]
	owned_only: Option<bool>,
	#[serde(default)]
	sort_by_last_modified: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesCollectionQuery {
	#[serde(default)]
	series_id: Option<i32>,
	#[serde(default)]
	owned_only: Option<bool>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Collection", get(collections));
	let router = route_ci(
		router,
		"/api/Collection/all-series",
		get(collections_for_series),
	);
	let router = route_ci(
		router,
		"/api/Collection/update-for-series",
		post(update_for_series),
	);
	route_ci(router, "/api/Collection/update-series", post(update_series))
}

/// The collections a user may list, with their Kavita ids and member counts.
pub(crate) async fn visible_collections(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	owned_only: bool,
) -> APIResult<Vec<(collection::Model, i32, i32)>> {
	let conn = ctx.conn();
	let mut query = collection::Entity::find().order_by_asc(collection::Column::Name);
	if owned_only {
		query = query.filter(collection::Column::CreatingUserId.eq(user.id.clone()));
	}
	let collections = query.all(conn).await?;
	if collections.is_empty() {
		return Ok(Vec::new());
	}
	let ids = collections
		.iter()
		.map(|row| row.id.clone())
		.collect::<Vec<_>>();
	let kavita_ids = KavitaIds::resolve_many(conn, IdKind::Collection, &ids).await?;
	let mut counts: HashMap<String, i32> = HashMap::new();
	for chunk in ids.chunks(LOOKUP_CHUNK) {
		for member in collection_series::Entity::find()
			.filter(collection_series::Column::CollectionId.is_in(chunk.to_vec()))
			.all(conn)
			.await?
		{
			*counts.entry(member.collection_id).or_default() += 1;
		}
	}
	Ok(collections
		.into_iter()
		.map(|row| {
			let id = kavita_ids[&row.id];
			let count = counts.get(&row.id).copied().unwrap_or(0);
			(row, id, count)
		})
		.collect())
}

/// `AppUserCollectionDto`s for loaded collections, with their owners resolved.
pub(crate) async fn map_collections(
	ctx: &dyn KavitaBackend,
	collections: Vec<(collection::Model, i32, i32)>,
) -> APIResult<Vec<AppUserCollectionDto>> {
	let mut owner_ids = collections
		.iter()
		.map(|(row, _, _)| row.creating_user_id.clone())
		.collect::<Vec<_>>();
	owner_ids.sort();
	owner_ids.dedup();
	let owners: HashMap<String, String> = user::Entity::find()
		.filter(user::Column::Id.is_in(owner_ids))
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|row| (row.id, row.username))
		.collect();
	Ok(collections
		.into_iter()
		.map(|(row, id, count)| {
			let owner = owners
				.get(&row.creating_user_id)
				.cloned()
				.unwrap_or_default();
			map_collection(id, &row, count, owner)
		})
		.collect())
}

/// `GET /api/Collection?ownedOnly&sortByLastModified`.
async fn collections(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<CollectionQuery>,
) -> APIResult<Json<Vec<AppUserCollectionDto>>> {
	let user = auth.user();
	let mut collections =
		visible_collections(ctx.as_ref(), &user, query.owned_only.unwrap_or(false))
			.await?;
	if query.sort_by_last_modified.unwrap_or(false) {
		collections.sort_by(|left, right| right.0.updated_at.cmp(&left.0.updated_at));
	}
	Ok(Json(map_collections(ctx.as_ref(), collections).await?))
}

/// `GET /api/Collection/all-series?seriesId&ownedOnly`: the collections a
/// series belongs to. Kavita answers an empty list for an unknown series.
async fn collections_for_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesCollectionQuery>,
) -> APIResult<Json<Vec<AppUserCollectionDto>>> {
	let user = auth.user();
	let Some(stump_series_id) =
		stump_series_id(ctx.as_ref(), &user, query.series_id.unwrap_or_default()).await?
	else {
		return Ok(Json(Vec::new()));
	};
	let member_of = collection_series::Entity::find()
		.filter(collection_series::Column::SeriesId.eq(stump_series_id))
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|member| member.collection_id)
		.collect::<std::collections::HashSet<_>>();
	let collections =
		visible_collections(ctx.as_ref(), &user, query.owned_only.unwrap_or(false))
			.await?
			.into_iter()
			.filter(|(row, _, _)| member_of.contains(&row.id))
			.collect::<Vec<_>>();
	Ok(Json(map_collections(ctx.as_ref(), collections).await?))
}

/// The Stump series a Kavita series id names: itself for a grouped series,
/// the series holding the file for a book.
async fn stump_series_id(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_id: i32,
) -> APIResult<Option<String>> {
	match resolve_series_key(ctx, series_id).await? {
		Some(SeriesKey::Series(id)) => Ok(Some(id)),
		Some(SeriesKey::Book(media_id)) => Ok(media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.filter(media::Column::DeletedAt.is_null())
			.one(ctx.conn())
			.await?
			.and_then(|row| row.series_id)),
		None => Ok(None),
	}
}

/// The Stump series behind a list of Kavita series ids, de-duplicated and in
/// request order. Ids that name nothing visible are dropped, as Kavita drops
/// series the user cannot see.
async fn stump_series_ids(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_ids: &[i32],
) -> APIResult<Vec<String>> {
	let mut resolved = Vec::with_capacity(series_ids.len());
	for id in series_ids {
		if let Some(stump_id) = stump_series_id(ctx, user, *id).await? {
			if !resolved.contains(&stump_id) {
				resolved.push(stump_id);
			}
		}
	}
	Ok(resolved)
}

/// The current membership of a collection, in insertion order.
async fn members(ctx: &dyn KavitaBackend, collection_id: &str) -> APIResult<Vec<String>> {
	Ok(collection_series::Entity::find()
		.filter(collection_series::Column::CollectionId.eq(collection_id))
		.order_by_asc(collection_series::Column::SeriesId)
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|member| member.series_id)
		.collect())
}

/// Resolve a Kavita collection id to the Stump collection it names.
async fn find_collection(
	ctx: &dyn KavitaBackend,
	id: i32,
) -> APIResult<collection::Model> {
	let stump_id = KavitaIds::lookup(ctx.conn(), IdKind::Collection, id)
		.await?
		.ok_or_else(|| APIError::BadRequest("Collection does not exist".to_owned()))?;
	collection::Entity::find_by_id(stump_id)
		.one(ctx.conn())
		.await?
		.ok_or_else(|| APIError::BadRequest("Collection does not exist".to_owned()))
}

/// `CollectionController.AddToMultipleSeries`: add series to a collection,
/// creating it when `collectionTagId` is `0`. Kavita answers `200` with an
/// empty body.
async fn update_for_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<CollectionTagBulkAddDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let series_ids = stump_series_ids(ctx.as_ref(), &user, &body.series_ids).await?;
	if body.collection_tag_id == 0 {
		let title = body
			.collection_tag_title
			.as_deref()
			.map(str::trim)
			.filter(|title| !title.is_empty())
			.ok_or_else(|| {
				APIError::BadRequest("A collection title is required".to_owned())
			})?;
		ctx.create_collection(&user, title.to_owned(), series_ids)
			.await?;
		return Ok(StatusCode::OK);
	}
	let collection = find_collection(ctx.as_ref(), body.collection_tag_id).await?;
	let mut membership = members(ctx.as_ref(), &collection.id).await?;
	for series_id in series_ids {
		if !membership.contains(&series_id) {
			membership.push(series_id);
		}
	}
	ctx.set_collection_series(&user, &collection.id, membership)
		.await?;
	Ok(StatusCode::OK)
}

/// `CollectionController.RemoveSeriesFromCollection`: drop series from a
/// collection. Kavita answers `200` with an empty body.
async fn update_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<UpdateSeriesForTagDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	let collection = find_collection(ctx.as_ref(), body.tag.id).await?;
	let removed =
		stump_series_ids(ctx.as_ref(), &user, &body.series_ids_to_remove).await?;
	let membership = members(ctx.as_ref(), &collection.id)
		.await?
		.into_iter()
		.filter(|series_id| !removed.contains(series_id))
		.collect::<Vec<_>>();
	ctx.set_collection_series(&user, &collection.id, membership)
		.await?;
	Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::{
		auth_user, db, library_of_type, request, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	async fn backend() -> (std::sync::Arc<TestBackend>, AuthUser, i32, i32) {
		let conn = db().await;
		let user_row = fake_data::User::new("curator").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (alpha, _) =
			series_with_files(&conn, &library.id, "Alpha", &[("a v01", "cbz", 10)]).await;
		let (beta, _) =
			series_with_files(&conn, &library.id, "Beta", &[("b v01", "cbz", 12)]).await;
		let alpha_id = KavitaIds::resolve(&conn, IdKind::Series, &alpha.id)
			.await
			.unwrap();
		let beta_id = KavitaIds::resolve(&conn, IdKind::Series, &beta.id)
			.await
			.unwrap();
		(
			std::sync::Arc::new(TestBackend::new(conn)),
			user,
			alpha_id,
			beta_id,
		)
	}

	/// `collectionTagId: 0` creates the collection; a second call with the
	/// returned id appends. `update-series` removes. Every write lands in
	/// `collections`/`collection_series`, the tables the other surfaces read.
	#[tokio::test]
	async fn update_for_series_creates_then_appends_and_update_series_removes() {
		let (backend, user, alpha, beta) = backend().await;

		let (status, first_body) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Collection/update-for-series",
			Some(serde_json::json!({
				"collectionTagId": 0,
				"collectionTagTitle": "Kav Test",
				"seriesIds": [alpha],
			})),
		)
		.await;
		assert_eq!(status, StatusCode::OK, "{first_body}");

		let (status, body) =
			request(backend.clone(), &user, "GET", "/api/Collection", None).await;
		assert_eq!(status, StatusCode::OK);
		let created = &body[0];
		assert_eq!(created["title"], "Kav Test");
		assert_eq!(created["itemCount"], 1);
		assert_eq!(created["owner"], "curator");
		assert_eq!(created["summary"], "");
		assert_eq!(created["promoted"], false);
		assert_eq!(created["ageRating"], 0);
		assert_eq!(created["source"], 0);
		assert_eq!(created["lastSyncUtc"], "0001-01-01T00:00:00");
		assert!(created["coverImage"].is_null());
		let collection_id = created["id"].as_i64().unwrap();

		// Appending keeps the existing member.
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/collection/update-for-series",
			Some(serde_json::json!({
				"collectionTagId": collection_id,
				"seriesIds": [beta],
			})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let (_, body) =
			request(backend.clone(), &user, "GET", "/api/Collection", None).await;
		assert_eq!(body[0]["itemCount"], 2);

		// `all-series` reports the collections a series is in.
		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Collection/all-series?seriesId={alpha}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body[0]["id"], collection_id);

		// Removing drops exactly the named series.
		let (status, _) = request(
			backend.clone(),
			&user,
			"POST",
			"/api/Collection/update-series",
			Some(serde_json::json!({
				"tag": {"id": collection_id, "title": "Kav Test"},
				"seriesIdsToRemove": [alpha],
			})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let (_, body) =
			request(backend.clone(), &user, "GET", "/api/Collection", None).await;
		assert_eq!(body[0]["itemCount"], 1);
		let (_, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Collection/all-series?seriesId={alpha}"),
			None,
		)
		.await;
		assert!(body.as_array().unwrap().is_empty());

		// An unknown series is an empty list, and a missing title on a
		// create is a bad request.
		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			"/api/Collection/all-series?seriesId=999999",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert!(body.as_array().unwrap().is_empty());
		let (status, _) = request(
			backend,
			&user,
			"POST",
			"/api/Collection/update-for-series",
			Some(serde_json::json!({"collectionTagId": 0, "seriesIds": [alpha]})),
		)
		.await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
	}

	/// `ownedOnly=true` hides another user's collections.
	#[tokio::test]
	async fn owned_only_restricts_to_the_callers_collections() {
		let (backend, user, alpha, _) = backend().await;
		let other_row = fake_data::User::new("other").insert(backend.conn()).await;
		let other = auth_user(&other_row);
		request(
			backend.clone(),
			&other,
			"POST",
			"/api/Collection/update-for-series",
			Some(serde_json::json!({
				"collectionTagId": 0,
				"collectionTagTitle": "Theirs",
				"seriesIds": [alpha],
			})),
		)
		.await;

		let (_, all) =
			request(backend.clone(), &user, "GET", "/api/Collection", None).await;
		assert_eq!(all.as_array().unwrap().len(), 1);
		let (_, owned) = request(
			backend,
			&user,
			"GET",
			"/api/Collection?ownedOnly=true",
			None,
		)
		.await;
		assert!(owned.as_array().unwrap().is_empty());
	}
}
