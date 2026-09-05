use std::{
	future::Future,
	sync::{Arc, Mutex},
	time::Duration,
};

use async_trait::async_trait;
use migrations::{Migrator, MigratorTrait};
use models::{
	entity::scheduled_job,
	shared::enums::{JobStatus, ScheduledJobKind},
};
use sea_orm::{
	ActiveValue::Set, Database, DatabaseBackend, DatabaseConnection, EntityTrait,
	MockDatabase,
};
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, timeout};

use crate::{
	run_job, JobContext, JobError, JobEvent, JobExecuteLog, JobExecutionContext,
	JobLifecycle, JobOutcome, JobOutputExt, JobPayload, JobRuntime, JobScheduler,
	JobTaskOutput, ScheduledJobDispatcher, WorkingState,
};

#[derive(Debug, Clone)]
enum TestJob {
	/// Runs `tasks` tasks, each taking `task_delay`
	Count { tasks: u32, task_delay: Duration },
	FailInit,
	FailPersist,
}

impl JobPayload for TestJob {
	fn name(&self) -> &'static str {
		match self {
			TestJob::Count { .. } => "count",
			TestJob::FailInit => "fail_init",
			TestJob::FailPersist => "fail_persist",
		}
	}

	fn description(&self) -> Option<String> {
		None
	}

	fn kind(&self) -> &'static str {
		"TEST"
	}
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CountOutput {
	completed: u32,
}

impl JobOutputExt for CountOutput {
	fn update(&mut self, updated: Self) {
		self.completed += updated.completed;
	}
}

struct CountJob {
	tasks: u32,
	task_delay: Duration,
}

#[async_trait]
impl JobLifecycle for CountJob {
	const NAME: &'static str = "count";
	type Context = TestHost;
	type Output = CountOutput;
	type Task = u32;

	fn description(&self) -> Option<String> {
		None
	}

	async fn init(
		&mut self,
		_ctx: &JobContext<TestHost>,
	) -> Result<WorkingState<Self::Output, Self::Task>, JobError> {
		Ok(WorkingState {
			output: None,
			tasks: (0..self.tasks).collect(),
			logs: vec![JobExecuteLog::warn("initialized")],
		})
	}

	async fn execute_task(
		&self,
		_ctx: &JobContext<TestHost>,
		_task: Self::Task,
	) -> Result<JobTaskOutput<Self>, JobError> {
		sleep(self.task_delay).await;
		Ok(JobTaskOutput {
			output: CountOutput { completed: 1 },
			subtasks: Vec::new(),
			logs: Vec::new(),
		})
	}
}

struct FailingJob;

#[async_trait]
impl JobLifecycle for FailingJob {
	const NAME: &'static str = "fail_init";
	type Context = TestHost;
	type Output = CountOutput;
	type Task = ();

	fn description(&self) -> Option<String> {
		None
	}

	async fn init(
		&mut self,
		_ctx: &JobContext<TestHost>,
	) -> Result<WorkingState<Self::Output, Self::Task>, JobError> {
		Err(JobError::InitFailed("nope".to_string()))
	}

	async fn execute_task(
		&self,
		_ctx: &JobContext<TestHost>,
		_task: Self::Task,
	) -> Result<JobTaskOutput<Self>, JobError> {
		unreachable!("init fails before any task")
	}
}

#[derive(Debug)]
struct Finished {
	id: String,
	outcome: JobOutcome,
}

#[derive(Debug, Default)]
struct Recorded {
	started: Vec<String>,
	finished: Vec<Finished>,
	events: Vec<JobEvent<CountOutput>>,
	scheduled: Vec<i32>,
}

struct TestHost {
	conn: DatabaseConnection,
	recorded: Mutex<Recorded>,
}

impl TestHost {
	fn mock() -> Arc<Self> {
		Arc::new(Self {
			conn: MockDatabase::new(DatabaseBackend::Sqlite).into_connection(),
			recorded: Mutex::default(),
		})
	}

