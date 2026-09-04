# Komga-compatibility findings for Grimmory (issue #164)

What Komelia 0.19.0, Mihon (Keiyoushi Komga extension + built-in Komga tracker) and Liseur actually require from a Komga-compatible server, learned while implementing the same profile for Stump (Rust), and how each item maps onto Grimmory `main` as of 2026-09-04.

Every client below was exercised end to end on an Android phone against my server (login, browse, read, progress, edits, offline downloads), first from a sanitized capture of the app's traffic and then from the phone itself against a live build. The regressions I hit map directly onto what Grimmory's `KomgaController` and Komga security chain do today, so I wrote them up route by route rather than as a wall of text on the issue. A Hurl harness (§10) replays every sequence the phones sent.

## Evidence pins

Every claim was checked against these, not against Komga's OpenAPI in the abstract.

| Source | Commit |
| --- | --- |
| Komelia 0.19.0 (Android) | `65f92fde60b7b7b62b85a55ceb80b92adf50eec8` |
| komga-client 0.11.0 (Kotlin, used by Komelia) | `74412a6e27402b90f73672c7452f60a0f05914ca` (= tag `0.11.0`) |
| Keiyoushi Komga extension (Mihon/Tachiyomi source) | `819e24c1c6581cf774ea9bcec979fda98cdb3e25` — `src/all/komga/src/eu/kanade/tachiyomi/extension/all/komga/Komga.kt` |
| Mihon built-in Komga tracker | `21af65b10012f23e53dba9d01fd2474fcb810f7c` — `app/src/main/java/eu/kanade/tachiyomi/data/track/komga/{KomgaApi,KomgaModels}.kt` |
| Liseur | `31f8182d524e3536cf9020594185e709a033094f` |
| Komga (reference behaviour) | `656001eb03bf8b54ca909f3e74fe2ec1b95dac48`; OpenAPI 1.26.3 |
| Grimmory | `main` @ `1682a98` (2026-09-04): `backend/src/main/java/org/booklore/controller/KomgaController.java`, `config/security/SecurityConfig.java`. PR #2333 (remember-me cookies) and #2377 (missing library DTO fields) are open and referenced where relevant. |

Legend: **[observed]** = present in the pinned source, the sanitized Komelia capture, or the phone's request log against my server; **[inference]** = conclusion drawn from those. Where I describe what Stump does, it is only to make the contract concrete — this is not a claim of full Komga compatibility on my side either.

---

## 1. Authentication — the blocker

### 1.1 What Komelia does [observed]

`komga-client/.../KomgaClientFactory.kt:42-83`:

- One shared `HttpCookies` storage for **all** clients (REST, SSE, image loading). Komelia supplies `RememberMePersistingCookieStore` (`komelia-domain/core/.../http/RememberMePersistingCookieStore.kt`).
- Basic credentials are installed only when no API-key provider exists. `HttpUserClient.getMe(username, password, rememberMe)` sends `Authorization: Basic` on `GET /api/v2/users/me?remember-me=true`; the app then relies on cookies.
- `RememberMePersistingCookieStore.addCookie` persists **only** a cookie named exactly `komga-remember-me` and drops any `KOMGA-SESSION`/`komga-remember-me` cookie whose `Path` differs from the server base path (`RememberMePersistingCookieStore.kt:32-46`).
- SSE: `install(SSE) { maxReconnectionAttempts = Int.MAX_VALUE; reconnectionTime = 10.seconds }`; Ktor's reconnect re-sends the original request (`DefaultClientSSESession.kt:136`, Ktor 3.5.2), so a stale cookie on the SSE connection keeps being replayed every 10 s.
- Background downloads run in a WorkManager worker (`AndroidDownloadManager.kt`, `DownloadWorker.kt`) that can start in a fresh process; it authenticates only with the persisted `komga-remember-me` cookie loaded via `loadRememberMeCookie()`.

### 1.2 Consequences [inference, confirmed on the phone]

