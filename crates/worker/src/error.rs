use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkerError {
	#[cfg(feature = "server")]
	#[error("{0}")]
	Database(#[from] sea_orm::DbErr),
	#[error("no worker job with id {0}")]
	NotFound(String),
	/// The frame named a job the sending worker does not hold. Never fatal to
	/// the connection: a `cancel` and a `result` can cross on the wire.
	#[error("job {job_id} is not held by worker {worker_id}")]
	NotAssigned { job_id: String, worker_id: String },
	#[error("worker job {0} did not finish in time")]
	Timeout(String),
	#[error("{0}")]
	Io(#[from] std::io::Error),
	#[error("{0}")]
	Invalid(String),
}

pub type WorkerResult<T> = Result<T, WorkerError>;
