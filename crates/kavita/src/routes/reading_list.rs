//! `ReadingListController`: Stump reading lists as Kavita reading lists.
//!
//! A Kavita reading-list item is a chapter; a Stump reading-list item is a
//! media item, which is exactly that chapter. Adding a *series* to a list
//! therefore appends every chapter of it, which is what
//! `ReadingListController.UpdateListBySeries` does, and adding a Kavita volume
//! appends the one media item behind it. Membership is replaced through
//! [`KavitaBackend::set_read_list_items`] so the write goes through the same
//! canonical container service the native, Komga and Kobo surfaces use;
//! reads come straight off `reading_lists`/`reading_list_items` with the
//! shared RBAC helper.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
	extract::Query,
	response::Response,
	routing::{get, post},
	Extension, Json, Router,
};
use models::entity::{reading_list, reading_list_item, user, user::AuthUser};
use sea_orm::{prelude::*, QueryOrder};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{
		CreateReadingListDto, PaginationHeader, ReadingListDto, ReadingListItemDto,
		UpdateReadingListByItemDto, UpdateReadingListBySeriesDto,
	},
	errors::{APIError, APIResult},
	ids::{IdKind, KavitaIds},
	mapper::{library_type, map_reading_list, map_reading_list_item},
	routes::series::{pagination_response, user_params, UserParams},
};

use super::{
	query::{find_media, find_series_input, library_for_series},
	route_ci, KavitaBackend,
};

/// A reader may read a list; `reading_list::Entity::find_for_user` takes the
/// minimum role, and Kavita has no notion of list roles beyond ownership.
const READER_ROLE: i32 = 1;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadingListIdQuery {
	#[serde(default)]
	reading_list_id: Option<i32>,
}

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
	let router = route_ci(
		router,
		"/api/ReadingList",
		get(reading_list_by_id).delete(delete_reading_list),
	);
	let router = route_ci(router, "/api/ReadingList/lists", post(reading_lists));
	let router = route_ci(
		router,
		"/api/ReadingList/lists-for-series",
		get(lists_for_series),
	);
	let router = route_ci(router, "/api/ReadingList/items", get(reading_list_items));
	let router = route_ci(router, "/api/ReadingList/create", post(create_reading_list));
	let router = route_ci(
		router,
		"/api/ReadingList/update-by-series",
		post(update_by_series),
	);
	let router = route_ci(
		router,
		"/api/ReadingList/update-by-volume",
		post(update_by_item),
	);
	route_ci(
		router,
		"/api/ReadingList/update-by-chapter",
		post(update_by_item),
	)
}

/// The user-visible lists with their Kavita ids, owner names and item counts.
async fn visible_lists(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
) -> APIResult<Vec<(reading_list::Model, i32, i32)>> {
	let conn = ctx.conn();
	let lists = reading_list::Entity::find_for_user(user, READER_ROLE)
		.all(conn)
		.await?;
	if lists.is_empty() {
		return Ok(Vec::new());
	}
	let ids = lists.iter().map(|list| list.id.clone()).collect::<Vec<_>>();
	let kavita_ids = KavitaIds::resolve_many(conn, IdKind::ReadingList, &ids).await?;
	let mut counts: HashMap<String, i32> = HashMap::new();
	for item in reading_list_item::Entity::find()
		.filter(reading_list_item::Column::ReadingListId.is_in(ids))
		.all(conn)
		.await?
	{
		*counts.entry(item.reading_list_id).or_default() += 1;
	}
	Ok(lists
		.into_iter()
		.map(|list| {
			let id = kavita_ids[&list.id];
			let count = counts.get(&list.id).copied().unwrap_or(0);
			(list, id, count)
		})
		.collect())
}

async fn owner_names(
	ctx: &dyn KavitaBackend,
	lists: &[(reading_list::Model, i32, i32)],
) -> APIResult<HashMap<String, String>> {
	let mut ids = lists
		.iter()
		.map(|(list, _, _)| list.creating_user_id.clone())
		.collect::<Vec<_>>();
	ids.sort();
	ids.dedup();
	Ok(user::Entity::find()
		.filter(user::Column::Id.is_in(ids))
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|row| (row.id, row.username))
		.collect())
}

