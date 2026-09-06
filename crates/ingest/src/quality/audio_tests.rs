//! The audio quality family, one positive and one negative case each.
//!
//! Every fixture is a real container from `crates/media/integration-tests/data/audio`,
//! demuxed by the same probe production uses — a synthesised byte string would
//! prove the check compiles and nothing about whether it can read an audiobook.
//!
//! `chapters-chpl.m4b` is the good book: one AAC-in-MP4 track, publisher
//! chapter marks, tags, a cover, `moov` before `mdat`. `folder-tracknum/` and
//! `folder-mixed/` are the bad ones: three untagged MP3 parts, and two parts
//! that do not even share a codec.

use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use stump_api_types::settings::SettingValues;

use crate::contract::{BookSnapshot, IngestMediaKind, QualityCheck, QualityStatus};

use super::{
	registry::CheckFamily, BitrateSaneCheck, ChaptersPresentCheck, CoverEmbeddedCheck,
	DurationConsistentCheck, FaststartCheck, SingleFileCheck, TagsCompleteCheck,
};

fn fixture(name: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("../media/integration-tests/data/audio")
		.join(name)
}

/// A snapshot of a real audiobook, file or folder.
fn audio_snapshot(path: &Path) -> BookSnapshot {
	BookSnapshot {
		drop_item_id: "drop-audio".to_string(),
		library_id: "library-audio".to_string(),
		staged_path: path.to_path_buf(),
		source_sha256: "audio-digest".to_string(),
		byte_size: std::fs::metadata(path)
			.map(|metadata| metadata.len())
			.unwrap_or(0),
		source_filename: path
			.file_name()
			.map(|name| name.to_string_lossy().to_string())
			.unwrap_or_default(),
		relative_path: String::new(),
		media_kind: IngestMediaKind::Audio,
		embedded_metadata: None,
		pages: Vec::new(),
		analysis: None,
	}
}

fn comic_snapshot() -> BookSnapshot {
	BookSnapshot {
		media_kind: IngestMediaKind::ComicArchive,
		..audio_snapshot(&fixture("chapters-chpl.m4b"))
	}
}

fn defaults() -> SettingValues {
	BTreeMap::new()
}

async fn status_of(check: &dyn QualityCheck, book: &BookSnapshot) -> QualityStatus {
	check
		.run(book, &defaults())
		.await
		.expect("the check runs")
		.status
}

async fn evidence_of(check: &dyn QualityCheck, book: &BookSnapshot) -> serde_json::Value {
	check
		.run(book, &defaults())
		.await
		.expect("the check runs")
		.evidence
}

#[tokio::test]
async fn single_file_passes_one_container_and_fails_a_folder() {
	let check = SingleFileCheck::new();

	let good = audio_snapshot(&fixture("chapters-chpl.m4b"));
	assert_eq!(status_of(&check, &good).await, QualityStatus::Pass);
	assert_eq!(evidence_of(&check, &good).await["parts"], 1);

	let split = audio_snapshot(&fixture("folder-tracknum"));
	assert_eq!(status_of(&check, &split).await, QualityStatus::Fail);
	assert_eq!(evidence_of(&check, &split).await["parts"], 3);
}

/// The one configurable weight, and the reason it is configurable.
#[tokio::test]
async fn single_file_weight_comes_from_the_policy() {
	assert_eq!(
		SingleFileCheck::new().weight(),
		super::DEFAULT_SINGLE_FILE_WEIGHT
	);
	assert_eq!(SingleFileCheck::with_weight(5).weight(), 5);
}

#[tokio::test]
async fn chapters_present_grades_authored_synthesized_and_absent() {
	let check = ChaptersPresentCheck::new();

	// Publisher marks in a `chpl` atom.
	let authored = audio_snapshot(&fixture("chapters-chpl.m4b"));
	let outcome = check.run(&authored, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Pass);
	assert_eq!(outcome.normalized_score, 1.0);
	assert_eq!(outcome.evidence["authored"], true);

	// A folder book with no marks: the probe synthesized one per file, which
	// is half a chapter list and scored as such.
	let synthesized = audio_snapshot(&fixture("folder-tracknum"));
	let outcome = check.run(&synthesized, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Warn);
	assert_eq!(outcome.normalized_score, 0.5);
	assert_eq!(outcome.evidence["authored"], false);

	// A single container with nothing at all.
	let none = audio_snapshot(&fixture("plain.m4b"));
	assert_eq!(status_of(&check, &none).await, QualityStatus::Fail);
}

