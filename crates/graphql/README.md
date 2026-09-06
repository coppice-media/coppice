# graphql

## Purpose

`graphql` is the async-graphql schema that backs the web UI, desktop and Expo
clients: `Query`, `Mutation` and `Subscription` roots, input/object types,
filters, ordering, pagination, permission guards, dataloaders, and the
`graphql-gen` binary that emits `schema.graphql` for TypeScript codegen. It
deliberately does **not** own HTTP mounting or the playground
(`apps/server/src/routers/api/graphql.rs`), authentication (`stump_auth` +
server middleware), persistence (`models`, `stump_core`), or any of the
protocol surfaces (OPDS, Komga, Kobo, KOReader, liseur-sync), which never link
this crate. The whole crate is optional in the server: feature `graphql`
(included by `headless`/`full`, absent from `minimal`).

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream `stumpapp/stump` `crates/graphql` (merge base `37fdb7d7`) | Fork keeps upstream types; additions are ingest operations, typed event subscription, metadata-provider mutations, reading lists, `SERVICE_UNAVAILABLE` error extension. |
| `crates/graphql/schema.graphql` (4174 lines, generated) | Contract consumed by `packages/graphql/codegen.ts` and `packages/browser/codegen.ts`; CI runs `cargo dump-schema -- --check` (`.github/workflows/ci.yaml:65`). |
| `docs/content/docs/developer/api.mdx` | Public description of the API, playground availability and auth methods. |
| `async-graphql` 7.2.1 (`chrono`, `decimal`, `dynamic-schema`, `dataloader`) | Workspace-pinned in root `Cargo.toml`. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Schema is built lazily once per server (`OnceCell` around `build_schema`) and `build_schema_bare` exists for codegen without a `Ctx` | Schema construction is not needed until the first GraphQL request; the generator must run without a database. | `src/schema.rs:27,101-111`; `apps/server/src/routers/api/graphql.rs:94,113` |
| Guards (`ServerOwnerGuard`, `SelfGuard`, `PermissionGuard`, book-club role guards) read `stump_auth::AuthContext` from request data | One auth type shared with REST/protocol handlers; guards never re-authenticate. | `src/guard.rs` (5 `impl Guard`) |
| `CoreError::FeatureDisabled` maps to `SERVICE_UNAVAILABLE` + `feature` extension | Optional services (jobs, watcher, providers) can be dormant in headless profiles; clients need a machine-readable reason. | `src/error.rs:8-19` + test |
| GraphQL `OffsetPagination` is a separate `InputObject` converting to/from `stump_api_types::OffsetPagination`; cursor/`Unpaginated` variants are GraphQL-only | Keeps the neutral crate free of async-graphql while sharing defaults and arithmetic. | `src/pagination.rs:33-80` |
| Dataloaders for author/favorite/library/media/series counts registered in `add_data_loaders` | Avoids N+1 on list views; `tokio::spawn` executor. | `src/schema.rs:42-99`, `src/loader/` |
| `AccessRole` registered manually as an output type | It only appears inside serialised SmartList JSON, so async-graphql cannot discover it. | `src/schema.rs:107-109` |
| Subscriptions merged from `LogSubscription`, `EventSubscription`, `IngestSubscription` | Typed core events replace ad-hoc polling for jobs/ingest progress. | `src/subscription/mod.rs:10`; commit `94a81c1a` |
| Depends on `models`/`email`/`metadata_integrations`/`stump_core` with `features = ["graphql"]` explicitly | Those crates default to no GraphQL derives; this crate is the only place that needs them. | `Cargo.toml` dependency block |
| Filter enums generated with `filter-gen` proc macro | Filter/ordering contracts track entity columns without hand-maintained duplicates. | `src/filter/`, `crates/macros/filter-gen` |
| Dead provider sources are filtered out of `providerCatalog`/`providerSourceHealth` unless `includeDead: true`, and `runProviderHealth` enqueues `StumpJob::ProviderSourceHealth` instead of probing inline | A resolver must not hold a request open for a catalog-wide probe, and an operator should not be offered a source that failed `provider_health_dead_after` runs. | `src/query/provider.rs::provider_catalog`, `src/mutation/provider.rs::run_provider_health` |
| Every mutation transaction opens with `models::txn::begin_write` (SQLite `BEGIN IMMEDIATE`); the read-only snapshot transactions in `query/smart_lists.rs` and `object/smart_lists.rs` keep `TransactionTrait::begin` | Mutations such as `cleanLibrary`, `resetLibraryMetadata` and the reading-progress resolvers read before they write, and SQLite refuses to run the busy handler when a deferred transaction has to promote its read snapshot to the write lock. Making the read-only smart-list transactions `IMMEDIATE` would instead serialise readers behind writers. | `src/mutation/*.rs`; `crates/models/src/txn.rs` |

## Layout

| Path | Responsibility |
| --- | --- |
| `src/schema.rs` | `AppSchema`, `build_schema`, `build_schema_bare`, dataloader registration |
| `src/query/`, `src/mutation/`, `src/subscription/` | Root resolvers per domain (library, media, series, user, book club, ingest, metadata provider, reading list, smart list, jobs, …) |
| `src/object/` | Output types wrapping `models` entities |
| `src/input/` | Input objects |
| `src/filter/`, `src/order.rs`, `src/pagination.rs` | Query shaping |
| `src/guard.rs`, `src/error.rs`, `src/error_message.rs` | Authorization guards and error mapping |
| `src/loader/` | Dataloaders |
| `src/data.rs` | `CoreContext = Arc<stump_core::Ctx>` |
| `bin/main.rs` | `graphql-gen`: write or `--check` `schema.graphql` |
| `schema.graphql` | Generated SDL (commit alongside resolver changes) |
| `src/tests/` | Shared test context helpers |

## How to verify

```text
cargo test -p graphql --lib                     # 105 unit tests (guards, pagination, error mapping, resolvers)
cargo dump-schema -- --check                    # alias for `cargo run -p graphql --bin graphql-gen -- --check`
cargo dump-schema                               # regenerate schema.graphql after resolver changes
cargo check -p stump_server --no-default-features --features minimal   # crate must be absent
cargo build -p stump_server --no-default-features --features headless,liseur-sync
```

Live probe: `POST http://127.0.0.1:25600/api/graphql` on the `stump-komga`
fixture (playground only with web UI + `enable_playground`). No Hurl replay
targets GraphQL; the harness covers protocol surfaces only.

## Deep docs

- `docs/content/docs/developer/api.mdx` — playground, auth methods
- `docs/content/docs/developer/server-architecture.mdx` — feature profiles, lazy schema
- `docs/content/docs/developer/modular-ingest.mdx` — ingest operations and subscription
- `crates/api-types/README.md`, `crates/auth/README.md`, `crates/models/README.md`
