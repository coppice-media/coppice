# Stump Project State

This is the on-demand state file for the headless/modular Stump worktree. It is a
snapshot, not a substitute for source, tests, or the developer contracts in
`docs/content/docs/developer/`.

## Working tree

- The checkout was already intentionally dirty: `git status --short` reported
  228 paths before this control-plane update (approximately 230). Do not reset,
  clean, or assume an application-source baseline.
- This update is limited to the six files under `.omp/`: the four injected
  context/agent files plus this state file and `NEXT_STEPS.md`.
- Local working-tree paths may be cited without commit pins until this fork is
  committed. External client/repository references must remain pinned and
  immutable.

## Fixture status before the current batch

The supervised fixture `stump-komga` is live on a binary built before the current
batch. The live profile has the provider-crate Komga surface, the nested Grimmory
`/komga` alias, OPDS 1.2 and 2.0, KOReader, Kobo initialization/sync/file routes,
and liseur-sync login. It also includes the Basic-auth cache, Komga remember-me,
virtual book URLs, and SSE mutation events. Source anchors are
`apps/server/src/routers/komga/mod.rs`, `crates/komga/src/routes/grimmory.rs`,
`apps/server/src/routers/opds_backend.rs`,
`apps/server/src/routers/koreader_backend.rs`,
`apps/server/src/routers/kobo_backend.rs`,
`apps/server/src/routers/kobo_backend/router.rs`,
`apps/server/src/routers/liseur_sync/mod.rs`,
`apps/server/src/middleware/auth.rs`,
`crates/komga/src/routes/mapper.rs`, and
`crates/komga/src/routes/sse.rs`.

`apps/server/src/routers/mod.rs` mounts liseur-sync whenever its Cargo feature is
compiled. `scripts/dev-fixture-server.sh` regenerates owner-only generated
`ENDPOINTS.md` with mode 0600 on every start and defaults `STUMP_ENABLE_KOMGA`,
`ENABLE_KOBO_SYNC`, and `ENABLE_KOREADER_SYNC` to true; the latter two names
intentionally have no `STUMP_` prefix. Never copy or quote that generated sheet.

## Evidence already reported for that binary

- `cargo clippy` with `-D warnings` passed for the provider crates, `stump_core`,
  `migrations`, and `stump_server` on the headless/liseur-sync profile.
- Provider crate suites passed: Komga 60 tests, Kobo 7, liseur-sync 4, and models
  75. The server `--lib` suite passed with the reported 72–78-test profile range.
- The external replay harness passed all 8 observed specs (`make replay`, 8/8).
  The harness is the sibling user-owned `../komga-compat/`; Hurl is its semantic
  test oracle, not a capture file.

## Landed and verified live (fixture `stump-komga`, headless build)

All of the previous "pending gate" batch is live and replayed (`make replay`
8/8, negative-auth 1/1), plus:

- KEPUB: `stump_kepub` is a verified 1:1 port of kepubify @9546034 (Go test
  tables ported; `crates/kepub/tests/real_books.rs` compares every ZIP entry
  byte-for-byte against a pinned kepubify build over `input/*.epub`, 35/35
  identical, 1 book kepubify itself rejects). Arena DOM (`crates/kepub/src/dom.rs`),
  own ZIP writer (`crates/kepub/src/archive.rs`, libdeflate), bounded rayon
  batches, `transform_epub_to(sink)`. 36-book corpus: 1.50× kepubify single-core,
  2.75× on 8 cores, lower peak RSS. The kepubify shell-out and
  `KOBO_KEPUBIFY_PATH` are removed. Kobo delivery converts straight into the
  cache file and serves it with `ServeFile` (range requests); warming on sync
  entitlements, unused-for-N-days eviction, `KOBO_KEPUB_PRECONVERT`,
  `KOBO_KEPUB_DEFLATE_LEVEL` (`apps/server/src/routers/kobo_backend/kepub_cache.rs`).
- Staged ingest (`docs/content/docs/developer/modular-ingest.mdx`, now marked
  implemented): contract `core/src/ingest/contract.rs`; entities
  `crates/models/src/entity/ingest_*.rs`; migration
  `m20260907_000000_add_ingest`; staging/drop folder/store/coordinator/progress
  ring under `core/src/ingest/`; seven deterministic quality checks
  (`core/src/ingest/quality/`); provider façade + `builtin:embedded` +
  field-pick apply (`core/src/ingest/providers/`); GraphQL `ingest*` operations,
  `ingestProgress` subscription and `GET /api/v2/ingest/events` SSE. Verified
  live: drop folder → scan → analyze (scores 85/92) → rework list →
  `applyIngestMetadata` → `approveIngestItem` commits media with picked
  title/series/tags, file lands in the library and downloads via the Komga alias.
  Config: `INGEST_DROP_DIR`, `INGEST_STAGING_DIR`, `INGEST_PROGRESS_RETENTION`.
