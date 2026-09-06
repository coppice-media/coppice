//! Canonical container mutations.
//!
//! Every create/rename/reorder/delete of a collection or reading list goes
//! through these functions so that all protocol surfaces (native GraphQL,
//! Komga compatibility CRUD, Kobo `tags` write-back) share one set of rules:
//!
//! - membership validation against the actor's visible series/books,
//! - owner checks: Komga-native mutations require the creating user (the
//!   pre-existing Komga route rule); Kobo device writes additionally allow
//!   the server owner (the device user manages their own shelves),
//! - `updated_at` bumps on every write, which is what incremental Kobo syncs
//!   key their `ChangedTag` emission on,
//! - `source_device` provenance: set to the device name on Kobo write-back,
//!   cleared to `None` on native name writes (last writer wins on `name`),
//! - a [`CoreEvent`](stump_core::event::CoreEvent) per mutation, emitted only
//!   after the transaction commits.

use std::collections::HashSet;

use chrono::Utc;
use models::entity::{
	collection, collection_series, kobo_shelf_tombstone, media, reading_list,
	reading_list_item, series, user::AuthUser,
};
use models::txn::begin_write;
use sea_orm::{
	ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr,
	EntityTrait, IntoActiveModel, QueryFilter, QuerySelect, Set,
};
use uuid::Uuid;

use stump_core::{
	error::{CoreError, CoreResult},
	event::{
		CollectionAdded, CollectionChanged, CollectionDeleted, CoreEvent, ReadListAdded,
		ReadListChanged, ReadListDeleted,
	},
	Ctx,
};

use crate::shelf::{container_member_ids, series_of_books, ContainerKind};

/// Kobo shelf `TagType`; stock firmware treats shelves as manual tags.
pub const KOBO_SHELF_TAG_TYPE: &str = "Manual";

fn validate_name(name: &str) -> CoreResult<String> {
	let name = name.trim();
	if name.is_empty() {
		return Err(CoreError::BadRequest("Name must not be blank".to_owned()));
	}
	Ok(name.to_owned())
}

fn reject_duplicates(ids: &[String], label: &str) -> CoreResult<()> {
	let mut seen = HashSet::with_capacity(ids.len());
	for id in ids {
		if !seen.insert(id) {
			return Err(CoreError::BadRequest(format!(
				"{label} must not contain duplicate ids"
			)));
		}
	}
	Ok(())
}

/// Member ids of a container after a mutation, used for change events.
#[derive(Debug, Clone)]
pub struct ContainerMembers {
	pub id: String,
	pub member_ids: Vec<String>,
}

/// Request payload for creating a collection.
#[derive(Debug, Clone)]
pub struct CollectionCreate {
	pub name: String,
	pub ordered: bool,
	pub series_ids: Vec<String>,
}

/// Request payload for updating a collection. `None` leaves the field
/// untouched; `Some` replaces it. Passing `Some(vec![])` empties membership.
#[derive(Debug, Default, Clone)]
pub struct CollectionUpdate {
	pub name: Option<String>,
	pub ordered: Option<bool>,
	pub series_ids: Option<Vec<String>>,
}

/// Request payload for creating a reading list.
#[derive(Debug, Clone)]
pub struct ReadListCreate {
	pub name: String,
	pub summary: Option<String>,
	pub ordered: bool,
	pub book_ids: Vec<String>,
}

/// Request payload for updating a reading list.
#[derive(Debug, Default, Clone)]
pub struct ReadListUpdate {
	pub name: Option<String>,
	pub summary: Option<Option<String>>,
	pub ordered: Option<bool>,
	pub book_ids: Option<Vec<String>>,
}

/// Creates a collection with validated visible members and emits
/// [`CollectionAdded`].
pub async fn create_collection(
	ctx: &Ctx,
	user: &AuthUser,
	input: CollectionCreate,
) -> CoreResult<collection::Model> {
	let name = validate_name(&input.name)?;
	reject_duplicates(&input.series_ids, "seriesIds")?;
	ensure_series_visible(ctx.conn.as_ref(), user, &input.series_ids).await?;

	let txn = begin_write(&ctx.conn).await?;
	let result = async {
		let model = collection::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			name: Set(name),
			ordered: Set(input.ordered),
			updated_at: Set(Utc::now().into()),
			kobo_shelf: Set(true),
			creating_user_id: Set(user.id.clone()),
			..Default::default()
		}
		.insert(&txn)
		.await?;

		replace_collection_members(&txn, &model.id, &input.series_ids).await?;
		Ok::<_, CoreError>(model)
	}
	.await;
	let model = commit(txn, result).await?;

	ctx.emit_event(CoreEvent::CollectionAdded(CollectionAdded {
		id: model.id.clone(),
		series_ids: input.series_ids,
	}));

	Ok(model)
}

