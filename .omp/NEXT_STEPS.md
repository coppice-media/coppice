# Coppice Next Steps

Agent-facing resume file for `coppice/client-updates-2026-10-04`, based on
the published `coppice/nightly` tip `057d2da8` (2026-09-26). The current
client-contract fixes, guest reader, Users screen, account-reader annotations,
and quality-settings changes are uncommitted. The 2026-10-04 gate and
server-contract replay passed; exact commands, counts, exclusions, and
browser evidence live in `.omp/PROJECT_STATE.md`.

`coppice-media/koreader-coppice` is public (`main`, `35a1d5b`); later host
checks do not imply a new phone/e-ink run. Remaining workstreams are
deployment/client verification, live MAM grab, optional stack links,
placement/readiness features, credentialed provider probes, sibling
publication, and upstream submission.
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
4. **Replay.** The latest server-contract pass is 2026-10-04: 12 Hurl files,
   375 requests, 0 failures, against a disposable fixture DB and the full
   binary. Pinned sidecar smokes, Komf→Kavita, Readium, and the containers
   spec were not run in that pass. From `../komga-compat/`, use
   `LD_LIBRARY_PATH=/tmp` and `PATH=$HOME/.cargo/bin:$PATH`; inject runtime
   credentials and IDs from the owner-only fixture sheet, never from docs.
   Exact evidence and fixture preparation live in `.omp/PROJECT_STATE.md`.

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
   The current uncommitted tree passed the 2026-10-04 gate. Server-contract
   replays and browser smokes have separate dated records; neither proves
   physical-device compatibility or live Pangolin deployment.
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
2. **Publish sibling repositories.** `coppice-media/koreader-coppice` is public
   (`main`, `35a1d5b`). `nickelcoppice` and `stump-mihon-extension` each
   need repository-specific confirmation; do not alter their unrelated work.
3. **Remaining replay lanes.** The main Komga/Kavita/Mihon/ABS/Liseur/native
   Komf replays passed on 2026-10-04. Still unrun in that pass:
   `replay-komf-kavita`, pinned Komf sidecar smokes, `replay-readium`, and
   the pending `replay-containers` spec. Use disposable fixture state.
4. **Phone check of recent-note taps**: the phone holds The Lottery as
   `72229c58`, the Home notes name `6829437f` (same KOReader hash
   `285f7004…`). With the 2026-09-26 plugin build, tapping a recent note must
   open the local copy and jump to the note.
5. **Delete fixture test requests** created by UI workers (Project Hail Mary
   and two summary books).

### 2. Device runs that need hardware or an account

| Gap                                        | Blocker                                                                                                                                         | Where the contract lives                                                             |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Broader official Audiobookshelf app retest | the scoped Android 35 emulator run is **Device-tested** and the socket.io lane is **Contract-tested**; a broader phone run still needs hardware | `docs/content/docs/developer/abs-compat.mdx`, `docs/audiobook-study.md` §2-3         |
| Physical Kobo against `/kobo/{api_key}`    | no device                                                                                                                                       | `docs/content/docs/developer/kobo-sync-capabilities.mdx`, `kobo-device-database.mdx` |
| `koreader-coppice` plugin on e-ink KOReader | **Device-tested** on PC + Android phone (zip `c87175dc`, Home notes anchored without duplicates); no e-ink reader run yet                       | `docs/content/docs/developer/koreader-plugin.mdx`                                    |
| Reviewed client updates | Latest Komelia needs the supplied Komf URLBuilder patch in a rebuilt client before query-key retesting; Liseur v0.20, Lissen grouping, and Kover chapter-list changes still need dated client runs | `client-verification.mdx`, `komelia-komf.mdx`, `liseur-sync-integration.mdx`, `abs-compat.mdx`, `kavita-compat.mdx` |
| A real `@kindle.com` delivery              | no address configured; only 4 route tests exist                                                                                                 | `docs/content/docs/developer/platforms.mdx:79-99`, `crates/kindle/README.md`         |

The remaining rows are **Blocked** on physical access or credentials, not on a
claim that the server contract is absent. When a device appears: launch the
fixture, hand over `$FIXTURE_ROOT/ENDPOINTS.md`, and triage with the recipe in
`client-verification.mdx` (“How a device report is triaged”) — fix at the adapter
boundary, add a focused test, and pin the shape in a Hurl spec.

### 3. Live metadata-provider evidence

The direct adapters are shipped with offline contracts. Recorded live probes
on 2026-09-22 passed AniList, MangaDex, MangaUpdates, Open Library, and
Audible; Google Books returned HTTP 429 (a quota blocker, not a missing
mandatory key). MAL, Comic Vine, Metron, and Hardcover still need scoped
authenticated probes when credentials are available.

