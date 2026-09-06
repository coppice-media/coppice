# ingest

| | |
| --- | --- |
| **Package** | `stump_ingest` (`crates/ingest`) |
| **Purpose** | The staged ingest pipeline: drop-folder discovery, immutable staging, the optional preprocess hook, the durable analysis queue and its replayable progress stream, deterministic quality checks, metadata providers and candidate storage, and field-level apply. It owns the whole workflow from "a file appeared" to "a media row exists". It deliberately does **not** own file processing (`stump_media`), library-row construction, the server encryption key, `StumpConfig`, or the client event contract — those are the host's, injected through `host::{RowFactory, ProviderClientFactory}`, `config::IngestSettings` and `event::IngestEventSink`. |
| **Reference / upstream** | Komf per-field metadata aggregation (`policy`): <https://github.com/Snd-R/komf/blob/master/README.md#metadata-aggregation>. Quality scoring contract and algorithm version in `src/contract.rs` (`ingest-quality-2`). Editor contract: `docs/content/docs/developer/modular-ingest.mdx`. Provider clients come from `metadata_integrations` (Comic Vine, Hardcover, AniList, MAL, MangaDex, MangaUpdates). |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| The crate has no `stump_core` dependency; the dependency points `stump_core` -> `stump_ingest` | `Ctx::ingest()` and `StumpConfig` live in core, so the reverse edge would be a cycle; keeping the pipeline host-free is also what makes the `minimal` server profile able to drop it entirely | `Cargo.toml` (no `stump_core`); `core/Cargo.toml` feature `ingest = ["dep:stump_ingest"]`; `cargo tree -p stump_server --no-default-features --features minimal -i stump_ingest` finds no package |
| Own error type `IngestError`, converted by the host | House style (`stump_jobs`, `stump_media`); GraphQL's ingest mappers are generic over `Display`, so no resolver changed | `src/error.rs`; `impl From<IngestError> for CoreError` in `core/src/ingest_host.rs`; `crates/graphql/src/query/ingest.rs::map_store_error` |
| Library rows are built through `host::RowFactory`, not by this crate | A committed staged file must be indistinguishable from a scanned one, and the scanner's `MediaBuilder`/`SeriesBuilder` need `StumpConfig` and the library config; the trait keeps that in core while `approve` keeps orchestrating | `src/host.rs`; `src/store.rs::approve`; `core/src/ingest_host.rs::CoreRowFactory` |
| Provider clients are built through `host::ProviderClientFactory` | Constructing one means decrypting a stored API token with the server encryption key; the key and the encryption scheme are core's | `src/providers/registry.rs::client_for`; `core/src/ingest_host.rs::CoreProviderClients` |
| The pipeline emits `IngestEvent` into a host sink, not `CoreEvent` | `CoreEvent` is a GraphQL union and a serialized client contract; mapping happens once, in the host | `src/event.rs`; `core/src/ingest_host.rs::CoreEventSink` |
| `IngestSettings` is a resolved struct (absolute directories, timeout seconds, `MediaConfig`), not a config-file type | The directory defaults hang off the host's config directory; resolving them once at the boundary keeps `Stump.toml`/env parsing out of this crate | `src/config.rs`; `core/src/ingest_host.rs::ingest_settings` |
| A configured-but-unusable preprocess command fails startup | A typo must stop the server, not silently fail every dropped file for the rest of its uptime | `src/preprocess.rs::validate_config`, called from `stump_core::StumpCore::init_config` via `ingest_host::validate_ingest_config` |
| The file move into the library happens inside the write transaction, with a move-back on failure | The media builder must see the file at its final path; a failure after the move would otherwise leave the database and the filesystem disagreeing | `src/store.rs::approve` |
| At most two analysis jobs run the snapshot/check/provider path concurrently | The path decodes pages and hits remote providers; unbounded concurrency starves the rest of the server | `src/coordinator.rs` (`Semaphore::new(2)`) |
| Quality reports carry an algorithm version (`ingest-quality-2`) | Weights and thresholds change; an old score must never be silently reinterpreted | `src/contract.rs::QUALITY_ALGORITHM_VERSION`, stamped in `src/quality/registry.rs` |
| Duplicate-page detection uses `stump_media::DUPLICATE_PAGE_TOLERANCE` | The quality check, the candidate aggregation, and the serve-time page skip must agree on what "the same page" is | `src/quality/duplicate_pages_across_books.rs`; `crates/media/src/hash.rs` |
| A field's provider order lives per field (`policy.rs`), not per provider as in Komf | Komf keys `priority` + a `seriesMetadata`/`bookMetadata` toggle map per provider; transposed, "who wrote this field" is one lookup instead of a scan across every provider's toggle map, and a library override can name one field without restating the provider table | `src/policy.rs`; tests `priority_order_decides_the_winner`, `per_library_override_beats_the_server_default` |
| `HIGHEST_RESOLUTION` ranks only what a cover value *advertises* (explicit `width`, a `WxH` token, a size component before the extension, `w=`/`width=`), unknown last | A provider returns a URL, not an image; the alternative is downloading every candidate cover inside a resolver, and `super_url`/`extraLarge`/`-L` are each a provider's own largest variant, so they carry no comparable pixel count | `src/policy.rs::advertised_cover_width`; tests `advertised_cover_width_reads_only_documented_forms`, `unknown_cover_sizes_fall_back_to_priority`, `cover_picks_the_largest_advertised_resolution` |
| `MERGE_UNION` unions the stored value first, then each allowed provider, deduped case-insensitively with the first spelling kept | The pick it emits is a `MANUAL` value, which bypasses `MergeStrategy` on the way to the row, so excluding the stored list would silently delete the user's own tags | `src/policy.rs::union`; test `union_dedupes_case_insensitively_and_keeps_stored_values` |
| One resolver (`MetadataPolicy::plan`) produces the `FieldPick` recipe for both auto-apply and the editor's "apply best" | Two resolvers would drift into two merge semantics and two audit shapes; a plan reuses the recipe the editor already submits by hand, so validation, merge, and the audit row are unchanged | `src/policy.rs::plan`; `crates/graphql/src/mutation/ingest.rs::{auto_apply_policy, apply_best_ingest_metadata}` |
| Every `ingest_drop_item` write goes through `IngestStore::persist`/`announce`, which emits `IngestEvent::ItemChanged` on each persisted `revision` bump | A client cannot keep a drop queue live by watching statuses alone — an attached quality report or a preprocess rewrite is just as stale-making; a single choke point makes "every write is announced" structural instead of a rule someone remembers. Deletions (`discard`) are not announced: the row is gone and the client that issued the discard already knows | `src/store.rs::{persist,announce}`; `src/event.rs::IngestEvent::ItemChanged`; sink wiring in `src/services.rs::with_event_sink` (rebuilds the coordinator so its `IngestStore` clone is the one carrying the sink) |

