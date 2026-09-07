use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use models::{
	entity::{job, library, log, metadata_fetch_record, scheduled_job},
	shared::enums::{JobStatus, MetadataFetchStatus, ScheduledJobKind},
};
use sea_orm::{prelude::*, sea_query::OnConflict, ActiveValue::Set, DatabaseConnection};
use stump_jobs::{
	run_job, JobContext, JobError, JobExecutionContext, JobOutcome, JobPayload,
	JobRuntime, ScheduledJobDispatcher,
};
use tokio::sync::broadcast;

use crate::{
	config::StumpConfig,
	event::CoreEvent,
	filesystem::media::visible_pages::VisiblePagesCache,
	filesystem::{
		image::{PlaceholderGenerationJob, ThumbnailGenerationJob},
		media::analysis::AnalyzeMediaJob,
		metadata::{MetadataFetchJob, MetadataFetchJobParams},
		scanner::{LibraryScanJob, SeriesScanJob},
	},
	job::{
		annotation_sync::AnnotationSyncJob, notification::NotificationDispatchJob,
		stump_job::StumpJob, CoreJobOutput,
	},
};

/// The host services core jobs run against: the database, the configuration, and the
/// core event channel.
///
/// This is deliberately not [`Ctx`](crate::Ctx): the job runtime holds its services for
/// as long as its executor runs, and the context owns the runtime.
pub struct JobServices {
	pub conn: Arc<DatabaseConnection>,
	pub config: Arc<StumpConfig>,
	event_tx: broadcast::Sender<CoreEvent>,
	visible_pages: Arc<VisiblePagesCache>,
	/// The provider host, shared with [`Ctx`](crate::Ctx) rather than built
	/// here: it is installed once during startup and a job must see the same
	/// one every route does. Read-only — a job never installs it, so a job
	/// that runs before the host is up simply has none.
	#[cfg(feature = "providers")]
	provider_host: Arc<std::sync::OnceLock<Arc<stump_provider::ProviderHost>>>,
}

impl JobServices {
	pub(crate) fn new(
		conn: Arc<DatabaseConnection>,
		config: Arc<StumpConfig>,
		event_tx: broadcast::Sender<CoreEvent>,
		visible_pages: Arc<VisiblePagesCache>,
	) -> Self {
		Self {
			conn,
			config,
			event_tx,
			visible_pages,
			#[cfg(feature = "providers")]
			provider_host: Arc::new(std::sync::OnceLock::new()),
		}
	}

	/// Share the context's provider-host cell, so a job sees the host the
	/// moment `crate::providers::init` installs it.
	#[cfg(feature = "providers")]
	pub(crate) fn with_provider_host(
		mut self,
		host: Arc<std::sync::OnceLock<Arc<stump_provider::ProviderHost>>>,
	) -> Self {
		self.provider_host = host;
		self
	}

	/// The provider host, if one is running.
	#[cfg(feature = "providers")]
	pub fn provider_host(&self) -> Option<Arc<stump_provider::ProviderHost>> {
		self.provider_host.get().cloned()
	}

	/// Drop the cached visible-page list for a media whose page hashes changed.
	pub fn invalidate_visible_pages(&self, media_id: &str) {
		self.visible_pages.invalidate_media(media_id);
	}

	/// Sends an event to the core event channel
	pub fn emit_event(&self, event: CoreEvent) {
		if let Err(error) = self.event_tx.send(event) {
			tracing::error!(?error, "Failed to emit core event");
		}
	}
}

#[async_trait]
impl JobExecutionContext for JobServices {
	type Job = StumpJob;
	type Config = StumpConfig;
	type Output = CoreJobOutput;
	type Event = CoreEvent;

	fn conn(&self) -> &DatabaseConnection {
		&self.conn
	}

	fn config(&self) -> &StumpConfig {
		&self.config
	}

	fn emit(&self, event: CoreEvent) {
		self.emit_event(event);
	}

	async fn persist_started(&self, id: &str, job: &StumpJob) -> Result<(), JobError> {
		let active_model = job::ActiveModel {
			id: Set(id.to_string()),
			name: Set(job.name().to_string()),
			description: Set(job.description()),
			status: Set(JobStatus::Running),
			created_at: Set(Utc::now().into()),
			ms_elapsed: Set(0),
			..Default::default()
		};

		job::Entity::insert(active_model)
			.on_conflict(
				OnConflict::column(job::Column::Id)
					.update_column(job::Column::Status)
					.to_owned(),
			)
			.exec_without_returning(self.conn.as_ref())
			.await?;

		Ok(())
	}

	async fn persist_finished(
		&self,
		id: &str,
		outcome: JobOutcome,
	) -> Result<(), JobError> {
		if !outcome.logs.is_empty() {
			let models = outcome.logs.into_iter().map(|l| log::ActiveModel {
				job_id: Set(Some(id.to_string())),
				message: Set(l.msg),
				level: Set(l.level),
				timestamp: Set(l.timestamp.into()),
				context: Set(l.context),
				..Default::default()
			});
			log::Entity::insert_many(models)
				.exec(self.conn.as_ref())
				.await?;
		}

		let mut update = job::Entity::update_many()
			.filter(job::Column::Id.eq(id))
			.col_expr(job::Column::Status, Expr::value(outcome.status.to_string()))
			.col_expr(
				job::Column::MsElapsed,
				Expr::value(outcome.elapsed.as_millis() as i64),
			)
			.col_expr(
				job::Column::CompletedAt,
				Expr::value(Some(Utc::now().fixed_offset())),
			);
		if outcome.status == JobStatus::Completed {
			update =
				update.col_expr(job::Column::OutputData, Expr::value(outcome.output));
		}
		update.exec(self.conn.as_ref()).await?;

		Ok(())
	}