System provider credentials belong to
`metadata_provider_configs.encrypted_api_token`; a user's Hardcover PAT is a
separate personal connection. Personal connection storage, consent-gated
metadata/narrator lookup, and manual `reading_journals` quote import with
optional local progress projection are shipped. A read-only authenticated probe
on 2026-10-04 confirmed the live schema and a `me.id`-scoped selection query;
the configured personal connection has metadata consent enabled and journal
import disabled. No live journal rows were fetched or imported, so pagination,
real-row mapping, and live progress projection remain unverified.
Synthetic `hardcover://` page locators do not prove exact EPUB/Kobo anchoring.
Remote writes, exact-edition/text-anchor mapping, and reconciliation remain
planned. See `provider-status.mdx` and `hardcover-integration.mdx`; never
replace offline or schema evidence with an unrun live claim.

### 4. Upstream integration and split

The published fork tip is `057d2da8` on `coppice/nightly`. The October work
is local on `coppice/client-updates-2026-10-04`; every commit and push needs
fresh confirmation of the exact branch, destination, and changes.

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
authenticated, and received `init` for the fixture owner. The 2026-10-04
ABS HTTP replay does not replace a fresh socket.io-client run or a broader
official-app phone test.

### 6. Ebook ↔ audiobook sync — delivery built, playback evidence open

Confirmed edition pairing, editable `media_chapter_map`, read-time position
conversion, the worker queue, typed `AlignInput`/`SyncMapV1` validation,
`media_sync_maps`, `Media.syncMap`, map import, strict source-EPUB SMIL
import, worker-local Storyteller and operator-configured native CTC backends,
active-job-deduplicating alignment enqueue, the validating finalizer, and
authenticated cache-only read-aloud status/download routes are **Shipped**.
Readiness quality checks, Storyteller UUID reuse, exact-edition manifest
export, virtual-composite delivery, and physical-reader playback remain
**Planned**; no synchronized device playback evidence is claimed.

Source of truth: `docs/content/docs/developer/read-aloud.mdx`; operator
contract: `.omp/skills/stump-read-aloud/`; measured evidence:
`docs/audiobook-study.md` §4 and `input/audio/experiments/`.

The deterministic streamed M4B-to-EPUB renderer and cache are already shipped;
do not list rendering as unimplemented. Next: readiness/edition validation
and synchronized playback proof. Virtual-composite delivery remains a
separate design requiring archive/range/source-invalidation and reader checks.

### 7. Book-club guest reader and 2026-10-04 UI gaps

Guest reading links are **Contract-tested** (club_reader HTTP regressions,
incl. SSE and discussion) and browser-smoked on loopback (see
`.omp/PROJECT_STATE.md`). Open:

1. **Apply the Pangolin bypass on the real resource** (user action; no
   Pangolin access from here). Exactly the three rules in
   `docs/content/docs/guides/features/book-clubs/guest-reader.mdx`, then
   open a link from outside the LAN to confirm.
2. Organizer discussion view in `/social` refreshes on open/Refresh only
   (guests get live SSE).
3. The member discussion and book-suggestion/vote GraphQL APIs exist, but
   `BookClubPanel.svelte` does not expose their controls. Guest-session
   discussion is a separate shipped UI; do not claim member-room parity.

Resolved 2026-10-04: guest live updates + session discussion, account-reader
annotation authoring (EPUB/page/audio), `/app/users`, Editor quality-check
setting controls (and the server never reading them), `packages/stump-ui`
codegen devDependencies, Lissen ABS grouping routes, Kover Kavita
`chapters-by-series`.

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

## Resume point (updated 2026-09-26)

Pushed in `47a9c61c` (details and live numbers in `.omp/PROJECT_STATE.md`
"Shipped 2026-09-24/25"):

- Legacy React/Vite/Expo/desktop/web packages removed; Bun workspaces are
  docs, home, editor, packages/stump-ui.
- MAM acquisition via the MAM Bridge sidecar (search live-probed; no grab).
- `unifiedSearch` split into `librarySearch` / `externalBookSearch` /
  `audibleBookSearch`; `audiobookNarrators`; request `format`, `isbn`,
  `preferredNarrator`; Home ⌘K search dialog and `RequestFormatControl`.
- Lazy WebP thumbnails on demand (1.6 MB → 47 KB, repeat ~4 ms).
- Liseur pin v0.19.0, `/v1/events` 404, work-identity repair migrations,
  SPA 404 prefixes, log rolling, annotation sync.
- KOReader plugin note anchoring fixed; migrations `m20260958`–`m20260967`.

