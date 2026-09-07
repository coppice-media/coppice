//! What an archive drop turns into, end to end.
//!
//! Every fixture here is built rather than checked in: a zip through the `zip`
//! crate, a rar by shelling the operator's `rar`, and the audio parts through
//! `ffmpeg`. A missing external tool skips the test it is needed for with a
//! message rather than failing, because "this machine has no `rar`" is not a
//! defect in the explode path — but the zip lane, the classifier, and the
//! staging lane have no external dependency at all and always run.

use std::{
	io::Write,
	path::{Path, PathBuf},
	process::Command,
	sync::Arc,
};

use models::{
	entity::{ingest_drop_item, library, library_config},
	shared::enums::FileStatus,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, Database, EntityTrait};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{
	archive::{self, ArchiveFormat},
	config::IngestSettings,
	contract::{DropItemStatus, IngestMediaKind},
	policy::AudioPolicy,
	store::{DropItemModel, IngestStore, Pagination},
};

const LIBRARY_ID: &str = "library";

/// A migrated database, one library, and a store whose drop and staging roots
/// are inside `root`.
async fn fixture(root: &Path) -> IngestStore {
	let conn = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
	<migrations::Migrator as migrations::MigratorTrait>::up(conn.as_ref(), None)
		.await
		.unwrap();
	let config = <library_config::ActiveModel as Default>::default()
		.insert(conn.as_ref())
		.await
		.unwrap();
	let library_root = root.join("library");
	std::fs::create_dir_all(&library_root).unwrap();
	library::ActiveModel {
		id: Set(LIBRARY_ID.to_string()),
		name: Set("Library".to_string()),
		path: Set(library_root.to_string_lossy().into_owned()),
		status: Set(FileStatus::Ready),
		config_id: Set(config.id),
		..Default::default()
	}
	.insert(conn.as_ref())
	.await
	.unwrap();
	IngestStore::new(Arc::new(IngestSettings::rooted_at(root)), conn)
}

fn drop_dir(root: &Path) -> PathBuf {
	let path = root.join("drop").join(LIBRARY_ID);
	std::fs::create_dir_all(&path).unwrap();
	path
}

/// Write a zip of `(member path, bytes)` pairs, stored uncompressed so a
/// corrupted payload is a CRC failure rather than a decode failure.
fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
	let file = std::fs::File::create(path).unwrap();
	let mut writer = ZipWriter::new(file);
	let options =
		SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
	for (name, bytes) in members {
		writer.start_file(*name, options).unwrap();
		writer.write_all(bytes).unwrap();
	}
	writer.finish().unwrap();
}

/// Enough bytes that a member is distinguishable, and not an image or a book
/// by content — classification is by name, which is the contract.
fn filler(tag: &str) -> Vec<u8> {
	format!("{tag}-{}", "x".repeat(64)).into_bytes()
}

fn items_by_name(items: &[DropItemModel]) -> Vec<(&str, &str)> {
	let mut named = items
		.iter()
		.map(|item| (item.source_filename.as_str(), item.media_kind.as_str()))
		.collect::<Vec<_>>();
	named.sort();
	named
}

/// A folder of MP3 parts inside one archive is one publication, not 35.
///
/// This is the collapse `PathUtils::dir_is_audio_book` performs for the
/// scanner: the directory becomes the item and its `cover.jpg`/`.nfo` become
/// that item's sidecars.
#[tokio::test]
async fn a_folder_of_mp3_parts_explodes_into_one_audio_item() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	let archive_path = drop_dir(temporary.path()).join("Three Body.zip");
	write_zip(
		&archive_path,
		&[
			("Three Body/01 - Chapter 1.mp3", &filler("one")),
			("Three Body/02 - Chapter 2.mp3", &filler("two")),
			("Three Body/03 - Chapter 3.mp3", &filler("three")),
			("Three Body/cover.jpg", &filler("cover")),
			("Three Body/info.nfo", &filler("nfo")),
		],
	);

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	assert_eq!(items.len(), 1, "{:?}", items_by_name(&items));
	let item = &items[0];
	assert_eq!(item.source_filename, "Three Body");
	assert_eq!(item.media_kind, "AUDIO");
	assert!(
		item.drop_group_id.is_some(),
		"an exploded item always names its delivery"
	);
	let staged = PathBuf::from(&item.staging_path);
	assert!(staged.is_dir(), "a folder book stages as a directory");
	assert_eq!(
		std::fs::read_dir(&staged).unwrap().count(),
		5,
		"every member of the folder book stages with it"
	);
	// The parts are the publication; only what describes it is a sidecar.
	let sidecars = IngestStore::sidecars(item);
	let names = sidecars
		.iter()
		.filter_map(|path| Path::new(path).file_name())
		.map(|name| name.to_string_lossy().into_owned())
		.collect::<Vec<_>>();
	assert_eq!(names, vec!["cover.jpg".to_string(), "info.nfo".to_string()]);
	assert!(
		!archive_path.exists(),
		"the archive was the delivery, not the book"
	);
}

