use std::{collections::BTreeMap, sync::Arc};

use models::shared::enums::JobStatus;
use tokio::sync::{Semaphore, broadcast};

use super::{
	contract::{
		AnalysisPhase, DropItemStatus, IngestProgressEvent, QualityReport, QualityStatus,
	},
	progress::{CursorExpired, ProgressStream, StoredProgressStream},
	providers::ProviderRegistry,
	quality::QualityRegistry,
	store::{AnalysisJobModel, AnalysisTarget, IngestStore, Pagination},
};
use crate::{
	error::{CoreError, CoreResult},
	event::{
		AnalysisJobFailed, CoreEvent, IngestAwaitingReview, ProviderMatchDone,
		QualityFailed,
	},
};

/// Durable coordinator for staged analysis jobs. At most two jobs execute the
/// expensive snapshot/check/provider path concurrently.
#[derive(Clone)]
pub struct IngestCoordinator {
	store: IngestStore,
	quality: Arc<QualityRegistry>,
	providers: Arc<ProviderRegistry>,
	semaphore: Arc<Semaphore>,
	/// Core event sink for notification-routed outcomes (quality failures,
	/// review waits, provider matches, job failures). `None` in tests.
	events: Option<broadcast::Sender<CoreEvent>>,
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
			events: None,
		}
	}

	/// Attach the core event channel so analysis outcomes are announced
	/// (and routable to notification channels).
	pub fn with_event_tx(mut self, events: broadcast::Sender<CoreEvent>) -> Self {
		self.events = Some(events);
		self
	}

	fn notify(&self, event: CoreEvent) {
		if let Some(sender) = &self.events {
			let _ = sender.send(event);
		}
	}

	fn failed_check_ids(report: &QualityReport) -> Vec<String> {
		report
			.checks
			.iter()
			.filter(|check| check.outcome.status == QualityStatus::Fail)
			.map(|check| check.outcome.check_id.clone())
			.collect()
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

	/// Enqueue one library-rework analysis job over existing media rows. The
	/// job runs the same quality checks and provider identify as staged
	/// analysis, storing reports and candidates against the media ids.
	pub async fn enqueue_media(
		&self,
		media_ids: Vec<String>,
		providers: Option<Vec<String>>,
		force: bool,
	) -> CoreResult<AnalysisJobModel> {
		if media_ids.is_empty() {
			return Err(CoreError::BadRequest("no media selected".to_string()));
		}
		for media_id in &media_ids {
			self.store
				.media(media_id)
				.await?
				.ok_or_else(|| CoreError::NotFound(format!("media {media_id}")))?;
		}
		if !force {
			if let Some(existing) = self
				.store
				.latest_pending_media_job_matching(&media_ids)
				.await?
			{
				return Ok(existing);
			}
		}
		let job = self
			.store
			.create_media_analysis_job(&media_ids, providers.as_deref())
			.await?;
		self.spawn(job.id.clone());
		Ok(job)
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
		let targets = self.store.job_targets(&job)?;
		let jobs = match targets.as_slice() {
			[target] if matches!(target, AnalysisTarget::DropItem(_)) => {
				self.enqueue(vec![target.id().to_string()], true).await?
			},
			targets => {
				let media_ids = targets
					.iter()
					.map(|target| target.id().to_string())
					.collect();
				vec![self.enqueue_media(media_ids, None, true).await?]
			},
		};
		jobs.into_iter().next().ok_or_else(|| {
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
		let targets = self.store.job_targets(&job)?;
		let run_result = match targets.as_slice() {
			[target] if matches!(target, AnalysisTarget::DropItem(_)) => {
				let item = self.store.item(target.id()).await?.ok_or_else(|| {
					CoreError::NotFound(format!("ingest item {}", target.id()))
				})?;
				self.run_phases(&job, &item).await
			},
			targets
				if targets
					.iter()
					.all(|target| matches!(target, AnalysisTarget::Media(_))) =>
			{
				self.run_media_phases(&job, targets).await
			},
			_ => Err(CoreError::InternalError(format!(
				"analysis job {job_id} has empty or mixed targets"
			))),
		};
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
				self.notify(CoreEvent::AnalysisJobFailed(AnalysisJobFailed {
					analysis_job_id: job_id.to_string(),
					error: error.to_string(),
				}));
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
		let failed_checks = Self::failed_check_ids(&report);
		if !failed_checks.is_empty() {
			self.notify(CoreEvent::QualityFailed(QualityFailed {
				library_id: item.library_id.clone(),
				drop_item_id: Some(item.id.clone()),
				media_id: None,
				created_by: item.created_by.clone(),
				score,
				failed_checks,
			}));
		}
		let updated = self
			.store
			.set_item_analysis_result(&item.id, score, !saved_candidates.is_empty())
			.await?;
		if !saved_candidates.is_empty() {
			self.notify(CoreEvent::ProviderMatchDone(ProviderMatchDone {
				library_id: item.library_id.clone(),
				drop_item_id: item.id.clone(),
				created_by: item.created_by.clone(),
				candidate_count: saved_candidates.len(),
			}));
		}
		if updated.status == DropItemStatus::AwaitingReview {
			self.notify(CoreEvent::IngestAwaitingReview(IngestAwaitingReview {
				library_id: item.library_id.clone(),
				drop_item_id: item.id.clone(),
				source_filename: item.source_filename.clone(),
				created_by: item.created_by.clone(),
			}));
		}
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

	/// Library-wide rework: run the same quality/provider phases over each
	/// targeted media row. Reports and candidates persist against the media
	/// ids; the job's score is the lowest across the batch (the rework
	/// ordering). Progress advances the job row only — the persisted event
	/// feed is keyed by drop item, which a media target does not have.
	async fn run_media_phases(
		&self,
		job: &AnalysisJobModel,
		targets: &[AnalysisTarget],
	) -> CoreResult<(u8, usize)> {
		let total = targets.len() as u32;
		let settings = BTreeMap::new();
		let mut worst_score = u8::MAX;
		let mut candidate_total = 0_usize;
		for (raw_index, target) in targets.iter().enumerate() {
			let index = raw_index as u32;
			let media_id = match target {
				AnalysisTarget::Media(id) => id,
				AnalysisTarget::DropItem(id) => {
					return Err(CoreError::InternalError(format!(
						"library rework job {} unexpectedly targets drop item {id}",
						job.id
					)));
				},
			};
			let media_row = self
				.store
				.media(media_id)
				.await?
				.ok_or_else(|| CoreError::NotFound(format!("media {media_id}")))?;
			let library_id = self.store.media_library(&media_row).await?;
			self.media_phase(job, AnalysisPhase::Staging, index, total, None, media_id)
				.await?;
			let snapshot = self.store.media_snapshot(media_id).await?;
			self.media_phase(job, AnalysisPhase::Parsing, index, total, None, media_id)
				.await?;
			self.media_phase(job, AnalysisPhase::Pages, index, total, None, media_id)
				.await?;
			self.media_phase(job, AnalysisPhase::Quality, index, total, None, media_id)
				.await?;
			let report =
				self.quality
					.run_all(&snapshot, &settings)
					.await
					.map_err(|error| {
						CoreError::InternalError(format!(
							"quality analysis failed: {error}"
						))
					})?;
			let score = report.score;
			self.store.save_report_for_media(media_id, &report).await?;
			self.media_phase(
				job,
				AnalysisPhase::Identify,
				index,
				total,
				Some(score),
				media_id,
			)
			.await?;
			let selected = super::store::analysis_providers_from_plan(&job.plan);
			let candidates = self
				.providers
				.identify_and_lookup_selected(
					&snapshot,
					&library_id,
					None,
					selected.as_deref(),
				)
				.await;
			let saved = self
				.store
				.save_candidates_for_media(media_id, &candidates)
				.await?;
			candidate_total += saved.len();
			worst_score = worst_score.min(score);
			self.media_phase(
				job,
				AnalysisPhase::Done,
				index + 1,
				total,
				Some(score),
				media_id,
			)
			.await?;
		}
		Ok((worst_score, candidate_total))
	}

	/// Phase bookkeeping for media targets: the job row advances so
	/// pause/resume/cancel/queue and polling stay accurate, but no progress
	/// event is persisted (events are drop-item keyed).
	async fn media_phase(
		&self,
		job: &AnalysisJobModel,
		phase: AnalysisPhase,
		completed: u32,
		total: u32,
		score: Option<u8>,
		media_id: &str,
	) -> CoreResult<()> {
		tracing::debug!(
			job_id = %job.id,
			media_id,
			?phase,
			completed,
			total,
			"Library rework analysis phase"
		);
		self.store
			.update_job_phase(&job.id, phase, completed == total, score)
			.await?;
		Ok(())
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

	/// Library-wide rework: quality checks run against a real fixture CBZ
	/// referenced as a media row, the report persists against the media id,
	/// and the embedded provider's candidate is stored for the media target.
	#[tokio::test]
	async fn library_rework_runs_quality_checks_against_media_rows() {
		use crate::ingest::contract::IngestMetadataProvider;
		use crate::ingest::providers::EmbeddedProvider;
		let embedded_provider_id = EmbeddedProvider::new().id().to_string();
		use migrations::MigratorTrait;
		use models::{
			entity::{library, library_config, media, series},
			shared::enums::FileStatus,
		};
		use sea_orm::{ActiveModelTrait, Database, Set};
		use std::io::Write as _;

		let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		migrations::Migrator::up(conn.as_ref(), None).await.unwrap();
		let config = <library_config::ActiveModel as std::default::Default>::default()
			.insert(conn.as_ref())
			.await
			.unwrap();
		library::ActiveModel {
			id: Set("library".to_string()),
			name: Set("Library".to_string()),
			path: Set("/tmp/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();
		series::ActiveModel {
			id: Set("series".to_string()),
			name: Set("Saga".to_string()),
			path: Set("/tmp/library/Saga".to_string()),
			status: Set(FileStatus::Ready),
			library_id: Set(Some("library".to_string())),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();

		// Real two-page CBZ so the snapshot and the archive checks run the
		// same code path as a staged item.
		let temp = tempfile::tempdir().unwrap();
		let fixture_path = temp.path().join("Saga 001.cbz");
		{
			let file = std::fs::File::create(&fixture_path).unwrap();
			let mut archive = zip::ZipWriter::new(file);
			let options = zip::write::SimpleFileOptions::default()
				.compression_method(zip::CompressionMethod::Stored);
			for name in ["page-1.png", "page-2.png"] {
				archive.start_file(name, options).unwrap();
				archive.write_all(&png_fixture(40, 60)).unwrap();
			}
			archive.finish().unwrap();
		}
		let (source_sha256, byte_size) = crate::ingest::staging::hash_file(&fixture_path)
			.await
			.unwrap();
		media::ActiveModel {
			id: Set("media-1".to_string()),
			name: Set("Saga 001".to_string()),
			size: Set(byte_size as i64),
			extension: Set("cbz".to_string()),
			pages: Set(2),
			hash: Set(Some(source_sha256)),
			path: Set(fixture_path.to_string_lossy().into_owned()),
			series_id: Set(Some("series".to_string())),
			..Default::default()
		}
		.insert(conn.as_ref())
		.await
		.unwrap();

		let stump_config = Arc::new(crate::config::StumpConfig::debug());
		let store =
			super::super::store::IngestStore::new(stump_config.clone(), conn.clone());
		let quality = Arc::new(QualityRegistry::builtin(conn.clone()));
		let providers =
			Arc::new(ProviderRegistry::new(stump_config.clone(), conn.clone()));
		let coordinator = IngestCoordinator::new(store, quality, providers);

		let job = coordinator
			.enqueue_media(vec!["media-1".to_string()], None, true)
			.await
			.unwrap();
		// Run in-process instead of waiting on the spawned task so the test
		// is deterministic.
		coordinator.run(job.id.clone()).await.unwrap();

		let finished = coordinator.job(&job.id).await.unwrap().unwrap();
		assert_eq!(finished.status, JobStatus::Completed);

		let report = coordinator
			.store()
			.report_for_media("media-1")
			.await
			.unwrap()
			.expect("quality report stored against the media row");
		assert_eq!(report.media_id.as_deref(), Some("media-1"));
		assert_eq!(report.drop_item_id, None);
		assert!((0..=100).contains(&report.score));

		let candidates = coordinator
			.store()
			.candidates_for_media("media-1")
			.await
			.unwrap();
		assert!(
			candidates
				.iter()
				.any(|candidate| candidate.provider_id == embedded_provider_id),
			"expected the embedded provider candidate, got {:?}",
			candidates
				.iter()
				.map(|candidate| candidate.provider_id.as_str())
				.collect::<Vec<_>>()
		);
		assert!(candidates
			.iter()
			.all(|candidate| candidate.drop_item_id.is_none()
				&& candidate.media_id.as_deref() == Some("media-1")));
	}

	fn png_fixture(width: u32, height: u32) -> Vec<u8> {
		use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
		use std::io::Cursor;

		let image = ImageBuffer::from_pixel(width, height, Rgba([255, 0, 0, 255]));
		let mut output = Cursor::new(Vec::new());
		DynamicImage::ImageRgba8(image)
			.write_to(&mut output, ImageFormat::Png)
			.expect("encode PNG fixture");
		output.into_inner()
	}
}
