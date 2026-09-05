use stump_jobs::JobError;

use crate::CoreError;

impl From<CoreError> for JobError {
	fn from(err: CoreError) -> Self {
		match err {
			CoreError::DBError(err) => Self::DbError(err),
			_ => Self::Unknown(err.to_string()),
		}
	}
}