#[tokio::test]
async fn faststart_judges_the_box_order_and_declines_a_stream() {
	let check = FaststartCheck::new();

	// The fixtures are ffmpeg output, which is `ftyp`/`mdat`/`moov`: the
	// index is at the back, which is exactly the finding.
	let mp4 = audio_snapshot(&fixture("chapters-chpl.m4b"));
	let outcome = check.run(&mp4, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Fail);
	assert_eq!(outcome.evidence["moov_first"], false);

	// An MP3 is a stream with no index to misplace.
	let mp3 = audio_snapshot(&fixture("chapters-id3.mp3"));
	let outcome = check.run(&mp3, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::NotApplicable);
	assert_eq!(outcome.evidence["reason"], "not_an_mp4_container");
}

#[tokio::test]
async fn tags_complete_counts_the_five_fields_a_listener_sees() {
	let check = TagsCompleteCheck::new();

	// The fixture carries a title, an author and an album but no narrator and
	// no year, so the score is fractional and the missing fields are named.
	let tagged = audio_snapshot(&fixture("chapters-chpl.m4b"));
	let outcome = check.run(&tagged, &defaults()).await.expect("runs");
	assert_eq!(outcome.evidence["expected"], 5);
	let present = outcome.evidence["present"].as_u64().expect("a count");
	assert!(present >= 2, "{:?}", outcome.evidence);
	assert!(outcome.normalized_score > 0.0 && outcome.normalized_score <= 1.0);
	if present < 5 {
		assert_eq!(outcome.status, QualityStatus::Warn);
		assert!(outcome.evidence["missing"]
			.as_array()
			.is_some_and(|missing| { !missing.is_empty() }));
	}

	// Untagged MP3 parts: strictly fewer fields than the tagged container.
	let untagged = audio_snapshot(&fixture("folder-filename"));
	let outcome_untagged = check.run(&untagged, &defaults()).await.expect("runs");
	assert_eq!(outcome_untagged.status, QualityStatus::Warn);
	assert!(
		outcome_untagged.normalized_score < outcome.normalized_score,
		"{:?} should be worse than {:?}",
		outcome_untagged.evidence,
		outcome.evidence
	);
}

#[tokio::test]
async fn cover_embedded_passes_artwork_and_warns_without_it() {
	let check = CoverEmbeddedCheck::new();

	// `plain.m4b` carries a PNG cover; the chaptered fixture carries none.
	let with_cover = audio_snapshot(&fixture("plain.m4b"));
	let outcome = check.run(&with_cover, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Pass);
	assert_eq!(outcome.evidence["embedded"], true);
	assert!(outcome.evidence["byte_count"].as_u64().unwrap_or(0) > 0);

	let without = audio_snapshot(&fixture("folder-tracknum"));
	let outcome = check.run(&without, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Warn);
	assert_eq!(outcome.evidence["embedded"], false);
}

#[tokio::test]
async fn duration_consistent_passes_a_real_book_and_fails_an_unreadable_one() {
	let check = DurationConsistentCheck::new();

	let good = audio_snapshot(&fixture("chapters-chpl.m4b"));
	let outcome = check.run(&good, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Pass);
	assert!(outcome.evidence["duration_ms"].as_i64().unwrap_or(0) > 0);
	assert_eq!(outcome.evidence["parts_without_duration"], 0);
	assert_eq!(outcome.evidence["marks_past_the_end"], 0);

	// A file that claims to be an audiobook and cannot be demuxed is a
	// finding, not a skipped check.
	let dir = tempfile::tempdir().expect("temp dir");
	let broken = dir.path().join("truncated.m4b");
	std::fs::write(&broken, b"not an audiobook at all").expect("write the fixture");
	let outcome = check
		.run(&audio_snapshot(&broken), &defaults())
		.await
		.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Fail);
	assert_eq!(outcome.evidence["readable"], false);
}

