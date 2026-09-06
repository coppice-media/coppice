//! Behavioural tests for the canonical container service and its Kobo shelf
//! projection, against an in-memory SQLite database.

use ::tests::{db::test_database, fake_data};
use chrono::{Duration, Utc};
use models::entity::{
	collection, kobo_shelf_tombstone, media, reading_list, series, user::AuthUser,
};
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, ColumnTrait, DbConn, EntityTrait,
	IntoActiveModel, QueryFilter,
};
use tokio::sync::broadcast::Receiver;

use super::{
	add_shelf_items, create_collection, create_device_shelf, create_read_list,
	delete_collection, delete_shelf, remove_shelf_items, rename_shelf, shelf_sync_delta,
	shelves_for_user, update_collection, update_read_list, CollectionCreate,
	CollectionUpdate, ReadListCreate, ReadListUpdate,
};
use stump_core::{error::CoreError, event::CoreEvent, Ctx};

const EPUB: &[&str] = &["epub"];

struct Fixture {
	ctx: Ctx,
	owner: AuthUser,
	other: AuthUser,
	series_a: series::Model,
	series_b: series::Model,
	a1: media::Model,
	a2: media::Model,
	b1: media::Model,
}

impl Fixture {
	fn conn(&self) -> &DbConn {
		self.ctx.conn.as_ref()
	}

	fn events(&self) -> Receiver<CoreEvent> {
		self.ctx.get_client_receiver()
	}
}

async fn fixture() -> Fixture {
	let db = test_database().await;
	let owner = fake_data::User::new("owner").insert(&db).await;
	let other = fake_data::User::new("other").insert(&db).await;
	let library = fake_data::Library::default().insert(&db).await;
	let series_a = fake_data::Series {
		library_id: Some(library.id.clone()),
		name: Some("Series A".to_owned()),
		..Default::default()
	}
	.insert(&db)
	.await;
	let series_b = fake_data::Series {
		library_id: Some(library.id.clone()),
		name: Some("Series B".to_owned()),
		..Default::default()
	}
	.insert(&db)
	.await;
	let book = |series_id: String, name: &str| fake_data::Media {
		series_id,
		id: Some(name.to_owned()),
		name: Some(name.to_owned()),
		..Default::default()
	};
	let a1 = book(series_a.id.clone(), "a1").insert(&db).await;
	let a2 = book(series_a.id.clone(), "a2").insert(&db).await;
	let b1 = book(series_b.id.clone(), "b1").insert(&db).await;
	// A comic in series A must never surface on an EPUB-only shelf.
	fake_data::Media {
		series_id: series_a.id.clone(),
		id: Some("a-comic".to_owned()),
		name: Some("a-comic".to_owned()),
		extension: Some("cbz".to_owned()),
		..Default::default()
	}
	.insert(&db)
	.await;

	Fixture {
		ctx: Ctx::for_testing(db),
		owner: AuthUser {
			id: owner.id,
			..Default::default()
		},
		other: AuthUser {
			id: other.id,
			..Default::default()
		},
		series_a,
		series_b,
		a1,
		a2,
		b1,
	}
}

fn collection_input(name: &str, series_ids: Vec<String>) -> CollectionCreate {
	CollectionCreate {
		name: name.to_owned(),
		ordered: false,
		series_ids,
	}
}

async fn tombstones_for(
	conn: &DbConn,
	shelf_id: &str,
) -> Vec<kobo_shelf_tombstone::Model> {
	kobo_shelf_tombstone::Entity::find()
		.filter(kobo_shelf_tombstone::Column::ShelfId.eq(shelf_id))
		.all(conn)
		.await
		.unwrap()
}

async fn shelf_items(f: &Fixture, user: &AuthUser, shelf_id: &str) -> Vec<String> {
	shelves_for_user(f.conn(), user, EPUB)
		.await
		.unwrap()
		.into_iter()
		.find(|shelf| shelf.shelf_id == shelf_id)
		.map(|shelf| shelf.items)
		.expect("shelf is projected")
}

