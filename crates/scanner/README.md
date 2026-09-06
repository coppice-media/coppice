# stump_scanner

## Purpose

Filesystem scan planning for Stump libraries: walking a library root to find
series directories, walking a series to find media files, and reconciling what
is on disk against what a persistence adapter reports (create / visit /
recovered / missing / skipped) using directory and file mtimes. The crate owns
`ScanOptions` (the user-facing scan strategy), the `ScanSource` read-only
persistence trait with its identity types, the walkers, and the in-memory
`TagCache`. It deliberately contains no application context, job runtime,
event channel, or database connection (`src/lib.rs:3-4`, `src/source.rs:92-95`);
the SeaORM adapter, job orchestration, media building, watcher wiring, and all
writes stay in `core/src/filesystem/scanner/`.

The crate also owns `sequence`, the one chapter/volume filename sequence
parser in the workspace (identifiers, omnibus ranges, decimal chapters,
release-metadata stripping, changing-token fallback, gap detection). It lives
here because both a write-side consumer (`stump_tools`' `missing-sequence`
tool) and a read-side one (`stump_core`'s `missing_chapters_in_series` quality
check) need it, and `stump_core` must not depend on `stump_tools`
(`src/sequence.rs:1-20`).

## Reference / upstream

| Reference | Pin | Notes |
| --- | --- | --- |
| Upstream Stump `core/src/filesystem/scanner/{walk.rs,options.rs,tag_cache.rs}` | `stumpapp/stump` nightly merge `37fdb7d7` (PR #1361) | Behavioural baseline; walkers there took the `Ctx`/`db` directly (`walk.rs:158,421@37fdb7d7`) |
| Extraction | `27764cf5` (crate created), `3cebf0b8` (core files deleted, `store.rs` adapter added) | `git log --oneline -- crates/scanner core/src/filesystem/scanner` |
| `CBZ-Missing-Sequence-Checker` behaviour (identifiers, omnibus ranges, decimals, metadata stripping, fallback, report modes) | <https://wiki.kavitareader.com/guides/external-tools/cbz-missing-sequence-checker/> (docs only) | Rules re-implemented in `src/sequence.rs` from the documented behaviour; no upstream source consulted |

`ScanOptions`/`ScanConfig` JSON is a public contract consumed by the GraphQL
library mutations and the scan-record object
(`crates/graphql/src/mutation/library.rs:34`,
`crates/graphql/src/object/library_scan_record.rs:4`); the camelCase shape is
pinned by `src/options.rs:127-235`.

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Walkers take a `ScanSource` trait object instead of the DB connection | Lets the walker be tested with an in-memory source and compiled without SeaORM; core implements the trait once | `src/source.rs:92-104`; `core/src/filesystem/scanner/store.rs:13-21,52-127`; tests `src/walk.rs:545-782` |
| `ScanSource` is read-only (`existing_series`, `existing_media`, `stored_dir_mtimes`) | Reconciliation needs identities only; writes stay with the jobs that own transactions | `src/source.rs:96-104`; `core/src/filesystem/scanner/{library_scan_job,series_scan_job,utils}.rs` |
| `async_trait` boundary, blocking `WalkDir` inside `spawn_blocking` | Object-safe async trait for adapters; filesystem traversal never runs on the executor | `src/source.rs:3,96`; `src/walk.rs:11,93,290` |
| Directory mtime short-circuit: unchanged non-root directories are pruned, root always traversed | Same as upstream; the root mtime does not reflect nested changes | `src/walk.rs:322-345`; upstream `walk.rs:346-378@37fdb7d7` |
| Ignored or hidden non-root directories prune their whole subtree (deviation from upstream) | Upstream only filtered files, so a glob naming a directory still let every child media file through | `src/walk.rs:311-319`; test `src/walk.rs:673`; upstream `walk.rs:330-345@37fdb7d7` has no `skip_current_dir` on ignore |
| Existing media newer than `modified_at` → `Rebuild`; otherwise `ScanOptions::book_operation()`; otherwise skip | `BuildChanged` is mtime-dependent so it yields `None` and defers to the file comparison | `src/options.rs:73-89`; `src/walk.rs:386-402,442-453` |
| `Missing`/`Unknown` records present on disk are reported as recovered | Lets the job flip status back without a rebuild | `src/source.rs:38-41`; `src/walk.rs:167-174,405-417` |
| `max_depth == Some(1)` selects collection-based discovery (`dir_has_media_deep`) | Collection libraries are one level of series directories with nested media | `src/walk.rs:82-89,110-118`; `crates/media/src/common.rs:136-139,241-280` |
| `TagCache` is a plain `HashMap<String, i32>`; loading/insertion stays in core | Per-file orchestration is not extracted yet | `src/tag_cache.rs:5-18`; `core/src/filesystem/scanner/utils.rs:36-83` |
| Depends on `stump_media` with `default-features = false` | Only `PathUtils` and the processed-result structs are needed; avoids compiling PDFium/RAR into the walker | `Cargo.toml:12`; `src/walk.rs:10`; `src/options.rs:2`; `crates/media/Cargo.toml:6-9` |
| `sequence` lives in the scanner, not in `stump_tools` | The parser has consumers on both sides of the dependency edge; putting it in `stump_tools` would force `stump_core` (quality check) to depend on the tools crate and its archive-writing surface | `src/sequence.rs:14-16`; `crates/tools/src/missing_sequence.rs:13-15`; `core/src/ingest/quality/missing_chapters_in_series.rs:7` |
| Sequence numbers are `i64` thousandths, not floats | Gap math and equality must be exact for decimal chapters (`112.1`), and `f64` keys cannot be used in a `BTreeSet` | `src/sequence.rs:44-79`; test `numbers_format_without_trailing_zeroes` |
| A decimal number covers its integer chapter (`112.1` → `112`) and a folder-wide span over 10000 refuses to enumerate gaps | Interstitial chapters must not open gaps, and a stray date/ISBN token would otherwise report millions of missing numbers | `src/sequence.rs:66-72,25-27`; tests `decimal_chapters_do_not_open_gaps`, `wide_spans_refuse_to_enumerate_gaps` |
| `walk_series` yields an audiobook **folder** as one media path and `skip_current_dir()`s its files; the series root is checked the same way | `Library/Book Title/01.mp3 … 12.mp3` is the common layout, and walking into it shatters one publication into twelve books. The check runs *after* the unchanged-directory mtime skip, so an unchanged subtree costs no extra `readdir`, while a first scan (no stored mtime) and the always-visited root always reach it | `src/walk.rs:347-368`; tests `audiobook_folder_under_a_series_is_one_media_path`, `audiobook_series_root_is_one_media_path` (`src/walk.rs:790,841`); predicate `crates/media/src/common.rs::dir_is_audio_book` |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Re-exports the public API |
| `src/options.rs` | `ScanOptions`, `ScanConfig` (`BuildChanged` / `ForceRebuild` / `Custom`), `CustomVisit`, `BookVisitOperation`, `CustomVisitResult`, serde shape; 5 tests |
| `src/source.rs` | `ScanSource` trait, `ScanError`/`ScanResult`, `ScanStatus`, `SeriesIdentity`, `MediaIdentity` |
| `src/tag_cache.rs` | Scan-wide tag name → id lookup |
| `src/walk.rs` | `WalkerCtx`, `walk_library` → `WalkedLibrary`, `walk_series` → `WalkedSeries`, mtime helpers; 7 tests with an in-memory `ScanSource` |
| `src/sequence.rs` | `SequenceKind`/`SequenceNumber`/`SequenceRange`, `parse_identifier`, `parse_identifier_of`, `clean_name`, `to_ranges`, `analyze_sequence` → `SequenceAnalysis`; 13 tests |

Consumers: `core/Cargo.toml:45`, `apps/server/Cargo.toml:82`,
`crates/graphql/Cargo.toml:40`; core call sites
`core/src/filesystem/scanner/{library_scan_job.rs:40-42,series_scan_job.rs:28-30,store.rs:9-11,utils.rs:33}`,
`core/src/filesystem/media/builder.rs:19`, `core/src/job/stump_job.rs:2`,
`apps/server/src/routers/komga_backend.rs:212`.

`sequence` consumers: `crates/tools/src/missing_sequence.rs`,
`core/src/ingest/quality/missing_chapters_in_series.rs`.

## How to verify

```text
cargo test -p stump_scanner            # 27 contract tests (5 options, 9 walker, 13 sequence)
cargo check -p stump_scanner           # feature-off stump_media builds without pdf/rar
cargo test -p stump_core --lib --tests # SeaORM adapter and scan jobs (gate line in .omp/PROJECT_STATE.md)
```

Live probe: trigger a scan on the fixture (`scripts/dev-fixture-server.sh`,
`http://127.0.0.1:25600`) through the web UI or the GraphQL `scanLibrary`
mutation, then re-run it: the second scan reports the series/media as skipped
with unchanged directories pruned. The Komga harness `make replay` in
`../komga-compat` exercises the scanned fixture library indirectly (series and
book listings); there is no scanner-specific Hurl target.

## Deep docs

- `docs/content/docs/developer/state.mdx` (crate table row for `stump_scanner`).
- `docs/content/docs/developer/server-architecture.mdx` and `modular-ingest.mdx` (ingest pipeline that sits in front of the scanner).
- `core/src/filesystem/scanner/store.rs` (the only `ScanSource` implementation).
