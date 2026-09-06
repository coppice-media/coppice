//! Reader for an audio publication: one container file (`.m4b`, `.mp3`, …) or
//! one flat folder of parts.
//!
//! An audiobook is not a paginated document, so most of [`FileProcessor`] is
//! meaningless for it. It implements the trait anyway because the trait is the
//! *only* entry point the scanner and the builder have: `process` is what
//! turns a path into a [`ProcessedFile`], and a shape that cannot be processed
//! cannot be scanned. The page-oriented methods therefore fail loudly rather
//! than inventing a page — a caller that reaches them has routed an audiobook
//! into the image pipeline, and a plausible-looking wrong answer would hide
//! that bug behind a broken reader.
//!
//! Everything this processor knows comes from [`crate::audio::probe`], which
//! dispatches on `is_dir()` itself, so a container and a folder of parts are
//! the same thing here.

use std::{collections::HashMap, path::Path, path::PathBuf};

use models::services::audio::AudioFacts;

use crate::{
	audio::{self, ProbedAudio},
	content_type::ContentType,
	error::FileError,
	hash::{self, generate_koreader_hash},
	media::{
		process::{AnalyzedPage, FileProcessor, FileProcessorOptions, ProcessedFile},
		ProcessedFileHashes, ProcessedMediaMetadata,
	},
	MediaConfig,
};

/// The `media.pages` value of an audiobook. `-1` is Stump's existing marker
/// for a publication that has no pages at all (see the `pages` doc comment on
/// `models::entity::media`), so no consumer has to special-case audio to know
/// there is nothing to paginate.
pub const AUDIO_PAGES: i32 = -1;

/// A file processor for audiobooks.
pub struct AudioProcessor;

impl AudioProcessor {
	/// The file whose bytes stand in for the publication when hashing.
	///
	/// A single container hashes itself. A folder book has no single file to
	/// sample, so it hashes its first part in the natural filename order
	/// [`audio::audio_files_in`] guarantees. That order is a pure function of
	/// the folder's contents, so the same folder always samples the same file,
	/// and — unlike playback order, which a complete `TRCK` numbering can
	/// permute — it costs no probe. The hash is an identity hint, not a
	/// checksum of the whole book: sampling the first part is the same
	/// approximation every other processor makes by sampling the first pages.
	fn hashed_file(path: &Path) -> Result<PathBuf, FileError> {
		if !path.is_dir() {
			return Ok(path.to_path_buf());
		}

		audio::audio_files_in(path)?
			.into_iter()
			.next()
			.ok_or_else(|| FileError::UnsupportedFileType(path.display().to_string()))
	}

	/// The probed tags in the shape the builder persists.
	///
	/// Only fields `media_metadata` actually has are set. The probe's
	/// `narrator` has no column anywhere in the schema — see
	/// `crates/abs/src/model.rs`, which documents the same gap — so it stays
	/// on the probe rather than being smuggled into an unrelated field.
	fn metadata_of(probed: &ProbedAudio) -> ProcessedMediaMetadata {
		ProcessedMediaMetadata {
			title: publication_title(probed),
			// `artist`/`album_artist` is the author for an audiobook, which is
			// the same role ComicInfo's `Writer` names.
			writers: probed.author.clone().map(|author| vec![author]),
			summary: probed.description.clone(),
			genres: probed.genre.clone().map(|genre| vec![genre]),
			year: probed.year,
			..Default::default()
		}
	}
}

/// The publication's title, which is not the same tag in both shapes.
///
/// A single container *is* the publication, so its `title` names the book. In
/// a folder book each file's `title` names that part ("Chapter 1"), and the
/// book's own name is the `album` every part shares — so the album wins there,
/// and the per-part title is only a fallback for an untagged folder.
fn publication_title(probed: &ProbedAudio) -> Option<String> {
	let (first, second) = if probed.tracks.len() > 1 {
		(&probed.album, &probed.title)
	} else {
		(&probed.title, &probed.album)
	};

	first.clone().or_else(|| second.clone())
}

/// An audiobook has no pages; every page-oriented call is a routing bug.
fn no_pages(path: &str) -> FileError {
	FileError::UnsupportedFileType(format!(
		"{path} is an audiobook and has no pages; serve its tracks instead"
	))
}

impl FileProcessor for AudioProcessor {
	fn get_sample_size(path: &str) -> Result<u64, FileError> {
		Ok(Self::hashed_file(Path::new(path))?.metadata()?.len())
	}

	fn generate_stump_hash(path: &str) -> Option<String> {
		let file = Self::hashed_file(Path::new(path)).ok()?;
		let sample = file.metadata().ok()?.len();

		match hash::generate(file.to_str()?, sample) {
			Ok(digest) => Some(digest),
			Err(error) => {
				tracing::debug!(error = ?error, path, "Failed to digest audiobook");
				None
			},
		}
	}

	/// Both hashes sample [`AudioProcessor::hashed_file`]: a directory has no
	/// bytes of its own, and a KOReader hash of one is an IO error rather than
	/// a digest.
	fn generate_hashes(
		path: &str,
		FileProcessorOptions {
			generate_file_hashes,
			generate_koreader_hashes,
			..
		}: FileProcessorOptions,
	) -> Result<ProcessedFileHashes, FileError> {
		let hash = generate_file_hashes
			.then(|| Self::generate_stump_hash(path))
			.flatten();
		let koreader_hash = generate_koreader_hashes
			.then(|| {
				Self::hashed_file(Path::new(path)).and_then(|file| {
					generate_koreader_hash(file).map_err(FileError::from)
				})
			})
			.transpose()?;

		Ok(ProcessedFileHashes {
			hash,
			koreader_hash,
		})
	}

