//! An audiobook has to be acquirable from both feeds.
//!
//! The decisions under test live in `stump_core::opds` (this crate owns routes
//! and the backend trait, not the wire types), but they are only reachable
//! from a test here: `stump_opds` is the crate that depends on both the feed
//! models and the entities they read.

use models::{
	domain::audio::AudioChapterSource,
	entity::{media, media_audio, media_audio_chapter, media_audio_track},
	services::audio::AudioBook,
	shared::enums::FileStatus,
};
use serde_json::Value;
use stump_core::opds::{
	v1_2::link::OpdsLinkType,
	v2_0::{
		audio,
		entity::{OPDSPublicationEntity, OPDSSeries},
		link::{OPDSLink, OPDSLinkFinalizer},
		metadata::OPDSWebPubMetadata,
		publication::OPDSPublication,
	},
};

const SERVICE_URL: &str = "https://stump.example";
const ACQUISITION: &str = "http://opds-spec.org/acquisition";

fn finalizer() -> OPDSLinkFinalizer {
	OPDSLinkFinalizer::new(String::from(SERVICE_URL))
}

/// A book row shaped like the audiobook fixtures: a folder book carries its
/// dominant track's extension, exactly as a single-container one does.
fn publication_entity(extension: &str) -> OPDSPublicationEntity {
	OPDSPublicationEntity {
		media: media::Model {
			id: String::from("book-1"),
			name: String::from("Silent Test Book"),
			size: 7168,
			extension: String::from(extension),
			pages: -1,
			updated_at: None,
			created_at: chrono::Utc::now().into(),
			modified_at: None,
			hash: None,
			koreader_hash: None,
			path: String::from("/library/silent-test-book"),
			status: FileStatus::Ready,
			thumbnail_meta: None,
			thumbnail_path: None,
			series_id: Some(String::from("series-1")),
			deleted_at: None,
			source_provider: None,
			remote_id: None,
			remote_chapter_id: None,
		},
		metadata: None,
		series: OPDSSeries {
			id: String::from("series-1"),
			name: String::from("Test Series"),
			metadata: None,
		},
		reading_session: None,
	}
}

fn track(index: i32, mime: &str, byte_size: i64) -> media_audio_track::Model {
	media_audio_track::Model {
		id: format!("track-{index}"),
		media_id: String::from("book-1"),
		index,
		path: format!("/library/silent-test-book/{index}"),
		duration_ms: 2000,
		start_offset_ms: i64::from(index) * 2000,
		byte_size,
		mime: String::from(mime),
	}
}

fn chapter(index: i32, title: &str, start_ms: i64) -> media_audio_chapter::Model {
	media_audio_chapter::Model {
		id: format!("chapter-{index}"),
		media_id: String::from("book-1"),
		index,
		title: Some(String::from(title)),
		start_ms,
		end_ms: None,
	}
}

/// Three files of 2000 ms with chapter marks that deliberately fall inside a
/// track rather than on its boundary, which is what makes the `#t=` offsets
/// worth asserting.
fn folder_audiobook() -> AudioBook {
	AudioBook {
		audio: media_audio::Model {
			media_id: String::from("book-1"),
			duration_ms: 6000,
			codec: String::from("mixed"),
			sample_rate: Some(44100),
			channels: Some(2),
			bitrate: Some(64000),
			chapter_source: AudioChapterSource::Id3Chap,
		},
		tracks: vec![
			track(0, "audio/mpeg", 1024),
			track(1, "audio/mpeg", 2048),
			track(2, "audio/opus", 4096),
		],
		chapters: vec![
			chapter(0, "Opening", 0),
			chapter(1, "Middle", 3000),
			chapter(2, "Closing", 4500),
		],
	}
}

/// One `.m4b` holding the whole publication.
fn single_container_audiobook() -> AudioBook {
	AudioBook {
		audio: media_audio::Model {
			media_id: String::from("book-1"),
			duration_ms: 6000,
			codec: String::from("aac"),
			sample_rate: Some(22050),
			channels: Some(1),
			bitrate: Some(32000),
			chapter_source: AudioChapterSource::Mp4Chpl,
		},
		tracks: vec![media_audio_track::Model {
			duration_ms: 6000,
			..track(0, "audio/mp4", 7168)
		}],
		chapters: vec![],
	}
}

fn links_to_json(links: &[OPDSLink]) -> Vec<Value> {
	serde_json::to_value(links)
		.expect("links must serialize")
		.as_array()
		.expect("links serialize as an array")
		.clone()
}

fn acquisitions(links: &[OPDSLink]) -> Vec<Value> {
	links_to_json(links)
		.into_iter()
		.filter(|link| link["rel"] == ACQUISITION)
		.collect()
}

fn strings(links: &[Value], key: &str) -> Vec<String> {
	links
		.iter()
		.map(|link| {
			link[key]
				.as_str()
				.unwrap_or_else(|| panic!("link has no {key}: {link}"))
				.to_string()
		})
		.collect()
}

