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
The contract-tested home-library source-worker foundation and its remaining
phased plan live in
`docs/content/docs/developer/remote-worker-libraries.mdx`.


Evidence labels are exact: Shipped, Contract-tested, Device-tested, Source-only,
Planned, Blocked. Harness/source evidence never becomes app/device proof.

## Working tree

- Branch `coppice/nightly`; baseline commit
  `30d251886fe614053ea43ec9d50a8ab4cb0b2753` is pushed and matched
  `origin/coppice/nightly` before the current working tree. The upstream v0.1.10
  security merge is complete. The current uncommitted tree removes the private
  in-process MAM acquisition path, retains the generic request ledger, and now
  adds source-import matching/approval, a read-only Calibre source, exact-file
  verification caching, persisted metadata covers, native SMIL import,
  external native CTC alignment, deterministic read-aloud delivery, and Liseur
  contract fixes. Its 2026-09-22 Rust, Bun, schema, and production-build gate is
  green.
- The dirty tree was preserved before the merge:
  - `/home/al/Code/.stump-integration-backup/headless-modular-2026-09-20.patch`
    (sha256 `c2e57571c02727b5f476f7b462297e39bf0612c1051d82514e562edf782fb3a0`)
  - `/home/al/Code/.stump-integration-backup/headless-modular-untracked-2026-09-20.tar.gz`
    (sha256 `9aaf6d0236f0f90911f3978fb3af54d0d7d8ebc8c6742c40b2ba1155b3ea6c3b`)
    Keep these recovery artifacts unchanged.
- On 2026-09-23 the archived HEAD `0526084b` was confirmed reachable from
  this checkout; the recovery patch byte-matched `git diff --binary HEAD`, and
  all 319 untracked paths byte-matched the tarball. Archived `target/`,
  root and nested `node_modules/`, and `services/mam-gateway/target/` were
  removed; the backed-up 1.2 GB session HTML was removed too. The 127 archived
  `input/` files (2,133,075,620 bytes) were copied to
  `/mnt/als/Coppice-archive-input-2026-09-22/` and verified with rsync checksums.
  About 31 MB of root-owned `input/audio/experiments/` remains; sudo requires
  user authentication. The retained source checkout is about 472 MB; Kate has
  its working directory inside its `docs/`, so do not remove the checkout yet.
- The working umbrella is `/home/al/Code/coppice/`: `stump/` is this main
  repository. `nickelstump/`, `koreader-stump/`, and
  `stump-mihon-extension/` are independent Git repositories; create/publish
  each only after repository-specific user confirmation. `komga-compat/` is
  private local evidence, not a publishable repository.
- `stump-sources/` stays local-only with no remote. Do not publish or link its
  derived site list. Remote source definitions and Cloudflare-challenged sites
  are deferred until a browser-worker authentication design and a
  source-by-source linkage review exist. Keep `STUMP_ENABLE_PROVIDERS=false`.
- The integration carries Home/editor dark-first UX, OIDC-first login and
  group-permission mapping, device management and sync summaries, head-backed
  progress/session fixes, liseur annotation projection, persistent read-aloud
  sync maps plus alignment enqueue, worker claim expiry, OPDS/client
  compatibility fixes, migrations, generated GraphQL, and docs.
- `git config core.hooksPath /dev/null` remains local configuration from the
  earlier integration; the active Bun workspace no longer ships Husky hooks.
- Upstream drift is checked without mutating Git refs by
  `bun run check:upstreams`. `scripts/upstreams.json` records reviewed commits
  or releases plus the licence/source policy; `scripts/check-upstreams.mjs`
  uses authenticated `gh api` calls and prints compare/release links.
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

## Active JavaScript tooling

- Bun 1.4.1 owns the active workspace, lockfile, scripts, CI, and production
  builds. The Expo tree is a frozen compatibility source snapshot outside the
  workspace; do not run native Gradle, Xcode, CocoaPods, or EAS lanes.
- Keep Bun's isolated linker. Home and Editor must use Vite's default realpath
  resolution with `resolve.dedupe = ['svelte']`; `preserveSymlinks` makes
  source-linked workspace packages resolve transitive dependencies from the
  wrong package boundary and fails on `esm-env`, `devalue`, query-core,
  `runed`, or `svelte-toolbelt`.

## Home-library source workers

- **Contract-tested foundation.** The implementation applies only to Coppice
  source workers. MouseSearch/acquisition remains separate and does not serve
  media.
- Source workers use `DeviceKind::SourceWorker`,
  `UserPermission::AccessRemoteSource`, a separately scoped API key, and the
  dedicated `/api/v2/source-workers/socket` control path. Compute credentials
  do not inherit source-root visibility.
- Append-only migration `m20260956_000000_add_remote_sources` adds logical
  `remote_source`, `remote_source_item`, and `media_location` records without
  overloading `media.path`; `m20260957_000000_add_remote_source_imports` adds
  the durable proposal/decision ledger.