/// An EPUB and a MOBI of one book are two items, and the cover beside them is
/// a sidecar rather than a third item.
#[tokio::test]
async fn an_ebook_delivery_explodes_into_one_item_per_book_with_a_cover_sidecar() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	let archive_path = drop_dir(temporary.path()).join("Ebooks.zip");
	write_zip(
		&archive_path,
		&[
			("Dark Eden/cover.jpg", &filler("cover")),
			("Dark Eden/Dark Eden - Beckett, Chris.epub", &filler("epub")),
			("Dark Eden/Dark Eden - Beckett, Chris.mobi", &filler("mobi")),
		],
	);

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	assert_eq!(
		items_by_name(&items),
		vec![
			("Dark Eden - Beckett, Chris.epub", "EPUB"),
			// A Kindle format has no page lane and no time lane; the
			// processor still reads it, which is what `UNKNOWN` records.
			("Dark Eden - Beckett, Chris.mobi", "UNKNOWN"),
		]
	);
	// Two publications and one cover: nothing can say which book the cover
	// belongs to, so it is reported rather than guessed at, and neither item
	// silently claims it.
	assert!(
		items
			.iter()
			.all(|item| IngestStore::sidecars(item).is_empty()),
		"an ambiguous cover is never attached to one of two books"
	);
	assert!(!archive_path.exists());
}

/// One cover and one book is unambiguous: the cover is that book's sidecar.
#[tokio::test]
async fn a_single_book_delivery_owns_the_cover_beside_it() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	write_zip(
		&drop_dir(temporary.path()).join("Book.zip"),
		&[
			("cover.jpg", &filler("cover")),
			("Piranesi.epub", &filler("epub")),
		],
	);

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	assert_eq!(items.len(), 1);
	let sidecars = IngestStore::sidecars(&items[0]);
	assert_eq!(sidecars.len(), 1, "{sidecars:?}");
	assert!(sidecars[0].ends_with("cover.jpg"), "{sidecars:?}");
}

/// A mixed delivery becomes a group: every item shares one `drop_group_id`,
/// which is what lets the editor show them together and what makes the
/// audiobook and the ebook of one book an edition-pair suggestion.
#[tokio::test]
async fn a_mixed_delivery_becomes_one_group_of_several_items() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	write_zip(
		&drop_dir(temporary.path()).join("Dark Eden.zip"),
		&[
			("Audiobook/Dark Eden 01-62.mp3", &filler("a1")),
			("Audiobook/Dark Eden 02-62.mp3", &filler("a2")),
			("Ebooks/Dark Eden.epub", &filler("epub")),
			("Ebooks/Dark Eden.mobi", &filler("mobi")),
			("Comics/Dark Eden.cbz", &filler("cbz")),
			("readme.url", &filler("url")),
		],
	);

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	assert_eq!(
		items_by_name(&items),
		vec![
			("Audiobook", "AUDIO"),
			("Dark Eden.cbz", "COMIC_ARCHIVE"),
			("Dark Eden.epub", "EPUB"),
			("Dark Eden.mobi", "UNKNOWN"),
		]
	);
	let groups = items
		.iter()
		.map(|item| item.drop_group_id.clone())
		.collect::<std::collections::BTreeSet<_>>();
	assert_eq!(groups.len(), 1, "one delivery is one group");
	assert!(groups.iter().all(Option::is_some));

	// The group is navigable from any member, which is what the editor's
	// sibling strip reads.
	let siblings = store.group_siblings(&items[0]).await.unwrap();
	assert_eq!(siblings.len(), 3);
	assert!(siblings.iter().all(|sibling| sibling.id != items[0].id));

	// Each item's relative path names the directory it commits *into*, so a
	// commit reproduces the delivery's shape. The audiobook's own folder is
	// the publication, so its relative path is the container root — naming
	// its own directory here would nest `Audiobook/` inside `Audiobook/`.
	let audio = items
		.iter()
		.find(|item| item.media_kind == "AUDIO")
		.unwrap();
	assert_eq!(audio.relative_path.as_deref(), None);
	assert_eq!(audio.source_filename, "Audiobook");
	let epub = items
		.iter()
		.find(|item| item.source_filename.ends_with(".epub"))
		.unwrap();
	assert_eq!(epub.relative_path.as_deref(), Some("Ebooks"));
}

