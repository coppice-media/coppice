# Coppice Home

Dark-first SvelteKit 2 user surface for the headless Stump server. The static
app is mounted at `/app`; the server redirects `/` to Home.

## Development

From the repository root:

```sh
bun install --frozen-lockfile
bun run home dev
```

Vite proxies `/api` to `http://127.0.0.1:25600`. Production calls the same
origin directly. Authentication is a server cookie session: configured OIDC is
the default login path; local credentials remain available at
`/app/login?manual=1` when the server permits them.

## Source map

| Area                                                         | Route or source                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Shell, navigation, permission gates                          | `src/routes/(app)/+layout.svelte`                                                                                                                                                                                                                                 |
| Dashboard                                                    | `src/routes/(app)/dashboard/`, `src/lib/components/dashboard/`, `src/lib/graphql/dashboard.graphql`                                                                                                                                                               |
| Library and series                                           | `src/routes/(app)/library/`, `src/routes/(app)/series/`, `src/lib/graphql/library.graphql`                                                                                                                                                                        |
| Search (header palette, `/search`) and requests              | `src/lib/components/ShellSearch.svelte`, `src/routes/(app)/search/`, `src/routes/(app)/requests/`, `src/lib/search.svelte.ts` (three independent queries: `librarySearch` from 2 characters, `externalBookSearch` and `audibleBookSearch` from 3; the Audible group lists only works Hardcover lacks), `src/lib/components/{UnifiedSearchResults,ExternalBookRequestCard}.svelte`, `src/lib/components/requests/{ExternalHitActions,RequestFormatControl,RequestAcquisitionPanel}.svelte` (`RequestFormatControl` is the one format + narrator pill: toggles, availability dimming, the `audiobookNarrators` popover; it also edits `preferredNarrator` on a request), `src/lib/graphql/{unified-search,requests}.graphql` |
| Reading, sessions, annotations                               | `src/routes/(app)/reading/`, `src/routes/(app)/annotations/`                                                                                                                                                                                                      |
| EPUB reader and chapter maps                                 | `src/routes/(app)/reader/[mediaId]/`, `src/lib/components/reader/`, `src/lib/graphql/reader.graphql`                                                                                                                                                              |
| Devices, protocol setup, CrossPoint pairing and LAN delivery | `src/routes/(app)/devices/`, one shared card for catalog options and paired devices in `src/lib/components/ClientCard.svelte` (status marks in `ClientStatusIcons.svelte`, Sync/Reads chips in `ClientCapabilities.svelte`; `DeviceCard.svelte` and `ClientPicker.svelte` compose it), plus `src/lib/components/{AddClientDialog,CredentialReveal,CrossPointTargetSetup,CrossPointDeliveryQueue}.svelte`, `src/lib/graphql/{operations,crosspoint}.graphql` |
| Worker management                                            | `src/routes/(app)/workers/`                                                                                                                                                                                                                                       |
| Account and settings                                         | `src/routes/(app)/account/`, `src/routes/(app)/settings/`                                                                                                                                                                                                         |
| Shared GraphQL client and UI                                 | `src/lib/stump-ui` → `../packages/stump-ui/src`                                                                                                                                                                                                                   |

## Security contracts

- The server is authoritative. UI permission gates improve navigation but do not
  authorize GraphQL operations.
- Device credentials inherit their owner's permissions. A `library_scope` can
  narrow visible content; it cannot grant content or actions the owner lacks.
- Requests store metadata intent. Unified search and approved MAM acquisition
  use the existing GraphQL client; bridge credentials and release transport
  remain server-side. The `ACQUIRE_RELEASES` UI gate is not authorization.
- `/app/devices` is the supported setup path for protocol-specific endpoints,
  QR data, rotation, last-seen state, and sync summaries. A raw API key remains
  authenticated and permission-bound but does not create that device-registry
  metadata.
- CrossPoint pairing uses the existing five-minute device approval and hands the keyed KOReader-compatible URL to the device once; rich `/api/v1` is only mounted beneath `/koreader/{key}`. The pinned physical firmware does not use that rich URL at a custom location.
- CrossPoint LAN delivery is a separate, unauthenticated device transfer lane. Home accepts only a user-confirmed private IPv4 target after `/api/status`, fixed HTTP/WS ports, bounded uploads, and never overwrites or deletes remote files.
- Home's general Ingest editor navigation link is limited to the server owner
  or a user with `MANAGE_LIBRARY`. Acquisition review links are contextual;
  Editor authorization remains server-enforced.

## Shared design system

`packages/stump-ui/` owns shadcn-svelte primitives, GraphQL session helpers,
semantic tokens, and theme persistence. `src/lib/stump-ui` is a source symlink;
`preserveSymlinks` keeps it typechecked against this app's dependencies.

The default is dark mode. `ThemeSwitcher` stores mode and preset in
`coppice.theme.mode` and `coppice.theme.preset`. Add primitives from this app so
the shadcn alias writes to the shared package:

```sh
bunx shadcn-svelte@latest add <name>
```

Do not copy theme variables or shared primitives into Home.

## GraphQL and build

```sh
bun run home codegen
bun run home check
bun run home build
```

Area operations live in `src/lib/graphql/*.graphql`; generated documents live
in `src/lib/graphql/generated/`. Generate shared operations from the repository
root:

```sh
bun run --filter @stump/ui codegen
```

The static adapter writes `home/build` for the `/app` base path. Override it
with `HOME_BASE_PATH`. The server serves the build when `STUMP_HOME_APP_DIR`
points to it; `scripts/dev-fixture-server.sh` sets that path for the local
fixture. Build Home and the ingest editor serially because each SvelteKit build
rewrites its own output tree.
