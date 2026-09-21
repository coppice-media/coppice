# Performance Log

## 2026-09-09 — initial project-local draft

- Created the concise router, four progressive-disclosure references, and a
  deterministic standard-library fixture CLI.
- Added behavior coverage for slicing, inspection, and mode comparison.
- Added eight trigger cases with four in-domain and four near-miss queries.
- Evals, script execution, formatters, linters, builds, and project-wide tests
  were intentionally not run in this authoring pass; the coordinator owns the
  final validation gate.

## 2026-09-09 — coordinator validation

- Behavior evals: 3/3 current answers passed; 2/3 source-aware baselines also
  passed because the named helper exposes two contracts directly.
- Runtime proof: two independent 10% slice runs produced identical EPUB and M4B
  SHA-256 values; normalized reports differed only in declared output paths.
- Quality review tightened shared-slice, actual exit-status, malformed-overlay,
  zero-common-target, and repeated-cardinality expectations.
