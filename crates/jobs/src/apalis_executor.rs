//! The Apalis-backed executor: an in-memory queue drained by a single worker.
//!
//! The worker is run directly rather than through an Apalis `Monitor`: the monitor's
//! shutdown signal only wakes a worker that has a task in flight, so an idle worker
//! would wait for the terminator timeout, while `Worker::stop` resolves immediately.

use std::sync::Arc;

use apalis::{
	layers::WorkerBuilderExt,
	prelude::{Context, Data, MemoryStorage, Worker, WorkerBuilder, WorkerFactoryFn},
};
use tokio::task::JoinHandle;

use crate::{
	worker::{dispatch, WorkerState},
	JobExecutionContext,
};

/// Spawns the worker task and returns it with the handle that stops it gracefully.
pub(crate) fn spawn<X: JobExecutionContext>(
	worker: Arc<WorkerState<X>>,
	storage: MemoryStorage<X::Job>,
) -> (JoinHandle<()>, Worker<Context>) {
	let runnable = WorkerBuilder::new("stump-worker")
		.enable_tracing()
		.data(worker)
		.concurrency(1)
		.backend(storage)
		.build_fn(handle::<X>)
		.run();
	let handle = runnable.get_handle();
	(tokio::spawn(runnable), handle)
}

/// The Apalis handler function for every queued payload
async fn handle<X: JobExecutionContext>(
	job: X::Job,
	worker: Data<Arc<WorkerState<X>>>,
) -> Result<(), apalis::prelude::Error> {
	dispatch(Arc::clone(&worker), job)
		.await
		.map_err(|error| apalis::prelude::Error::Failed(Arc::new(Box::new(error))))
}