/// A container of nothing but pages is a comic, and stays one item of one
/// book. This is the rule that keeps every existing `.cbz`/`.zip` comic drop
/// behaving exactly as it did before archives could explode.
#[tokio::test]
async fn a_container_of_pages_is_still_one_comic() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	let archive_path = drop_dir(temporary.path()).join("Berserk 01.zip");
	write_zip(
		&archive_path,
		&[
			("001.jpg", &filler("p1")),
			("002.jpg", &filler("p2")),
			("ComicInfo.xml", &filler("info")),
		],
	);

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	assert_eq!(items.len(), 1);
	assert_eq!(items[0].source_filename, "Berserk 01.zip");
	assert_eq!(items[0].media_kind, "COMIC_ARCHIVE");
	assert_eq!(
		items[0].drop_group_id, None,
		"one book is not a delivery of several"
	);
	assert!(
		PathBuf::from(&items[0].staging_path).is_file(),
		"the container itself is the staged book"
	);
}

/// A container that cannot be extracted fails one item carrying the
/// extractor's own message, and leaves the archive in the drop folder so
/// fixing the installation and rescanning is the whole recovery.
#[tokio::test]
async fn a_failed_extraction_fails_one_item_and_keeps_the_archive() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	let archive_path = drop_dir(temporary.path()).join("Broken.zip");
	write_zip(&archive_path, &[("Book.epub", &filler("epub"))]);
	// The central directory still lists `Book.epub`, so the drop is a
	// delivery; its payload no longer matches its checksum, so extracting it
	// is what fails. Exactly the shape of a truncated download.
	let mut bytes = std::fs::read(&archive_path).unwrap();
	let offset = bytes.len() / 3;
	bytes[offset] ^= 0xFF;
	std::fs::write(&archive_path, &bytes).unwrap();

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	assert_eq!(items.len(), 1);
	let item = &items[0];
	assert_eq!(item.status, DropItemStatus::Failed.as_str());
	assert_eq!(item.source_filename, "Broken.zip");
	let error = item.error.as_deref().unwrap_or_default();
	assert!(!error.is_empty(), "a failed drop always names its cause");
	assert!(
		archive_path.exists(),
		"a failed explode never consumes the archive"
	);

	// Rescanning updates the reason in place instead of accumulating a row
	// per scan.
	let again = store.scan_drop_folder(LIBRARY_ID).await.unwrap();
	assert_eq!(again.len(), 1);
	assert_eq!(again[0].id, item.id);
	let (all, total) = store
		.list_items(Some(LIBRARY_ID), None, Pagination::default())
		.await
		.unwrap();
	assert_eq!((all.len(), total), (1, 1));
}