fn map_lists(
	lists: Vec<(reading_list::Model, i32, i32)>,
	owners: &HashMap<String, String>,
) -> Vec<ReadingListDto> {
	lists
		.into_iter()
		.map(|(list, id, count)| {
			let owner = owners
				.get(&list.creating_user_id)
				.cloned()
				.unwrap_or_default();
			map_reading_list(id, &list, count, owner)
		})
		.collect()
}

/// Kavita's default list ordering is by title; ties break on the Kavita id so
/// two lists that share a title — the replay harness makes exactly that —
/// keep a stable place across refreshes.
fn by_title(
	left: &(reading_list::Model, i32, i32),
	right: &(reading_list::Model, i32, i32),
) -> std::cmp::Ordering {
	left.0
		.name
		.to_lowercase()
		.cmp(&right.0.name.to_lowercase())
		.then_with(|| left.1.cmp(&right.1))
}

/// `ReadingListController.GetListsForUser`: the user's lists, sorted by last
/// modified (newest first) or by title, paged with the `Pagination` header.
/// Stump has no promoted lists, so `includePromoted` adds nothing.
pub(crate) async fn list_reading_lists(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	sort_by_last_modified: bool,
	params: UserParams,
) -> APIResult<(Vec<ReadingListDto>, PaginationHeader)> {
	let mut lists = visible_lists(ctx, user).await?;
	if sort_by_last_modified {
		lists.sort_by(|left, right| right.0.updated_at.cmp(&left.0.updated_at));
	} else {
		lists.sort_by(by_title);
	}
	let total = i32::try_from(lists.len()).unwrap_or(i32::MAX);
	let page = if params.page_size == UserParams::MAX_PAGE_SIZE {
		lists
	} else {
		lists
			.into_iter()
			.skip(params.offset())
			.take(usize::try_from(params.page_size).unwrap_or(0))
			.collect()
	};
	let owners = owner_names(ctx, &page).await?;
	Ok((
		map_lists(page, &owners),
		PaginationHeader::new(params.requested_page_number, params.page_size, total),
	))
}

/// Whether a query flag was sent as `true`; Kavita's defaults are
/// `includePromoted=true` and `sortByLastModified=false`.
fn query_flag(uri: &axum::http::Uri, name: &str, default: bool) -> bool {
	uri.query()
		.unwrap_or_default()
		.split('&')
		.find_map(|pair| {
			let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
			key.eq_ignore_ascii_case(name)
				.then(|| !value.eq_ignore_ascii_case("false"))
		})
		.unwrap_or(default)
}

async fn reading_lists(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	uri: axum::http::Uri,
) -> APIResult<Response> {
	let user = auth.user();
	let (items, header) = list_reading_lists(
		ctx.as_ref(),
		&user,
		query_flag(&uri, "sortByLastModified", false),
		user_params(&uri),
	)
	.await?;
	pagination_response(items, header)
}

/// `GET /api/ReadingList?readingListId=`.
async fn reading_list_by_id(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ReadingListIdQuery>,
) -> APIResult<Json<ReadingListDto>> {
	let user = auth.user();
	let id = query.reading_list_id.unwrap_or_default();
	let lists = visible_lists(ctx.as_ref(), &user).await?;
	let found = lists
		.into_iter()
		.find(|(_, kavita_id, _)| *kavita_id == id)
		.ok_or_else(|| APIError::BadRequest("Reading list does not exist".to_owned()))?;
	let owners = owner_names(ctx.as_ref(), std::slice::from_ref(&found)).await?;
	Ok(Json(
		map_lists(vec![found], &owners)
			.pop()
			.expect("one list maps to one dto"),
	))
}

/// Kavita's `ReadingListController.DeleteList` reply, verbatim: `200` with a
/// plain-text body whether or not anything was deleted.
const DELETED: &str = "Reading List was deleted";

