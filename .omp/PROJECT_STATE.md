# Coppice Project State

On-demand resume file. Product-facing copy is Coppice; compatibility-sensitive
repository, Rust/package/binary identifiers, environment keys, routes, storage
fields, app IDs/schemes, Docker slugs, and upstream URLs remain unchanged.
The evidence snapshot lives in
`docs/content/docs/developer/state.mdx`; the peer comparison in
`docs/content/docs/developer/comparison.mdx`; the roadmap in
`docs/content/docs/developer/roadmap.mdx` (+ `## Gaps vs peers`). Integration
taxonomy and request orchestration live in
`docs/content/docs/developer/integration-architecture.mdx`.

Evidence labels are exact: Shipped, Contract-tested, Device-tested, Source-only,
Planned, Blocked. Harness/source evidence never becomes app/device proof.

## Working tree

- Branch `integrate/upstream-nightly-2026-09-20`; an active no-commit merge of
  `origin/nightly` at `766c7347dbc0a2fc8c93d04a8f922db3b5a41ba8` is in progress.
  `HEAD`/`ORIG_HEAD` is `0526084b62dc012069a2d1f47a12dde605004010`, the
  pre-merge `headless-modular` tip; `MERGE_HEAD` is the nightly SHA. GitHub's
  compare counted 87 upstream commits; local `HEAD..MERGE_HEAD` is 88 because
  one merged release parent is included.
- The dirty tree was preserved before the merge:
  - `/home/al/Code/.stump-integration-backup/headless-modular-2026-09-20.patch`
    (sha256 `c2e57571c02727b5f476f7b462297e39bf0612c1051d82514e562edf782fb3a0`)
  - `/home/al/Code/.stump-integration-backup/headless-modular-untracked-2026-09-20.tar.gz`
    (sha256 `9aaf6d0236f0f90911f3978fb3af54d0d7d8ebc8c6742c40b2ba1155b3ea6c3b`)
    Keep these recovery artifacts unchanged.
- The integration carries Home/editor dark-first UX, OIDC-first login and
  group-permission mapping, device management and sync summaries, head-backed
  progress/session fixes, liseur annotation projection, persistent read-aloud
  sync maps plus alignment enqueue, worker claim expiry, OPDS/client
  compatibility fixes, migrations, generated GraphQL, and docs.
- `git config core.hooksPath /dev/null` is set on purpose: upstream's husky hook
  runs prettier/cargo-fmt on every commit and aborts on generated files; the
  gate replaces it.
- Upstream drift is checked without mutating Git refs by `yarn check:upstreams`.
  `scripts/upstreams.json` records reviewed commits or releases plus the
  licence/source policy; `scripts/check-upstreams.mjs` uses authenticated
  `gh api` calls and prints compare/release links.
- Scanner implementation belongs to `crates/scanner`, and watcher
  implementation belongs to `crates/watcher`. The removed upstream
  `core/src/scan` duplicates are not an ownership location.
- Dev profile: `incremental = false` + line-tables debuginfo (root Cargo.toml)
  after a 261 GB `target/` killed the session twice. Workers never override
  CARGO_INCREMENTAL; check `df -h /` before big builds and stop under 20 GB.
  Prune only `target/debug/incremental`, between batches and only when neither
  Cargo nor rustc is running. Never delete `deps`, `build`, or `.fingerprint`
  just to make space. Root disk exhaustion may surface as a linker failure
  rather than a direct ENOSPC report.

## Absorbed upstream security fixes

- Upstream security commit `3d5854228d8f314f36ed6aeb9a4714cc4c30ed50`
  is included in the merge target. Its protections now cover the user
  self-permission-escalation path, the `libraryMissingEntities` management
  guard, user-scoped `mediaMetadataOverview` results, and book-club read
  access.

## Fixture and launcher

- One instance, every profile on: `hub` process `stump-komga`, port 25600,
  launcher `scripts/dev-fixture-server.sh`, binary `target/debug/stump_server`
  (headless,liseur-sync). Launch env: `STUMP_ENABLE_KAVITA=true
STUMP_ENABLE_BACKGROUND_JOBS=true STUMP_ENABLE_PROVIDERS=true`; Komga,
  Kobo, KOReader default on; `/editor` and `/app` mount the built SvelteKit
  apps (`INGEST_EDITOR_DIR`, `STUMP_HOME_APP_DIR`).
- Root `$HOME/.local/share/stump-komga-test`; the launcher regenerates
  `$FIXTURE_ROOT/ENDPOINTS.md` (0600) with every URL and credential; never
  quote that sheet. The DB was upgraded in place through `m20260949` on
  2026-09-12.
- Komf replay mutates fixture metadata (the current synthetic series title is
  `Komf replay title`); restore the series before a later Mihon replay, or run
  Mihon before Komf as this verification pass did.
