# Coppice Next Steps

Agent-facing resume file for branch `coppice/nightly`. Baseline commit
`29115590` (2026-09-23) is pushed; the current uncommitted, fully gated
(2026-09-25) tree removes the legacy React/Vite/Expo/desktop/web packages and
adds MAM acquisition through the MAM Bridge sidecar, split unified search,
narrator lookup and preferred narrator on requests, lazy thumbnails, the
Liseur v0.19.0 pin with work-identity repair, SPA 404 prefixes, log rolling,
and annotation sync. Remaining workstreams are the live MAM grab, device
verification, deferred placement/readiness features, credentialed provider
probes, repository publication, and upstream submission.
Product roadmap: `docs/content/docs/developer/roadmap.mdx`. Evidence for what
already landed: `docs/content/docs/developer/state.mdx`. Working-tree, gate,
and disk rules: `.omp/PROJECT_STATE.md` — that file is authoritative and this
one never restates its command list.

A design page or replay fixture is evidence for a contract, not proof that a
feature is shipped. Integration ownership and evidence labels are defined in
`docs/content/docs/developer/integration-architecture.mdx`.

## Resume recipe

1. **Read state.** `.omp/PROJECT_STATE.md` (working tree, disk discipline,
   "Only definition of green"), then `git log --oneline -3`.
   Run `bun run check:upstreams` before choosing integration work. It reads
   `scripts/upstreams.json`, queries GitHub through authenticated `gh api`, and
   reports commit/release drift without fetching or changing local refs.
2. **Launch the instance.** One instance, every profile on — a supervised `hub`
   process, not a bare shell command:

   ```text
   hub op:"start" name:"stump-komga" application:"scripts/dev-fixture-server.sh"
        ready:{ port: 25600, timeout: 120 }
   ```

   The launcher builds/uses `target/debug/stump_server`
   (`--no-default-features --features headless,liseur-sync`) and exports
   `STUMP_ENABLE_KOMGA`, `ENABLE_KOBO_SYNC`, `ENABLE_KOREADER_SYNC`,
   `STUMP_ENABLE_ABS`, `KOBO_KEPUB_CONVERSION`, `STUMP_ENABLE_UPLOAD`,
   `PDFIUM_PATH`, `INGEST_EDITOR_DIR`, `STUMP_HOME_APP_DIR`, and
   `STUMP_VERBOSITY=2`. Add
   `STUMP_ENABLE_KAVITA=true STUMP_ENABLE_BACKGROUND_JOBS=true` for the Kavita
   and jobs lanes. Keep remote providers disabled with
   `STUMP_ENABLE_PROVIDERS=false`. MAM acquisition is enabled only when both
   `~/.config/mam-bridge/url` and `~/.config/mam-bridge/api-token` exist
   (local, mode 600); never write the bridge origin or token anywhere.
   Fixture root `$HOME/.local/share/stump-komga-test`; the launcher regenerates
   `$FIXTURE_ROOT/ENDPOINTS.md` (0600) with every URL and credential — never
   quote that sheet into a commit, doc, or message.

3. **Know the profiles.** `--no-default-features` is leaner than
   `--no-default-features --features minimal`; `minimal=["formats"]`. The
   KOReader-only composition is
   `--no-default-features --features koreader` plus
   `ENABLE_KOREADER_SYNC=true`. `webui=["graphql", "graphql/web"]`, so WebUI
   implies GraphQL.
4. **Replay.** The last full server-contract pass (2026-09-21) was green: 15
   Hurl files, 341 requests, 0 failures; it was not rerun on 2026-09-25.
   From the user-owned sibling harness
   `../komga-compat/`, use `LD_LIBRARY_PATH=/tmp` and
   `PATH=$HOME/.cargo/bin:$PATH`; inject runtime credentials and IDs from the
   owner-only fixture sheet instead of writing them into this file. The exact
   evidence and current results live in `.omp/PROJECT_STATE.md`.

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

   Run Mihon before Komf because Komf mutates the synthetic series. The
   `replay-containers` pending spec remains explicitly unrun.

