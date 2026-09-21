# Fixture and replay workflow

## 1. Establish the target

Read `.omp/PROJECT_STATE.md` first. Treat its fixture launcher, enabled features,
current credentials location, target variables, and replay commands as the only
live operational source. This skill deliberately omits ports, IDs, counts,
commit-local batch state, and credential values.

The fixture launcher is `scripts/dev-fixture-server.sh`. It uses a disposable
fixture root, writes an owner-only generated `ENDPOINTS.md`, and then runs the
server in the foreground. `ENDPOINTS.md` is a local configuration sheet, not
evidence. Read it only on the operator machine and pass values as environment
variables or Hurl variables.

## 2. Bring up the long-running service

Use `hub` process management, not a background shell, for the fixture server:

1. `hub start` the launcher named by project state.
2. Wait for the process readiness signal and configured port.
3. Inspect `hub logs` only for operational errors; redact before sharing.
4. Keep the process alive for all selected lanes; restart only for a clean-state
   requirement documented by project state.

Do not start a second fixture instance merely to run another independent Hurl
file. A reference container (for example ABS or Kavita) is a separate oracle;
its lifecycle and credentials remain runtime-only and come from the state sheet
or sibling README.

## 3. Run the sibling harness

Work from `../komga-compat/`. Use its declared environment prerequisites and
runtime variables; current values belong in `.omp/PROJECT_STATE.md`. The Hurl
specs are the semantic oracle. `mitmdump` captures client traffic only and MUST
preserve the incoming Host header when used for Komga/Readium evidence.

Choose exactly one relevant Make target from the matrix. The target's own
preflight checks validate Hurl, target origin, required variables, and (where
needed) fixture IDs. A missing variable or stale target is a setup failure, not
a protocol regression. Never paste the command's expanded environment into a
report.

## 4. State and cleanup

Use fresh synthetic IDs where a target requires them. Use a disposable existing
library root for library management; it MUST be outside and not above existing
library roots. A replay that mutates reading progress, lists, collections, or
metadata may alter the shared fixture, so follow the ordering and restore notes
in project state before another lane.

Do not claim the project is green from one target. The exact workspace gate and
post-deploy replay set are defined only in `.omp/PROJECT_STATE.md`; the
coordinator runs that gate after worker changes, not this skill.

## Safe evidence shape

Publish only sanitized records containing protocol/client label, target name,
method, relative path, status, selected assertion result, and a concise reason.
Strip Authorization, cookies, API keys, passwords, local roots, raw media,
HAR/mitm flows, databases, and machine-specific URLs. Preserve dynamic IDs as
placeholders and test invariants (type, status, relationship, ordering) rather
than pinning values.
