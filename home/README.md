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
| EPUB/paged/audio reader, reader annotation authoring (create/edit/delete own highlights and page notes), and chapter maps | `src/routes/(app)/reader/[mediaId]/`, `src/lib/components/reader/` (annotation panel in `ReaderAnnotations.svelte`), `src/lib/graphql/reader.graphql`                                                                                                |
| Book-club guest reader, live stream, session discussion, session REST transport, and BookClub controls        | `src/routes/club-reader/[sessionId]/`, `src/lib/club-reader-api.ts`, `src/lib/components/social/{BookClubPanel,ClubReaderDiscussion,ReaderSessionDiscussion}.svelte`, `src/lib/reader-sessions.ts`, `src/lib/graphql/social.graphql` |
| Devices, protocol setup, CrossPoint pairing and LAN delivery | `src/routes/(app)/devices/`, one shared card for catalog options and paired devices in `src/lib/components/ClientCard.svelte` (status marks in `ClientStatusIcons.svelte`, Sync/Reads chips in `ClientCapabilities.svelte`; `DeviceCard.svelte` and `ClientPicker.svelte` compose it), plus `src/lib/components/{AddClientDialog,CredentialReveal,CrossPointTargetSetup,CrossPointDeliveryQueue}.svelte`, `src/lib/graphql/{operations,crosspoint}.graphql` |
| Worker management                                            | `src/routes/(app)/workers/`                                                                                                                                                                                                                                       |
| User management (`/users`)                                   | `src/routes/(app)/users/`, `src/lib/components/users/{UserEditor,PermissionPicker,UserActionDialog}.svelte`, `src/lib/users.ts` (permission areas, labels, and the client mirror of the server's implied-permission table), `src/lib/oidc.ts` (OIDC config query shared with `/login`), `src/lib/graphql/users.graphql` |
| Account and settings                                         | `src/routes/(app)/account/`, `src/routes/(app)/settings/`                                                                                                                                                                                                         |
| Shared GraphQL client and UI                                 | `src/lib/stump-ui` → `../packages/stump-ui/src`                                                                                                                                                                                                                   |

## Security contracts

- The server is authoritative. UI permission gates improve navigation but do not
  authorize GraphQL operations.
- Book-club guest links are per-participant revocable capabilities. The SPA at
  `/app/club-reader/{sessionId}#token=...` removes the fragment before redeeming
  it; the capability is then carried only by the scoped HttpOnly cookie. This
  route does not bootstrap `MeDocument`, use native GraphQL, or redirect to login.
- Guest EPUB, paged/PDF, and audio resources use only
  `/api/v2/club-reader/...`; guest progress and annotations remain in a separate
  session ledger and never write account reading history. Progress and notes
  are private by default; guests explicitly opt into coarse group progress or
  sharing an individual annotation.
- The guest page's live stream (`EventSource` on
  `/api/v2/club-reader/sessions/{id}/events`) carries only change kinds; the page
  refetches through the authorized snapshot/messages GETs, and a `revoked` event
  shows the unavailable state. Session discussion is visible only to that
  session's active participants (never the club's member discussions) and is
  rendered as plain text; organizers read and moderation-delete it from the
  BookClub panel.
- Account members join a guest session explicitly with a chosen alias. Organizer
  session, participant, publication, and queue controls require
  `SHARE_BOOK_CLUB_READER` plus the club's Admin/Creator role. A changed queue
  book can pause guest reading until the organizer publishes it.
- Reverse-proxy deployments must keep resource authentication enabled and add
  only the narrow guest SPA/API bypass plus required immutable Home assets.
  Never bypass all of `/app/*` or `/api/*`; this repository does not apply a
  live proxy rule.
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
- `/app/users` is linked for the server owner or `READ_USERS`; create/edit
  controls need `MANAGE_USERS` and lock, sign-out, and delete need the owner,
  mirroring the `users`, `updateUser`, `updateUserLockStatus`,
  `deleteUserSessions`, and `deleteUser` guards. The editor is never offered on
  your own account (the server ignores privileged fields on a self-update) or
  the owner's. The browser cannot see `STUMP_OIDC_SYNC_PERMISSIONS`, so with
  OIDC enabled the screen warns that sync may replace an OIDC account's
  permissions at sign-in.

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
