<!-- Read-only study; plain Markdown outside the site build. -->

# Audiobook lane study — servers, clients, state, formats

Read-only study for a future audiobook lane in this fork. No repo files changed, no code written. **[observed]** =
read in the pinned source / official docs / measured here; **[inference]** = conclusion. Licence guard: ABS is
GPL-3.0, Grimmory and BookOrbit AGPL-3.0 — everything below is route/DTO/feature observation (wire behaviour),
which is what `.omp/PROJECT_STATE.md:101` permits; no code is copied.

## 0. Pins, runtimes, measurements

| Subject | Pin | Runtime / licence |
| --- | --- | --- |
| Audiobookshelf (ABS) | `v2.36.0` → `57d01f881cf211dff0a7219312aee2b1bb7ef746`, 2026-07-27 | Node 20 alpine, GPL-3.0 (`package.json:41`), `socket.io ^4.5.4`, `sequelize`+`sqlite3` (`:44-63`) |
| ABS mobile app | `v0.14.0-beta` = `12025ab59a5c9f5654b31b4b68c1fc71ad676f61` | Nuxt 2 + Capacitor 7 + native Kotlin/Swift; `socket.io-client ^4.1.3` |
| ABS API docs | live https://api.audiobookshelf.org/ (Slate), source repo `audiobookshelf/audiobookshelf-api-docs` | 153 documented HTTP routes (extracted from the rendered page) |
| Grimmory | release `v3.3.3` (2026-08-22); `develop` head `703c7f29` (2026-09-05) | Java / Spring Boot (`org.booklore`), AGPL-3.0 |
| BookOrbit | release `v2.8.1` (2026-08-29); `main` head `e3241eea` (2026-09-05) | TypeScript / NestJS + Fastify, AGPL-3.0 |
| symphonia | `v0.6.1` (`ee35874b571a`) | MPL-2.0 |
| Lissen (Android) | `1.11.22-release` = `f30bf9be95ea3792bcc53e4e958576121d450a4d` | Kotlin, MIT |

**[observed] Method** — one stack at a time, empty library, ports 13400/13410/13420, removed afterwards with `docker compose down -v` (verified: 0 containers, 0 images, 0 volumes left; 25600–25699 untouched).
Idle = boot → create admin + login + 3 API calls → 30 s quiescent, then `VmRSS` summed over the container's PIDs
*and* `docker stats` cgroup usage (which subtracts inactive file cache, hence two columns).

| Server | Image | Image size | Idle VmRSS | Idle cgroup | Extra service |
| --- | --- | --- | --- | --- | --- |
| ABS 2.36.0 | `ghcr.io/advplyr/audiobookshelf:2.36.0` | **320 MB** | **37 MiB** | 44.6 MiB | none (SQLite in-process) |
| Grimmory 3.3.3 | `ghcr.io/grimmory-tools/grimmory:v3.3.3` | **481 MB** | **358 MiB** | 380.5 MiB | MariaDB `lscr.io/linuxserver/mariadb:11.4.8`, 348 MB, 8/11.8 MiB — **mandatory** |
| BookOrbit 2.8.1 | `ghcr.io/bookorbit/bookorbit:latest` (=v2.8.1, `sha256:cc1ecc94…`) | **817 MB** | **210 MiB** | 197 MiB | `pgvector/pgvector:pg18`, 445 MB, 157/90.8 MiB — **mandatory** |

ABS is the only single-process peer: 320 MB / 37 MiB against Grimmory's 829 MB / 366 MiB and BookOrbit's 1.26 GB / 367 MiB two-container stacks.
**[inference]** A Rust audiobook lane competes on the axis this fork already wins (`docs/.../comparison.mdx:93-94`: single binary yes / no / no).

## 1. Servers — audiobook features and client API

### 1.1 Audiobookshelf

