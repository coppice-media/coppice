# stump_kobo

## Purpose

`stump_kobo` is the Kobo store-API compatibility seam: `ProviderHost`, the
`KoboBackend` trait, the route tree under `/kobo/{api_key}/v1/*` plus
`DELETE /api/v2/kobo/sync-sessions`, the opaque `SyncToken`, and the
database-backed `KoboSync`/`SyncPage` pagination algorithm. It deliberately does
**not** own authentication (API-key path middleware and the `AccessKoboSync`
guard live in `apps/server`), the Kobo wire DTOs and `ReadingState` ↔ reading
session projection (`core/src/kobo/{sync_types,entity,mod}.rs`), media
serving/thumbnails, or KEPUB conversion (`crates/kepub` is conversion-only; the
lazy cache is `apps/server/src/routers/kobo_backend/{kepub,kepub_cache}.rs`).
There is no store proxy: `reqwest` is a dependency only for its header error
types (`src/sync_token.rs:14`).

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| Komga `KoboProxy.kt` | [master `#L148-L341`](https://github.com/gotson/komga/blob/master/komga/src/main/kotlin/org/gotson/komga/infrastructure/kobo/KoboProxy.kt#L148-L341) | `NATIVE_KOBO_RESOURCES_JSON` initialization map and `x-kobo-apitoken: e30=` (`core/src/kobo/mod.rs:64-67`; `apps/server/src/routers/kobo_backend/router.rs:177-180`) |
| Calibre-Web `cps/kobo.py` | `a97826402f1b39c45b7ea8d906efddc9f1750934` | Route inventory, `ReadingStates[0]` state wire shape, sync-item vocabulary (`docs/content/docs/developer/kobo-sync-capabilities.mdx:8-70`) |
| Komga Kobo ReadingState/KEPUB | `656001eb03bf8b54ca909f3e74fe2ec1b95dac48` | Second ReadingState precedent (`kobo-sync-capabilities.mdx` §6-7) |
| kepubify | `9546034bc023891af5ce30709de6ae2dcf264628` | KEPUB byte parity target; see `crates/kepub/README.md` |
| Liseur `KoboClient.kt` | [`31f8182d`](https://github.com/chmouel/liseur/commit/31f8182d524e3536cf9020594185e709a033094f) | Only pinned software client: `library/sync`, `library/{id}/state` GET/PUT (`docs/content/docs/developer/liseur-providers.mdx:83-103`) |
| Kobo firmware format support | [Kobo help](https://help.kobo.com/hc/en-us/articles/360017763713) | JPEG/PNG/GIF/BMP/TIFF only — no WebP in CBZ/KEPUB (`clients.mdx:90`) |

Client-verification status (`docs/content/docs/developer/client-verification.mdx:33`):
**Harness only** — `initialization` 200, `library/sync`, KEPUB delivery with
`Range`, `ReadingState` round-trip. **No physical Kobo has been tested.** The
harness in this case is the fixture curl probe in
`kobo-sync-capabilities.mdx` (Fixture probe section); `/home/al/Code/komga-compat`
has no Kobo Hurl spec or replay target.

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Full Stump API key in the path (`/kobo/{api_key}`), `AccessKoboSync` required; `DownloadFile` for file bytes | Stock firmware cannot send Stump credentials; mirrors Calibre-Web `/kobo/<token>` | `apps/server/src/routers/kobo_backend.rs:64-75`; `router.rs:130-142`; `apps/server/src/utils/serve_media.rs:18-43` |
| Sync token = no-pad standard Base64 of tagged JSON `IncompleteV1{sync_id,next_offset}` / `CompletedV1{sync_id}`; always returned in `x-kobo-synctoken`, `x-kobo-sync: continue` while pages remain | Opaque to the device; lets a session resume at an offset | `src/sync_token.rs:18-37,56-96`; `router.rs:66-89` |
| Missing/invalid token → full sync; `CompletedV1` → incremental (created/modified media ∪ changed reading sessions since prior session); `IncompleteV1` → resume | Stateless device, stateful server session (`kobo_sync_sessions`) | `src/sync.rs:77-139,151-208` |
| Sync scope is a caller-supplied extension list (`KoboSync::next_page(.., extensions)`): the server passes `["epub"]` unless `STUMP_TRANSFORM_ENABLED`, then `epub` + CBZ/ZIP/CBR/RAR/PDF; non-EPUB entitlements advertise `Format::KEPUB` because the host serves them as a transformed fixed-layout KEPUB, with `Size` = cached transformed size when present, else the source size | Off → byte-identical EPUB-only sync; on → comics reach stock Kobos without WebP | `src/sync.rs` (`begin_new_sync`, `sync_items`, `download_format`); test `test_only_includes_epubs`; `apps/server/src/routers/kobo_backend/comic_transform.rs` (`sync_extensions`, `advertise_cached_sizes`); `docs/content/docs/developer/comics-transform.mdx` |
| Emitted items: `NewEntitlement`, `ChangedProductMetadata`, `ChangedReadingState`; `ChangedEntitlement` declared but never constructed | Nothing in Stump maps to an entitlement change | `src/sync.rs:249-338`; `core/src/kobo/sync_types.rs:24-30` |
| Stale sessions for the same device are pruned; the acknowledged session is kept | Bounded table growth without losing the resume point | `src/sync.rs:180-208`; test `test_sync_pruning` |
| Page cap `ITEMS_PER_PAGE = 100` in the adapter | Matches Kobo's observed page tolerance | `router.rs:66-68` |
| `GET/PUT /v1/library/{book_id}/state` are real: PUT parses `ReadingStates[0]`, stores raw JSON in `reading_sessions.kobo_state`, promotes `Type=KoboSpan` to `kobo_span`; GET reconstructs `ReadyToRead|Reading|Finished` | Progress round-trip for stock devices (older docs claiming a `{}` stub are stale) | `src/lib.rs:147-150`; `router.rs:274-423`; `core/src/kobo/entity.rs:322-393,499-563`; migration `m20260905_000000_add_kobo_reading_state` |
| `/v1/auth/device` returns fresh UUIDs, nothing persisted | Compatibility data, not a Stump login | `router.rs:93-104` |
| `any /v1/{*path}` catch-all logs at debug and returns `{}` | Tags/shelves, archive, store proxy, device refresh are out of scope | `src/lib.rs:162`; `router.rs:104-127` |
| Initialization rewrites Stump-owned resources (`library_sync`, `reading_state`, `library_metadata`, tags, thumbnails) to `{host}/kobo/{api_key}/v1/...`; store URLs untouched | Device must reach Stump for those; store features stay official | `core/src/kobo/mod.rs:21-55`; `apps/server/src/routers/kobo_backend.rs:31-49` |
| Thumbnails are always JPEG, resized to the requested box | Kobo renders JPEG only | `router.rs:322-359` |
| Metadata: `media.id` reused for entitlement/revision/work/cover IDs; `EPUB3`, DRM `None`, language `en`, zero USD price; `KEPUB` when conversion is on | Kobo ignores `format: EPUB`; neutral defaults for fields Stump lacks | `core/src/kobo/entity.rs:191-303` |
| KEPUB served from the same `/file/epub` URL, lazily converted and cached by `<media-id>-<mtime>-<options>-d<level>`; `Range` preserved; comics (with `STUMP_TRANSFORM_ENABLED`) take the same URL through `comic_transform::serve_comic` (device `transform_profile` else `STUMP_TRANSFORM_KOBO_PROFILE`, coerced to KEPUB + JPEG), falling back to the original file on any transform failure | Kobo discovers the URL during sync and resumes large downloads | `apps/server/src/routers/kobo_backend/kepub.rs` `book_file`; `comic_transform.rs` |
| Switches: `ENABLE_KOBO_SYNC` (no `STUMP_` prefix; release `false`, debug `true`), `KOBO_KEPUB_CONVERSION`, `KOBO_KEPUB_PRECONVERT`, `KOBO_KEPUB_CACHE_MAX_AGE_DAYS`, `KOBO_KEPUB_DEFLATE_LEVEL`; Cargo feature `kobo = ["dep:stump_kobo", "dep:stump_kepub"]` | Runtime switches for behaviour; feature only drops the crates from `minimal` | `core/src/config/env_keys.rs:36-41`; `core/src/config/protocols.rs:20-45`; `apps/server/Cargo.toml`; `apps/server/src/routers/mod.rs:40-43` |

Routes (`src/lib.rs:135-176`):

| Method | Path (under `/kobo/{api_key}`) | Backend method |
| --- | --- | --- |
| GET | `/v1/initialization` | `initialization` |
| GET | `/v1/library/sync` | `library_sync` |
| GET / PUT | `/v1/library/{book_id}/state` | `book_state` / `update_book_state` |
| GET | `/v1/library/{book_id}/metadata` | `book_metadata` |
| GET | `/v1/books/{book_id}/thumbnail/{w}/{h}/{is_greyscale}/image.jpg` and `/{w}/{h}/{quality}/{is_greyscale}/image.jpg` | `book_thumbnail` |
| POST | `/v1/auth/device` | `auth_device` |
| GET | `/v1/books/{book_id}/file/epub` | `book_file` (default → `book_download`) |
| ANY | `/v1/{*path}` | `stubbed_route` |
| DELETE | `/api/v2/kobo/sync-sessions` (session auth, not key) | `delete_sync_sessions` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `ProviderHost`, `KoboBackend` (11 methods), path extractors, `kobo_router` / `session_router` |
| `src/sync.rs` | `KoboSync`: session lookup/prune, full vs incremental selection, `SyncPage` item classification; 8 async tests |
| `src/sync_token.rs` | `SyncToken` enum, Base64(JSON) encode/decode, header conversion errors |
| `apps/server/src/routers/kobo_backend.rs` | `mount()`: host injection + API-key + `authorize` layers; `KoboBackendImpl` |
| `apps/server/src/routers/kobo_backend/router.rs` | concrete handlers: initialization, sync response, state GET/PUT, metadata, thumbnail, download, catch-all |
| `apps/server/src/routers/kobo_backend/kepub.rs`, `kepub_cache.rs` | on-demand KEPUB conversion, cache key, range serving; bounded background warmer and eviction |
| `core/src/kobo/mod.rs`, `sync_types.rs`, `entity.rs` | resource map + rewriter; `SyncItem`/`ReadingState` DTOs; metadata/entitlement/state projections |

## How to verify

```text
cargo test -p stump_kobo --lib --tests             # 8 tests in sync.rs (first sync, pagination, incremental,
                                                   #   3 item representations, pruning, epub-only)
cargo test -p stump_server --test api_tests kobo   # 2 auth tests (API key on device route; session on DELETE)
cargo test -p stump_core --lib kobo                # entity/sync_types mapping + serialization tests
cargo check -p stump_server --no-default-features --features minimal   # feature-off (crate absent)
ENABLE_KOBO_SYNC=false                             # runtime-off: routers::mount skips the tree
```

Live probe: the curl/jq sequence in
`docs/content/docs/developer/kobo-sync-capabilities.mdx` (Fixture probe) —
initialization resource rewrite → first `library/sync` + token → state PUT →
state GET → incremental sync asserting `ChangedReadingState`. Base
`http://127.0.0.1:25600`, key from the fixture's generated `ENDPOINTS.md`.

## Deep docs

- `docs/content/docs/developer/kobo-sync-capabilities.mdx` — protocol surface vs Calibre-Web/Komga, decision matrix, fixture probe, KEPUB research.
- `docs/content/docs/developer/kobo-device-database.mdx` — `KoboReader.sqlite` ingestion study (design only; nothing implemented).
- `docs/content/docs/developer/standards.mdx` (Kobo sync) — what we implement / deliberately do not.
- `docs/content/docs/developer/provider-status.mdx` (Kobo section) — status table and user setup.
- `docs/content/docs/developer/unified-reading-state.mdx` (Kobo `ReadingState`) — projection precision and losses.
- `docs/content/docs/developer/sync-platforms.mdx` and `clients.mdx` (Kobo) — **partly stale**: they still describe `reading_state` as a `{}` stub and omit `ChangedReadingState`; the code above is authoritative.
- `crates/kepub/README.md` — kepubify parity, `KOBO_KEPUB_*` keys, cache, measured speed.
