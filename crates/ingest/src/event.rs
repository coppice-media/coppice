//! Analysis outcomes the host routes onwards (notification channels, client
//! event stream).
//!
//! The pipeline emits its own vocabulary rather than the host's event enum:
//! that enum is a GraphQL union and a serialized client contract, and pulling
//! it in here would point the dependency back at `stump_core`. `stump_core`
//! maps every variant onto a `CoreEvent` one-to-one in
//! `core/src/ingest_host.rs`.

/// Something an analysis run decided that a human may need to know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestEvent {
	/// An analysis job failed; the job row carries the same message.
	AnalysisJobFailed {
		analysis_job_id: String,
		error: String,
	},
	/// A quality report contains at least one failed check.
	QualityFailed {
		library_id: String,
		/// Set for staged items; media-rework reports carry `media_id`.
		drop_item_id: Option<String>,
		media_id: Option<String>,
		created_by: Option<String>,
		score: u8,
		failed_checks: Vec<String>,
	},
	/// Provider identify/lookup finished and candidates were stored.
	ProviderMatchDone {
		library_id: String,
		drop_item_id: String,
		created_by: Option<String>,
		candidate_count: usize,
	},
	/// A staged item finished analysis and is waiting for a decision.
	AwaitingReview {
		library_id: String,
		drop_item_id: String,
		source_filename: String,
		created_by: Option<String>,
	},
}

/// Where [`IngestEvent`]s go. Absent in tests, which assert on rows instead.
pub trait IngestEventSink: Send + Sync + 'static {
	fn emit(&self, event: IngestEvent);
}
