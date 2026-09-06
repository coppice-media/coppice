# stump_provider

## Purpose

`stump_provider` is the remote-source host: a Mihon-shaped `Source` trait
(popular / latest / search / details / chapters / pages / page bytes), the
process-wide `ProviderHost` (source registry, bounded on-disk page cache,
Keiyoushi catalog, health probes, virtual-browse cache), materialisation of
remote series into ordinary `series`/`media` rows, virtual-library live browse,
and the GC that reclaims unread materialised series. It deliberately does
**not** own any HTTP route, DTO mapping, or auth: the Komga and OPDS branches
live in `apps/server/src/routers/provider_virtual.rs`, and GraphQL in
`crates/graphql/src/{query,mutation,object}/provider.rs`. Only MangaDex is
compiled in (`crates/provider-mangadex`); other catalog entries are listed
for operators but cannot be enabled.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| Mihon source model | `eu.kanade.tachiyomi.source.CatalogueSource` (popular/latest/search/details/chapters/pages) | `src/source.rs` trait shape and `BrowseKind` |
| Keiyoushi extension index | <https://raw.githubusercontent.com/keiyoushi/extensions/repo/index.json> (`src/catalog.rs:27-31`, refreshed daily) | `providerCatalog`, health probes |
| MangaDex API | <https://api.mangadex.org>, 5 req/s, MangaDex@Home report endpoint | `crates/provider-mangadex/src/lib.rs:1-8` |
| Komga `SeriesSearch` condition DSL / Komelia `65f92fde` | `libraryId` + `fullTextSearch` conditions | Mode B request mapping (`apps/server/src/routers/provider_virtual.rs::browse_kind`) |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Cargo feature `providers` (core, graphql, server) **and** runtime `STUMP_ENABLE_PROVIDERS` (default `false`) | The feature removes reqwest/MangaDex code from `minimal`; the switch keeps the host dormant in default builds until an operator opts in | `core/Cargo.toml` `providers`; `core/src/providers.rs::is_enabled`; `apps/server/src/http_server.rs` init call |
| Provider rows keep `path` non-null as a `provider://<source>/<remote>[/<chapter>]` URI; the host is the `VirtualMediaResolver` for that scheme | Every page-serving route (native, Komga, OPDS, Kobo) keeps calling the same media functions | `src/virtual_path.rs`; `src/host.rs` `impl VirtualMediaResolver`; `crates/media/src/virtual_media.rs` |
| Deterministic ids: `uuid5(series:<source>:<remote_id>)`, `uuid5(media:<source>:<chapter>)`, `uuid5(library:<source>)` | The same remote series has one identity across live browse, search, materialisation, and clients that cache ids | `src/virtual_path.rs::{series_id,media_id,library_id}`; test `browse_ids_are_stable_and_materialise_under_the_same_id` |
| Mode A (`addProviderSeries`) materialises into any library; Mode B virtual libraries (`createVirtualLibrary`) browse live and write nothing until a series' books are opened | Browsing a source must not bloat the database; opening a series is intent | `src/materialize.rs::add_series`; `src/virtual_library.rs`; `apps/server/src/routers/provider_virtual.rs::materialise_virtual_series` |
| Live browse pages are cached per `(source, kind, page)` for `virtual_series_ttl` (default 300 s) with a reverse index from stump id to origin | UUID v5 cannot be reversed; the index lets `series/{id}` and materialisation resolve ids the browse just served | `src/browse.rs`; `core/src/config/providers.rs::virtual_series_ttl` |
| Komga mapping: `fullTextSearch` → search, engagement sorts (`readCount`, `booksCount`, `readProgress`) → popular, everything else → latest; only a lone `libraryId is` condition routes live | Mihon exposes exactly three feeds; combined conditions are answered from the database, which has no virtual rows | `apps/server/src/routers/provider_virtual.rs::browse_kind`; `crates/komga/src/routes/catalog.rs::virtual_library_id_from_search` |
| Live results honour the per-user library filter and live cards report `booksCount = 0` | No live path may bypass `find_for_user`; the chapter count is unknown until materialisation | `provider_virtual.rs::user_can_access_library`; `komga_backend.rs::virtual_series_list` |
| Scanner skips virtual libraries and never marks `provider://` series missing | A virtual library has no filesystem root; a scheduled scan must not flag materialised rows | `core/src/filesystem/scanner/library_scan_job.rs::init`; `crates/scanner/src/walk.rs` missing-series filter |
| GC: provider-backed series with no reading head on any book and `created_at` older than `provider_gc_days` (default 30) are deleted daily (`StumpJob::ProviderGc`) | Materialised rows are a cache of the source; unread ones are reclaimable | `src/gc.rs`; `core/src/providers.rs::spawn_gc_scheduler`; `core/src/job/services.rs::run_provider_gc` |
| Page cache bounded by `provider_cache_max_bytes` (default 2 GiB), manifests re-resolved after 10 min | MangaDex@Home URLs expire after ~15 min; disk use must be capped | `src/cache.rs`; `src/host.rs::MANIFEST_TTL` |
| Health: probe timeout 10 s, 8 concurrent probes, `DEAD` after `provider_health_dead_after` (default 3) consecutive failures, one reachable run resets the count | Operators see which catalog sources are reachable before enabling one, and a single blip must not bury a source | `src/health.rs:19-24`, `HealthProbe::next_state`; `core/src/config/providers.rs::provider_health_dead_after` |
| Health runs as `StumpJob::ProviderSourceHealth`, one task per base URL, every `provider_health_interval_secs` (default 6 h) or on demand via `runProviderHealth`; the job builds its own catalog reader instead of borrowing the host | Sources sharing a host are probed once per run and enabled instances first; jobs only ever receive the database and the config, and the catalog index is a file both sides read | `src/health.rs::{plan,probe_target}`; `core/src/job/provider_health.rs`; `core/src/providers.rs::spawn_health_scheduler` |
| `DEAD` sources are hidden from `providerCatalog`/`providerSourceHealth` unless `includeDead: true`, and every entry carries its health row as a badge | An operator should not be offered a source that has been unreachable for three runs, but must still be able to inspect it | `crates/graphql/src/query/provider.rs::provider_catalog`; `crates/graphql/src/object/provider.rs::ProviderSourceHealth` |
| Dedupe is recorded, never enforced: materialisation writes `provider_series_identity` (normalised title + strongest external id) and links a match from another source in `provider_series_links` | Sources overlap heavily; a false positive must be a row an operator can inspect rather than silently merged (and lost) reading progress | `src/identity.rs::record_identity`; migration `m20260924_000000_add_provider_series_links`; test `same_title_from_another_source_is_linked_to_the_first` |
| `mergeProviderSeries(keep, drop)` replays the dropped heads onto the kept chapters through `reading_state::apply` (chapter number, then normalised title) and refuses non-provider series | The conflict rule and provenance must stay the single writer of reading state; a locally scanned series is never deleted by a provider merge | `src/identity.rs::merge_series`; tests `merge_repoints_heads_and_deletes_the_dropped_series`, `merge_refuses_local_series_and_self_merges` |
| GC and identity write transactions open with `models::txn::begin_write` (SQLite `BEGIN IMMEDIATE`) | The GC sweep and `merge_series` read the rows they are about to delete or repoint, and a deferred `BEGIN` cannot promote that read snapshot to the write lock while a scan or materialisation holds it | `src/gc.rs`, `src/identity.rs`; `crates/models/src/txn.rs` |

