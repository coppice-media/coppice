//! Probe tests against real containers.
//!
//! Every fixture under `integration-tests/data/audio` is a genuine, tiny
//! (< 7 KB) file muxed by ffmpeg — one per chapter mechanism plus three folder
//! shapes. Synthetic byte blobs would prove nothing here: the point of the
//! probe is that it reads what real muxers write, including the two MP4
//! chapter mechanisms that no single library covers.
//!
//! All fixtures encode 6 s of silence split into three 2 s chapters titled
//! `Opening`/`Middle`/`Closing`, so the assertions can be exact.

use std::path::PathBuf;

use super::*;
use crate::error::FileError;

fn fixture(name: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("integration-tests/data/audio")
		.join(name)
}

fn titles(probed: &ProbedAudio) -> Vec<Option<&str>> {
	probed
		.chapters
		.iter()
		.map(|chapter| chapter.title.as_deref())
		.collect()
}

fn windows(probed: &ProbedAudio) -> Vec<(i64, Option<i64>)> {
	probed
		.chapters
		.iter()
		.map(|chapter| (chapter.start_ms, chapter.end_ms))
		.collect()
}

fn names(probed: &ProbedAudio) -> Vec<String> {
	probed
		.tracks
		.iter()
		.map(|track| {
			track
				.path
				.file_name()
				.unwrap()
				.to_string_lossy()
				.to_string()
		})
		.collect()
}

/// The four chapter mechanisms are four different formats in four different
/// containers, and every one of them must land on the same three marks. The
/// provenance must name the actual mechanism: a client that hides synthesized
/// chapters can only do that if `chapter_source` survives the probe.
#[test]
fn audio_chapter_source_names_the_container_mechanism() {
	let cases = [
		("chapters-chpl.m4b", ChapterSource::Mp4Chpl),
		("chapters-track.m4b", ChapterSource::Mp4ChapterTrack),
		("chapters-id3.mp3", ChapterSource::Id3Chap),
		("chapters-vorbis.opus", ChapterSource::VorbisComment),
	];

	for (name, expected) in cases {
		let probed = probe_file(&fixture(name)).expect(name);
		assert_eq!(probed.chapter_source, expected, "{name}");
		assert_eq!(
			titles(&probed),
			[Some("Opening"), Some("Middle"), Some("Closing")],
			"{name}"
		);
		assert_eq!(
			probed
				.chapters
				.iter()
				.map(|c| c.start_ms)
				.collect::<Vec<_>>(),
			[0, 2_000, 4_000],
			"{name}"
		);
	}
}

/// A book with no marks reports `None`, not an empty publisher chapter list
/// and not a synthesized one: a single-file book has no per-file fallback.
#[test]
fn audio_file_without_chapters_reports_no_source() {
	let probed = probe_file(&fixture("plain.m4b")).unwrap();

	assert_eq!(probed.chapter_source, ChapterSource::None);
	assert!(probed.chapters.is_empty());
	assert_eq!(probed.tracks.len(), 1, "a single container is one track");
}

/// Duration, codec, sample rate and channel count are what every client shows
/// and what a reading position is expressed against, so they come from the
/// container rather than from the extension.
#[test]
fn audio_probe_reads_duration_and_codec_facts() {
	let m4b = probe_file(&fixture("plain.m4b")).unwrap();
	assert_eq!(m4b.duration_ms, 6_000);
	assert_eq!(m4b.codec, "aac");
	assert_eq!(m4b.sample_rate, Some(22_050));
	assert_eq!(m4b.channels, Some(1));

	let mp3 = probe_file(&fixture("chapters-id3.mp3")).unwrap();
	assert_eq!(mp3.duration_ms, 6_000);
	assert_eq!(mp3.codec, "mp3");

	let opus = probe_file(&fixture("chapters-vorbis.opus")).unwrap();
	// Opus is 20 ms-framed, so 6 s of audio rounds up to the frame boundary.
	assert_eq!(opus.duration_ms, 6_006);
	assert_eq!(opus.codec, "opus");
	assert_eq!(opus.sample_rate, Some(48_000));

	for probed in [&m4b, &mp3, &opus] {
		let bitrate = probed.bitrate.expect("a probed duration yields a bitrate");
		assert!(bitrate > 0, "bitrate must be positive, got {bitrate}");
	}
}

/// An embedded cover is the audiobook's thumbnail source, and its content
/// type has to be right or the thumbnail pipeline cannot decode it.
#[test]
fn audio_probe_extracts_the_embedded_cover() {
	let probed = probe_file(&fixture("plain.m4b")).unwrap();

	let (content_type, bytes) = probed.cover.expect("plain.m4b carries a covr atom");
	assert_eq!(content_type, ContentType::PNG);
	assert!(content_type.is_image());
	assert!(!bytes.is_empty());
	assert_eq!(
		&bytes[1..4],
		b"PNG",
		"the bytes are the image, not a wrapper"
	);
}

/// Tags name the book in every client and feed the compatibility profiles.
#[test]
fn audio_probe_reads_publication_tags() {
	let probed = probe_file(&fixture("chapters-chpl.m4b")).unwrap();

	assert_eq!(probed.title.as_deref(), Some("Silent Test Book"));
	assert_eq!(probed.author.as_deref(), Some("Test Narrator"));
}