**[observed]** Formats: whatever `ffprobe` accepts; direct play is a pure MIME-set intersection —
`includedAudioFiles.every(af => supportedMimeTypes.includes(af.mimeType))` (`server/models/Book.js:278-284`);
clients send e.g. `["audio/flac","audio/mpeg","audio/mp4"]`. Otherwise **transcode to HLS** via ffmpeg
(`playMethod` 0=DirectPlay 1=DirectStream 2=Transcode 3=Local, `managers/PlaybackSessionManager.js:321-351`;
`routers/HlsRouter.js`; `Stream.clientPlaylistUri = /hls/<id>/output.m3u8`). The runtime image installs exactly
`tzdata ffmpeg tini` — **ffmpeg/ffprobe are hard deps**. Multi-file books: `audioFiles[]` → `tracks[]` with
`startOffset`. Chapters: ffprobe-extracted per file (`utils/prober.js:139,278`), then
`AudioFileScanner.getBookChaptersFromAudioFiles` prefers the first file's embedded chapters and otherwise
synthesises one chapter per file (`scanner/AudioFileScanner.js:504-508`); editable via
`POST /api/items/{id}/chapters`, looked up via `GET /api/search/chapters` (Audnexus). Podcasts/RSS: full,
including **serving** feeds (`POST /api/feeds/item|collection|series/{id}/open`, public `GET /feed/{slug}`,
`server/Server.js:328-338`). Tools: `POST /api/tools/item/{id}/encode-m4b`, `/embed-metadata`.
**No OPDS at all** (`docs/.../comparison.mdx:44`).

**Auth [observed]**: `POST /login` → `user.accessToken` JWT (1 h default) + refresh cookie (30 d);
`POST /auth/refresh` (`server/Auth.js:320,329`; `auth/TokenManager.js:17-29`). Verified live on 2.36.0:
`Authorization: Bearer` **and** `?token=` both 200 on `/api/me`; unauthenticated 401. Admin API keys since
2.26.0 (`routers/ApiRouter.js:338-341`, `models/ApiKey.js`).

**[observed] Doc-vs-source divergences at v2.36.0** — matters if we ever impersonate the profile:

| Slate docs say | v2.36.0 `ApiRouter.js` actually has |
| --- | --- |
| `POST /api/me/sync-local-progress` | `POST /api/session/local` and `/api/session/local-all` (`:238-239`) |
| `AudioTrack.contentUrl = /s/item/<id>/<file>` | `/api/items/{id}/file/{ino}` (`objects/files/AudioTrack.js:32`, `models/Book.js:299`) |
| `/api/items/{id}/tone-object`, `POST …/tone-scan/{i}` | `/metadata-object` (`:121`), `GET /api/items/{id}/ffprobe/{fileid}` (`:123`) |
| — (undocumented) | `/api/items/{id}/download`, `/file/{fileid}/download`, `/ebook/{fileid}`, `/play/{episodeId}`, `/api/api-keys` |

The in-repo `docs/openapi.json` at v2.36.0 covers only **31 paths** (authors, emails, libraries, notifications,
podcasts, series) and contains **none** of the play/session/progress/bookmark routes. **[inference]** The
contract is `server/routers/ApiRouter.js` plus the Slate pages; the OpenAPI file is not usable as a spec.

### 1.2 ABS client-profile subset (~35 routes; all REST)

