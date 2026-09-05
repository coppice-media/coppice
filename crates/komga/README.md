# stump_komga

## Purpose

`stump_komga` is the Komga compatibility **profile** — the REST/SSE subset that
Komelia, Mihon, Liseur, Komf and Grimmory actually call — not full Komga
parity. It owns the Komga DTOs (`book`, `series`, `library`, `collection`,
`read_list`, `search`, `readium`, `sse`, `user`, `settings`, `announcements`,
`common` Page/Sort/Author types), `APIError` mapping, the Spring-style `Page<T>`
pagination, ETag/cache helpers, every `/api/v1`, `/api/v2`, `/sse/v1` route
handler in `src/routes/`, the Grimmory `/komga` shim routes, and the
`is_komga_path` ownership predicate. It reaches persistence through
`KomgaBackend` (narrow `DatabaseConnection` accessor + async file/image/EPUB/job
methods). It deliberately does **not** own identity/session/settings/announcement
routes (`apps/server/src/routers/komga/{identity,settings}.rs`), auth middleware,
Basic-auth path scope, remember-me cookies, or route mounting; nor does it add
Komga persistence or a second identity store.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| Komga oracle | 1.26.3, `4cabeb8abd05ddfff473a8f850a3c33c8f9e9aa1`; [OpenAPI](https://raw.githubusercontent.com/gotson/komga/1.26.3/komga/docs/openapi.json) | Identity/session paths, DTO field names (`/home/al/Code/komga-compat/config/pins.json`) |
| komga-client (Kotlin) | 0.11.0 [`74412a6e`](https://github.com/Snd-R/komga-client/commit/74412a6e27402b90f73672c7452f60a0f05914ca) | DTO strictness (`WPLink.rel: String?`, `published: LocalDate`, contributor lists, `ThumbnailSeries.Type`) |
| Komelia (Android) | 0.19.0 [`65f92fde`](https://github.com/Snd-R/Komelia/commit/65f92fde60b7b7b62b85a55ceb80b92adf50eec8) | Observed route profile (`evidence/sanitized-komelia-0.19.0.json`), one-based page requests, scalar `libraryId` query encoding |
| Mihon tracker + Keiyoushi extension | Mihon `21af65b` (`KomgaApi.kt`), Keiyoushi v1.6.69 `819e24c1` | `GET/PUT /api/v2/series/{id}/read-progress/tachiyomi`, legacy `GET /api/v1/books` |
| Liseur Komga provider | [`31f8182d`](https://github.com/chmouel/liseur/commit/31f8182d524e3536cf9020594185e709a033094f) | API-key-only REST catalog/file/progression (`liseur-providers.mdx:32-56`) |
| Grimmory `KomgaController` | main (`komga-compat.mdx` Grimmory shim section) | `/komga` prefix alias: GET listing + own `users/me` shape |
| Komf | master `428dac6` via komga-client 0.10.3 | Basic + SSE `TaskQueueStatus`, metadata/cover writes, author role `cover` (commit `ab31ea4c`) |
| Readium Web Publication | <https://readium.org/webpub> | Native EPUB routes keep spec shapes; the Komga alias narrows to Komelia's types |

Client-verification status (`docs/content/docs/developer/client-verification.mdx:24-32`):

| Client | Level | Harness |
| --- | --- | --- |
| Komelia 0.19.0 | **Device** (login, browse, CBZ+EPUB read, offline download, progress; 2026-09-03/04) | `make replay` 8/8, `replay-readium` |
| Liseur (Komga profile) | **Device** (login, browse, read, progress; 2026-09-04) | — |
| Mihon / Tachiyomi | **Device** (browse, download, read, enhanced tracker; 2026-09-04) | `replay-mihon` (Basic + `X-API-Key`) |
| Komf `428dac6` | **Live container** (identify → series/book PATCH, cover upload/delete, SSE) | `replay-komf` 16/16 |
| Grimmory alias | **Device** login (via Liseur); listing/file/thumbnail harness | `specs/grimmory.hurl` |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Two gates: Cargo feature `komga = ["readium", "dep:stump_komga"]` **and** `STUMP_ENABLE_KOMGA=true` (release `false`, debug `true`); runtime-on/feature-off logs a warning | Feature drops the crate from `minimal`; switch keeps `/api/v1` free for native use | `apps/server/Cargo.toml`; `core/src/config/protocols.rs:48-52`; `apps/server/src/routers/mod.rs:45-48,80-85` |
| `KomgaBackend: Send + Sync` exposes `conn()`/`conn_arc()` plus 20 async platform methods; handlers run SeaORM queries themselves | Keeps existing visibility/query expressions in one place; only file/image/EPUB/job ops are abstracted | `src/routes/mod.rs:88-207` |
| Router is `Router<S>` with backend + `KomgaEvents` as `Extension`s; `legacy_books_router` and `grimmory_routes` are separate | Server applies its auth middleware after composition and mounts the same adapter at root and `/komga` | `src/routes/mod.rs:236-286`; `apps/server/src/routers/komga/mod.rs` |
| `/api/v1/{*path}` wildcard warns and 404s | Komga profile owns the namespace; never fall through to the SPA | `src/routes/mod.rs:257-259,301-307` |
| `is_komga_path` (`/api/v1`, `/api/v2`, `/sse/v1`) is the single path-ownership predicate | Server Basic-auth/remember-me scope and Komga-401 cookie handling key off it | `src/routes/mod.rs:288-299`; `apps/server/src/middleware/auth.rs` |
| Basic/`X-API-Key` allowed only on an enumerated path list incl. the Mihon tracker path and exact `/sse/v1/events` | Mihon sends only Basic/API-key on the tracker path (was 401 on device before widening) | `komga-compat.mdx:76-94`; `client-verification.mdx:31` |
| Komga 401s do not clear the session cookie | Komelia's REST/SSE/image clients share one cookie jar | `apps/server/src/middleware/auth.rs`; `komga-compat.mdx:99-128` |
| `Page<T>`: zero-based `page`, `size` default 20, clamp >200, `size=0` count-only, `unpaged`; negative → 400 | Spring envelope Komelia parses | `src/routes/catalog.rs:104-170`; `src/common.rs:113-170` |
| Query extraction via `axum_extra::extract::Query`; list fields are `Vec` + `#[serde(default)]` | Komelia sends scalar `libraryId`; repeated and scalar forms both decode | `komga-compat.mdx:231` |
| Search conditions are an explicit allowlist; unsupported variants → 400 with a warning | Silent ignore would return wrong result sets | `src/routes/catalog.rs:759-1108`; `src/search.rs` |
| IDs are Stump IDs; `book.url`/`series.url` are virtual last components, never real paths; missing → `UNKNOWN` | No filesystem leak; Komelia only displays them | `src/routes/mapper.rs:30-90,739-855` |
| Page requests are one-based; zero-based/conversion options rejected; read-progress page is zero-based `0..pages`; Readium positions one-based | Matches Komelia's observed calls and Komga semantics | `src/routes/media.rs:24-35,151-176`; `src/routes/progress.rs:216-255` |
| EPUB `pages` list is `[]` (Komga behaviour for non-Divina EPUBs) | Stump's synthetic `pagesCount` broke Komelia open/download with a 500 | `komga-compat.mdx:41`; `apps/server/src/routers/komga_backend.rs` `book_pages` |
| `normalize_manifest_for_komga_client`: flatten `rel` to string, truncate `published` to `YYYY-MM-DD`, wrap scalar contributors | komga-client 0.11 types are stricter than the Readium spec | `src/routes/readium.rs:111-178`; `komga-compat.mdx:42-44` |
| Readium resource paths must be relative, non-empty, traversal-free; stale `modified` on `PUT /progression` → 409 | Path safety; shared timestamp-conflict rule | `src/routes/readium.rs:203-249,300-490` |
| Series thumbnail lists never emit `GENERATED`; generated cover only via `/series/{id}/thumbnail` | Komelia decodes `ThumbnailSeries.Type` with an unguarded `valueOf` over `SIDECAR|USER_UPLOADED` | `src/routes/media.rs:311-380`; `src/series.rs:127-151` |
| `cached_bytes`: strong SHA-256 ETag, private revalidation, 304 on `If-None-Match`; no byte ranges on pages/thumbnails; files via `ServeFile` | Komelia caches by ETag; ranges only matter for full files | `src/routes/response.rs:10-65`; `specs/cache-range.hurl` |
| SSE: 15 s keepalive, visibility-filtered (`find_for_user`), private progress events; core events forwarded: `CreatedMedia`, `MediaDeleted`, `SeriesDeleted`, `JobQueueStatus` → `TaskQueueStatus` | Komf automatic mode only flushes on a zero-count `TaskQueueStatus`; nothing fabricated | `src/routes/sse.rs`; `src/routes/mod.rs:54-86`; `apps/server/src/routers/komga_backend.rs:45-75` |
| Author role `cover` (lowercase Komga vocabulary) accepted and emitted | Komf's first live run sent `COVER` and was rejected | commit `ab31ea4c`; `src/routes/book.rs` |
| Permissions: metadata/analyze/thumbnail writes need `ManageLibrary`, scan `ScanLibrary`, file download `DownloadFile`, filesystem browse owner-only | Reuse Stump's permission model unchanged | `src/routes/media.rs:388-523`; `src/routes/library.rs:45-48`; `src/routes/book.rs:42-44` |

Route families (declared in `src/routes/*.rs`; full matrix in `komga-compat.mdx`): catalog (`libraries`, `series`, `books`, `ondeck`, `new`/`updated`, `previous`/`next`, tags/authors/genres/publishers/languages/age-ratings/release-dates/sharing-labels, `POST books/list`, `POST series/list`, legacy `GET /api/v1/books`); lists (`readlists`, `collections` GET/PATCH, tachiyomi read-list progress); library actions (`metadata/refresh`, `analyze`, `empty-trash`); book/series metadata PATCH and read-progress; media (pages, thumbnails GET/POST/DELETE, `file`, scan, analyze, filesystem); progress (`PATCH/DELETE read-progress`); Readium (`manifest[.json]`, `positions[.json]`, resource wildcard, `GET/PUT progression`); `GET /sse/v1/events`; Grimmory (`GET /api/v1/books`, `GET /api/v2/users/me`).

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | module declarations and flat re-exports of every DTO |
| `src/{announcements,book,collection,common,filesystem,library,read_list,readium,search,series,settings,sse,user}.rs` | Serde DTOs per Komga resource; `search.rs` = recursive `AllOf/AnyOf` condition tree; `common.rs` = `Page<T>`, sort, author, patch helpers |
| `src/errors.rs` | `APIError` → status + `{status,message}` body |
| `src/routes/mod.rs` | `KomgaImage`, `KomgaCoreEvent`, `KomgaBackend`, `KomgaEvents` bus, `router`, `legacy_books_router`, `grimmory_routes`, `is_komga_path` |
| `src/routes/catalog.rs` | catalog/search routes, pagination validation, condition evaluation, full-text search |
| `src/routes/lists.rs`, `library.rs`, `book.rs`, `series.rs`, `progress.rs` | read lists/collections; library jobs; metadata writes; series read state; book read-progress |
| `src/routes/media.rs`, `readium.rs`, `response.rs` | pages/thumbnails/files/jobs; Readium aliases + normalizer + progression; ETag/cache helpers |
| `src/routes/mapper.rs`, `sse.rs`, `grimmory.rs` | Stump→Komga projections; event stream; Grimmory shim |
| `apps/server/src/routers/komga/{mod,identity,settings}.rs`, `komga_backend.rs` | mounting at root + `/komga`, identity/session/settings, concrete backend |

## How to verify

```text
cargo test -p stump_komga --lib --tests       # 77 inline tests (18 DTO, 59 route/mapper/sse)
cargo test -p stump_server --lib komga        # mount test in routers/komga/mod.rs (must stay: Axum panics on overlapping routes)
cargo check -p stump_server --no-default-features --features minimal   # feature-off
STUMP_ENABLE_KOMGA=false                      # runtime-off; native /api routes only
cd ../komga-compat && make replay             # 8/8 observed specs (auth, libraries, catalog, books, siblings, cache-range, session-cookie-scope, grimmory)
make replay-negative-auth; make replay-readium; make replay-mihon SERIES_ID=…; make replay-komf
```

Live probe (fixture `http://127.0.0.1:25600`, credentials from generated
`ENDPOINTS.md`): `GET /api/v2/users/me?remember-me=true` with Basic → 200 +
`komga-remember-me` cookie; `POST /api/v1/books/list` `{}` → `Page` envelope;
`GET /sse/v1/events` → `text/event-stream`. After a rebuild log out/in once to
re-store the remember-me token.

## Deep docs

- `docs/content/docs/developer/komga-compat.mdx` — the authoritative profile spec: routes, auth, pagination, search, DTO losses, replay diagnostics.
- `docs/content/docs/developer/standards.mdx` (Komga profile, Grimmory alias) — one-table summaries.
- `docs/content/docs/developer/provider-status.mdx` (Komga) — status table (SSE row is stale: four core events are forwarded, not only `CreatedMedia`).
- `docs/content/docs/developer/clients.mdx`, `client-verification.mdx` — Komelia/Mihon/Liseur/Komf/Grimmory rows.
- `docs/content/docs/developer/liseur-providers.mdx` (Komga provider, Grimmory) — Liseur's exact requests.
- `docs/content/docs/developer/unified-reading-state.mdx` (Komga Readium) — locator mapping precision.
- `docs/content/docs/developer/roadmap.mdx` — Komf follow-ups; `local://komf-compat-study.md`.
- Harness: `/home/al/Code/komga-compat/README.md`, `specs/*.hurl`, `evidence/sanitized-komelia-0.19.0.json`.
