# Stump OMP Rules

## Scope and preservation

- `.omp/` is project OMP configuration. Never create legacy `.pi/` configuration or copy global runtime/session state.
- Do not alter `plan.txt`, Git remotes, unrelated work, or application source outside the requested implementation.
- Source and upstream conventions outrank plans; keep injected guidance concise and put volatile rollout detail in `.omp/PROJECT_STATE.md`.

## Rust and provider contracts

- Preserve default/full behavior and mobile compatibility. Runtime switches are for dormant work; Cargo features remove code/dependencies only after tracing all consumers.
- Treat Axum routes, auth, SQLite/SeaORM, GraphQL, OPDS, KOReader, Kobo, Komga, liseur-sync, media, and mobile clients as public contracts; trace callers before changing APIs, features, env keys, routes, schemas, or payloads.
- Prefer lazy ownership and clean cutovers over speculative rewrites, aliases, compatibility shims, or dead fallback paths.

## Defect guardrails

- `crates/migrations/src/lib.rs` is append-only: retain every existing `mod`/`Box::new` and order; one SQLite column per `alter_table`.
- Komelia decodes DTO enums with unguarded `valueOf`: mirror every Komga enum to `komga-client` 0.11.0; series thumbnails have no `GENERATED`. Never emit `Set-Cookie ... Max-Age=0` on a Komga 401.
- OPDS Basic auth is unconditional; `X-Stump-Save-Session` controls session creation only. Preserve the Basic cache and `basic_auth_accepted` rule.
- Keep the Komga mount collision test and Grimmory `/komga` identity split (`routes_without_current_user`); Axum can panic while building routes.
- Workers do not validate mid-batch; they report exact errors. The coordinator
  runs one gate after every worker yields and fixes dead imports/helpers.
- On btrfs, metadata ENOSPC can appear near 92% metadata use: prune `target/debug/incremental` between batches, never during a build.

## Evidence, upstream, and validation

- Local working-tree docs need no pins until this fork is committed; external client/repository pins are immutable. Use pinned clients, not Komga OpenAPI alone, for client-facing contracts.
- Follow `.github/CONTRIBUTING.md`; do not commit or push from this bootstrap.
- The only definition of green is the exact gate and post-build replay in `.omp/PROJECT_STATE.md`; never claim an unrun command or smoke result.