/// Updates name, ordering, and/or membership of a collection and emits
/// [`CollectionChanged`]. Only the creating user may update a collection.
pub async fn update_collection(
	ctx: &Ctx,
	user: &AuthUser,
	id: &str,
	input: CollectionUpdate,
) -> CoreResult<ContainerMembers> {
	let txn = begin_write(&ctx.conn).await?;
	let result = apply_collection_update(&txn, user, id, &input).await;
	let member_ids = commit(txn, result).await?;

	ctx.emit_event(CoreEvent::CollectionChanged(CollectionChanged {
		id: id.to_owned(),
		series_ids: member_ids.clone(),
	}));

	Ok(ContainerMembers {
		id: id.to_owned(),
		member_ids,
	})
}

async fn apply_collection_update(
	txn: &DatabaseTransaction,
	user: &AuthUser,
	id: &str,
	input: &CollectionUpdate,
) -> CoreResult<Vec<String>> {
	let model = collection::Entity::find_by_id(id.to_owned())
		.one(txn)
		.await?
		.ok_or_else(|| CoreError::NotFound(format!("Collection {id} not found")))?;
	if model.creating_user_id != user.id {
		return Err(CoreError::Forbidden(
			"Only the collection owner may update it".to_owned(),
		));
	}

	if let Some(series_ids) = &input.series_ids {
		reject_duplicates(series_ids, "seriesIds")?;
		ensure_series_visible(txn, user, series_ids).await?;
	}

	let mut active = model.into_active_model();
	if let Some(name) = &input.name {
		active.name = Set(validate_name(name)?);
		active.source_device = Set(None);
	}
	if let Some(ordered) = input.ordered {
		active.ordered = Set(ordered);
	}
	active.updated_at = Set(Utc::now().into());
	active.update(txn).await?;

	if let Some(series_ids) = &input.series_ids {
		replace_collection_members(txn, id, series_ids).await?;
	}

	Ok(container_member_ids(txn, ContainerKind::Collection, id).await?)
}

/// Deletes a collection (and, when it was projected to Kobo, records a
/// tombstone so devices emit `DeletedTag`) and emits [`CollectionDeleted`].
pub async fn delete_collection(ctx: &Ctx, user: &AuthUser, id: &str) -> CoreResult<()> {
	let txn = begin_write(&ctx.conn).await?;
	let result = async {
		let model = collection::Entity::find_by_id(id.to_owned())
			.one(&txn)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("Collection {id} not found")))?;
		if model.creating_user_id != user.id {
			return Err(CoreError::Forbidden(
				"Only the collection owner may delete it".to_owned(),
			));
		}

		let member_ids =
			container_member_ids(&txn, ContainerKind::Collection, id).await?;
		if model.kobo_shelf {
			record_shelf_tombstone(&txn, &model.creating_user_id, id).await?;
		}
		collection::Entity::delete_by_id(id.to_owned())
			.exec(&txn)
			.await?;
		Ok::<_, CoreError>(member_ids)
	}
	.await;
	let member_ids = commit(txn, result).await?;

	ctx.emit_event(CoreEvent::CollectionDeleted(CollectionDeleted {
		id: id.to_owned(),
		series_ids: member_ids,
	}));
	Ok(())
}

