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

- 2026-10-05: branch `coppice/client-updates-2026-10-04` (base
  `origin/coppice/nightly` = `057d2da8`) holds all 2026-10-04/05 work in five
  path-grouped commits: Rust server (`86169e4d`), JS workspaces (`ff97d2e9`),
  Komf contract tooling and CI (`48140fc7`), docs (`e326cfba`), and this OMP
  state plus upstream review records. Code content equals the 2026-10-05
  gate-green tree.
- Stump's upstream reading timeline (`b988179f`) is not adopted. Do not port
  its `session_id` FK migration; if it is wanted, derive session events at
  query time from `reading_sessions` (which already has readthroughs, elapsed
  time, and Liseur/Kobo projections). See `.omp/NEXT_STEPS.md`.
- The pre-review `bun run check:upstreams` on 2026-10-05 showed Stump +1,
  Liseur +42, KOReader +5, Lissen +5 (release 1.12.9), and Komga +1; all
  other tracked heads/releases were at their prior baselines. Reviewed each
  delta: Stump adds a reading-timeline feature while its EPUB positions
  offload is already present; Liseur's `WorkResolver` derives the existing
  `dc` identifier using `identityAuthor` when available (Coppice already
  accepts `dc`); KOReader changes are local UI/state; Lissen changes are
  player/UI/CI/test work; Komga's sole change is a Gradle action version in
  CI. No contract-relevant change needs porting. Review baselines are recorded
  in `scripts/upstreams.json`; the post-review check reported zero commit
  delta for those five repositories. No upstream pull or merge was done.
- Historical 2026-09-25 baseline: `coppice/nightly` commit `47a9c61c`
  (MAM acquisition and UI rework), pushed on top of `29115590`
  (2026-09-23). It deletes the legacy React/Vite, Expo, desktop,
  and web packages (`packages/{browser,components,client,sdk,i18n,graphql}`,
  `apps/{expo,desktop,web}`) — removed, not frozen — and adds MAM acquisition
  through the separate MAM Bridge sidecar, split unified search, narrator
  lookup and preferred narrator on requests, lazy thumbnails, Liseur v0.19.0
  pin plus work-identity repair, SPA 404 prefixes, log rolling, annotation
  sync (canonical CAS, revisions, origin device, Home editing with
  `expectedRevision`), and the Home/KOReader UI work below; its 2026-09-25
  gate was green. The 2026-09-26 follow-up (8 MiB runtime thread stacks,
  annotation book ranking + `koreaderHash`) is in `.omp/NEXT_STEPS.md`
  "Resume point".
- Stray `DocsApply` and a 118 MB `omp-session-*.html` were deleted
  (user-approved).
- Recovery artifacts (keep unchanged) under
  `/home/al/Code/.stump-integration-backup/`:
  `headless-modular-2026-09-20.patch` (sha256 `c2e57571…`) and
  `headless-modular-untracked-2026-09-20.tar.gz` (sha256 `9aaf6d02…`); both
  were byte-verified against the archived HEAD `0526084b` on 2026-09-23. The
  127 archived `input/` files live in
  `/mnt/als/Coppice-archive-input-2026-09-22/` (rsync-checksum verified);
  ~31 MB of root-owned `input/audio/experiments/` remains and needs sudo. Kate
  has its working directory inside the retained checkout's `docs/`; do not
  remove that checkout yet.