- `stump-worker` accepts explicit read-only filesystem roots and
  `kind=calibre` roots. Calibre discovery opens `metadata.db` read-only,
  validates its application/schema identity, and publishes only catalogued
  format paths that resolve below the non-symlink root. Absolute paths remain
  in the worker's private catalog. `candidate_only` still fails closed because
  no interest-set protocol exists.
- Explicit verification streams the whole object and recomputes SHA-256.
  Successful checks enter a 128-entry LRU keyed by exact local file identity
  plus expected digest; replacement/rewrite metadata invalidates reuse before
  a later range. Materialization still rechecks the digest and enters existing
  ingest staging.
- `POST /api/v2/remote-sources/{id}/match` creates only digest/lineage-bound
  proposals. Matching uses verified locations, typed identifiers, then exact
  normalized title/author, with ambiguous evidence rejected. Proposal listing
  plus explicit approve/reject routes persist the actor and recheck current
  source identity before idempotent linking or materialization. Inventory and
  matching never publish media automatically.
- Native media serving keeps Coppice as the reader-facing ACL and HTTP range
  boundary. Direct and outbound-tunnel reads use one-use exact-byte grants and
  require the worker to bind the opened file to the server-verified SHA-256.
  Invalid/multiple ranges return `416`; a known offline location returns `503`.
- Focused evidence: 51 worker tests, four source-worker server tests, and the
  remote-source migration test passed; the complete gate is recorded below.
  Source of record:
  `docs/content/docs/developer/remote-worker-libraries.mdx`
  (**Contract-tested**).

## Absorbed upstream security fixes

- Upstream commit `3d5854228d8f314f36ed6aeb9a4714cc4c30ed50` protects the
  user self-permission-escalation path, the `libraryMissingEntities` management
  guard, user-scoped `mediaMetadataOverview` results, and book-club read
  access.
- Upstream security release v0.1.10 / commit
  `44e1de12dd058bc27d40323cf495fe751088b364` prevents owner authority from
  flowing through custom API keys and requires an interactive session before
  creating or updating inherited-permission API keys. Coppice extends the same
  boundary to API/Web device creation, credential rotation, and pairing
  approval. Reader-device credentials remain custom/narrowed and are
  unaffected.

## Fixture and launcher

- One instance, every supported protocol profile on: `hub` process
  `stump-komga`, port 25600, launcher `scripts/dev-fixture-server.sh`, binary
  `target/debug/stump_server` (headless,liseur-sync). Launch env:
  `STUMP_ENABLE_KAVITA=true STUMP_ENABLE_BACKGROUND_JOBS=true
STUMP_ENABLE_PROVIDERS=false`; Komga, Kobo, and KOReader default on. `/editor`
  and `/app` mount the source-relative SvelteKit builds (`INGEST_EDITOR_DIR`,
  `STUMP_HOME_APP_DIR`).
- Root `$HOME/.local/share/stump-komga-test`; the launcher regenerates
  `$FIXTURE_ROOT/ENDPOINTS.md` (0600) with every URL and credential; never
  quote that sheet. The DB was upgraded in place through `m20260949` on
  2026-09-12.
- Komf direct replay mutates synthetic fixture metadata; use disposable IDs
  and restore the series before a later Mihon replay. The unpinned
  `komf-stump` container is stopped. The pinned `komf-komga` and
  `komf-kavita` profiles use separate data volumes and ports 25607/25608;
  both were stopped after their respective smoke checks. Never attach both
  adapters to the same library.

## Replay commands

The 2026-09-21 MAM-removal/Bun-cutover replay passed every currently required
server contract. These are server-contract results, not physical-client proof:

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
make replay-komf
make replay-komf-kavita
make smoke-komf-komga
make smoke-komf-kavita
```

## Only definition of green

The 2026-09-22 source/read-aloud/metadata/Liseur integration tree is green:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --exclude stump_server
cargo test -p stump_server --no-default-features --features headless,liseur-sync
cargo build -p stump_server --no-default-features --features headless,liseur-sync
cargo dump-schema -- --check
bun install --frozen-lockfile --ignore-scripts
bun run check-types
bun run test
bun run web build
bun run home build
bun run editor build
bun run docs build
```

Observed results: 1,922 non-server Rust tests passed (29 ignored across 94
suites), 377 headless server tests passed across 4 suites, and all 302 active
Bun tests passed (24 SDK, 273 browser, 5 desktop). GraphQL schema/client drift
checks passed after regenerating the changed schema. Web, Home, Editor, and
docs production builds passed; docs prerendered 198 pages.

Focused proof also passed for the worker (51), read-aloud media (4), sync-map
library (4), ingest (134), remote-source migration (1), source-worker server
(4), and Liseur invalid-limit regression (1) paths. The private real-input
archive ingest smoke passed end-to-end. Live provider probes passed for AniList,
MangaDex, MangaUpdates, OpenLibrary, and Audible; Google Books returned an
external HTTP 429, while MAL, Hardcover, Comic Vine, and Metron credentials
were unavailable.

