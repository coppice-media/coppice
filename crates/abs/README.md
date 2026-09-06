# stump_abs

## Purpose

`stump_abs` is the Audiobookshelf compatibility **profile**: the root-mounted
`/ping`, `/status`, `/login`, `/auth/refresh` and `/api/...` routes, DTOs,
mapper, `abs_ids`/`abs_sessions` side tables and HS256 tokens that Lissen
1.11.22 calls. It reaches persistence through `AbsBackend` (a
`DatabaseConnection` accessor plus async audio/cover/track/session methods) and
does **not** own authentication middleware, route mounting or the concrete
backend (`apps/server/src/routers/abs_backend.rs`), podcasts, playlists,
collections, the sessions list, or the socket.io lane.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| Audiobookshelf (`abs-ref` container) | `ghcr.io/advplyr/audiobookshelf:2.36.0`, port 13450 (`komga-compat/abs/`) | 35 JSON captures under `komga-compat/abs/capture/`; `ABS_VERSION` is what `/status` reports |
| Lissen | 1.11.22-release `f30bf9be` | The client contract: `channel/audiobookshelf/common/client/AudiobookshelfApiClient.kt` (26 declarations, 4 podcast-only) |
| Replay | `komga-compat/specs/abs.hurl` (43 chained entries), `make replay-abs`, `make replay-abs-diff` (`tooling/abs_keydiff.sh`) | Behaviour and key-set parity against `abs-ref` |

Licensing boundary: Audiobookshelf is GPL-3.0; nothing here derives from its
source. Shapes come from captured JSON and Lissen's Retrofit declarations.

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| `AbsBackend::serve_track` takes the authenticating `device_id`, resolved from the `AuthContext` the server's middleware already put in the request extensions | The audio transform preset that decides whether a track is transcoded lives on the device, so a backend without it would serve every ABS client the stored encoding while the native route honoured the preset — the same book would depend on which protocol the player speaks | `src/routes/mod.rs` `serve_track`, `src/routes/items.rs::file`; `apps/server/src/routers/abs_backend.rs` (delegates to `routers::audio_transform`); `src/test_support.rs` records `(media_id, index, range, device_id)` |
| Routes mount at the server **root**, gated by the server `abs` feature (inside `headless`) and `STUMP_ENABLE_ABS` (default `false`) | Audiobookshelf clients build absolute paths from a bare host with no base path; Lissen concatenates host + declared path | `apps/server/src/routers/abs_backend.rs`, `apps/server/Cargo.toml` `abs`; `docs/.../abs-compat.mdx` |
| ABS ids are the Stump UUIDs; `abs_ids` maps allocated ids (authors) to Stump names | UUIDs are valid ABS ids, so items, libraries, users and sessions pass through; only authors have no Stump row to point at | `src/ids.rs`; `crates/migrations/src/m20260935_000000_add_abs_compat.rs` |
| A Stump series holding one audio book is that book's own folder, not an ABS series (`seriesName: ""`, `hideSingleBookSeries: true`) | Matches the `abs-ref` capture for the same fixture; ABS never lists a one-book folder as a series | `src/mapper.rs`; `capture/item.json` |
| `POST /api/authorize` and `GET /api/me` carry `token` alone; `POST /login` alone adds `isOldToken: true` | Verbatim from `capture/authorize.json`, `me.json`, `login.json` | `src/routes/identity.rs`, `src/routes/me.rs` |
| Every time value crosses the wire in **seconds**; Stump stores milliseconds | ABS `currentTime`/`duration`/`chapters[].start` are fractional seconds, `media_audio*` columns are `_ms`; converted exactly once in `mapper` | `src/mapper.rs` `ms_to_secs`/`secs_to_ms`; test `seconds_round_trip_without_losing_a_half_second` |
| `audioFiles[].chapters[]` are clipped to the file and rebased to its timeline; `media.chapters[]` stay publication-relative | `abs-ref` reads per-file chapters from each file's embedded marks | `src/mapper.rs` `file_chapters`; test `file_chapters_are_clipped_and_rebased_per_track` |
| `metaTags` emits only tags Stump read; `tagAlbum` is the book title | `abs-ref` omits absent tags; the probe reads a folder book's title from the shared `album` tag | `src/mapper.rs` `meta_tags`; `crates/media/src/media/format/audio.rs` `publication_title` |
| The play session reuses the item's mapped `audioFiles` for `audioTracks`; `deviceInfo` echoes the four request descriptors and emits them `null` on read-back | Remapping with no metadata dropped every tag from the session; `abs-ref` always emits `deviceName`/`manufacturer`/`model`/`sdkVersion` | `src/mapper.rs` `session_dto`/`tracks_of`; `src/dto.rs` `DeviceInfoDto` |
| Cover = Stump's media thumbnail (page 1 of an audiobook is its cover art) | One cover pipeline for every profile; the audio processor serves embedded art or a `cover.*` sidecar as page 1 | `apps/server/src/routers/abs_backend.rs` `cover`; `crates/media/README.md` |
| `DeviceKind::Abs` under `DeviceProtocol::Api`, `SourceProtocol::Abs` for reading-state writes | An ABS client is an API-key surface like Kavita; the unified head records which lane wrote | `crates/models/src/shared/enums.rs`; `crates/devices/src/credential.rs` |

## Layout

| Path | Owns |
| --- | --- |
| `src/routes/{identity,libraries,items,me,session,query}.rs` | Handlers per route family; `query.rs` is the visibility funnel (`media_for_user`, `context`) |
| `src/mapper.rs` | Every Stump → ABS shape (`item`, `audio_files`, `audio_tracks`, `chapters`, `file_chapters`, `session_dto`, progress, bookmarks) |
| `src/model.rs` | `AbsAudio`/`AbsAudioTrack`/`AbsAudioChapter`, progress and position types, `ItemShape` |
| `src/dto.rs` | Wire types (serde, `camelCase`) |
| `src/auth.rs`, `src/ids.rs`, `src/sessions.rs` | HS256 tokens, `abs_ids`, `abs_sessions` |
| `src/test_support.rs`, `src/routes/tests.rs` | In-memory backend and route tests |

## How to verify

```sh
cargo test -p stump_abs
cargo check -p stump_server --no-default-features --features headless,liseur-sync
# Live, against a fixture instance with STUMP_ENABLE_ABS=true and a scanned audiobook library:
cd ../komga-compat && make replay-abs BASE_URL=http://127.0.0.1:25600 USERNAME=… PASSWORD=… MITM_KEEP_HOST_HEADER=true
make replay-abs-diff BASE_URL=… USERNAME=… PASSWORD=… ABS_REF_BASE_URL=http://127.0.0.1:13450 ABS_REF_USERNAME=root ABS_REF_PASSWORD=…
```

## Deep docs

- `docs/content/docs/developer/abs-compat.mdx` (route matrix, deviations, unit table).
- `docs/content/docs/guides/fundamentals/audiobooks.mdx` (what the scanner stores).
- `docs/audiobook-study.md` (the peer/client study that chose the Lissen-first profile).
