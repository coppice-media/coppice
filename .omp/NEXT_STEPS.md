# Coppice Next Steps

Agent-facing resume file for the active no-commit merge on branch
`integrate/upstream-nightly-2026-09-20` (merge target
`origin/nightly` `766c7347dbc0a2fc8c93d04a8f922db3b5a41ba8`).
The remaining workstreams are hardware/account verification, live provider
credentials, upstream PR extraction, and read-aloud execution.
Audiobook sync has shipped tier 1, persistent sync maps,
explicit deduplicated alignment enqueue, and the worker substrate, but no
complete alignment playback flow. Ordering, rationale, and non-goals:
`docs/content/docs/developer/roadmap.mdx`. Evidence for what already landed:
`docs/content/docs/developer/state.mdx`. Working-tree, gate, and disk rules:
`.omp/PROJECT_STATE.md` — that file is authoritative and this one never
restates its command list.

A design page or replay fixture is evidence for a contract, not proof that a
feature is shipped. Integration ownership and evidence labels are defined in
`docs/content/docs/developer/integration-architecture.mdx`.

## Resume recipe

1. **Read state.** `.omp/PROJECT_STATE.md` (working tree, disk discipline,
   "Only definition of green"), then `git log --oneline -3`.
   Run `yarn check:upstreams` before choosing integration work. It reads
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
   `STUMP_ENABLE_KAVITA=true STUMP_ENABLE_BACKGROUND_JOBS=true
STUMP_ENABLE_PROVIDERS=true` for the Kavita, jobs, and provider lanes.
   Fixture root `$HOME/.local/share/stump-komga-test`; the launcher regenerates
   `$FIXTURE_ROOT/ENDPOINTS.md` (0600) with every URL and credential — never
   quote that sheet into a commit, doc, or message.

3. **Know the profiles.** `--no-default-features` is leaner than
   `--no-default-features --features minimal`; `minimal=["formats"]`. The
   KOReader-only composition is
   `--no-default-features --features koreader` plus
   `ENABLE_KOREADER_SYNC=true`. `webui=["graphql", "graphql/web"]`, so WebUI
   implies GraphQL.
4. **Replay.** The 2026-09-20 server-contract pass is green: 15 Hurl files,
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

The active no-commit merge is on
`integrate/upstream-nightly-2026-09-20`: `ORIG_HEAD` is
`0526084b62dc012069a2d1f47a12dde605004010` and `MERGE_HEAD` targets
`origin/nightly` at `766c7347dbc0a2fc8c93d04a8f922db3b5a41ba8`. GitHub's
compare counted 87 upstream commits; local ancestry counts 88 because one
merged release parent is included. The preserved dirty-tree recovery artifacts
and hashes are recorded in `.omp/PROJECT_STATE.md`.

The upstream security commit
`3d5854228d8f314f36ed6aeb9a4714cc4c30ed50` is absorbed: self-permission
escalation is blocked, `libraryMissingEntities` is management-guarded,
`mediaMetadataOverview` is user-scoped, and book-club reads enforce access.
The post-merge full gate and replay set are still pending.

After verification, keep `coppice/nightly` as the long-lived branch. For an
upstream PR, claim or open the issue first, reconstruct a fresh branch from
`origin/nightly` (not a cherry-picked 41-commit integration block), target
`nightly`, and keep the PR narrow with focused tests/docs plus the required
Prettier/rustfmt checks. Disclose LLM assistance in the PR body, do not use an
LLM-signed commit, and never submit the integration branch.

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

The request ledger and Requests UI, approval/release selection, automation
defaults, core processing, download polling, and private `mam-gateway`
implementation are **Shipped**. Mocked protocol-contract evidence and the
post-merge gate/replays remain pending; live MAM/VPN/qBittorrent deployment is
untested.

The future external flow remains:

```text
search -> wanted request -> fulfillment connector
       -> shared-folder export or other ingest input
       -> ingest + identifier match -> fulfilled
```

Shelfmark is not bundled. A shared-folder export from an external Shelfmark
instance into Coppice ingest is the first **Planned** integration. A future
request bridge is **Blocked** until Shelfmark exposes a stable
token-authenticated service API. Manual fulfillment or another lawful provider
host may be a connector; Coppice must not ship a downloader or imply that
search acquired a file. See
`docs/content/docs/developer/integration-architecture.mdx`.

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

## Resume point (updated 2026-09-20)

Not yet done from the user's live test with `input/`:

1. **Real-data ingest run on 25600** (IngestArchives fixed the `unrar l` 4
   KB-truncation bug but was cut off before re-running): copy `input/audio/*.rar`
   - `input/audio/The Three-Body Problem - Cixin Liu.epub` into 25600's drop
     folder, scan from the editor, report per item kind/title/tracks/chapters/
     duration/quality score, the pair suggestion (Three-Body EPUB <-> audiobook),
     and assembled M4Bs (Three-Body 37 MP3, Dark Eden 62 MP3; MP3->AAC via
     `/usr/bin/ffmpeg`, minutes). Env-gated harness:
     `STUMP_LIVE_INPUT=/home/al/Code/stump/input cargo test -p stump_ingest explode_tests::live_archive_drop_of_the_real_input`.
2. **Broader official ABS app retest on a phone** (the scoped emulator evidence
   and socket.io-client fixture proof are recorded): play retry loop, covers,
   download, playlists, ebooks tab, and live bookmark updates.
3. **Deferred Mihon-derived remote sources:** keep `STUMP_ENABLE_PROVIDERS=false`
   (the default) and keep the sibling `stump-sources` repository local-only;
   do not publish its site list. Revisit only after the browser-worker
   Cloudflare authentication model is solved and every candidate source has
   been curated for public linkage.
4. **Read-aloud next:** import exact Storyteller/SMIL timing, then complete the
   worker-side alignment/playback acceptance. The pairing, SyncMap validation,
   alignment enqueue, and authenticated cache-only status/download path are
   present; render the read-aloud EPUB and run the virtual source-M4B ZIP
   experiment before choosing the long-term derivative cache. Then readiness
   and editions provenance.
5. **Post-merge compatibility verification:** the upstream security fixes are
   absorbed, but the full gate and replay set still need to run. Record only
   post-merge evidence, then review remaining Liseur/liseur-sync contracts and
   reconstruct any upstream PR branches from `origin/nightly`.
6. **Calibre library source:** Stump has neither a native WebDAV source nor a
   `metadata.db`-aware Calibre source. Implement the local, read-only Calibre
   source first: treat `metadata.db` as catalog authority and serve discovered
   files through existing Coppice routes/OPDS. Defer native WebDAV credentials,
   cache validation, and ETag handling until a real remote-source need is
   confirmed; an externally mounted WebDAV directory remains the interim path.
   The repository has 36 root EPUB inputs but no Calibre `metadata.db`, and
   Calibre is not installed on this workstation. Build a small public-domain or
   synthetic Calibre fixture instead of committing personal books.