On 2026-09-23 the integration gate above passed: 1,926 non-server Rust tests
(29 ignored across 94 suites), 377 headless server tests, schema check,
format/check, frozen Bun install, type check, Bun tests, and all four production
builds. The pinned Komf source manifest was generated then checked. Direct
Komga replay passed 38 requests; direct Kavita replay passed 37. Pinned
sidecar connection/library smoke passed two requests per profile. The baseline
`make replay` passed 36 requests using a clean single-book fixture, and
`make replay-kavita` passed 170 requests. These are contract/runtime tiers,
not physical-client evidence; the older 341-request aggregate was not rerun.
The official ABS phone retest also remains external and blocked on a physical
device. Expo remains a frozen compatibility snapshot outside the active
Bun/native-tooling gate.

`apps/server/src/{lib,main}.rs` carry `#![recursion_limit = "256"]` because
the merged GraphQL schema overflows rustc's query depth.

## Upstream contribution strategy

- `coppice/nightly` is the long-lived verified fork branch. The assistant may
  commit/push it only under the turn-specific authorization in `.omp/AGENTS.md`.
- GitHub `origin` currently advertises `main` (`42a9918c`, the Stump-derived
  default) while the published Coppice work is `coppice/nightly`
  (`30d25188`). Change the GitHub default to `coppice/nightly` only after
  explicit user confirmation; do not push to or rewrite `main`. Every commit,
  push, remote creation, and separate-repository publication requires the
  confirmation gate in `.omp/AGENTS.md`.
- Claim or open an upstream issue first. Reconstruct each candidate PR branch
  from an immutable commit on `stumpapp/stump`'s then-current `nightly`, not
  from the Coppice fork history; target upstream `nightly`, keep each change
  narrow, and run the required Prettier/rustfmt checks.
- Disclose LLM assistance in the PR body and never use an LLM-signed commit.

Candidate upstream extraction order:

1. `coppice/nightly` — long-lived verified fork line; contains upstream plus
   Coppice-only product/protocol work.
2. `upstream/fix-case-insensitive-extensions` — reconstruct from the pinned
   `stumpapp/stump` `nightly` base; narrow fix for upstream issue #1422.
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

Each `upstream/*` branch is reconstructed from the then-current immutable
`stumpapp/stump` `nightly` commit; none is cut from or merges
`coppice/nightly`.

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
  active-input-deduplicating alignment enqueue, strict source-EPUB SMIL import,
  and authenticated cache-only status/download endpoints are shipped. Native
  SMIL is accepted only when its embedded audio bytes equal the sole paired
  external M4B. Preparation preserves source XHTML IDs and assigns
  deterministic targets only where absent.
- The server finalizer rechecks requester/pair, canonical source digests,
  prepared targets, durations, worker output bytes/hash, and strict audio
  identity. Its deterministic mimetype-first EPUB renderer streams the original
  M4B into the derivative rather than buffering it.
- Optional worker-local Storyteller and external native CTC backends are
  shipped. The native backend invokes only an operator-configured executable
  and model for the typed CPU/fp32/CTC profile; both backends stream authorized
  sources to temporary files and return only `SyncMapV1`. Credentials, model
  paths, and source content never enter the server job payload.
- GraphQL, authenticated API status/download, and OPDS acquisition links are
  cache-only and current-user/pair scoped. GET/status paths never enqueue or
  run ML. Folder audiobooks remain unsupported/not-ready. Virtual composite
  delivery, readiness quality checks, manifest federation, Storyteller UUID
  reuse, and synchronized physical-reader playback remain planned.
- Metadata cover application now persists the selected provider artwork through
  a bounded, redirect-revalidated, DNS-pinned public HTTP(S) fetch. The decoded
  content-addressed file is staged before the SQLite write transaction; rollback
  and superseded-art cleanup preserve DB/filesystem consistency.
- Liseur catalog listings no longer label sampled `media.hash` values as
  SHA-256. Explicit resolution hashes the readable file and binds it to stable
  `source` identity; invalid signed query limits use the pinned client defaults
  (`500` changes/annotations, `50` positions).
- The request ledger and Home Requests UI are shipped: users create
  metadata-backed intent with destination/visibility and managers approve or
  reject it. The former release search/selection, automation, grab/poll/retry,
  acquisition ORM, scheduler jobs, and private `mam-gateway` runtime have been
  removed in the current tree. A future independent authenticated sidecar may
  implement acquisition; shared-folder or equivalent output enters through
  staged ingest. Ordinary EPUB and audiobook playback remain separate.

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
  cbzit) is copied or linked into this MIT tree. Calibre conversion remains a
  shell-out; the source adapter independently reads the documented SQLite
  catalog contract in read-only mode.