/// Creates a reading list with validated visible members and emits
/// [`ReadListAdded`]. `summary` is stored in `description`.
pub async fn create_read_list(
	ctx: &Ctx,
	user: &AuthUser,
	input: ReadListCreate,
) -> CoreResult<reading_list::Model> {
	let name = validate_name(&input.name)?;
	reject_duplicates(&input.book_ids, "bookIds")?;
	ensure_books_visible(ctx.conn.as_ref(), user, &input.book_ids).await?;

	let txn = begin_write(&ctx.conn).await?;
	let result = async {
		let model = reading_list::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			name: Set(name),
			description: Set(input.summary),
			updated_at: Set(Utc::now().into()),
			visibility: Set("PRIVATE".to_string()),
			ordering: Set(if input.ordered {
				"MANUAL".to_string()
			} else {
				"CREATED_AT".to_string()
			}),
			kobo_shelf: Set(true),
			creating_user_id: Set(user.id.clone()),
			..Default::default()
		}
		.insert(&txn)
		.await?;

		replace_reading_list_members(&txn, &model.id, &input.book_ids).await?;
		Ok::<_, CoreError>(model)
	}
	.await;
	let model = commit(txn, result).await?;

	ctx.emit_event(CoreEvent::ReadListAdded(ReadListAdded {
		id: model.id.clone(),
		book_ids: input.book_ids,
	}));

	Ok(model)
}

/// Updates name, summary, ordering, and/or membership of a reading list and
/// emits [`ReadListChanged`]. Only the creating user may update a list.
pub async fn update_read_list(
	ctx: &Ctx,
	user: &AuthUser,
	id: &str,
	input: ReadListUpdate,
) -> CoreResult<ContainerMembers> {
	let txn = begin_write(&ctx.conn).await?;
	let result = apply_read_list_update(&txn, user, id, &input).await;
	let member_ids = commit(txn, result).await?;

	ctx.emit_event(CoreEvent::ReadListChanged(ReadListChanged {
		id: id.to_owned(),
		book_ids: member_ids.clone(),
	}));

	Ok(ContainerMembers {
		id: id.to_owned(),
		member_ids,
	})
}

async fn apply_read_list_update(
	txn: &DatabaseTransaction,
	user: &AuthUser,
	id: &str,
	input: &ReadListUpdate,
) -> CoreResult<Vec<String>> {
	let model = reading_list::Entity::find_by_id(id.to_owned())
		.one(txn)
		.await?
		.ok_or_else(|| CoreError::NotFound(format!("Reading list {id} not found")))?;
	if model.creating_user_id != user.id {
		return Err(CoreError::Forbidden(
			"Only the reading list owner may update it".to_owned(),
		));
	}

	if let Some(book_ids) = &input.book_ids {
		reject_duplicates(book_ids, "bookIds")?;
		ensure_books_visible(txn, user, book_ids).await?;
	}

	let mut active = model.into_active_model();
	if let Some(name) = &input.name {
		active.name = Set(validate_name(name)?);
		active.source_device = Set(None);
	}
	if let Some(summary) = &input.summary {
		active.description = Set(summary.clone());
	}
	if let Some(ordered) = input.ordered {
		active.ordering = Set(if ordered {
			"MANUAL".to_string()
		} else {
			"CREATED_AT".to_string()
		});
	}
	active.updated_at = Set(Utc::now().into());
	active.update(txn).await?;

	if let Some(book_ids) = &input.book_ids {
		replace_reading_list_members(txn, id, book_ids).await?;
	}

	Ok(container_member_ids(txn, ContainerKind::ReadingList, id).await?)
}

/// Deletes a reading list (with a Kobo tombstone when projected) and emits
/// [`ReadListDeleted`].
pub async fn delete_read_list(ctx: &Ctx, user: &AuthUser, id: &str) -> CoreResult<()> {
	let txn = begin_write(&ctx.conn).await?;
	let result = async {
		let model = reading_list::Entity::find_by_id(id.to_owned())
			.one(&txn)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("Reading list {id} not found")))?;
		if model.creating_user_id != user.id {
			return Err(CoreError::Forbidden(
				"Only the reading list owner may delete it".to_owned(),
			));
		}

		let member_ids =
			container_member_ids(&txn, ContainerKind::ReadingList, id).await?;
		if model.kobo_shelf {
			record_shelf_tombstone(&txn, &model.creating_user_id, id).await?;
		}
		reading_list::Entity::delete_by_id(id.to_owned())
			.exec(&txn)
			.await?;
		Ok::<_, CoreError>(member_ids)
	}
	.await;
	let member_ids = commit(txn, result).await?;

	ctx.emit_event(CoreEvent::ReadListDeleted(ReadListDeleted {
		id: id.to_owned(),
		book_ids: member_ids,
	}));
	Ok(())
}

