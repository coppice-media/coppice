//! Shelf projection: containers as Kobo `Tag` entitlements.
//!
//! A shelf is a container (`collections` ∪ `reading_lists`) whose
//! `kobo_shelf` flag is set. Its items are the media ids the device can
//! download — the caller names the media extensions its sync session
//! covers: for a collection, every non-deleted matching book in the member
//! series; for a reading list, the non-deleted matching items in list order.
//!
//! Visibility mirrors the surface that owns the container: collections have
//! no visibility column, so a shelf is visible to its creating user only;
//! reading lists reuse the standard reading-list RBAC (creator, PUBLIC with
//! an empty rule table, or SHARED with a rule) at reader role.

use std::collections::{HashMap, HashSet};

use chrono::{Duration, Utc};
use models::entity::{
	collection, collection_series, kobo_shelf_tombstone, media, reading_list,
	reading_list_item, user::AuthUser,
};
use sea_orm::{
	prelude::*, ColumnTrait, ConnectionTrait, EntityTrait, JoinType, QueryFilter,
	QueryOrder, QuerySelect,
};

type DbResult<T> = Result<T, DbErr>;

/// How long a shelf tombstone is kept before it is pruned. A device that does
/// not sync within this window misses the deletion until a full resync.
pub const KOBO_SHELF_TOMBSTONE_TTL: Duration = Duration::days(30);

/// Which canonical table backs a shelf id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
	Collection,
	ReadingList,
}

impl ContainerKind {
	/// Resolve a shelf id (== container id) to its backing table.
	pub async fn resolve<C: ConnectionTrait>(conn: &C, shelf_id: &str) -> DbResult<Self> {
		if collection::Entity::find_by_id(shelf_id)
			.one(conn)
			.await?
			.is_some()
		{
			return Ok(Self::Collection);
		}
		if reading_list::Entity::find_by_id(shelf_id)
			.one(conn)
			.await?
			.is_some()
		{
			return Ok(Self::ReadingList);
		}
		Err(DbErr::RecordNotFound(format!(
			"No shelf with id {shelf_id}"
		)))
	}
}

/// A container projected as a Kobo shelf.
#[derive(Debug, Clone, PartialEq)]
pub struct ShelfProjection {
	/// The shelf id; identical to the container id.
	pub shelf_id: String,
	pub name: String,
	pub last_modified: DateTimeWithTimeZone,
	/// Non-deleted media ids on the shelf matching the requested extensions,
	/// in shelf order.
	pub items: Vec<String>,
}

/// The shelf changes a device should see relative to its previous sync.
#[derive(Debug, Default)]
pub struct ShelfSyncDelta {
	/// Every shelf, on a full sync. Incremental syncs cannot distinguish a
	/// created container from an updated one (containers carry only
	/// `updated_at`), so new shelves arrive as `changed` there — devices
	/// accept `ChangedTag` for a shelf they do not have yet.
	pub new: Vec<ShelfProjection>,
	/// Shelves whose `updated_at` falls inside the incremental window.
	pub changed: Vec<ShelfProjection>,
	/// Shelf ids deleted inside the incremental window (tombstones).
	pub deleted_ids: Vec<String>,
}

/// All shelves the user should have on the device, with the items whose
/// media extension is in `extensions` resolved.
pub async fn shelves_for_user<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	extensions: &[&str],
) -> DbResult<Vec<ShelfProjection>> {
	let collection_shelves = collection::Entity::find()
		.filter(collection::Column::KoboShelf.eq(true))
		.filter(collection::Column::CreatingUserId.eq(user.id.clone()))
		.order_by_asc(collection::Column::Name)
		.all(conn)
		.await?;

	// The RBAC query left-joins the rule table, so a shared list with several
	// rules comes back once per rule; keep the first occurrence.
	let mut seen = HashSet::new();
	let reading_list_shelves: Vec<reading_list::Model> =
		reading_list::Entity::find_for_user(user, 1)
			.filter(reading_list::Column::KoboShelf.eq(true))
			.order_by_asc(reading_list::Column::Name)
			.all(conn)
			.await?
			.into_iter()
			.filter(|model| seen.insert(model.id.clone()))
			.collect();

	let mut shelves =
		Vec::with_capacity(collection_shelves.len() + reading_list_shelves.len());

	let collection_ids: Vec<String> = collection_shelves
		.iter()
		.map(|model| model.id.clone())
		.collect();
	let collection_items = collection_items(conn, &collection_ids, extensions).await?;

	for model in collection_shelves {
		let items = collection_items.get(&model.id).cloned().unwrap_or_default();
		shelves.push(ShelfProjection {
			shelf_id: model.id,
			name: model.name,
			last_modified: model.updated_at,
			items,
		});
	}

	let reading_list_ids: Vec<String> = reading_list_shelves
		.iter()
		.map(|model| model.id.clone())
		.collect();
	let reading_list_items =
		reading_list_items(conn, &reading_list_ids, extensions).await?;

	for model in reading_list_shelves {
		let items = reading_list_items
			.get(&model.id)
			.cloned()
			.unwrap_or_default();
		shelves.push(ShelfProjection {
			shelf_id: model.id,
			name: model.name,
			last_modified: model.updated_at,
			items,
		});
	}

	shelves.sort_by(|a, b| {
		a.name
			.cmp(&b.name)
			.then_with(|| a.shelf_id.cmp(&b.shelf_id))
	});
	Ok(shelves)
}

