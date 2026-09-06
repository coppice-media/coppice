# Stump Project State

On-demand resume file. The evidence snapshot lives in
`docs/content/docs/developer/state.mdx`; the peer comparison in
`docs/content/docs/developer/comparison.mdx`; the roadmap in
`docs/content/docs/developer/roadmap.mdx` (+ `## Gaps vs peers`).

## Working tree

- Branch `headless-modular` @ batch 6 (Kamigura EPUB/series-by-ids fixes, tools parity webp-convert/calibre-polish/meta-edit, boko shell-out, native MOBI/KF8 reader + mobi2epub, epub-polish, per-device library scope, annotations hub + sink settings, liseur attachment lane + NickelStump 6ea9eed, ExternalTool, ingest preprocess hook, Kavita Formats filter, docs site build, send-to-Kindle; batch 5: Console UI in home/: dashboard, library/series/entities, foliate reader, notifications settings; series reshape service core/src/series.rs; NickelStump pass 2 in ../nickelstump @ 0170361; batch 4: SQLite BEGIN IMMEDIATE via `models::txn::begin_write`, migration on a 1-connection pool, Kavita search/bookmarks/collections/scan/stats/annotation routes, provider source-health job + cross-source dedupe/merge), nothing pushed.
  Clean tree at that commit. `git config core.hooksPath /dev/null` is set on
  purpose: upstream's husky hook runs prettier/cargo-fmt on every commit and
  aborts on generated files; the gate replaces it.
- Dev profile: `incremental = false` + line-tables debuginfo (root Cargo.toml)
  after a 261 GB `target/` killed the session twice. Workers never override
  CARGO_INCREMENTAL; check `df -h /` before big builds and stop under 20 GB.
  Prune `target/debug/{incremental,deps,build,.fingerprint}` only when no
  cargo process runs (`pgrep -x rustc`). Root disk at 100% shows up as a
  linker failure, not as ENOSPC.

## Fixture and launcher

- One instance, every profile on: `hub` process `stump-komga`, port 25600,
  launcher `scripts/dev-fixture-server.sh`, binary `target/debug/stump_server`
  (headless,liseur-sync). Launch env: `STUMP_ENABLE_KAVITA=true
  STUMP_ENABLE_BACKGROUND_JOBS=true STUMP_ENABLE_PROVIDERS=true`; Komga,
  Kobo, KOReader default on; `/editor` and `/app` mount the built SvelteKit
  apps (`INGEST_EDITOR_DIR`, `STUMP_HOME_APP_DIR`).
- Root `$HOME/.local/share/stump-komga-test`; the launcher regenerates
  `$FIXTURE_ROOT/ENDPOINTS.md` (0600) with every URL and credential; never
  quote that sheet. The DB was upgraded in place from `m20260909` through
  `m20260923` (reading-heads backfill: 6 sessions -> 4 heads) on 2026-09-06.
- The separate 25670 Kavita instance is retired. Komf live sessions mutate
  fixture metadata (renamed "Synthetic Solo" -> "Berserk", book number 1 -> 5);
  restore via Komga PATCH before `make replay-mihon`.
- Reference containers: `kavita-ref` 25620 (admin / see
  `../komga-compat/kavita/README.md`), `komf-stump` 8085.

## Replay commands

From the user-owned sibling harness `../komga-compat/` (hurl needs
`LD_LIBRARY_PATH=/tmp`, `PATH=$HOME/.cargo/bin:$PATH`; every target needs
`BASE_URL API_KEY USERNAME PASSWORD MITM_KEEP_HOST_HEADER=true`):

```text
make replay                 # 8/8; BOOK_ID/SERIES_ID = single-book "Synthetic Solo", THUMBNAIL_ID = a freshly uploaded book thumbnail
make replay-negative-auth   # 1/1; INVALID_USERNAME/INVALID_PASSWORD
make replay-mihon           # 1/1; SERIES_ID (spec pins the Synthetic Capture Library by name; libraries[0] is now "Books")
make replay-liseur-sync     # 1/1
make replay-kavita          # 1/1
make replay-library-management LIBRARY_ROOT=<dir>   # 1/1 since batch 4
make replay-containers SERIES_ID= BOOK_ID=          # pending spec, not yet run
```