/// Materializes a device-created shelf as a canonical collection.
///
/// Kobo shelves are book sets; Stump's shelf contract projects them onto
/// collections, whose members are series. The device-supplied shelf id is
/// honored when it parses as a UUID so the shelf id is stable from the first
/// write; otherwise a fresh UUID is minted and echoed. Creation is
/// idempotent the way Calibre-Web's `HandleTagCreate` is: an existing shelf
/// of this user with the same id, or the same name, is returned as-is.
pub async fn create_device_shelf(
	ctx: &Ctx,
	user: &AuthUser,
	shelf_id: Option<String>,
	name: String,
	device: Option<String>,
) -> CoreResult<collection::Model> {
	let name = validate_name(&name)?;
	let shelf_id = match shelf_id {
		Some(id) if Uuid::parse_str(&id).is_ok() => id,
		_ => Uuid::new_v4().to_string(),
	};

	if let Some(existing) = collection::Entity::find_by_id(shelf_id.clone())
		.one(ctx.conn.as_ref())
		.await?
	{
		if existing.creating_user_id == user.id {
			return Ok(existing);
		}
		return Err(CoreError::Forbidden(
			"A shelf with this id already exists".to_owned(),
		));
	}

	if let Some(existing) = collection::Entity::find()
		.filter(collection::Column::CreatingUserId.eq(user.id.clone()))
		.filter(collection::Column::KoboShelf.eq(true))
		.filter(collection::Column::Name.eq(name.clone()))
		.one(ctx.conn.as_ref())
		.await?
	{
		return Ok(existing);
	}

	let model = collection::ActiveModel {
		id: Set(shelf_id),
		name: Set(name),
		ordered: Set(false),
		updated_at: Set(Utc::now().into()),
		kobo_shelf: Set(true),
		source_device: Set(device),
		creating_user_id: Set(user.id.clone()),
		..Default::default()
	}
	.insert(ctx.conn.as_ref())
	.await?;

	ctx.emit_event(CoreEvent::CollectionAdded(CollectionAdded {
		id: model.id.clone(),
		series_ids: Vec::new(),
	}));

	Ok(model)
}

/// Renames a shelf (collection or reading list) on behalf of a device. The
/// write is last-writer-wins: the name replaces the current one and the
/// device provenance is recorded.
pub async fn rename_shelf(
	ctx: &Ctx,
	user: &AuthUser,
	shelf_id: &str,
	name: String,
	device: Option<String>,
) -> CoreResult<()> {
	let name = validate_name(&name)?;
	let txn = begin_write(&ctx.conn).await?;
	let result = rename_shelf_txn(&txn, user, shelf_id, name, device).await;
	commit(txn, result).await
}

async fn rename_shelf_txn(
	txn: &DatabaseTransaction,
	user: &AuthUser,
	shelf_id: &str,
	name: String,
	device: Option<String>,
) -> CoreResult<()> {
	let kind = resolve_shelf(txn, shelf_id).await?;
	let owner = shelf_owner_id(txn, kind, shelf_id).await?;
	assert_shelf_owner(user, &owner)?;
	match kind {
		ContainerKind::Collection => {
			touch_collection(txn, shelf_id, Some(name), device).await
		},
		ContainerKind::ReadingList => {
			touch_reading_list(txn, shelf_id, Some(name), device).await
		},
	}
}

/// Adds books to a shelf. For a collection-backed shelf each book maps to its
/// series and the series becomes a member; for a reading list the books
/// become items. Books the actor cannot see, or books without a series on a
/// collection shelf, are rejected. Already-present members are skipped.
pub async fn add_shelf_items(
	ctx: &Ctx,
	user: &AuthUser,
	shelf_id: &str,
	book_ids: Vec<String>,
	device: Option<String>,
) -> CoreResult<()> {
	reject_duplicates(&book_ids, "bookIds")?;
	ensure_books_visible(ctx.conn.as_ref(), user, &book_ids).await?;

	let txn = begin_write(&ctx.conn).await?;
	let result = add_shelf_items_txn(&txn, user, shelf_id, &book_ids, device).await;
	commit(txn, result).await
}