- The working umbrella is `/home/al/Code/coppice/`: `stump/` is this main
  repository. `nickelcoppice/`, `koreader-coppice/`, and
  `stump-mihon-extension/` are independent Git repositories; create/publish
  each only after repository-specific user confirmation. `koreader-coppice/`
  is public as `coppice-media/koreader-coppice` (`main`, `35a1d5b`,
  2026-09-26; earlier publication `ddbcdde` was 2026-09-25).
  `komga-compat/` is private local evidence, not a publishable repository.
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
- Optional stack links remain planned in Coppice; the sibling shared metadata
  crate is not yet a dependency here. Its cutover needs a CI-fetchable pinned
  source and publication authorization. See `integration-architecture.mdx`
  “Optional stack links” and `.omp/NEXT_STEPS.md` “Remaining-work index”;
  preserve solo operation and do not move folders, secrets, or user data.
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
  builds. Workspaces: `docs`, `home`, `editor`, `packages/stump-ui`. Root
  scripts have no `test` and no `web`; the Expo/desktop/web packages are
  removed, so no native Gradle, Xcode, CocoaPods, or EAS lanes exist.
- Keep Bun's isolated linker. Home and Editor must use Vite's default realpath
  resolution with `resolve.dedupe = ['svelte']`; `preserveSymlinks` makes
  source-linked workspace packages resolve transitive dependencies from the
  wrong package boundary and fails on `esm-env`, `devalue`, query-core,
  `runed`, or `svelte-toolbelt`.

## Documentation reconciliation (2026-10-04)

- Reconciled compatibility/provider guides, crate READMEs, roadmap, and handoff
  files with the October working tree. Completed fixes are not pending work;
  historical client/device evidence stays dated. Optional stack links remain
  planned. Remaining source risks live in `.omp/NEXT_STEPS.md`.
- Fixed the docs runtime: Fumadocs core/UI aligned at `16.10.3`, last-modified
  metadata passed to the renderer, unused legacy wrappers removed, formatter
  stylesheet paths corrected, and a single correctly targeted GitHub edit
  anchor replacing invalid nested anchors.
- Scoped gate passed using **Bun 1.4.1** via `bunx bun@1.4.1`:
  root `bunx bun@1.4.1 install --frozen-lockfile --ignore-scripts` (no changes);
  root `bunx bun@1.4.1 x prettier --check prettier.config.js docs/package.json docs/src/lib/shared.ts docs/src/lib/source.ts 'docs/src/routes/docs/$.tsx'`;
  from `docs/`, `bunx bun@1.4.1 run types:check && bunx bun@1.4.1 run build`.
- Internal page-link audit: **88 pages, 401 `/docs/...` links, zero missing
  page targets**. This is not an external-link or fragment-anchor audit.
- Production preview smoke: **13 distinct pages**, expected headings/content
  and fork edit URLs, Markdown endpoint `200`, no browser page errors.
  Windows installation tabs selected the visible Windows download panel;
  mobile guest-reader TOC opened/closed and exposed section links.
  Desktop `1440×1000` and mobile `390×844` screenshots were captured.
- Screenshot review passed for desktop and settled mobile. The initial mobile
  capture caught a TOC-label transition; no persistent overlap remained.
- Existing binary CLI inventory smoke: `account --help`, `system --help`,
  and `tools --help` exited successfully without changing account data.
- Docs-only verification; the full application gate and server replay below
  were **not rerun**. No new physical-device or live-proxy evidence.

## Priority-1 safety work (2026-10-05)

- Hardcover personal search and narrator lookup require `use_for_metadata`;
  caches are credential/consent scoped. Manual sync imports the authenticated
  user's complete paged `reading_journals` quote set as provenance; optional
  local progress requires valid non-future source time and an explicit fraction
  newer than the local head. No outbound write or fabricated EPUB locator.
- Live read-only verification: the fixture database contains one configured
  personal connection (`use_for_metadata=1`, `import_journals=0`), no enabled
  global Hardcover provider. The PAT was decrypted only inside the temporary
  process and never printed. Live `reading_journals` introspection and the
  exact `me.id` selection query returned HTTP 200 with no GraphQL errors; the
  selected user ID was deliberately nonexistent and returned no rows. No
  consent toggle changed and no real journal rows were fetched or imported.