1. A `STATELESS` Basic chain works for the login call and then fails everything else: Komelia sends Basic once and cookies afterwards.
2. Any `401` that carries `Set-Cookie: <session>=; Max-Age=0` — the classic "clear the cookie on auth failure" — wipes the session for the REST, image and SSE clients simultaneously, because they share the jar. On the phone this looked like a spontaneous logout mid-read, every ~10 s, triggered by the SSE reconnect replaying a stale cookie. Komga never clears the cookie on 401.
3. Without `komga-remember-me` being both **issued** (PR #2333) and **accepted on `/api/v1/books/{id}/file` with no `Authorization` header**, browsing works and every background download fails silently. This was the single most confusing symptom to diagnose from the app side.

### 1.3 Grimmory today [observed at `1682a98`]

`SecurityConfig.java:105-121` — `komgaBasicAuthSecurityChain`: `securityMatcher("/komga/api/v1/**", "/komga/api/v2/**")`, `SessionCreationPolicy.STATELESS`, `httpBasic`, 401 entry point writes a `WWW-Authenticate: Basic` challenge. No session, no remember-me. `/v2/users/me` (`KomgaController.java`) resolves the OPDS user and ignores `remember-me`.

**Delta:** session-on-Basic (or accept Basic on every route *and* make sure nothing in the chain emits a cookie clear), remember-me acceptance on the download path, no `Set-Cookie … Max-Age=0` on 401 for `/komga/api/**`.

### 1.4 Every client authenticates differently [observed]

| Client / profile | Auth on first call | Auth on every other request | Notes |
| --- | --- | --- | --- |
| Komelia (Komga profile) | `Authorization: Basic` + `?remember-me=true` on `GET /api/v2/users/me` | `KOMGA-SESSION` cookie; `komga-remember-me` in background | shared cookie jar |
| Mihon extension | `X-API-Key` **or** Basic (user's choice in the extension settings) on `GET /api/v1/libraries` | same header on every request; **no cookie is ever kept** | `Komga.kt` builds an OkHttp client without a cookie jar |
| Mihon tracker | Basic or `X-API-Key`, same setting | same; paths are under **`/api/v2`** (§6) | separate code from the extension |
| Liseur, Komga provider | `X-API-Key: <key>` | `X-API-Key: <key>` | **no `Authorization` header at all**; user pastes a Komga API key |
| Liseur, Grimmory provider | `Authorization: Basic` (OPDS user) on `/komga/api/v2/users/me` | Basic on `/komga/api/v1/**` | `GrimmorySetupClient.kt:28-49,144-175`; listing via `GET /komga/api/v1/books?page=&size=200`, file via `/komga/api/v1/books/{id}/file` |

So a Komga-compatible chain has to accept, on the Komga paths, all three of: Basic (creating a session), a session cookie, and `X-API-Key` — and must do so **cold** (no cookie) on every route Mihon touches, including the `/api/v2/series/{id}/read-progress/tachiyomi` tracker path. Grimmory's current chain matches Liseur's *Grimmory* profile and nothing else.

---

## 2. Catalog routes Komelia calls (and Grimmory's status)

From the sanitized Komelia capture + `komga-client` `Http*Client.kt`. Status column is Grimmory `main` (`KomgaController.java`): ✅ mounted, 501 = in the explicit "Unimplemented" list, 404 = falls to the catch-all.

| Method | Path | Komelia use | Grimmory |
| --- | --- | --- | --- |
| HEAD | `/login` | server probe before login | 404 (catch-all; Komelia tolerates non-2xx here [observed]) |
| GET | `/api/v2/users/me?remember-me=true` | login | ✅ (no remember-me) |
| GET | `/api/v1/libraries`, `/api/v1/libraries/{id}` | sidebar | ✅ (see PR #2377 for missing DTO fields) |
| **POST** | **`/api/v1/series/list`** (body: search conditions, `?page=&size=&sort=&unpaged=`) | every series listing/filter | **501** |
| **POST** | **`/api/v1/books/list`** | every book listing/filter, count-only with `size=0` | **501** |
| GET | `/api/v1/series/{id}`, `/api/v1/series/{id}/books` | series screen | ✅ |
| GET | `/api/v1/series/new`, `/api/v1/series/updated` | home dashboard | 501 |
| GET | `/api/v1/books/ondeck` | home dashboard | 501 |
| GET | `/api/v1/books/{id}` | book screen, download flow | ✅ |
| GET | `/api/v1/books/{id}/pages`, `/pages/{n}`, `/pages/{n}/thumbnail` | reader — **and the download flow, see §5** | ✅ pages; page thumbnail 404 |
| GET | `/api/v1/books/{id}/thumbnail`, `/thumbnails`, `/thumbnails/{tid}` | covers, cover picker | ✅ single; list/by-id 404 |
| GET | `/api/v1/books/{id}/file` | download (must honour `Range`, `Content-Length`, `Content-Disposition`) | ✅ |
| GET | `/api/v1/books/{id}/next`, `/previous` | reader navigation | 404 |
| GET | `/api/v1/books/{id}/readlists` | book screen (plain array, not `Page`) | 501 |
| GET | `/api/v1/books/{id}/manifest`, `/manifest.json`, `/positions`, `/resource/{path}` | EPUB reader (Readium WebPub) — see §5 for the DTO strictness | 501 manifest; others 404 |
| GET/PUT | `/api/v1/books/{id}/progression` | EPUB progress read/write (R2 locator) | 404 |
| PATCH/DELETE | `/api/v1/books/{id}/read-progress` | mark page / mark read/unread | 404 |
| POST/DELETE | `/api/v1/series/{id}/read-progress` | mark series read/unread | 404 |
| PATCH | `/api/v1/books/{id}/metadata`, `/api/v1/series/{id}/metadata` | edit dialogs | 404 |
| GET | `/api/v1/collections`, `/collections/{id}`, `/collections/{id}/series` | collections | ✅ list; detail/series 404 |
| GET | `/api/v1/readlists`, `/readlists/{id}`, `/readlists/{id}/books` | read lists | 501 |
| GET | `/api/v1/series/{id}/collections` | series screen (plain array) | 404 |
| GET | `/api/v1/tags`, `/tags/book`, `/tags/series`, `/genres`, `/publishers`, `/languages`, `/age-ratings`, `/sharing-labels`, `/series/release-dates`, `/api/v2/authors` | filter panel referentials | 404 |
| GET | `/sse/v1/events` | live updates | 404 |
| GET/PATCH | `/api/v1/settings` | settings screen | 404 |
| GET/PUT | `/api/v1/announcements` | settings screen | 404 |
| PATCH | `/api/v2/users/me/password`, `/api/v2/users/{id}/password` | change password | 404 |
| GET/POST, PATCH/DELETE | `/api/v2/users`, `/api/v2/users/{id}` | user management (server owner) | 404 |
| GET | `/api/v2/users/me/authentication-activity`, `/api/v2/users/authentication-activity`, `/api/v2/users/{id}/authentication-activity/latest` | settings screen | 404 |
| POST | `/api/logout` | logout button | 404 |
| POST | `/api/v1/libraries/{id}/scan`, `/analyze`, `/metadata/refresh`, `/empty-trash`; `/api/v1/books/{id}/analyze`, `/metadata/refresh` | library/book actions | 404 |
| POST | `/api/v1/filesystem` | library picker | 404 |

Priority order from the capture volume: **1** `books/list` + `series/list`, **2** auth/session (§1), **3** `read-progress` + `progression`, **4** SSE, **5** referentials, **6** settings/users.

---

## 3. Search-condition DSL (for `books/list` / `series/list`) [observed]

`komga-client/.../KomgaBookSearch.kt`, `KomgaSeriesSearch.kt`, and the Komelia capture. Body: `{"condition": <Condition>, "fullTextSearch": "..."}`; query: `page`, `size`, `sort=field,asc|desc` (repeatable), `unpaged`.

Two dialects must both decode:

```jsonc
// Tagged (kotlinx default, Liseur):
{"condition":{"type":"AllOfBook","allOf":[{"type":"MediaProfile","operator":"is","value":"EPUB"}]}}
// Untagged (Komelia's KomgaCatalogClient builds raw JSON, no "type"):
{"condition":{"allOf":[{"mediaProfile":{"operator":"is","value":"EPUB"}}]}}
```

Decode rule that worked for every observed body: exactly one distinguishing key per object (`allOf`/`anyOf`/`libraryId`/`seriesId`/`mediaProfile`/`mediaStatus`/`readStatus`/`tag`/`author`/`genre`/`publisher`/`language`/`ageRating`/`releaseDate`/`deleted`/`oneshot`/`title`/`titleSort`/`numberSort`/`collectionId`/`readListId`/`poster`/`sharingLabel`/`complete`/`seriesStatus`), optional `type` cross-checked when present; multiple or zero keys → 400.

Conditions Komelia actually sends: `allOf`, `anyOf`, `libraryId`, `seriesId`, `mediaProfile`, `mediaStatus`, `readStatus`, `tag`, `author`, `genre`, `publisher`, `language`, `ageRating`, `releaseDate` (with `isInTheLast` duration), `deleted`, `oneshot`, `collectionId`, `readListId`.

Sort keys observed (lowercased on decode; unknown → endpoint default, not 400): `metadata.titleSort`, `metadata.numberSort`, `metadata.releaseDate`, `createdDate`, `lastModified`, `readProgress.readDate`, `booksMetadata.releaseDate`, `name`, `number`.

Pagination: Spring `Page<T>` envelope (`content`, `pageable{sort,pageNumber,pageSize,offset,paged,unpaged}`, `totalElements`, `totalPages`, `last`, `first`, `number`, `numberOfElements`, `size`, `empty`, `sort{sorted,unsorted,empty}`). `size=0` → `content: []` with real `totalElements` (Komelia count idiom). `size>200` → clamp, don't reject. `page<0`/`size<0` → 400.

Two endpoints return a **plain array**, not `Page`: `GET /api/v1/books/{id}/readlists` and `GET /api/v1/series/{id}/collections` (`HttpBookClient.kt`, `HttpSeriesClient.kt`).

---

## 4. DTO details the Kotlin client is strict about [observed]

Enums must be exact (kotlinx `enum` decode fails hard unless the client wraps it):

| Field | Accepted values | Client guard |
| --- | --- | --- |
| `media.status` | `READY` `UNKNOWN` `ERROR` `UNSUPPORTED` `OUTDATED` | none |
| `media.mediaProfile` | `DIVINA` `PDF` `EPUB` | lenient (`MediaProfileSerializer` → null) |
| `readProgress` / read status | `UNREAD` `IN_PROGRESS` `READ` | none |
| `series.metadata.status` | `ENDED` `ONGOING` `ABANDONED` `HIATUS` | none |
| `metadata.readingDirection` | `LEFT_TO_RIGHT` `RIGHT_TO_LEFT` `VERTICAL` `WEBTOON` | none |
| `library.seriesCover` | `FIRST` `FIRST_UNREAD_OR_FIRST` `FIRST_UNREAD_OR_LAST` `LAST` | none |
| `library.scanInterval` | `DISABLED` `HOURLY` `EVERY_6H` `EVERY_12H` `DAILY` `WEEKLY` | none |
| book thumbnail `type` | `GENERATED` `SIDECAR` `USER_UPLOADED` | lenient |
| **series thumbnail `type`** | **`SIDECAR` `USER_UPLOADED` only** | **none — the offline importer uses an unguarded `valueOf`; emitting `GENERATED` fails every download with "No enum constant". Komga never lists generated series covers, so don't.** |
| `ageRestriction.restriction` | `ALLOW_ONLY` `EXCLUDE` (`NONE` on update) | none |

Other hard requirements:

- `book.url` / `series.url`: filesystem-style paths. Komelia's downloader uses `Path(book.url).name` as the local filename (`BookDownloadService.kt:97-108`), so the last segment must be the real file name with extension. I emit a virtual `/<series dir>/<file>` rather than the real path; the real path is not needed.
- `book.media.mediaType`: real MIME (`application/vnd.comicbook+zip`, `application/epub+zip`, `application/pdf`); Liseur's Grimmory client filters on `application/epub+zip`.
- `book.fileLastModified`, `created`, `lastModified`: ISO-8601 with zone.
- `KomgaUser`: `id`, `email`, `roles: Set<String>` (Komelia checks `ADMIN`, `FILE_DOWNLOAD`, `PAGE_STREAMING`), `sharedAllLibraries`, `sharedLibrariesIds`, `labelsAllow`, `labelsExclude`, `ageRestriction?`. Missing `labelsAllow`/`labelsExclude` → decode failure.
- `PATCH` request bodies use `PatchValue` semantics: field absent = unset, `null` = clear, value = set (`PatchValue.kt`). Unknown/unsupported fields should 400 with a message rather than be silently dropped.

---

## 5. Real EPUBs: what synthetic fixtures do not exercise [observed on the phone]

A synthetic test EPUB with no `dc:date`, no publisher and no TOC passed everything; these four only surfaced with real commercial EPUBs. Each is reproducible from the app's own error message, so if a tester reports one of these strings you know the cause:

| Symptom in Komelia | Cause | Rule |
| --- | --- | --- |
| `GET /api/v1/books/{id}/pages` → 500; blocks **open and download** | Komelia calls the page list before opening *and* before downloading; walking `1..pagesCount` as chapters fails past the spine when `pagesCount` is a synthetic position count | For non-Divina EPUBs return `[]`, which is what Komga does. `pagesCount` on the book DTO can stay a position count. |
| `Unexpected JSON token … $.links[0].rel` | komga-client 0.11 types `WPLink.rel` as `String?`; the Readium spec allows an array | Emit `rel` as a single string in the Komga-facing manifest (first value). Keep your native Readium routes spec-shaped if you have them. |
| `DateTimeParseException '1994-07-14T10:00:00+00:00' … index 10` | `WPMetadata.published` is `LocalDate`; `dc:date` passed through verbatim | Truncate to `YYYY-MM-DD`. |
| (latent) `publisher` as a string | `WPMetadata.publisher` and the other contributor fields (`author`, `translator`, `editor`, `artist`, `illustrator`, `letterer`, `penciler`, `colorist`, `inker`, `contributor`) are `List<String>` | Wrap scalars in arrays. `readingProgression` must be one of `rtl`/`ltr`/`ttb`/`btt`/`auto`. |

Also from that session: an out-of-range `/pages/{n}` and a missing EPUB resource should be `404`, not `500` — Komelia treats 500 as "server broken" and 404 as "skip". And `GET /api/v1/books/{id}/read-progress` can be `405`: progress reads come from the Readium `progression` route; only `PATCH` needs to exist.

---

## 6. Mihon / Tachiyomi [observed on the phone]

Two separate pieces of code talk to the server.

### 6.1 The Keiyoushi Komga extension (`Komga.kt`)

It does **not** use `POST /series/list`. It browses with Komga's **legacy GET list routes** (`Komga.kt:144-193`, `244-249`, `574-579`):

| Call | Shape |
| --- | --- |
| popular / latest / search | `GET /api/v1/series?search=&page=0&deleted=false&library_id=a,b&sort=metadata.titleSort,asc` (zero-based `page`; `library_id` comma-separated; `read_status`/`author` repeated; `status`/`genre`/`tag`/`publisher` comma-separated; `sort` repeatable) — or `/api/v1/books`, `/api/v1/readlists`, `/api/v1/collections/{id}/series` with the same query shape, depending on the "type" filter |
| chapter list | `GET /api/v1/series/{id}/books?unpaged=true&media_status=READY&deleted=false` (or `/api/v1/books?…` for a read list); ordered by `number` |
| pages | `GET /api/v1/books/{id}/pages` then `/pages/{n}` |
| filter data | `GET /api/v1/genres`, `/tags`, `/publishers` (plain string sets), `GET /api/v1/authors` as a **plain array of `{name, role}`**, `GET /api/v1/collections?unpaged=true` |

Login and `GET /api/v1/libraries` succeed without those routes, and then every browse is a 404 — the app just shows "no results", which is easy to misread as an auth problem. In Grimmory's table above these are the ✅ `/v1/series`, `/v1/books`, `/v1/series/{id}/books` mappings, so the extension is closer to working against Grimmory than Komelia is; the missing piece is the query-parameter filter set and `/v1/authors`.

### 6.2 The built-in "Komga" enhanced tracker (`KomgaApi.kt`)

Distinct code inside Mihon. It calls, on the series the user tracks:

- `GET /api/v1/series/{id}` (also `/api/v1/readlists/{id}` for read lists) — the `SeriesDto` in `KomgaModels.kt` is strict: `booksCount`, `booksReadCount`, `booksUnreadCount`, `booksInProgressCount`, `metadata{status,title,titleSort,summary,…Lock fields}`, `booksMetadata{authors,releaseDate,summary,summaryNumber,created,lastModified}` all required.
- `GET /api/v2/series/{id}/read-progress/tachiyomi` → `ReadProgressV2Dto {booksCount, booksReadCount, booksUnreadCount, booksInProgressCount, lastReadContinuousNumberSort: double, maxNumberSort: float}`. Status is derived client-side: `booksCount == booksUnreadCount` → unread, `== booksReadCount` → completed, else reading; `last_chapter_read = lastReadContinuousNumberSort`.
- `PUT` same path with `{"lastBookNumberSortRead": n}` → mark every book with `number <= n` read; then it re-GETs.
- Read-list variant under **v1**: `GET/PUT /api/v1/readlists/{id}/read-progress/tachiyomi` with `ReadProgressDto {…, lastReadContinuousIndex: int}` / `{"lastBookRead": i}` (1-based index in read-list order).

`lastReadContinuousNumberSort` is the number of the last book in the unbroken read prefix (books ordered by number, stop at the first unread), `0` when none.

**The trap:** this path is under `/api/v2`, while everything else Mihon touches is under `/api/v1`. If Basic/API-key acceptance is scoped to `/api/v1/**` (mine was), the tracker gets 401 while browsing and reading work, and the app reports "could not get item". Grimmory's chain matches both prefixes, so this specific bug does not apply — but the route itself is on the catch-all today.

---

## 7. Liseur [observed on the phone]

- **Liseur has no OPDS 2 parser.** `OpdsHttp.kt:93` accepts only `application/atom+xml;profile=opds-catalog`; a valid OPDS 2 (`application/opds+json`) catalog produces "server answered but not with an OPDS catalog". That is the app, not the server — point users at the 1.2 URL.
- Its Komga provider (`X-API-Key`, §1.4), OPDS 1.2 (merged library, covers, downloads) and KOReader-sync pairing all work against a Komga-profile server with no Liseur-specific code.
- Its own `liseur-sync` protocol (folders/books catalog with device tokens and scopes, positions/heads/changes, CAS annotations) is a separate contract; I implemented it natively rather than through the Komga profile. Not relevant to #164, mentioned only so nobody expects the Komga profile to cover it.

---

## 8. SSE contract [observed]

`GET /sse/v1/events`, `text/event-stream`, keep-alive comment every ~15 s. Komelia handles (`KomgaEvent.kt:288-330`): `SeriesAdded/Changed/Deleted`, `BookAdded/Changed/Deleted`, `ReadProgressChanged/Deleted`, `ReadProgressSeriesChanged/Deleted`, `ReadListAdded/Changed/Deleted`, `CollectionAdded/Changed/Deleted`, `Thumbnail*`, `SessionExpired`, `TaskQueueStatus`; unknown names are ignored. Frame: `event: <PascalCaseName>` + `data: {camelCase payload, no type tag}`.

What refreshes the UI after edits (minimum set): metadata PATCH → `SeriesChanged{seriesId,libraryId}` / `BookChanged{bookId,seriesId,libraryId}`; progress writes → `ReadProgressChanged{bookId,userId}` + `ReadProgressSeriesChanged{seriesId,userId}` + `BookChanged` + `SeriesChanged`; collection PATCH → `CollectionChanged{collectionId,seriesIds}`. Gate per subscriber: book/series/collection events only if that user can see the item; `ReadProgress*` only to the user whose progress changed (Komga semantics). Support `Last-Event-ID` on reconnect if you can; Komelia reconnects constantly on mobile.

---

## 9. Sessions & cookies contract I ended up with (works with Komelia 0.19.0)

Cookie names below are Stump's (`stump_session`); Komga's is `KOMGA-SESSION`. Komelia does not care about the session cookie's name — it only special-cases `komga-remember-me` — so a server may keep its own session cookie as long as it sets `Path=/`.

| Situation | Response |
| --- | --- |
| `GET /api/v2/users/me?remember-me=true` + valid Basic | 200, `Set-Cookie: <session>=…; Path=/; HttpOnly; SameSite=Lax` **and** `Set-Cookie: komga-remember-me=<opaque>; Path=/; HttpOnly; SameSite=Lax; Max-Age=1209600` |
| any Komga path, valid session cookie | authenticated, no `Set-Cookie` |
| Komga path, stale/unknown session cookie, nothing else | 401, **no** `Set-Cookie` |
| Komga path, no session/Authorization, valid `komga-remember-me` | 200 + fresh session cookie |
| Komga path, `X-API-Key` only (Liseur, Mihon) | authenticated, no cookie issued |
| `komga-remember-me` on a non-Komga path | ignored |
| `POST /api/logout` | 204, both cookies cleared (`Max-Age=0`), remember token revoked server-side |

The remember-me token is stored as a session row with its own expiry, so ordinary session revocation/expiry cleanup applies.

---

## 10. Harness

The phone sessions are pinned by Hurl specs that run against any base URL — a server that passes them will work with these apps for the covered flows: `auth`, `session-cookie-scope`, `catalog`, `books`, `siblings`, `readlists-collections`, `cache-range`, `readium` (real-EPUB manifest/page-list shape), `mihon` (the extension's exact browse sequence under both auth modes, cold, plus the tracker GET→PUT→GET), `liseur-sync`, and a `grimmory` alias spec for the `/komga/api/**` prefix. They are how most of §5–§6 were found — from the phone's own request sequence, not from reading the OpenAPI. I'm happy to contribute them, or the manifest normaliser, if that helps; say which shape (a `hurl` directory in your repo, or a gist) and I'll adapt the variables.

---

## 11. What Stump did *not* do (so nobody copies a gap)

- `POST /api/v1/filesystem` is an owner-only, root-constrained directory listing for Komelia's library picker — not Komga's general filesystem browser.
- No library create/delete/task routes, no Komga user CRUD beyond password + age restriction (`roles`, labels are rejected with 400), no sharing labels, no alternate titles persistence.
- No `ReadListAdded/Changed/Deleted` emission (no read-list write route yet).
- Komga's own KOReader/Kobo/OPDS endpoints are not mirrored; Stump has native ones.
- Mihon's readlist tracker route is implemented but only harness-tested (my fixture has no read lists).

---

## 12. Quick verification recipe (curl)

```sh
B=https://your-server            # Komga root for the Komelia profile
S=KOMGA-SESSION                  # your session cookie name
K=<api key>
SER=<series id>; BK=<epub book id>
# 1. login shape
curl -si -u user:pass "$B/api/v2/users/me?remember-me=true" | grep -i set-cookie
# 2. follow-up with session only (must be 200)
curl -si -b "$S=<from above>" "$B/api/v1/libraries" | head -1
# 3. stale cookie must NOT clear
curl -si -b "$S=stale" "$B/api/v1/libraries" | grep -ic 'max-age=0'   # expect 0
# 4. Komelia count idiom
curl -s -u user:pass -X POST "$B/api/v1/books/list?size=0" -H 'content-type: application/json' -d '{}' | jq .totalElements
# 5. untagged condition
curl -s -u user:pass -X POST "$B/api/v1/books/list?size=1" -H 'content-type: application/json' \
  -d '{"condition":{"allOf":[{"mediaProfile":{"operator":"is","value":"EPUB"}}]}}' | jq '.content[0].url'
# 6. Mihon browse, cold, API key (must be 200 with a Page)
curl -s -H "X-API-Key: $K" "$B/api/v1/series?search=&page=0&deleted=false&sort=metadata.titleSort%2Casc" | jq '.totalElements'
# 7. Mihon tracker, cold, under /api/v2 (must be 200)
curl -s -u user:pass "$B/api/v2/series/$SER/read-progress/tachiyomi" | jq .
# 8. real-EPUB shape: empty page list, string rel, date-only published
curl -s -u user:pass "$B/api/v1/books/$BK/pages" | jq 'length'                       # expect 0
curl -s -u user:pass "$B/api/v1/books/$BK/manifest" | jq '.links[0].rel, .metadata.published'
```