Routes that answer from a virtual library (Mode B):

| Route | Behaviour |
| --- | --- |
| Komga `POST /api/v1/series/list` with `libraryId is <virtual>` | live browse page, deterministic ids, no rows (`crates/komga/src/routes/catalog.rs::virtual_series_page`) |
| Komga `GET /api/v1/series/{id}` | live details while unmaterialised (`get_series_by_id` → `virtual_series_by_id`) |
| Komga `GET /api/v1/series/{id}/thumbnail` | source cover via host cache for live or stored provider series |
| Komga `GET /api/v1/series/{id}/books` | materialises on first call, then the ordinary database path (`get_series_books`) |
| Komga `GET /api/v1/books/{id}/pages/{n}` | `provider://` media resolved by the host (unchanged route) |
| OPDS 1.2 `GET /opds/v1.2/libraries/{id}` | live browse feed with phantom next-page count (`v1_2.rs::virtual_library_feed_or_none`) |
| OPDS 1.2 `GET /opds/v1.2/series/{id}` | materialises on first call, then books/PSE pages as usual |

## Layout

| File | Responsibility |
| --- | --- |
| `src/source.rs` | `Source` trait, `RemoteSeries`/`RemoteChapter`/`RemotePage`, `SourceError` |
| `src/host.rs` | `ProviderHost`, `SourceFactory`, page/cover/archive resolution, `VirtualMediaResolver` impl |
| `src/browse.rs` | `BrowseKind`, `VirtualBrowseCache` (TTL + reverse index), `RemoteOrigin` |
| `src/virtual_library.rs` | idempotent `create_virtual_library` (deterministic library id, `watch` off) |
| `src/materialize.rs` | `add_series` / `refresh_series` → `series`/`media`/metadata rows |
| `src/gc.rs` | `gc_materialised_series` + `GcReport` |
| `src/virtual_path.rs` | `provider://` URI encode/parse, deterministic ids |
| `src/cache.rs`, `src/http.rs`, `src/rate_limit.rs` | bounded disk cache, shared HTTP client, per-source limiter |
| `src/catalog.rs`, `src/health.rs` | Keiyoushi index snapshot/refresh, health plan/probe → `source_health` rows |
| `src/identity.rs` | cross-source dedupe keys, duplicate links, `merge_series` |
| `src/mock.rs`, `src/mock_http.rs` | `MockSource` for tests (feature `mock`) |

## How to verify

```text
cargo test -p stump_provider                       # 44 unit tests: browse TTL/ids, materialise, cache, GC, catalog, health, dedupe/merge
cargo test -p stump_provider_mangadex              # MangaDex mapping + rate-limit tests (no network)
cargo check -p stump_server --no-default-features --features minimal   # feature-off build
STUMP_ENABLE_PROVIDERS=true ./scripts/dev-fixture-server.sh            # live: enableProviderSource → createVirtualLibrary → Komga series/list
```

## Deep docs

- `docs/content/docs/developer/provider-host.mdx` — design, Mode A vs B, config, GC, quotas.
- `docs/content/docs/developer/provider-status.mdx` — metadata providers (a different subsystem).