/// The shelf delta for a sync session covering the given media extensions.
/// Tombstones outside the window are pruned opportunistically so the table
/// does not grow unbounded.
pub async fn shelf_sync_delta<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	previous_sync_at: Option<DateTimeWithTimeZone>,
	extensions: &[&str],
) -> DbResult<ShelfSyncDelta> {
	prune_tombstones(conn).await?;

	let shelves = shelves_for_user(conn, user, extensions).await?;
	let mut delta = ShelfSyncDelta::default();

	match previous_sync_at {
		None => delta.new = shelves,
		Some(previous_sync_at) => {
			for shelf in shelves {
				if shelf.last_modified >= previous_sync_at {
					delta.changed.push(shelf);
				}
			}

			delta.deleted_ids = kobo_shelf_tombstone::Entity::find()
				.filter(kobo_shelf_tombstone::Column::UserId.eq(user.id.clone()))
				.filter(kobo_shelf_tombstone::Column::DeletedAt.gte(previous_sync_at))
				.order_by_asc(kobo_shelf_tombstone::Column::DeletedAt)
				.all(conn)
				.await?
				.into_iter()
				.map(|tombstone| tombstone.shelf_id)
				.collect();
		},
	}

	Ok(delta)
}

async fn prune_tombstones<C: ConnectionTrait>(conn: &C) -> DbResult<()> {
	let cutoff: DateTimeWithTimeZone = Utc::now()
		.checked_sub_signed(KOBO_SHELF_TOMBSTONE_TTL)
		.ok_or_else(|| DbErr::Custom("Could not compute tombstone cutoff".to_owned()))?
		.into();

	kobo_shelf_tombstone::Entity::delete_many()
		.filter(kobo_shelf_tombstone::Column::DeletedAt.lt(cutoff))
		.exec(conn)
		.await?;
	Ok(())
}

/// Matching media ids for each collection, ordered by membership order.
async fn collection_items<C: ConnectionTrait>(
	conn: &C,
	collection_ids: &[String],
	extensions: &[&str],
) -> DbResult<HashMap<String, Vec<String>>> {
	if collection_ids.is_empty() {
		return Ok(HashMap::new());
	}

	let rows: Vec<(String, String, i32)> = collection_series::Entity::find()
		.filter(collection_series::Column::CollectionId.is_in(collection_ids.to_vec()))
		.join(
			JoinType::InnerJoin,
			collection_series::Relation::Series.def(),
		)
		.join(JoinType::InnerJoin, media::Relation::Series.def().rev())
		.filter(media::Column::Extension.is_in(extensions.iter().copied()))
		.filter(media::Column::DeletedAt.is_null())
		.select_only()
		.column(collection_series::Column::CollectionId)
		.column(media::Column::Id)
		.column(collection_series::Column::DisplayOrder)
		.order_by_asc(collection_series::Column::DisplayOrder)
		.order_by_asc(media::Column::Name)
		.into_tuple()
		.all(conn)
		.await?;

	let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
	for (collection_id, media_id, _) in rows {
		grouped.entry(collection_id).or_default().push(media_id);
	}
	Ok(grouped)
}

/// Matching media ids for each reading list, ordered by list order.
async fn reading_list_items<C: ConnectionTrait>(
	conn: &C,
	reading_list_ids: &[String],
	extensions: &[&str],
) -> DbResult<HashMap<String, Vec<String>>> {
	if reading_list_ids.is_empty() {
		return Ok(HashMap::new());
	}

	let rows: Vec<(String, String, i32)> = reading_list_item::Entity::find()
		.filter(reading_list_item::Column::ReadingListId.is_in(reading_list_ids.to_vec()))
		.join(
			JoinType::InnerJoin,
			reading_list_item::Relation::Media.def(),
		)
		.filter(media::Column::Extension.is_in(extensions.iter().copied()))
		.filter(media::Column::DeletedAt.is_null())
		.select_only()
		.column(reading_list_item::Column::ReadingListId)
		.column(media::Column::Id)
		.column(reading_list_item::Column::DisplayOrder)
		.order_by_asc(reading_list_item::Column::DisplayOrder)
		.into_tuple()
		.all(conn)
		.await?;

	let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
	for (reading_list_id, media_id, _) in rows {
		grouped.entry(reading_list_id).or_default().push(media_id);
	}
	Ok(grouped)
}

/// The series a set of books belongs to, preserving input order, without
/// duplicates or books without a series.
pub(crate) async fn series_of_books<C: ConnectionTrait>(
	conn: &C,
	book_ids: &[String],
) -> DbResult<Vec<String>> {
	let series_ids: Vec<String> = media::Entity::find()
		.filter(media::Column::Id.is_in(book_ids.to_vec()))
		.filter(media::Column::SeriesId.is_not_null())
		.select_only()
		.column(media::Column::SeriesId)
		.into_tuple()
		.all(conn)
		.await?;

	let mut seen = HashSet::with_capacity(series_ids.len());
	let mut ordered = Vec::with_capacity(series_ids.len());
	for series_id in series_ids {
		if seen.insert(series_id.clone()) {
			ordered.push(series_id);
		}
	}
	Ok(ordered)
}

/// Current member ids of a container, in display order: series ids for a
/// collection, media ids for a reading list. Used to echo events.
pub(crate) async fn container_member_ids<C: ConnectionTrait>(
	conn: &C,
	kind: ContainerKind,
	id: &str,
) -> DbResult<Vec<String>> {
	match kind {
		ContainerKind::Collection => collection_series::Entity::find()
			.filter(collection_series::Column::CollectionId.eq(id.to_owned()))
			.order_by_asc(collection_series::Column::DisplayOrder)
			.all(conn)
			.await
			.map(|members| members.into_iter().map(|m| m.series_id).collect()),
		ContainerKind::ReadingList => reading_list_item::Entity::find()
			.filter(reading_list_item::Column::ReadingListId.eq(id.to_owned()))
			.order_by_asc(reading_list_item::Column::DisplayOrder)
			.all(conn)
			.await
			.map(|items| items.into_iter().map(|item| item.media_id).collect()),
	}
}