2026-09-26 follow-up (device report: phone "Offline", empty Continue reading,
recent note opened the wrong book):

- The server had aborted with a stack overflow: the plugin's book-detail query
  (media → series → media, full field set) exceeded Tokio's 2 MiB worker stack
  in the debug build. Runtime threads now get 8 MiB
  (`apps/server/src/config/runtime.rs`).
- Liseur-sync notes name their edition, then a non-audio edition, then the
  oldest link (window-ranked; SQLite rejects outer columns in a scalar
  subquery's `ORDER BY`). `AnnotationBook.koreaderHash` lets the plugin open a
  byte-identical local copy; the plugin matches on-device files by
  `partial_md5_checksum`.

## Remaining-work index

The numbered open items above own verification blockers and user-gated
actions. Do not duplicate them as a second implementation queue.

Additional engineering work, with its source of truth:

| Work | Boundary / next action |
| --- | --- |
| Hardcover consent and journal import | Implemented in `crates/graphql/src/query/unified_search.rs`, `mutation/hardcover.rs`, and `integrations/metadata/src/providers/hardcover.rs`. Authenticated `/reading_journals` schema and empty-page query verified; the disposable live connection had no enabled global provider or import consent, so no real journal rows were fetched/imported. Synthetic journal/progress tests cover quote preservation, pagination, consent, and future-dated updates. |
| Optional stack links | `integration-architecture.mdx` “Optional stack links”: preserve solo operation; publish a CI-fetchable pinned shared metadata source before Coppice cutover, then wire MetaMeta configuration/policy. Hub request authority, scoped read export, reviewed user mapping, and shared acquisition extraction remain planned. No moves or data/secret migration authorized. |
| Source-worker later phases | `remote-worker-libraries.mdx`: interest-set protocol before enabling `candidate_only`; automatic matching/publication, placement, caching/eviction, verification scheduling, and replication remain planned. |
| OIDC account migration | Implemented atomic ownership transfer of discovered SQLite references, with rollback on conflicts/unmapped saved jobs and session revocation. Approved-but-unissued device pairings are denied before ownership moves. CLI tests and a disposable CLI smoke passed: reading head, Hardcover connection, and Liseur annotation transferred; source row and sessions were removed. A conflicting Hardcover connection failed and rollback preserved both accounts and rows. |
| Permanent account-deletion contract | Implemented soft/hard deletion, explicit retained public/shared records, capability revocation, source-worker liveness and transactional inventory writes, retry checks, and UI wording in `UserActionDialog.svelte`. Soft/hard lifecycle and live socket regressions cover the contract; disposable Home hard-delete UI smoke verified typed confirmation and removal of the synthetic user's remaining ownership references. External Markdown/Git exports remain untouched. |
| Annotation export edition selection | Implemented confirmed/visible/ready non-audiobook edition ranking in `crates/annotation-sync/src/model.rs`; exact annotation edition hash wins and unmatched exports remain standalone. Focused tests and disposable runtime export passed; exact-edition, unbound, and unmatched annotations routed as intended. |
| Liseur storage module split | `apps/server/src/routers/liseur_sync/storage.rs`: deferred structural cleanup after behavior/device verification; not a compatibility fix. |
| Annotation attachment retention | Content-addressed attachments ship; retention sweeping remains deferred. |
| Reading timeline (optional) | Upstream `b988179f` groups `reading_sessions` by `readthrough_number` and attaches bookmarks/annotations via a new `session_id` FK. Do not port the FK migration: KOReader/Liseur/Kobo/ABS events carry no Stump session id, so it would be empty for most history. Instead derive a read-only Home/GraphQL view: sessions by readthrough + elapsed time, with bookmark/annotation (and `position_ms` audio note) events assigned at query time to the same user/media session whose interval contains `created_at`. Mark unassigned events unassigned; never guess. ABS `abs_sessions` listening time joins only after its projection into `reading_sessions` is verified. |
| Remote definitions | Keep `STUMP_ENABLE_PROVIDERS=false` and `stump-sources` local-only until browser-worker authentication and source-by-source linkage review exist. |
| Upstream submission | Claim/open the issue, reconstruct a narrow branch from immutable upstream `nightly`, follow `.github/CONTRIBUTING.md`, disclose LLM assistance. Never blanket-merge legacy app changes. |

Resolved in the October working tree: EPUB spine reading direction,
Komga author existence/negation, stable OPDS progression and preview
pagination, older Kavita sort bodies, Lissen grouping, Kover chapter lists,
guest SSE/discussion, Users management, account-reader annotation authoring,
and Editor quality settings. Komf query-key handling has a client patch
artifact; the released client is not thereby fixed or device-tested.