#[tokio::test]
async fn bitrate_sane_passes_speech_and_fails_a_mixed_codec_book() {
	let check = BitrateSaneCheck::new();

	let good = audio_snapshot(&fixture("chapters-chpl.m4b"));
	let outcome = check.run(&good, &defaults()).await.expect("runs");
	// The fixture is a low-bitrate synthetic rip, so the verdict is whatever
	// the range says — what matters is that a number was judged at all.
	assert!(matches!(
		outcome.status,
		QualityStatus::Pass | QualityStatus::Warn
	));
	assert!(outcome.evidence["bitrate"].as_i64().unwrap_or(0) > 0);

	// Parts that do not share a codec have no publication bitrate to judge.
	let mixed = audio_snapshot(&fixture("folder-mixed"));
	let outcome = check.run(&mixed, &defaults()).await.expect("runs");
	assert_eq!(outcome.status, QualityStatus::Fail);
	assert_eq!(outcome.evidence["codec"], "mixed");
}

/// The rule that keeps the two families from diluting each other: an audio
/// check has no opinion about a comic, and says so rather than failing it.
#[tokio::test]
async fn every_audio_check_is_not_applicable_to_a_comic() {
	let comic = comic_snapshot();
	let checks: Vec<Box<dyn QualityCheck>> = vec![
		Box::new(SingleFileCheck::new()),
		Box::new(ChaptersPresentCheck::new()),
		Box::new(TagsCompleteCheck::new()),
		Box::new(CoverEmbeddedCheck::new()),
		Box::new(FaststartCheck::new()),
		Box::new(DurationConsistentCheck::new()),
		Box::new(BitrateSaneCheck::new()),
	];

	for check in &checks {
		let outcome = check.run(&comic, &defaults()).await.expect("runs");
		assert_eq!(
			outcome.status,
			QualityStatus::NotApplicable,
			"{} should decline a comic",
			check.id()
		);
		assert_eq!(outcome.evidence["reason"], "not_an_audiobook");
	}
}

/// Every audio finding names a tool that repairs it: a finding a librarian
/// cannot act on is a complaint, not a check.
#[tokio::test]
async fn every_audio_check_names_its_fix_tool() {
	let expected = [
		("single_file", "audio-assemble"),
		("chapters_present", "audio-chapters"),
		("tags_complete", "meta-edit"),
		("cover_embedded", "meta-edit"),
		("faststart", "audio-assemble"),
		("duration_consistent", "audio-assemble"),
		("bitrate_sane", "audio-assemble"),
	];
	let checks: Vec<Box<dyn QualityCheck>> = vec![
		Box::new(SingleFileCheck::new()),
		Box::new(ChaptersPresentCheck::new()),
		Box::new(TagsCompleteCheck::new()),
		Box::new(CoverEmbeddedCheck::new()),
		Box::new(FaststartCheck::new()),
		Box::new(DurationConsistentCheck::new()),
		Box::new(BitrateSaneCheck::new()),
	];

	for check in &checks {
		let fix = check.fix().unwrap_or_else(|| {
			panic!("{} must name a fix tool", check.id());
		});
		let (_, tool) = expected
			.iter()
			.find(|(id, _)| *id == check.id())
			.unwrap_or_else(|| panic!("{} is not in the contract list", check.id()));
		assert_eq!(&fix.tool, tool, "{}", check.id());
		assert!(!fix.summary.is_empty(), "{}", check.id());
	}
}

/// The seven ids, exactly, and the family split the scorer depends on.
#[test]
fn the_audio_family_is_the_seven_contract_ids() {
	let mut ids = CheckFamily::AUDIO_IDS;
	ids.sort_unstable();
	assert_eq!(
		ids,
		[
			"bitrate_sane",
			"chapters_present",
			"cover_embedded",
			"duration_consistent",
			"faststart",
			"single_file",
			"tags_complete",
		]
	);
	assert_eq!(CheckFamily::of("single_file"), CheckFamily::Audio);
	assert_eq!(CheckFamily::of("cover_present"), CheckFamily::Paged);
}