/// Re-dropping the same delivery deduplicates onto the items it already
/// produced rather than creating a second group of the same books.
#[tokio::test]
async fn re_dropping_a_delivery_deduplicates_onto_the_existing_items() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	// A flat folder of audio is one book; the ebook sits beside it at the
	// container root. Two items, one of each shape a re-drop has to
	// deduplicate: a directory identity and a file identity.
	let members: [(&str, &[u8]); 3] = [
		("Book/01.mp3", &[1, 2, 3, 4]),
		("Book/02.mp3", &[5, 6, 7, 8]),
		("Book.epub", &[9, 10, 11, 12]),
	];
	write_zip(&drop_dir(temporary.path()).join("Drop.zip"), &members);
	let first = store.scan_drop_folder(LIBRARY_ID).await.unwrap();
	assert_eq!(
		items_by_name(&first),
		vec![("Book", "AUDIO"), ("Book.epub", "EPUB")]
	);

	write_zip(&drop_dir(temporary.path()).join("Drop.zip"), &members);
	let second = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	let mut first_ids = first.iter().map(|item| &item.id).collect::<Vec<_>>();
	let mut second_ids = second.iter().map(|item| &item.id).collect::<Vec<_>>();
	first_ids.sort();
	second_ids.sort();
	assert_eq!(first_ids, second_ids, "the same bytes are the same items");
	let total = ingest_drop_item::Entity::find()
		.all(store.conn().as_ref())
		.await
		.unwrap()
		.len();
	assert_eq!(total, 2);
}

/// The same explosion through the RAR lane, which on a build without the
/// `rar` feature exercises the operator-installed `unrar`: its listing parser
/// and its extraction argv.
#[tokio::test]
async fn a_rar_delivery_explodes_through_the_rar_lane() {
	let Some(rar) = external("rar") else {
		eprintln!("skipping: no `rar` on PATH to build the fixture archive");
		return;
	};
	if external("unrar").is_none() && !cfg!(feature = "rar") {
		eprintln!("skipping: no `unrar` on PATH and the `rar` feature is off");
		return;
	}
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;

	// `rar a` archives paths relative to its working directory, so the tree
	// is built there and added by name.
	let build = temporary.path().join("build");
	let book = build.join("Dark Eden");
	std::fs::create_dir_all(&book).unwrap();
	std::fs::write(book.join("01 - Part 1.mp3"), filler("one")).unwrap();
	std::fs::write(book.join("02 - Part 2.mp3"), filler("two")).unwrap();
	std::fs::write(book.join("cover.jpg"), filler("cover")).unwrap();
	std::fs::write(build.join("Dark Eden.epub"), filler("epub")).unwrap();

	let archive_path = drop_dir(temporary.path()).join("Dark Eden.rar");
	let status = Command::new(rar)
		.current_dir(&build)
		.args([
			"a",
			"-r",
			"-idq",
			&archive_path.to_string_lossy(),
			"Dark Eden",
			"Dark Eden.epub",
		])
		.status()
		.unwrap();
	assert!(status.success(), "building the rar fixture failed");

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();

	assert_eq!(
		items_by_name(&items),
		vec![("Dark Eden", "AUDIO"), ("Dark Eden.epub", "EPUB")],
		"a rar of an audiobook folder plus an ebook is two items"
	);
	let groups = items
		.iter()
		.map(|item| item.drop_group_id.clone())
		.collect::<std::collections::BTreeSet<_>>();
	assert_eq!(groups.len(), 1);
	let audio = items
		.iter()
		.find(|item| item.media_kind == "AUDIO")
		.unwrap();
	assert!(PathBuf::from(&audio.staging_path).is_dir());
	assert!(!archive_path.exists());
}

/// The member list of a real rar is read without extracting it, and a rar of
/// pages is still one comic.
#[tokio::test]
async fn a_rar_of_pages_is_listed_without_extraction_and_stays_one_comic() {
	let Some(rar) = external("rar") else {
		eprintln!("skipping: no `rar` on PATH to build the fixture archive");
		return;
	};
	if external("unrar").is_none() && !cfg!(feature = "rar") {
		eprintln!("skipping: no `unrar` on PATH and the `rar` feature is off");
		return;
	}
	let temporary = tempfile::tempdir().unwrap();
	let build = temporary.path().join("build");
	std::fs::create_dir_all(&build).unwrap();
	std::fs::write(build.join("001.jpg"), filler("p1")).unwrap();
	std::fs::write(build.join("002.jpg"), filler("p2")).unwrap();
	let archive_path = temporary.path().join("Berserk 01.cbr");
	assert!(Command::new(rar)
		.current_dir(&build)
		.args([
			"a",
			"-idq",
			&archive_path.to_string_lossy(),
			"001.jpg",
			"002.jpg"
		])
		.status()
		.unwrap()
		.success());

	let members = archive::list_members(ArchiveFormat::Rar, &archive_path).unwrap();

	let mut names = members
		.iter()
		.filter(|member| !member.is_dir)
		.map(|member| member.path.clone())
		.collect::<Vec<_>>();
	names.sort();
	assert_eq!(names, vec!["001.jpg".to_string(), "002.jpg".to_string()]);
	assert!(
		!archive::holds_publications(&members),
		"a rar of pages is one comic"
	);
}

