use serde::{Deserialize, Serialize};
use stump_jobs::JobPayload;
use stump_scanner::ScanOptions;

use crate::filesystem::{
	image::{PlaceholderGenerationJobConfig, ThumbnailGenerationJobParams},
	media::analysis::AnalysisJobConfig,
	metadata::MetadataFetchJobParams,
};

use models::shared::image_processor_options::ImageProcessorOptions;

/// A unified job enum that can represent any job in the system.
/// This is the type queued by the job runtime and is what
/// gets enqueued via `Ctx::enqueue()`.
///
/// Each variant contains the data needed to construct and run the corresponding job.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StumpJob {
	LibraryScan {
		id: String,
		path: String,
		options: Option<ScanOptions>,
	},
	SeriesScan {
		id: String,
		path: String,
		options: Option<ScanOptions>,
	},
	ThumbnailGeneration {
		options: ImageProcessorOptions,
		params: ThumbnailGenerationJobParams,
	},
	PlaceholderGeneration {
		config: PlaceholderGenerationJobConfig,
	},
	MetadataFetch {
		params: MetadataFetchJobParams,
	},
	AnalyzeMedia {
		config: AnalysisJobConfig,
	},
	/// Reclaims materialised provider series with no reading head older than
	/// `provider_gc_days`. Scheduled internally; see `core/src/providers.rs`.
	#[cfg(feature = "providers")]
	ProviderGc,
	/// Probes every catalog source's base URL and records `source_health`
	/// rows; see `core/src/job/provider_health.rs`.
	#[cfg(feature = "providers")]
	ProviderSourceHealth,
	/// Delivers queued notification targets; see `core/src/job/notification.rs`.
	NotificationDispatch {
		deliveries: Vec<crate::job::notification::QueuedDelivery>,
	},
	/// Exports one user's annotations to their enabled sinks; see
	/// `core/src/annotation_sync.rs`.
	AnnotationSync {
		user_id: String,
	},
	/// Advances one approved/automated book request through search, selection,
	/// grab polling, and ingest handoff. The processor is idempotent and keeps
	/// the durable request row as its source of truth.
	BookRequestAutomation {
		request_id: String,
	},
	/// Polls one opaque gateway grab and advances it to ingest handoff.
	BookRequestPoll {
		grab_id: String,
	},
}

impl JobPayload for StumpJob {
	/// Returns the human-readable name of the job
	fn name(&self) -> &'static str {
		match self {
			StumpJob::LibraryScan { .. } => "library_scan",
			StumpJob::SeriesScan { .. } => "series_scan",
			StumpJob::ThumbnailGeneration { .. } => "thumbnail_generation",
			StumpJob::BookRequestPoll { .. } => "book_request_poll",
			StumpJob::PlaceholderGeneration { .. } => "placeholder_generation",
			StumpJob::MetadataFetch { .. } => "metadata_fetch",
			StumpJob::AnalyzeMedia { .. } => "analyze_media",
			StumpJob::BookRequestAutomation { .. } => "book_request_automation",
			#[cfg(feature = "providers")]
			StumpJob::ProviderGc => "provider_gc",
			#[cfg(feature = "providers")]
			StumpJob::ProviderSourceHealth => "provider_source_health",
			StumpJob::NotificationDispatch { .. } => "notification_dispatch",
			StumpJob::AnnotationSync { .. } => "annotation_sync",
		}
	}

	/// Returns a description for the job
	fn description(&self) -> Option<String> {
		match self {
			StumpJob::LibraryScan { path, .. } => Some(path.clone()),
			StumpJob::SeriesScan { path, .. } => Some(path.clone()),
			StumpJob::ThumbnailGeneration { params, .. } => {
				Some(format!("Thumbnail generation: {:?}", params))
			},
			StumpJob::PlaceholderGeneration { .. } => Some(
				"Generate placeholder thumbnail metadata for media, series, or libraries"
					.to_string(),
			),
			StumpJob::MetadataFetch { params } => {
				Some(format!("Metadata fetch: {:?}", params.scope))
			},
			StumpJob::AnalyzeMedia { config } => {
				Some(format!("Analyze media: {:?}", config.scope))
			},
			StumpJob::BookRequestAutomation { request_id } => {
				Some(format!("Automate book request {request_id}"))
			},
			StumpJob::BookRequestPoll { grab_id } => {
				Some(format!("Poll book request grab {grab_id}"))
			},
			#[cfg(feature = "providers")]
			StumpJob::ProviderGc => Some("Reclaim stale materialised provider series".to_string()),
			#[cfg(feature = "providers")]
			StumpJob::ProviderSourceHealth => Some("Probe remote provider sources".to_string()),
			StumpJob::NotificationDispatch { deliveries } => {
				Some(format!("Deliver {} notification(s)", deliveries.len()))
			},
			StumpJob::AnnotationSync { user_id } => {
				Some(format!("Export annotations for user {user_id}"))
			},
		}
	}

	/// The Komga-style queue bucket reported in job queue status events
	fn kind(&self) -> &'static str {
		match self {
			StumpJob::LibraryScan { .. } | StumpJob::SeriesScan { .. } => "SCAN",
			StumpJob::AnalyzeMedia { .. } => "ANALYZE",
			StumpJob::BookRequestAutomation { .. } | StumpJob::BookRequestPoll { .. } => {
				"REQUESTS"
			},
			#[cfg(feature = "providers")]
			StumpJob::ProviderGc => "GC",
			#[cfg(feature = "providers")]
			StumpJob::ProviderSourceHealth => "MAINTENANCE",
			StumpJob::NotificationDispatch { .. } => "NOTIFY",
			StumpJob::AnnotationSync { .. } => "ANNOTATIONS",
			StumpJob::MetadataFetch { .. } => "METADATA",
			StumpJob::ThumbnailGeneration { .. }
			| StumpJob::PlaceholderGeneration { .. } => "THUMBNAIL",
		}
	}
}

impl StumpJob {
	pub fn library_scan(id: String, path: String, options: Option<ScanOptions>) -> Self {
		StumpJob::LibraryScan { id, path, options }
	}

	pub fn series_scan(id: String, path: String, options: Option<ScanOptions>) -> Self {
		StumpJob::SeriesScan { id, path, options }
	}

	pub fn thumbnail_generation(
		options: ImageProcessorOptions,
		params: ThumbnailGenerationJobParams,
	) -> Self {
		StumpJob::ThumbnailGeneration { options, params }
	}

	pub fn placeholder_generation(config: PlaceholderGenerationJobConfig) -> Self {
		StumpJob::PlaceholderGeneration { config }
	}

	pub fn metadata_fetch(params: MetadataFetchJobParams) -> Self {
		StumpJob::MetadataFetch { params }
	}

	pub fn analyze_media(config: AnalysisJobConfig) -> Self {
		StumpJob::AnalyzeMedia { config }
	}
	pub fn book_request_automation(request_id: String) -> Self {
		StumpJob::BookRequestAutomation { request_id }
	}
	pub fn book_request_poll(grab_id: String) -> Self {
		StumpJob::BookRequestPoll { grab_id }
	}
}