- Editor: SvelteKit 2 / Svelte 5 / Tailwind 4 / shadcn-svelte static app in
  `editor/` (outside the yarn workspace glob; Bun; Vite pinned to 7 because
  Vite 8's rolldown transform walks into the repo-root tsconfig chain). Routes
  `/drop`, `/queue`, `/rework`, `/bulk`, `/settings/providers`, login/logout,
  dev proxy to :25600 on port 5174. Browser-verified against the fixture.

Also verified live on the same fixture: `stageIngestUploads` multipart (with
`relativePath`, auto-analysis, `STUMP_ENABLE_UPLOAD=true` now exported by the
launcher), dedup of identical re-uploads (`deduplicated` counted, staged file
preserved — the first version deleted it), `bulkApplyIngestMetadata` (per-item
failures with no partial write for unsupported fields), `requeueIngestAnalysis`,
`rejectIngestItem` (file moved to `staging/rejected/`, approve refused after),
and `discardIngestItem`.

The built editor is served by the headless server itself: `INGEST_EDITOR_DIR`
(`apps/server/src/routers/ingest_editor.rs`, mounted under `/editor`, SPA
fallback, immutable caching for `_app/immutable`); the app is built with
`paths.base = /editor` and all navigation is base-aware (`$app/paths` `resolve`).
Browser-verified against the fixture without the dev proxy: unauthenticated
`/editor/drop` → `/editor/login?returnTo=…` → drop/queue/settings.

Client verification is recorded in
`docs/content/docs/developer/client-verification.mdx` (device/harness/source
matrix). 2026-09-04 device results: Komelia reads + downloads CBZ and EPUB
(after four real-EPUB fixes in the Komga adapter: EPUB page list `[]`, Readium
manifest normalized to komga-client types — `rel` string, `published`
calendar date, contributor lists); Liseur works over Komga profile, OPDS 1.2,
and KOReader sync; Liseur has no OPDS 2 parser (app limit); liseur-sync logs
in but the catalog routes (`/v1/folders`, `/v1/books/*`) were never implemented
and `/v1/works/{id}/annotations` has a SQL bug — worker `LiseurCatalog` is
implementing both against the pinned Liseur client.

## Landed 2026-09-04 (late): `stump_scanner` + lazy lifecycle; repo checkpointed

- `crates/scanner` (`stump_scanner`): ScanOptions, `ScanSource` trait (existing_series/existing_media with scanner-owned `ScanStatus`, stored_dir_mtimes), pure walker + mtime reconciliation, TagCache; 12 contract tests. Core keeps jobs/utils/watcher and the SeaORM `ScanSource` adapter (`core/src/filesystem/scanner/store.rs`).
- Lazy lifecycle: `Ctx` unconditionally owns config/conn/events only. `JobRuntime` (storage/state/Apalis monitor + shutdown Notify) created on first enqueue; watcher constructed only when jobs enabled and a library watches (or first add_watcher); scheduler only for enabled scheduled-job rows, reloads on config mutation; server stops only what it created. `/api/v2/health` reports `jobs` (disabled/enabled/idle/running) and `watcher` (disabled/inactive/active). Verified live: health enabled→idle across the first post-boot scanLibrary, job COMPLETED without restart.
- Idle RSS delta from the lifecycle change is noise (62→66 MB debug on a copied fixture, same 19 threads); the win is structural, not the RAM number.
- Gate on the tip: workspace check all-targets, clippy -D warnings on our crates, media 134 / scanner 12 / core 146 / komga 68 / liseur 8 / graphql 91 / server 81 tests, headless build; four replays green on the deployed build.
- Git: 10 commits on `headless-modular` (ahead of origin/nightly), omp identity, nothing pushed. Upstream `.gitignore` `lib/`/`static` patterns had excluded 118 editor sources — negated. `/input/` (personal books) ignored.

## Landed 2026-09-04 (night): `stump_media` crate

- `crates/media` (`stump_media`): format processors, ContentType/FileError, hashing, Readium, EPUB search, image primitives, `MediaConfig`; features `pdf`/`rar` (default on). No dependency on core/jobs/DB. 134 tests default, 121 with `--no-default-features` (feature-off contracts covered).
- `StumpConfig.media` snapshot via `finalize_media_config` (init_config + Ctx::new). Every processor caller migrated (core, graphql, server, benches); no re-export facade left in core.
- Server features: `formats = ["pdf","rar"]` in every profile; `headless` deps 779 with formats, 770 without. `crates/cli` no longer re-enables core defaults.
- Live: all four replays green on the deployed build; CBZ/EPUB/PDF page + thumbnail + file paths probed; PDF ingest end-to-end (rust_book.pdf: analysis score 100, approve, 671 rendered pages) — fixture launcher now exports `PDFIUM_PATH` (default `/tmp/libpdfium.so`).
- Studies: `'/home/al/.omp/agent/sessions/-Code-stump/2026-09-01T22-32-48-845Z_01a05f1a-85cd-766c-8f2a-214be248b41e/local/scanner-split-plan.md'` (stump_scanner/stump_watcher seams, lazy lifecycle, PR-sized steps), `'/home/al/.omp/agent/sessions/-Code-stump/2026-09-01T22-32-48-845Z_01a05f1a-85cd-766c-8f2a-214be248b41e/local/feature-gaps.md'` (plan.txt status, comparator gaps, ops gaps).
- Known upstream lints untouched: `emailer/sender.rs` and `tests/reading_progress/manual_progression_changes.rs` (`useless vec!`).

## Landed 2026-09-04 (evening)

- Mihon/Keiyoushi Komga extension: legacy GET catalog routes (`/api/v1/series`, `/api/v1/books` root-only via `legacy_books_router`, `/api/v1/series/{id}/books`, `/api/v1/authors`), device-verified browse/download/read.
- Mihon built-in Komga tracker: `GET/PUT /api/v2/series/{id}/read-progress/tachiyomi` and readlist variant; Basic/API-key scope widened to that path (`is_komga_basic_auth_path`). Harness passes cold; device retest pending.
- Liseur read-only catalog (`/v1/folders`, `/v1/folders/{id}/books`, `/v1/books/{id}` detail/cover/download/resolve, series) with `library-read` scope; annotation insert column list repaired (`storage.rs` `INSERT INTO liseur_sync_annotations`).
- Harness: all four replays (`replay`, `replay-negative-auth`, `replay-mihon`, `replay-liseur-sync`) green and rerun-stable on the deployed fixture; `replay-mihon` needs `SERIES_ID`.

## Pending

- The `/bulk` editor grid has not been driven in the browser (its mutation has).
- Rework detail sheet shows a large blank band above the header at 1280×900.

## Only definition of green

The coordinator runs this gate once after all workers yield; every command must
exit 0. Workers do not run formatters, linters, builds, or project-wide suites
mid-batch:

```text
cargo fmt --all
cargo clippy -p stump_komga -p stump_opds -p stump_kobo -p stump_koreader -p stump_liseur_sync -p stump_kepub -p stump_core -p migrations -p stump_server --no-default-features --features headless,liseur-sync -- -D warnings
cargo test -p stump_komga --lib --tests
cargo test -p stump_opds --lib --tests
cargo test -p stump_kobo --lib --tests
cargo test -p stump_koreader --lib --tests
cargo test -p stump_liseur_sync --lib --tests
cargo test -p stump_kepub --lib --tests
cargo test -p stump_core --lib --tests
cargo test -p migrations --lib --tests
cargo test -p models --lib --tests
cargo test -p stump_server --lib
cargo build -p stump_server --no-default-features --features headless,liseur-sync
```

After a successful build, deploy via a `hub` restart of the supervised fixture
named `stump-komga`, then run `make replay` from `../komga-compat`; green
requires 100% of the observed replay.
For structural changes, also run `cargo check -p stump_server --no-default-features --features minimal`
and `cargo check -p stump_server` for the default profile. A source-only pass is
not a deployment or replay result.

## Known gaps and caveats

- Komga collection creation is not mounted: `crates/komga/src/routes/lists.rs`
  exposes collection GET routes and `PATCH /api/v1/collections/{id}`, but no
  POST collection route.
- The Komga filesystem browser is partial and owner-only:
  `crates/komga/src/routes/media.rs` accepts an absolute directory and returns
  safe child directories, not a general file browser.
- There is no `pdf` Cargo feature in `apps/server/Cargo.toml`; PDFium remains an
  unconditional core dependency in `core/Cargo.toml` and
  `core/src/filesystem/media/format/pdf.rs`. The attempted PDF feature gate is
  therefore dropped, not a current compile boundary.
- GraphQL is feature-gated, not removed: `apps/server/Cargo.toml` defines the
  feature and `apps/server/src/routers/api/mod.rs` conditionally mounts it.
- KOReader fixture sync requires `generate_koreader_hashes` before rescanning;
  the CBZ processor currently returns no KOReader hash because the branch is
  commented in `core/src/filesystem/media/format/zip.rs`.
- The Komga mount test in `apps/server/src/routers/komga/mod.rs` must remain: Axum
  panics while building overlapping routes, before a request can expose it.

## Contract evidence

Client-facing claims are checked against pinned client sources, not Komga's
OpenAPI in the abstract: Komelia `65f92fde`, `komga-client` 0.11.0
`74412a6e`, Liseur `31f8182d`, and Grimmory main. The repository's detailed
records are `docs/content/docs/developer/komga-compat.mdx`,
`docs/content/docs/developer/kobo-sync-capabilities.mdx`,
`docs/content/docs/developer/kobo-device-database.mdx`,
`docs/content/docs/developer/unified-reading-state.mdx`,
`docs/content/docs/developer/liseur-sync-integration.mdx`,
`docs/content/docs/developer/liseur-providers.mdx`,
`docs/content/docs/developer/modular-ingest.mdx`, and
`docs/content/docs/developer/server-architecture.mdx`.
