# Evidence triage

Triage in this order. Stop at the first proven setup defect; do not rewrite
server code to hide a harness problem.

| Observation                                                              | Classification        | Action                                                                                                                                       |
| ------------------------------------------------------------------------ | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Missing Hurl/jq, missing variable, stale ID, target validation rejection | Harness/setup         | Fix runtime inputs or fixture state; no compatibility claim                                                                                  |
| Fixture process not ready, wrong feature gate, route absent at startup   | Fixture/configuration | Check project state, launcher output, and enabled feature; restart via `hub` if required                                                     |
| HTTP status, header, byte-range, cookie, or JSON assertion mismatch      | Server contract       | Save sanitized request shape and response invariant; trace the adapter and update the matching replay/spec only when implementation is wrong |
| ABS key-path mismatch against reference                                  | Projection difference | Compare client-consumed fields and documented deviations; keep diff paths only, never values                                                 |
| IDs, timestamps, ordering, or counts differ after an otherwise valid run | Dynamic fixture       | Prefer stable relationships/types/monotonicity; do not pin volatile values or declare failure on incidental order                            |
| Hurl passes but app renders, downloads, syncs, or resumes incorrectly    | Physical client       | Capture the app request and device state; classify parser/UI/cache/firmware separately from server behavior                                  |
| Proxy-generated EPUB URLs point at the wrong origin                      | Capture topology      | Re-run with Host preservation; mark prior capture invalid rather than blaming Komga or Stump                                                 |

## Minimal report

1. **Claim:** one route/client behavior, not a whole protocol.
2. **Tier:** harness, server probe, physical device, or source-only.
3. **Fixture:** sanitized label and whether a reference container was used.
4. **Inputs:** variable names only; never values.
5. **Evidence:** method, relative path, status, selected invariant, and redacted
   error text.
6. **Verdict:** pass, server regression, fixture/setup failure, client-only,
   or unknown.
7. **Next action:** one focused replay/probe or one client retest.

## Report-safe transformations

- Replace UUIDs, keys, cookies, tokens, usernames, passwords, and local paths
  with typed placeholders.
- Keep response key names, status codes, content types, range semantics, and
  relationships when they establish the contract.
- For comparison output, publish paths and categories (missing/extra/type),
  never response values or raw diffs containing credentials.
- Do not publish `ENDPOINTS.md`, Hurl variable expansions, HAR/mitm flows,
  databases, logs with request headers, APKs, media, or reference-container
  state.

A passing target is evidence for only the routes and assertions it exercised.
The exact green gate and post-deploy replay set remain in
`.omp/PROJECT_STATE.md` and must not be copied into this skill.