- Disposable runtime evidence (isolated `/tmp` DB): the OIDC CLI transferred one
  reading head, one Hardcover connection, and one Liseur annotation to the
  surviving account; the local source disappeared and both session tables
  were empty. A duplicate Hardcover connection was rejected and rollback left
  both users and both connections intact. The Home hard-delete dialog required
  exact-name confirmation and removed the synthetic user and remaining user
  ownership references. Annotation export routed exact-edition and unbound
  notes to their selected files and left an unmatched note standalone.
- OIDC migration transfers the current discovered SQLite references in one
  write transaction, revokes both accounts' login sessions, and denies
  approved-but-unissued pairings before ownership moves. External markdown/
  Git exports remain outside the DB migration. Hard/soft deletion invalidates
  user and guest capabilities, closes live sockets, and prevents queued
  annotation/notification work from acting for deleted users.
- Deletion races serialize against credential creation: JWT refresh-row inserts
  and pairing device/key creation are in the transaction that locks and checks
  the account; cookie-session issuance is followed by a locked liveness check.
  Source-root registration and each bounded manifest chunk also lock/recheck
  the account in their write transaction before mutating inventory. SQLite's
  writer lock and other backends' account-row lock ensure either the creation
  commits first (then deletion revokes it) or deletion commits first (then the
  stale caller is rejected).
- Liseur Markdown/Git export chooses only confirmed, visible, ready,
  non-audiobook media; matching edition hash ranks first. Existing historical
  exports are not automatically pruned or rewritten.
- Baseline repro evidence: on a disposable account, consent-off search still
  attempted to decrypt its personal credential; the old OIDC command deleted
  the source but left its Hardcover connection behind; and annotation routing
  lacked an eligible edition candidate. Regression tests now cover these paths.
- No real Hardcover journal rows were fetched or imported, and no physical-device
  behavior was exercised.

## Home-library source workers

- **Contract-tested foundation**, applying only to Coppice source workers.
  Source of record: `docs/content/docs/developer/remote-worker-libraries.mdx`.
- Source workers use `DeviceKind::SourceWorker`,
  `UserPermission::AccessRemoteSource`, a separately scoped API key, and the
  dedicated `/api/v2/source-workers/socket` control path; compute credentials
  do not inherit source-root visibility. Migrations `m20260956` (logical
  `remote_source`/`remote_source_item`/`media_location`, no `media.path`
  overload) and `m20260957` (proposal/decision ledger).
- `stump-worker` accepts read-only filesystem and `kind=calibre` roots
  (`metadata.db` opened read-only, schema identity validated, only catalogued
  format paths below the non-symlink root published). `candidate_only` fails
  closed until an interest-set protocol exists.
- Verification streams the whole object and recomputes SHA-256; successes
  enter a 128-entry LRU keyed by exact file identity + expected digest.
  `POST /api/v2/remote-sources/{id}/match` creates only digest/lineage-bound
  proposals (verified locations, typed identifiers, then exact normalized
  title/author; ambiguity rejected); approve/reject persist the actor and
  recheck source identity. Nothing publishes media automatically.
- Native media serving keeps Coppice as the ACL and HTTP range boundary with
  one-use exact-byte grants bound to the server-verified SHA-256;
  invalid/multiple ranges `416`, known offline location `503`.

## Book-club guest reader (2026-10-04, uncommitted on `coppice/client-updates-2026-10-04`)

- Per-person revocable links for accountless readers. Guide:
  `docs/content/docs/guides/features/book-clubs/guest-reader.mdx` (incl. the
  three Pangolin bypass rules, verified against pinned `pathMatch.ts`
  `58a3701`; no live proxy rule was applied — no Pangolin config or SSH host
  is reachable from this workspace).
