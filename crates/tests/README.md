# tests

## Purpose

`tests` is the workspace's shared **test-only** fixture crate: `db::test_database`
(an in-memory SQLite `DbConn` with every SeaORM entity's table created) and the
`fake_data` builders (`Media`, `Series`, `Library`, `User`, `ReadingSession`)
that insert minimally-valid rows. It owns no production code and is listed only
under `[dev-dependencies]` — never in a shipped binary. It deliberately does
**not** own migrations (that is `crates/migrations`), entity definitions
(`crates/models`), HTTP-level harnesses (`apps/server/tests/common/`) or
provider mocks (`metadata_integrations::mock_http`).

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream Stump `crates/tests` | Origin of `db::test_database` and the `fake_data` builder style ("`None` means *use some default*"), `src/fake_data.rs:15-17`. |
| `crates/models` (`models::entity::*`, `models::services::reading_state`) | Only dependency that carries schema meaning: tables are built from entities, and `ReadingSession` fixtures call the real `reading_state::apply`. `src/db.rs:1-13`, `src/fake_data.rs:250-270` |
| `crates/migrations` | **Not** a dependency (`Cargo.toml:6-13`). Suites that need the migrated schema build their own DB and pass it in. |
| `docs/content/docs/developer/contributing.mdx:161-170` | "Rust server integration tests … uses a lot of `fake_data` to insert data into the in-memory database." |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `sqlite::memory:` rather than a temp file | Nothing to clean up and no cross-test file contention; the database lives and dies with the returned `DbConn`, so each test that calls `test_database()` gets its own isolated schema. No `cache=shared` URI, so the handle is the only way in. | `src/db.rs:15-25` |
| Schema is built from entity definitions (`Schema::create_table_from_entity`) instead of running `migrations` | Keeps the fixture crate free of the migration graph and makes bootstrap a single round of `CREATE TABLE`s. Cost: the table list is hand-maintained, so a new entity that a test touches must be added here or the test fails with `no such table`. | `src/db.rs:27-80` (list at `:30-76`) |
| Tables only — no migration-authored views, triggers or non-entity tables | Those have no SeaORM entity to derive from. Suites that need the real thing (e.g. liseur-sync tables) take the `with_parts(db, config)` door with a migrated connection. | `src/db.rs:28-76`; `apps/server/tests/common/app.rs:26-32` |
| `create_database_tables(&db)` is public alongside `test_database()` | Lets a caller apply the entity schema to a connection it already owns instead of forcing the in-memory one. | `src/db.rs:27` |
| Fixtures are **not** seeded/deterministic: ids are `Uuid::new_v4()`, default library/series names are random `rand::distr::Alphabetic` strings from `rand::rng()` | Random names keep unique-path/name constraints from colliding across fixtures in one DB; nothing in the crate depends on reproducing a specific value. Tests that assert on identity pass an explicit `id`/`name`. | `src/fake_data.rs:113-122`, `src/fake_data.rs:154-163` |
| `created_at` is applied by a second `update` after `insert` | `ActiveModelBehavior` stamps `created_at` on insert, so a fixture cannot set it directly; tests that need deterministic ordering between rows set the field and pay one extra statement. | `src/fake_data.rs:63-71`, `src/fake_data.rs:186-188`, `src/fake_data.rs:221-232` |
| `ReadingSession::insert` also materializes the unified reading head | Every real protocol write applies both; a fixture that only inserted the session would look like a book with no progress to the KOReader/Komga/Kobo/OPDS/GraphQL read paths. Stump's own writes carry no device clock, so `updated_at` is `None`. | `src/fake_data.rs:234-271` |
| Builders `expect` instead of returning `Result`, and `insert` returns the `Model` | A broken fixture should panic at its own line with a message, not add `?` noise to every test. | `src/fake_data.rs:61`, `src/fake_data.rs:100`, `src/fake_data.rs:176` |
| `Media` defaults are concrete, not empty: `size = 1234`, `pages = 940`, `status = Ready`, `path = "{name}.{extension}"`, extension `epub` | Pagination, reader and file-status code paths need plausible values; a zero-page `Missing` book would fail unrelated assertions. | `src/fake_data.rs:43-59` |
| `User::insert` always sets `is_server_owner = true`, `is_locked = false` | Fixture users exist to make request-scoped tests run; permission/lockout suites build their own restricted users rather than un-setting flags here. | `src/fake_data.rs:89-101` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Re-exports `db` and `fake_data` |
| `src/db.rs` | `test_database()`, `create_database_tables()`, the entity table list |
| `src/fake_data.rs` | `Media`, `User`, `Library`, `Series`, `ReadingSession` builders |
| `Cargo.toml` | `sea-orm`, `models`, `uuid`, `rand`, `rust_decimal`, `serde_json`, `chrono` |

Consumers (all `[dev-dependencies]`): `apps/server` (`Cargo.toml:153`), `core`
(`:107`), and `crates/{graphql,komga,kobo,kavita,devices,provider,annotation-sync,ingest,collections,library,abs,kindle}`.

## How to verify

```text
cargo test -p tests                      # compiles the fixtures; the crate has no tests of its own
cargo test -p stump_core --lib
cargo test -p graphql
cargo test -p stump_server --test api_tests
cargo test -p stump_provider
```

The consumer suites are the real proof: `tests::db::test_database` is used by
`apps/server/tests/common/app.rs:9,23`, `crates/provider/src/host.rs`,
`crates/ingest/src/quality/tests.rs` and others, so a missing table or a broken
builder shows up there, not here.

## Deep docs

- `docs/content/docs/developer/contributing.mdx` — "Rust server integration tests" (§ Testing)