| Group | Routes | Key fields |
| --- | --- | --- |
| identity | `POST /login`, `POST /auth/refresh`, `POST /api/authorize`, `GET /api/me`, `GET /status`\|`/ping`\|`/healthcheck` | `user.accessToken`, `user.mediaProgress[]`, `user.bookmarks[]`, `serverSettings`, `userDefaultLibraryId` |
| libraries | `GET /api/libraries`, `/{id}`, `/{id}/items`, `/{id}/personalized`, `/{id}/series`, `/{id}/authors`, `/{id}/collections`, `/{id}/playlists`, `/{id}/filterdata`, `/{id}/search?q=`, `/{id}/recent-episodes` | `items` query: `limit,page,sort,desc,filter(base64),collapseseries,minified,include` |
| item detail | `GET /api/items/{id}?expanded=1&include=progress,rssfeed,authors`, `POST /api/items/batch/get` | `media.chapters[]{id,start,end,title}`; `media.audioFiles[]{index,ino,duration,codec,bitRate,channels,trackNumFromMeta,trackNumFromFilename,chapters[],metaTags,mimeType}`; `media.tracks[]{index,startOffset,duration,title,contentUrl,mimeType}` |
| bytes | `GET /api/items/{id}/cover?raw=1`, `/file/{fileid}` (Range), `/file/{fileid}/download`, `/download`, `/ebook/{fileid}?` | `contentUrl` → `/api/items/{id}/file/{ino}` |
| play | `POST /api/items/{id}/play`, `POST /api/items/{id}/play/{episodeId}` | req `{deviceInfo{clientName,clientVersion,deviceId,manufacturer,model,sdkVersion}, mediaPlayer, forceDirectPlay?, forceTranscode?, supportedMimeTypes[]}` → **PlaybackSession** `{id,userId,libraryId,libraryItemId,episodeId,mediaType,mediaMetadata,chapters[],displayTitle,displayAuthor,coverPath,duration,playMethod,mediaPlayer,deviceInfo,serverVersion,date,dayOfWeek,timeListening,startTime,currentTime,startedAt,updatedAt,audioTracks[]}` |
| session | `GET /api/session/{id}`, `POST /api/session/{id}/sync`, `POST /api/session/{id}/close`, `POST /api/session/local`, `POST /api/session/local-all` | sync/close body **`{currentTime, timeListened, duration}`**; `local`/`local-all` merge offline sessions |
| progress | `GET /api/me/progress/{itemId}[/{episodeId}]`, `PATCH` same, `PATCH /api/me/progress/batch/update`, `DELETE /api/me/progress/{id}`, `GET /api/me/items-in-progress` | **MediaProgress** `{id,libraryItemId,episodeId,duration,progress(0..1),currentTime,isFinished,hideFromContinueListening,lastUpdate,startedAt,finishedAt,ebookLocation,ebookProgress}`; PATCH is partial (`{"isFinished":true}`) |
| bookmarks | `POST`/`PATCH /api/me/item/{id}/bookmark`, `DELETE /api/me/item/{id}/bookmark/{time}`, `GET /api/me/bookmarks[/{itemId}]` | **AudioBookmark** `{libraryItemId,title,time(int s),createdAt}` |
| stats | `GET /api/me/listening-sessions`, `/api/me/listening-stats`, `/api/me/item/listening-sessions/{id}` | per-day/per-item totals |
| browse | `GET /api/series/{id}`, `/api/authors/{id}?include=items`, `/api/authors/{id}/image`, `/api/collections`, `/api/playlists`, `/api/search/{books,authors,chapters,covers,podcast}` | — |

**Socket-only, no REST equivalent [observed]** (`server/SocketAuthority.js`): cover-search streaming
(`search_covers` → `cover_search_result`/`_complete`/`_error`/`_cancelled`, `:197-198,342-413`); log tailing
(`set_log_listener`/`remove_log_listener`, `:201-210`); `cancel_scan` (`:191`); `message_all_users` →
`admin_message` (`:239-244`); the post-auth `init` payload (`:325`); `user_online`/`user_offline` presence
(`:224,312`); `pong` (`:249`).
**Push mirrors of REST state** (pollable instead): `user_item_progress_updated`
(`managers/PlaybackSessionManager.js:265,402`), `user_stream_update` (`:361,428`), `user_session_closed` (`:429`),
`item_{added,updated,removed}`, `library_*`, `author_*`, `series_*`, `collection_*`,
`task_{started,finished,progress}`, `track_{started,progress,finished}`, `metadata_embed_queue_update`,
`episode_download_{queued,started,finished}`, `episode_download_queue_cleared`, `episode_added`,
`rss_feed_open`/`_closed`, `share_open`/`_closed`, `backup_applied`, `notifications_updated`,
`stream_reset` (`routers/HlsRouter.js:87`).

### 1.3 Grimmory — **yes, its own audiobook client API** (not ABS-shaped)