- Server: `apps/server/src/routers/api/v2/club_reader/` (readium-gated).
  Management `/api/v2/book-clubs/{clubId}/reader-sessions...` needs
  `UserPermission::ShareBookClubReader` + club Admin/Creator and refuses API
  keys/device credentials. Guest `/api/v2/club-reader/...` uses a 256-bit
  digest-stored capability → HttpOnly `coppice_club_reader` cookie scoped to
  `/api/v2/club-reader/sessions/{id}`; mutations need same-origin `Origin`
  and `X-Club-Reader-Participant` (409 on mismatch). Every request rechecks
  issuer/publisher live (LoginUser, not deleted/locked), role, permission,
  publisher media visibility (incl. `deleted_at`), and that the published
  book is still the queue head (else 409 paused). Guest 401s never clear
  `stump_session` (`strip_session_cookie_clears` is no longer Komga-gated).
- Storage: migration `m20260969` + `book_club_reader_{session,participant,
  progress,annotation}`; never writes reading heads or native annotations.
- Home: `/app/club-reader/[sessionId]` outside `(app)`; organizer controls in
  `BookClubPanel.svelte` via `home/src/lib/reader-sessions.ts`. EPUB
  sections in BOTH readers are sanitized with DOMPurify on
  `doc.documentElement` (passing the Document throws) keeping stylesheet
  `<link>` and `epub:type`, plus a per-section CSP meta.
- Regressions: `apps/server/tests/club_reader/mod.rs` (4 tests).

## Absorbed upstream security fixes

- Upstream `3d585422` (self-permission escalation, `libraryMissingEntities`
  guard, user-scoped `mediaMetadataOverview`, book-club read access) and
  v0.1.10 `44e1de12` (no owner authority through custom API keys; interactive
  session required for inherited-permission keys) are merged. Coppice extends
  the v0.1.10 boundary to API/Web device creation, credential rotation, and
  pairing approval; reader-device credentials stay custom/narrowed.

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
- MAM Bridge: the fixture launcher reads the bridge origin from
  `~/.config/mam-bridge/url` and the token from
  `~/.config/mam-bridge/api-token` (both local, mode 600); MAM is enabled
  only when both exist (`STUMP_ENABLE_MAM_ACQUISITION=true`). The handoff
  root comes from `MAM_BRIDGE_HANDOFF_ROOT`. Never write the bridge origin,
  LAN paths, or the token into docs, commits, or messages. Fixture default
  `STUMP_VERBOSITY=2`.
- Komf direct replay mutates synthetic fixture metadata; use disposable IDs
  and restore the series before a later Mihon replay. The unpinned
  `komf-stump` container is stopped. The pinned `komf-komga` and
  `komf-kavita` profiles use separate data volumes and ports 25607/25608;
  both were stopped after their respective smoke checks. Never attach both
  adapters to the same library.

## Replay commands

Last full post-build pass **2026-10-05** (full binary, isolated `/tmp` fixture
copy): 16 Hurl files, 375 requests, 0 failures — `replay` 36,
`replay-negative-auth` 3, `replay-mihon` 19, `replay-liseur-sync` 43,
`replay-kavita` 170, `replay-library-management` 17 (disposable root),
`replay-abs` 49, `replay-komf` 38 (last; mutates its copy). Not run:
`replay-komf-kavita`, the Komf sidecar smokes, `replay-readium`;
`make replay-containers` remains an unrun pending spec. Server-contract results,
not physical-client proof.

Fixture pitfalls seen 2026-10-04 (data, not code): earlier Komf replays left
`00 synthetic prelude` with number 7, so Mihon's PUT marks nothing — clear it
first (`media_metadata.number = NULL` for `bd94f729…`), as the Komf note above
says; `replay` must use the one-book Synthetic Solo series (the sibling spec
expects 404 previous/next); Kavita `recently-updated-series` has a 12-day window,
so a fixture older than that returns `[]` unless one book's `created_at` is
recent. Copy the DB without its `-shm` file or the server reports
"database disk image is malformed".
- 2026-10-05 replay copy also inherited stale user-uploaded thumbnails; clear
  the selected synthetic series and book `thumbnail_path` values in the copy
  before `replay-komf`. These fixture-only resets do not alter the source DB.

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