5. **Gate.** Only the command list in `.omp/PROJECT_STATE.md` counts as green.
   The 2026-09-25 gate (Rust, schema, Bun install/type/Komf-contract checks,
   Home/Editor/docs builds, KOReader plugin checks) passed. Replays,
   physical-device verification, and the pending containers replay are not
   part of that claim.
6. **Docs.** `cd docs && bun run build` must exit 0. `detect-libc` must resolve
   to `^2` for lightningcss 1.33 (`familySync`).

## Open items

Each names its blocker, where the design lives, and the first move. None is
waiting on an unnamed decision.

### 1. User-gated actions (highest priority; each needs an explicit go)

1. **Live MAM grab end-to-end.** Bridge is ready/live and search works
   (probe + full search, top hit scored, no grab performed). Exercise Add →
   `grabRelease(confirm)` → polling → completed download copied into staged
   ingest → Editor approval → request fulfilled. Do not grab without the
   user's go.
2. **Publish `koreader-coppice`** as public `coppice-media/koreader-coppice`
   (approved; not yet pushed — confirm the push). Every other sibling repo
   (`nickelcoppice`, `stump-mihon-extension`) needs its own confirmation.
3. **Pinned Komf run + protocol replays** (Komga/Kavita/Mihon/ABS/Liseur)
   were not rerun this session; blocked on user testing time.
4. **Phone check of The Lottery notes**: the merged work resolves 200 and 3
   Home notes are present server-side; the phone view is confirmed only when
   the user opens the book.
5. **Delete fixture test requests** created by UI workers (Project Hail Mary
   and two summary books).

### 2. Device runs that need hardware or an account

| Gap                                        | Blocker                                                                                                                                         | Where the contract lives                                                             |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Broader official Audiobookshelf app retest | the scoped Android 35 emulator run is **Device-tested** and the socket.io lane is **Contract-tested**; a broader phone run still needs hardware | `docs/content/docs/developer/abs-compat.mdx`, `docs/audiobook-study.md` §2-3         |
| Physical Kobo against `/kobo/{api_key}`    | no device                                                                                                                                       | `docs/content/docs/developer/kobo-sync-capabilities.mdx`, `kobo-device-database.mdx` |
| `koreader-coppice` plugin on e-ink KOReader | **Device-tested** on PC + Android phone (zip `c87175dc`, Home notes anchored without duplicates); no e-ink reader run yet                       | `docs/content/docs/developer/koreader-plugin.mdx`                                    |
| A real `@kindle.com` delivery              | no address configured; only 4 route tests exist                                                                                                 | `docs/content/docs/developer/platforms.mdx:79-99`, `crates/kindle/README.md`         |

The remaining rows are **Blocked** on physical access or credentials, not on a
claim that the server contract is absent. When a device appears: launch the
fixture, hand over `$FIXTURE_ROOT/ENDPOINTS.md`, and triage with the recipe in
`client-verification.mdx` (“How a device report is triaged”) — fix at the adapter
boundary, add a focused test, and pin the shape in a Hurl spec.

### 3. API keys for Hardcover, Metron, Open Library, and Google Books

The four metadata clients are **Shipped** and **Contract-tested** against
recorded payloads (`crates/integrations/metadata/src/providers/{hardcover,metron,openlibrary,googlebooks}.rs`).
No live credential exists, so live mapper, rate-limit, and error verification is
**Blocked**. Keys are stored per provider in
`metadata_provider_configs.encrypted_api_token`; the editor's API-keys page
writes them. First move once a key exists: `verify_credentials`, then one real
identify through the ingest editor, then record the run in
`client-verification.mdx`. Hardcover account, progress, and journal sync are
separate **Planned** work and remain **Blocked** on a stable authenticated API.
User-gated.

### 4. Upstream integration and split

