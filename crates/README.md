# Workspace crates

Every crate ships a `README.md` in the template below and a `//!` crate doc in
its `lib.rs` pointing at it. Status reflects the working tree on 2026-09-05
(branch `headless-modular`); "in flight" means the directory is untracked or
its `lib.rs` has uncommitted changes and its README is owned by the worker
editing it.

| Crate dir | Package | Purpose | Feature flag (server / core) | Status |
| --- | --- | --- | --- | --- |
| `api-types` | `stump_api_types` | Transport-neutral `RequestOrigin` URL building and `OffsetPagination` | always linked | README done |
| `auth` | `stump_auth` | `AuthContext` + `AuthorizationError`, permission/owner enforcement | always linked | README done |
| `cli` | `cli` | Server CLI subcommands (account, config) embedded in `stump_server` | always linked | README pending (dirty: `commands/account.rs`, `config.rs`) |
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
| `liseur-sync` | `stump_liseur_sync` | Native liseur-sync wire contract and `LiseurSyncBackend` trait | server `liseur-sync` | README by `ReadmeProtocols` |
| `macros/filter-gen` | `filter-gen` | Proc macro generating filter/ordering enums for entities | always linked (`models`, `graphql`) | README pending |
| `macros/stump-config-gen` | `stump-config-gen` | Proc macro generating `StumpConfig` env/partial-config impls | always linked (`stump_core`) | in flight (dirty `lib.rs`; `ConfigSplit`) — skipped |
| `media` | `stump_media` | File/image processing, archive formats, thumbnails | `pdf` (PDFium), `rar` (unrar) via server `formats` | README by `ReadmeMediaKepub` |
| `migrations` | `migrations` | Append-only SeaORM schema migrations, `migrate` CLI | always linked; `cli` feature off in server graph | README done |
| `models` | `models` | SeaORM entities, stored value types, shared DB services | always linked; `graphql` derives opt-in | README done |
| `opds` | `stump_opds` | OPDS 1.2/2.0 catalog routes and `OpdsBackend` trait | server `opds` | README by `ReadmeProtocols` |
| `scanner` | `stump_scanner` | Filesystem scan planning primitives | always linked via `stump_core` | README pending |
| `tests` | `tests` | Shared test DB/fake-data helpers for integration tests | dev only | README pending (dirty: `src/db.rs`) |
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
