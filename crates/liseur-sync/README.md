# stump_liseur_sync

## Purpose

`stump_liseur_sync` is the native liseur-sync wire contract: every `/v1/*` DTO,
`LiseurSyncError` and its HTTP mapping, the bearer `token_auth` middleware,
scope/field validation, and the Axum `routes()` tree. Persistence is supplied by
the host through the `LiseurSyncBackend` trait. The crate
deliberately does **not** own tokens, work identity, position/annotation
storage, catalog lookup, or Coppice media access — all of that is the server
adapter (`apps/server/src/routers/liseur_sync/{mod,storage}.rs`). Remote
annotations stay in adapter-owned revision/sequence/tombstone tables; valid
Readium highlights/bookmarks also have stable native projections without
replacing the CAS source of truth.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| liseur-sync server | [`906889ff` `docs/openapi.yaml`](https://github.com/chmouel/liseur-sync/blob/906889ffe11fc7260b071c1f142aeee1fa8a1419/docs/openapi.yaml) | Current documented routes, measured sessions, and structured refusals; `/v1/me/settings` is implemented in Go source but absent from this OpenAPI revision |
| liseur-sync ADR 0028 | [`docs/adr/0028-annotation-sync.md`](https://github.com/chmouel/liseur-sync/blob/f8ce32b79aca0cddde84e9004fedde121fb77365/docs/adr/0028-annotation-sync.md) | Annotations beside, not inside, the append-only op log |
| Liseur Android client | [`v0.19.0` `62ecb5a5`](https://github.com/chmouel/liseur/commit/62ecb5a5c9dd8eb4e7fa6d97ce50d1bddae0bcd6) — `AnnotationWire.kt`, `LiseurSyncApi.kt`, `LiseurSyncSettings.kt`, `LiseurSyncPositionSync.kt` | Current source contracts; older device replay remains pinned to `31f8182d` |
| Liseur Android client | [`v0.13.0` `799a8c7a`](https://github.com/chmouel/liseur/commit/799a8c7afac022d49388cb637f091765557b0acf) — `app/src/main/kotlin/com/chmouel/liseur/data/liseursync/AnnotationWire.kt` | Strict six-color annotation decoder drops a whole record for an unsupported color; legacy responses therefore omit only extended fields |

Client-verification status (`docs/content/docs/developer/client-verification.mdx:29`):
**Device** (2026-09-04, Liseur `31f8182d`) — login, token mint, `/v1/token`,
folders/books/covers, download, positions/heads. The last recorded
`make replay-liseur-sync` run covered 25 requests on that older baseline.
This change adds Hurl assertions for account-settings LWW/merge behavior,
session `active_ms`, and structured work-refusal identities. These expanded
contracts have not yet been replayed against a rebuilt Coppice server.

Current Liseur v0.19.0 (`62ecb5a5`) and liseur-sync (`906889ff`) pins are
source-only; no device run is claimed for these newly inspected client-source
contracts.

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Compile-time only: Cargo feature `liseur-sync` (in `headless`); mounted whenever compiled, no runtime switch | Nothing dormant to toggle; `/v1/*` does not collide with native routes | `apps/server/Cargo.toml` `liseur-sync = ["dep:stump_liseur_sync"]`; `apps/server/src/routers/mod.rs:50-53` |
| Backend arrives as `Extension<B>` (`B: Clone`), router state `S` unused | Host injects one adapter; crate never sees `AppState` | `src/lib.rs:668-676`; `apps/server/src/routers/liseur_sync/mod.rs:28-30` |
| Only `POST /v1/login` is public; every other route sits behind `token_auth` (`Authorization: Bearer`, exact non-empty) which inserts `AuthContext` + `LiseurToken` | Matches upstream; keeps Stump session cookies out of the protocol | `src/lib.rs:677-706,708-730`; test `login_route_is_public_but_protected_routes_require_bearer` |
| Login session (1 h) may only mint/revoke; device tokens sync, self-introspect, and read catalog; `admin` scope needs server owner | Least privilege per token kind | `src/lib.rs:1015-1100,1483-1501`; tests `session_tokens_are_management_only`, `device_tokens_can_sync_and_self_introspect` |
| Seven scopes (`sync`, `read-insights`, `library-read`, `library-manage`, `library-upload`, `library-delete`, `admin`); scalar `scope` and array `scopes` must agree, deduplicated | Upstream vocabulary; tolerate both client encodings | `src/lib.rs:39-47,168-207`; test `token_scope_requests_are_validated_and_canonicalized` |
| Sync routes need a non-session token with `sync`; catalog needs `library-read`; catalog-book resolve needs both | Mirrors client's per-route expectations | `src/lib.rs:792-824,963-986,1483-1501` |
| Errors use documented status/code fields; `/v1/ops` and `/v1/sessions` item refusals include `item_index` and the relevant `op_id` or `session_id`, with `work_id`/`limit` when applicable | Liseur uses these identities to re-resolve stale works, retry without an oversized locator, split batches, or refuse only the irreparable session | `src/lib.rs` `push_ops`, `push_sessions`, `error_response`; liseur-sync `906889ff` `docs/openapi.yaml:704-713,848-861` |
| Settings and sync batch JSON bodies are capped at 1 MiB; the annotation `body` field is capped at 16 KiB, token names at 256 B, and JSON rejection maps to 400 or 413 | Bound request buffering without confusing a field limit with a whole-body limit | `src/lib.rs` `MAX_SYNC_BODY_BYTES`, `MAX_BODY_BYTES`, `push_ops`, `push_sessions`, `get_settings`, `put_settings` |
| `GET/PUT /v1/me/settings` stores a per-account map; PUT applies only strictly newer RFC3339 timestamps and returns the full merged snapshot. Missing `value` means empty; explicit `null` is invalid | Matches the current Android client’s account-settings synchronization and upstream LWW semantics | `src/lib.rs` `SettingValue`, `SettingUpdate`, `validated_settings`; `apps/server/src/routers/liseur_sync/storage.rs` `settings`, `put_settings` |
| `SessionInput.active_ms` is optional, bounded to the JavaScript safe-integer maximum, and authoritative for projected elapsed time; legacy records still use wall duration minus `idle_ms` | The current client sends measured time only when `GET /v1/token` advertises `session_active_ms` | `src/lib.rs` `SessionInput`, `validate_session`, token introspection; `storage.rs` `project_liseur_session` |
| Download goes through `tower_http::ServeFile::try_call` then overrides media type and sanitized attachment filename; missing file → 410 | Keeps `Range`/conditional semantics `LiseurSyncFileSource.kt` relies on | `src/lib.rs:927-960` |
| Attachments are side objects keyed by annotation id: `PUT /v1/annotations/{id}/attachments/{kind}`, `GET .../attachments`. Storing one never touches the record's `rev`/`seq` | A markup SVG plus a page snapshot cannot fit `body` (16 KiB), and bending the CAS contract around a binary would break every client that pinned the OpenAPI | `src/lib.rs` `ATTACHMENT_KINDS`, `put_attachment`, `list_attachments`; `apps/server/src/routers/liseur_sync/attachments.rs` |
| The digest is the idempotency key: the crate hashes the body and compares it to `X-Attachment-Sha256` (409 on mismatch); a repeat of the same digest returns the same attachment id | Every backend then reports the same failure for a corrupted transfer, and a device replaying a batch on its backup URL costs one request | `src/lib.rs` `read_attachment`; api test `upload_is_idempotent_by_digest_and_never_moves_the_annotation_revision` |
| `Content-Length` over `attachment_max_bytes` is refused before the body streams; a chunked body is bounded by `axum::body::to_bytes(body, max)`. `DEFAULT_ATTACHMENT_MAX_BYTES` is 8 MiB, overridable through `LiseurSyncBackend::attachment_max_bytes` | The limit is a host policy (`STUMP_ATTACHMENT_MAX_BYTES`), but the crate must not buffer an unbounded body to discover it | `src/lib.rs` `read_attachment`; api test `uploads_are_rejected_on_digest_mismatch_size_kind_media_type_and_identity` |
| An annotation carrying attachments needs an id of `[A-Za-z0-9._-]` not starting with a dot; the annotation lane still accepts any bounded id | The id names the storage directory, and `Path<String>` percent-decodes, so `ks1%2Fescape` would otherwise leave the attachment root | `src/lib.rs` `validate_attachment_annotation_id` |
| Cover URLs built from `X-Forwarded-Proto` + `Host`; cover response sets `X-Content-Type-Options: nosniff` | Proxy-correct absolute links | `src/lib.rs:764-790,900-924` |
| Catalog trait methods default to `NotFound("catalog unavailable")`; sync/token/annotation/settings methods are required | A host may ship sync without a catalog; Stump supplies the production catalog, series-name, and sync adapters | `src/lib.rs` `LiseurSyncBackend`; `apps/server/src/routers/liseur_sync/mod.rs` |
| `ANY /v1/entities/series/{id}/order` → 404 `catalog entity route is not implemented` | Explicit deferral instead of silent wildcard | `src/lib.rs` `deferred_catalog` |
| `GET /v1/events` → 404 `live event stream is not implemented` behind `token_auth` | Liseur ≥ v0.15.0 opens the SSE feed once per foreground session, stops for the session on 401/403/404/501, and retries with capped backoff on anything else — including the host SPA fallback's redirect to HTML; the feed itself stays deferred | `src/lib.rs` `deferred_events`, test `live_event_stream_is_refused_with_not_found_behind_bearer_auth`; Liseur `62ecb5a5` [`LiveRetry.kt:19-20`](https://github.com/chmouel/liseur/blob/62ecb5a5c9dd8eb4e7fa6d97ce50d1bddae0bcd6/app/src/main/kotlin/com/chmouel/liseur/data/remote/LiveRetry.kt#L19-L20), [`LiseurSyncLive.kt:142-149`](https://github.com/chmouel/liseur/blob/62ecb5a5c9dd8eb4e7fa6d97ce50d1bddae0bcd6/app/src/main/kotlin/com/chmouel/liseur/data/liseursync/LiseurSyncLive.kt#L142-L149) |
| Liseur v0.19.0 pin (`62ecb5a5`) needs no wire change from v0.18.0; v0.17.0 exact-screen bookmarks (#213) leave `AnnotationWire` unchanged and never send the local page number | `data/liseursync`, `data/komga`, `data/opds`, `data/kosync`, `data/kobo` differ from `b00ee789` only by a `RemoteServer` parameter on `FileSource.downloadRequest`; Liseur-authored rows are stored and replayed as raw locator JSON, so `locations.liseurAnchor`/`cssSelector` survive, while native `stump-native:bookmark:*` exports carry no viewport anchor and are matched client-side by `href` + `position`/`progression` | Liseur `62ecb5a5` [`ReadingPlace.kt:28-47`](https://github.com/chmouel/liseur/blob/62ecb5a5c9dd8eb4e7fa6d97ce50d1bddae0bcd6/app/src/main/kotlin/com/chmouel/liseur/reader/progress/ReadingPlace.kt#L28-L47), [`AnnotationWire.kt:81-137`](https://github.com/chmouel/liseur/blob/62ecb5a5c9dd8eb4e7fa6d97ce50d1bddae0bcd6/app/src/main/kotlin/com/chmouel/liseur/data/liseursync/AnnotationWire.kt#L81-L137); `docs/content/docs/developer/liseur-providers.mdx` "Current Liseur v0.19.0 source changes" |
| Ops idempotent by `op_id`; annotations CAS on `base_rev` with tombstones; separate per-user position and annotation sequences | Upstream conflict model; a stale rev is a conflict, not an overwrite | `apps/server/src/routers/liseur_sync/storage.rs:1229-1321,1392-1651,1773-2058` |
| `ANNOTATION_COLUMNS` selects `annotation_id AS id` (table PK is `row_id`) | Fix for the `no such column: id` 500 on `/v1/works/{id}/annotations` | `storage.rs:1673-1676,1994-1997`; regression test `work_annotations_reads_annotation_id_column` (`storage.rs:2085`) |
| Changes/annotation limits outside `1..=500` use the pinned client's default `500`; position limits outside `1..=200` use `50` | The reference client treats absent, zero, negative, and oversized values as defaults rather than clamping them to a different page size | `src/lib.rs::{cursor_params,position_limit}`; test `invalid_query_limits_use_reference_client_defaults` |
| Catalog listings omit optional SHA-256; explicit book resolution computes a full-file digest and always carries the stable `source` id, while sampled `media.hash` is never labelled or queried as SHA-256 | Liseur permits a nullable catalog digest; computing it for every listing is wasteful, and confusing Stump's sampled identity with a cryptographic digest can verify the wrong edition | `apps/server/src/routers/liseur_sync/storage.rs::{resolve_catalog_book,find_media_for_identifier,resolve_work}` |
| Deferred: `/v1/register`, token listing/scope updates, `/v1/insights/*`, upload/delete, tombstone retention | Not needed by the pinned client's verified flows | `docs/content/docs/developer/liseur-sync-integration.mdx` |
| Extended KOReader colors/drawers require `X-Liseur-Annotation-Capabilities: annotation-color-drawer-v1`; without it, unsupported colors are omitted but the annotation remains visible | Pinned Liseur v0.13/v0.18 discard records whose `color` is outside their six-token palette; the plugin opts in to the full KOReader palette and style | `src/lib.rs` `annotation_record_for_client`; pinned `AnnotationWire.kt` sources; regression test `validates_protocol_boundaries` |
| Linked native highlights/notes/bookmarks export under deterministic `stump-native:*` CAS IDs. Liseur edits use `base_rev` to write text/color/locator back; Home GraphQL edits change body/color only and preserve the locator. Either side's delete publishes a tombstone. `liseur-sync:*` mirrors are excluded from re-export; `origin_device_id` is immutable creator attribution and `device_id` tracks the last writer | One ordered feed preserves native identity without projection echoes; CAS revisions/sequence track every edit and delete | `apps/server/src/routers/liseur_sync/storage.rs` `reconcile_native_annotations`, `writeback_native_annotation`, `delete_annotation`; `crates/graphql/src/mutation/epub.rs` `update_liseur_annotation`, `delete_liseur_annotation`; `m20260962_add_liseur_annotation_origin`; round-trip tests in both crates |
| Series-name PUT/DELETE supports per-account `personal` overlays only; normalized names cannot collide with another visible series, and scanned `series.name` stays unchanged | Personal display labels must not mutate folder metadata or leak to another account | `apps/server/src/routers/liseur_sync/storage.rs:463-621`; test `series_names_are_personal_overlays_and_conflicts_are_normalized` |

Routes (`src/lib.rs`):

| Group | Routes |
| --- | --- |
| Public | `POST /v1/login` |
| Tokens | `GET /v1/token`, `POST /v1/tokens`, `DELETE /v1/tokens/{id}` |
| Account settings | `GET /v1/me/settings`, `PUT /v1/me/settings` |
| Identity / positions | `POST /v1/works/resolve`, `POST /v1/ops`, `GET /v1/changes`, `GET /v1/heads`, `GET /v1/works/{id}/positions`, `POST /v1/sessions` |
| Annotations | `POST /v1/annotations`, `GET /v1/annotations/changes`, `DELETE /v1/annotations/{id}?rev=`, `GET /v1/works/{id}/annotations?include_deleted=` |
| Catalog | `GET /v1/folders`, `/v1/folders/{f}/books`, `/v1/folders/{f}/search`, `/v1/books/{id}`, `/cover`, `/download`, `/series`; `POST /v1/books/{id}/resolve` |
| Personal series names | `PUT/DELETE /v1/entities/series/{id}/name` |
| Deferred | `ANY /v1/entities/series/{id}/order` → 404; `GET /v1/events` → 404 (Liseur stops its live connector for the session) |
| The catalogue (`/v1/folders/{id}/books`, `/search`, `/books/{id}`, `/download`) excludes audiobook rows (`models::entity::media::audio_extension_condition().not()`) | Liseur reads paginated documents; an audiobook has no pages to track, and a folder book's "download" is a directory — `ServeFile` answered that with a truncated `200` (`Content-Length: 82`, 35 bytes) and broke `replay-liseur-sync`. The ABS profile serves audio | `apps/server/src/routers/liseur_sync/storage.rs` `visible_media`; server test `audio::audiobooks_are_not_liseur_catalogue_books` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` (single file) | scopes/limits; `LiseurSyncError`; token kinds; public DTOs (`LoginResult`, `TokenCreateRequest`, `Identifier`, `ResolveRequest/Result`, `OpInput/Record/Result`, `ChangesPage`, `HeadsPage`, `SessionInput`, settings DTOs, `AnnotationInput/Record/Result`, `Catalog*`); `LiseurSyncBackend`; `routes()`; middleware; handlers; validators; `error_response`; tests |
| `apps/server/src/routers/liseur_sync/storage.rs` | SeaORM persistence: credentials/tokens, account settings, catalog, work identity, ops, sessions, annotations |

## How to verify

```text
cargo test -p stump_liseur_sync --lib
cargo test -p stump_server --lib liseur_sync
cargo test -p migrations --test liseur_sync_settings
cd ../komga-compat && make replay-liseur-sync BASE_URL=http://127.0.0.1:25600 USERNAME=… PASSWORD=…
```

Live probe: `POST /v1/login` with Stump credentials → 200 + session token;
`POST /v1/tokens` with `{"scopes":["sync","library-read"]}` → device token;
`GET /v1/folders` with that bearer → 200. Base `http://127.0.0.1:25600`,
credentials from the fixture's generated `ENDPOINTS.md`.

## Deep docs

- `docs/content/docs/developer/liseur-sync-integration.mdx` — wire contract, catalog, annotation bridge, auth, deviations, deferred list.
- `docs/content/docs/developer/liseur-providers.mdx` (native REST section) — current deltas pinned to Liseur v0.19.0 `62ecb5a5`; historical route evidence remains separately pinned.
- `docs/content/docs/developer/unified-reading-state.mdx` (liseur-sync projection) — position ops, sessions, annotation CAS in the unified model.
- `docs/content/docs/developer/standards.mdx` (liseur-sync) and `provider-status.mdx` — summary tables (provider-status catalog row is stale: catalog is read-only implemented).
- `docs/content/docs/developer/clients.mdx`, `client-verification.mdx` — Liseur liseur-sync rows.