The v0.1.10 security merge is committed on `coppice/nightly` (baseline
`29115590`). The current uncommitted tree is gated but not yet committed;
commit/push only with fresh explicit authorization.

Keep `coppice/nightly` as the long-lived fork line. For an upstream PR, claim
or open the issue first, reconstruct a fresh branch from an immutable commit on
`stumpapp/stump`'s then-current `nightly` (not Coppice history), target upstream
`nightly`, and keep the PR narrow with focused tests/docs plus required
Prettier/rustfmt checks. Disclose LLM assistance in the PR body and do not use
an LLM-signed commit.

### 5. ABS socket.io lane — shipped; broader device run pending

The Engine.IO 4 / Socket.IO protocol 5 lane is mounted at `/socket.io`, accepts
the access-token `auth` event, emits the official app's required initialization
and update events, and is documented in
`docs/content/docs/developer/abs-compat.mdx`. The
`socket.io-client@4.8.3` fixture proof on 2026-09-11 connected over WebSocket,
authenticated, and received `init` for the fixture owner. That is pre-merge
evidence; the post-merge focused replay and broader official-app phone run
remain pending.

### 6. Ebook ↔ audiobook sync — delivery built, playback evidence open

Confirmed edition pairing, editable `media_chapter_map`, read-time position
conversion, the worker queue, typed `AlignInput`/`SyncMapV1` validation,
`media_sync_maps`, `Media.syncMap`, map import, strict source-EPUB SMIL
import, worker-local Storyteller and operator-configured native CTC backends,
active-job-deduplicating alignment enqueue, the validating finalizer, and
authenticated cache-only read-aloud status/download routes are **Shipped**.
Readiness quality checks, Storyteller UUID reuse, exact-edition manifest
export, virtual composite delivery, and physical-reader playback remain
**Planned**; no device playback evidence is claimed.

Source of truth: `docs/content/docs/developer/read-aloud.mdx`; operator
contract: `.omp/skills/stump-read-aloud/`; measured evidence:
`docs/audiobook-study.md` §4 and `input/audio/experiments/`.

Next move: render the standards-based EPUB, then test a virtual composite ZIP
whose stored audio entry maps directly to source-M4B ranges. Accept it only
after byte/reference, archive, range, source-invalidation, and real-reader
checks; retain a materialized fallback.

## Request workflow and MAM acquisition

The metadata-backed request ledger and Home Requests UI are **Shipped**: users
create intent with format (`EBOOK`/`AUDIOBOOK`/`ANY`), ISBN, optional
preferred narrator, destination, and visibility; managers approve or reject
it. Acquisition is **Shipped** through the separate MAM Bridge sidecar
(feature `mam-acquisition`, permission `ACQUIRE_RELEASES`, env
`STUMP_ENABLE_MAM_ACQUISITION`/`MAM_BRIDGE_*`; details in
`.omp/PROJECT_STATE.md`):

```text
metadata search -> request -> manager approval
                -> bridge searchReleases -> grabRelease(confirm)
                -> polling job -> completed download copied to staged ingest
                -> Editor approval -> fulfilled
```

Live evidence stops at search (bridge ready/live, probe + full search); the
grab leg is open item 1.1. Credentials, tracker cookies, VPN/qBittorrent
control, and the bridge origin never enter this repository. Known limit: the
live Hardcover index has no `audio_seconds`, so audiobook length in search
comes only from Audible. See
`docs/content/docs/developer/integration-architecture.mdx` and
`docs/content/docs/guides/integrations/acquisition.mdx`.

## Standing reminders

- `crates/migrations/src/lib.rs` is append-only: keep every existing `mod` and
  `Box::new` in order, one SQLite column per `alter_table`.
- Shared files (`core/src/lib.rs`, `core/src/{context,event}.rs`, GraphQL
  `mod.rs`, `crates/migrations/src/lib.rs`, root `Cargo.toml`) are edited as
  single-line hunks after a fresh read; wholesale rewrites have lost `pub mod`
  lines and migration registrations before.
