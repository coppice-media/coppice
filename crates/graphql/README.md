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
(included by `headless`/`full`, absent from `minimal`), and inside it feature
`web` (default on, enabled by the server's `webui`) carries the SPA-only
resolvers, so a `headless` build serves the schema the SvelteKit `editor`/`home`
apps and the protocol adapters need and nothing else.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| Upstream `stumpapp/stump` `crates/graphql` (merge base `37fdb7d7`) | Fork keeps upstream types; additions are ingest operations, typed event subscription, metadata-provider mutations, reading lists, `SERVICE_UNAVAILABLE` error extension. |
| `crates/graphql/schema.graphql` (4174 lines, generated with `--all-features`, so it is the *full* schema) | Contract consumed by `packages/graphql/codegen.ts`, `packages/browser/codegen.ts`, `editor/codegen.ts` and `home/codegen.ts`; CI runs `cargo dump-schema -- --check` (`.github/workflows/ci.yaml:65`). |
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
| `sendToKindle` mails one book through the primary emailer and `stump_notify::EmailChannel`, converts EPUB → AZW3 only when the operator's `boko` is present, and reports the fallback in `note` instead of failing | A second SMTP path would drift from `sendAttachmentEmail`'s size cap, forbidden-recipient list, and send records; Amazon accepts EPUB and converts it itself, so a server without the GPL converter installed must still deliver a readable book. The conversion is `Tool::plan`/`apply` on `spawn_blocking` into a temporary under the cache dir. | `src/mutation/kindle.rs`, `src/mutation/emailer/sender.rs` (`get_emailer`, `build_emailer_client_config`, `check_forbidden_recipients`, `update_send_records` are `pub(crate)` for it); tests `cargo test -p graphql --lib kindle` |
| Feature `web` (default on, forwarded by the server's `webui`) gates the resolvers no client but the upstream React SPA (`packages/browser`) and the Expo app reaches. Gated modules: `query/{book_club, book_club_book, book_club_discussion, book_club_invitation, book_club_suggestion, custom_emoji, smart_list_view, smart_lists, smart_lists_builder}`, `mutation/{book_club, book_club_book, book_club_discussion, book_club_invitation, book_club_member, book_club_suggestion, custom_emoji, smart_list_view, smart_lists, upload}`, `object/{book_club, book_club_book, book_club_book_suggestion, book_club_discussion, book_club_discussion_message, book_club_invitation, book_club_member, custom_emoji, smart_list_item, smart_list_view, smart_lists, user_preferences}`, `input/{book_club, smart_list_view, smart_lists}`. Gated items inside shared files: `guard::BookClubRoleGuard`, `UserMutation::{update_viewer_preferences, update_navigation_arrangement, update_navigation_arrangement_lock}` with `update_user_preferences_by_id`, `User::preferences`, `input::user::{UpdateUserPreferencesInput, NavigationArrangementInput}`, `utils::save_user_session`, the `CursorPaginatedBookClubDiscussionMessageResponse` concrete, and the manual `AccessRole` registration. A headless build drops 5,748 lines of resolver source (5,476 in the 34 wholly gated modules + 272 in the cfg'd blocks) | The ungated set is defined by what actually calls it: every root field, object field and input in the SvelteKit `editor/`, `home/` and `packages/stump-ui` `.graphql` documents, plus everything the OPDS/Komga/Kobo/KOReader/liseur-sync adapters reach, stays compiled. Book clubs, smart lists + views, custom emoji, viewer preferences/navigation arrangement and the multipart upload lane appear in no such document, so a server built without the SPA has no caller for them. Subscriptions need no arm: none of the four (`log`, `event`, `ingest`, `device`) belongs to a gated domain. | `Cargo.toml` `[features] web`; `apps/server/Cargo.toml` `webui = ["graphql?/web"]`; `cargo check -p graphql --no-default-features` |
| `schema.graphql` is the *full* schema: `cargo dump-schema` runs with `--all-features` | One checked-in SDL has to be a superset for every client's codegen, whatever a given server build compiles; a feature-cut SDL would silently break `packages/graphql`, `packages/browser`, `editor/` and `home/` codegen depending on who regenerated it last. Clients that call a resolver the server did not compile get a normal "unknown field" GraphQL error, which is the same failure mode as calling a `providers`-less build. | `.cargo/config.toml` `dump-schema` alias; `bin/main.rs` |

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
cargo dump-schema -- --check                    # alias for `cargo run -p graphql --bin graphql-gen --all-features -- --check`
cargo dump-schema                               # regenerate schema.graphql after resolver changes
cargo check -p graphql --no-default-features    # no `web`, no `providers`
cargo check -p stump_server --no-default-features --features minimal   # crate must be absent
cargo build -p stump_server --no-default-features --features headless,liseur-sync   # graphql without `web`
```

Live probe: `POST http://127.0.0.1:25600/api/graphql` on the `stump-komga`
fixture (playground only with web UI + `enable_playground`). No Hurl replay
targets GraphQL; the harness covers protocol surfaces only.

## Deep docs

- `docs/content/docs/developer/api.mdx` — playground, auth methods
- `docs/content/docs/developer/server-architecture.mdx` — feature profiles, lazy schema
- `docs/content/docs/developer/modular-ingest.mdx` — ingest operations and subscription
- `crates/api-types/README.md`, `crates/auth/README.md`, `crates/models/README.md`
