# Stump ingest editor

A standalone Bun-managed SvelteKit 2 editor for the staged ingest workflow. It lives at the repository root (`editor/`) so it is not included in the Yarn workspace under `apps/*`.

## Development

```sh
cd editor
bun install
bun run dev
```

The Vite development server proxies `/api` (including `/api/graphql/ws`) to `http://127.0.0.1:25600`. Run the Stump fixture/server on that address, or change `server.proxy` in `vite.config.ts` for another backend. The editor uses the server's cookie session from `POST /api/v2/auth/login` and sends GraphQL requests to `/api/graphql`.

## Shared UI package

`src/lib/stump-ui` is a symlink to `../packages/stump-ui/src`, the UI
foundation shared with the Home app (`home/`): the GraphQL client and
subscription helper, the shared `Me` operation, the shadcn-svelte components,
and `cn`/`uuid`. It is imported as `@stump/ui/...` (a `kit.alias` in
`svelte.config.js`); `preserveSymlinks` in `tsconfig.json` and
`vite.config.ts` keeps the linked files inside this project so they typecheck
as first-class sources and resolve `bits-ui`, `svelte`, `graphql`, … from this
app's `node_modules`. The package therefore has no install step of its own;
its runtime dependencies are declared as peers and installed here. Add
components with `bunx shadcn-svelte@latest add <name>` from this directory —
`components.json` targets the package through the alias.

## Typed GraphQL documents

The editor uses the ingest SDL fallback in `src/lib/graphql/ingest.graphql` until the server schema includes the additive ingest types. Regenerate documents with:

```sh
bun run codegen
```

Shared operations (`packages/stump-ui/src/graphql/operations.graphql`) are
generated separately with `cd packages/stump-ui && bun run codegen`, which
uses this app's toolchain. `codegen.ts` automatically switches to `../crates/graphql/schema.graphql` once that file contains `IngestDropItem`.

## Production build

```sh
bun run check
bun run build
bun run preview
```

The static adapter writes the deployable SPA to `editor/build`, built for the
`/editor` base path (`paths.base` in `svelte.config.js`; override with
`EDITOR_BASE_PATH` at build time). The Stump server serves it itself when
`INGEST_EDITOR_DIR` points at that directory:

```sh
INGEST_EDITOR_DIR=/path/to/stump/editor/build stump_server
# → http://<server>/editor
```

Hashed assets under `/editor/_app/immutable/*` are served with immutable
caching; the shell and client-side routes fall back to `index.html`. API calls
use the server origin directly (`/api/graphql`, `/api/v2/auth/*`), so no proxy
is involved in production. `scripts/dev-fixture-server.sh` exports
`INGEST_EDITOR_DIR` for the local fixture. Route paths below are relative to
the base.

## Routes

- `/login` — cookie-session login
- `/drop` — upload/stage, scan, queue, discard, and live item progress
- `/queue` — analysis queue controls and live phase progress
- `/rework` — quality evidence, provider candidates, field-level picks, and commit/reject/requeue
- `/bulk` — TanStack Table v9 row selection and bulk manual-field recipe
- `/settings/providers` — provider settings, verification, and quality-check catalog/toggles
