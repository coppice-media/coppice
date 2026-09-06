# stump_watcher

## Purpose

`stump_watcher` owns library filesystem watching: `notify` setup, the lazily
created OS backend, change debouncing, path accumulation, and the listener
lifecycle (`init`, `add_watcher`, `remove_watcher`, `stop`). It does **not**
scan. It has no database, job queue, application context or event type: the
owner implements `WatchedLibraries` to say which roots are watched and
`ScanSubmitter` to turn one debounced burst into one scan. Ignore/glob rules,
series discovery and the scan itself stay in `core/src/filesystem/scanner`;
`notify` is linked into the server only through `stump_core`'s `watcher`
feature.

## Reference / upstream

| Reference | Relation |
| --- | --- |
| `notify` 8.0 — `RecommendedWatcher`, `EventKind`, `RecursiveMode::Recursive`, `notify::ErrorKind::PathNotFound` | The only backend. `WatcherError::Notify` wraps `notify::Error` verbatim so callers can still match its kind. `Cargo.toml:9`, `src/lib.rs:48-71` |
| Upstream Stump `core/src/filesystem/scanner/library_watcher.rs` | Extracted into this crate in `bc347a78` / `7617c66d`; command and public-method behaviour unchanged, including the lazy backend. |
| `core/src/filesystem/scanner/watcher_adapters.rs:17-65` | The only production implementation of both traits: ready libraries with `library_config.watch = true`, and a `StumpJob::library_scan` enqueue on the job runtime. |
| `core/src/context.rs:260-272` | One `Watcher` per context, built lazily in a `OnceLock` with `DEFAULT_DEBOUNCE`. |

## Decisions

| Decision | Why | Evidence |
| --- | --- | --- |
| Debounce is a 5 s *quiet period* per burst, not a fixed poll interval | Copying a large file emits one create plus a stream of modify events; the scan must not start until the copy stops. Any new event resets the window. | `src/watcher.rs:16-20`, `src/watcher.rs:98-117` |
| One spawned Tokio task per burst re-checks `last_change.elapsed()` and sends `Command::Flush` once, then exits | A per-event timer would fan out one task per filesystem event; this keeps a burst of thousands of events to a single timer. | `src/watcher.rs:104-117` |
| Only `EventKind::Create(_)` and `EventKind::Modify(_)` are forwarded; every other kind is dropped inside the `notify` callback | New or changed files need indexing. `Access` would make every *read* a scan trigger, and a `Remove` is reconciled by the next scan of that library rather than by an immediate one. Renames arrive as `Modify(ModifyKind::Name)`, so a rename does trigger a scan. | `src/watcher.rs:37-49` |
| The sync `notify` callback only pushes `Command::ChangedFiles` onto an unbounded channel; every await happens in the listener task | `notify` calls back on its own thread with no Tokio runtime attached, so the callback must neither block nor await; unbounded means it can never drop an event under load. | `src/watcher.rs:34-52`, `src/watcher.rs:169-217` |
| Paths accumulate in a `HashSet`; flush takes the set and emits at most one `ScanRequest` per library, matched by `changed.starts_with(root)` | Duplicates inside the window collapse, and a burst spanning one library is one scan. There is no glob/ignore filtering here — per-library `ignore_rules` are applied by the scan job (`core/src/filesystem/scanner/library_scan_job.rs:205`). | `src/watcher.rs:119-123`, `src/watcher.rs:220-247` |
| The OS backend is created on the first `add_watcher` and then reused for the crate's lifetime | A server whose libraries all have watching off never allocates an inotify/FSEvents instance. | `src/watcher.rs:82-90`; tests `test_backend_is_lazy_and_reused`, `test_first_add_lazily_creates_backend` |
| Watches are always recursive | A library root is a tree; per-directory watches would miss new sub-series. | `src/watcher.rs:181-190` |
| `init` warns and skips a root that `notify` reports as `PathNotFound`, and propagates every other error | An unmounted drive must not stop the remaining libraries from being watched; anything else is a real initialization failure. | `src/watcher.rs:282-304`, `src/lib.rs:61-71`; test `test_init_skips_missing_library_path` |
| `remove_watcher` is fire-and-forget: an `unwatch` failure is logged and the listener keeps serving commands | Removing a path that was never watched (or was watched before a restart) is a legitimate no-op, and one bad unwatch must not kill watching for every library. | `src/watcher.rs:191-198`, `src/watcher.rs:257-259`; test `test_remove_twice_keeps_listener_alive` |
| Error taxonomy is `Stopped` / `Notify` / `Libraries` / `Submit { library_id }`, with owner errors as `BoxError` | Keeps `CoreError` out of the crate; `Stopped` names the command that could not be delivered, which is the only diagnosis available once the listener task is gone. | `src/lib.rs:14-15`, `src/lib.rs:43-59`, `src/watcher.rs:250-254`; core folds all of them into `CoreError::InitializationError` (`core/src/error.rs:77-82`) |

## Layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | `LibraryRoot`, `ScanRequest`, the `WatchedLibraries` / `ScanSubmitter` traits, `BoxError`, `WatcherError` + `is_path_not_found` |
| `src/watcher.rs` | `Command` enum, `notify` callback and event filter, `Listener` (debounce, accumulate, flush), `Watcher` public API, `submit_scans`, 16 `#[cfg(test)]` tests on `tempfile` roots |
| `Cargo.toml` | `notify` 8.0, `async-trait`, `thiserror`, `tokio` (`time`), `tracing`; `tempfile` for tests |

## How to verify

```text
cargo test -p stump_watcher                                                   # 16 tests
cargo test -p stump_watcher test_changed_files_are_debounced_into_one_scan    # one burst -> one scan
cargo test -p stump_watcher test_submit_scans_ignores_paths_outside_libraries # root matching
cargo check -p stump_core --no-default-features --features watcher
cargo check -p stump_server --no-default-features --features minimal
```

The last two prove both sides of the switch: `watcher = ["dep:stump_watcher"]`
on core (`core/Cargo.toml:29`) is reached from the server's `headless`
aggregate only, so the `minimal` profile links no `notify` at all.

## Deep docs

- `docs/content/docs/developer/server-architecture.mdx` — the boundary
  statement, the lazy-backend lifecycle, and the `minimal`/`headless` split.
- `docs/content/docs/developer/modular-ingest.mdx` — where a debounced change
  becomes a `library_scan` job.
