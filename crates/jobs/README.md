# stump_jobs

## Purpose

`stump_jobs` is the job-type-agnostic half of Stump's background work: the
queue, the single executor draining it, the typed queue counters, the
cancellation registry, the per-execution `JobContext<X>`, the `JobLifecycle`
contract (`init` → `execute_task` loop → `finalize`, driven by `run_job`), and
the cron `JobScheduler`. It never names a concrete job. The host supplies a
`JobExecutionContext` (database + config access, event sink, job-record
persistence, and the dispatch table from a queued payload to work), so every
concrete job — library scan, thumbnail generation, media analysis, metadata
fetch, annotation sync, notifications, provider health — stays in
`core/src/job/` and its siblings under `core/src/filesystem/`, implemented
against `JobServices` (`core/src/job/services.rs:37-77`, `:243`).

## Reference / upstream

| Reference                                                                                                                      | Relation                                                                                                                                           |
| ------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Upstream Stump's in-core job system (`core/src/job/`)                                                                          | Extracted into this crate in `bc347a78`, integrated (workspace compiling, crate tests green) in `7617c66d`. Concrete jobs stayed behind in `core`. |
| `apalis` 0.7 (workspace `Cargo.toml:31`, features `limit`, `tracing`)                                                          | Default queue backend: `MemoryStorage` plus one `Worker` at concurrency 1. Optional dependency, not a hard requirement.                            |
| `croner` 3.0.1 (workspace `Cargo.toml:47`)                                                                                     | Cron expression parsing and next-occurrence arithmetic for scheduled-job rows.                                                                     |
| `models::entity::scheduled_job`                                                                                                | The only persistence this crate queries directly (enabled rows, `last_run_at` write-back).                                                         |
| `docs/content/docs/developer/server-architecture.mdx` (§ "Current startup lifecycle" step 5, § "Package seams" → `stump_jobs`) | Boundary statement and lazy-runtime lifecycle.                                                                                                     |

## Decisions

| Decision                                                                                                                                                                       | Why                                                                                                                                                                                            | Evidence                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `apalis` is a default feature but an interchangeable backend: `JobRuntime::new` picks the Apalis worker with it, the inline executor without it                                | Keeps the minimal server graph free of `apalis` while the default build gets a real queue backend; both executors share dispatch, counters, cancellation and `JobContext`.                     | `Cargo.toml:7-11`, `src/runtime.rs:74-118`                                                                                                 |
| The inline executor runs each payload on the Tokio blocking pool via `spawn_blocking` + `block_on`, one at a time over an unbounded channel                                    | A scan must not compete with request handling for async worker threads; no queue backend is needed.                                                                                            | `src/inline_executor.rs:1-3,22-42`                                                                                                         |
| `core` links `stump_jobs` with `default-features = false`; the feature is re-exposed as `stump_core/apalis` → server `apalis`                                                  | Feature choice belongs to the binary: server `apalis = ["stump_core/apalis"]` is in `headless`/`full`, absent from `minimal`.                                                                  | `core/Cargo.toml:7-8,56`, `apps/server/Cargo.toml` (`headless`, `minimal`)                                                                 |
| The Apalis worker is run directly, not under a `Monitor`                                                                                                                       | The monitor's shutdown signal only wakes a worker with a task in flight, so an idle worker would block until the terminator timeout; `Worker::stop` resolves immediately.                      | `src/apalis_executor.rs:1-5,21-34`                                                                                                         |
| `graphql` is derives-only (`async_graphql::SimpleObject` on `JobUpdate`, `JobProgress`, `JobQueueStatus`) and forwards `models/graphql`                                        | Lets the minimal graph exclude `async-graphql` without changing the runtime.                                                                                                                   | `Cargo.toml:12`, `src/progress.rs:7,24`, `src/queue.rs:7`                                                                                  |
| Progress is not a channel or stream owned here: `JobContext::report_progress` wraps a `JobProgress` patch in `JobEvent::Progress(JobUpdate)` and hands it to the host's `emit` | The host already owns an event stream (core's `CoreEvent`); a second broadcast channel would duplicate fan-out and lifetimes. `JobProgress` is a patch — absent fields are ignored by clients. | `src/worker.rs:41-43,164-193`, `src/context.rs:24-35`, `src/progress.rs:19-44`                                                             |
| Cancellation is a `tokio_util::sync::CancellationToken` per job in a `DashMap` keyed by job id; `cancel_job` returns whether a token existed                                   | Cooperative checks (`is_canceled`) inside long task loops, plus `cancel_all` on `stop`/`Drop`; the map is the source of truth for `is_idle`.                                                   | `src/worker.rs:24-29,56-71,109-131`, `src/runtime.rs:140-147`                                                                              |
| Queue counters live in-process (`BTreeMap` behind a `Mutex`), and every transition returns the new snapshot which is published as `JobEvent::QueueStatus`                      | Depth must be reportable without querying persistence or waking the executor; decrements clamp at zero so a failed enqueue or stray finish never goes negative.                                | `src/queue.rs:37-91`, tests `queue_state_reports_queued_running_and_zero_transitions`, `failed_enqueue_and_stray_finish_never_go_negative` |
| `JobContext::finish` unregisters the token and releases the running counter exactly once (`AtomicBool` swap), and `Drop` calls it                                              | A job that panics or is dropped mid-flight must not leak a running count or a cancellation token.                                                                                              | `src/worker.rs:249-262`                                                                                                                    |
| `stop` cancels running jobs, signals the executor, then waits `SHUTDOWN_GRACE` (30 s) before aborting the task                                                                 | Bounded shutdown: an in-flight job gets a chance to persist its terminal state, a wedged one does not block exit.                                                                              | `src/runtime.rs:14-15,159-183`                                                                                                             |
| `JobScheduler::init` takes a runtime **factory** and returns `Option<Self>`: no scheduler, task, or runtime is created when no valid enabled row exists                        | The job runtime is a lazy resource in `Ctx` (`OnceLock`); an idle deployment should not construct a queue and executor. Invalid cron rows are logged and skipped, not fatal.                   | `src/scheduler.rs:38-62,115-164`, `core/src/context.rs:43-53,179-184,352-374`                                                              |
| Host-owned durable maintenance is an optional scheduler loop, separate from cron rows                                                                                          | Persisted retry/poll timestamps are scanned and enqueued without sleeping in a worker permit; hosts opt in at startup through `init_with_maintenance`, while the default `init` remains lazy.  | `src/scheduler.rs`, `core/src/job/services.rs`                                                                                             |
| `JobError` has four variants only (`InitFailed`, `TaskFailed`, `DbError`, `Unknown`), with `EntityError` folded into `Unknown`                                                 | The runtime cannot know host-specific failure modes; `core` maps these into `CoreError` (`DbError` preserved as `DBError`).                                                                    | `src/error.rs:3-19`, `core/src/error.rs:83-86`                                                                                             |
| `run_job` persists a terminal state on every exit path (init failure, task failure, cancellation, finalize failure, success)                                                   | A job record must never stay `Running` after the executor moved on.                                                                                                                            | `src/lifecycle.rs:160-229`                                                                                                                 |
| `open_job` exposes a `JobContext` without queueing                                                                                                                             | Tests and benchmarks drive a lifecycle directly; the executor stays a single-consumer loop.                                                                                                    | `src/runtime.rs:149-157`, test `open_job_drives_a_lifecycle_without_the_queue`                                                             |