	async fn run(&self, job: StumpJob, ctx: &JobContext<Self>) -> Result<(), JobError> {
		match job {
			StumpJob::LibraryScan { id, path, options } => {
				run_job(ctx, &mut LibraryScanJob::new(id, path, options)).await
			},
			StumpJob::SeriesScan { id, path, options } => {
				run_job(
					ctx,
					&mut SeriesScanJob {
						id,
						path,
						config: None,
						options: options.unwrap_or_default(),
					},
				)
				.await
			},
			StumpJob::ThumbnailGeneration { options, params } => {
				run_job(ctx, &mut ThumbnailGenerationJob { options, params }).await
			},
			StumpJob::PlaceholderGeneration { config } => {
				run_job(ctx, &mut PlaceholderGenerationJob { config }).await
			},
			StumpJob::MetadataFetch { params } => {
				run_job(
					ctx,
					&mut MetadataFetchJob {
						params,
						provider_cache: None,
					},
				)
				.await
			},
			StumpJob::AnalyzeMedia { config } => {
				run_job(ctx, &mut AnalyzeMediaJob { config }).await
			},
			StumpJob::NotificationDispatch { deliveries } => {
				run_job(ctx, &mut NotificationDispatchJob::new(deliveries)).await
			},
			StumpJob::AnnotationSync { user_id } => {
				run_job(ctx, &mut AnnotationSyncJob::new(user_id)).await
			},
			#[cfg(feature = "providers")]
			StumpJob::ProviderGc => run_provider_gc(self).await,
			#[cfg(feature = "providers")]
			StumpJob::ProviderSourceHealth => {
				run_job(
					ctx,
					&mut crate::job::provider_health::ProviderSourceHealthJob::new(),
				)
				.await
			},
		}
	}
}

#[cfg(feature = "providers")]
async fn run_provider_gc(services: &JobServices) -> Result<(), JobError> {
	let cutoff = crate::providers::gc_cutoff(services.config());
	let thumbnails_dir = services.config().get_thumbnails_dir();
	let report = stump_provider::gc_materialised_series(
		services.conn(),
		Some(&thumbnails_dir),
		cutoff,
	)
	.await
	.map_err(|error| JobError::Unknown(error.to_string()))?;

	if report.series.is_empty() && report.media.is_empty() {
		return Ok(());
	}

	for media in &report.media {
		services.emit(CoreEvent::MediaDeleted(crate::event::MediaDeleted {
			id: media.id.clone(),
			series_id: media.series_id.clone(),
			library_id: media.library_id.clone(),
		}));
	}
	for series in &report.series {
		services.emit(CoreEvent::SeriesDeleted(crate::event::SeriesDeleted {
			id: series.id.clone(),
			library_id: series.library_id.clone(),
		}));
	}

	Ok(())
}

#[async_trait]
impl ScheduledJobDispatcher for JobServices {
	async fn dispatch_scheduled(
		&self,
		job: &scheduled_job::Model,
		runtime: &JobRuntime<Self>,
	) -> Result<(), JobError> {
		match job.kind {
			ScheduledJobKind::LibraryScan => dispatch_library_scan(job, runtime).await,
			ScheduledJobKind::MetadataRetry => {
				dispatch_metadata_retry(job, runtime).await
			},
		}
	}
}

async fn dispatch_library_scan(
	job: &scheduled_job::Model,
	runtime: &JobRuntime<JobServices>,
) -> Result<(), JobError> {
	let config = job
		.library_scan_config()
		.ok_or_else(|| JobError::Unknown("Invalid scheduled scan config".to_string()))?;
	let conn = runtime.services().conn();

	let libraries = if config.library_ids.is_empty() {
		library::Entity::find().all(conn).await?
	} else {
		library::Entity::find()
			.filter(library::Column::Id.is_in(config.library_ids.clone()))
			.all(conn)
			.await?
	};

	if libraries.is_empty() {
		tracing::warn!("No libraries found for scheduled scan");
		return Ok(());
	}

	for lib in libraries {
		tracing::info!(
			library_name = %lib.name,
			"Enqueuing library scan from scheduler"
		);
		runtime
			.enqueue(StumpJob::library_scan(
				lib.id.clone(),
				lib.path.clone(),
				None,
			))
			.await?;
	}

	Ok(())
}

async fn dispatch_metadata_retry(
	job: &scheduled_job::Model,
	runtime: &JobRuntime<JobServices>,
) -> Result<(), JobError> {
	let config = job.metadata_retry_config();

	let statuses = config
		.as_ref()
		.map(|c| c.statuses.clone())
		.unwrap_or_else(|| vec![MetadataFetchStatus::RateLimited]);

	let records = metadata_fetch_record::Entity::find()
		.filter(metadata_fetch_record::Column::Status.is_in(statuses))
		.all(runtime.services().conn())
		.await?;

	if records.is_empty() {
		tracing::debug!(
			id = job.id,
			name = %job.name,
			"No records to retry"
		);
		return Ok(());
	}

	let series_ids: Vec<String> =
		records.iter().filter_map(|r| r.series_id.clone()).collect();
	let media_ids: Vec<String> =
		records.iter().filter_map(|r| r.media_id.clone()).collect();

	if !series_ids.is_empty() {
		tracing::info!(
			count = series_ids.len(),
			"Enqueuing metadata retry for series"
		);
		let params = MetadataFetchJobParams::series(series_ids);
		runtime.enqueue(StumpJob::metadata_fetch(params)).await?;
	}

	if !media_ids.is_empty() {
		tracing::info!(
			count = media_ids.len(),
			"Enqueuing metadata retry for media"
		);
		let params = MetadataFetchJobParams::media(media_ids);
		runtime.enqueue(StumpJob::metadata_fetch(params)).await?;
	}

	Ok(())
}