async fn add_shelf_items_txn(
	txn: &DatabaseTransaction,
	user: &AuthUser,
	shelf_id: &str,
	book_ids: &[String],
	device: Option<String>,
) -> CoreResult<()> {
	let kind = resolve_shelf(txn, shelf_id).await?;
	let owner = shelf_owner_id(txn, kind, shelf_id).await?;
	assert_shelf_owner(user, &owner)?;

	let existing: HashSet<String> = container_member_ids(txn, kind, shelf_id)
		.await?
		.into_iter()
		.collect();
	let mut order = existing.len() as i32;
	let mut next_order = || {
		let current = order;
		order += 1;
		current
	};

	match kind {
		ContainerKind::Collection => {
			let series_ids = series_of_books(txn, book_ids).await?;
			if series_ids.len() != book_ids.len() {
				return Err(CoreError::BadRequest(
					"Every book on a collection shelf must belong to a series".to_owned(),
				));
			}
			let members: Vec<collection_series::ActiveModel> = series_ids
				.into_iter()
				.filter(|series_id| !existing.contains(series_id))
				.map(|series_id| collection_series::ActiveModel {
					collection_id: Set(shelf_id.to_owned()),
					series_id: Set(series_id),
					display_order: Set(next_order()),
					..Default::default()
				})
				.collect();
			if !members.is_empty() {
				collection_series::Entity::insert_many(members)
					.exec(txn)
					.await?;
			}
			touch_collection(txn, shelf_id, None, device).await
		},
		ContainerKind::ReadingList => {
			let items: Vec<reading_list_item::ActiveModel> = book_ids
				.iter()
				.filter(|book_id| !existing.contains(*book_id))
				.map(|book_id| reading_list_item::ActiveModel {
					reading_list_id: Set(shelf_id.to_owned()),
					media_id: Set(book_id.clone()),
					display_order: Set(next_order()),
					..Default::default()
				})
				.collect();
			if !items.is_empty() {
				reading_list_item::Entity::insert_many(items)
					.exec(txn)
					.await?;
			}
			touch_reading_list(txn, shelf_id, None, device).await
		},
	}
}

/// Removes books from a shelf. Removing a book from a collection-backed shelf
/// removes its series membership — the shelf loses the whole series, since
/// collections cannot express per-book membership. Reading-list shelves lose
/// exactly the listed books.
pub async fn remove_shelf_items(
	ctx: &Ctx,
	user: &AuthUser,
	shelf_id: &str,
	book_ids: Vec<String>,
	device: Option<String>,
) -> CoreResult<()> {
	let txn = begin_write(&ctx.conn).await?;
	let result = remove_shelf_items_txn(&txn, user, shelf_id, book_ids, device).await;
	commit(txn, result).await
}

async fn remove_shelf_items_txn(
	txn: &DatabaseTransaction,
	user: &AuthUser,
	shelf_id: &str,
	book_ids: Vec<String>,
	device: Option<String>,
) -> CoreResult<()> {
	let kind = resolve_shelf(txn, shelf_id).await?;
	let owner = shelf_owner_id(txn, kind, shelf_id).await?;
	assert_shelf_owner(user, &owner)?;

	match kind {
		ContainerKind::Collection => {
			let series_ids = series_of_books(txn, &book_ids).await?;
			if !series_ids.is_empty() {
				collection_series::Entity::delete_many()
					.filter(
						collection_series::Column::CollectionId.eq(shelf_id.to_owned()),
					)
					.filter(collection_series::Column::SeriesId.is_in(series_ids))
					.exec(txn)
					.await?;
			}
			touch_collection(txn, shelf_id, None, device).await
		},
		ContainerKind::ReadingList => {
			if !book_ids.is_empty() {
				reading_list_item::Entity::delete_many()
					.filter(
						reading_list_item::Column::ReadingListId.eq(shelf_id.to_owned()),
					)
					.filter(reading_list_item::Column::MediaId.is_in(book_ids))
					.exec(txn)
					.await?;
			}
			touch_reading_list(txn, shelf_id, None, device).await
		},
	}
}