The current full application gate passed on 2026-10-05 after the priority-1
safety changes. Exact commands:
```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --exclude stump_server
cargo test -p stump_server --no-default-features --features headless,liseur-sync
cargo build -p stump_server --no-default-features --features headless,liseur-sync
cargo build -p stump_server --features full
cargo dump-schema -- --check
bun install --frozen-lockfile --ignore-scripts
bun run check-types
bun run check:komf-contract
bun run home build
bun run editor build
bun run docs build
(cd ../koreader-coppice && luajit tests/pure_checks.lua && luajit tests/browser_checks.lua && bash tests/static_checks.sh)
```

2026-10-04 (pre-priority-1 uncommitted `coppice/client-updates-2026-10-04`):
**baseline gate green** — 2,521 Rust tests (2,044 workspace + 477 headless
server), 0 failed, 22 ignored (live-network tests); fmt, workspace check,
headless and full builds, `dump-schema --check`, frozen Bun install,
check-types, Komf contract, Home/Editor/docs builds, and KOReader plugin
(580 pure checks, browser, static). The priority-1 changes are not covered by
this baseline.
- 2026-10-05 post-priority-1 gate: all commands above passed. The headless
  server run reported 158 library tests and 165 API tests; the KOReader suite
  passed 580 pure checks plus browser and static checks. Bun type/contract
  checks and Home, Editor, and docs production builds passed. The first schema
  check identified stale generated GraphQL output; `cargo dump-schema` updated
  it, then the exact gate passed. Post-build `/tmp` runtime checks: OIDC
  transfer/conflict rollback, Home hard deletion, and annotation export as
  recorded above. No real journal import, physical-reader, or live-proxy run.
Gate note: use `CARGO_BUILD_JOBS=1` while a desktop browser and other projects'
`/tmp` (tmpfs, RAM-backed) Cargo targets are present — rustc was OOM-killed
twice at ~4 GB. Runtime smoke (loopback, `/tmp` DB copy): anonymous guest
EPUB/CBZ reading, shared/private annotations, 5 % group progress, live SSE
(message/progress ~0.3 s, moderation, revocation), rotate/revoke/advance,
same-tab link switch, organizer panel, `/app/users` create/grant/lock, account
EPUB highlight and audiobook note, Editor quality setting save/re-save. No live
Pangolin, Komelia, or device run.

On 2026-09-25 the gate passed: 2,414 Rust tests passed, 0 failed, 22 ignored
(workspace + headless server suites); check, headless and full builds, schema
check, frozen Bun install, type check, Komf contract check, Home/Editor/docs
production builds, and KOReader plugin checks (528 pure checks, browser
layout, static) passed. `cargo fmt --check` first failed on unformatted new
code; `cargo fmt --all` was applied and the check then passed. The
Komga/Kavita/Mihon/ABS/Liseur replays and the pinned Komf run were NOT rerun
this session; the 2026-09-21/23 replay results above are historical.

Historical (superseded list also ran `bun run test` and `bun run web build`,
removed with the legacy packages): 2026-09-23 — 1,926 non-server + 377
headless server Rust tests, Bun tests, four builds; direct Komga replay 38,
direct Kavita 37, pinned Komf smoke 2/profile, `make replay` 36,
`replay-kavita` 170. 2026-09-22 — 1,922 + 377 Rust tests, 302 Bun tests,
private real-input archive ingest smoke; live provider probes passed for
AniList, MangaDex, MangaUpdates, OpenLibrary, Audible; Google Books HTTP 429;
MAL, Hardcover, Comic Vine, Metron credentials unavailable. The official ABS
phone retest remains blocked on a physical device.

`apps/server/src/{lib,main}.rs` carry `#![recursion_limit = "256"]` because
the merged GraphQL schema overflows rustc's query depth.

## Upstream contribution strategy

