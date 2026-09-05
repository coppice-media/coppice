use models::error::EntityError;

#[derive(Debug, thiserror::Error)]
pub enum JobError {
	#[error("Job failed while initializing: {0}")]
	InitFailed(String),
	#[error("A task experienced a critical error while executing: {0}")]
	TaskFailed(String),
	#[error("A query error occurred: {0}")]
	DbError(#[from] sea_orm::DbErr),
	#[error("An unknown error occurred: {0}")]
	Unknown(String),
}

impl From<EntityError> for JobError {
	fn from(err: EntityError) -> Self {
		Self::Unknown(err.to_string())
	}
}
