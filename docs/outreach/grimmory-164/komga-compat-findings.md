# Komga-compatibility findings for Grimmory (issue #164)

What Komelia 0.19.0 and Liseur actually require from a Komga-compatible server, learned while implementing the same profile for Stump, and how each item maps onto Grimmory `main` (4e28fde, 2026-09-02).

Evidence pins (every claim below was checked against these, not against Komga's OpenAPI in the abstract):

| Source | Commit |
| --- | --- |
| Komelia 0.19.0 (Android) | `65f92fde60b7b7b62b85a55ceb80b92adf50eec8` |
| komga-client 0.11.0 (Kotlin, used by Komelia) | `74412a6e27402b90f73672c7452f60a0f05914ca` (= tag `0.11.0`) |
| Liseur | `31f8182d524e3536cf9020594185e709a033094f` |
| Komga (reference behavior) | `656001eb03bf8b54ca909f3e74fe2ec1b95dac48` |
| Grimmory | `4e28fde` (`backend/src/main/java/org/booklore/controller/KomgaController.java`, `config/security/SecurityConfig.java`) |

Legend: **[observed]** = present in the pinned source or the sanitized Komelia capture; **[inference]** = conclusion drawn from those. "Stump" rows describe our implementation only where it clarifies the contract; this is not a claim of full Komga compatibility on our side either.

---

## 1. Authentication — the blocker

### 1.1 What Komelia does [observed]

`komga-client/.../KomgaClientFactory.kt:42-83`:

- One shared `HttpCookies` storage for **all** clients (REST, SSE, image loading). Komelia supplies `RememberMePersistingCookieStore` (`komelia-domain/core/.../http/RememberMePersistingCookieStore.kt`).
- Basic credentials are installed only when no API-key provider exists. `HttpUserClient.getMe(username, password, rememberMe)` sends `Authorization: Basic` on `GET /api/v2/users/me?remember-me=true`; the app then relies on cookies.
- `RememberMePersistingCookieStore.addCookie` persists **only** a cookie named exactly `komga-remember-me` and drops any `KOMGA-SESSION`/`komga-remember-me` cookie whose `Path` differs from the server base path (`RememberMePersistingCookieStore.kt:32-46`).
- SSE: `install(SSE) { maxReconnectionAttempts = Int.MAX_VALUE; reconnectionTime = 10.seconds }`; Ktor's reconnect re-sends the original request (`DefaultClientSSESession.kt:136`, Ktor 3.5.2), so a stale cookie on the SSE connection keeps being replayed every 10 s.
- Background downloads run in a WorkManager worker (`AndroidDownloadManager.kt`, `DownloadWorker.kt`) that can start in a fresh process; it authenticates only with the persisted `komga-remember-me` cookie loaded via `loadRememberMeCookie()`.

### 1.2 Consequences [inference, confirmed live against Stump]

1. **Basic must create a session.** A stateless Basic chain "works" only because Ktor's `Auth` plugin is installed with `sendWithoutRequest { true }` when username/password are configured — but Komelia configures the factory *without* username/password after login (it uses `getMe` once), so subsequent requests carry no `Authorization`. Result: 401 on every request after the login screen.
2. **Never clear the session cookie on 401.** If any one client (typically the SSE reconnect with a pre-restart cookie) gets `Set-Cookie: KOMGA-SESSION=; Max-Age=0`, the shared jar loses the live session and *every* client starts failing. Komga does not clear cookies on 401; a compatible server must not either, on the Komga paths.
3. **Accept `komga-remember-me` with no `Authorization` header** on the file/thumbnail/metadata routes, or background downloads 401 (we saw bursts of five 401s per download attempt in the trace). Komga's default validity is `P14D` (`komga.remember-me.validity`).
4. Cookie attributes that survive Komelia's store filter: `Path=/` (must equal the base path), `HttpOnly`, `SameSite=Lax`.

### 1.3 Grimmory today [observed]

`SecurityConfig.java:103-121` — `komgaBasicAuthSecurityChain`: `securityMatcher("/komga/api/v1/**", "/komga/api/v2/**")`, `SessionCreationPolicy.STATELESS`, `httpBasic`, 401 entry point writes a `WWW-Authenticate: Basic` challenge. No session, no remember-me (PR #2333 adds issuance). `/v2/users/me` (`KomgaController.java:222-235`) resolves the OPDS user and ignores `remember-me`.

**Delta:** session-on-Basic (or accept Basic on every route *and* make sure nothing in the chain emits a cookie clear), remember-me acceptance on the download path, no `Set-Cookie … Max-Age=0` on 401 for `/komga/api/**`.

### 1.4 Liseur is different [observed]

| Client / profile | Auth on `GET …/users/me` | Auth on every other request | Notes |
| --- | --- | --- | --- |
| Komelia (Komga profile) | `Authorization: Basic` + `?remember-me=true` | `KOMGA-SESSION` cookie; `komga-remember-me` in background | shared cookie jar |
| Liseur, Komga provider | `X-API-Key: <key>` | `X-API-Key: <key>` | **no `Authorization` header at all**; user pastes a Komga API key (`Liseur … KomgaCatalogClient.kt`) |
| Liseur, Grimmory provider | `Authorization: Basic` (OPDS user) on `/komga/api/v2/users/me` | Basic on `/komga/api/v1/**` | `GrimmorySetupClient.kt:28-49,144-175`; listing via `GET /komga/api/v1/books?page=&size=200` (`GrimmoryCatalogClient.kt:34-64,364-419`), file via `/komga/api/v1/books/{id}/file` (`GrimmoryFileSource.kt:17-29`) |

So "the Komga key" in Liseur means an **API key sent as `X-API-Key`**, which a Basic-only chain rejects; Komelia never sends one. Grimmory's current chain matches Liseur's *Grimmory* profile and nothing else.

---

## 2. Catalog routes Komelia calls (and Grimmory's status)

From the sanitized Komelia capture + `komga-client` `Http*Client.kt`. Status column is Grimmory `main` (`KomgaController.java`): ✅ mounted, 501 = in the explicit "Unimplemented" list (`:251-276`), 404 = falls to the catch-all (`:278-290`).

| Method | Path | Komelia use | Grimmory |
| --- | --- | --- | --- |
| HEAD | `/login` | server probe before login | 404 (catch-all; Komelia tolerates non-2xx here [observed]) |
| GET | `/api/v2/users/me?remember-me=true` | login | ✅ (no remember-me) |
| GET | `/api/v1/libraries`, `/api/v1/libraries/{id}` | sidebar | ✅ (see PR #2377 for missing DTO fields) |
| **POST** | **`/api/v1/series/list`** (body: search conditions, `?page=&size=&sort=&unpaged=`) | every series listing/filter | **501** |
| **POST** | **`/api/v1/books/list`** | every book listing/filter, count-only with `size=0` | **501** |
| GET | `/api/v1/series/{id}`, `/api/v1/series/{id}/books` | series screen | ✅ (`/books` is GET; Komelia uses POST list with `seriesId` condition instead) |
| GET | `/api/v1/series/new`, `/api/v1/series/updated` | home dashboard | 501 |
| GET | `/api/v1/books/ondeck` | home dashboard | 501 |
| GET | `/api/v1/books/{id}` | book screen, download flow | ✅ |
| GET | `/api/v1/books/{id}/pages`, `/pages/{n}`, `/pages/{n}/thumbnail` | reader | ✅ pages; page thumbnail 404 |
| GET | `/api/v1/books/{id}/thumbnail`, `/thumbnails`, `/thumbnails/{tid}` | covers, cover picker | ✅ single; list/by-id 404 |
| GET | `/api/v1/books/{id}/file` | download (must honor `Range`, `Content-Length`, `Content-Disposition`) | ✅ |
| GET | `/api/v1/books/{id}/next`, `/previous` | reader navigation | 404 |
| GET | `/api/v1/books/{id}/readlists` | book screen (plain array, not `Page`) | 501 |
| GET | `/api/v1/books/{id}/manifest`, `/manifest.json`, `/positions`, `/resource/{path}` | EPUB reader (Readium WebPub) | 501 manifest; others 404 |
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
| thumbnail `type` | `GENERATED` `SIDECAR` `USER_UPLOADED` | lenient |
| `ageRestriction.restriction` | `ALLOW_ONLY` `EXCLUDE` (`NONE` on update) | none |

Other hard requirements:

- `book.url` / `series.url`: filesystem-style paths. Komelia's downloader uses `Path(book.url).name` as the local filename (`BookDownloadService.kt:97-108`), so the last segment must be the real file name with extension. We emit a virtual `/<series dir>/<file>` rather than the real path; the real path is not needed.
- `book.media.mediaType`: real MIME (`application/vnd.comicbook+zip`, `application/epub+zip`, `application/pdf`); Liseur's Grimmory client filters on `application/epub+zip`.
- `book.fileLastModified`, `created`, `lastModified`: ISO-8601 with zone.
- `KomgaUser`: `id`, `email`, `roles: Set<String>` (Komelia checks `ADMIN`, `FILE_DOWNLOAD`, `PAGE_STREAMING`), `sharedAllLibraries`, `sharedLibrariesIds`, `labelsAllow`, `labelsExclude`, `ageRestriction?`. Missing `labelsAllow`/`labelsExclude` → decode failure.
- `PATCH` request bodies use `PatchValue` semantics: field absent = unset, `null` = clear, value = set (`PatchValue.kt`). Unknown/unsupported fields should 400 with a message rather than be silently dropped.

---

## 5. Progress and Readium [observed]

- Comics: `PATCH /api/v1/books/{id}/read-progress` `{"page": N, "completed": bool}` (page is **1-based**), `DELETE` clears. `POST/DELETE /api/v1/series/{id}/read-progress` marks all books.
- EPUB: `GET/PUT /api/v1/books/{id}/progression` with an R2 `Locator` (`href`, `type`, `locations{progression,totalProgression,position,fragments}`, `koboSpan`, `text`), `device{id,name}`, `modified`. Komelia's EPUB reader also needs `GET /api/v1/books/{id}/manifest` (Readium WebPub manifest with absolute resource links built from the request `Host`, so a reverse proxy must preserve it) and `/resource/{path}`.
- `GET /api/v1/books/{id}/read-progress` is not called by Komelia; Komga answers 405 there.

---

## 6. SSE contract [observed]

`GET /sse/v1/events`, `text/event-stream`, keep-alive comment every ~15 s. Komelia handles (`KomgaEvent.kt:288-330`): `SeriesAdded/Changed/Deleted`, `BookAdded/Changed/Deleted`, `ReadProgressChanged/Deleted`, `ReadProgressSeriesChanged/Deleted`, `ReadListAdded/Changed/Deleted`, `CollectionAdded/Changed/Deleted`, `Thumbnail*`, `SessionExpired`, `TaskQueueStatus`; unknown names are ignored. Frame: `event: <PascalCaseName>` + `data: {camelCase payload, no type tag}`.

What refreshes the UI after edits (minimum set): metadata PATCH → `SeriesChanged{seriesId,libraryId}` / `BookChanged{bookId,seriesId,libraryId}`; progress writes → `ReadProgressChanged{bookId,userId}` + `ReadProgressSeriesChanged{seriesId,userId}` + `BookChanged` + `SeriesChanged`; collection PATCH → `CollectionChanged{collectionId,seriesIds}`. Gate per subscriber: book/series/collection events only if that user can see the item; `ReadProgress*` only to the user whose progress changed (Komga semantics).

---

## 7. Sessions & cookies contract we ended up with (works with Komelia 0.19.0)

Cookie names below are Stump's (`stump_session`); Komga's is `KOMGA-SESSION`. Komelia does not care about the session cookie's name — it only special-cases `komga-remember-me` — so a server may keep its own session cookie as long as it sets `Path=/`.

| Situation | Response |
| --- | --- |
| `GET /api/v2/users/me?remember-me=true` + valid Basic | 200, `Set-Cookie: stump_session=…; Path=/; HttpOnly; SameSite=Lax` **and** `Set-Cookie: komga-remember-me=<opaque>; Path=/; HttpOnly; SameSite=Lax; Max-Age=1209600` |
| any Komga path, valid session cookie | authenticated, no `Set-Cookie` |
| Komga path, stale/unknown session cookie, nothing else | 401, **no** `Set-Cookie` |
| Komga path, no session/Authorization, valid `komga-remember-me` | 200 + fresh `stump_session` |
| `komga-remember-me` on a non-Komga path | ignored |
| `POST /api/logout` | 204, both cookies cleared (`Max-Age=0`), remember token revoked server-side |

The remember-me token is stored as a session row with its own expiry, so ordinary session revocation/expiry cleanup applies.

---

## 8. What Stump did *not* do (so nobody copies a gap)

- `POST /api/v1/filesystem` is an owner-only, root-constrained directory listing for Komelia's library picker — not Komga's general filesystem browser.
- No library create/delete/task routes, no Komga user CRUD beyond password + age restriction (`roles`, labels are rejected with 400), no sharing labels, no alternate titles persistence.
- No `ReadListAdded/Changed/Deleted` emission (no read-list write route yet).
- Komga's own KOReader/Kobo/OPDS endpoints are not mirrored; Stump has native ones.
- Mihon (Keiyoushi Komga extension) has not been tested against the profile yet.

---

## 9. Quick verification recipe (curl)

```sh
B=https://your-server            # Komga root for the Komelia profile
S=KOMGA-SESSION                  # your session cookie name (Komga: KOMGA-SESSION; Stump: stump_session)
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
```