## Layout

| File | Responsibility |
| --- | --- |
| `contract.rs` | The shared vocabulary: `BookSnapshot`, `IngestMediaKind`, `MetadataField`/`FieldPick`, `QualityCheck`/`QualityReport`, `IngestProgressEvent`, provider traits, `score_report` |
| `config.rs` | `IngestSettings` (resolved host configuration) |
| `error.rs` | `IngestError`, `IngestResult` |
| `event.rs` | `IngestEvent` and the `IngestEventSink` the host implements |
| `host.rs` | `RowFactory`, `ProviderClientFactory` — the two things the host must supply |
| `services.rs` | `IngestServices`: the host-facing facade (store, coordinator, registries) and `approve` |
| `drop_folder.rs`, `staging.rs` | Symlink-free drop-folder traversal; immutable staged copies and content hashes |
| `preprocess.rs` | The optional per-item preprocess hook (child process under a timeout) |
| `store.rs` | All persistence: drop items, analysis jobs, quality reports, candidates, applications, plugin settings, snapshots, `approve`/`reject` |
| `coordinator.rs` | Durable analysis jobs: phase progression, concurrency limit, event emission |
| `progress.rs` | Typed progress events, retention, replay by cursor, live broadcast |
| `quality/` | The built-in checks and their registry (cover, page counts, image dimensions, EPUB TOC, filename parse, DRM, duplicates within and across books, series gaps) |
| `providers/` | `registry` (discovery, settings, identify/lookup), `builtin_embedded`, `facade` (metadata-integrations adapter), `llm`, `apply` (field-level writes with audit rows) |
| `policy.rs` | The per-field metadata policy: rule types, the Komf-mirroring server default, per-library override storage (`library_configs.metadata_policy`), and `plan` — the single resolver behind auto-apply and "apply best" |

## How to verify

```bash
cargo test -p stump_ingest                       # pipeline, quality checks, providers, apply, policy
cargo test -p stump_core --lib ingest            # host glue (settings, row factory, event mapping)
cargo check -p stump_server --no-default-features --features minimal          # pipeline absent
cargo check -p stump_server --no-default-features --features headless,liseur-sync
cargo tree -p stump_server --no-default-features --features minimal -i stump_ingest   # must not match any package
```

## Deep docs

`docs/content/docs/developer/modular-ingest.mdx` — the editor contract, the
staged workflow, quality scoring, and provider settings.