/// Deletes a shelf (collection or reading list) on behalf of a device,
/// recording the tombstone and emitting the corresponding deleted event.
pub async fn delete_shelf(ctx: &Ctx, user: &AuthUser, shelf_id: &str) -> CoreResult<()> {
	let txn = begin_write(&ctx.conn).await?;
	let result = delete_shelf_txn(&txn, user, shelf_id).await;
	let (kind, member_ids) = commit(txn, result).await?;

	match kind {
		ContainerKind::Collection => {
			ctx.emit_event(CoreEvent::CollectionDeleted(CollectionDeleted {
				id: shelf_id.to_owned(),
				series_ids: member_ids,
			}));
		},
		ContainerKind::ReadingList => {
			ctx.emit_event(CoreEvent::ReadListDeleted(ReadListDeleted {
				id: shelf_id.to_owned(),
				book_ids: member_ids,
			}));
		},
	}
	Ok(())
}

async fn delete_shelf_txn(
	txn: &DatabaseTransaction,
	user: &AuthUser,
	shelf_id: &str,
) -> CoreResult<(ContainerKind, Vec<String>)> {
	let kind = resolve_shelf(txn, shelf_id).await?;
	let member_ids = container_member_ids(txn, kind, shelf_id).await?;
	let owner = shelf_owner_id(txn, kind, shelf_id).await?;
	assert_shelf_owner(user, &owner)?;
	record_shelf_tombstone(txn, &owner, shelf_id).await?;

	match kind {
		ContainerKind::Collection => {
			collection::Entity::delete_by_id(shelf_id.to_owned())
				.exec(txn)
				.await?;
		},
		ContainerKind::ReadingList => {
			reading_list::Entity::delete_by_id(shelf_id.to_owned())
				.exec(txn)
				.await?;
		},
	}
	Ok((kind, member_ids))
}

/// Resolves a shelf id to its backing table, surfacing an unknown id as a
/// [`CoreError::NotFound`] rather than a database error.
async fn resolve_shelf(
	txn: &DatabaseTransaction,
	shelf_id: &str,
) -> CoreResult<ContainerKind> {
	ContainerKind::resolve(txn, shelf_id)
		.await
		.map_err(|error| match error {
			DbErr::RecordNotFound(message) => CoreError::NotFound(message),
			other => CoreError::DBError(other),
		})
}

async fn shelf_owner_id(
	txn: &DatabaseTransaction,
	kind: ContainerKind,
	shelf_id: &str,
) -> CoreResult<String> {
	match kind {
		ContainerKind::Collection => {
			Ok(collection::Entity::find_by_id(shelf_id.to_owned())
				.one(txn)
				.await?
				.ok_or_else(|| {
					CoreError::NotFound(format!("Shelf {shelf_id} not found"))
				})?
				.creating_user_id)
		},
		ContainerKind::ReadingList => {
			Ok(reading_list::Entity::find_by_id(shelf_id.to_owned())
				.one(txn)
				.await?
				.ok_or_else(|| {
					CoreError::NotFound(format!("Shelf {shelf_id} not found"))
				})?
				.creating_user_id)
		},
	}
}

/// Device write rule: the creating user, or the server owner.
fn assert_shelf_owner(user: &AuthUser, creating_user_id: &str) -> CoreResult<()> {
	if creating_user_id != user.id && !user.is_server_owner {
		return Err(CoreError::Forbidden(
			"Only the shelf owner may write to it".to_owned(),
		));
	}
	Ok(())
}

async fn touch_collection(
	txn: &DatabaseTransaction,
	id: &str,
	name: Option<String>,
	device: Option<String>,
) -> CoreResult<()> {
	let model = collection::Entity::find_by_id(id.to_owned())
		.one(txn)
		.await?
		.ok_or_else(|| CoreError::NotFound(format!("Collection {id} not found")))?;
	let mut active = model.into_active_model();
	if let Some(name) = name {
		active.name = Set(name);
	}
	active.source_device = Set(device);
	active.updated_at = Set(Utc::now().into());
	active.update(txn).await?;
	Ok(())
}

async fn touch_reading_list(
	txn: &DatabaseTransaction,
	id: &str,
	name: Option<String>,
	device: Option<String>,
) -> CoreResult<()> {
	let model = reading_list::Entity::find_by_id(id.to_owned())
		.one(txn)
		.await?
		.ok_or_else(|| CoreError::NotFound(format!("Reading list {id} not found")))?;
	let mut active = model.into_active_model();
	if let Some(name) = name {
		active.name = Set(name);
	}
	active.source_device = Set(device);
	active.updated_at = Set(Utc::now().into());
	active.update(txn).await?;
	Ok(())
}