/// `auto_assemble` turns a folder of parts into the canonical M4B, makes it
/// the item's file, and keeps the parts as sidecars.
///
/// Run against the real `audio-assemble` on real MP3s, because the two things
/// that can go wrong — the tool refusing its own output, and the item pointing
/// at a file that is not what the row describes — are invisible to a mock.
#[tokio::test]
async fn auto_assemble_replaces_a_folder_book_with_one_m4b() {
	let Some(ffmpeg) = external("ffmpeg") else {
		eprintln!("skipping: no `ffmpeg` on PATH to build the audio fixture");
		return;
	};
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	// The parts are built outside the drop folder and delivered as an
	// archive, which is the shape this test exists for: the assemble runs
	// against a *staged* folder book produced by an explode.
	let build = temporary.path().join("build");
	std::fs::create_dir_all(&build).unwrap();
	let mut members = Vec::new();
	for (index, frequency) in [220, 330, 440].into_iter().enumerate() {
		let name = format!("{:02} - Part {}.mp3", index + 1, index + 1);
		let part = build.join(&name);
		write_mp3(&ffmpeg, &part, frequency);
		members.push((format!("Sine Book/{name}"), std::fs::read(&part).unwrap()));
	}
	write_zip(
		&drop_dir(temporary.path()).join("Sine Book.zip"),
		&members
			.iter()
			.map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
			.collect::<Vec<_>>(),
	);

	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();
	assert_eq!(items.len(), 1, "{:?}", items_by_name(&items));
	let item = &items[0];
	assert_eq!(item.media_kind, "AUDIO");

	let analysed = store.run_audio_analysis(item).await.unwrap();
	let analysis =
		IngestStore::audio_analysis(&analysed).expect("an audio item is probed");
	assert_eq!(analysis.tracks.len(), 3, "one track per part");
	assert!(analysis.duration_ms > 0);
	assert_eq!(
		analysis.chapter_source, "per_track",
		"parts with no marks synthesize one chapter per file"
	);
	assert_eq!(analysis.chapters.len(), 3);
	// Offsets are publication-relative and monotonic, which is what a resume
	// depends on.
	let offsets = analysis
		.tracks
		.iter()
		.map(|track| track.start_offset_ms)
		.collect::<Vec<_>>();
	assert_eq!(offsets[0], 0);
	assert!(
		offsets[1] > offsets[0] && offsets[2] > offsets[1],
		"{offsets:?}"
	);

	let policy = AudioPolicy {
		auto_assemble: true,
		keep_original: true,
		..AudioPolicy::default()
	};
	let assembled = store.run_audio_assemble(&analysed, &policy).await.unwrap();

	let staged = PathBuf::from(&assembled.staging_path);
	assert_eq!(
		staged.extension().and_then(|extension| extension.to_str()),
		Some("m4b"),
		"the item's file is now the canonical container"
	);
	assert!(staged.is_file());
	assert_eq!(
		staged.metadata().unwrap().len() as i64,
		assembled.byte_size,
		"the row's size describes the file the library will receive"
	);
	assert_ne!(
		assembled.source_sha256, analysed.source_sha256,
		"the assembled book is re-hashed like a preprocess rewrite"
	);

	let after = IngestStore::audio_analysis(&assembled).unwrap();
	let produced = after.assembled.expect("the assemble is recorded");
	assert_eq!(produced.chapters, 3);
	assert!(produced.faststart, "moov must precede mdat");
	assert!(produced.parts_kept);
	assert!(produced.byte_size > 0);
	assert!(
		produced.duration_ms > 0,
		"the output is re-probed, not trusted from the plan"
	);

	// The parts became sidecars: still on disk, no longer the publication.
	let sidecars = IngestStore::sidecars(&assembled);
	assert_eq!(sidecars.len(), 3, "{sidecars:?}");
	assert!(sidecars
		.iter()
		.all(|path| Path::new(path).is_file() && path.ends_with(".mp3")));

	// Re-running is a no-op: the item is already one container.
	let again = store.run_audio_assemble(&assembled, &policy).await.unwrap();
	assert_eq!(again.staging_path, assembled.staging_path);
	assert_eq!(again.revision, assembled.revision);
}