#[tokio::test]
async fn create_collection_validates_members_and_emits_added() {
	let f = fixture().await;
	let mut events = f.events();

	let blank = create_collection(&f.ctx, &f.owner, collection_input("  ", vec![])).await;
	assert!(matches!(blank, Err(CoreError::BadRequest(_))));

	let duplicate = create_collection(
		&f.ctx,
		&f.owner,
		collection_input("dup", vec![f.series_a.id.clone(), f.series_a.id.clone()]),
	)
	.await;
	assert!(matches!(duplicate, Err(CoreError::BadRequest(_))));

	let unknown = create_collection(
		&f.ctx,
		&f.owner,
		collection_input("ghost", vec!["missing-series".to_owned()]),
	)
	.await;
	assert!(matches!(unknown, Err(CoreError::NotFound(_))));
	assert!(events.try_recv().is_err(), "rejected creates emit nothing");

	let created = create_collection(
		&f.ctx,
		&f.owner,
		collection_input(
			" Favourites ",
			vec![f.series_b.id.clone(), f.series_a.id.clone()],
		),
	)
	.await
	.unwrap();
	assert_eq!(created.name, "Favourites");
	assert!(created.kobo_shelf);
	assert_eq!(created.source_device, None);

	match events.try_recv().unwrap() {
		CoreEvent::CollectionAdded(added) => {
			assert_eq!(added.id, created.id);
			assert_eq!(
				added.series_ids,
				vec![f.series_b.id.clone(), f.series_a.id.clone()]
			);
		},
		other => panic!("unexpected event {other:?}"),
	}

	// Membership order is the request order; the projection follows it.
	assert_eq!(
		shelf_items(&f, &f.owner, &created.id).await,
		vec!["b1", "a1", "a2"]
	);
}