## Only definition of green

Coordinator runs once after all workers yield; every command exits 0:

```text
cargo fmt --all
cargo check --workspace --all-targets
cargo test -p <every workspace crate except stump_server>   # 31 crates, 1447 tests at c4a0b239
cargo test -p stump_server --no-default-features --features headless,liseur-sync   # 281
cargo build -p stump_server --no-default-features --features headless,liseur-sync
```

then deploy (`hub` restart `stump-komga`) and run the replay set above.
`apps/server/src/{lib,main}.rs` carry `#![recursion_limit = "256"]` because
the merged GraphQL schema overflows rustc's query depth.

## Batch-coordination rules learned

- Root `Cargo.toml` is coordinator-owned; workers send the exact line.
- Shared files (`core/src/lib.rs`, `crates/migrations/src/lib.rs`,
  `core/src/{context,event}.rs`, graphql `mod.rs`) are edited only as
  single-line hunks after a fresh read; wholesale rewrites lost `pub mod job;`,
  `pub mod library;`, the graphql `tokio` dep and the `m20260919` registration.
- `cargo dump-schema` regenerates `schema.graphql` from the whole tree: each
  worker regenerates when its resolvers compile, last writer wins.
- `Entity::insert(active_model)` bypasses `ActiveModelBehavior::before_save`;
  set `created_at` explicitly.
- `route_ci` (Kavita) lowercases literal segments only; lowercasing `{param}`
  is a matchit conflict that only surfaces at router build.

## Next (see todo / roadmap)

- Kavita: Kamigura has no EPUB reader (image lane 404s for EPUB like kavita-ref); EPUB clients are Turnleaf/Inkita/Kover via Book/* routes. Device retest pending.
- Devices carry `library_scope` (virtual pathing) and `kindle_email`; every protocol resolves visibility through `VisibilityScope` on the request AuthUser.
- Studies (plain Markdown, outside the site build): docs/drm-study.md, docs/audiobook-study.md.
- home/ `bun run build` is not concurrency-safe (wipes .svelte-kit/output): serialize builds; `rm -rf home/.svelte-kit/output` after an aborted one. GraphQL ops live per area in home/src/lib/graphql/{operations,dashboard,library,reader}.graphql; codegen unions them.
- liseur-sync has no attachment lane: Kobo markup snapshots/notebooks are described (id, anchor, sha256) not transported; a blob route is a protocol change to design.
- Write transactions go through `models::txn::begin_write` (SQLite `BEGIN IMMEDIATE`; plain `begin()` only for read-only work). Migrations run on a dedicated 1-connection pool (sea-orm-migration does not transact SQLite DDL).
- Docs site build: `detect-libc` must be `^2` for lightningcss 1.33 (`familySync`).
- DRM: detector only (`drm_protected`, weight 0, blocking); study at
  `local://drm-study.md`; removal stays out of tree by decision.

## Standing rules

- Contract evidence pinned to Komelia `65f92fde`, `komga-client` 0.11.0
  `74412a6e`, Liseur `31f8182d`, Grimmory main, Kamigura `f4baeff4`, Turnleaf
  `b54f1f71`, kavita-ref 0.9.1.4; records in
  `docs/content/docs/developer/{komga-compat,kavita-compat,kobo-sync-capabilities,kobo-device-database,unified-reading-state,liseur-sync-integration,liseur-providers,modular-ingest,server-architecture,calibre-tooling,comparison}.mdx`.
- The Komga mount test in `apps/server/src/routers/komga/mod.rs` and the
  Kavita `kavita_router_composes_without_route_collisions` test must remain.
- No GPL/AGPL code (calibre, DeDRM, BookOrbit, Grimmory, MangaManager, Sigil,
  cbzit) in this MIT tree; behaviour/specs only; calibre is shell-out only.