- Reference containers: `kavita-ref` 25620 (admin / see
  `../komga-compat/kavita/README.md`), `komf-stump` 8085. The Komf Hurl profile
  itself targets the Stump fixture through `KOMF_BASE_URL`.

## Replay commands

The 2026-09-20 post-merge replay passed every currently required server
contract. These are server-contract results, not physical-client proof:

- `make replay`: 8 files, 36 requests.
- `make replay-negative-auth`: 1 file, 3 requests.
- `make replay-mihon`: 1 file, 19 requests.
- `make replay-liseur-sync`: 1 file, 35 requests.
- `make replay-kavita`: 1 file, 170 requests.
- `make replay-library-management`: 1 file, 17 requests, using a disposable
  empty root removed after the run.
- `make replay-abs`: 1 file, 45 requests.
- `make replay-komf`: 1 file, 16 requests, run last because it mutates the
  synthetic fixture.
- Total: 15 files, 341 requests, 0 failures.
- `make replay-containers` remains explicitly unrun; it is a pending spec, not
  part of the current green claim.

From the user-owned sibling harness `../komga-compat/`, Hurl needs
`LD_LIBRARY_PATH=/tmp` and `PATH=$HOME/.cargo/bin:$PATH`. Runtime credentials
and IDs come from the owner-only fixture sheet and must never be committed or
quoted:

```text
make replay
make replay-negative-auth
make replay-mihon
make replay-liseur-sync
make replay-kavita
make replay-library-management LIBRARY_ROOT=<disposable-dir>
make replay-abs
make replay-komf KOMF_BASE_URL=$BASE_URL
```

## Only definition of green

The 2026-09-20 post-merge gate is green:

```text
cargo fmt --all
cargo check --workspace --all-targets
cargo test --workspace --exclude stump_server
cargo test -p stump_server --no-default-features --features headless,liseur-sync
cargo build -p stump_server --no-default-features --features headless,liseur-sync
```

The non-server workspace run passed 1,897 tests (29 ignored across 93 suites);
the headless server run passed 366 tests across four suites. Schema generation
and drift checking passed (`cargo dump-schema` and `cargo dump-schema --check`).
`yarn check-types`, Expo `check-types`, Home and Editor Svelte checks, and the
web, Home, Editor, and docs builds passed; docs prerendered 199 pages. The
authenticated `/app/dashboard` and public `/editor/` surfaces loaded in a real
browser against the fixture server. Expo native visual/device verification did
not run, and native `ktlint`/`swiftformat` are unavailable on this workstation.

After deploying the headless build, the replay set above passed 341 requests
with no failures. `make replay-containers` remains explicitly unrun.

`apps/server/src/{lib,main}.rs` carry `#![recursion_limit = "256"]` because
the merged GraphQL schema overflows rustc's query depth.

## Upstream contribution strategy

- Keep `coppice/nightly` as the long-lived branch only after this merge's
  post-merge full gate and replay set pass.
- Claim or open the issue first. Reconstruct each candidate PR branch fresh from
  `origin/nightly` rather than cherry-picking the current 41-commit integration
  block; target `nightly`, keep each change to its narrow code/tests/docs scope,
  and run the upstream-required Prettier/rustfmt checks for that PR.
- Disclose LLM assistance in the PR body, never use an LLM-signed commit, and
  never submit `integrate/upstream-nightly-2026-09-20` as a PR branch.

Ordered branch map after the integration merge is committed locally:

1. `coppice/nightly` — long-lived verified fork line; contains the complete
   upstream merge plus Coppice-only product/protocol work.
2. `upstream/fix-case-insensitive-extensions` — fresh from `origin/nightly`;
   narrow fix for upstream issue #1422.
3. `upstream/scanner-mtime-arc` — immutable scanner mtime snapshot sharing.
4. `upstream/watcher-lazy-lifecycle` — lazy `notify` construction without
   changing public watcher behavior.
5. `upstream/server-profile-boundaries` — compile/runtime/container profile
   boundaries, split further if upstream review requires it.
6. `upstream/graphql-lazy-schema` — one-time schema/DataLoader construction and
   resource-bound configuration with unchanged defaults.
7. `upstream/neutral-request-auth` — dependency-light request, pagination, and
   auth contracts with an unchanged generated schema.
8. `upstream/sqlite-write-discipline` — centralized immediate-write
   transactions and migration connection discipline.
9. `upstream/scanner-reconciliation-cache` — isolated scanner reconciliation
   cache work after the mtime branch.
10. `upstream/metadata-provider-work` — only issue-scoped pieces accepted under
    #1168; do not send the fork's provider product surface wholesale.
11. `upstream/permission-hardening` — only after upstream resolves #1035's
    intended permission model.
12. Protocol branches (`upstream/komga-*`, Kobo, KOReader, liseur-sync, ABS,
    Kavita) — only after an upstream issue/discussion accepts the compatibility
    surface and evidence scope.

Each `upstream/*` branch is reconstructed from the then-current
`origin/nightly`; none is cut from or merges `coppice/nightly`.

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

