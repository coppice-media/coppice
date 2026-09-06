<p align="center">
  <img alt="Stump's logo. It depicts a young individual sitting on a tree stump reading a book. Inspired by the developer's childhood, where they spent a significant amount of time reading on a tree stump in their backyard" src="./.github/images/logo.png" style="width: 30%" />
  <br />
  <a href="https://github.com/stumpapp/stump/blob/main/LICENSE">
    <img src="https://img.shields.io/static/v1?label=License&message=MIT&color=CF9977" />
  </a>
</p>

## What this fork is

This is a fork of [stumpapp/stump](https://github.com/stumpapp/stump) (nightly branch) focused on **headless, modular, protocol-compatible** operation. It is still one Rust server process with one database and the same authentication system: `stump_server` adapts HTTP/WebSocket requests with Axum, `stump_core` owns orchestration and filesystem/job behavior, and `crates/models` owns entities, domain types, and services. The web UI is a separately built Vite application whose static assets may be served by the same process — but are not required. See [Server architecture](docs/content/docs/developer/server-architecture.mdx) for the full source map.

This is a fork for personal/protocol-compatibility work. It is not a claim of upstream support; consult [stumpapp/stump](https://github.com/stumpapp/stump) for the upstream project.

## What works today

Every claim below traces to the linked page; verification levels (device-verified vs. harness-only) are defined in [Client verification](docs/content/docs/developer/client-verification.mdx).

- **Komga compatibility adapter** (`/komga/api/*`), used by Komelia, Grimmory, and Mihon/Tachiyomi — [Komga compatibility](docs/content/docs/developer/komga-compat.mdx)
- **OPDS 1.2** (with OPDS-PSE) and **OPDS 2.0** (including progression) — [Provider status](docs/content/docs/developer/provider-status.mdx)
- **KOReader sync** (`/koreader/{api_key}`) — [Provider status](docs/content/docs/developer/provider-status.mdx)
- **Kobo sync** (`/kobo/{api_key}`) with optional KEPUB conversion — [Kobo sync capabilities](docs/content/docs/developer/kobo-sync-capabilities.mdx)
- **Native liseur-sync** (`/v1/*` sync routes) — [liseur-sync integration](docs/content/docs/developer/liseur-sync-integration.mdx)
- **Readium EPUB web publication** routes (`/api/v2/epub/*`) — [Server architecture](docs/content/docs/developer/server-architecture.mdx)
- **Unified reading state** across protocols — [Unified reading state](docs/content/docs/developer/unified-reading-state.mdx)
- **Staged ingest** with drop folders, immutable staging, and progress events — [Modular ingest](docs/content/docs/developer/modular-ingest.mdx)

Cross-app support matrices and verification levels: [Clients](docs/content/docs/developer/clients.mdx) · [Standards/protocols](docs/content/docs/developer/standards.mdx) · [Sync platforms](docs/content/docs/developer/sync-platforms.mdx).

## Build profiles

The server Cargo graph exposes three aggregates: default `full` = `headless + webui`; `headless` contains every backend route feature (including the optional `liseur-sync` provider); the no-default `minimal` composition keeps REST/auth/database and omits optional protocol route trees. `formats = ["pdf", "rar"]` is part of every profile so defaults stay behavior-identical. Commands from [Server architecture](docs/content/docs/developer/server-architecture.mdx):

```sh
# full/default (compatibility baseline; compiles SPA routes)
cargo run --package stump_server --bin stump_server

# headless: every backend route capability, no SPA routes
cargo run --package stump_server --no-default-features --features headless

# minimal: REST/auth/database route profile only
cargo run --package stump_server --no-default-features

# release build (needs a usable PDFium library: PDFIUM_PATH or system lib)
yarn web build && cargo build --package stump_server --release
```

Docker has two final targets:

```sh
docker buildx build -f docker/Dockerfile --target full .
docker buildx build -f docker/Dockerfile --target headless .
```

## Runtime switches

Any compiled profile can be tuned without rebuilding (from [Server configuration](docs/content/docs/guides/configuration/server-config.mdx)):

| Variable | Default | Effect |
| --- | --- | --- |
| `STUMP_ENABLE_WEBUI` | `true` | Serves the web UI/SPA. Only effective when compiled with the `webui` feature; a headless build cannot re-enable it. |
| `STUMP_ENABLE_BACKGROUND_JOBS` | `true` | Scheduled jobs, library watcher, job executor. `false` keeps HTTP/database up; job/watcher calls return an explicit disabled error. |
| `STUMP_ENABLE_KOMGA` | `false` (`true` in debug builds) | Mounts the Komga compatibility routes (requires the `komga` Cargo feature). |
| `STUMP_ENABLE_UPLOAD` | `false` | Enables the file upload interface. |
| `ENABLE_KOBO_SYNC` | `false` in release (debug differs) | Mounts Kobo sync routes. |
| `ENABLE_KOREADER_SYNC` | `false` in release (debug differs) | Mounts KOReader sync routes. |
| `ENABLE_OPDS_PROGRESSION` | `false` | OPDS page access updates reading progress. |
| `KOBO_KEPUB_CONVERSION` | `false` | Converts Kobo EPUB downloads to KEPUB with pagination wrappers and `koboSpan` anchors. |
| `KOBO_KEPUB_PRECONVERT` | `false` | Warms the KEPUB cache after each library scan (needs `KOBO_KEPUB_CONVERSION`). |
| `KOBO_KEPUB_CACHE_MAX_AGE_DAYS` | `90` | Retention for unused KEPUB cache files. |
| `KOBO_KEPUB_DEFLATE_LEVEL` | `6` (1–12) | libdeflate compression level; part of the cache key. |
| `INGEST_DROP_DIR` | `<config dir>/ingest/drop` | Per-library drop folders scanned via `scanIngestDropFolder`. |
| `INGEST_STAGING_DIR` | `<config dir>/ingest/staging` | Immutable staged ingest files (`<sha256>-<filename>`); rejects go to `rejected/`. |
| `INGEST_PROGRESS_RETENTION` | — | Typed ingest progress events retained for `ingestProgress` and `/api/v2/ingest/events` SSE replay. |
| `INGEST_EDITOR_DIR` | — | Serves a built ingest editor under `/editor` when it contains `index.html`. |
| `STUMP_HOME_APP_DIR` | — | Serves the built Home app (devices, reading, account) under `/app` when it contains `index.html`. |

## Quick start

`scripts/dev-fixture-server.sh` launches a headless debug build against a fixture library on the LAN — the same server used for device verification of Komelia, Grimmory, Mihon, and Kobo sync. The external `komga-compat` Hurl harness (`make replay`, `replay-negative-auth`, `replay-readium`, ...) replays protocol contracts against it. See [Client verification](docs/content/docs/developer/client-verification.mdx) for the levels, dates, and app versions behind each verdict.

## Crate map

| Crate | Role |
| --- | --- |
| `crates/media` (`stump_media`) | File/image processing: format processors (EPUB, ZIP, PDF behind `pdf`, RAR behind `rar`), content types, hashing, Readium manifests, thumbnails. |
| `crates/scanner` (`stump_scanner`) | Library scanning and the shared directory-mtime snapshot walker. |
| `crates/jobs` (`stump_jobs`) | Job-type-agnostic queue, executor (Apalis behind the `apalis` feature, inline blocking-pool otherwise), `JobLifecycle`/`JobContext` contracts, queue counters, and cron scheduler. |
| `crates/komga` (`stump_komga`) | Komga compatibility DTOs, pagination, and error mapping. |
| `crates/kobo` (`stump_kobo`) | Kobo backend traits and contracts. |
| `crates/kepub` | KEPUB conversion (Kobo pagination wrappers, `koboSpan` anchors); conversion-only. |
| `crates/koreader` (`stump_koreader`) | KOReader sync backend traits and hash contracts. |
| `crates/opds` (`stump_opds`) | OPDS 1.2/2.0 backend traits. |
| `crates/liseur-sync` | Native liseur-sync provider traits. |
| `crates/auth` (`stump_auth`) | Authenticated user/session context and authorization errors. |
| `crates/api-types` (`stump_api_types`) | Transport-neutral request-origin URL construction and offset pagination. |
| `crates/graphql` | GraphQL schema construction, input wrappers, and resolver errors. |
| `crates/models` | Entities, domain types, and persistence-facing services. |

## Project state and roadmap

- [Project state](.omp/PROJECT_STATE.md) — branch, source map, and current coordination state.
- [Next steps](.omp/NEXT_STEPS.md) — the working roadmap.
- [Roadmap docs](docs/content/docs/developer/) — including proposed (not implemented) work in [Server architecture](docs/content/docs/developer/server-architecture.mdx) and [Modular ingest](docs/content/docs/developer/modular-ingest.mdx).

## Upstream relationship

This repository is a fork of [stumpapp/stump](https://github.com/stumpapp/stump), tracking the nightly branch. Changes here are made to be narrowly scoped and upstreamable, but they are not reviewed or endorsed by upstream. For the upstream project, its documentation, and its installation guides, see [stumpapp.dev](https://www.stumpapp.dev).

## License

> If a package or subfolder has its own license file, that license takes precedence over the repository-level license and will be listed below.

- The [expo application](./apps/expo/LICENSE) is licensed under [GPL-3.0](https://www.gnu.org/licenses/gpl-3.0.html)
- All other code in the repository is licensed under [MIT License](https://www.tldrlegal.com/license/mit-license)

## Attribution

This fork inherits the upstream Stump project and the work of its contributors:

- Some of the icons used in the web and mobile applications are from the [Spacedrive](https://github.com/spacedriveapp/spacedrive/tree/main/packages/assets/icons) repository, and are licensed under the [FSL-1.1-ALv2](https://github.com/spacedriveapp/spacedrive/blob/main/LICENSE) license.
- The native Readium expo modules were adapted from [Storyteller](https://gitlab.com/storyteller-platform/storyteller)
