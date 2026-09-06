# migrations

## Purpose

`migrations` is the schema authority: an append-only, chronologically ordered
list of `sea-orm-migration` steps (`m20250807_202824_init` through
`m20260909_000000_add_ingest_media_targets`, plus in-flight untracked ones)
exposed as `Migrator`. `stump_core` runs `Migrator::up` on every connect
(`core/src/database.rs:150`), so the server never starts on a stale schema.
It deliberately does **not** define entities (`crates/models`), seed data, or
backend selection (`core/src/database.rs`). The `migrate` binary is a thin
`sea-orm-migration` CLI wrapper gated behind the `cli` feature, which the
server graph does not enable.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream `stumpapp/stump` `crates/migrations` (merge base `37fdb7d7`) | All migrations up to `m20260816_000000_drop_legacy_epubcfi` are upstream and immutable. |
| Fork additions (`b068deb9`, `ab5b7e20`, `294bd461`, later) | `m20260902` reading lists/collections, `m20260904` liseur-sync, `m20260905` Kobo reading state, `m20260906` liseur token metadata, `m20260907` ingest, `m20260908` series-metadata Komga fields, `m20260909` ingest media targets. |
| Komga `komga-client` 0.11.0 `74412a6e` | `m20260908` columns mirror the Komga series-metadata DTO. |
| Liseur `31f8182d` | `m20260904`/`m20260906` tables mirror Liseur sync payloads (tokens, works, ops, annotations). |
| SQLite `ALTER TABLE` limits | Drives the one-column-per-`alter_table` and rebuild-instead-of-alter rules below. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `src/lib.rs` is append-only: never reorder or remove a `mod` / `Box::new` | SeaORM records applied migration names; reordering breaks every existing database. | `src/lib.rs:44-45` comment; `.omp/RULES.md` "Defect guardrails" |
| One SQLite column per `alter_table` call | SQLite accepts a single `ADD COLUMN` per statement; multi-column alters fail at runtime only on SQLite. | `.omp/RULES.md`; pattern in `m20251013_*`, `m20260908_*` |
| Nullability changes rebuild the table (create new → copy → drop → rename) | SQLite cannot alter a column's nullability in place. | `src/m20260909_000000_add_ingest_media_targets.rs:1-10` |
| Reading-list/collection FKs use `ON DELETE RESTRICT` | Membership must be removed explicitly before media/series hard delete (see `models::services::lists`). | `src/m20260902_000000_add_reading_lists_and_collections.rs`; test `tests/reading_lists_collections.rs` |
| Every migration must run on PostgreSQL too | Server keeps PostgreSQL URL selection; smoke test behind `postgres-tests` uses testcontainers. | `tests/postgres.rs`; `Cargo.toml` `postgres-tests` feature |
| `cli` feature (default) pulls `sea-orm-migration/cli`; server depends with `default-features = false` | Narrowed server dependency graph (no clap-based migration CLI linked). | `Cargo.toml:8-14`; `docs/.../server-architecture.mdx:20` |
| Schema tests inspect `sqlite_master`/`PRAGMA` directly rather than entity queries | Proves the DDL, not the ORM mapping; catches missing indexes/FKs. | `tests/reading_lists_collections.rs:4-104` |
| A data backfill projects rows with raw SQL plus Rust, never through `models` entities, and its `down` is a no-op | An append-only migration must keep working when the entities it mirrors change; and once a head exists a protocol write may have advanced it, so deleting backfilled rows would discard newer progress. | `src/m20260923_000000_backfill_reading_heads.rs`; `tests/backfill_reading_heads.rs` |
| `m20260924` adds `provider_series_identity` and `provider_series_links` as separate tables rather than columns on `series` | The dedupe key and its links are provider-only indexed lookups; putting them on the shared `series` table would need one `ADD COLUMN` per field and touch every non-provider row. | `src/m20260924_000000_add_provider_series_links.rs`; `crates/provider/src/identity.rs` |
| `m20260928` puts the Send-to-Kindle address on `devices` rather than reusing `registered_email_devices` | That table is the `sendAttachmentEmail` address book: free-form recipients with no owner, no last-seen state and no protocol, so a send to one could never be recorded as a device sighting. One nullable `ADD COLUMN` keeps the address next to the device's transform profile and library scope. | `src/m20260928_000000_add_device_kindle_email.rs`; `crates/devices/README.md`; `docs/content/docs/developer/devices.mdx` (`## Send to Kindle`) |

## Layout

| Path | Responsibility |
| --- | --- |
| `src/lib.rs` | `Migrator` + ordered module list (append here) |
| `src/m<timestamp>_<name>.rs` | One migration each; `up`/`down` via `SchemaManager` |
| `bin/migrate.rs` | `cargo run -p migrations --bin migrate` CLI (`cli` feature; defaults `DATABASE_URL` to `sqlite://./core/dev.db?mode=rwc`) |
| `tests/reading_lists_collections.rs` | SQLite DDL assertions for the fork's list/collection migration |
| `tests/backfill_reading_heads.rs` | Seeds a v2-era database (sessions only) and asserts the reading-head backfill's mapping, ordering and idempotency |
| `tests/postgres.rs` | PostgreSQL run-through (feature `postgres-tests`, needs Docker) |

## How to verify

```text
cargo test -p migrations --lib --tests                       # part of the project gate
cargo test -p migrations --features postgres-tests           # optional, Docker required
cargo run -p migrations --bin migrate -- status              # against core/dev.db or $DATABASE_URL
cargo check -p stump_server --no-default-features --features minimal
```

Live probe: restart the `stump-komga` fixture via `hub`; startup log shows
migrations applied before the listener binds. Harness (`../komga-compat/`):
`make replay` and `make replay-liseur-sync` run against the migrated schema.

## Deep docs

- `docs/content/docs/developer/server-architecture.mdx` — startup/migration lifecycle, PostgreSQL retention
- `docs/content/docs/developer/liseur-sync-integration.mdx`, `modular-ingest.mdx`, `unified-reading-state.mdx`, `komga-compat.mdx`
- `crates/models/README.md`
