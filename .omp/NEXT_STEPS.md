# Stump Next Steps

Immediate slices, in order. Full ordering, rationale, dependencies, and
non-goals: `docs/content/docs/developer/roadmap.mdx`. A design page or replay
fixture is evidence for a contract, not proof that a feature is shipped.

## 1. Komf API-mode manual identification (roadmap phase 1)

- Implement Komf's manual identify/search/match routes against our Komga
  profile, plus thumbnail upload/delete in `crates/komga/src/routes/media.rs`
  (GET only today). Contract: `local://komf-compat-study.md` and
  `docs/content/docs/developer/komga-compat.mdx`.
- Acceptance: a pinned Komf client build can identify a series and replace a
  thumbnail through Stump; the Komelia replay suite still passes.

## 2. Crate split completion (roadmap phase 2)

- Extract per-file scan orchestration behind `ScanStore` / `ScanReporter`,
  then `stump_watcher` behind `WatchedLibraries` / `ScanSubmitter`, per
  section 4 of `local://scanner-split-plan.md` (PR order and required tests
  are specified there).
- Acceptance: `stump_scanner` and `stump_watcher` have no `JobContext`,
  `StumpJob`, `CoreEvent`, or `MemoryStorage` imports; the watcher test matrix
  ports unchanged against trait fakes; kobo-only and comics-only feature
  builds compile without apalis/notify/email/metadata deps.

## 3. SSE `TaskQueueStatus` emission (roadmap phase 1.3)

- Emit `TaskQueueStatus` on `crates/komga/src/routes/sse.rs` matching Komf's
  buffered-event flush contract (`KomgaEventHandler`, count==0 flush).
- Acceptance: a Komf automatic-mode run against a fresh synthetic fixture
  flushes without hanging; event visibility matches the Komga profile tests.

After these: proceed to roadmap phase 3 (provider host) per
`docs/content/docs/developer/roadmap.mdx`.
