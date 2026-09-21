# Stump OMP Context

Stump is a Rust-first, self-hosted comics/manga/digital-book server in a Cargo
and Yarn monorepo: `apps/server` (Axum), `core` (`stump_core`), and persistence
contracts in `crates/models`, `crates/migrations`, and `crates/graphql`.
This checkout carries headless/modular work. `plan.txt` is intent, not proof.
Preserve normal/full behavior and mobile compatibility.

## Routing

- Use `.omp/agents/stump-dev.md` for Rust/server/core lifecycle, feature
  boundaries, API/protocol compatibility, and upstream-ready implementation.
- Use global `scout` for read-only exploration and `hiring-manager` for
  onboarding or project-coverage audits.
- Load `.omp/PROJECT_STATE.md` and `.omp/NEXT_STEPS.md` on demand; do not inject
  volatile rollout state or the roadmap by default.
- OMP discovers this file and `.omp/RULES.md`; never add legacy configuration or
  copy deployed runtime/session state into the repository.

## Source map

- Provider traits `KomgaBackend`, `OpdsBackend`, `KoboBackend`,
  `KoreaderBackend`, and `LiseurSyncBackend` live in `crates/komga`,
  `crates/opds`, `crates/kobo`, `crates/koreader`, and `crates/liseur-sync`;
  `crates/kepub` is Kobo conversion and has no `Backend` trait.
- Server adapters implement those traits in
  `apps/server/src/routers/komga_backend.rs`,
  `apps/server/src/routers/opds_backend.rs`/`apps/server/src/routers/opds_backend/`,
  `apps/server/src/routers/kobo_backend.rs`/`apps/server/src/routers/kobo_backend/`,
  `apps/server/src/routers/koreader_backend.rs`/`apps/server/src/routers/koreader_backend/`,
  `apps/server/src/routers/liseur_sync/` (Kobo includes
  `apps/server/src/routers/kobo_backend/kepub.rs`).
- Komga identity/settings are server-local in
  `apps/server/src/routers/komga/`; provider exports
  `stump_komga::routes::is_komga_path`, while auth uses broader
  `is_komga_basic_auth_path` for identity/alias routes.
- `apps/server/Cargo.toml` defines `minimal`, `headless` (GraphQL, OPDS, Readium, Kobo, KOReader, Komga, liseur-sync), and `full` (headless + webui).
- `apps/server/src/middleware/auth.rs` owns Basic caching, remember-me,
  Komga 401 cookie-clear stripping, and `basic_auth_accepted`.
- `home/` is the dark-first SvelteKit user surface mounted at `/app`; route
  pages live in `home/src/routes/(app)/`, area operations in
  `home/src/lib/graphql/`, and reader components in
  `home/src/lib/components/reader/`.
- `editor/` is the dark-first SvelteKit ingest surface mounted at `/editor`;
  its workflow routes are `drop`, `queue`, `rework`, `bulk`, and `library`.
  Both Svelte apps consume the source-linked `packages/stump-ui/`; theme
  tokens and the preset switcher are owned there, not duplicated per app.
- User/device security is owned by GraphQL plus
  `apps/server/src/middleware/auth.rs`: device credentials inherit the owning
  user's permissions and may only narrow access through `library_scope`.
  `/app/devices` is the supported device setup surface; raw API keys are an
  authenticated fallback, not a permission bypass.
- Client integrations also include the sibling checkouts
  `../koreader-stump/` (KOReader plugin) and `../nickelstump/` (Kobo
  NickelMenu client). Their host tests are client evidence only; physical
  reader verification remains a separate evidence tier.

## Evidence discipline

Record exact paths, symbols, routes, dependency declarations, and URLs/commits.
Use pinned Komelia `65f92fde`, `komga-client` 0.11.0 `74412a6e`, Liseur
`31f8182d`, and Grimmory main rather than Komga OpenAPI alone.
Source-of-record docs:
`docs/content/docs/developer/komga-compat.mdx`,
`docs/content/docs/developer/kobo-sync-capabilities.mdx`,
`docs/content/docs/developer/kobo-device-database.mdx`,
`docs/content/docs/developer/unified-reading-state.mdx`,
`docs/content/docs/developer/liseur-sync-integration.mdx`,
`docs/content/docs/developer/liseur-providers.mdx`,
`docs/content/docs/developer/modular-ingest.mdx`, and
`docs/content/docs/developer/server-architecture.mdx`.
The sibling user-owned `../komga-compat/` uses `make replay`; Hurl 6.x
multi-value cookie assertions use `cookie "name[Attr]"`.
The fixture launcher is `scripts/dev-fixture-server.sh`; it regenerates
owner-only `ENDPOINTS.md` with mode 0600 and exports `STUMP_ENABLE_KOMGA`,
`ENABLE_KOBO_SYNC`, and `ENABLE_KOREADER_SYNC`; only the first key has the
`STUMP_` prefix.