/// An audiobook's OPDS 1.2 acquisition link advertises the audio MIME. Every
/// audio extension used to be unknown to the feed, so the entry builder fell
/// back to `application/zip` and a reader had no way to tell it was audio.
#[test]
fn v1_acquisition_type_is_the_audio_mime() {
	for (extension, mime) in [
		("m4b", "audio/mp4"),
		("m4a", "audio/mp4"),
		("mp3", "audio/mpeg"),
		("opus", "audio/opus"),
		("ogg", "audio/ogg"),
		("flac", "audio/flac"),
	] {
		let link_type = OpdsLinkType::from_extension(extension)
			.unwrap_or_else(|| panic!("{extension} has no OPDS 1.2 link type"));

		assert_eq!(
			link_type.mime().as_ref(),
			mime,
			"wrong MIME for .{extension}"
		);
	}

	// A book that is not audio keeps the type it always advertised.
	let epub = OpdsLinkType::from_extension("epub").expect("epub is a known type");
	assert_eq!(epub.mime().as_ref(), "application/epub+zip");
}

/// A multi-file audiobook is acquired one track at a time, in `index` order,
/// each entry carrying that track's own MIME and byte size. The single
/// `/file` acquisition it replaces points at a directory for such a book, so
/// it must not survive alongside them.
#[test]
fn v2_multi_file_audiobook_has_one_acquisition_per_track() {
	let links = OPDSPublication::links_for_book(
		&publication_entity("mp3"),
		Some(&folder_audiobook()),
		&finalizer(),
	)
	.expect("links must build");

	let acquisitions = acquisitions(&links);
	assert_eq!(acquisitions.len(), 3);

	assert_eq!(
		strings(&acquisitions, "href"),
		vec![
			format!("{SERVICE_URL}/api/v2/media/book-1/audio/track/0"),
			format!("{SERVICE_URL}/api/v2/media/book-1/audio/track/1"),
			format!("{SERVICE_URL}/api/v2/media/book-1/audio/track/2"),
		]
	);
	assert_eq!(
		strings(&acquisitions, "type"),
		vec!["audio/mpeg", "audio/mpeg", "audio/opus"]
	);
	assert_eq!(
		acquisitions
			.iter()
			.map(|link| link["properties"]["length"].as_i64())
			.collect::<Vec<_>>(),
		vec![Some(1024), Some(2048), Some(4096)]
	);
	assert_eq!(
		acquisitions
			.iter()
			.map(|link| link["duration"].as_f64())
			.collect::<Vec<_>>(),
		vec![Some(2.0), Some(2.0), Some(2.0)]
	);

	let feed = serde_json::to_string(&links).expect("links must serialize");
	assert!(
		!feed.contains("/opds/v2.0/books/book-1/file"),
		"the whole-book file acquisition cannot serve a folder audiobook: {feed}"
	);
}

/// A single-container audiobook keeps the one file acquisition every other
/// publication has, and advertises the container's audio MIME rather than the
/// generic type an unknown extension used to produce.
#[test]
fn v2_single_container_audiobook_keeps_one_file_acquisition() {
	let links = OPDSPublication::links_for_book(
		&publication_entity("m4b"),
		Some(&single_container_audiobook()),
		&finalizer(),
	)
	.expect("links must build");

	let acquisitions = acquisitions(&links);
	assert_eq!(acquisitions.len(), 1);
	assert_eq!(
		strings(&acquisitions, "href"),
		vec![format!("{SERVICE_URL}/opds/v2.0/books/book-1/file")]
	);
	assert_eq!(strings(&acquisitions, "type"), vec!["audio/mp4"]);
}

/// A book that is not an audiobook is untouched: one file acquisition, typed
/// from its extension.
#[test]
fn v2_non_audio_book_keeps_its_file_acquisition() {
	let links =
		OPDSPublication::links_for_book(&publication_entity("epub"), None, &finalizer())
			.expect("links must build");

	let acquisitions = acquisitions(&links);
	assert_eq!(acquisitions.len(), 1);
	assert_eq!(
		strings(&acquisitions, "href"),
		vec![format!("{SERVICE_URL}/opds/v2.0/books/book-1/file")]
	);
	assert_eq!(strings(&acquisitions, "type"), vec!["application/epub+zip"]);
}

/// Chapter marks become a `toc`, and each entry seeks: it names the track
/// holding the mark plus the offset inside that track as a media fragment in
/// seconds. The length of the collection is the chapter count a client shows.
#[test]
fn v2_chapters_become_a_seekable_toc() {
	let toc =
		audio::toc("book-1", &folder_audiobook(), &finalizer()).expect("toc must build");

	let entries = links_to_json(&toc);
	assert_eq!(entries.len(), 3);

	assert_eq!(
		strings(&entries, "title"),
		vec!["Opening", "Middle", "Closing"]
	);
	assert_eq!(
		strings(&entries, "href"),
		vec![
			"/api/v2/media/book-1/audio/track/0#t=0",
			"/api/v2/media/book-1/audio/track/1#t=1",
			"/api/v2/media/book-1/audio/track/2#t=0.5",
		]
	);
}

/// A publication with no chapter marks gets no `toc` entries rather than an
/// entry per track: `chapters` is empty for `chapter_source = none`.
#[test]
fn v2_chapterless_audiobook_has_no_toc() {
	let toc = audio::toc("book-1", &single_container_audiobook(), &finalizer())
		.expect("toc must build");

	assert!(toc.is_empty());
}

/// The publication duration reaches the feed in seconds, the unit the Readium
/// metadata schema states, from the milliseconds Stump stores.
#[test]
fn v2_duration_is_published_in_seconds() {
	let metadata = OPDSWebPubMetadata::default().with_duration_ms(6006);

	let json = serde_json::to_value(&metadata).expect("metadata must serialize");
	assert_eq!(json["duration"].as_f64(), Some(6.006));
}
