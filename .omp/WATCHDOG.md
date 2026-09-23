# Stump Watchdog

Before changing code, identify the target crate/module or Bun workspace and
trace its callers, feature propagation, route ownership, auth middleware,
generated GraphQL clients, and focused checks. Treat Axum paths, OPDS 1.2/2.0,
KOReader, Kobo, Komga/Grimmory, liseur-sync, GraphQL/mobile API,
SQLite/SeaORM schema, media streaming, and generated SDK types as
compatibility-sensitive.

## Stop and re-check

- A change removes or gates a route, dependency, environment key, default, or
  public DTO.
- Scheduler/watcher/job ownership moves between `apps/server` and `core`.
- PDF/media behavior or protocol authentication changes.
- A new Cargo feature is introduced without tracing workspace consumers.
- A migration is reordered, replaces existing registry entries, or alters more
  than one SQLite column in a single `alter_table`.
- A Komga 401 could emit `Set-Cookie ... Max-Age=0`, or a Grimmory route could
  collide with the root identity tree.
- Browser/server work grows release search, tracker credentials, acquisition,
  retries, VPN/qBittorrent control, or transport instead of defining a narrow
  authenticated future sidecar protocol.
- Yarn/Lerna, subsidiary lockfiles, or native Expo tooling re-enter the active
  Bun workspace without an explicit lane decision.

Keep the default build working, preserve user files/remotes and client behavior,
and leave no speculative shim or dead compatibility path. Do not rely on
`plan.txt` where source, a focused test, or a pinned external contract is
available. Workers skip project-wide validation; the coordinator owns one final
gate and reports exact evidence. Commit or push only under the turn-specific
`coppice/*` authorization in `.omp/AGENTS.md`.
