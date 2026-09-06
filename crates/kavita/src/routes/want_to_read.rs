//! `WantToReadController`: the user's want-to-read shelf.
//!
//! Kavita stores `AppUserWantToRead` rows against series. A Kavita series is
//! either a Stump series (Manga/Comic libraries) or a single media item of a
//! Book/LightNovel library, so membership lands in `favorite_series` or
//! `favorite_media` respectively — the same per-user tables Stump's own
//! "favourites" use, which is what makes the shelf visible to every surface
//! instead of only to Kavita clients.

use std::sync::Arc;

use axum::{
	extract::Query,
	http::StatusCode,
	response::Response,
	routing::{get, post},
	Extension, Json, Router,
};
use models::entity::{favorite_media, favorite_series, user::AuthUser};
use sea_orm::{prelude::*, sea_query::OnConflict, ActiveValue::Set, TransactionTrait};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{PaginationHeader, SeriesDto, UpdateWantToReadDto},
	errors::APIResult,
	filter::SeriesFilterV2Dto,
};

use super::{
	query::{resolve_series_key, SeriesKey},
	route_ci,
	series::{list_series, pagination_response, user_params},
	KavitaBackend,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesIdQuery {
	#[serde(default)]
	series_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/want-to-read/v2", post(want_to_read_v2));
	let router = route_ci(router, "/api/want-to-read", get(is_want_to_read));
	let router = route_ci(router, "/api/want-to-read/add-series", post(add_series));
	route_ci(
		router,
		"/api/want-to-read/remove-series",
		post(remove_series),
	)
}

/// The Kavita series keys on the user's shelf.
pub(crate) async fn want_to_read_keys(
	ctx: &dyn KavitaBackend,
	user_id: &str,
) -> APIResult<Vec<SeriesKey>> {
	let mut keys = favorite_series::Entity::find()
		.filter(favorite_series::Column::UserId.eq(user_id.to_owned()))
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|row| SeriesKey::Series(row.series_id))
		.collect::<Vec<_>>();
	keys.extend(
		favorite_media::Entity::find()
			.filter(favorite_media::Column::UserId.eq(user_id.to_owned()))
			.all(ctx.conn())
			.await?
			.into_iter()
			.map(|row| SeriesKey::Book(row.media_id)),
	);
	Ok(keys)
}

/// `WantToReadController.GetWantToReadForUserV2`: the filter pipeline over
/// the shelf, paged with the `Pagination` header.
pub(crate) async fn list_want_to_read(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	filter: &SeriesFilterV2Dto,
	params: super::series::UserParams,
) -> APIResult<(Vec<SeriesDto>, PaginationHeader)> {
	let keys = want_to_read_keys(ctx, &user.id).await?;
	list_series(ctx, user, filter, params, None, Some(&keys)).await
}

async fn want_to_read_v2(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
	Json(filter): Json<SeriesFilterV2Dto>,
) -> APIResult<Response> {
	let user = auth.user();
	let (items, header) =
		list_want_to_read(ctx.as_ref(), &user, &filter, user_params(&uri)).await?;
	pagination_response(items, header)
}

/// `GET /api/want-to-read?seriesId=`: whether the series is on the shelf.
/// A series the user cannot see, or an unknown id, is simply not on it.
pub(crate) async fn is_want_to_read_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_id: i32,
) -> APIResult<bool> {
	let Some(key) = resolve_series_key(ctx, series_id).await? else {
		return Ok(false);
	};
	Ok(want_to_read_keys(ctx, &user.id).await?.contains(&key))
}

async fn is_want_to_read(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Json<bool>> {
	let user = auth.user();
	Ok(Json(
		is_want_to_read_for(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?,
	))
}

/// `WantToReadController.AddSeries`. Kavita ignores ids it cannot resolve and
/// answers `200`; adding an id already on the shelf is a no-op.
pub(crate) async fn add_series_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_ids: &[i32],
) -> APIResult<()> {
	// Every id is resolved before the transaction opens: resolution reads
	// through the same pool, and holding a transaction across it would
	// deadlock a single-connection pool.
	let mut series = Vec::new();
	let mut media = Vec::new();
	for id in series_ids {
		match resolve_series_key(ctx, *id).await? {
			Some(SeriesKey::Series(series_id)) => series.push(series_id),
			Some(SeriesKey::Book(media_id)) => media.push(media_id),
			None => {},
		}
	}
	let now = chrono::Utc::now().into();
	let txn = ctx.conn().begin().await?;
	for series_id in series {
		favorite_series::Entity::insert(favorite_series::ActiveModel {
			user_id: Set(user.id.clone()),
			series_id: Set(series_id),
			favorited_at: Set(now),
		})
		.on_conflict(
			OnConflict::columns([
				favorite_series::Column::UserId,
				favorite_series::Column::SeriesId,
			])
			.do_nothing()
			.to_owned(),
		)
		.do_nothing()
		.exec(&txn)
		.await?;
	}
	for media_id in media {
		favorite_media::Entity::insert(favorite_media::ActiveModel {
			user_id: Set(user.id.clone()),
			media_id: Set(media_id),
			favorited_at: Set(now),
		})
		.on_conflict(
			OnConflict::columns([
				favorite_media::Column::UserId,
				favorite_media::Column::MediaId,
			])
			.do_nothing()
			.to_owned(),
		)
		.do_nothing()
		.exec(&txn)
		.await?;
	}
	txn.commit().await?;
	Ok(())
}

async fn add_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<UpdateWantToReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	add_series_for(ctx.as_ref(), &user, &body.series_ids).await?;
	Ok(StatusCode::OK)
}

