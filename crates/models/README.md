# models

## Purpose

`models` is the persistence contract of Stump: every SeaORM entity (69 tables
under `src/entity/`), the shared enums/value objects stored in those tables
(`src/shared/`), the `EntityError` type, the `Prefixer` join helper, and the
small set of DB-touching services that must be shared by several transports
(reading-progress upsert, list/collection membership cleanup). It deliberately
does **not** own schema migrations (`crates/migrations`), HTTP/GraphQL shapes
(`crates/graphql`, `stump_api_types`), authorization (`stump_auth`), or
filesystem/media processing (`stump_media`). GraphQL derives on enums and
objects are opt-in behind the `graphql` feature so the `minimal` server links
without `async-graphql`.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream `stumpapp/stump` `crates/models` (merge base `37fdb7d7`) | Fork keeps upstream entity names/columns; additions are append-only columns and new tables. |
| `crates/migrations/src/*.rs` | Every entity column must match the migration that created it; migrations are the schema authority. |
| Komga `komga-client` 0.11.0 `74412a6e` / Komelia `65f92fde` | `series_metadata` Komga fields (`komga_sort_title`, reading direction, alternate titles) mirror the Komga series-metadata DTO so the adapter can round-trip without lossy mapping. |
| Liseur `31f8182d` | `liseur_*` entities (tokens, works, ops, annotations) mirror the Liseur sync payloads. |
| Kobo firmware sync protocol | `reading_session.kobo_state` (JSON) and `kobo_sync_session` hold device-side state exactly as the device reports it. |
| KOReader sync API | `reading_session.koreader_progress` stores the opaque KOReader progress string. |
| `docs/content/docs/developer/unified-reading-state.mdx` | Design for the reading-session model these entities implement. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `graphql` feature gates every `async_graphql` derive (`#[cfg_attr(feature = "graphql", …)]`, 222 sites in 49 files) | `minimal`/protocol-only server profiles must not compile `async-graphql`; `graphql`/`stump_core` enable it explicitly. | `Cargo.toml` features; `src/shared/enums.rs:9,30,86,…`; `apps/server/Cargo.toml:16-22` |
| Every write transaction is opened with `txn::begin_write` (SQLite `BEGIN IMMEDIATE`), never `TransactionTrait::begin` | sea-orm 1.1 emits a deferred `BEGIN`, so a transaction that reads before it writes has to promote its read snapshot to the write lock — SQLite refuses to run the busy handler for a promotion and returns `SQLITE_BUSY` immediately, which is how `DELETE /api/v1/libraries/{id}` during a scan became `500 database is locked`. `begin_write` takes the lock up front, where `busy_timeout` applies. | `src/txn.rs`; `sea-orm-1.1.16/src/database/transaction.rs:69-76`; `core/src/library.rs` `delete_library` |
| `txn::begin_write` takes `&DatabaseConnection`, not `impl TransactionTrait` | `begin()` on an existing transaction opens a `SAVEPOINT`; the `ROLLBACK` + `BEGIN IMMEDIATE` swap would discard the enclosing transaction, so nesting must be a compile error. Read-only transactions and `crates/migrations` (which runs inside the migrator's transaction) keep `begin()`. | `src/txn.rs`; `crates/graphql/src/{query,object}/smart_lists.rs`; `crates/migrations/src/m20260923_000000_backfill_reading_heads.rs:155` |
| `txn::is_write_lock_contention` matches the error message, not a native code | sea-orm does not re-export `sqlx::sqlite::SqliteError`, so the code is unreachable without downcasting through an unnameable type. Both surfaces map a match to `503` + `Retry-After: 1`. | `src/txn.rs`; `crates/komga/src/errors.rs`; `apps/server/src/errors.rs` |
| `UserPermission` and other stored enums serialise `SCREAMING_SNAKE_CASE` via strum + serde and are stored as `String` columns | Stable on-disk representation independent of Rust variant order; matches upstream. | `src/shared/enums.rs:728-735` |
| `AuthUser` is a projection (id, flags, permissions, age restriction, preferences), not the `user::Model` | Auth context must not carry password hash; `stump_auth` depends on this type only. | `src/entity/user.rs:58-68` |
| Reading-progress domain rules (`should_extend_session`, logical date) are pure functions in `src/domain/`; DB writes in `src/services/reading_progress.rs` | OPDS, Kobo, KOReader, Komga and GraphQL all write progress; one implementation of grace-period/extension semantics. | `src/domain/reading_progress.rs`, `src/services/reading_progress.rs` |
| `services::annotation_attachment` owns the `annotation_attachments` rows and `relative_path` is the single definition of the on-disk layout (`<annotation_id>/<sha256>.<ext>`, relative to the attachment root); the bytes are written and served by the application | The liseur-sync adapter, the GraphQL query and the `/api/v2` download all need the same rows, but only the server knows the configured root — and a relative path survives moving the configuration directory. | `src/services/annotation_attachment.rs`; `src/entity/annotation_attachment.rs`; `apps/server/src/routers/liseur_sync/attachments.rs` |
| A reading position is written twice: `services::reading_progress` keeps the session/statistics history, `services::reading_state::apply` materialises the one head (plus provenance) every protocol read path uses | The head owns the cross-protocol conflict rule and raw payloads; sessions own per-day statistics and readthroughs. Every writer — and every fixture, `tests::fake_data::ReadingSession::insert` — must do both, or a book with progress reads as unread. | `src/services/reading_state.rs`, `src/domain/reading_state.rs`; `crates/tests/src/fake_data.rs`; `crates/migrations/src/m20260923_000000_backfill_reading_heads.rs` backfills pre-head databases |
| Reading-list / collection membership FKs use `ON DELETE RESTRICT`; `services::lists::remove_memberships_for_*` must run in the same transaction before hard delete | Prevents silent loss of list membership when the scanner removes media. | `src/services/lists.rs:5-29` |
| `Prefixer` aliases columns as `<table><column>` for nested `FromQueryResult` joins | SeaORM has no built-in nested-struct hydration for joins. | `src/prefixer.rs` (from SeaQL/sea-orm discussion #1502) |
| `filter-gen` proc macro generates ordering/filter enums for entities | Keeps filter contracts in sync with entity columns without hand-written enums. | `src/entity/media.rs:5`; `crates/macros/filter-gen` |
| `EntityError` wraps `DbErr`, glob (`ignore_rules`) and serde errors | One error type for all entity helpers. | `src/error.rs` |
| Provider dedupe lives in two provider-only entities (`provider_series_identity`, `provider_series_link`) keyed by `series_id` with `ON DELETE CASCADE` | A locally scanned series never gets a row, and deleting a series (GC, merge, library clean) takes its dedupe state with it. | `src/entity/provider_series_identity.rs`, `src/entity/provider_series_link.rs`; `crates/provider/src/identity.rs` |
| `shared::visibility::VisibilityScope` is the single visibility funnel: every `find_for_user`-family helper on `library`, `series` and `media` takes `impl Into<VisibilityScope>`, and `library_condition` `AND`s the user's `library_exclusions` subquery with the authenticating device's `devices.library_scope` | `From<&AuthUser>` yields the request's resolved scope, so every existing caller is scoped by construction instead of by remembering to pass one, and no protocol can forget the device scope. `VisibilityScope::inherit(user)` is the only, explicit opt-out, for the few call sites that must describe what a *user* owns rather than what a device currently sees (the Kobo removal item). | `src/shared/visibility.rs`; `src/entity/{library,series,media}.rs`; `src/entity/user.rs` `AuthUser::device_library_scope`; `crates/devices/src/scope.rs` |
| `entity::media::AUDIO_EXTENSIONS` + `audio_extension_condition()` are the single SQL-side definition of "this row is an audiobook"; `stump_media::ContentType::is_audio` is the same set as a predicate and its test iterates this list | The ABS profile includes audio, the liseur-sync catalogue excludes it, and the two lists had already been duplicated once (`crates/abs/src/routes/query.rs`); a drift would silently make a book visible to one profile and downloadable by neither | `src/entity/media.rs`; `crates/media/src/content_type.rs` `test_audio_content_types_are_audio`; `apps/server/src/routers/liseur_sync/storage.rs` `visible_media` |

## Layout

| Path | Responsibility |
| --- | --- |
| `src/entity/` | One SeaORM entity per table (`library`, `media`, `series`, `user`, `reading_session`, `book_club_*`, `ingest_*`, `liseur_*`, `kobo_sync_session`, `reading_list*`, `collection*`, `device`, `device_credential`, `device_pairing`, …) and `mod.rs` re-exports |
| `src/shared/` | Stored value types: `enums.rs` (permissions, statuses, layouts), `permission_set.rs`, `readium.rs` (locators), `ignore_rules.rs`, `image*.rs`, `series_metadata.rs`, `book_club.rs`, `ordering.rs`, `arrangement.rs`, `alphabet.rs`, `analysis.rs`, `api_key.rs`; plus `visibility.rs`, the per-request `VisibilityScope` (user exclusions ∩ device library scope) |
| `src/domain/reading_progress.rs` | Pure reading-session rules (grace period, logical date) |
| `src/services/` | `reading_progress.rs` (normalised progression upsert), `lists.rs` (membership cleanup) |
| `src/prefixer.rs` | Join column prefixing helper |
| `src/txn.rs` | `begin_write` (SQLite `BEGIN IMMEDIATE` write transactions) and `is_write_lock_contention` |
| `src/error.rs` | `EntityError` |
| `src/tests/common.rs` | SQL-to-string helpers used by unit tests |

## How to verify

```text
cargo test -p models --lib --tests             # 82 unit tests (permission sets, ignore rules, progress rules, filters, write-lock contention)
cargo test -p models --features graphql        # derives compile
cargo check -p stump_server --no-default-features --features minimal   # async-graphql must be absent from the graph
cargo test -p migrations --lib --tests         # schema/entity agreement (reading_lists_collections.rs)
```

Harness (from `../komga-compat/`): `make replay` and `make replay-liseur-sync`
persist through these entities; `make replay-mihon` exercises
`reading_session` via the tracker. Live probe: `stump-komga` fixture at
`http://127.0.0.1:25600`.

## Deep docs

- `docs/content/docs/developer/unified-reading-state.mdx`
- `docs/content/docs/developer/server-architecture.mdx`
- `docs/content/docs/developer/komga-compat.mdx`, `liseur-sync-integration.mdx`, `kobo-sync-capabilities.mdx`, `modular-ingest.mdx`
- `crates/migrations/README.md`
