# Coppice Next Steps

Agent-facing resume file for branch `coppice/nightly`. Baseline commit
`30d251886fe614053ea43ec9d50a8ab4cb0b2753` is pushed; the current uncommitted,
fully gated tree removes private in-process MAM acquisition, retains the generic
request ledger, and adds source-import approval, read-only Calibre discovery,
metadata-cover persistence, native read-aloud paths, and Liseur fixes.
Remaining workstreams are physical-device verification, deferred placement and
readiness features, credentialed provider probes, and upstream submission.
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
   `PDFIUM_PATH`, `INGEST_EDITOR_DIR`, `STUMP_HOME_APP_DIR`
   (`scripts/dev-fixture-server.sh:108-118`). Add
   `STUMP_ENABLE_KAVITA=true STUMP_ENABLE_BACKGROUND_JOBS=true` for the Kavita
   and jobs lanes. Keep remote providers disabled with
   `STUMP_ENABLE_PROVIDERS=false`.
   Fixture root `$HOME/.local/share/stump-komga-test`; the launcher regenerates
   `$FIXTURE_ROOT/ENDPOINTS.md` (0600) with every URL and credential — never
   quote that sheet into a commit, doc, or message.

3. **Know the profiles.** `--no-default-features` is leaner than
   `--no-default-features --features minimal`; `minimal=["formats"]`. The
   KOReader-only composition is
   `--no-default-features --features koreader` plus
   `ENABLE_KOREADER_SYNC=true`. `webui=["graphql", "graphql/web"]`, so WebUI
   implies GraphQL.
4. **Replay.** The 2026-09-21 server-contract pass is green: 15 Hurl files,
   341 requests, 0 failures. From the user-owned sibling harness
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
   The 2026-09-20 Rust gate, schema drift check, JavaScript/Svelte checks and
   builds, authenticated browser smoke, and replay set passed. Physical devices,
   Expo native visual verification, and the pending containers replay are not
   part of that claim.
6. **Docs.** `cd docs && bun run build` must exit 0. `detect-libc` must resolve
   to `^2` for lightningcss 1.33 (`familySync`).

## Open items

Each names its blocker, where the design lives, and the first move. None is
waiting on an unnamed decision.

### 1. Device runs that need hardware or an account

| Gap                                        | Blocker                                                                                                                                         | Where the contract lives                                                             |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Broader official Audiobookshelf app retest | the scoped Android 35 emulator run is **Device-tested** and the socket.io lane is **Contract-tested**; a broader phone run still needs hardware | `docs/content/docs/developer/abs-compat.mdx`, `docs/audiobook-study.md` §2-3         |
| Physical Kobo against `/kobo/{api_key}`    | no device                                                                                                                                       | `docs/content/docs/developer/kobo-sync-capabilities.mdx`, `kobo-device-database.mdx` |
| `stump.koplugin` inside KOReader           | never loaded on a device or desktop KOReader                                                                                                    | `docs/content/docs/developer/koreader-plugin.mdx` (`:29` states the limit)           |
| A real `@kindle.com` delivery              | no address configured; only 4 route tests exist                                                                                                 | `docs/content/docs/developer/platforms.mdx:79-99`, `crates/kindle/README.md`         |

The remaining rows are **Blocked** on physical access or credentials, not on a
claim that the server contract is absent. When a device appears: launch the
fixture, hand over `$FIXTURE_ROOT/ENDPOINTS.md`, and triage with the recipe in
`client-verification.mdx` (“How a device report is triaged”) — fix at the adapter
boundary, add a focused test, and pin the shape in a Hurl spec.

### 2. API keys for Hardcover, Metron, Open Library, and Google Books

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

### 3. Upstream integration and split

The v0.1.10 security merge is committed on `coppice/nightly`; its full gate and
341-request replay passed before this current MAM-removal/Bun-cutover tree. The
security fixes block self-permission escalation, guard
`libraryMissingEntities`, scope `mediaMetadataOverview`, and enforce book-club
read access.

Keep `coppice/nightly` as the long-lived fork line. For an upstream PR, claim
or open the issue first, reconstruct a fresh branch from an immutable commit on
`stumpapp/stump`'s then-current `nightly` (not Coppice history), target upstream
`nightly`, and keep the PR narrow with focused tests/docs plus required
Prettier/rustfmt checks. Disclose LLM assistance in the PR body and do not use
an LLM-signed commit.

### 4. ABS socket.io lane — shipped; broader device run pending

The Engine.IO 4 / Socket.IO protocol 5 lane is mounted at `/socket.io`, accepts
the access-token `auth` event, emits the official app's required initialization
and update events, and is documented in
`docs/content/docs/developer/abs-compat.mdx`. The
`socket.io-client@4.8.3` fixture proof on 2026-09-11 connected over WebSocket,
authenticated, and received `init` for the fixture owner. That is pre-merge
evidence; the post-merge focused replay and broader official-app phone run
remain pending.

### 5. Ebook ↔ audiobook sync — persistence and delivery built, alignment open