async fn record_shelf_tombstone(
	txn: &DatabaseTransaction,
	user_id: &str,
	shelf_id: &str,
) -> CoreResult<()> {
	kobo_shelf_tombstone::ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		user_id: Set(user_id.to_owned()),
		shelf_id: Set(shelf_id.to_owned()),
		..Default::default()
	}
	.insert(txn)
	.await?;
	Ok(())
}

async fn ensure_series_visible(
	conn: &impl ConnectionTrait,
	user: &AuthUser,
	series_ids: &[String],
) -> CoreResult<()> {
	if series_ids.is_empty() {
		return Ok(());
	}
	let visible = series::Entity::find_for_user(user)
		.filter(series::Column::Id.is_in(series_ids.to_vec()))
		.select_only()
		.column(series::Column::Id)
		.into_tuple::<String>()
		.all(conn)
		.await?;
	if visible.len() != series_ids.len() {
		return Err(CoreError::NotFound(
			"One or more series were not found".to_owned(),
		));
	}
	Ok(())
}

async fn ensure_books_visible(
	conn: &impl ConnectionTrait,
	user: &AuthUser,
	book_ids: &[String],
) -> CoreResult<()> {
	if book_ids.is_empty() {
		return Ok(());
	}
	let visible = media::Entity::find_for_user(user)
		.filter(media::Column::Id.is_in(book_ids.to_vec()))
		.filter(media::Column::DeletedAt.is_null())
		.select_only()
		.column(media::Column::Id)
		.into_tuple::<String>()
		.all(conn)
		.await?;
	if visible.len() != book_ids.len() {
		return Err(CoreError::NotFound(
			"One or more books were not found".to_owned(),
		));
	}
	Ok(())
}

async fn replace_collection_members(
	txn: &DatabaseTransaction,
	collection_id: &str,
	series_ids: &[String],
) -> CoreResult<()> {
	collection_series::Entity::delete_many()
		.filter(collection_series::Column::CollectionId.eq(collection_id.to_owned()))
		.exec(txn)
		.await?;
	if !series_ids.is_empty() {
		let members: Vec<collection_series::ActiveModel> = series_ids
			.iter()
			.enumerate()
			.map(
				|(display_order, series_id)| collection_series::ActiveModel {
					collection_id: Set(collection_id.to_owned()),
					series_id: Set(series_id.clone()),
					display_order: Set(display_order as i32),
					..Default::default()
				},
			)
			.collect();
		collection_series::Entity::insert_many(members)
			.exec(txn)
			.await?;
	}
	Ok(())
}

async fn replace_reading_list_members(
	txn: &DatabaseTransaction,
	reading_list_id: &str,
	book_ids: &[String],
) -> CoreResult<()> {
	reading_list_item::Entity::delete_many()
		.filter(reading_list_item::Column::ReadingListId.eq(reading_list_id.to_owned()))
		.exec(txn)
		.await?;
	if !book_ids.is_empty() {
		let items: Vec<reading_list_item::ActiveModel> = book_ids
			.iter()
			.enumerate()
			.map(|(display_order, media_id)| reading_list_item::ActiveModel {
				reading_list_id: Set(reading_list_id.to_owned()),
				media_id: Set(media_id.clone()),
				display_order: Set(display_order as i32),
				..Default::default()
			})
			.collect();
		reading_list_item::Entity::insert_many(items)
			.exec(txn)
			.await?;
	}
	Ok(())
}

/// Commits a transaction whose inner work produced `result`, rolling back on
/// error and surfacing database failures as [`CoreError::DBError`].
async fn commit<T>(txn: DatabaseTransaction, result: CoreResult<T>) -> CoreResult<T> {
	match result {
		Ok(value) => {
			txn.commit().await.map_err(CoreError::DBError)?;
			Ok(value)
		},
		Err(error) => {
			if let Err(rollback_error) = txn.rollback().await {
				tracing::error!(
					?rollback_error,
					"Failed to roll back container mutation"
				);
			}
			Err(error)
		},
	}
}
