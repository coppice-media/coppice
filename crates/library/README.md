# library

| | |
| --- | --- |
| **Package** | `stump_library` (`crates/library`) |
| **Purpose** | Library create/update/delete (`library`) and series reshape — move books between series, merge series, split books out into a new series (`series`) — as one shared path for every surface. Owns validation, persistence, watcher and scheduled-scan wiring, the filesystem moves that keep the next scan a no-op, and core-event emission. It does not own scanning, thumbnailing, or the transport surfaces (GraphQL mutations, Komga/Kavita routes) that call in here. |
| **Reference / upstream** | Komga library CRUD semantics pinned to `komga-client` 0.11.0 `74412a6e` (`crates/komga/src/routes/library.rs` delegates here); scanner recognition rules in `stump_core::filesystem::scanner` (existing media are matched by `series_id` **and** `path`). |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Every surface (GraphQL, Komga, Kavita) calls these functions rather than writing rows itself | Validation, watcher wiring, scheduled scans and events cannot drift between protocols | `src/library.rs` module doc; callers `crates/graphql/src/mutation/library.rs`, `apps/server/src/routers/komga_backend.rs` |
| A reshape always moves the file into the target series' directory inside the write transaction | The scanner matches known books by `series_id` and `path`, so a database-only regrouping is undone by the next scan (old file re-created as a second row, moved row reported missing) | `src/series.rs` module doc; `apply_moves`/`undo_moves` |
| A failed move renames back whatever already moved and rolls the transaction back | The database and the filesystem must never end up disagreeing | `src/series.rs::apply_moves` (`undo_moves` on the first error) |
| Filesystem moves use `stump_media::move_file` (rename, falling back to copy + remove) | Library roots and staging roots are frequently on different filesystems, where `rename` fails with `EXDEV` | `src/series.rs`; `crates/media/src/common.rs::move_file` |
| Thumbnails are removed through `stump_media::image::remove_thumbnails` on delete | Orphaned thumbnails otherwise survive the row and are served for a book that no longer exists | `src/library.rs`, `src/series.rs` |
| The crate depends on `stump_core` (`Ctx`, `CoreEvent`, `CoreError`, `job::stump_job`) | It is an application service above core: it dispatches scan jobs and emits core events. Nothing in `stump_core` depends on it, so there is no cycle | `Cargo.toml`; `src/library.rs` imports |

## Layout

| File | Responsibility |
| --- | --- |
| `lib.rs` | Crate docs and the two modules |
| `library.rs` | `create`/`update`/`delete` for libraries, tag and config handling, watcher add/remove, scheduled-scan wiring, `LibraryCreated`/`Updated`/`Deleted` events (1 test) |
| `series.rs` | Series reshape: move books, merge two series, split books into a new series, with the filesystem renames and rollback (8 tests) |

## How to verify

```bash
cargo test -p stump_library                      # CRUD + reshape, including the rollback paths
cargo test -p graphql --lib library              # the GraphQL surface that delegates here
```

From `../komga-compat/`: `make replay-library-management LIBRARY_ROOT=<dir>`.

## Deep docs

`docs/content/docs/developer/server-architecture.mdx` — where the shared
library path sits relative to the protocol adapters.