Formats M4B/M4A/MP3/OPUS (`README.md:46`), port 6060. **[observed]** at v3.3.3:

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/api/v1/audiobooks/{bookId}/info` | `AudiobookInfo{bookId,bookFileId,title,author,narrator,durationMs,bitrate,codec,sampleRate,channels,totalSizeBytes,folderBased,chapters[],tracks[]}` (`controller/AudiobookReaderController.java:22,33`; `model/dto/response/AudiobookInfo.java:14-28`) |
| GET | `/api/v1/audiobooks/{bookId}/stream` | whole-book stream (`:47`) |
| GET | `/api/v1/audiobooks/{bookId}/track/{trackIndex}/stream` | per-track stream (`:64`) |
| GET | `/api/v1/audiobooks/{bookId}/cover` | embedded cover (`:82`) |
| GET | `/api/v1/media/book/{id}/audiobook-thumbnail`, `/audiobook-cover` | `controller/BookMediaController.java:60,69` |
| POST | `/api/v1/books/progress` | body carries `audiobookProgress` (`controller/BookController.java:289`) |
| — | `/api/v1/app/**` (8 controllers) | first-party **mobile-app** API incl. audiobook progress (`app/controller/App{Author,Book,Filter,Library,Notebook,Series,Shelf,User}Controller.java`; `app/service/AppBookService.java:178-224`) |

DTOs worth borrowing conceptually: `AudiobookChapter{index,title,startTimeMs,endTimeMs,durationMs}`,
`AudiobookTrack{index,fileName,title,durationMs,fileSizeBytes,cumulativeStartMs}`,
`AudiobookProgress{positionMs,trackIndex,trackPositionMs,percentage}`
(`model/dto/progress/AudiobookProgress.java:13-19`), and one bookmark table spanning all media —
`BookMarkEntity{cfi,positionMs,trackIndex,pageNumber,title,color,notes,priority,version}` (`:39-66`).
**Grimmory's OPDS serves audio acquisition links**: `audio/mpeg` / `audio/opus` / `audio/mp4`
(`service/opds/OpdsFeedService.java:713-718`). No first-party app repo exists yet; the `/app` tree implies one.

### 1.4 BookOrbit — own web player plus a 5-route private REST; **no audiobook catalog protocol**

Formats M4B/MP3/M4A/OPUS/OGG/FLAC in a built-in web reader (`README.md:46`). **[observed]** the audiobook
reader (`client/src/features/reader/audiobook/`) calls exactly: `GET /api/v1/books/{id}`,
`GET /api/v1/books/{id}/cover`, `GET /api/v1/books/files/{fileId}/serve`,
`GET|PATCH /api/v1/books/{id}/audio-progress` (`server/src/modules/book/book.controller.ts:497,502`),
`…/books/{id}/bookmarks[/{id}]`. Chapters exist server-side as `audioMetadata.chapters: AudiobookChapter[]`
(`modules/book/dto/book-detail.dto.ts:29`), editable via `update-book-metadata.dto.ts:18`.
**Negatives, evidenced by what was read, not by absent hits**: its OPDS mime mapper enumerates
epub/pdf/mobi/azw3/fb2/cbz/cbr and falls through to `application/octet-stream` — **no audio mime, so OPDS is
ebook/comic-only** (`server/src/modules/opds/opds-xml.helpers.ts:34-53`); and its ABS code is a **migration
adapter** (`modules/migration/adapters/audiobookshelf/audiobookshelf-api.connector.ts`,
`docs/AUDIOBOOKSHELF_MIGRATION.md`) — BookOrbit is an ABS *client for one-time import*, never an ABS-compatible
server. Verdict: **own web player only**; no ABS compatibility, no audio OPDS, no first-party app (it ships a
KOReader plugin instead).

## 2. socket.io in ABS

| Fact | Value | Evidence |
| --- | --- | --- |
| Server / client lib | `socket.io ^4.5.4` / `socket.io-client ^4.1.3` | `package.json:61`; app `package.json` |
| Socket.IO protocol | **revision 5** | socketio/socket.io-protocol `Readme.md:589` ("used in Socket.IO v3 and above") |
| Engine.IO protocol | **revision 4** (`EIO=4`) | ibid. `:591`; socketio/engine.io-protocol `README.md:359+` |
| Path | `/socket.io`, plus a second server at `${RouterBasePath}/socket.io` for sub-path installs | `SocketAuthority.js:164-173` |
| CORS | `origin:'*'`, `methods:['GET','POST']` | `SocketAuthority.js:157-161` |
| Transports | app forces `transports:['websocket'], upgrade:false` → **no long-polling fallback** | app `plugins/server.js:36-42` |
| Auth handshake | client emits `auth` with the JWT; server replies `init` or `auth_failed` | `plugins/server.js:66`; `SocketAuthority.js:188,278-325` |

**[observed]** The official app subscribes to only **7 server events** plus 3 manager events: `connect`,
`disconnect`, `init`, `auth_failed`, `user_updated`, `user_item_progress_updated`, `playlist_added`,
`reconnect_attempt`, `reconnect_error`, `reconnect_failed` (`plugins/server.js:52-61`). The server emits ~40
(§1.2); the app ignores the rest.

**REST-only server: what breaks.** **[observed]** Playback and progress sync never touch the socket — they are
native (`android/.../server/ApiHandler.kt`, `media/MediaProgressSyncer.kt` contain no socket code), so login,
browse, play, sync and offline merge keep working. **[observed]** but the player's **bookmark button is gated on
socket connectivity**: `v-if="… && socketConnected"` (`components/app/AudioPlayer.vue:68`, state
`store/index.js:15,176`) — bookmarks become unreachable in the official app without socket.io.
**[inference]** Everything else is degraded-not-fatal: no cross-device live position, no live user/permission
updates, no playlist-added push, no scan/task/podcast-download progress, no toasts.

## 3. Clients

| Client | Platform | Licence | Pin | ABS routes | Position sync | Bookmarks | Chapters | Offline | Non-ABS |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Official ABS app** | Android + iOS | GPL-3.0 | `v0.14.0-beta` `12025ab5` | 23 paths, `ApiHandler.kt:427-854` | **15 s** timer, **60 s** metered; `{timeListened,duration,currentTime}` → `POST /api/session/{id}/sync` | read/write, **UI socket-gated** | session `chapters` + item `media.chapters` | yes: `/api/session/local`, `/local-all`, `PATCH /api/me/progress/{id}` | none |
| **Lissen** | Android | MIT | `1.11.22-release` `f30bf9be` | 26 Retrofit decls in `channel/audiobookshelf/**` | **45 s**, **5 s** near boundary; `timeListened = unsyncedMs/1000` | read/write | `book.chapters` (`PlaybackService.kt:234`) | ExoPlayer cache + Room | **none** (`channel/` = `audiobookshelf/` + `common/` only) |
| **ShelfPlayer** | iOS/tvOS | — | `55f46edb` | **n/a — source removed** | — | — | — | — | — |
| **Plappa** | iOS/iPadOS | OSS (Swift) | `b6b811c6` (2026-08-08) | not captured (GitHub API rate limit hit) | not captured | — | — | yes (per plappa.me) | **Jellyfin** + ABS |
| **Prologue** | iOS | closed | — | — | — | — | — | — | Plex-first, ABS added |

**[observed] ShelfPlayer is gone as an OSS reference.** At `55f46edb` the repo contains a single `README.md`
titled "Goodbye": the author sold ShelfPlayer (then v3.3.0) to the Italian company *Space Mushrooms*, "handed
over full control of the IP and development", and contributions "were excluded and replaced in the sold source
code". No Swift source left to cite.

**[observed] Sync citations.** Official app: `Timer("ListeningTimer", false).schedule(15000L, 15000L)`
(`media/MediaProgressSyncer.kt:79`), `METERED_CONNECTION_SYNC_INTERVAL = 60000` (`:33`), payload fields
`timeListened: Long /*seconds*/, duration: Double, currentTime: Double` (`:17-19`), posted at
`server/ApiHandler.kt:646`; offline merge `:677` (`/api/session/local`) and `:775` (`/local-all`); progress
fallback `:688,696,854`. Lissen: `SYNC_INTERVAL_LONG = 45_000L`, `SYNC_INTERVAL_SHORT = 5_000L`,
`chooseSyncInterval(durationMs, positionMs)` (`playback/service/PlaybackSynchronizationService.kt:221-235`),
payload `:111,155-163`; login/refresh/authorize `common/client/AudiobookshelfApiClient.kt:164,170,59`; bookmark
POST/DELETE `:65,71`. **Lissen has no socket.io dependency at all** — the existence proof that a REST-only
ABS-compatible server is usable by a real client.

**[observed] Official third-party catalogue** (audiobookshelf.org/docs/documentation/community/community-apps)
lists 11 apps — AbsorbA, AudioBooth, Auribook, Harmshelf, plappa, SoundLeaf, Still, Storii, Verbara, Voca, yaabsa
— warning that "many use endpoints that are not recommended anymore". Neither Lissen nor ShelfPlayer is listed.

## 4. State mapping: ABS → this fork's unified head

Fork side **[observed]**: `reading_sessions{session_date, start/end_locator: ReadiumLocator, start/end_page,
start/end_percentage, koreader_progress, elapsed_seconds, readthrough_number, status, notes, kobo_state,
device_ids, media_id, user_id, created_at, updated_at}` (`crates/models/src/entity/reading_session.rs:25-82`);
`ReadiumLocator{chapter_title, href, title, locations{fragments, progression, position, total_progression,
css_selector, partial_cfi}, text, kobo_span, type}` (`crates/models/src/shared/readium.rs:122-176`);
`media_annotations{id, locator (**required**), annotation_text, media_id, user_id, …}`;
`bookmarks{id, preview_content, locator, page, media_id, user_id, created_at}`;
`devices{id, user_id, name, kind, transform_profile, library_scope, last_seen_at, last_sync_at, revoked_at}`.
Precision tiers are `range|resource|page|publication|unknown` (`docs/.../unified-reading-state.mdx:98-112`).

| ABS field | Unified head target | Fit |
| --- | --- | --- |
| `mediaProgress.progress` (0..1) | `end_percentage` / `locations.total_progression` | ✅ tier `publication` |
| `mediaProgress.currentTime` (float **s**) | — | ❌ **gap 1: no time-domain position.** Nearest is `locations.position: Option<i32>`, a page ordinal — explicitly not interchangeable (`unified-reading-state.mdx:112`) |
| `mediaProgress.duration` | — | ❌ no media-duration field on the head |
| `isFinished` / `finishedAt` | `status: ReadingStatus` + `updated_at` | ✅ |
| `lastUpdate` / `startedAt` | `updated_at` / `created_at`, `timestamp_kind=device` | ✅ |
| `hideFromContinueListening` | — | ❌ no flag (no Komga/Kobo analogue either) |
| `episodeId` | — | ❌ **gap 4: no per-episode sub-identity** (needs `edition_id`-style scoping) |
| `playbackSession.currentTime` / `startTime` / `duration` | — | ❌ same gap 1 |
| `playbackSession.timeListening` (float s, cumulative) | `elapsed_seconds: Option<i64>` | ⚠️ right semantics (time spent, not position) but **int vs float**, and ABS syncs `timeListened` *deltas* to accumulate |
| `deviceInfo{deviceId,clientName,clientVersion,manufacturer,model,sdkVersion,osName…}` | `devices{id,name,kind}` + `reading_sessions.device_ids` | ✅ mostly; `kind` needs an audio-player variant, `clientName/Version` have no column |
| `playbackSession.playMethod` (0..3) | — | ❌ no delivery-method provenance (would live in `raw_native_payload`) |
| `playbackSession.chapters[]` | — | ❌ **gap 3: chapters unmodelled** anywhere in `stump_media` |
| `audioTracks[].index` / `startOffset` | — | ❌ **gap 2: no track index / per-track offset.** Grimmory already has `trackIndex`, `trackPositionMs`, `cumulativeStartMs` |
| `bookmark{time,title,createdAt}` | `bookmarks{page?,locator?,preview_content}` | ⚠️ storable only as `preview_content=title` + a synthetic locator; **`time` has no column** — Grimmory's `BookMarkEntity.positionMs` is the shape we lack |
| audio annotations | `media_annotations.locator` is **required** | ❌ an audio anchor has no `href`, and `unified-reading-state.mdx:52` forbids inventing one → locator must become optional or gain an audio variant |

**[inference] Minimum schema delta**: nullable `position_ms: i64`, `track_index: i32`, `duration_ms` on the
position/session head; `bookmarks.position_ms`; a `time` precision tier ranked beside `page`; an audio locator
variant so annotations stop being EPUB-only. Because `crates/migrations/src/lib.rs` is append-only with **one
SQLite column per `alter_table`** (`.omp/RULES.md`), that is ~5 migration files, not one.

## 5. Formats and tooling

| Container | Chapter mechanism | Spec | Who reads it |
| --- | --- | --- | --- |
| MP4/M4A/**M4B** | (a) Nero `chpl` list in `moov/udta`; (b) QuickTime chapter **track** (text track linked via `tref`/`chap`) — **two different things** | ISO/IEC 14496-12 + QuickTime File Format; `chpl` is a de-facto Nero atom | Apple Books/iTunes read the chapter track; most tools write both |
| MP3 / ID3v2 | `CHAP` + `CTOC` frames (a `CHAP` may embed `TIT2`, `APIC`) | ID3v2 Chapter Frame Addendum 1.0 (id3.org — **served 503 during this study**; mirrored in mutagen's frame docs) | ffmpeg, mutagen, `tone` |
| Ogg/Opus, FLAC (Vorbis comments) | `CHAPTER000=hh:mm:ss.sss` + `CHAPTER000NAME=` | https://wiki.xiph.org/Chapter_Extension | ffmpeg, mpv |
| FLAC | `CUESHEET` metadata block (CD-oriented track/index points) | https://xiph.org/flac/format.html#metadata_block_cuesheet | flac, metaflac |
| Sidecar | `.cue`; ffmpeg metadata (`;FFMETADATA1` + `[CHAPTER]` `TIMEBASE/START/END/title`) | https://ffmpeg.org/ffmpeg-formats.html#Metadata | ffmpeg — **ABS writes exactly this** (`utils/ffmpegHelpers.js:240-278`, `-map_chapters 1` at `:316`) |
| Folder of MP3/M4A | no chapters; one per file | — | ABS: `trackNumFromMeta` else `trackNumFromFilename`, chapters synthesised per file (`AudioFileScanner.js:504-508`) |

| Crate | Ver | Reads | Writes | Chapters? |
| --- | --- | --- | --- | --- |
| `symphonia` | 0.6.1 | mp3, aac/alac (isomp4), flac, vorbis, ogg, wav, mkv; packets, duration, `Visual` covers | no | `Chapter`/`ChapterGroup` exist (`symphonia-core/src/meta.rs:664-700`) and `FormatReader::chapters()` (`symphonia-core/src/formats/mod.rs:567-572`) is implemented by **ogg, mkv, mp3, flac** — but **not by `symphonia-format-isomp4`** (no `fn chapters`; its atom set is `alac avcc co64 … udta wave`, **no `chpl`**). **M4B chapters are invisible to symphonia today.** |
| `mp4ameta` | 0.13.0 | M4A/M4B tags, covers, audio info | **yes** | **both**: `chapter_list()/chapter_list_mut()` (`chpl`, with `ChplTimescale`) **and** `chapter_track()/chapter_track_mut()`, then `write_to_path()` (`src/lib.rs:43-80`) |
| `id3` | 1.17.1 | ID3v2 | yes | **yes**: `id3::frame::Chapter` + `TableOfContents` |
| `lofty` | 0.25.1 | mp3/mp4/flac/opus tags, covers, duration | yes | **ID3v2 only**: `ChapterFrame`, `ChapterTableOfContentsFrame` (CHANGELOG) |
| `metaflac` | 0.2.8 | FLAC blocks incl. CUESHEET | yes | via CUESHEET |
| `mp4` | 0.14.0 | ISO-BMFF read + write (`Mp4Writer`) | yes | no; `MediaConfig` = `Avc/Aac/Ttxt/Vp9` only → **no MP3-in-MP4** (`src/track.rs:28-60`) |
| `ebur128` | 0.1.10 | — | — | **pure-Rust** port of libebur128; M/S/I modes, LRA, true peak; passes EBU TECH 3341/3342 (README) |
| `m3u8-rs` / `ffmpeg-sidecar` | 6.0.1 / 2.5.2 | HLS playlists / shell-out driver with progress | yes / n/a | n/a |
| `fdk-aac` | 0.8.0 | AAC encode | — | crate is MIT but **binds libfdk-aac**, whose Fraunhofer licence is not OSI-approved — a redistribution problem for an MIT single binary |

**"cbzit for audio"** — same `Tool` plan/apply contract `crates/tools` already uses for `cbzit`:

| Capability | Verdict | Crate / gap |
| --- | --- | --- |
| Probe duration, codec, bitrate, channels | **pure Rust today** | `symphonia` (+`lofty` tags) |
| Order a folder into one logical book | **pure Rust today** | tag track number → filename fallback (mirror ABS's precedence) |
| Synthesise chapters from per-file durations | **pure Rust today** | arithmetic over probed durations |
| Read existing M4B chapters | **pure Rust today** | `mp4ameta` (**not** symphonia) |
| Write/fix `chpl` + chapter track in an M4B | **pure Rust today** | `mp4ameta` `*_mut` + `write_to_path` |
| Read/write MP3 `CHAP`/`CTOC`; embed cover art | **pure Rust today** | `id3`/`lofty`; `mp4ameta`/`lofty` |
| Measure loudness (R128 / true peak) | **pure Rust today** | `ebur128` over `symphonia`-decoded frames |
| Apply loudness normalisation | **needs ffmpeg** | no pure-Rust AAC encoder; MP3 global-gain only |
| MP3 folder → single **M4B** | **needs ffmpeg** | needs AAC/ALAC encode (no pure-Rust encoder; `fdk-aac` licence blocker) and `mp4` cannot mux MP3-in-MP4 |
| Remux M4A → M4B (extension + atom edit) | **pure Rust today** | `mp4ameta` (same container) |
| Split an M4B into per-chapter files | **pure Rust with work** | `symphonia` demux + `mp4` `Mp4Writer` AAC passthrough; sample-accurate cuts are the work |

Reference CLIs to shell out to (this fork already shells out to calibre): `ffmpeg`/`ffprobe`, `tone` (Apache-2.0,
C#, sandreas — ABS wraps it as `advplyr/node-tone`), `m4b-tool` (MIT, PHP, wraps ffmpeg+mp4v2), `AtomicParsley`
(GPL-2.0), `mp4v2`/`mp4chaps` (MPL-1.1). **[inference]** Everything but *encoding* is pure Rust; ffmpeg is
needed only for transcode/normalise/assembly.

## 6. Verdict — recommended first slice

**Do not start with an ABS-compatible profile.** (1) The client contract is undocumented in the official
OpenAPI (31 of 153 routes, none of play/session/progress), so the profile must be derived from GPL-3.0 source —
the same position we are in with Grimmory/BookOrbit. (2) It is ~35 routes **plus** a socket.io v5/EIO-4 server,
and the official app hides its bookmark UI without socket (`AudioPlayer.vue:68`). (3) The head cannot represent
a single ABS position today: `currentTime`, `trackIndex`, `timeListening` and `chapters` have no column.

**First slice: audio in `stump_media` + time-domain positions in the head, served on lanes we already own.**

1. **Audio probe in `crates/media`** — one new module, no new crate: `symphonia` for duration/codec/channels,
   `mp4ameta` for M4B `chpl` + chapter track, `id3`/`lofty` for MP3 `CHAP`/`CTOC`, folder ordering with ABS's
   track-number precedence, chapter synthesis from per-file durations. No ffmpeg, no transcode.
2. **Schema** — append-only, one column per `alter_table`: `position_ms`, `track_index`, `duration_ms` on the
   position/session head; `bookmarks.position_ms`; a `time` precision tier. Grimmory's
   `AudiobookProgress{positionMs,trackIndex,trackPositionMs,percentage}` and its single cross-media
   `BookMarkEntity` are the proven shape.
3. **Serve it on existing lanes** — OPDS 1.2/2.0 audio acquisition links (`audio/mpeg`, `audio/mp4`,
   `audio/opus`, `audio/flac`) exactly as Grimmory does (`OpdsFeedService.java:713-718`), Range-served bytes on
   the existing media route, chapters + tracks on the native GraphQL API. This is where we already beat ABS,
   which has **no OPDS at all**.
4. **Only then** choose between an ABS profile crate and a native audio player.

| Item | Estimate |
| --- | --- |
| New crates | **0** (audio module inside `stump_media`; assembly tools beside `cbzit` in `crates/tools`) |
| New dependencies | 4 (`symphonia`, `mp4ameta`, `id3` or `lofty`, `ebur128` — all pure Rust, no ffmpeg) |
| Migrations | **5** append-only files (one SQLite column each) |
| New/changed routes | **6–8**: OPDS 1.2 + 2.0 audio acquisition entries, a Range audio-file route, an audio manifest/chapters read, position + bookmark writes carrying `position_ms`/`track_index` |
| GraphQL surface | ~4 types (`AudioTrack`, `AudioChapter`, `AudioPosition`) + 2 mutations |
| Tests | **~14–18**: 5–6 media (M4B `chpl`, M4B chapter track, MP3 `CHAP`/`CTOC`, Vorbis `CHAPTER00`, folder ordering tag-vs-filename, duration without full decode); 4–5 state (ms↔percentage round-trip, `time` vs `page` tier ordering, track-index-aware head selection, bookmark ms); 3 OPDS (audio mime per format, Range 206, feed shape); 2 negative (chapterless file, mixed-codec folder) |
| Harness | 1 new Hurl spec (`specs/audio-opds.hurl`) beside the existing replay targets |
| A full ABS profile, for contrast | **1 crate + ~35 routes + a socket.io v5/EIO-4 server + ~30 DTOs** — a Komga-sized programme, not a first slice |

---

**Research gaps.** Two facts were not established rather than guessed: Plappa's route list (the GitHub API rate
limit was exhausted mid-study) and the ID3v2 chapter addendum's canonical URL (id3.org returned 503).