/// `WantToReadController.RemoveSeries`.
pub(crate) async fn remove_series_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_ids: &[i32],
) -> APIResult<()> {
	let mut series = Vec::new();
	let mut media = Vec::new();
	for id in series_ids {
		match resolve_series_key(ctx, *id).await? {
			Some(SeriesKey::Series(series_id)) => series.push(series_id),
			Some(SeriesKey::Book(media_id)) => media.push(media_id),
			None => {},
		}
	}
	let txn = ctx.conn().begin().await?;
	if !series.is_empty() {
		favorite_series::Entity::delete_many()
			.filter(favorite_series::Column::UserId.eq(user.id.clone()))
			.filter(favorite_series::Column::SeriesId.is_in(series))
			.exec(&txn)
			.await?;
	}
	if !media.is_empty() {
		favorite_media::Entity::delete_many()
			.filter(favorite_media::Column::UserId.eq(user.id.clone()))
			.filter(favorite_media::Column::MediaId.is_in(media))
			.exec(&txn)
			.await?;
	}
	txn.commit().await?;
	Ok(())
}

async fn remove_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<UpdateWantToReadDto>,
) -> APIResult<StatusCode> {
	let user = auth.user();
	remove_series_for(ctx.as_ref(), &user, &body.series_ids).await?;
	Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ids::{IdKind, KavitaIds};
	use crate::routes::series::UserParams;
	use crate::test_support::{
		auth_user, db, library_of_type, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	/// A grouped series and a book of a Book library round-trip through the
	/// shelf: `add-series` puts both on it, `v2` pages them with the
	/// `Pagination` total, `GET /api/want-to-read` reports each one, and
	/// `remove-series` empties it.
	#[tokio::test]
	async fn want_to_read_round_trips_series_and_books() {
		let conn = db().await;
		let user_row = fake_data::User::new("shelver").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);

		let comics = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (comic_series, _) = series_with_files(
			&backend.conn,
			&comics.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let books = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		let (_, book_files) = series_with_files(
			&backend.conn,
			&books.id,
			"Collection",
			&[("alice", "epub", 15)],
		)
		.await;

		let comic_id =
			KavitaIds::resolve(&backend.conn, IdKind::Series, &comic_series.id)
				.await
				.unwrap();
		let book_id =
			KavitaIds::resolve(&backend.conn, IdKind::BookSeries, &book_files[0].id)
				.await
				.unwrap();

		let (empty, header) = list_want_to_read(
			&backend,
			&user,
			&SeriesFilterV2Dto::default(),
			UserParams::parse("PageNumber=1&PageSize=20"),
		)
		.await
		.unwrap();
		assert!(empty.is_empty());
		assert_eq!(header.total_items, 0);

		add_series_for(&backend, &user, &[book_id, comic_id])
			.await
			.unwrap();
		let (page, header) = list_want_to_read(
			&backend,
			&user,
			&SeriesFilterV2Dto::default(),
			UserParams::parse("PageNumber=1&PageSize=20"),
		)
		.await
		.unwrap();
		let mut ids = page.iter().map(|dto| dto.id).collect::<Vec<_>>();
		ids.sort_unstable();
		assert_eq!(ids, {
			let mut expected = vec![comic_id, book_id];
			expected.sort_unstable();
			expected
		});
		assert_eq!(header.total_items, 2);
		assert!(is_want_to_read_for(&backend, &user, comic_id)
			.await
			.unwrap());
		assert!(is_want_to_read_for(&backend, &user, book_id).await.unwrap());

		// Kavita answers 200 for ids it cannot resolve; the shelf is unchanged.
		add_series_for(&backend, &user, &[999_999]).await.unwrap();
		assert_eq!(
			want_to_read_keys(&backend, &user_row.id)
				.await
				.unwrap()
				.len(),
			2
		);

		remove_series_for(&backend, &user, &[comic_id, book_id])
			.await
			.unwrap();
		let (page, header) = list_want_to_read(
			&backend,
			&user,
			&SeriesFilterV2Dto::default(),
			UserParams::parse("PageNumber=1&PageSize=20"),
		)
		.await
		.unwrap();
		assert!(page.is_empty());
		assert_eq!(header.total_items, 0);
		assert!(!is_want_to_read_for(&backend, &user, comic_id)
			.await
			.unwrap());
	}

	/// A second user's shelf is not this user's.
	#[tokio::test]
	async fn shelves_are_per_user() {
		let conn = db().await;
		let owner = fake_data::User::new("owner").insert(&conn).await;
		let other = fake_data::User::new("other").insert(&conn).await;
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (series, _) = series_with_files(
			&backend.conn,
			&library.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let series_id = KavitaIds::resolve(&backend.conn, IdKind::Series, &series.id)
			.await
			.unwrap();

		add_series_for(&backend, &auth_user(&owner), &[series_id])
			.await
			.unwrap();
		assert!(
			!is_want_to_read_for(&backend, &auth_user(&other), series_id)
				.await
				.unwrap()
		);
	}
}