	fn with_conn(conn: DatabaseConnection) -> Arc<Self> {
		Arc::new(Self {
			conn,
			recorded: Mutex::default(),
		})
	}

	fn recorded(&self) -> std::sync::MutexGuard<'_, Recorded> {
		self.recorded.lock().expect("recorded mutex poisoned")
	}

	fn last_queue_status(&self) -> Option<crate::JobQueueStatus> {
		self.recorded()
			.events
			.iter()
			.rev()
			.find_map(|event| match event {
				JobEvent::QueueStatus(status) => Some(status.clone()),
				_ => None,
			})
	}
}

#[async_trait]
impl JobExecutionContext for TestHost {
	type Job = TestJob;
	type Config = ();
	type Output = CountOutput;
	type Event = JobEvent<CountOutput>;

	fn conn(&self) -> &DatabaseConnection {
		&self.conn
	}

	fn config(&self) -> &() {
		&()
	}

	fn emit(&self, event: Self::Event) {
		self.recorded().events.push(event);
	}

	async fn persist_started(&self, id: &str, job: &TestJob) -> Result<(), JobError> {
		if matches!(job, TestJob::FailPersist) {
			return Err(JobError::Unknown("persist failed".to_string()));
		}
		self.recorded().started.push(id.to_string());
		Ok(())
	}

	async fn persist_finished(
		&self,
		id: &str,
		outcome: JobOutcome,
	) -> Result<(), JobError> {
		self.recorded().finished.push(Finished {
			id: id.to_string(),
			outcome,
		});
		Ok(())
	}

	async fn run(&self, job: TestJob, ctx: &JobContext<Self>) -> Result<(), JobError> {
		match job {
			TestJob::Count { tasks, task_delay } => {
				run_job(ctx, &mut CountJob { tasks, task_delay }).await
			},
			TestJob::FailInit | TestJob::FailPersist => {
				run_job(ctx, &mut FailingJob).await
			},
		}
	}
}

#[async_trait]
impl ScheduledJobDispatcher for TestHost {
	async fn dispatch_scheduled(
		&self,
		job: &scheduled_job::Model,
		runtime: &JobRuntime<Self>,
	) -> Result<(), JobError> {
		self.recorded().scheduled.push(job.id);
		runtime
			.enqueue(TestJob::Count {
				tasks: 1,
				task_delay: Duration::ZERO,
			})
			.await
	}
}