/// `DELETE /api/ReadingList?readingListId=`.
///
/// Kavita deletes from the caller's own lists (`user.ReadingLists`) and
/// answers `200 "Reading List was deleted"` regardless — `kavita-ref` 0.9.1.4
/// says it for an id that never existed too — so a list the caller can read
/// but does not own is left alone rather than refused, and a client's retry
/// of a delete it already made is not an error.
pub(crate) async fn delete_reading_list_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	reading_list_id: i32,
) -> APIResult<&'static str> {
	let owned =
		visible_lists(ctx, user)
			.await?
			.into_iter()
			.find(|(list, kavita_id, _)| {
				*kavita_id == reading_list_id && list.creating_user_id == user.id
			});
	if let Some((list, _, _)) = owned {
		ctx.delete_read_list(user, &list.id).await?;
	}
	Ok(DELETED)
}

async fn delete_reading_list(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ReadingListIdQuery>,
) -> APIResult<&'static str> {
	let user = auth.user();
	delete_reading_list_for(
		ctx.as_ref(),
		&user,
		query.reading_list_id.unwrap_or_default(),
	)
	.await
}

/// `ReadingListController.GetListsForSeries`: the lists that already hold at
/// least one chapter of the series, in the same title order as
/// `ReadingList/lists`.
pub(crate) async fn lists_for_series_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	series_id: i32,
) -> APIResult<Vec<ReadingListDto>> {
	let Some(input) = find_series_input(ctx, user, series_id).await? else {
		return Ok(Vec::new());
	};
	let media_ids = input
		.media
		.iter()
		.map(|media| media.media.id.clone())
		.collect::<Vec<_>>();
	if media_ids.is_empty() {
		return Ok(Vec::new());
	}
	let holding = reading_list_item::Entity::find()
		.filter(reading_list_item::Column::MediaId.is_in(media_ids))
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|item| item.reading_list_id)
		.collect::<std::collections::HashSet<_>>();
	let mut lists = visible_lists(ctx, user)
		.await?
		.into_iter()
		.filter(|(list, _, _)| holding.contains(&list.id))
		.collect::<Vec<_>>();
	lists.sort_by(by_title);
	let owners = owner_names(ctx, &lists).await?;
	Ok(map_lists(lists, &owners))
}

async fn lists_for_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Json<Vec<ReadingListDto>>> {
	let user = auth.user();
	Ok(Json(
		lists_for_series_for(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?,
	))
}

/// `ReadingListController.GetListForUser`: the list's chapters in order.
/// Items whose media the user cannot see are skipped, and `order` is
/// renumbered over what remains so the response is always `0..n`.
pub(crate) async fn reading_list_items_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	reading_list_id: i32,
) -> APIResult<Vec<ReadingListItemDto>> {
	let lists = visible_lists(ctx, user).await?;
	let Some((list, _, _)) = lists
		.iter()
		.find(|(_, kavita_id, _)| *kavita_id == reading_list_id)
	else {
		return Err(APIError::BadRequest(
			"Reading list does not exist".to_owned(),
		));
	};
	let rows = reading_list_item::Entity::find()
		.filter(reading_list_item::Column::ReadingListId.eq(list.id.clone()))
		.order_by_asc(reading_list_item::Column::DisplayOrder)
		.order_by_asc(reading_list_item::Column::Id)
		.all(ctx.conn())
		.await?;
	let mut items = Vec::with_capacity(rows.len());
	for row in rows {
		let chapter_id =
			KavitaIds::resolve(ctx.conn(), IdKind::Media, &row.media_id).await?;
		let Some((input, index)) = find_media(ctx, user, chapter_id).await? else {
			continue;
		};
		let library = library_for_series(ctx, user, &input).await?;
		let library_type =
			library_type(library.as_ref().and_then(|(_, config)| config.as_ref()));
		let order = i32::try_from(items.len()).unwrap_or(i32::MAX);
		items.push(map_reading_list_item(
			reading_list_id,
			row.id,
			order,
			&input,
			&input.media[index],
			library_type,
		));
	}
	Ok(items)
}

async fn reading_list_items(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ReadingListIdQuery>,
) -> APIResult<Json<Vec<ReadingListItemDto>>> {
	let user = auth.user();
	Ok(Json(
		reading_list_items_for(
			ctx.as_ref(),
			&user,
			query.reading_list_id.unwrap_or_default(),
		)
		.await?,
	))
}

