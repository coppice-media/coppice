# stump_liseur_sync

## Purpose

`stump_liseur_sync` is the native liseur-sync wire contract: every `/v1/*` DTO,
`LiseurSyncError` and its HTTP mapping, the bearer `token_auth` middleware,
scope/field validation, and the Axum `routes()` tree. Persistence is supplied by
the host through the 22-method `LiseurSyncBackend` trait. The crate
deliberately does **not** own tokens, work identity, position/annotation
storage, catalog lookup, or Stump media access — all of that is the server
adapter (`apps/server/src/routers/liseur_sync/{mod,storage}.rs`). Remote
annotations stay in adapter-owned revision/sequence/tombstone tables; projection
into Stump's native `media_annotations`/`bookmarks` is deferred by design.

## Reference / upstream

| Reference | Pin | Used for |
| --- | --- | --- |
| liseur-sync server (OpenAPI 1.0) | [`f8ce32b7` `docs/openapi.yaml`](https://github.com/chmouel/liseur-sync/blob/f8ce32b79aca0cddde84e9004fedde121fb77365/docs/openapi.yaml) | Wire names (`serialized_wire_names_match_openapi` test), scopes, error shapes |
| liseur-sync ADR 0028 | [`docs/adr/0028-annotation-sync.md`](https://github.com/chmouel/liseur-sync/blob/f8ce32b79aca0cddde84e9004fedde121fb77365/docs/adr/0028-annotation-sync.md) | Annotations beside, not inside, the append-only op log |
| Liseur Android client | [`31f8182d`](https://github.com/chmouel/liseur/commit/31f8182d524e3536cf9020594185e709a033094f) — `LiseurSyncApi.kt`, `LiseurSyncCatalogClient.kt`, `WorkResolver.kt`, `LiseurSyncFileSource.kt` | The only real client; route/status expectations in `docs/content/docs/developer/liseur-sync-integration.mdx:94-119` |

Client-verification status (`docs/content/docs/developer/client-verification.mdx:29`):
**Device** (2026-09-04, Liseur `31f8182d`) — login, token mint, `/v1/token`,
folders/books/covers, download, positions/heads. **Harness** —
`make replay-liseur-sync` (`specs/liseur-sync.hurl`, 25 requests: login,
session-scope rejection, device mint/introspect, unknown bearer 401, resolve,
idempotent ops, changes, sessions, annotation push/changes/live/delete/tombstone,
catalog chain, revocation; rerun-stable). The earlier `GET /v1/folders` 404 and
`GET /v1/works/{id}/annotations` 500 (`no such column: id`) are fixed and
device-retested; `.omp/PROJECT_STATE.md:65-66` still lists them as in-flight —
the code is authoritative.

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Compile-time only: Cargo feature `liseur-sync` (in `headless`); mounted whenever compiled, no runtime switch | Nothing dormant to toggle; `/v1/*` does not collide with native routes | `apps/server/Cargo.toml` `liseur-sync = ["dep:stump_liseur_sync"]`; `apps/server/src/routers/mod.rs:50-53` |
| Backend arrives as `Extension<B>` (`B: Clone`), router state `S` unused | Host injects one adapter; crate never sees `AppState` | `src/lib.rs:668-676`; `apps/server/src/routers/liseur_sync/mod.rs:28-30` |
| Only `POST /v1/login` is public; every other route sits behind `token_auth` (`Authorization: Bearer`, exact non-empty) which inserts `AuthContext` + `LiseurToken` | Matches upstream; keeps Stump session cookies out of the protocol | `src/lib.rs:677-706,708-730`; test `login_route_is_public_but_protected_routes_require_bearer` |
| Login session (1 h) may only mint/revoke; device tokens sync, self-introspect, and read catalog; `admin` scope needs server owner | Least privilege per token kind | `src/lib.rs:1015-1100,1483-1501`; tests `session_tokens_are_management_only`, `device_tokens_can_sync_and_self_introspect` |
| Seven scopes (`sync`, `read-insights`, `library-read`, `library-manage`, `library-upload`, `library-delete`, `admin`); scalar `scope` and array `scopes` must agree, deduplicated | Upstream vocabulary; tolerate both client encodings | `src/lib.rs:39-47,168-207`; test `token_scope_requests_are_validated_and_canonicalized` |
| Sync routes need a non-session token with `sync`; catalog needs `library-read`; catalog-book resolve needs both | Mirrors client's per-route expectations | `src/lib.rs:792-824,963-986,1483-1501` |
| Errors → 401/403/400/404/410/409/500 as `{"error": msg}`; `IdentityConflict` adds `works`; delete conflict is 409 `rev conflict` + `server` | Exact shapes Liseur parses | `src/lib.rs:48-66,1671-1694,1442-1482` |
| Bodies capped at 16 KiB, token names at 256 B, batches bounded, JSON rejection → 400 `invalid JSON body` | Abuse limits without touching semantics | `src/lib.rs:36-37,994-998,1503-1670`; test `validates_protocol_boundaries` |
| Download goes through `tower_http::ServeFile::try_call` then overrides media type and sanitized attachment filename; missing file → 410 | Keeps `Range`/conditional semantics `LiseurSyncFileSource.kt` relies on | `src/lib.rs:927-960` |
| Attachments are side objects keyed by annotation id: `PUT /v1/annotations/{id}/attachments/{kind}`, `GET .../attachments`. Storing one never touches the record's `rev`/`seq` | A markup SVG plus a page snapshot cannot fit `body` (16 KiB), and bending the CAS contract around a binary would break every client that pinned the OpenAPI | `src/lib.rs` `ATTACHMENT_KINDS`, `put_attachment`, `list_attachments`; `apps/server/src/routers/liseur_sync/attachments.rs` |
| The digest is the idempotency key: the crate hashes the body and compares it to `X-Attachment-Sha256` (409 on mismatch); a repeat of the same digest returns the same attachment id | Every backend then reports the same failure for a corrupted transfer, and a device replaying a batch on its backup URL costs one request | `src/lib.rs` `read_attachment`; api test `upload_is_idempotent_by_digest_and_never_moves_the_annotation_revision` |
| `Content-Length` over `attachment_max_bytes` is refused before the body streams; a chunked body is bounded by `axum::body::to_bytes(body, max)`. `DEFAULT_ATTACHMENT_MAX_BYTES` is 8 MiB, overridable through `LiseurSyncBackend::attachment_max_bytes` | The limit is a host policy (`STUMP_ATTACHMENT_MAX_BYTES`), but the crate must not buffer an unbounded body to discover it | `src/lib.rs` `read_attachment`; api test `uploads_are_rejected_on_digest_mismatch_size_kind_media_type_and_identity` |
| An annotation carrying attachments needs an id of `[A-Za-z0-9._-]` not starting with a dot; the annotation lane still accepts any bounded id | The id names the storage directory, and `Path<String>` percent-decodes, so `ks1%2Fescape` would otherwise leave the attachment root | `src/lib.rs` `validate_attachment_annotation_id` |
| Cover URLs built from `X-Forwarded-Proto` + `Host`; cover response sets `X-Content-Type-Options: nosniff` | Proxy-correct absolute links | `src/lib.rs:764-790,900-924` |
| Catalog trait methods default to `NotFound("catalog unavailable")`; sync/token/annotation methods are required | A host may ship sync without a catalog; Stump overrides all 22 | `src/lib.rs:533-665`; `apps/server/src/routers/liseur_sync/mod.rs:33-229` |
| `ANY /v1/entities/series/{id}/{order,name}` → 404 `catalog entity route is not implemented` | Explicit deferral instead of silent wildcard | `src/lib.rs:699-700,988-991` |
| Ops idempotent by `op_id`; annotations CAS on `base_rev` with tombstones; separate per-user position and annotation sequences | Upstream conflict model; a stale rev is a conflict, not an overwrite | `apps/server/src/routers/liseur_sync/storage.rs:1229-1321,1392-1651,1773-2058` |
| `ANNOTATION_COLUMNS` selects `annotation_id AS id` (table PK is `row_id`) | Fix for the `no such column: id` 500 on `/v1/works/{id}/annotations` | `storage.rs:1673-1676,1994-1997`; regression test `work_annotations_reads_annotation_id_column` (`storage.rs:2085`) |
| Deferred: `/v1/register`, token listing/scope update, `/v1/insights/*`, upload/delete, series claims, retention sweeps, local annotation projection | Not needed by the pinned client's verified flows | `docs/content/docs/developer/liseur-sync-integration.mdx:269-294` |

Routes (`src/lib.rs:677-706`):

| Group | Routes |
| --- | --- |
| Public | `POST /v1/login` |
| Tokens | `GET /v1/token`, `POST /v1/tokens`, `DELETE /v1/tokens/{id}` |
| Identity / positions | `POST /v1/works/resolve`, `POST /v1/ops`, `GET /v1/changes`, `GET /v1/heads`, `GET /v1/works/{id}/positions`, `POST /v1/sessions` |
| Annotations | `POST /v1/annotations`, `GET /v1/annotations/changes`, `DELETE /v1/annotations/{id}?rev=`, `GET /v1/works/{id}/annotations` |
| Catalog (read-only) | `GET /v1/folders`, `/v1/folders/{f}/books`, `/v1/folders/{f}/search`, `/v1/books/{id}`, `/cover`, `/download`, `/series`; `POST /v1/books/{id}/resolve` |
| Deferred | `ANY /v1/entities/series/{id}/order`, `.../name` → 404 |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` (single file) | scopes/limits; `LiseurSyncError`; token kinds; public DTOs (`LoginResult`, `TokenCreateRequest`, `Identifier`, `ResolveRequest/Result`, `OpInput/Record/Result`, `ChangesPage`, `HeadsPage`, `SessionInput`, `AnnotationInput/Record/Result`, `Catalog*`); `LiseurSyncBackend`; `routes()`; `token_auth`; handlers; validators; `error_response`; tests |
| `apps/server/src/routers/liseur_sync/mod.rs` | `Backend` adapter over `AppState`, `mount()`, one-to-one trait forwarding |
| `apps/server/src/routers/liseur_sync/storage.rs` | SeaORM persistence: credentials/tokens, catalog, work identity, ops, sessions, annotations |

## How to verify

```text
cargo test -p stump_liseur_sync --lib --tests      # 8 tests (5 sync + 3 tokio; see Decisions table)
cargo test -p stump_server --lib liseur_sync       # adapter tests incl. annotation_id regression
cargo build -p stump_server --no-default-features --features headless,liseur-sync
cargo check -p stump_server --no-default-features --features minimal   # feature-off (routes absent)
cd ../komga-compat && make replay-liseur-sync BASE_URL=http://127.0.0.1:25600 USERNAME=… PASSWORD=…
```

Live probe: `POST /v1/login` with Stump credentials → 200 + session token;
`POST /v1/tokens` with `{"scopes":["sync","library-read"]}` → device token;
`GET /v1/folders` with that bearer → 200. Base `http://127.0.0.1:25600`,
credentials from the fixture's generated `ENDPOINTS.md`.

## Deep docs

- `docs/content/docs/developer/liseur-sync-integration.mdx` — wire contract, catalog, annotation bridge, auth, deviations, deferred list.
- `docs/content/docs/developer/liseur-providers.mdx` (native REST section) — every Liseur request against this surface, pinned to `31f8182d`.
- `docs/content/docs/developer/unified-reading-state.mdx` (liseur-sync projection) — position ops, sessions, annotation CAS in the unified model.
- `docs/content/docs/developer/standards.mdx` (liseur-sync) and `provider-status.mdx` — summary tables (provider-status catalog row is stale: catalog is read-only implemented).
- `docs/content/docs/developer/clients.mdx`, `client-verification.mdx` — Liseur liseur-sync rows.
