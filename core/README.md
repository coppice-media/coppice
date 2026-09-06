# Core 🏭

The `core` crate contains Stump's core functionalities

## Structure 📦

The `src` directory contains the following modules:

- `config`: Configuration for the any apps consuming the core, including environment variables and tracing
- `db`: Database client, models, and utilities
- `filesystem`: Anything related to the filesystem and handling of files
  - `image`: Image processing and utilities
  - `media`: Media processing and utilities
  - `scanner`: The bulk of the indexing and scanning logic
- `job`: Background job processing and execution lifecycle helpers, Apalis-backed system
- `opds`: OPDS feed generation and XML utilities

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| SQLite migrations run on a dedicated `max_connections = 1` pool, which is closed before the serving pool opens; PostgreSQL keeps migrating on the serving connection | `sea-orm-migration` only transacts migrations for PostgreSQL (`sea-orm-migration-1.1.16/src/migrator.rs:261-272`), so on SQLite each DDL statement lands on whichever pooled connection is free. With `db_max_connections = 10` the create/copy/drop/rename in `m20260909_000000_add_ingest_media_targets` straddled connections and a fresh database failed to start with `(code: 1) there is already another table or index with this name: ingest_analysis_jobs`. | `src/database.rs` `SQLITE_MIGRATION_POOL_SIZE`, `migrate`, `sqlite_pool`; test `migrates_a_fresh_database_behind_a_multi_connection_pool` (fails with the constant raised above 1) |
| Every write transaction in `core` opens with `models::txn::begin_write` (SQLite `BEGIN IMMEDIATE`); read-only transactions keep `TransactionTrait::begin` | `delete_library`, the collection/shelf services, the scanner writers and the ingest store all read before they write, and SQLite will not run the busy handler when a deferred transaction has to promote its read snapshot to the write lock — `DELETE /api/v1/libraries/{id}` during a scan returned `500 database is locked`. | `src/library.rs`, `src/collections/service.rs`, `src/ingest/store.rs`, `src/filesystem/scanner/utils.rs`; `crates/models/src/txn.rs`; test `delete_library_waits_out_a_concurrent_writer` |