/// `ReadingListController.CreateList`: a blank list with the given title.
pub(crate) async fn create_reading_list_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	title: &str,
) -> APIResult<ReadingListDto> {
	let title = title.trim();
	if title.is_empty() {
		return Err(APIError::BadRequest(
			"Reading list title cannot be empty".to_owned(),
		));
	}
	let list = ctx.create_read_list(user, title.to_owned()).await?;
	let id = KavitaIds::resolve(ctx.conn(), IdKind::ReadingList, &list.id).await?;
	Ok(map_reading_list(id, &list, 0, user.username.clone()))
}

async fn create_reading_list(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<CreateReadingListDto>,
) -> APIResult<Json<ReadingListDto>> {
	let user = auth.user();
	Ok(Json(
		create_reading_list_for(ctx.as_ref(), &user, &body.title).await?,
	))
}

/// Kavita's `AddChaptersToReadingList` result: `"Updated"` when the list grew,
/// `"Nothing to do"` when every chapter was already on it. Both are `200`
/// with a plain-text body.
const UPDATED: &str = "Updated";
const NOTHING_TO_DO: &str = "Nothing to do";

/// Append `media_ids` to the list, keeping the existing order and skipping
/// members already on it.
async fn append_items(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	reading_list_id: i32,
	media_ids: Vec<String>,
) -> APIResult<&'static str> {
	let lists = visible_lists(ctx, user).await?;
	let Some((list, _, _)) = lists
		.iter()
		.find(|(_, kavita_id, _)| *kavita_id == reading_list_id)
	else {
		return Err(APIError::BadRequest(
			"Reading list does not exist".to_owned(),
		));
	};
	let mut existing = reading_list_item::Entity::find()
		.filter(reading_list_item::Column::ReadingListId.eq(list.id.clone()))
		.order_by_asc(reading_list_item::Column::DisplayOrder)
		.order_by_asc(reading_list_item::Column::Id)
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|item| item.media_id)
		.collect::<Vec<_>>();
	let added = media_ids
		.into_iter()
		.filter(|id| !existing.contains(id))
		.collect::<Vec<_>>();
	if added.is_empty() {
		return Ok(NOTHING_TO_DO);
	}
	existing.extend(added);
	ctx.set_read_list_items(user, &list.id, existing).await?;
	Ok(UPDATED)
}

async fn update_by_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<UpdateReadingListBySeriesDto>,
) -> APIResult<&'static str> {
	let user = auth.user();
	update_by_series_for(ctx.as_ref(), &user, body.reading_list_id, body.series_id).await
}

/// `POST /api/ReadingList/update-by-series`: every chapter of the series.
pub(crate) async fn update_by_series_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	reading_list_id: i32,
	series_id: i32,
) -> APIResult<&'static str> {
	let Some(input) = find_series_input(ctx, user, series_id).await? else {
		return Err(APIError::BadRequest("Series does not exist".to_owned()));
	};
	let media_ids = input
		.media
		.iter()
		.map(|media| media.media.id.clone())
		.collect::<Vec<_>>();
	append_items(ctx, user, reading_list_id, media_ids).await
}

async fn update_by_item(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Json(body): Json<UpdateReadingListByItemDto>,
) -> APIResult<&'static str> {
	let user = auth.user();
	update_by_item_for(
		ctx.as_ref(),
		&user,
		body.reading_list_id,
		body.item_id().unwrap_or_default(),
	)
	.await
}

