use std::{collections::BTreeMap, sync::Arc};

use sea_orm::DatabaseConnection;
use serde::Serialize;
use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	BookSnapshot, FixAction, QualityCheck, QualityCheckError, QualityReport,
	QUALITY_ALGORITHM_VERSION,
};

use super::{
	BitrateSaneCheck, ChaptersPresentCheck, CoverEmbeddedCheck, CoverNotPageTwoCheck,
	CoverPresentCheck, DrmProtectedCheck, DuplicateExistingCheck,
	DuplicatePagesAcrossBooksCheck, DurationConsistentCheck, EpubTocChaptersCheck,
	FaststartCheck, FilenameSeriesParseCheck, ImageDimensionsConsistentCheck,
	MissingChaptersInSeriesCheck, PageCountMatchesArchiveEntriesCheck, SingleFileCheck,
	TagsCompleteCheck, DEFAULT_SINGLE_FILE_WEIGHT,
};

/// Discovery information for one registered quality check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckDescriptor {
	pub id: String,
	pub name: String,
	pub version: String,
	pub weight: u16,
	pub algorithm_version: String,
	pub settings: Vec<SettingDefinition>,
	/// The tool that repairs a failing outcome, when one exists. Discovery,
	/// so a client can offer the fix beside the finding instead of leaving a
	/// librarian to work out which tool applies.
	pub fix: Option<FixAction>,
}

/// Ordered quality-check registry used by the staged analysis coordinator.
pub struct QualityRegistry {
	checks: Vec<Arc<dyn QualityCheck>>,
}

impl QualityRegistry {
	/// The built-in set, with the server-default weight for a split
	/// audiobook. A run that has a library in scope uses
	/// [`Self::with_single_file_weight`] instead.
	pub fn builtin(conn: Arc<DatabaseConnection>) -> Self {
		Self::with_single_file_weight(conn, DEFAULT_SINGLE_FILE_WEIGHT)
	}

	/// The built-in set with the library's own weight for a split audiobook,
	/// from `policy::AudioPolicy::single_file_weight`.
	///
	/// The weight has to reach [`QualityCheck::weight`], which the scorer
	/// calls with no library in scope, so it is bound when the registry is
	/// constructed rather than looked up per book. `AudioPolicy` is `Copy`, so
	/// building a registry per library costs nothing but the `Arc`s.
	pub fn with_single_file_weight(
		conn: Arc<DatabaseConnection>,
		single_file_weight: u16,
	) -> Self {
		Self {
			checks: vec![
				// Blocking gate first: an unreadable file makes every other
				// finding moot.
				Arc::new(DrmProtectedCheck::new()),
				Arc::new(CoverPresentCheck::new()),
				Arc::new(CoverNotPageTwoCheck::new()),
				Arc::new(PageCountMatchesArchiveEntriesCheck::new()),
				Arc::new(ImageDimensionsConsistentCheck::new()),
				Arc::new(EpubTocChaptersCheck::new()),
				Arc::new(DuplicateExistingCheck::new(conn.clone())),
				Arc::new(DuplicatePagesAcrossBooksCheck::new(conn.clone())),
				Arc::new(FilenameSeriesParseCheck::new()),
				Arc::new(MissingChaptersInSeriesCheck::new(conn)),
				// The audio family. Every one of these is NOT_APPLICABLE for
				// a comic and every comic check is NOT_APPLICABLE for a
				// recording, and `score_report` excludes NOT_APPLICABLE from
				// both sums — so the two families never dilute each other and
				// each sums to 100 on its own kind of book.
				Arc::new(SingleFileCheck::with_weight(single_file_weight)),
				Arc::new(ChaptersPresentCheck::new()),
				Arc::new(TagsCompleteCheck::new()),
				Arc::new(CoverEmbeddedCheck::new()),
				Arc::new(FaststartCheck::new()),
				Arc::new(DurationConsistentCheck::new()),
				Arc::new(BitrateSaneCheck::new()),
			],
		}
	}

	pub fn checks(&self) -> &[Arc<dyn QualityCheck>] {
		&self.checks
	}

	pub fn catalog(&self) -> Vec<CheckDescriptor> {
		self.checks
			.iter()
			.map(|check| CheckDescriptor {
				id: check.id().to_string(),
				name: check.name().to_string(),
				version: check.version().to_string(),
				weight: check.weight(),
				algorithm_version: QUALITY_ALGORITHM_VERSION.to_string(),
				settings: check.settings().to_vec(),
				fix: check.fix(),
			})
			.collect()
	}

	pub async fn run_all(
		&self,
		book: &BookSnapshot,
		settings: &BTreeMap<String, SettingValues>,
	) -> Result<QualityReport, QualityCheckError> {
		let mut outcomes = Vec::with_capacity(self.checks.len());
		let mut settings_snapshot = BTreeMap::new();
		for check in &self.checks {
			let mut effective = check
				.settings()
				.iter()
				.map(|definition| {
					(definition.key.to_string(), definition.default.clone())
				})
				.collect::<SettingValues>();
			if let Some(configured) = settings.get(check.id()) {
				effective.extend(configured.clone());
			}
			settings_snapshot.insert(check.id().to_string(), effective.clone());
			let outcome = check.run(book, &effective).await?;
			outcomes.push((outcome, check.weight()));
		}
		let (score, checks) = crate::contract::score_report(&outcomes);
		Ok(QualityReport {
			source_sha256: book.source_sha256.clone(),
			algorithm_version: QUALITY_ALGORITHM_VERSION.to_string(),
			score,
			checks,
			settings_snapshot,
		})
	}

	/// Summed weight of the checks that apply to one kind of book.
	///
	/// There is no single total any more: the comic family and the audio
	/// family each sum to 100 and never both apply to one book, so a bare sum
	/// over the whole registry would be 200 and would describe nothing.
	pub fn family_weight(&self, family: CheckFamily) -> u16 {
		self.checks
			.iter()
			.filter(|check| CheckFamily::of(check.id()) == family)
			.map(|check| check.weight())
			.sum()
	}
}

/// Which kind of book a check has an opinion about.
///
/// Membership is by id rather than by asking a check, because a check answers
/// about one snapshot and this is a property of the check itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckFamily {
	/// Paged and reflowable books: comics, PDFs, EPUBs.
	Paged,
	/// Recordings.
	Audio,
}

impl CheckFamily {
	/// The ids of the audio family, alphabetical, as the contract names them.
	pub const AUDIO_IDS: [&'static str; 7] = [
		"bitrate_sane",
		"chapters_present",
		"cover_embedded",
		"duration_consistent",
		"faststart",
		"single_file",
		"tags_complete",
	];

	#[must_use]
	pub fn of(check_id: &str) -> Self {
		if Self::AUDIO_IDS.contains(&check_id) {
			Self::Audio
		} else {
			Self::Paged
		}
	}
}