	fn process(
		path: &str,
		options: FileProcessorOptions,
		_: &MediaConfig,
	) -> Result<ProcessedFile, FileError> {
		tracing::trace!(?path, "Processing audiobook");

		let probed = audio::probe(Path::new(path))?;
		let ProcessedFileHashes {
			hash,
			koreader_hash,
		} = Self::generate_hashes(path, options)?;

		Ok(ProcessedFile {
			path: PathBuf::from(path),
			hash,
			koreader_hash,
			metadata: options.process_metadata.then(|| Self::metadata_of(&probed)),
			pages: AUDIO_PAGES,
			audio: Some(AudioFacts::from(&probed)),
		})
	}

	fn process_metadata(path: &str) -> Result<Option<ProcessedMediaMetadata>, FileError> {
		let probed = audio::probe(Path::new(path))?;
		Ok(Some(Self::metadata_of(&probed)))
	}

	fn get_page(
		path: &str,
		_page: i32,
		_: &MediaConfig,
	) -> Result<(ContentType, Vec<u8>), FileError> {
		Err(no_pages(path))
	}

	fn get_page_count(_path: &str, _: &MediaConfig) -> Result<i32, FileError> {
		Ok(AUDIO_PAGES)
	}

	fn get_page_content_types(
		path: &str,
		_pages: Vec<i32>,
	) -> Result<HashMap<i32, ContentType>, FileError> {
		Err(no_pages(path))
	}

	fn analyze_page(
		path: &str,
		_page: i32,
		_: &MediaConfig,
	) -> Result<AnalyzedPage, FileError> {
		Err(no_pages(path))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fixture(name: &str) -> String {
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("integration-tests/data/audio")
			.join(name)
			.to_string_lossy()
			.to_string()
	}

	fn options() -> FileProcessorOptions {
		FileProcessorOptions {
			generate_file_hashes: true,
			process_metadata: true,
			..Default::default()
		}
	}

	/// The builder reads `pages` and `audio` off the processed file; a folder
	/// of parts must arrive as ONE publication carrying every track, or the
	/// scan writes twelve books where the shelf holds one.
	#[test]
	fn audio_processor_processes_a_folder_as_one_publication() {
		let path = fixture("folder-filename");
		let processed =
			AudioProcessor::process(&path, options(), &MediaConfig::default()).unwrap();

		assert_eq!(processed.pages, AUDIO_PAGES);
		let facts = processed.audio.expect("a folder book carries audio facts");
		assert_eq!(facts.tracks.len(), 3);
		assert_eq!(
			facts.duration_ms,
			facts
				.tracks
				.iter()
				.map(|track| track.duration_ms)
				.sum::<i64>()
		);
		assert!(processed.hash.is_some(), "a folder book is still hashable");
	}

	/// A folder has no bytes of its own: hashing it directly is an IO error,
	/// so the first part stands in — and the same folder must always pick the
	/// same part, or every rescan invents a new identity.
	#[test]
	fn audio_processor_hashes_the_first_part_of_a_folder() {
		let folder = fixture("folder-filename");
		let first_part = fixture("folder-filename/01 - Part 1.mp3");

		assert_eq!(
			AudioProcessor::generate_stump_hash(&folder),
			AudioProcessor::generate_stump_hash(&first_part),
		);
		assert_eq!(
			AudioProcessor::get_sample_size(&folder).unwrap(),
			std::fs::metadata(&first_part).unwrap().len(),
		);
	}

	/// A per-part `title` tag names the part, not the book, so a folder book's
	/// title comes from the shared `album`. `folder-tracknum` carries both
	/// (`title=Opening`, `album=Folder Book`); getting this backwards labels
	/// the whole publication "Opening" in every client.
	#[test]
	fn audio_processor_titles_a_folder_book_from_its_album() {
		let folder = AudioProcessor::process_metadata(&fixture("folder-tracknum"))
			.unwrap()
			.expect("a probe always yields metadata");
		assert_eq!(folder.title.as_deref(), Some("Folder Book"));

		// A single container *is* the publication, so its own title wins.
		let container = AudioProcessor::process_metadata(&fixture("chapters-chpl.m4b"))
			.unwrap()
			.expect("a probe always yields metadata");
		assert_eq!(container.title.as_deref(), Some("Silent Test Book"));
		assert_eq!(container.writers, Some(vec!["Test Narrator".to_string()]));
	}

	/// The page routes are unreachable for audio, and a silent wrong answer
	/// would surface as a broken reader instead of a routing bug.
	#[test]
	fn audio_processor_refuses_every_page_operation() {
		let path = fixture("plain.m4b");
		let config = MediaConfig::default();

		assert!(matches!(
			AudioProcessor::get_page(&path, 1, &config),
			Err(FileError::UnsupportedFileType(_))
		));
		assert!(matches!(
			AudioProcessor::get_page_content_types(&path, vec![1]),
			Err(FileError::UnsupportedFileType(_))
		));
		assert!(matches!(
			AudioProcessor::analyze_page(&path, 1, &config),
			Err(FileError::UnsupportedFileType(_))
		));
		assert_eq!(
			AudioProcessor::get_page_count(&path, &config).unwrap(),
			AUDIO_PAGES
		);
	}
}