- `coppice/nightly` is the long-lived verified fork branch. The assistant may
  commit/push it only under the turn-specific authorization in `.omp/AGENTS.md`.
- GitHub `origin` currently advertises `main` (`42a9918c`, the Stump-derived
  default) while the published Coppice work is `coppice/nightly`
  (`29115590`). Change the GitHub default to `coppice/nightly` only after
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
- Read-aloud (Shipped): persistent validated `SyncMapV1` import,
  active-input-deduplicating alignment enqueue, strict source-EPUB SMIL import
  (accepted only when embedded audio bytes equal the sole paired M4B), and
  authenticated cache-only status/download endpoints. The finalizer rechecks
  requester/pair, source digests, prepared targets, durations, worker output
  hash, and audio identity; the mimetype-first EPUB renderer streams the M4B.
  Worker-local Storyteller and operator-configured native CPU/fp32 CTC
  backends return only `SyncMapV1`; credentials, model paths, and source
  content never enter the server job payload. GET/status paths never enqueue
  or run ML. Planned: folder audiobooks, virtual composite delivery, readiness
  checks, manifest federation, Storyteller UUID reuse, physical-reader
  playback.
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
  reject it. Acquisition now ships through the MAM Bridge sidecar (below);
  ordinary EPUB and audiobook playback remain separate.

## Shipped 2026-09-24/25 (pushed in `47a9c61c`)

- **MAM acquisition (Shipped, live-probed).** Feature `mam-acquisition` (in
  `headless`/`full`); env `STUMP_ENABLE_MAM_ACQUISITION`, `MAM_BRIDGE_URL`,
  `MAM_BRIDGE_TOKEN_FILE`, `MAM_BRIDGE_HANDOFF_ROOT`, `MAM_BRIDGE_SOURCE_ROOT`;
  permission `ACQUIRE_RELEASES`. GraphQL: `acquisitionStatus`,
  `searchReleases`, `cachedReleaseSearch`, `acquisitionGrabs`,
  `grabRelease(confirm)`. Flow: request → manager approval → bridge search →
  confirmed grab → polling job → completed download COPIED into staged ingest
  → Editor approval fulfils the request. Ranking: exact vs partial title,
  comic-category/CBR penalty for prose, HTML entity decoding, f64 scores,
  narrator +30, ASIN +100. Migration `m20260965`. Live: bridge ready/live;
  probe (5 of 14) then full search; top hit Ender's Game EPUB, 404 seeders,
  score 80. No grab performed; the end-to-end grab needs explicit user go.
- **Unified search split (Shipped, live-timed).** `unifiedSearch` removed;
  `librarySearch(query, limit=20)` local ~3–6 ms; `externalBookSearch(query,
  limit=10)` Hardcover, one index request, 5-min bounded cache, ~150–730 ms
  live / 4 ms cached; `audibleBookSearch(query, limit=5, language="en")`
  Audible catalog, no key, title-only relevance filter, drops
  podcasts/periodicals and unnarrated <60 min. Hits carry
  `hasEbook`/`hasAudiobook` (Hardcover index), `audioSeconds` (Audible only —
  the live Hardcover index has no `audio_seconds` key despite its docs),
  `asin`, `narrators`. Home header search is a shadcn Command dialog
  (⌘/Ctrl-K) with library / Hardcover / "Only on Audible" groups, ~1.2 s.
- **Narrators (Shipped).** `audiobookNarrators(provider, remoteId, title,
  authors, language="en")`: Hardcover audio editions ∥ Audible catalog,
  deduped, ordered both-sources → Audible → Hardcover, 1 h cache. Live:
  Project Hail Mary → Ray Porter first. Book requests carry `format:
  RequestFormat {EBOOK,AUDIOBOOK,ANY}`, `isbn`, `preferredNarrator`
  (`setBookRequestPreferredNarrator`); Audible-sourced requests store the ASIN
  as `remoteId`. Migrations `m20260964`, `m20260967`. Home
  `RequestFormatControl` shares the format toggle + narrator popover.