#[tokio::test]
async fn update_collection_is_owner_only_and_replaces_membership() {
	let f = fixture().await;
	let created = create_device_shelf(
		&f.ctx,
		&f.owner,
		None,
		"From device".to_owned(),
		Some("Kobo Clara".to_owned()),
	)
	.await
	.unwrap();
	assert_eq!(created.source_device.as_deref(), Some("Kobo Clara"));

	let forbidden = update_collection(
		&f.ctx,
		&f.other,
		&created.id,
		CollectionUpdate {
			name: Some("hijack".to_owned()),
			..Default::default()
		},
	)
	.await;
	assert!(matches!(forbidden, Err(CoreError::Forbidden(_))));

	let mut events = f.events();
	let updated = update_collection(
		&f.ctx,
		&f.owner,
		&created.id,
		CollectionUpdate {
			name: Some("Renamed natively".to_owned()),
			ordered: Some(true),
			series_ids: Some(vec![f.series_a.id.clone()]),
		},
	)
	.await
	.unwrap();
	assert_eq!(updated.member_ids, vec![f.series_a.id.clone()]);
	assert!(matches!(
		events.try_recv().unwrap(),
		CoreEvent::CollectionChanged(changed) if changed.id == created.id
	));

	let stored = collection::Entity::find_by_id(created.id.clone())
		.one(f.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(stored.name, "Renamed natively");
	assert!(stored.ordered);
	assert_eq!(
		stored.source_device, None,
		"native name writes clear provenance"
	);
	assert!(stored.updated_at > created.updated_at);

	// `Some(vec![])` empties membership; `None` leaves it alone.
	update_collection(
		&f.ctx,
		&f.owner,
		&created.id,
		CollectionUpdate {
			series_ids: Some(vec![]),
			..Default::default()
		},
	)
	.await
	.unwrap();
	let untouched = update_collection(
		&f.ctx,
		&f.owner,
		&created.id,
		CollectionUpdate {
			ordered: Some(false),
			..Default::default()
		},
	)
	.await
	.unwrap();
	assert!(untouched.member_ids.is_empty());
}

#[tokio::test]
async fn deleting_a_projected_collection_leaves_a_tombstone() {
	let f = fixture().await;
	let projected = create_collection(
		&f.ctx,
		&f.owner,
		collection_input("projected", vec![f.series_a.id.clone()]),
	)
	.await
	.unwrap();
	let hidden = create_collection(&f.ctx, &f.owner, collection_input("hidden", vec![]))
		.await
		.unwrap();
	let mut hidden_row = hidden.clone().into_active_model();
	hidden_row.kobo_shelf = Set(false);
	hidden_row.update(f.conn()).await.unwrap();

	assert!(matches!(
		delete_collection(&f.ctx, &f.other, &projected.id).await,
		Err(CoreError::Forbidden(_))
	));

	let mut events = f.events();
	delete_collection(&f.ctx, &f.owner, &projected.id)
		.await
		.unwrap();
	delete_collection(&f.ctx, &f.owner, &hidden.id)
		.await
		.unwrap();

	match events.try_recv().unwrap() {
		CoreEvent::CollectionDeleted(deleted) => {
			assert_eq!(deleted.id, projected.id);
			assert_eq!(deleted.series_ids, vec![f.series_a.id.clone()]);
		},
		other => panic!("unexpected event {other:?}"),
	}
	assert_eq!(tombstones_for(f.conn(), &projected.id).await.len(), 1);
	assert!(tombstones_for(f.conn(), &hidden.id).await.is_empty());
	assert!(matches!(
		delete_collection(&f.ctx, &f.owner, &projected.id).await,
		Err(CoreError::NotFound(_))
	));
}

#[tokio::test]
async fn device_shelf_creation_is_idempotent_by_id_and_name() {
	let f = fixture().await;
	let shelf_id = "8f4b1e2a-3c5d-4e6f-8a9b-0c1d2e3f4a5b".to_owned();

	let created = create_device_shelf(
		&f.ctx,
		&f.owner,
		Some(shelf_id.clone()),
		"Beach reads".to_owned(),
		Some("Kobo Libra".to_owned()),
	)
	.await
	.unwrap();
	assert_eq!(created.id, shelf_id, "a UUID from the device is honoured");

	let by_id = create_device_shelf(
		&f.ctx,
		&f.owner,
		Some(shelf_id.clone()),
		"Beach reads".to_owned(),
		None,
	)
	.await
	.unwrap();
	assert_eq!(by_id, created);

	let by_name =
		create_device_shelf(&f.ctx, &f.owner, None, "Beach reads".to_owned(), None)
			.await
			.unwrap();
	assert_eq!(
		by_name.id, shelf_id,
		"same name for the same user reuses the shelf"
	);

	let minted = create_device_shelf(
		&f.ctx,
		&f.owner,
		Some("not-a-uuid".to_owned()),
		"Other".to_owned(),
		None,
	)
	.await
	.unwrap();
	assert_ne!(minted.id, "not-a-uuid");
	assert!(uuid::Uuid::parse_str(&minted.id).is_ok());

	assert!(matches!(
		create_device_shelf(&f.ctx, &f.other, Some(shelf_id), "Theirs".to_owned(), None)
			.await,
		Err(CoreError::Forbidden(_))
	));
}

#[tokio::test]
async fn collection_shelf_items_map_books_onto_series() {
	let f = fixture().await;
	let shelf = create_device_shelf(&f.ctx, &f.owner, None, "Shelf".to_owned(), None)
		.await
		.unwrap();

	add_shelf_items(
		&f.ctx,
		&f.owner,
		&shelf.id,
		vec![f.a1.id.clone(), f.b1.id.clone()],
		Some("Kobo".to_owned()),
	)
	.await
	.unwrap();
	assert_eq!(
		shelf_items(&f, &f.owner, &shelf.id).await,
		vec!["a1", "a2", "b1"],
		"the whole series joins, in membership order, EPUBs only"
	);

	// Re-adding a book of an already-present series is a no-op.
	add_shelf_items(&f.ctx, &f.owner, &shelf.id, vec![f.a2.id.clone()], None)
		.await
		.unwrap();
	let stored = collection::Entity::find_by_id(shelf.id.clone())
		.one(f.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(
		stored.source_device, None,
		"the last writer's provenance wins"
	);
	assert_eq!(shelf_items(&f, &f.owner, &shelf.id).await.len(), 3);

	assert!(matches!(
		add_shelf_items(
			&f.ctx,
			&f.owner,
			&shelf.id,
			vec!["missing".to_owned()],
			None
		)
		.await,
		Err(CoreError::NotFound(_))
	));
	assert!(matches!(
		add_shelf_items(&f.ctx, &f.other, &shelf.id, vec![f.b1.id.clone()], None).await,
		Err(CoreError::Forbidden(_))
	));
	assert!(matches!(
		add_shelf_items(
			&f.ctx,
			&f.owner,
			"no-such-shelf",
			vec![f.b1.id.clone()],
			None
		)
		.await,
		Err(CoreError::NotFound(_))
	));

	// Removing one book drops its whole series.
	remove_shelf_items(&f.ctx, &f.owner, &shelf.id, vec![f.a1.id.clone()], None)
		.await
		.unwrap();
	assert_eq!(shelf_items(&f, &f.owner, &shelf.id).await, vec!["b1"]);
}

#[tokio::test]
async fn reading_list_shelf_items_are_exact_books() {
	let f = fixture().await;
	let list = create_read_list(
		&f.ctx,
		&f.owner,
		ReadListCreate {
			name: "Queue".to_owned(),
			summary: Some("next up".to_owned()),
			ordered: true,
			book_ids: vec![f.b1.id.clone()],
		},
	)
	.await
	.unwrap();
	assert_eq!(list.ordering, "MANUAL");
	assert_eq!(list.description.as_deref(), Some("next up"));

	add_shelf_items(
		&f.ctx,
		&f.owner,
		&list.id,
		vec![f.a2.id.clone(), f.b1.id.clone()],
		None,
	)
	.await
	.unwrap();
	assert_eq!(shelf_items(&f, &f.owner, &list.id).await, vec!["b1", "a2"]);

	remove_shelf_items(&f.ctx, &f.owner, &list.id, vec![f.b1.id.clone()], None)
		.await
		.unwrap();
	assert_eq!(shelf_items(&f, &f.owner, &list.id).await, vec!["a2"]);

	let updated = update_read_list(
		&f.ctx,
		&f.owner,
		&list.id,
		ReadListUpdate {
			summary: Some(None),
			ordered: Some(false),
			book_ids: Some(vec![f.a1.id.clone(), f.a2.id.clone()]),
			..Default::default()
		},
	)
	.await
	.unwrap();
	assert_eq!(updated.member_ids, vec![f.a1.id.clone(), f.a2.id.clone()]);
	let stored = reading_list::Entity::find_by_id(list.id.clone())
		.one(f.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(stored.description, None);
	assert_eq!(stored.ordering, "CREATED_AT");
}

#[tokio::test]
async fn rename_shelf_records_device_and_requires_owner() {
	let f = fixture().await;
	let list = create_read_list(
		&f.ctx,
		&f.owner,
		ReadListCreate {
			name: "Before".to_owned(),
			summary: None,
			ordered: false,
			book_ids: vec![],
		},
	)
	.await
	.unwrap();

	assert!(matches!(
		rename_shelf(&f.ctx, &f.other, &list.id, "Hijack".to_owned(), None).await,
		Err(CoreError::Forbidden(_))
	));
	assert!(matches!(
		rename_shelf(&f.ctx, &f.owner, &list.id, " ".to_owned(), None).await,
		Err(CoreError::BadRequest(_))
	));

	let server_owner = AuthUser {
		id: f.other.id.clone(),
		is_server_owner: true,
		..Default::default()
	};
	rename_shelf(
		&f.ctx,
		&server_owner,
		&list.id,
		"After".to_owned(),
		Some("Kobo Sage".to_owned()),
	)
	.await
	.unwrap();

	let stored = reading_list::Entity::find_by_id(list.id.clone())
		.one(f.conn())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(stored.name, "After");
	assert_eq!(stored.source_device.as_deref(), Some("Kobo Sage"));
}

#[tokio::test]
async fn shelf_sync_delta_reports_full_changed_and_deleted() {
	let f = fixture().await;
	let kept = create_collection(&f.ctx, &f.owner, collection_input("kept", vec![]))
		.await
		.unwrap();
	let doomed = create_collection(&f.ctx, &f.owner, collection_input("doomed", vec![]))
		.await
		.unwrap();
	let list = create_read_list(
		&f.ctx,
		&f.owner,
		ReadListCreate {
			name: "list".to_owned(),
			summary: None,
			ordered: false,
			book_ids: vec![f.a1.id.clone()],
		},
	)
	.await
	.unwrap();
	let unprojected =
		create_collection(&f.ctx, &f.owner, collection_input("unprojected", vec![]))
			.await
			.unwrap();
	let mut row = unprojected.into_active_model();
	row.kobo_shelf = Set(false);
	row.update(f.conn()).await.unwrap();

	// Full sync: every projected shelf, sorted by name, nothing deleted.
	let full = shelf_sync_delta(f.conn(), &f.owner, None, EPUB)
		.await
		.unwrap();
	let names: Vec<&str> = full.new.iter().map(|shelf| shelf.name.as_str()).collect();
	assert_eq!(names, vec!["doomed", "kept", "list"]);
	assert!(full.changed.is_empty());
	assert!(full.deleted_ids.is_empty());
	assert!(shelves_for_user(f.conn(), &f.other, EPUB)
		.await
		.unwrap()
		.is_empty());

	let mut events = f.events();
	delete_shelf(&f.ctx, &f.owner, &doomed.id).await.unwrap();
	assert!(matches!(
		events.try_recv().unwrap(),
		CoreEvent::CollectionDeleted(deleted) if deleted.id == doomed.id
	));
	delete_shelf(&f.ctx, &f.owner, &list.id).await.unwrap();
	assert!(matches!(
		events.try_recv().unwrap(),
		CoreEvent::ReadListDeleted(deleted) if deleted.id == list.id && deleted.book_ids == vec![f.a1.id.clone()]
	));

	// An expired tombstone is pruned and never reported.
	let expired = kobo_shelf_tombstone::ActiveModel {
		user_id: Set(f.owner.id.clone()),
		shelf_id: Set("long-gone".to_owned()),
		..Default::default()
	}
	.insert(f.conn())
	.await
	.unwrap();
	let mut expired = expired.into_active_model();
	expired.deleted_at = Set((Utc::now() - Duration::days(31)).into());
	expired.update(f.conn()).await.unwrap();

	let incremental = shelf_sync_delta(
		f.conn(),
		&f.owner,
		Some((Utc::now() - Duration::hours(1)).into()),
		EPUB,
	)
	.await
	.unwrap();
	assert!(incremental.new.is_empty());
	assert_eq!(
		incremental
			.changed
			.iter()
			.map(|shelf| shelf.shelf_id.as_str())
			.collect::<Vec<_>>(),
		vec![kept.id.as_str()]
	);
	let mut deleted = incremental.deleted_ids.clone();
	deleted.sort();
	let mut expected = vec![doomed.id.clone(), list.id.clone()];
	expected.sort();
	assert_eq!(deleted, expected);
	assert!(tombstones_for(f.conn(), "long-gone").await.is_empty());

	// Nothing written since the previous sync: an empty delta.
	let quiet = shelf_sync_delta(
		f.conn(),
		&f.owner,
		Some((Utc::now() + Duration::hours(1)).into()),
		EPUB,
	)
	.await
	.unwrap();
	assert!(quiet.changed.is_empty() && quiet.deleted_ids.is_empty());
}