/// A folder of parts is ONE publication: the durations sum, the offsets are a
/// running total, and the file list becomes the chapter list — recorded as
/// `PerTrack` so it is never mistaken for a publisher's marks.
#[test]
fn audio_folder_book_synthesizes_one_chapter_per_file() {
	let probed = probe_folder(&fixture("folder-filename")).unwrap();

	assert_eq!(probed.tracks.len(), 3);
	assert_eq!(
		probed.duration_ms,
		probed.tracks.iter().map(|t| t.duration_ms).sum::<i64>()
	);
	assert_eq!(probed.chapter_source, ChapterSource::PerTrack);
	assert_eq!(
		titles(&probed),
		[
			Some("01 - Part 1"),
			Some("02 - Part 2"),
			Some("03 - Part 3")
		]
	);

	// The synthesized marks tile the publication with no gap and no overlap.
	let windows = windows(&probed);
	assert_eq!(windows[0].0, 0);
	for pair in windows.windows(2) {
		assert_eq!(pair[0].1, Some(pair[1].0));
	}
	assert_eq!(windows.last().unwrap().1, Some(probed.duration_ms));

	// Every track carries what a range request needs without a stat.
	for track in &probed.tracks {
		assert!(track.byte_size > 0);
		assert_eq!(track.mime, "audio/mpeg");
	}
}

/// A publisher who numbered the files meant that order, even when the
/// filenames sort the other way. `alpha`/`mike`/`zulu` sort alphabetically in
/// exactly the reverse of their `TRCK` tags, so a wrong order is unmissable.
#[test]
fn audio_folder_book_orders_by_track_number_over_filename() {
	let probed = probe_folder(&fixture("folder-tracknum")).unwrap();

	assert_eq!(names(&probed), ["zulu.mp3", "mike.mp3", "alpha.mp3"]);
	assert_eq!(
		probed
			.tracks
			.iter()
			.map(|t| t.track_number)
			.collect::<Vec<_>>(),
		[Some(1), Some(2), Some(3)]
	);
	// The synthesized chapter titles follow the tags, not the filenames.
	assert_eq!(
		titles(&probed),
		[Some("Opening"), Some("Middle"), Some("Closing")]
	);
	// Offsets follow the chosen order, not the directory order.
	assert_eq!(
		windows(&probed),
		[(0, Some(2_000)), (2_000, Some(4_000)), (4_000, Some(6_000))]
	);
}

/// A folder whose parts disagree on codec is still one publication; the
/// per-track mime stays exact so each file is served correctly, and the
/// publication codec says `mixed` rather than lying with the first file's.
#[test]
fn audio_folder_book_with_mixed_codecs_keeps_per_track_mime() {
	let probed = probe_folder(&fixture("folder-mixed")).unwrap();

	assert_eq!(probed.codec, "mixed");
	assert_eq!(names(&probed), ["01.mp3", "02.opus"]);
	assert_eq!(
		probed
			.tracks
			.iter()
			.map(|t| t.mime.as_str())
			.collect::<Vec<_>>(),
		["audio/mpeg", "audio/opus"]
	);
	assert_eq!(probed.tracks[0].codec, "mp3");
	assert_eq!(probed.tracks[1].codec, "opus");
	assert_eq!(probed.duration_ms, 4_006);
}

/// `probe` is what the scanner calls; it must not need to know the shape.
#[test]
fn audio_probe_dispatches_on_file_or_folder() {
	let file = probe(&fixture("chapters-chpl.m4b")).unwrap();
	assert_eq!(file.tracks.len(), 1);

	let folder = probe(&fixture("folder-filename")).unwrap();
	assert_eq!(folder.tracks.len(), 3);
}

/// "Resume at chapter" is a comparison against start marks; an offset before
/// the first mark is in no chapter, and the last mark runs to the end.
#[test]
fn audio_chapter_at_resolves_a_publication_offset() {
	let probed = probe_file(&fixture("chapters-chpl.m4b")).unwrap();

	assert_eq!(
		probed.chapter_at(0).unwrap().title.as_deref(),
		Some("Opening")
	);
	assert_eq!(
		probed.chapter_at(1_999).unwrap().title.as_deref(),
		Some("Opening")
	);
	assert_eq!(
		probed.chapter_at(2_000).unwrap().title.as_deref(),
		Some("Middle")
	);
	assert_eq!(
		probed.chapter_at(5_999).unwrap().title.as_deref(),
		Some("Closing")
	);

	let plain = probe_file(&fixture("plain.m4b")).unwrap();
	assert!(plain.chapter_at(1_000).is_none(), "no marks, no chapter");
}

/// The write path assigns `index` and `start_offset_ms` from the probe order,
/// so the running sum can never disagree with the durations.
#[test]
fn audio_facts_conversion_assigns_contiguous_offsets() {
	use models::services::audio::AudioFacts;

	let probed = probe_folder(&fixture("folder-tracknum")).unwrap();
	let facts = AudioFacts::from(&probed);

	assert_eq!(facts.duration_ms, 6_000);
	assert_eq!(facts.chapter_source, ChapterSource::PerTrack);
	assert_eq!(facts.tracks.len(), 3);
	assert_eq!(facts.chapters.len(), 3);
	// `TrackFacts` carries no offset: it is derived on write, from this order.
	assert_eq!(
		facts
			.tracks
			.iter()
			.map(|t| t.duration_ms)
			.collect::<Vec<_>>(),
		[2_000, 2_000, 2_000]
	);
	assert!(facts
		.tracks
		.iter()
		.all(|track| track.path.ends_with(".mp3")));
}

/// A non-audio file is rejected by type, not by a failed demux: the scanner
/// asks the probe about files it has already classified, and a wrong answer
/// here would create an audiobook out of a comic.
#[test]
fn audio_probe_rejects_a_non_audio_file() {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("integration-tests/data/science_comics_001.cbz");

	let error = probe_file(&path).expect_err("a cbz is not an audiobook");
	assert!(
		matches!(error, FileError::UnsupportedFileType(_)),
		"expected UnsupportedFileType, got {error}"
	);
}