## Layout

| File                     | Responsibility                                                                                             |
| ------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `src/lib.rs`             | Crate doc, module wiring, public surface (`JobRuntime`, `JobLifecycle`, `JobContext`, `JobScheduler`, …)   |
| `src/context.rs`         | `JobPayload`, `JobExecutionContext` (host services trait), `JobEvent`, `JobOutcome`                        |
| `src/lifecycle.rs`       | `JobLifecycle`, `JobOutputExt`, `WorkingState`, `JobTaskOutput`, `JobExecuteLog`, `run_job`                |
| `src/runtime.rs`         | `JobRuntime` — executor selection, `enqueue`, `cancel_job`, `is_idle`, `open_job`, graceful `stop`, `Drop` |
| `src/worker.rs`          | `WorkerState` (services, sender, token registry, counters), shared `dispatch`, `JobContext`                |
| `src/queue.rs`           | `JobQueueStatus` and the clamped queued/running counters plus their unit tests                             |
| `src/progress.rs`        | `JobUpdate` and the `JobProgress` patch constructors                                                       |
| `src/scheduler.rs`       | `ScheduledJobDispatcher`, `JobScheduler` (`init`/`reload`/`stop`), cron loading and `cron_loop`            |
| `src/apalis_executor.rs` | Apalis `MemoryStorage` + single worker, feature-gated                                                      |
| `src/inline_executor.rs` | Channel-fed blocking-pool executor, always compiled                                                        |
| `src/tests.rs`           | Test host (`TestHost`, `TestJob`, `CountJob`) and the runtime/scheduler integration tests                  |

## How to verify

```text
cargo test -p stump_jobs                      # both executors: runtimes() adds the Apalis runtime
cargo test -p stump_jobs --no-default-features # inline-only path
```

Executor swap: `inline_runtime_runs_job_to_completion`,
`typed_counters_reach_zero_with_every_runtime`, `stop_cancels_the_running_job`,
`init_failure_is_persisted_as_failed`,
`failed_start_persistence_releases_the_queued_count` (the latter four loop over
`runtimes()`, `src/tests.rs:275-281`). Scheduling:
`scheduler_stays_idle_without_rows_and_never_creates_a_runtime`,
`scheduler_fires_rows_through_the_dispatcher_and_records_last_run` (skips the
invalid cron row, asserts `last_run_at`). Progress/output events:
`inline_runtime_runs_job_to_completion` asserts `JobEvent::Started`,
`JobEvent::Output`, and a `JobEvent::Progress` with `status = Completed`.

## Deep docs

- `docs/content/docs/developer/server-architecture.mdx` — jobs boundary (§ "Background job execution lives in `stump_jobs`"), lazy runtime lifecycle, `minimal` feature profile
