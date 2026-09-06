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
| The ingest preprocess hook is resolved at startup (`StumpCore::init_config`), runs at most once per item behind `ingest_drop_items.preprocessed_at`, and re-hashes the staged file on exit `0` | A hook is operator-supplied, so the two failure modes are a command that never runs and a command that runs too often: an unresolvable `ingest_preprocess_command` validated lazily would silently skip every dropped file for the server's uptime, and a hook re-run on re-analysis would feed a conversion pipeline its own output. Re-hashing is what keeps `source_sha256`, the page index, the `quality_report`, and every candidate describing one file after the hook rewrites it. | `src/ingest/preprocess.rs`, `src/ingest/store.rs` `run_preprocess`, `src/ingest/coordinator.rs` `run_phases`, `src/lib.rs` `init_config`; `crates/migrations/src/m20260927_000000_add_ingest_preprocess.rs`; tests `unresolvable_command_fails_validation`, `preprocess_hook_rewrite_is_rehashed_onto_the_item`, `preprocess_hook_failure_fails_the_item_with_the_stderr_tail`, `preprocess_hook_runs_once_per_item`, `preprocess_hook_runs_before_staged_analysis` |
