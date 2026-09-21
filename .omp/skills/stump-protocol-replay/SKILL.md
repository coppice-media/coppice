---
name: stump-protocol-replay
description: 'Replay and triage Stump protocol contracts against the local fixture server and the sibling ../komga-compat Hurl harness. Use for Komga/Komelia, Kavita, Audiobookshelf/ABS, Kobo, KOReader/KOSync, Liseur, Mihon, or library-management compatibility checks; for choosing a focused replay, bringing up the fixture service, separating server evidence from physical-client verification, or handling replay failures.'
---

# Stump protocol replay

Use this skill for contract evidence, not broad application testing. RFC 2119 applies: MUST/REQUIRED are absolute; SHOULD/RECOMMENDED allow justified exceptions; NEVER/AVOID prohibit an unsafe or misleading action.

<critical>
- MUST read `.omp/PROJECT_STATE.md` before launch or replay; it owns current env, ports, credentials sheet, and green gate.
- MUST use the user-owned `../komga-compat/` harness; NEVER copy its fixtures, credentials, or GPL/AGPL code into Stump.
- MUST keep `ENDPOINTS.md` owner-only and local; NEVER quote it or put secrets in logs, reports, specs, or committed files.
- MUST label Hurl/probe output as server-contract evidence; it does not prove a physical client works.
- MUST select the narrowest relevant target; NEVER run unrelated protocol lanes to explain one failure.
</critical>

## Workflow

1. Read project state, then identify the claimed protocol, client, route, and evidence tier.
2. Start the fixture server as a long-running `hub` process; wait for readiness before Hurl.
3. Read the local endpoint sheet only when configuring runtime values; keep values in process environment.
4. Choose one target from `references/protocol-matrix.md`; inject IDs and credentials at runtime.
5. Preserve Host headers for any reverse recorder; reject arbitrary upstream targets.
6. Capture sanitized method/path/status and assertion evidence, never raw flows or secrets.
7. Triage failures with `references/evidence-triage.md`; report contract, fixture, or client causes separately.
8. Stop/restart the shared fixture through `hub` only when state requires a clean instance.

## Target routing

| Claim                                                | Focused replay or probe                                                 |
| ---------------------------------------------------- | ----------------------------------------------------------------------- |
| Komga / Komelia REST, auth, catalog, media, sessions | `make replay`                                                           |
| Mihon Komga catalog, reader, tracker                 | `make replay-mihon`                                                     |
| Kavita clients and auth styles                       | `make replay-kavita`                                                    |
| ABS/Lissen API session                               | `make replay-abs`                                                       |
| ABS response-shape comparison                        | `make replay-abs-diff`                                                  |
| Native Liseur sync, catalog, positions, annotations  | `make replay-liseur-sync`                                               |
| Komga library create/patch/filesystem/delete         | `make replay-library-management`                                        |
| Kobo or KOReader                                     | targeted fixture probe or physical-client lane; no invented Make target |

Read `references/protocol-matrix.md` for prerequisites and lane boundaries. Use `scripts/select-replay.sh` when a deterministic lane-to-target lookup is useful; it only prints routing and never launches a service.

## Evidence boundary

A passing Hurl target proves the exercised HTTP contract against this fixture. A device run proves that a particular app/device can consume it. A source-only claim proves neither. Record the tier, target, fixture identity, and sanitized assertions; route current IDs, launch values, and exact green replay set to `.omp/PROJECT_STATE.md`.

See `references/workflow.md` for fixture bring-up and safe runtime handling.
See `references/client-boundaries.md` for Kobo, KOReader, Liseur, and app/device separation.
See `references/evidence-triage.md` for failure classification and report-safe evidence.
