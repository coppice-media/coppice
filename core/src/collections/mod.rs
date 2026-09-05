//! Canonical shelf containers: collections and reading lists.
//!
//! Collections (unordered or ordered sets of series) and reading lists
//! (ordered or unordered sets of books) are the only canonical containers.
//! Every protocol surface that speaks of "shelves" projects onto them:
//!
//! - **Kobo** devices see containers with `kobo_shelf` enabled as `Tag`
//!   entitlements, and the Kobo `tags` endpoints write back to the container
//!   with device provenance (see [`shelf`] and [`service`]).
//! - **Komga** clients speak of collections/read lists; the compatibility
//!   CRUD routes delegate to the same service functions, so a shelf created
//!   on a device and a collection created by a Komga client are the same row
//!   with the same id.
//!
//! The stable shelf id is the container id itself: both tables use Text UUID
//! primary keys, and the Kobo write-back honors a device-supplied id, so a
//! shelf keeps its id across renames and reorderings.
//!
//! See `docs/content/docs/developer/shelves-and-collections.mdx` for the
//! cross-protocol contract.

pub mod service;
pub mod shelf;

pub use service::{
	add_shelf_items, create_collection, create_device_shelf, create_read_list,
	delete_collection, delete_read_list, delete_shelf, remove_shelf_items, rename_shelf,
	update_collection, update_read_list, CollectionCreate, CollectionUpdate, ContainerMembers,
	ReadListCreate, ReadListUpdate,
};
pub use shelf::{
	shelf_sync_delta, shelves_for_user, ContainerKind, ShelfProjection, ShelfSyncDelta,
	KOBO_SHELF_TOMBSTONE_TTL,
};
