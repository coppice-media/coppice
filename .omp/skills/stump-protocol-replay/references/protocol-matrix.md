# Protocol target matrix

Use this table to avoid paying for unrelated lanes. The Make targets and spec
names below are from the sibling harness; do not invent a target when one is
not listed.

| Surface                       | Target                           | What it exercises                                                                                                                     | Runtime inputs                                                                                 |
| ----------------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Komga / Komelia               | `make replay`                    | Auth/session, catalog, books/media, siblings, cache/range, SSE/session-cookie scope, and Grimmory alias where included by the harness | Base URL, owner/user credentials, fresh book/series/thumbnail IDs as required by state         |
| Mihon Komga extension/tracker | `make replay-mihon`              | Basic and `X-API-Key` catalog/reader calls, pages, images, and tracker progress                                                       | Base URL, username/password, API key, visible series ID                                        |
| Kavita wave                   | `make replay-kavita`             | `apiKey` query, `x-api-key`, Plugin JWT, libraries, series/volumes/chapters, images, progress and client filters                      | Base URL and API key; feature/runtime enablement from state                                    |
| ABS / Lissen                  | `make replay-abs`                | Status/login/refresh/authorize, library and item traversal, cover/audio ranges, playback, progress, bookmarks, and negatives          | Base URL, username/password; ABS feature and audible fixture item                              |
| ABS oracle comparison         | `make replay-abs-diff`           | JSON key/path comparison against the pinned `abs-ref` reference; values are not the oracle                                            | Target + reference URLs and runtime credentials from state                                     |
| Native Liseur sync            | `make replay-liseur-sync`        | Login/token scopes, work resolution, idempotent positions/sessions, annotation CAS/feed, catalog and attachment lane                  | Base URL, username/password; synthetic identity values only                                    |
| Library management            | `make replay-library-management` | Throwaway library create/patch, filesystem browse, scan/refresh, and delete contract                                                  | Base URL, owner credentials, disposable `LIBRARY_ROOT`                                         |
| Collections/read lists        | `make replay-containers`         | Throwaway collection/read-list mutations and ordered readback                                                                         | Base URL, credentials, visible book and series IDs; run only when state says the spec is ready |

## Lane choices that are easy to confuse

- **Liseur as an app using Komga:** use Komga `make replay` for server
  contract, then a device run for the app. Native `/v1/*` sync is the separate
  `make replay-liseur-sync` lane.
- **Kobo:** no sibling Hurl Make target is the physical-device oracle. Use the
  fixture's Kobo initialization/sync/state/download probe described by the
  Kobo capability documentation, or a real stock Kobo run; label it a probe
  or device result.
- **KOReader:** no sibling Hurl Make target is currently listed. Use the
  fixture's KOSync endpoint probe and a KOReader run for the `users/auth` and
  progress round trip; do not relabel it as Komga or Liseur-native evidence.
- **ABS diff:** a key-set difference is a triage input, not automatically a
  failure. Check the documented Stump projection and client-consumed fields.
- **Readium:** it is a separate pending/targeted lane when listed by project
  state; do not add it to a Komga claim unless EPUB resource/progression is the
  claim under review.

## Evidence selection

For one changed route, select the smallest existing target that reaches it. If
no target reaches the route, record a server-side probe or source-only status
instead of stretching a neighboring target's result. Use captured IDs only as
runtime variables; assertions should check relationships and wire invariants,
not fixture-specific names, row counts, or numeric IDs.
