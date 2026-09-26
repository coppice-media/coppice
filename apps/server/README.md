# Server

The `stump_server` binary composes the HTTP API and compatibility protocols with the built Home and Editor apps.

## Browser routes

- `STUMP_HOME_APP_DIR` optionally serves the Home app at `/app`.
- `INGEST_EDITOR_DIR` optionally serves the ingest Editor at `/editor`.
- With the `webui` Cargo feature and `STUMP_ENABLE_WEBUI=true`, `/` and unknown non-API browser paths redirect to `/app`.
- Native API and protocol routes resolve before browser redirects. Unknown API, OPDS, public, and KOReader paths remain not-found responses.

The `webui` Cargo feature remains the build gate for web UI routing and GraphQL playground behavior; the Home and Editor assets are independently configured runtime directories.

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Compile Komf into `headless`/`full`, leave `minimal` without it, and keep mounting behind the default-off `STUMP_ENABLE_KOMF` runtime switch | Komf is a dormant compatibility route group unless explicitly enabled | `apps/server/Cargo.toml:8-10,30,36-37`; `core/src/config/protocols.rs:55-61`; `apps/server/src/routers/mod.rs:205-208` |
| Accept the `apiKey` query only on exact Komf route shapes, bind it to a device, and redact its value from request spans | Komelia stores a Komf base URL with its device key in the query; auth must not widen to Kavita or native `/api` routes | `apps/server/src/middleware/auth.rs:130-199`; `apps/server/src/http_server.rs:28-66` |
| Leave reads to authenticated device scope, require `EditMetadata` for metadata/job writes, and preserve `MetadataProviderManage` for config patches | Device keys default to download-only, while native provider configuration remains separately privileged | `apps/server/src/routers/komf_backend.rs:774-954`; `crates/graphql/src/mutation/metadata_provider.rs:28,44,66,87` |
| The Liseur adapter reconciles linked native Readium highlights, notes, and bookmarks into stable CAS records, and writes CAS edits/deletes back to their source rows | One user-scoped annotation stream reaches every reader without re-exporting either mirror or violating library visibility | `src/routers/liseur_sync/storage.rs` `reconcile_native_annotations`; `crates/graphql/src/mutation/epub.rs`; focused native annotation tests |
| `/v1/books/{id}/resolve` keeps the existing user/media binding, but self-heals it into the single alias-backed work when the linked work is alias-less and pairing-evidenced; multiple strong-alias works still produce 409 | Source, KoReader partial-MD5, SHA-256 edition and server-recorded file hashes converge without weakening genuine conflicts; runtime and append-only repair share the same data-preserving merge | `src/routers/liseur_sync/storage.rs::resolve_catalog_book`; `tests/liseur/mod.rs::linked_aliasless_pair_resolve_self_heals_and_preserves_identity_conflicts`; migration 20260963/20260966 |
| Include `mam-acquisition` in `headless`/`full` while keeping the runtime switch off by default; `minimal` omits it | Manager-only MAM bridge operations require the server API and staged ingest, but an unconfigured deployment must not contact the bridge | `apps/server/Cargo.toml`; `core/src/config/mam_acquisition.rs`; `apps/server/src/routers/mod.rs` |
| `get_media_thumbnail` never serves a raw page when it can encode one: with no stored and no on-disk thumbnail it calls `generate_thumbnail_on_demand` with the library's `thumbnail_config` (or the media crate's WebP 400×600 default) and the result is kept under `thumbnails/`; series/library fallbacks, Komga, Kavita, Kobo, OPDS 2.0, ABS, Liseur and the OPDS 1.2 `/books/{id}/thumbnail` route (previously a raw page 1 extraction of its own) all go through it | On the fixture no library has a `thumbnail_config`, so every cover on every profile's grid was a fresh 1–2 MB page extraction; a bounded file generated once and looked up by the existing `get_thumbnail` name is a plain read afterwards, while a configured library still gets exactly its configured format and size | `src/routers/api/v2/media.rs::get_media_thumbnail` + tests `unconfigured_library_gets_a_kept_bounded_thumbnail_not_the_page`, `configured_library_keeps_its_own_format`; `src/routers/opds_backend/v1_2.rs::get_book_thumbnail`; `crates/media/README.md` |

OPDS 2.0 authentication documents retain the Help and authentication-document links; no logo link is emitted.
