# collections

| | |
| --- | --- |
| **Package** | `stump_collections` (`crates/collections`) |
| **Purpose** | The canonical shelf containers — collections (sets of series) and reading lists (ordered or unordered sets of books) — and their Kobo `Tag` projection. Owns every create/rename/reorder/delete rule, the membership validation, the `source_device` provenance, and the tombstone TTL that incremental Kobo syncs key on. It does not own the transport surfaces (GraphQL mutations, Komga CRUD routes, Kobo `tags` endpoints) — they all call in here. |
| **Reference / upstream** | Kobo `Tag`/`ChangedTag`/`DeletedTag` sync shapes as consumed by `stump_kobo` (`crates/kobo/src/sync.rs`); Komga collection/read-list CRUD semantics pinned to `komga-client` 0.11.0 `74412a6e`; cross-protocol contract in `docs/content/docs/developer/shelves-and-collections.mdx`. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| The stable shelf id is the container id itself, and a Kobo write may supply it | A shelf must keep its id across renames and reorderings, and a shelf created on a device is the same row as a collection created by a Komga client | `src/service.rs::create_device_shelf`; `src/lib.rs` module doc |
| Every mutation goes through this crate, from every protocol | Membership validation, owner checks, `updated_at` bumps and provenance cannot drift between GraphQL, Komga CRUD and Kobo write-back | callers: `crates/graphql/src/mutation/reading_list.rs`, `apps/server/src/routers/{komga_backend.rs,kavita/backend.rs,kobo_backend/tags.rs}` |
| Owner rules differ by surface: native mutations require the creating user; Kobo device writes also allow the server owner | The pre-existing Komga route rule must not loosen, but a device user manages their own shelves | `src/service.rs` owner checks |
| `source_device` is set on Kobo write-back and cleared on a native name write (last writer wins on `name`) | The projection has to say where a name came from without inventing a merge policy | `src/service.rs` |
| A `CoreEvent` is emitted only after the transaction commits | A rolled-back mutation must not produce a client event | `src/service.rs` |
| Shelf visibility mirrors the owning surface: collections are creator-only, reading lists reuse reading-list RBAC | Collections have no visibility column; inventing one here would fork the model | `src/shelf.rs` |
| Tombstones are kept for 30 days | A device that does not sync inside the window misses the deletion until a full resync; unbounded tombstones grow forever | `src/shelf.rs::KOBO_SHELF_TOMBSTONE_TTL` |
| The crate depends on `stump_core` (`Ctx`, `CoreEvent`, `CoreError`) | It is an application service above core, not a leaf: it needs the shared connection, the event channel, and the core error vocabulary. Nothing in `stump_core` depends on it, so there is no cycle | `Cargo.toml`; `src/service.rs` imports |

## Layout

| File | Responsibility |
| --- | --- |
| `lib.rs` | Module docs (the cross-protocol shelf contract) and the public surface |
| `service.rs` | Create/rename/reorder/delete for collections and reading lists, membership validation, device shelves, event emission |
| `shelf.rs` | The shelf projection: containers as Kobo `Tag` entitlements, `shelves_for_user`, `shelf_sync_delta`, tombstone TTL |
| `tests.rs` | Behavioural tests against an in-memory SQLite database (8 tests) |

## How to verify

```bash
cargo test -p stump_collections                  # service rules + shelf projection/delta
cargo test -p stump_kobo                         # the sync consumer of the projection
```

From `../komga-compat/`: `make replay` (Komga collection/read-list CRUD).

## Deep docs

`docs/content/docs/developer/shelves-and-collections.mdx` — the cross-protocol
shelf contract, id stability, and the Kobo delta rules.