/// The default policy analyses and reports without rewriting anything, which
/// is the only behaviour that is safe to run over somebody's library
/// unattended.
#[tokio::test]
async fn the_default_policy_leaves_a_folder_book_alone() {
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	write_zip(
		&drop_dir(temporary.path()).join("Book.zip"),
		&[
			("Book/01.mp3", &filler("one")),
			("Book/02.mp3", &filler("two")),
		],
	);
	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();
	let item = &items[0];

	let policy = store.audio_policy(LIBRARY_ID).await.unwrap();
	assert!(!policy.auto_assemble, "assembling is opt-in per library");
	let unchanged = store.run_audio_assemble(item, &policy).await.unwrap();

	assert_eq!(unchanged.staging_path, item.staging_path);
	assert!(PathBuf::from(&unchanged.staging_path).is_dir());
}

/// A publication kind pair only becomes an edition suggestion across the
/// audio/text boundary — asserted here against the kinds an explode actually
/// produces.
#[test]
fn a_delivery_pairs_its_audiobook_with_its_ebook() {
	assert!(crate::pairing::is_edition_pair(
		IngestMediaKind::Audio,
		IngestMediaKind::Epub
	));
	assert!(!crate::pairing::is_edition_pair(
		IngestMediaKind::Epub,
		IngestMediaKind::ComicArchive
	));
}

/// A binary on `PATH`, or `None`.
fn external(name: &str) -> Option<PathBuf> {
	std::env::var_os("PATH").and_then(|paths| {
		std::env::split_paths(&paths)
			.map(|directory| directory.join(name))
			.find(|candidate| stump_tools::external::is_executable(candidate))
	})
}

/// One second of a sine tone as a real, decodable MP3.
fn write_mp3(ffmpeg: &Path, target: &Path, frequency: u32) {
	let output = Command::new(ffmpeg)
		.args([
			"-nostdin",
			"-loglevel",
			"error",
			"-y",
			"-f",
			"lavfi",
			"-i",
			&format!("sine=frequency={frequency}:duration=1"),
			"-c:a",
			"libmp3lame",
			"-b:a",
			"64k",
			&target.to_string_lossy(),
		])
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"ffmpeg could not build the fixture: {}",
		String::from_utf8_lossy(&output.stderr)
	);
}

