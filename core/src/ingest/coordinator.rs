use std::{collections::BTreeMap, sync::Arc};

use models::shared::enums::JobStatus;
use tokio::sync::Semaphore;

use super::{
	contract::{AnalysisPhase, DropItemStatus, IngestProgressEvent},
	progress::{CursorExpired, ProgressStream, StoredProgressStream},
	providers::ProviderRegistry,
	quality::QualityRegistry,
	store::{AnalysisJobModel, IngestStore, Pagination},
};
use crate::error::{CoreError, CoreResult};

/// Durable coordinator for staged analysis jobs. At most two jobs execute the
/// expensive snapshot/check/provider path concurrently.
#[derive(Clone)]
pub struct IngestCoordinator {
	store: IngestStore,
	quality: Arc<QualityRegistry>,
	providers: Arc<ProviderRegistry>,
	semaphore: Arc<Semaphore>,
}

impl IngestCoordinator {
	pub fn new(
		store: IngestStore,
		quality: Arc<QualityRegistry>,
		providers: Arc<ProviderRegistry>,
	) -> Self {
		Self {
			store,
			quality,
			providers,
			semaphore: Arc::new(Semaphore::new(2)),
		}
	}

	pub fn store(&self) -> &IngestStore {
		&self.store
	}

	pub async fn enqueue(
		&self,
		item_ids: Vec<String>,
		force: bool,
	) -> CoreResult<Vec<AnalysisJobModel>> {
		let mut queued = Vec::with_capacity(item_ids.len());
		for item_id in item_ids {
			let item =
				self.store.item(&item_id).await?.ok_or_else(|| {
					CoreError::NotFound(format!("ingest item {item_id}"))
				})?;
			if let Some(existing) = self.store.latest_job_for_item(&item_id).await? {
				if !force && existing.status.is_pending() {
					queued.push(existing);
					continue;
				}
			}
			let job = self.store.create_analysis_job(&item, force).await?;
			queued.push(job.clone());
			self.spawn(job.id.clone());
		}
		Ok(queued)
	}

