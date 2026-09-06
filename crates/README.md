# Workspace crates

Every crate ships a `README.md` in the template below and a `//!` crate doc in
its `lib.rs` pointing at it. Status reflects the working tree on 2026-09-05
(branch `headless-modular`); "in flight" means the directory is untracked or
its `lib.rs` has uncommitted changes and its README is owned by the worker
editing it.

| Crate dir | Package | Purpose | Feature flag (server / core) | Status |
| --- | --- | --- | --- | --- |
| `annotation-sync` | `stump_annotation_sync` | Canonical annotation export model, `Sink` trait, markdown (Obsidian) and git sinks | always linked via `stump_core`; crate feature `git` (default) links libgit2 | README done |
| `api-types` | `stump_api_types` | Transport-neutral `RequestOrigin` URL building and `OffsetPagination` | always linked | README done |
| `auth` | `stump_auth` | `AuthContext` + `AuthorizationError`, permission/owner enforcement | always linked | README done |
| `cli` | `cli` | Server CLI subcommands (account, config, `tools list`/`plan`/`apply`) embedded in `stump_server` | always linked | README pending (dirty: `commands/account.rs`, `config.rs`, `commands/tools.rs`, `bin/main.rs`) |
| `devices` | `stump_devices` | Unified device registry: per-device credentials, endpoints, last-seen/last-sync tracking | always linked via `stump_core`; `graphql` derives opt-in | README done |
| `email` | `email` | SMTP sender via `lettre`, emailer config | always linked; `graphql` derives opt-in (`stump_core/graphql`) | README pending |
| `graphql` | `graphql` | async-graphql schema, guards, loaders, `graphql-gen` | server `graphql` (in `headless`/`full`, not `minimal`) | README done |
| `integrations/metadata` | `metadata_integrations` | Metadata provider clients (Comic Vine, Hardcover, AniList, MAL, MangaDex, MangaUpdates), scoring, merge | always linked via `stump_core`; `graphql` derives opt-in | README done |
| `integrations/notification` | `integrations` | Discord/Telegram notifier clients | always linked via `stump_core` | README exists (legacy short form; needs template) |
| `jobs` | `stump_jobs` | Job-type-agnostic background job runtime, lifecycle contracts, cron scheduler | being wired into `stump_core` | in flight (untracked; `JobsCrate`) |
| `kavita` | `stump_kavita` | Serde contract types and routes for the client-derived Kavita compatibility API | planned server `kavita` feature | in flight (untracked; `KavitaWave1`) |
| `kepub` | `stump_kepub` | Pure-Rust EPUB → KEPUB conversion (kepubify parity) | server `kobo` (`dep:stump_kepub`) | README done (`ReadmeMediaKepub`) |
| `kobo` | `stump_kobo` | Kobo sync protocol routes and `KoboBackend` trait | server `kobo` | README by `ReadmeProtocols` |
| `komga` | `stump_komga` | Komga-compatible API surface and `KomgaBackend` trait | server `komga` (implies `readium`) | README by `ReadmeProtocols` |
| `koreader` | `stump_koreader` | KOReader sync API and `KoreaderBackend` trait | server `koreader` | README by `ReadmeProtocols` |
| `liseur-sync` | `stump_liseur_sync` | Native liseur-sync wire contract, annotation attachment lane, and `LiseurSyncBackend` trait | server `liseur-sync` | README by `ReadmeProtocols` |
| `macros/filter-gen` | `filter-gen` | Proc macro generating filter/ordering enums for entities | always linked (`models`, `graphql`) | README pending |
| `macros/stump-config-gen` | `stump-config-gen` | Proc macro generating `StumpConfig` env/partial-config impls | always linked (`stump_core`) | in flight (dirty `lib.rs`; `ConfigSplit`) — skipped |
| `media` | `stump_media` | File/image processing, archive formats, thumbnails, DRM/encryption detection (`drm`) | `pdf` (PDFium), `rar` (unrar) via server `formats` | README by `ReadmeMediaKepub` (Decisions/Layout rows for `src/drm.rs` by `CalibreAdapter`) |
| `migrations` | `migrations` | Append-only SeaORM schema migrations, `migrate` CLI | always linked; `cli` feature off in server graph | README done |
| `models` | `models` | SeaORM entities, stored value types, shared DB services (reading progress, annotation attachments) | always linked; `graphql` derives opt-in | README done |
| `opds` | `stump_opds` | OPDS 1.2/2.0 catalog routes and `OpdsBackend` trait | server `opds` | README by `ReadmeProtocols` |
| `notify` | `stump_notify` | Notification channels (ntfy, email, webhook), routing-rule resolution, retry policy | always linked via `stump_core`; `graphql` derives opt-in (`stump_core/graphql`) | README done |
| `provider` | `stump_provider` | Remote source host: `Source` trait, page cache, Keiyoushi catalog/health, materialisation, virtual-library browse, GC | server/core/graphql `providers` (in `headless`, not `minimal`) + runtime `STUMP_ENABLE_PROVIDERS` | README done |
| `provider-mangadex` | `stump_provider_mangadex` | MangaDex `Source` implementation (API + MangaDex@Home pages) | via `providers` | covered by `provider/README.md` |
| `scanner` | `stump_scanner` | Filesystem scan planning primitives | always linked via `stump_core` | README pending |
| `tests` | `tests` | Shared test DB/fake-data helpers for integration tests | dev only | README pending (dirty: `src/db.rs`) |
| `tools` | `stump_tools` | Library maintenance tools (Kavita/MangaManager "external tools" parity): the `Tool` plan/apply contract plus `calibre-convert`, `calibre-meta`, `calibre-polish`, `cbz-covers`, `cbzit`, `epub-check`, `epub2cbz`, `meta-edit`, `missing-sequence`, `webp-convert`, driven by `stump tools list`/`plan`/`apply` | always linked via `cli`; not in the server route graph | in flight (untracked; `ToolsCore` + per-tool workers) |
| `watcher` | `stump_watcher` | Library filesystem watching, debounced scan requests | core/server `watcher` (in `headless`, not `minimal`) | in flight (untracked; `WatcherCrate`) |

## README template

Each crate README uses exactly these sections, ≤ 120 lines, tables over prose:

1. **Purpose** — what it owns, what it deliberately does not.
2. **Reference / upstream** — what it derives from or must stay compatible with, with pinned commits/URLs.
3. **Decisions** — table `decision | why | evidence` (file:line, doc link, harness spec, measurement); include deviations from the reference.
4. **Layout** — module map (file → responsibility).
5. **How to verify** — exact commands: `cargo test -p <crate>`, feature-off checks, the `../komga-compat` replay target, live probe.
6. **Deep docs** — links to `docs/content/docs/developer/*.mdx` and local studies.

When a crate's behaviour changes, update its Decisions table in the same change
(see `.omp/RULES.md`).

One documented exception to the line budget: `crates/tools/README.md` carries a
`## Tools` section with one subsection per tool and one table row per option
(required by the `Tool` contract), so it grows with the tool count.