/// `POST /api/ReadingList/update-by-chapter` and `update-by-volume`: the one
/// media item behind the Kavita volume/chapter id.
pub(crate) async fn update_by_item_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	reading_list_id: i32,
	item_id: i32,
) -> APIResult<&'static str> {
	let Some((input, index)) = find_media(ctx, user, item_id).await? else {
		return Err(APIError::BadRequest("Chapter does not exist".to_owned()));
	};
	let media_id = input.media[index].media.id.clone();
	append_items(ctx, user, reading_list_id, vec![media_id]).await
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::dto::{LibraryType, MangaFormat};
	use crate::test_support::{
		auth_user, db, library_of_type, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;

	/// Create a list, add a Book-library series and a Comic chapter to it, and
	/// read the items back: the Kavita ids, order, per-chapter identity block
	/// and `Volume <title>` naming come out the way `kavita-ref` reports them.
	#[tokio::test]
	async fn reading_list_create_add_and_items_round_trip() {
		let conn = db().await;
		let user_row = fake_data::User::new("lister").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);

		let books = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		let (_, book_files) = series_with_files(
			&backend.conn,
			&books.id,
			"Collection",
			&[("alice", "epub", 15)],
		)
		.await;
		let comics = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (comic_series, comic_files) = series_with_files(
			&backend.conn,
			&comics.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;

		let book_series_id =
			KavitaIds::resolve(&backend.conn, IdKind::BookSeries, &book_files[0].id)
				.await
				.unwrap();
		let comic_chapter_id =
			KavitaIds::resolve(&backend.conn, IdKind::Media, &comic_files[0].id)
				.await
				.unwrap();

		let (lists, header) =
			list_reading_lists(&backend, &user, true, UserParams::parse(""))
				.await
				.unwrap();
		assert!(lists.is_empty());
		assert_eq!(header.total_items, 0);

		let created = create_reading_list_for(&backend, &user, "  Phone list  ")
			.await
			.unwrap();
		assert_eq!(created.title, "Phone list");
		assert_eq!(created.item_count, 0);
		assert_eq!(created.owner_user_name, "lister");
		assert_eq!(created.provider, crate::dto::ReadingListProvider::None);

		assert_eq!(
			update_by_series_for(&backend, &user, created.id, book_series_id)
				.await
				.unwrap(),
			UPDATED
		);
		assert_eq!(
			update_by_series_for(&backend, &user, created.id, book_series_id)
				.await
				.unwrap(),
			NOTHING_TO_DO,
			"a chapter already on the list is not added twice"
		);
		assert_eq!(
			update_by_item_for(&backend, &user, created.id, comic_chapter_id)
				.await
				.unwrap(),
			UPDATED
		);

		let items = reading_list_items_for(&backend, &user, created.id)
			.await
			.unwrap();
		assert_eq!(items.len(), 2);
		assert_eq!(
			items.iter().map(|item| item.order).collect::<Vec<_>>(),
			[0, 1]
		);
		let book = &items[0];
		assert_eq!(book.reading_list_id, created.id);
		assert_eq!(book.series_id, book_series_id);
		assert_eq!(book.chapter_id, book.volume_id);
		assert_eq!(book.series_format, MangaFormat::Epub);
		assert_eq!(book.library_type, LibraryType::Book);
		assert_eq!(book.library_name, books.name);
		assert_eq!(book.pages_total, 15);
		assert_eq!(book.volume_number, "-100000");
		assert_eq!(book.chapter_number, "alice");
		assert!(book.is_special);
		// `FormatReadingListItemTitle` for an EPUB with a real chapter number
		// under a loose-leaf volume: `kavita-ref` item 1 of "Phone list".
		assert_eq!(book.title, "Volume alice");
		assert_eq!(book.chapter.id, book.chapter_id);
		assert_eq!(book.chapter.range, "alice");
		assert_eq!(book.volume.name, "-100000");
		assert_eq!(book.volume.series_id, book_series_id);

		let comic = &items[1];
		assert_eq!(comic.chapter_id, comic_chapter_id);
		assert_eq!(comic.library_type, LibraryType::Comic);
		assert_eq!(comic.pages_total, 36);
		assert_eq!(comic.volume_number, "1");
		assert!(!comic.is_special);

		// The list now reports two items and shows up for both series.
		let (lists, header) =
			list_reading_lists(&backend, &user, true, UserParams::parse(""))
				.await
				.unwrap();
		assert_eq!(lists.len(), 1);
		assert_eq!((lists[0].id, lists[0].item_count), (created.id, 2));
		assert_eq!(header.total_items, 1);

		let comic_series_id =
			KavitaIds::resolve(&backend.conn, IdKind::Series, &comic_series.id)
				.await
				.unwrap();
		for series_id in [book_series_id, comic_series_id] {
			let holding = lists_for_series_for(&backend, &user, series_id)
				.await
				.unwrap();
			assert_eq!(
				holding.iter().map(|list| list.id).collect::<Vec<_>>(),
				[created.id]
			);
		}
	}

	/// A blank title is rejected, and another user's list is invisible.
	#[tokio::test]
	async fn lists_are_owner_scoped_and_titles_required() {
		let conn = db().await;
		let owner = fake_data::User::new("owner").insert(&conn).await;
		let other = fake_data::User::new("other").insert(&conn).await;
		let backend = TestBackend::new(conn);

		assert!(create_reading_list_for(&backend, &auth_user(&owner), "   ")
			.await
			.is_err());
		let created = create_reading_list_for(&backend, &auth_user(&owner), "Mine")
			.await
			.unwrap();
		let (lists, _) =
			list_reading_lists(&backend, &auth_user(&other), true, UserParams::parse(""))
				.await
				.unwrap();
		assert!(lists.is_empty(), "a private list is not another user's");
		assert!(
			reading_list_items_for(&backend, &auth_user(&other), created.id)
				.await
				.is_err()
		);
	}

	/// `DELETE /api/ReadingList?readingListId=` removes the caller's list and
	/// answers Kavita's plain-text body; an unknown id and a list the caller
	/// can see but does not own both answer the same way without deleting
	/// anything, which is what `kavita-ref` 0.9.1.4 does.
	#[tokio::test]
	async fn deleting_a_reading_list_is_owner_scoped_and_idempotent() {
		let conn = db().await;
		let owner_row = fake_data::User::new("owner").insert(&conn).await;
		let other_row = fake_data::User::new("other").insert(&conn).await;
		let owner = auth_user(&owner_row);
		let other = auth_user(&other_row);
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

		let created = create_reading_list_for(&backend, &owner, "Mine")
			.await
			.unwrap();
		update_by_series_for(&backend, &owner, created.id, series_id)
			.await
			.unwrap();

		assert_eq!(
			delete_reading_list_for(&backend, &other, created.id)
				.await
				.unwrap(),
			DELETED
		);
		assert_eq!(
			list_reading_lists(&backend, &owner, false, UserParams::parse(""))
				.await
				.unwrap()
				.0
				.len(),
			1,
			"a non-owner's delete is a no-op, not a deletion"
		);

		assert_eq!(
			delete_reading_list_for(&backend, &owner, created.id)
				.await
				.unwrap(),
			DELETED
		);
		assert!(
			list_reading_lists(&backend, &owner, false, UserParams::parse(""))
				.await
				.unwrap()
				.0
				.is_empty()
		);
		assert!(
			lists_for_series_for(&backend, &owner, series_id)
				.await
				.unwrap()
				.is_empty(),
			"the deleted list stops holding the series"
		);
		assert_eq!(
			delete_reading_list_for(&backend, &owner, created.id)
				.await
				.unwrap(),
			DELETED,
			"repeating the delete is not an error"
		);
		assert_eq!(
			delete_reading_list_for(&backend, &owner, 999_999)
				.await
				.unwrap(),
			DELETED
		);
	}

	/// Two lists that share a title — the shape the replay harness leaves
	/// behind — come back in a stable order from both `lists` and
	/// `lists-for-series`, so a client's refresh does not reshuffle them.
	#[tokio::test]
	async fn same_titled_lists_keep_a_stable_order() {
		let conn = db().await;
		let user_row = fake_data::User::new("lister").insert(&conn).await;
		let user = auth_user(&user_row);
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

		let mut ids = Vec::new();
		for _ in 0..3 {
			let list = create_reading_list_for(&backend, &user, "hurl replay list")
				.await
				.unwrap();
			update_by_series_for(&backend, &user, list.id, series_id)
				.await
				.unwrap();
			ids.push(list.id);
		}
		ids.sort_unstable();

		let (lists, _) =
			list_reading_lists(&backend, &user, false, UserParams::parse(""))
				.await
				.unwrap();
		assert_eq!(lists.iter().map(|list| list.id).collect::<Vec<_>>(), ids);
		let holding = lists_for_series_for(&backend, &user, series_id)
			.await
			.unwrap();
		assert_eq!(holding.iter().map(|list| list.id).collect::<Vec<_>>(), ids);
	}
}
