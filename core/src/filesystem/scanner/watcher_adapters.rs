//! Core-side implementations of the `stump_watcher` boundary traits.
//!
//! The watcher crate knows nothing about SeaORM or the Apalis queue; these adapters supply the
//! watched library roots from the database and turn scan requests into `StumpJob::LibraryScan`.

use std::sync::Arc;

use async_trait::async_trait;
use models::entity::{library, library_config};
use sea_orm::{prelude::*, Select};
use stump_watcher::{
	BoxError, LibraryRoot, ScanRequest, ScanSubmitter, WatchedLibraries,
};

use crate::job::{stump_job::StumpJob, JobServices};

/// Ready libraries whose configuration enables filesystem watching.
pub(crate) fn watched_libraries_query() -> Select<library::Entity> {
	library::Entity::find()
		.inner_join(library_config::Entity)
		.filter(library::Column::Status.eq("READY"))
		.filter(library_config::Column::Watch.eq(true))
}

/// [`WatchedLibraries`] backed by the SeaORM connection.
pub(crate) struct WatchedLibraryRoots {
	pub conn: Arc<DatabaseConnection>,
}

#[async_trait]
impl WatchedLibraries for WatchedLibraryRoots {
	#[tracing::instrument(skip(self), err)]
	async fn enabled_libraries(&self) -> Result<Vec<LibraryRoot>, BoxError> {
		let libraries = watched_libraries_query()
			.into_partial_model::<library::LibraryIdentSelect>()
			.all(self.conn.as_ref())
			.await?;

		Ok(libraries
			.into_iter()
			.map(|library| LibraryRoot {
				id: library.id,
				path: library.path,
			})
			.collect())
	}
}

/// [`ScanSubmitter`] that enqueues a library scan on the job runtime.
pub(crate) struct EnqueueLibraryScan {
	pub runtime: Arc<stump_jobs::JobRuntime<JobServices>>,
}

#[async_trait]
impl ScanSubmitter for EnqueueLibraryScan {
	async fn submit(&self, request: ScanRequest) -> Result<(), BoxError> {
		self.runtime
			.enqueue(StumpJob::library_scan(request.library_id, request.path, None))
			.await
			.map_err(|error| error.into())
	}
}