/// The real archives, end to end.
///
/// Gated on `STUMP_LIVE_INPUT` because it reads hundreds of megabytes of a
/// user's own downloads and transcodes them; it is a live proof, not a
/// regression test. Run:
///
/// ```text
/// STUMP_LIVE_INPUT=/home/al/Code/stump/input cargo test -p stump_ingest \
///   live_archive_drop_of_the_real_input -- --nocapture --exact
/// ```
#[tokio::test]
async fn live_archive_drop_of_the_real_input() {
	let Ok(input) = std::env::var("STUMP_LIVE_INPUT") else {
		eprintln!("skipping: STUMP_LIVE_INPUT is unset");
		return;
	};
	let input = PathBuf::from(input).join("audio");
	let temporary = tempfile::tempdir().unwrap();
	let store = fixture(temporary.path()).await;
	let drop = drop_dir(temporary.path());
	std::fs::create_dir_all(drop.join("Dark Eden")).unwrap();
	for (from, to) in [
		(
			input
				.join("Liu, Cixin - Three Body 01 - The Three-Body Problem 2014 mp3.rar"),
			drop.join("Liu, Cixin - Three Body 01 - The Three-Body Problem 2014 mp3.rar"),
		),
		(
			input.join("The Three-Body Problem - Cixin Liu.epub"),
			drop.join("The Three-Body Problem - Cixin Liu.epub"),
		),
		(
			input.join("Dark Eden/Audiobook.rar"),
			drop.join("Dark Eden/Audiobook.rar"),
		),
		(
			input.join("Dark Eden/Ebooks.rar"),
			drop.join("Dark Eden/Ebooks.rar"),
		),
	] {
		std::fs::copy(&from, &to)
			.unwrap_or_else(|error| panic!("copy {}: {error}", from.display()));
	}

	let scan_started = std::time::Instant::now();
	let items = store.scan_drop_folder(LIBRARY_ID).await.unwrap();
	eprintln!(
		"\n=== drop scan: {} item(s) in {:.1}s ===",
		items.len(),
		scan_started.elapsed().as_secs_f64()
	);

	let registry = crate::quality::QualityRegistry::builtin(store.conn().clone());
	let policy = AudioPolicy {
		auto_assemble: true,
		keep_original: true,
		..AudioPolicy::default()
	};

	for item in &items {
		eprintln!(
			"\n--- {} | kind={} | group={} | status={} ---",
			item.source_filename,
			item.media_kind,
			item.drop_group_id.as_deref().unwrap_or("-"),
			item.status
		);
		if let Some(error) = &item.error {
			eprintln!("    error: {error}");
			continue;
		}
		let analysed = store.run_audio_analysis(item).await.unwrap();
		if let Some(audio) = IngestStore::audio_analysis(&analysed) {
			eprintln!(
				"    audio: {}ms codec={} bitrate={:?} tracks={} chapters={} source={}",
				audio.duration_ms,
				audio.codec,
				audio.bitrate,
				audio.tracks.len(),
				audio.chapters.len(),
				audio.chapter_source
			);
			eprintln!(
				"    tags: title={:?} author={:?} narrator={:?} album={:?} year={:?} cover={:?}",
				audio.title,
				audio.author,
				audio.narrator,
				audio.album,
				audio.year,
				audio.cover_content_type
			);
			for chapter in audio.chapters.iter().take(3) {
				eprintln!(
					"      ch {:?} @ {}ms",
					chapter.title.as_deref().unwrap_or("-"),
					chapter.start_ms
				);
			}
		}
		let snapshot = store.snapshot(&analysed.id).await.unwrap();
		let report = registry
			.run_all(&snapshot, &std::collections::BTreeMap::new())
			.await
			.unwrap();
		let failed = report
			.checks
			.iter()
			.filter(|check| {
				matches!(check.outcome.status, crate::contract::QualityStatus::Fail)
			})
			.map(|check| check.outcome.check_id.as_str())
			.collect::<Vec<_>>();
		eprintln!("    quality: {}/100 failed={failed:?}", report.score);
		eprintln!(
			"    embedded title={:?}",
			snapshot
				.embedded_metadata
				.as_ref()
				.and_then(|meta| meta.title.clone())
		);

		if analysed.media_kind == "AUDIO" {
			let started = std::time::Instant::now();
			let assembled = store.run_audio_assemble(&analysed, &policy).await.unwrap();
			let elapsed = started.elapsed().as_secs_f64();
			match IngestStore::audio_analysis(&assembled).and_then(|a| a.assembled) {
				Some(produced) => eprintln!(
					"    ASSEMBLED in {elapsed:.1}s -> {} ({} bytes, {}ms, {} chapters, faststart={}, parts_kept={}, method={})\n      path: {}",
					produced.filename,
					produced.byte_size,
					produced.duration_ms,
					produced.chapters,
					produced.faststart,
					produced.parts_kept,
					produced.method,
					assembled.staging_path
				),
				None => eprintln!("    assemble produced nothing after {elapsed:.1}s"),
			}
			eprintln!(
				"    sidecars after assemble: {}",
				IngestStore::sidecars(&assembled).len()
			);
		}
		let siblings = store.group_siblings(item).await.unwrap();
		if !siblings.is_empty() {
			eprintln!(
				"    siblings: {:?}",
				siblings
					.iter()
					.map(|s| (s.source_filename.as_str(), s.media_kind.as_str()))
					.collect::<Vec<_>>()
			);
			for sibling in &siblings {
				eprintln!(
					"      pair candidate with {}: {}",
					sibling.source_filename,
					crate::pairing::is_edition_pair(
						crate::store::media_kind_for_item(item),
						crate::store::media_kind_for_item(sibling)
					)
				);
			}
		}
	}
}