- Write transactions go through `models::txn::begin_write` (SQLite
  `BEGIN IMMEDIATE`); plain `begin()` is read-only work only.
- Studies live outside the site build: `docs/drm-study.md`,
  `docs/audiobook-study.md`.
- `home/`'s `bun run build` is not concurrency-safe (it wipes
  `.svelte-kit/output`): serialize builds and remove that output only after an
  aborted build.
- Scanner implementation stays in `crates/scanner`; watcher implementation
  stays in `crates/watcher`. Do not restore the removed `core/src/scan`
  duplicates.
- liseur-sync attachment side objects are a Coppice extension, not a pinned
  client/device claim; bytes are content-addressed and retention sweeping is
  deferred.

## Resume point (updated 2026-09-25)

The 2026-09-22 source-worker/metadata-cover/read-aloud/Liseur work is
committed in baseline `29115590`. Completed in the current uncommitted tree
(details and live numbers in `.omp/PROJECT_STATE.md` "Shipped 2026-09-24/25"):

- Legacy React/Vite/Expo/desktop/web packages removed; Bun workspaces are
  docs, home, editor, packages/stump-ui.
- MAM acquisition via the MAM Bridge sidecar (search live-probed; no grab).
- `unifiedSearch` split into `librarySearch` / `externalBookSearch` /
  `audibleBookSearch`; `audiobookNarrators`; request `format`, `isbn`,
  `preferredNarrator`; Home ⌘K search dialog and `RequestFormatControl`.
- Lazy WebP thumbnails on demand (1.6 MB → 47 KB, repeat ~4 ms).
- Liseur pin v0.19.0, `/v1/events` 404, work-identity repair migrations,
  SPA 404 prefixes, log rolling, annotation sync.
- KOReader plugin note anchoring fixed and installed on PC + phone.
- Migrations `m20260958`–`m20260967`; the 2026-09-25 gate is green.

Still actionable, by priority:

1. **User-gated (open item 1):** live MAM grab end-to-end; publish
   `koreader-coppice` (approved, not pushed); pinned Komf run + protocol
   replays; phone check of The Lottery notes; delete the fixture test
   requests.
2. **Commit/push** the gated tree to `coppice/nightly` only with fresh
   explicit authorization.
3. **`liseur_sync/storage.rs` split** (5,053 lines) — deferred until after
   device testing; do not start it mid-verification.
4. **Official ABS app phone retest:** physical-device proof remains required
   for the play retry loop, covers, download, playlists, ebooks tab, and live
   bookmark updates.
5. **Source-worker later phases:** keep `candidate_only` disabled until an
   interest-set protocol exists. Automatic publication, desired-placement
   reconciliation, durable byte caching/eviction, verification scheduling, and
   replication remain planned; do not merge them into acquisition authority.
6. **Read-aloud later phases:** readiness quality checks, verified Storyteller
   UUID reuse, exact-edition manifest export, gap-only repair, optional
   virtual-composite delivery, physical-reader playback evidence.
7. **Provider evidence:** Google Books is externally rate-limited (HTTP 429);
   MAL, Hardcover (beyond the public index), Comic Vine, and Metron live
   probes require credentials; do not fabricate green results. Hardcover
   audiobook length is unavailable in search (index lacks `audio_seconds`).
8. **Upstream preparation:** open/confirm the upstream issue before
   reconstructing narrow PR branches from immutable `stumpapp/stump`
   `nightly`; follow `.github/CONTRIBUTING.md`, disclose LLM assistance.
9. **Deferred remote definitions:** keep `STUMP_ENABLE_PROVIDERS=false` and the
   sibling `stump-sources` repository local-only until the browser-worker
   Cloudflare authentication model and source-by-source linkage review exist.
10. **Not worth chasing:** 48 duplicate crate versions in the dependency graph.
