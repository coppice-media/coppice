# Stump Next Steps

Agent-facing resume file for branch `headless-modular` at `7de64602`
(2026-09-07, batch 9). Five items are open; nothing else is in flight. Ordering,
rationale and non-goals: `docs/content/docs/developer/roadmap.mdx`. Evidence for
what already landed: `docs/content/docs/developer/state.mdx`. Working-tree,
gate and disk rules: `.omp/PROJECT_STATE.md` — that file is authoritative and
this one never restates it.

A design page or a replay fixture is evidence for a contract, not proof that a
feature is shipped.

## Resume recipe

1. **Read state.** `.omp/PROJECT_STATE.md` (working tree, disk discipline,
   "Only definition of green"), then `git log --oneline -3`.
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
   STUMP_ENABLE_PROVIDERS=true` for the Kavita, jobs and provider lanes.
   Fixture root `$HOME/.local/share/stump-komga-test`; the launcher regenerates
   `$FIXTURE_ROOT/ENDPOINTS.md` (0600) with every URL and credential — never
   quote that sheet into a commit, a doc, or a message.
3. **Replay.** From the user-owned sibling harness `../komga-compat/`
   (`LD_LIBRARY_PATH=/tmp`, `PATH=$HOME/.cargo/bin:$PATH`, and every target
   needs `BASE_URL API_KEY USERNAME PASSWORD MITM_KEEP_HOST_HEADER=true`):

   ```text
   make replay                    # 8/8   Komga profile; BOOK_ID/SERIES_ID = "Synthetic Solo"
   make replay-negative-auth      # 1/1
   make replay-mihon              # 1/1   restore Komf-mutated metadata first (see PROJECT_STATE)
   make replay-liseur-sync        # 1/1
   make replay-kavita             # 1/1
   make replay-library-management # 1/1   needs LIBRARY_ROOT=<dir>
   make replay-abs                # 1/1   USERNAME=synthetic-owner PASSWORD=synthetic-pass-1
   make replay-komf               # 16/16 needs the komf-stump container on 8085
   make replay-containers         #       written, never run
   ```

4. **Gate.** Only the command list in `.omp/PROJECT_STATE.md` counts as green,
   run once after every worker has yielded. At `7de64602`: 1741 crate tests +
   317 server tests, both profiles building, replays 7/7.
5. **Docs.** `cd docs && bun run build` must exit 0. `detect-libc` must resolve
   to `^2` for lightningcss 1.33 (`familySync`).

## Open items

Each names its blocker, where the design lives, and the first move. None is
waiting on a decision.

### 1. Device runs that need hardware or an account

| Gap | Blocker | Where the contract lives |
| --- | --- | --- |
| **Official Audiobookshelf app** phone test | no run has happened; needs item 4 first for bookmarks | `docs/content/docs/developer/abs-compat.mdx`, `docs/audiobook-study.md` §2-3 |
| Physical Kobo against `/kobo/{api_key}` | no device | `docs/content/docs/developer/kobo-sync-capabilities.mdx`, `kobo-device-database.mdx` |
| `stump.koplugin` inside KOReader | never loaded on a device or desktop KOReader | `docs/content/docs/developer/koreader-plugin.mdx` (`:29` states the limit) |
| A real `@kindle.com` delivery | no address configured; only 4 route tests exist | `docs/content/docs/developer/platforms.mdx:79-99`, `crates/kindle/README.md` |

First move when a device appears: launch the fixture, hand over
`$FIXTURE_ROOT/ENDPOINTS.md`, and triage with the recipe in
`client-verification.mdx` ("How a device report is triaged") — fix at the adapter
boundary, add a unit test, pin the shape in a Hurl spec.

### 2. API keys for Hardcover, Metron, Open Library, Google Books

Blocker: no live credential. The four clients, their field mappers and their
rate-limit handling are implemented and unit-tested against recorded payloads
(`crates/integrations/metadata/src/providers/{hardcover,metron,openlibrary,googlebooks}.rs`).
Design and per-provider key source:
`docs/content/docs/developer/provider-status.mdx` (Metadata providers table) and
`hardcover-integration.mdx`. Keys are stored per provider in
`metadata_provider_configs.encrypted_api_token`; the editor's API-keys page
already writes them. First move once a key exists: `verify_credentials`, then
one real identify through the ingest editor, then record the run in
`client-verification.mdx`. User-gated.

### 3. Coppice rename and the upstream split

Blocker: user decision on timing. Not started — `grep -ri coppice` over the tree
returns nothing, and `.github/CONTRIBUTING.md` still describes the unchanged
upstream flow. Scope: product and repository name first (binary and package
names later), then the fork-local additive work as separate PRs against
`stumpapp/stump` nightly — core lifecycle and feature-gate changes, then the
protocol profiles (`komga`, `kavita`, `kobo`, `koreader`, `liseur-sync`, `abs`,
`kindle`), then the provider host with its theme engines, then ingest and tools.
PR ordering rationale:
`docs/content/docs/developer/server-architecture.mdx:441-454`
("Incremental upstream PR sequence"). Do not commit or push from a bootstrap
session.

### 4. ABS socket.io lane — in flight

Not in `7de64602`: `crates/abs/README.md:12` says the socket.io lane is not
owned by the crate. Protocol pinned in `docs/audiobook-study.md` §2 — Socket.IO
protocol revision 5, Engine.IO revision 4, path `/socket.io` plus a second
server under a router base path. Target shape and the client contract:
`docs/content/docs/developer/abs-compat.mdx` (owner of that page states the
implementation status). Verification once landed: `make replay-abs`, a
`socket.io-client@4` script against the fixture, then item 1's phone test.

### 5. Ebook ↔ audiobook sync — design accepted, not implemented

Blocker: nothing but sequencing. `reading_heads` already stores a locator *or*
`position_ms` + `track_index` (`crates/migrations/src/m20260933_000000_add_time_positions.rs`);
no conversion exists. The accepted design — tier 1 chapter + percentage, tier 2
forced alignment on a remote worker, Storyteller-compatible EPUB 3 Media
Overlays rendered on demand, the `crates/worker` / `worker_jobs` protocol, and
the requests model — is `docs/content/docs/developer/read-aloud.mdx`; the
measured gap analysis behind it is `docs/audiobook-study.md` §4. Follow its §9
implementation order and start at step 1 (`pair_editions`,
`media_chapter_map`, read-time position conversion), because step 2 (the worker
protocol) is the only other step with a local fallback and everything after it
depends on one of the two.

## Standing reminders

- `crates/migrations/src/lib.rs` is append-only: keep every existing `mod` and
  `Box::new` in order, one SQLite column per `alter_table`.
- Shared files (`core/src/lib.rs`, `core/src/{context,event}.rs`, graphql
  `mod.rs`, `crates/migrations/src/lib.rs`, root `Cargo.toml`) are edited as
  single-line hunks after a fresh read; wholesale rewrites have lost `pub mod`
  lines and migration registrations before.
- Write transactions go through `models::txn::begin_write` (SQLite
  `BEGIN IMMEDIATE`); plain `begin()` is read-only work only.
- Studies live outside the site build: `docs/drm-study.md`,
  `docs/audiobook-study.md`.
- `home/`'s `bun run build` is not concurrency-safe (it wipes
  `.svelte-kit/output`): serialize builds and `rm -rf home/.svelte-kit/output`
  after an aborted one.

## Resume point (paused 2026-09-07 after batch 11, HEAD = this commit)

Not yet done from the user's live test with `input/`:
1. **Real-data ingest run on 25600** (IngestArchives fixed the `unrar l` 4 KB-truncation bug but was cut off before re-running): copy `input/audio/*.rar` + `input/audio/The Three-Body Problem - Cixin Liu.epub` into 25600's drop folder, scan from the editor, report per item kind/title/tracks/chapters/duration/quality score, the pair suggestion (Three-Body EPUB <-> audiobook), and the assembled M4Bs (Three-Body 37 MP3, Dark Eden 62 MP3; MP3->AAC via /usr/bin/ffmpeg, minutes). Env-gated harness: `STUMP_LIVE_INPUT=/home/al/Code/stump/input cargo test -p stump_ingest explode_tests::live_archive_drop_of_the_real_input`.
2. **Official ABS app retest on the phone** (round-2 fixes deployed on 25600 but not phone-verified): play (retry loop), covers, download, playlists, ebooks tab.
3. **Browser lane** for challenged sources (cf_clearance is fingerprint-bound): HTML fetches through the browser worker, images direct; downgrade the cookie recipe in provider-host.mdx.
4. Read-aloud steps 3-4 (align job + aligner container on the RTX 3070; on-demand Storyteller EPUB), then requests, readiness checks, editions/provenance implementation - see docs/developer/read-aloud.mdx and editions.mdx.
5. Worker claim-deadline sweep (a third-party worker that never claims holds a row until disconnect).
