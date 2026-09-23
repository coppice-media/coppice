# Coppice OMP Context

Coppice is a Rust-first, self-hosted comics/manga/digital-book server in a
Cargo and Bun 1.4.1 monorepo. Compatibility-sensitive crate, package, binary,
route, environment-key, and app identifiers retain upstream Stump names.
`plan.txt` is intent, not proof. Preserve normal/full behavior and mobile
compatibility.

## Routing

- Use `.omp/agents/stump-dev.md` for Rust/server/core lifecycle, persistence,
  feature boundaries, request-ledger backend, API/protocol compatibility, and
  upstream-ready implementation.
- Use `.omp/agents/coppice-web.md` for Bun workspace tooling, Home `/app`,
  Editor `/editor`, shared `stump-ui`, generated web clients, docs UI, and
  legacy React/Vite compatibility.
- Use global `scout` for read-only exploration and `hiring-manager` for
  onboarding or project-coverage audits.
- Load `.omp/PROJECT_STATE.md` and `.omp/NEXT_STEPS.md` on demand; do not inject
  volatile rollout state or the roadmap by default.
- OMP discovers this file and `.omp/RULES.md`; never add legacy configuration or
  copy deployed runtime/session state into the repository.

## Git authorization

- Before every commit or push, ask the user to confirm the exact branch, destination, and changes. General permission is not confirmation for a particular operation.
- For this repository, the only permitted push destination is the existing Coppice `origin` (`https://github.com/coppice-media/coppice.git`), and only branches under `coppice/*`. Never push to Stump upstream or any other repository under this authorization.
- Other Coppice-related repositories will be prepared and published separately; creating their GitHub repositories or pushing to them requires explicit, repository-specific confirmation first.
- Without confirmation, leave changes locally for the user. Never force-push, rewrite published history, change remotes without authorization, push secrets, or push directly to protected branches.

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
- Book requests are a metadata-backed ledger: users create intent and managers
  approve or reject it. Release search, acquisition, retries, and transport
  belong to a future independent authenticated provider sidecar; staged ingest
  remains the file-entry boundary.
- Home-library discovery and remote locations use the separately scoped Coppice
  source-worker role, not the acquisition sidecar or compute-worker authority.
  Catalog inventory, explicit verification/link/materialization, and native
  direct/tunnel range serving are contract-tested; matching, placement, caching,
  and replication remain planned. Follow
  `docs/content/docs/developer/remote-worker-libraries.mdx`.
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
- Root JavaScript tooling uses Bun 1.4.1 with one `bun.lock`. `apps/expo/` is a
  frozen compatibility snapshot outside the active workspace and gate; trace it
  for API breakage but do not add native mobile tooling.
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
v0.16.0 `bf5a4fd6fb0aca92a1f47c7feaf102202fd99d53`, and Grimmory main rather
than Komga OpenAPI alone.
Source-of-record docs:
`docs/content/docs/developer/komga-compat.mdx`,
`docs/content/docs/developer/kobo-sync-capabilities.mdx`,
`docs/content/docs/developer/kobo-device-database.mdx`,
`docs/content/docs/developer/unified-reading-state.mdx`,
`docs/content/docs/developer/liseur-sync-integration.mdx`,
`docs/content/docs/developer/liseur-providers.mdx`,
`docs/content/docs/developer/modular-ingest.mdx`,
`docs/content/docs/developer/integration-architecture.mdx`,
`docs/content/docs/developer/remote-worker-libraries.mdx`,
`docs/content/docs/guides/integrations/acquisition.mdx`, and
`docs/content/docs/developer/server-architecture.mdx`.
The sibling user-owned `../komga-compat/` uses `make replay`; Hurl 6.x
multi-value cookie assertions use `cookie "name[Attr]"`.
The fixture launcher is `scripts/dev-fixture-server.sh`; it regenerates
owner-only `ENDPOINTS.md` with mode 0600 and exports `STUMP_ENABLE_KOMGA`,
`ENABLE_KOBO_SYNC`, and `ENABLE_KOREADER_SYNC`; only the first key has the
`STUMP_` prefix.