Blockers are a complete align runner and an operator-configured Storyteller
acceptance instance. Confirmed edition pairing, editable
`media_chapter_map`, read-time position conversion, the worker queue, typed
`AlignInput`/`SyncMapV1` validation, `media_sync_maps`, `Media.syncMap`, map
import, active-job-deduplicating alignment enqueue, and authenticated
cache-only read-aloud status/download routes are **Shipped**. The finalizer
validates the requester/pair and worker output before publishing the cached
EPUB. Storyteller/SMIL import, a complete align/playback run, readiness checks,
and virtual composite delivery remain **Planned**; no post-merge evidence is
claimed.

Source of truth:
`docs/content/docs/developer/read-aloud.mdx`; operator contract:
`.omp/skills/stump-read-aloud/`; measured evidence:
`docs/audiobook-study.md` §4 and `input/audio/experiments/`.

Next moves, in order:

1. import exact existing Storyteller/SMIL timing before spending compute;
2. add a worker-local Storyteller API adapter: reuse an exact UUID when present,
   otherwise upload/process/poll/download through the operator's instance; keep
   credentials off the server and return only `SyncMapV1`. Storyteller remains a
   worker alignment backend, never a search/catalog provider or bundled
   dependency/default backend;
3. add the independent native known-text CTC/Viterbi runner;
4. render the standards-based EPUB, then test a virtual composite ZIP whose
   stored audio entry maps directly to source-M4B ranges. Accept it only after
   byte/reference, archive, range, source-invalidation, and real-reader checks;
   retain a materialized fallback.

## Request workflow and planned external fulfillment

The metadata-backed request ledger and Home Requests UI are **Shipped**: users
create intent with destination and visibility; managers approve or reject it;
permissions, notifications, recommendations/social handoff, and historical
status decoding remain Coppice-owned.

The former release search/selection, automation, grab/poll/retry operations,
acquisition ORM, scheduler jobs, and private `mam-gateway` implementation are
removed in the current tree. Future acquisition remains:

```text
metadata search -> request -> manager approval
                -> authenticated provider sidecar
                -> shared-folder or equivalent ingest input
                -> staged ingest + identifier match -> fulfilled
```

Shelfmark's release-source/download-handler split is design inspiration, not a
1:1 subsystem or runtime dependency. MouseSearch is the current candidate for a
separate operator-only sidecar after it exposes a stable token-authenticated
JSON service API; Coppice must not scrape its HTML UI, store tracker cookies,
control VPN/qBittorrent, or imply metadata search acquired a file. See
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

## Resume point (updated 2026-09-22)

Completed in the current uncommitted tree:

- The private real-input archive ingest smoke passed end-to-end.
- Source workers now cache successful full-digest checks by exact local file
  identity, scan `kind=calibre` roots read-only, create typed digest/lineage-bound
  match proposals, and persist explicit approve/reject decisions before
  idempotent link/materialize effects.
- Metadata policy application persists its selected cover through a bounded,
  SSRF-hardened, staged filesystem/SQLite commit path.
- Native EPUB SMIL import, deterministic streamed-M4B read-aloud rendering, and
  an operator-configured external CPU/fp32 CTC worker backend are implemented.
- Liseur invalid query defaults and full-SHA/stable-source catalog resolution
  match the pinned client contract. Narrow reversible upstream patch artifacts
  were prepared in `/tmp` for the limit and SHA changes; reconstruct them from
  the current diff if the temporary files are gone.
- The exact Rust/Bun/schema/build gate in `.omp/PROJECT_STATE.md` is green.

Still actionable:

1. **Official ABS app phone retest:** physical-device proof remains required for
   the play retry loop, covers, download, playlists, ebooks tab, and live
   bookmark updates. Automated server evidence is not a substitute.
2. **Source-worker later phases:** keep `candidate_only` disabled until an
   interest-set protocol exists. Automatic publication, desired-placement
   reconciliation, durable byte caching/eviction, verification scheduling, and
   replication remain planned; do not merge them into acquisition authority.
3. **Read-aloud later phases:** add readiness quality checks, verified
   Storyteller UUID reuse, exact-edition manifest export, gap-only repair,
   optional virtual-composite delivery, and physical-reader playback evidence.
   The shipped external native runner does not require an in-process Rust model.
4. **Provider evidence:** Google Books is externally rate-limited (HTTP 429).
   Re-run only after quota recovery. MAL, Hardcover, Comic Vine, and Metron
   live probes require credentials; do not fabricate green results.
5. **Upstream preparation:** open/confirm the relevant upstream issue before
   reconstructing narrow PR branches from immutable `stumpapp/stump` `nightly`.
   Follow `.github/CONTRIBUTING.md`, include behavior-focused tests/docs, and
   disclose LLM assistance. Do not push from this tree without fresh explicit
   authorization.
6. **Deferred remote definitions:** keep `STUMP_ENABLE_PROVIDERS=false` and the
   sibling `stump-sources` repository local-only. Revisit only after the
   browser-worker Cloudflare authentication model and source-by-source public
   linkage review exist.