- **Lazy thumbnails (Shipped, live).**
  `stump_media::image::generate_thumbnail_on_demand` encodes WebP fit-within
  400×600 at quality 80 (or the library `thumbnail_config`) on first request
  and keeps `thumbnails/<id>.<ext>`; OPDS 1.2 shares the path; the scanner
  drops on-demand files for rebuilt books; the WebP encoder honours `quality`
  (unset = 100). Live: Vol. 1 Ch. 1 1.6 MB → 47 KB; repeat ~4 ms.
- **Liseur v0.19.0 pin + work identity (Shipped).** Wire format unchanged;
  `/v1/events` now 404 (Liseur retried forever on the SPA redirect); BookOrbit
  (v0.19 native provider) is not implemented — Liseur reaches Coppice via the
  Komga provider + liseur-sync. Migrations `m20260963` + `m20260966` repair
  pair/alias-tied work splits via shared `merge_liseur_work`; the resolver
  self-heals alias-less pair works. Live: The Lottery merged into one work,
  resolve 200, 3 Home notes present.
- **SPA fallback (Shipped, tested).** Unknown `/v1/`, `/kobo/`, `/komga/`
  paths 404 (`NON_BROWSER_PREFIXES`; test in `apps/server/tests/webui/mod.rs`).
- **Logging (Shipped).** Startup rolls `Stump.log` > 64 MiB to `Stump.log.1`;
  per-statement SQL logging (sqlx + sea_orm driver) only at verbosity 3.
- **Migrations** appended `m20260958`–`m20260967`: liseur sync settings,
  series names, annotation projection, pairing Komf metadata editing,
  annotation origin, pair-work identity repair, request format/isbn, MAM
  acquisition, alias-split repair, preferred narrator.
- **Home UI (browser-verified).** Compact request cards, MAM release list,
  reading heatmap and phone "Continue reading" grid without horizontal
  overflow at 390 px; add-client dialog with segmented filters, inline status
  icons, one shared `ClientCard`, ABS eBooks supported.
- **KOReader plugin (Device-tested).** Home/Readium note anchoring fixed
  (href/chapter filter only when reported, `chapterTitle`, unique exact
  anchors, context breaks ties). Live: The Lottery's 3 Home notes stored as
  highlights without duplicates; zip `c87175dc` installed on PC and phone.
- Docs: acquisition "deferred" wording replaced in README, 7 docs pages, and
  `.omp/RULES.md`.

## Standing rules

- Contract evidence pinned to Komelia `65f92fde`, `komga-client` 0.11.0
  `74412a6e`, Liseur v0.19.0 `62ecb5a5c9dd8eb4e7fa6d97ce50d1bddae0bcd6`
  (source-only; wire unchanged since v0.18.0 `b00ee789`), liseur-sync `906889ff`, Grimmory
  v3.5.0 `402e89b4`, Kamigura `f4baeff4`, Turnleaf `b54f1f71`, and kavita-ref 0.9.1.4.
  The 2026-09-04 Liseur device/replay baseline remains separately scoped to
  its older tested client run; records are in
  `docs/content/docs/developer/{komga-compat,kavita-compat,kobo-sync-capabilities,kobo-device-database,unified-reading-state,liseur-sync-integration,liseur-providers,modular-ingest,server-architecture,calibre-tooling,comparison}.mdx`.
- The Komga mount test in `apps/server/src/routers/komga/mod.rs` and the
  Kavita `kavita_router_composes_without_route_collisions` test must remain.
- No GPL/AGPL code (calibre, DeDRM, BookOrbit, Grimmory, MangaManager, Sigil,
  cbzit) is copied or linked into this MIT tree. Calibre conversion remains a
  shell-out; the source adapter independently reads the documented SQLite
  catalog contract in read-only mode.