	pub async fn pause(&self, job_id: &str) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		if job.status == JobStatus::Queued || job.status == JobStatus::Running {
			self.store
				.update_job_status(job_id, JobStatus::Paused, None)
				.await
		} else {
			Ok(job)
		}
	}

	pub async fn resume(&self, job_id: &str) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		if job.status == JobStatus::Paused {
			let job = self
				.store
				.update_job_status(job_id, JobStatus::Queued, None)
				.await?;
			self.spawn(job.id.clone());
			Ok(job)
		} else {
			Ok(job)
		}
	}

	pub async fn cancel(&self, job_id: &str) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		if job.status.is_pending() {
			self.store
				.update_job_status(job_id, JobStatus::Cancelled, Some("cancelled"))
				.await
		} else {
			Ok(job)
		}
	}

	pub async fn retry(&self, job_id: &str) -> CoreResult<AnalysisJobModel> {
		let job = self
			.job(job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		let mut jobs = self.enqueue(vec![job.drop_item_id], true).await?;
		jobs.pop().ok_or_else(|| {
			CoreError::InternalError("retry did not enqueue a job".to_string())
		})
	}

	pub async fn requeue(
		&self,
		item_id: &str,
		force: bool,
	) -> CoreResult<AnalysisJobModel> {
		let mut jobs = self.enqueue(vec![item_id.to_string()], force).await?;
		jobs.pop().ok_or_else(|| {
			CoreError::InternalError("requeue did not enqueue a job".to_string())
		})
	}

	pub async fn queue(
		&self,
		status: Option<JobStatus>,
		page: Pagination,
	) -> CoreResult<(Vec<AnalysisJobModel>, u64)> {
		self.store.queue_jobs(status, page).await
	}

	pub async fn job(&self, job_id: &str) -> CoreResult<Option<AnalysisJobModel>> {
		self.store.job(job_id).await
	}

	pub async fn subscribe(
		&self,
		library_id: Option<&str>,
		drop_item_id: Option<&str>,
		analysis_job_id: Option<&str>,
		after: Option<&str>,
	) -> Result<ProgressStream, CursorExpired> {
		self.store
			.subscribe_progress(library_id, drop_item_id, analysis_job_id, after)
			.await
	}
	pub async fn subscribe_stored(
		&self,
		library_id: Option<&str>,
		drop_item_id: Option<&str>,
		analysis_job_id: Option<&str>,
		after: Option<&str>,
	) -> Result<StoredProgressStream, CursorExpired> {
		self.store
			.subscribe_progress_stored(library_id, drop_item_id, analysis_job_id, after)
			.await
	}
	fn spawn(&self, job_id: String) {
		let coordinator = self.clone();
		tokio::spawn(async move {
			if let Err(error) = coordinator.run(job_id).await {
				tracing::error!(?error, "Ingest analysis job failed");
			}
		});
	}

	async fn run(&self, job_id: String) -> CoreResult<()> {
		let permit = self
			.semaphore
			.clone()
			.acquire_owned()
			.await
			.map_err(|error| CoreError::InternalError(error.to_string()))?;
		let job = self
			.store
			.job(&job_id)
			.await?
			.ok_or_else(|| CoreError::NotFound(format!("analysis job {job_id}")))?;
		if job.status == JobStatus::Paused || job.status == JobStatus::Cancelled {
			drop(permit);
			return Ok(());
		}
		self.store.start_job(&job_id).await?;
		let item = self.store.item(&job.drop_item_id).await?.ok_or_else(|| {
			CoreError::NotFound(format!("ingest item {}", job.drop_item_id))
		})?;
		let run_result = self.run_phases(&job, &item).await;
		drop(permit);
		match run_result {
			Ok((score, candidate_count)) => {
				self.store
					.finish_job(&job_id, score, candidate_count > 0)
					.await?;
				Ok(())
			},
			Err(error) => {
				self.store.fail_job(&job_id, &error.to_string()).await?;
				Err(error)
			},
		}
	}

	async fn run_phases(
		&self,
		job: &AnalysisJobModel,
		item: &super::store::DropItemModel,
	) -> CoreResult<(u8, usize)> {
		let total = 7;
		self.emit_phase(
			job,
			item,
			AnalysisPhase::Staging,
			1,
			total,
			None,
			"Staging source",
		)
		.await?;
		let snapshot = self.store.snapshot(&item.id).await?;
		self.emit_phase(
			job,
			item,
			AnalysisPhase::Parsing,
			2,
			total,
			None,
			"Parsing source",
		)
		.await?;
		self.emit_phase(
			job,
			item,
			AnalysisPhase::Pages,
			3,
			total,
			None,
			"Indexing pages",
		)
		.await?;
		let settings = BTreeMap::new();
		self.emit_phase(
			job,
			item,
			AnalysisPhase::Quality,
			4,
			total,
			None,
			"Running quality checks",
		)
		.await?;
		let report =
			self.quality
				.run_all(&snapshot, &settings)
				.await
				.map_err(|error| {
					CoreError::InternalError(format!("quality analysis failed: {error}"))
				})?;
		let score = report.score;
		self.store.save_report(&item.id, &report).await?;
		self.emit_phase(
			job,
			item,
			AnalysisPhase::Identify,
			5,
			total,
			Some(score),
			"Identifying metadata",
		)
		.await?;
		let candidates = self
			.providers
			.identify_and_lookup(&snapshot, &item.library_id, item.created_by.as_deref())
			.await;
		self.emit_phase(
			job,
			item,
			AnalysisPhase::Lookup,
			6,
			total,
			Some(score),
			"Looking up metadata",
		)
		.await?;
		let saved_candidates = self.store.save_candidates(&item.id, &candidates).await?;
		self.store
			.set_item_analysis_result(&item.id, score, !saved_candidates.is_empty())
			.await?;
		self.emit_phase(
			job,
			item,
			AnalysisPhase::Done,
			total,
			total,
			Some(score),
			"Analysis complete",
		)
		.await?;
		Ok((score, saved_candidates.len()))
	}

	#[allow(clippy::too_many_arguments)] // Internal progress emission keeps the event fields explicit.
	async fn emit_phase(
		&self,
		job: &AnalysisJobModel,
		item: &super::store::DropItemModel,
		phase: AnalysisPhase,
		completed: u32,
		total: u32,
		score: Option<u8>,
		message: &str,
	) -> CoreResult<()> {
		self.store
			.update_job_phase(&job.id, phase, completed == total, score)
			.await?;
		self.store
			.emit_progress(IngestProgressEvent {
				cursor: String::new(),
				library_id: item.library_id.clone(),
				drop_item_id: item.id.clone(),
				analysis_job_id: Some(job.id.clone()),
				phase,
				status: DropItemStatus::parse(&item.status)
					.unwrap_or(DropItemStatus::Analyzing),
				completed,
				total,
				score,
				message: message.to_string(),
			})
			.await?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn phase_order_is_stable() {
		assert!(AnalysisPhase::Staging < AnalysisPhase::Parsing);
		assert!(AnalysisPhase::Lookup < AnalysisPhase::Done);
	}

	#[test]
	fn queue_order_compares_priority_then_time_then_id() {
		let mut keys = [(1, 7_u64, "b"), (0, 7_u64, "a"), (0, 3_u64, "z")];
		keys.sort_by(|left, right| {
			left.0
				.cmp(&right.0)
				.then_with(|| left.1.cmp(&right.1))
				.then_with(|| left.2.cmp(right.2))
		});
		assert_eq!(
			keys.iter().map(|key| key.2).collect::<Vec<_>>(),
			["z", "a", "b"]
		);
	}
}
