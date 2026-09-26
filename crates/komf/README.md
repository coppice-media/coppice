# Komf

## Purpose

`stump_komf` owns the Komf 2.0.1 HTTP DTO and route contract consumed by Komelia
0.19.3. The server adapter supplies Coppice-native provider search, matching,
persistence, library visibility, and write behavior; this crate does not own a
metadata shadow store or acquisition transport.

## Reference / upstream

| Source | Pin |
| --- | --- |
| Komf HTTP API | [`5ad67370c5da6d94372e7c51bff62dd75053ac64`](https://github.com/Snd-R/komf/commit/5ad67370c5da6d94372e7c51bff62dd75053ac64) |
| Komelia client calls | [`3c5f501ef24b3acc239302adcab28efe0772c513`](https://github.com/Snd-R/komelia/commit/3c5f501ef24b3acc239302adcab28efe0772c513), bundled `komf-client` 2.0.0 |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Keep the Komf API contract separate from the Komga media-server adapter; Coppice mounts Komf's root config/jobs and `/api/komga` routes only when the `komf` feature and runtime switch are enabled | The third-party client contract and media-server contract have different route and DTO ownership | `crates/komf/src/routes.rs:103-138`; `apps/server/src/routers/mod.rs:205-208` |
| Accept query `apiKey` only on registered Komf routes and require a device-bound credential; the server request span redacts its value | Komelia stores a base URL with the device key as a query parameter; widening the lane would change other clients' auth surface | `apps/server/src/middleware/auth.rs:130-199`; `apps/server/src/http_server.rs:28-66` |
| Reads need authentication and device library scope, not provider-management permissions; identify/match/reset/jobs deletion require `EditMetadata` | Komelia credentials are download-only unless the owner explicitly grants metadata editing | `apps/server/src/routers/komf_backend.rs:129-165,774-899,912-954`; `crates/devices/src/credential.rs:137-161` |
| `PATCH /api/config` retains the native `MetadataProviderManage` guard, and config reads return secret fields as `null` | Komf configuration writes alter native provider credentials/settings; these are not part of the device opt-in | `apps/server/src/routers/komf_backend.rs:902-910,1276-1343`; `crates/graphql/src/mutation/metadata_provider.rs:28,44,66,87` |
| Keep jobs process-local, provider matching native-only, and `removeComicInfo=true` unsupported (`422`) | Do not create a parallel durable-job system, downloader, or archive mutation path for this adapter | `apps/server/src/routers/komf_backend.rs:430-452,864-900,965-1159`; `crates/integrations/metadata` |

## Layout

| Path | Contents |
| --- | --- |
| `src/lib.rs` | Public Komf HTTP contract and router exports |
| `src/routes.rs` | Axum route tree, request handling, SSE framing |
| `src/types.rs` | Komf DTOs, errors, and captured response fixture checks |
| `tests/fixtures/komf-responses.json` | Pinned response-shape fixtures used by crate and server route tests |

## How to verify

`cargo test -p stump_komf`

Server-route and permission coverage:

`cargo test -p stump_server --no-default-features --features headless,liseur-sync --test api_tests komf`

## Deep docs

- `docs/content/docs/developer/komga-compat.mdx`
- `scripts/contracts/komf-media-server.json`