- Kavita: Kamigura has no EPUB reader (image lane 404s for EPUB like
  kavita-ref); Turnleaf and Kamigura have **Device-tested** runs, and the
  Kavita profile is **Contract-tested**. EPUB clients are Turnleaf/Inkita/Kover
  via `Book/*` routes.
- Cargo profiles: the bare `--no-default-features` server is leaner than
  `minimal=["formats"]`; WebUI is `webui=["graphql","graphql/web"]` and implies
  GraphQL. KOReader-only is `--no-default-features --features koreader` plus
  `ENABLE_KOREADER_SYNC=true`.
- Devices carry `library_scope` (virtual pathing) and `kindle_email`; every
  protocol resolves visibility through `VisibilityScope` on the request
  `AuthUser`. Device credentials inherit the owner's permissions and scopes can
  only narrow them. Writing lanes record last-seen/last-sync summaries for the
  Home device dashboard.
- Studies (plain Markdown, outside the site build): docs/drm-study.md,
  docs/audiobook-study.md.
- home/ `bun run build` is not concurrency-safe (wipes .svelte-kit/output):
  serialize builds; `rm -rf home/.svelte-kit/output` after an aborted one.
  GraphQL ops live per area in
  home/src/lib/graphql/{operations,dashboard,library,reader}.graphql; codegen
  unions them.
- liseur-sync attachment side objects are implemented as a Coppice extension:
  `PUT /v1/annotations/{id}/attachments/{kind}`, attachment listing, and the
  authenticated `/api/v2/annotations/{id}/attachments/{attachment_id}`
  download. Bytes are content-addressed under the configuration attachment
  root. This is source/contract evidence, not a pinned-client or device claim;
  attachment retention sweeping remains deferred.
- Write transactions go through `models::txn::begin_write` (SQLite `BEGIN
IMMEDIATE`; plain `begin()` only for read-only work). Migrations run on a
  dedicated 1-connection pool (sea-orm-migration does not transact SQLite DDL).
- Docs site build: `detect-libc` must be `^2` for lightningcss 1.33 (`familySync`).
- DRM: detector only (`drm_protected`, weight 0, blocking); study at
  `local://drm-study.md`; removal stays out of tree by decision.

- Read-aloud delivery: persistent validated `SyncMapV1` import, explicit
  active-input-deduplicating alignment enqueue, and authenticated cache-only
  status/download endpoints are shipped. The server finalizer recomputes
  canonical prepared-EPUB targets and strict audio identity, validates the
  requester/pair, worker output bytes/hash, durations and target IDs, then
  persists the map and publishes a deterministic cached read-aloud EPUB before
  the worker job becomes `done`.
- An optional worker-local Storyteller delegate is shipped behind the worker
  client feature. It uses its own URL/username/password, uploads the exact
  prepared EPUB plus single-file M4B, polls a bounded process, and returns only
  validated `SyncMapV1`; credentials and source content never enter the server
  job payload. Native CTC remains a later backend.
- GraphQL, authenticated API status/download, and OPDS acquisition links are
  cache-only and current-user/pair scoped. GET/status paths never enqueue or
  run ML. Folder audiobooks remain unsupported/not-ready. Virtual composite
  delivery, readiness quality checks, manifest federation, and a complete
  Storyteller/SMIL playback flow remain planned.
- The request workflow is present: request ledger and UI, approval/release
  selection, automation defaults, core processing, download polling, and the
  private `mam-gateway` implementation are shipped. Mocked protocol-contract
  evidence and all post-merge verification are pending; real MAM/VPN/qBittorrent
  deployment and physical-device delivery remain untested. Ordinary EPUB and
  audiobook playback remain separate.

## Standing rules

- Contract evidence pinned to Komelia `65f92fde`, `komga-client` 0.11.0
  `74412a6e`, Liseur v0.16.0 `bf5a4fd6fb0aca92a1f47c7feaf102202fd99d53`
  (source-only annotation evidence), liseur-sync OpenAPI `f8ce32b7`, Grimmory
  main, Kamigura `f4baeff4`, Turnleaf `b54f1f71`, and kavita-ref 0.9.1.4.
  The 2026-09-04 Liseur device/replay baseline remains separately scoped to
  its older tested client run; records are in
  `docs/content/docs/developer/{komga-compat,kavita-compat,kobo-sync-capabilities,kobo-device-database,unified-reading-state,liseur-sync-integration,liseur-providers,modular-ingest,server-architecture,calibre-tooling,comparison}.mdx`.
- The Komga mount test in `apps/server/src/routers/komga/mod.rs` and the
  Kavita `kavita_router_composes_without_route_collisions` test must remain.
- No GPL/AGPL code (calibre, DeDRM, BookOrbit, Grimmory, MangaManager, Sigil,
  cbzit) in this MIT tree; behaviour/specs only; calibre is shell-out only.
