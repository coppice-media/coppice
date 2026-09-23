# Stump OMP Rules

## Scope and preservation

- `.omp/` is project OMP configuration. Never create legacy `.pi/` configuration or copy global runtime/session state.
- Do not alter `plan.txt`, Git remotes, unrelated work, or application source outside the requested implementation.
- Source and upstream conventions outrank plans; keep injected guidance concise and put volatile rollout detail in `.omp/PROJECT_STATE.md`.

## Rust and provider contracts

- Preserve default/full behavior and mobile compatibility. Runtime switches are for dormant work; Cargo features remove code/dependencies only after tracing all consumers.
- Treat Axum routes, auth, SQLite/SeaORM, GraphQL, OPDS, KOReader, Kobo, Komga, liseur-sync, media, and mobile clients as public contracts; trace callers before changing APIs, features, env keys, routes, schemas, or payloads.
- Prefer lazy ownership and clean cutovers over speculative rewrites, aliases, compatibility shims, or dead fallback paths.
- Root JavaScript tooling is Bun 1.4.1 with one `bun.lock`; `apps/expo/` is frozen compatibility source outside the active workspace and native-tooling gate.
- Coppice owns metadata-backed request intent, permissions, visibility, approvals, notifications, and staged ingest. Release search, acquisition, retries, credentials, and transport belong to a future independent authenticated provider sidecar, not the Rust server or browser.

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
- Follow `.github/CONTRIBUTING.md` for upstream contributions. The assistant MAY commit and push only branches under `coppice/*` to this fork's existing `origin` when the user explicitly authorizes that commit and push in the current conversation. Never force-push, rewrite published history, change remotes, push secrets, or push directly to upstream/protected branches.
- The only definition of green is the exact gate and post-build replay in `.omp/PROJECT_STATE.md`; never claim an unrun command or smoke result.

## Crate documentation

- Every crate under `crates/` ships a `README.md` in the template indexed by `crates/README.md` (Purpose, Reference / upstream, Decisions, Layout, How to verify, Deep docs; ≤ 120 lines, tables over prose) and a `//!` crate doc in `lib.rs` pointing at it. Any behaviour change updates that crate's Decisions table (decision | why | evidence) in the same change; new crates add a row to `crates/README.md`.