async fn wait_for<F, Fut>(what: &str, mut condition: F)
where
	F: FnMut() -> Fut,
	Fut: Future<Output = bool>,
{
	timeout(Duration::from_secs(10), async {
		while !condition().await {
			sleep(Duration::from_millis(5)).await;
		}
	})
	.await
	.unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

fn count(tasks: u32) -> TestJob {
	TestJob::Count {
		tasks,
		task_delay: Duration::ZERO,
	}
}

/// One runtime per compiled backend, each over a fresh host so recordings never mix.
fn runtimes() -> Vec<JobRuntime<TestHost>> {
	let mut runtimes = vec![JobRuntime::inline(TestHost::mock())];
	#[cfg(feature = "apalis")]
	runtimes.push(JobRuntime::apalis(TestHost::mock()));
	runtimes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_runtime_runs_job_to_completion() {
	let host = TestHost::mock();
	let runtime = JobRuntime::inline(Arc::clone(&host));
	assert_eq!(runtime.backend(), "inline");

	runtime.enqueue(count(3)).await.expect("enqueue");
	wait_for("job completion", || async { !host.recorded().finished.is_empty() })
		.await;

	let recorded = host.recorded();
	let finished = &recorded.finished[0];
	assert_eq!(recorded.started, vec![finished.id.clone()]);
	assert_eq!(finished.outcome.status, JobStatus::Completed);
	assert_eq!(finished.outcome.logs.len(), 1);
	let output: CountOutput =
		serde_json::from_slice(finished.outcome.output.as_deref().expect("output"))
			.expect("output json");
	assert_eq!(output, CountOutput { completed: 3 });

	assert!(recorded
		.events
		.iter()
		.any(|event| matches!(event, JobEvent::Started { id } if *id == finished.id)));
	assert!(recorded.events.iter().any(|event| matches!(
		event,
		JobEvent::Output { id, output } if *id == finished.id && output.completed == 3
	)));
	assert!(recorded.events.iter().any(|event| matches!(
		event,
		JobEvent::Progress(update) if update.id == finished.id
			&& update.payload.status == Some(JobStatus::Completed)
	)));
	drop(recorded);

	assert!(runtime.is_idle());
	assert_eq!(runtime.queue_depth().count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_counters_reach_zero_with_every_runtime() {
	for runtime in runtimes() {
		let host = Arc::clone(runtime.services());
		runtime
			.enqueue(count(2))
			.await
			.expect("enqueue first");
		runtime
			.enqueue(count(1))
			.await
			.expect("enqueue second");

		let depth = runtime.queue_depth();
		assert_eq!(depth.count, 2, "{}: both jobs are accounted for", runtime.backend());
		assert_eq!(depth.count_by_type.get("TEST"), Some(&2));

		wait_for("both jobs to finish", || async {
			host.recorded().finished.len() == 2
		})
		.await;

		let depth = runtime.queue_depth();
		assert_eq!(depth.count, 0, "{}: counters drained", runtime.backend());
		assert!(depth.count_by_type.is_empty());
		assert_eq!(
			host.last_queue_status().map(|status| status.count),
			Some(0),
			"{}: final queue status event reports zero",
			runtime.backend()
		);
		assert!(runtime.is_idle());
		runtime.stop().await;
	}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn init_failure_is_persisted_as_failed() {
	for runtime in runtimes() {
		let host = Arc::clone(runtime.services());
		runtime.enqueue(TestJob::FailInit).await.expect("enqueue");
		wait_for("failure to be persisted", || async {
			!host.recorded().finished.is_empty()
		})
		.await;

		let recorded = host.recorded();
		let finished = recorded.finished.last().expect("finished");
		assert_eq!(finished.outcome.status, JobStatus::Failed);
		assert!(finished.outcome.output.is_none());
		assert!(recorded.events.iter().any(|event| matches!(
			event,
			JobEvent::Progress(update) if update.id == finished.id
				&& update.payload.status == Some(JobStatus::Failed)
				&& update.payload.message.as_deref() == Some("Init failed: Job failed while initializing: nope")
		)));
		drop(recorded);
		assert_eq!(runtime.queue_depth().count, 0, "{}", runtime.backend());
		assert!(runtime.is_idle());
	}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_start_persistence_releases_the_queued_count() {
	for runtime in runtimes() {
		let host = Arc::clone(runtime.services());
		runtime.enqueue(TestJob::FailPersist).await.expect("enqueue");
		wait_for("queued count to be released", || async {
			runtime.queue_depth().count == 0
				&& host.last_queue_status().is_some_and(|status| status.count == 0)
		})
		.await;

		let recorded = host.recorded();
		assert!(recorded.started.is_empty(), "{}", runtime.backend());
		assert!(recorded.finished.is_empty(), "{}", runtime.backend());
		assert!(!recorded
			.events
			.iter()
			.any(|event| matches!(event, JobEvent::Started { .. })));
	}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_cancels_the_running_job() {
	for runtime in runtimes() {
		let host = Arc::clone(runtime.services());
		runtime
			.enqueue(TestJob::Count {
				tasks: 10_000,
				task_delay: Duration::from_millis(5),
			})
			.await
			.expect("enqueue");
		wait_for("job to start", || async { !runtime.is_idle() }).await;

		runtime.stop().await;

		let recorded = host.recorded();
		let finished = recorded.finished.last().expect("cancelled job persisted");
		assert_eq!(finished.outcome.status, JobStatus::Cancelled, "{}", runtime.backend());
		drop(recorded);
		assert!(runtime.is_idle());
		assert_eq!(runtime.queue_depth().count, 0);
		assert!(
			runtime.enqueue(count(1)).await.is_err() || runtime.backend() == "apalis",
			"the inline executor rejects work after stop"
		);
	}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_job_drives_a_lifecycle_without_the_queue() {
	let host = TestHost::mock();
	let runtime = JobRuntime::inline(Arc::clone(&host));

	let ctx = runtime
		.open_job("manual-job".to_string(), &count(2))
		.await
		.expect("open job");
	assert_eq!(ctx.job_id(), "manual-job");
	assert!(!runtime.is_idle());
	assert_eq!(runtime.queue_depth().count, 1);

	let mut job = CountJob {
		tasks: 2,
		task_delay: Duration::ZERO,
	};
	run_job(&ctx, &mut job).await.expect("run job");
	drop(ctx);

	assert!(runtime.is_idle());
	assert_eq!(runtime.queue_depth().count, 0);
	assert_eq!(host.recorded().started, vec!["manual-job".to_string()]);
	assert_eq!(
		host.recorded().finished[0].outcome.status,
		JobStatus::Completed
	);
}

async fn migrated_database() -> DatabaseConnection {
	let conn = Database::connect("sqlite::memory:")
		.await
		.expect("sqlite connection");
	Migrator::up(&conn, None).await.expect("migrations");
	conn
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_stays_idle_without_rows_and_never_creates_a_runtime() {
	let conn = migrated_database().await;
	let scheduler = JobScheduler::init(&conn, || -> Result<Arc<JobRuntime<TestHost>>, JobError> {
		panic!("runtime factory must not run without scheduled rows")
	})
	.await
	.expect("scheduler query");
	assert!(scheduler.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_fires_rows_through_the_dispatcher_and_records_last_run() {
	let conn = migrated_database().await;
	let inserted = scheduled_job::Entity::insert(scheduled_job::ActiveModel {
		name: Set("every second".to_string()),
		kind: Set(ScheduledJobKind::LibraryScan),
		schedule: Set("* * * * * *".to_string()),
		enabled: Set(true),
		created_at: Set(chrono::Utc::now()),
		..Default::default()
	})
	.exec(&conn)
	.await
	.expect("insert scheduled job");
	scheduled_job::Entity::insert(scheduled_job::ActiveModel {
		name: Set("broken".to_string()),
		kind: Set(ScheduledJobKind::LibraryScan),
		schedule: Set("not a cron".to_string()),
		enabled: Set(true),
		created_at: Set(chrono::Utc::now()),
		..Default::default()
	})
	.exec(&conn)
	.await
	.expect("insert invalid scheduled job");

	let host = TestHost::with_conn(conn);
	let conn = host.conn();
	let runtime = Arc::new(JobRuntime::inline(Arc::clone(&host)));
	let factory_runtime = Arc::clone(&runtime);
	let scheduler = JobScheduler::init(conn, move || Ok(factory_runtime))
		.await
		.expect("scheduler init")
		.expect("scheduler created for the valid row");
	assert_eq!(scheduler.job_count(), 1, "invalid cron rows are skipped");

	wait_for("scheduled dispatch and job completion", || async {
		!host.recorded().finished.is_empty()
	})
	.await;
	assert_eq!(host.recorded().scheduled[0], inserted.last_insert_id);

	wait_for("last_run_at to be recorded", || async {
		scheduled_job::Entity::find_by_id(inserted.last_insert_id)
			.one(conn)
			.await
			.expect("query")
			.and_then(|row| row.last_run_at)
			.is_some()
	})
	.await;

	scheduler.stop();
	assert_eq!(scheduler.job_count(), 0);
	runtime.stop().await;
}
