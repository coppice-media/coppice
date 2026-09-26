# stump_opds

## Purpose

`stump_opds` is the OPDS 1.2 (+ Page Streaming Extension) and OPDS 2.0 route
contract: `ProviderHost` (proxy-aware origin for absolute links), v2
`BrowseParams`, the 35-method `OpdsBackend` trait (14 v1.2 + 21 v2.0), and the
two unprefixed Axum route trees (`v1_router`, `v2_router`). Handlers do protocol extraction and
dispatch only. The crate deliberately does **not** own the wire types (Atom/XML
and JSON feed models stay in `core/src/opds/{v1_2,v2_0}`), authentication
(mounted by `apps/server` with `auth_middleware` / `api_key_middleware`),
database or media access, or reading-session writes. It carries no SeaORM
dependency; its `dev-dependencies` do, because the audio acquisition tests
build `models` entity fixtures (see _How to verify_).

## Reference / upstream

| Reference                          | Pin                                                                                             | Used for                                                                                            |
| ---------------------------------- | ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| OPDS 1.2                           | <https://specs.opds.io/opds-1.2>                                                                | Atom catalog/acquisition feeds (`core/src/opds/v1_2/mod.rs:1-2`)                                    |
| OPDS Page Streaming Extension      | <https://github.com/anansi-project/opds-pse/blob/master/v1.2.md>                                | `pse:count`/page links on `/books/{id}/pages/{page}` (`core/src/opds/v1_2/link.rs`)                 |
| OPDS 2.0 draft                     | <https://drafts.opds.io/opds-2.0>                                                               | JSON catalog, groups, auth document (`core/src/opds/v2_0/mod.rs:1-3`)                               |
| Readium Web Publication / locator  | `application/vnd.readium.progression+json`                                                      | v2 `progression` GET/PUT body (`core/src/opds/v2_0/progression.rs`)                                 |
| Liseur (OPDS 1.2 client)           | [v0.19.0 `62ecb5a5`](https://github.com/chmouel/liseur/commit/62ecb5a5c9dd8eb4e7fa6d97ce50d1bddae0bcd6) | Source-only current client; OPDS 2.0 is unreachable because the app has no `application/opds+json` parser; older device evidence is listed in the canonical matrix |
| Readium Web Publication (link)     | <https://readium.org/webpub-manifest/schema/link.schema.json>                                   | `duration` in seconds on an audio link (`core/src/opds/v2_0/link.rs` `OPDSAudioLink`)               |
| Readium Web Publication (metadata) | <https://readium.org/webpub-manifest/schema/metadata.schema.json>                               | publication `duration` in seconds (`core/src/opds/v2_0/metadata.rs`)                                |
| W3C Media Fragments URI 1.0        | <https://www.w3.org/TR/media-frags/>                                                            | `#t=` chapter offsets on `toc` entries (`core/src/opds/v2_0/audio.rs`)                              |
| Atom `link@length`                 | <https://datatracker.ietf.org/doc/html/rfc4287#section-4.2.7.5>                                 | per-track byte size as v2 `properties.length` (`core/src/opds/v2_0/properties.rs`)                  |

Client/device evidence and current source pins are recorded in the canonical
[client verification matrix](../../docs/content/docs/developer/client-verification.mdx).

## Decisions

| Decision                                                                                                                                                                            | Why                                                                                                                                                                                                                                                      | Evidence                                                                                                                                                                    |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Feature-only gate (`opds` Cargo feature, in `headless`); no runtime switch                                                                                                          | OPDS is part of the base server contract; nothing to keep dormant                                                                                                                                                                                        | `apps/server/Cargo.toml` `opds = ["dep:stump_opds"]`; `apps/server/src/routers/mod.rs:89-92`                                                                                |
| Three mounts: `/opds/v1.2` (session/Basic), `/opds/{api_key}/v1.2` (key in path), `/opds/v2.0` (session/Basic); no v2 path-key alias                                                | v1 alias exists for readers that cannot send headers; v2 path-key contract undefined                                                                                                                                                                     | `apps/server/src/routers/opds_backend.rs:49-77`; `docs/content/docs/developer/standards.mdx:30`                                                                             |
| `ProviderHost` lives here, injected by the server's `HostExtractor` middleware                                                                                                      | Absolute links must honour `X-Forwarded-*`; the crate must not depend on server middleware                                                                                                                                                               | `src/lib.rs:15-33`; `apps/server/src/routers/opds_backend.rs:28-47`                                                                                                         |
| `BrowseParams` mirrors Readium metadata query names and spells its pagination out rather than flattening `OffsetPagination`                                                         | The filters are spec vocabulary; `Query` deserializes with `serde_urlencoded`, which hands a flattened struct string values, so `?page=1` — the very parameter the feed's own `next` link carries — was a 400 (`invalid type: string "1", expected u64`) | `src/lib.rs:46-85`; `src/lib.rs:794-843` (extractor tests)                                                                                                                  |
| Each v2 search group links to a mounted per-kind route (`/libraries/search`, `/series/search`, `/books/search`) and every group honours `page`/`page_size`                          | The mixed `/search` feed emitted those three self links already, but nothing mounted them, so a client following one got a 404; each group also capped at ten with no way to ask for more                                                                | `src/lib.rs:350-407`; `apps/server/src/routers/opds_backend/v2_0.rs` (`search`, `search_libraries`, `search_series`, `search_books`); `apps/server/tests/opds/v2/search.rs` |
| A feed emits `first`/`previous`/`next`/`last` only for pages other than the one being served, from one shared builder, and `self` names the page it is                              | `next` used to be unconditional, so the final page pointed at an empty feed a client could not tell from a dead route; the library list named an unmounted `/libraries/browse` and book feeds an unmounted `/books/catalog`                              | `apps/server/src/routers/opds_backend/v2_0.rs` (`pagination_links`, `paginated_feed_links`, `paginated_group_links`); `apps/server/tests/opds/v2/browse.rs`                 |
| v1.2 page GET writes progress only when `ENABLE_OPDS_PROGRESSION=true` (default `false`)                                                                                            | Readers preload pages; counting loads would corrupt progress                                                                                                                                                                                             | `apps/server/src/routers/opds_backend/v1_2.rs:788-830`; `core/src/config/protocols.rs:64-66`                                                                                |
| v2 `PUT /books/{id}/progression` is explicit: 204, 409 on stale `modified`, 400 on out-of-range page; unaffected by the switch                                                      | Timestamp-conflict rule shared with Komga Readium and liseur-sync                                                                                                                                                                                        | `apps/server/src/routers/opds_backend/v2_0.rs:1261-1363`; `docs/content/docs/developer/unified-reading-state.mdx:352-357`                                                   |
| `GET /opds/v2.0/auth` is public and advertises Basic                                                                                                                                | OPDS 2 authentication-document flow                                                                                                                                                                                                                      | `core/src/opds/v2_0/authentication.rs:31-43`                                                                                                                                |
| Facets, EPUB chapter/resource streaming, v1 strict mode not implemented                                                                                                             | Open TODOs in core/adapter; not needed by any verified client                                                                                                                                                                                            | `core/src/opds/v2_0/mod.rs:20`; `apps/server/src/routers/opds_backend/v2_0.rs:1225-1227`; `v1_2.rs:731-736`                                                                 |
| Errors are the server's `APIError` (`{status,message}`, `no-store`); DB not-found → 404, other DB → 500                                                                             | Same shape as native routes                                                                                                                                                                                                                              | `apps/server/src/routers/opds_backend.rs:80`; `apps/server/src/errors.rs:140-183`                                                                                           |
| An audio acquisition advertises the MIME `stump_media`'s `ContentType` gives it; no second extension table                                                                          | v1.2 typed every audio container `application/zip`, because `OpdsLinkType::from_extension` did not know one; `ContentType` already maps all six containers and v2 already routed through it                                                              | `core/src/opds/v1_2/link.rs` (`OpdsLinkType::Audio(ContentType)`, `mime()`); `core/src/opds/v2_0/publication.rs` `links_for_book`                                           |
| A multi-track audiobook's v2 acquisition is one link per track at `/api/v2/media/{id}/audio/track/{index}`, replacing the single `/file` link; a single-container one keeps `/file` | A folder book's `media.path` is a directory, so `/file` can serve nothing; the native track route answers ranges. Order is `media_audio_tracks.index`, and MIME and byte size are the probe's verdict, used verbatim                                     | `core/src/opds/v2_0/audio.rs`; `crates/opds/tests/audio_acquisition.rs`                                                                                                     |
| v2 states `duration` in seconds and publishes chapters as `toc` entries; no chapter-count key was invented                                                                          | Readium states every duration in seconds and has no chapter-count field — the length of `toc` _is_ the count, and `toc` is where a seekable mark belongs                                                                                                 | `core/src/opds/v2_0/metadata.rs` (`with_duration_ms`), `audio.rs` (`toc`)                                                                                                   |
| v1.2 gets the acquisition MIME only: no per-track XML entries, no `toc`                                                                                                             | OPDS 1.2 is page-oriented, its entry builder is synchronous with no database access, and the format carries no track list; a folder audiobook is therefore listed but not acquirable from 1.2                                                            | `core/src/opds/v1_2/entry.rs` (`entry_file_acquisition_link_type`); `docs/content/docs/guides/features/opds.mdx`                                                            |
| OPDS does not carry `readingOrder` for an audiobook                                                                                                                                 | `GET /api/v2/media/{id}/audio/manifest` is the Readium manifest a player consumes; OPDS answers "how do I get the bytes"                                                                                                                                 | `core/src/opds/v2_0/audio.rs` module docs                                                                                                                                   |

Route trees (`src/lib.rs:307-339`, `350-407`):

| v1.2 (`/opds/v1.2`, `/opds/{api_key}/v1.2`)                                            | v2.0 (`/opds/v2.0`)                                                                                                  |
| -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `GET /catalog`, `/search`, `/search/feed`, `/keep-reading`                             | `GET /auth`, `/catalog`, `/search`                                                                                   |
| `GET /libraries/`, `/libraries/{id}`                                                   | `GET /libraries/`, `/libraries/search`, `/libraries/{id}/`, `/libraries/{id}/books/`, `/libraries/{id}/books/latest` |
| `GET /series/`, `/series/latest`, `/series/{id}`                                       | `GET /series/`, `/series/search`, `/series/{id}/`                                                                    |
| `GET /books/`, `/books/latest`                                                         | `GET /books/browse`, `/books/search`, `/books/latest`, `/books/keep-reading`, `/books/{id}/`                         |
| `GET /books/{id}/thumbnail`, `/books/{id}/pages/{page}`, `/books/{id}/file/{filename}` | `GET /books/{id}/thumbnail`, `/books/{id}/pages/{page}`, `GET+PUT /books/{id}/progression`, `GET /books/{id}/file`   |

## Layout

| File                                           | Responsibility                                                                                                              |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`                                   | `ProviderHost`, `BrowseParams`, `OpdsBackend`, `v1_routes`/`v2_routes`, `v1_router`/`v2_router`, extractor→backend wrappers |
| `apps/server/src/routers/opds_backend.rs`      | host injection, three mounts, `OpdsBackendImpl` forwarding to `v1_2`/`v2_0` handlers                                        |
| `apps/server/src/routers/opds_backend/v1_2.rs` | XML catalog/search/navigation, pagination, thumbnails, pages (optional progress side effect), acquisition                   |
| `apps/server/src/routers/opds_backend/v2_0.rs` | auth document, JSON catalog/groups/browse, pages, progression GET/PUT, acquisition                                          |
| `core/src/opds/v1_2/`                          | Atom feed/entry/link/author/OpenSearch serializers                                                                          |
| `core/src/opds/v2_0/`                          | authentication, feed, group, link, metadata, publication, progression, `OPDSV2Error`                                        |
| `core/src/opds/v2_0/audio.rs`                  | per-track acquisition links, chapter `toc`, batched `media_audio*` loads                                                    |

## How to verify

```text
cargo test -p stump_opds                              # 3 browse-extractor + 7 audio acquisition tests
cargo test -p stump_server --no-default-features --features headless,liseur-sync --test api_tests opds
                                                      # 18: 7 browse + 5 search + 2 progression + 3 keep-reading + 1 v1.2 library scope
cargo check -p stump_server --no-default-features --features minimal   # feature-off build (both trees absent)
```

Live probe against the fixture (`scripts/dev-fixture-server.sh`, base
`http://127.0.0.1:25600`, credentials from the generated `ENDPOINTS.md`):
`GET /opds/v1.2/catalog` with Basic → 200 `application/atom+xml`;
`GET /opds/v2.0/auth` unauthenticated → 200 JSON; `GET /opds/v2.0/catalog`
with Basic → 200 `application/opds+json`. Device evidence is the Liseur OPDS 1.2
row in `client-verification.mdx`.

## Deep docs

- `docs/content/docs/developer/standards.mdx` (OPDS 1.2 / OPDS 2.0 tables) — one-screen contract summary.
- `docs/content/docs/developer/provider-status.mdx` (OPDS section) — status per feature, auth variants, TODOs.
- `docs/content/docs/developer/server-architecture.mdx` — feature profiles and `ENABLE_OPDS_PROGRESSION` default.
- `docs/content/docs/developer/unified-reading-state.mdx` (OPDS projection) — progression mapping and conflict rule.
- `docs/content/docs/developer/clients.mdx`, `client-verification.mdx` — Liseur OPDS rows and generic-reader claims.
- `docs/content/docs/guides/features/opds.mdx` — user-facing URLs and reader compatibility table.
