//! An executor that needs no queue backend: payloads arrive over an unbounded channel and
//! run one at a time on the Tokio blocking pool, so a scan never competes with request
//! handling for the async worker threads.

use std::sync::Arc;

use tokio::{
	sync::{mpsc, Notify},
	task::JoinHandle,
};

use crate::{
	worker::{dispatch, WorkerState},
	JobExecutionContext,
};

pub(crate) fn spawn<X: JobExecutionContext>(
	worker: Arc<WorkerState<X>>,
	mut receiver: mpsc::UnboundedReceiver<X::Job>,
	shutdown: Arc<Notify>,
) -> JoinHandle<()> {
	tokio::spawn(async move {
		loop {
			let job = tokio::select! {
				biased;
				() = shutdown.notified() => break,
				job = receiver.recv() => match job {
					Some(job) => job,
					None => break,
				},
			};

			let worker = Arc::clone(&worker);
			let handle = tokio::runtime::Handle::current();
			let finished = tokio::task::spawn_blocking(move || {
				handle.block_on(dispatch(worker, job))
			})
			.await;
			if let Err(error) = finished {
				tracing::error!(?error, "Inline job executor panicked");
			}
		}
		tracing::debug!("Inline job executor stopped");
	})
}
