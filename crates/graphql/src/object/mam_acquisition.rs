use async_graphql::{SimpleObject, ID};
use chrono::{DateTime, FixedOffset};

#[derive(Clone, SimpleObject)]
pub struct ReleaseCandidate {
	pub torrent_id: i32,
	pub title: String,
	pub authors: Vec<String>,
	pub narrators: Vec<String>,
	pub series: Vec<String>,
	pub kind: String,
	pub category_name: Option<String>,
	pub language_code: Option<String>,
	pub file_type: Option<String>,
	pub size: Option<String>,
	pub num_files: Option<i32>,
	pub added: Option<String>,
	pub seeders: Option<i32>,
	pub leechers: Option<i32>,
	pub times_completed: Option<i32>,
	pub freeleech: bool,
	pub vip: bool,
	pub snatched: bool,
	pub isbn: Option<String>,
	pub score: f64,
	pub match_reasons: Vec<String>,
}

impl From<stump_core::mam_acquisition::ReleaseCandidate> for ReleaseCandidate {
	fn from(candidate: stump_core::mam_acquisition::ReleaseCandidate) -> Self {
		Self {
			torrent_id: candidate.torrent_id,
			title: candidate.title,
			authors: candidate.authors,
			narrators: candidate.narrators,
			series: candidate.series,
			kind: candidate.kind,
			category_name: candidate.category_name,
			language_code: candidate.language_code,
			file_type: candidate.file_type,
			size: candidate.size,
			num_files: candidate.num_files,
			added: candidate.added,
			seeders: candidate.seeders,
			leechers: candidate.leechers,
			times_completed: candidate.times_completed,
			freeleech: candidate.freeleech,
			vip: candidate.vip,
			snatched: candidate.snatched,
			isbn: candidate.isbn,
			score: candidate.score,
			match_reasons: candidate.match_reasons,
		}
	}
}

#[derive(Clone, SimpleObject)]
pub struct ReleaseSearch {
	pub candidates: Vec<ReleaseCandidate>,
	pub found: Option<i32>,
	pub probe: bool,
	pub searched_at: DateTime<FixedOffset>,
}

impl From<stump_core::mam_acquisition::ReleaseSearch> for ReleaseSearch {
	fn from(search: stump_core::mam_acquisition::ReleaseSearch) -> Self {
		Self {
			candidates: search.candidates.into_iter().map(Into::into).collect(),
			found: search.found,
			probe: search.probe,
			searched_at: search.searched_at,
		}
	}
}

#[derive(Clone, SimpleObject)]
pub struct AcquisitionStatus {
	pub configured: bool,
	pub reachable: bool,
	pub ready: bool,
	pub mode: Option<String>,
	pub message: Option<String>,
}

impl From<stump_core::mam_acquisition::AcquisitionStatus> for AcquisitionStatus {
	fn from(status: stump_core::mam_acquisition::AcquisitionStatus) -> Self {
		Self {
			configured: status.configured,
			reachable: status.reachable,
			ready: status.ready,
			mode: status.mode,
			message: status.message,
		}
	}
}

#[derive(Clone, SimpleObject)]
pub struct AcquisitionGrab {
	pub id: ID,
	pub request_id: ID,
	pub torrent_id: i32,
	pub title: String,
	pub phase: String,
	pub progress: f32,
	pub error: Option<String>,
	pub ingest_item_id: Option<ID>,
	pub created_at: DateTime<FixedOffset>,
	pub updated_at: DateTime<FixedOffset>,
}

impl From<stump_core::mam_acquisition::AcquisitionGrab> for AcquisitionGrab {
	fn from(grab: stump_core::mam_acquisition::AcquisitionGrab) -> Self {
		Self {
			id: ID::from(grab.id),
			request_id: ID::from(grab.request_id),
			torrent_id: grab.torrent_id,
			title: grab.title,
			phase: grab.phase,
			progress: grab.progress,
			error: grab.error,
			ingest_item_id: grab.ingest_item_id.map(ID::from),
			created_at: grab.created_at,
			updated_at: grab.updated_at,
		}
	}
}
