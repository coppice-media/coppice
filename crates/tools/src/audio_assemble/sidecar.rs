//! Chapter marks that live *beside* an audiobook instead of inside it.
//!
//! A folder audiobook that came from anywhere but a store almost always ships
//! its chapter list as a text file, because the files themselves cannot carry
//! one: an MP3 part has no place to say "chapter 4 starts 12 minutes into me
//! and is called *The Wall*". Three formats cover essentially everything a
//! librarian actually has on disk, and all three are read here so
//! [`crate::audio_assemble`] can bake them into the container it builds:
//!
//! | File | Origin | Shape |
//! | --- | --- | --- |
//! | `chapters.txt` | `mp4chaps`, AAXtoMP3, m4b-tool | `HH:MM:SS.mmm Title` per line |
//! | `*.cue` | CD rips, EAC, `shntool` | `TRACK`/`TITLE`/`INDEX MM:SS:FF` |
//! | `metadata.json` | `audible-cli`, AAXtoMP3 | Audible's own `chapter_info` |
//!
//! Every parser returns marks in **publication milliseconds**, ascending, so
//! the caller never has to know which file a list came out of. A file that
//! parses to nothing is not an error: a `chapters.txt` holding a track listing
//! and no timestamps is simply not a chapter list, and the caller falls
//! through to the next source in its own priority order.
//!
//! # Why not one permissive parser
//!
//! The three formats disagree about the *unit* of the number they write —
//! milliseconds, hundredths, and CD frames of 1/75 s — and two of them use
//! `MM:SS:xx` shapes that are indistinguishable without knowing the format.
//! Guessing the unit from the value is how a chapter list ends up 22% out, so
//! the format is chosen by filename and each parser knows exactly one unit.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One parsed mark, in publication milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidecarMark {
	pub title: String,
	pub start_ms: i64,
}

/// Which file a mark list came out of, for the plan detail. Provenance, so a
/// librarian reviewing a plan can see *why* the tool disagrees with the file
/// list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarKind {
	/// `chapters.txt` in the `mp4chaps` line format.
	ChaptersTxt,
	/// A CD cue sheet.
	Cue,
	/// Audible's `metadata.json`.
	AudibleJson,
}

impl SidecarKind {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::ChaptersTxt => "chapters.txt",
			Self::Cue => "cue",
			Self::AudibleJson => "audible-json",
		}
	}
}

/// A sidecar chapter list and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
	pub kind: SidecarKind,
	pub path: PathBuf,
	pub marks: Vec<SidecarMark>,
}

/// The best sidecar chapter list beside `book`, or `None`.
///
/// `book` is the publication: a folder is searched directly, and a single
/// container is searched in its own directory for a sidecar sharing its file
/// stem (`Book.m4b` + `Book.cue`) before the generic names. The formats are
/// tried in descending fidelity — Audible's JSON carries exact millisecond
/// offsets it was mastered with, `chapters.txt` carries whatever the person
/// who typed it managed, and a cue sheet is quantised to 1/75 s — so a book
/// that ships two of them gets the better one.
pub fn discover(book: &Path) -> Option<Sidecar> {
	let (dir, stem) = if book.is_dir() {
		(book.to_path_buf(), None)
	} else {
		(
			book.parent()?.to_path_buf(),
			book.file_stem()
				.map(|stem| stem.to_string_lossy().to_string()),
		)
	};

	let mut candidates: Vec<(SidecarKind, PathBuf)> = Vec::new();
	let mut push = |kind: SidecarKind, name: String| {
		let path = dir.join(name);
		if path.is_file() {
			candidates.push((kind, path));
		}
	};

	if let Some(stem) = &stem {
		push(SidecarKind::AudibleJson, format!("{stem}.json"));
		push(SidecarKind::ChaptersTxt, format!("{stem}.chapters.txt"));
		push(SidecarKind::Cue, format!("{stem}.cue"));
	}
	push(SidecarKind::AudibleJson, "metadata.json".to_string());
	push(SidecarKind::ChaptersTxt, "chapters.txt".to_string());
	// A folder rip names its cue after the album, not after a convention, so
	// the only reliable way to find one is to look for the extension.
	if let Some(cue) = first_with_extension(&dir, "cue") {
		candidates.push((SidecarKind::Cue, cue));
	}

	for (kind, path) in candidates {
		let Ok(text) = std::fs::read_to_string(&path) else {
			continue;
		};
		let marks = match kind {
			SidecarKind::ChaptersTxt => parse_chapters_txt(&text),
			SidecarKind::Cue => parse_cue(&text),
			SidecarKind::AudibleJson => parse_audible_json(&text),
		};
		if !marks.is_empty() {
			return Some(Sidecar { kind, path, marks });
		}
	}

	None
}

/// The alphabetically first file in `dir` with `extension`.
///
/// Deterministic on purpose: `read_dir` order is filesystem order, and a plan
/// that picks a different cue sheet on two runs over the same folder is a plan
/// nobody can review.
fn first_with_extension(dir: &Path, extension: &str) -> Option<PathBuf> {
	let mut found: Option<PathBuf> = None;
	for entry in std::fs::read_dir(dir).ok()?.filter_map(Result::ok) {
		let path = entry.path();
		let matches = path.is_file()
			&& path
				.extension()
				.and_then(|value| value.to_str())
				.is_some_and(|value| value.eq_ignore_ascii_case(extension));
		if matches && found.as_ref().is_none_or(|best| path < *best) {
			found = Some(path);
		}
	}
	found
}

/// `mp4chaps` lines: `HH:MM:SS.mmm Title`.
///
/// The separator between the timestamp and the title is any run of whitespace;
/// the hour field and the fractional part are both optional, because every
/// generator writes a slightly different subset and a listener typing one by
/// hand writes the shortest form that works. A line without a leading
/// timestamp is a comment or a stray track listing and is skipped rather than
/// failing the file.
#[must_use]
pub fn parse_chapters_txt(text: &str) -> Vec<SidecarMark> {
	let mut marks = Vec::new();
	for line in text.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
			continue;
		}
		let (stamp, title) = match line.split_once(char::is_whitespace) {
			Some((stamp, rest)) => (stamp, rest.trim()),
			// A bare timestamp is a real mark with no title.
			None => (line, ""),
		};
		let Some(start_ms) = parse_timestamp(stamp) else {
			continue;
		};
		marks.push(SidecarMark {
			title: title.to_string(),
			start_ms,
		});
	}
	finish(marks)
}

/// `[[HH:]MM:]SS[.mmm]` in decimal seconds.
///
/// Returns `None` for anything that is not entirely a timestamp, which is what
/// makes a prose line distinguishable from a mark.
fn parse_timestamp(value: &str) -> Option<i64> {
	let (whole, fraction) = match value.split_once('.') {
		Some((whole, fraction)) => (whole, Some(fraction)),
		None => (value, None),
	};

	let mut total_seconds: i64 = 0;
	let mut fields = 0;
	for field in whole.split(':') {
		if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
			return None;
		}
		total_seconds = total_seconds
			.checked_mul(60)?
			.checked_add(field.parse::<i64>().ok()?)?;
		fields += 1;
	}
	if fields == 0 || fields > 3 {
		return None;
	}

	let millis = match fraction {
		Some(fraction) => {
			if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit())
			{
				return None;
			}
			// `.5` is 500 ms and `.5000` is still 500 ms: pad or truncate to
			// exactly three digits rather than parsing an unknown scale.
			let mut digits = fraction.to_string();
			digits.truncate(3);
			while digits.len() < 3 {
				digits.push('0');
			}
			digits.parse::<i64>().ok()?
		},
		None => 0,
	};

	total_seconds.checked_mul(1000)?.checked_add(millis)
}

/// A CD cue sheet.
///
/// Only `TRACK`, `TITLE` and `INDEX 01` matter: `INDEX 00` is the pre-gap, and
/// a chapter starts where playback does. `INDEX` times are `MM:SS:FF` where
/// `FF` counts CD frames of 1/75 s — the one place a `:`-separated third field
/// is *not* a fraction of a second, and the reason cue sheets get their own
/// parser.
///
/// A `TITLE` before the first `TRACK` is the album title, not a chapter's, so
/// titles are only collected inside a track block.
#[must_use]
pub fn parse_cue(text: &str) -> Vec<SidecarMark> {
	const FRAMES_PER_SECOND: i64 = 75;

	let mut marks: Vec<SidecarMark> = Vec::new();
	let mut in_track = false;
	let mut title: Option<String> = None;

	for line in text.lines() {
		let line = line.trim();
		let mut words = line.split_whitespace();
		match words.next().map(str::to_ascii_uppercase).as_deref() {
			Some("TRACK") => {
				in_track = true;
				title = None;
			},
			Some("TITLE") if in_track => {
				title = Some(unquote(line["TITLE".len()..].trim()).to_string());
			},
			Some("INDEX") if in_track => {
				// `INDEX 01 MM:SS:FF`; index 00 is the pre-gap.
				let Some(number) = words.next() else { continue };
				if number.trim_start_matches('0').parse::<u32>().unwrap_or(0) != 1 {
					continue;
				}
				let Some(stamp) = words.next() else { continue };
				let fields: Vec<&str> = stamp.split(':').collect();
				let [minutes, seconds, frames] = fields.as_slice() else {
					continue;
				};
				let (Ok(minutes), Ok(seconds), Ok(frames)) = (
					minutes.parse::<i64>(),
					seconds.parse::<i64>(),
					frames.parse::<i64>(),
				) else {
					continue;
				};
				let start_ms =
					(minutes * 60 + seconds) * 1000 + (frames * 1000) / FRAMES_PER_SECOND;
				marks.push(SidecarMark {
					title: title.clone().unwrap_or_default(),
					start_ms,
				});
			},
			_ => {},
		}
	}

	finish(marks)
}

fn unquote(value: &str) -> &str {
	value
		.strip_prefix('"')
		.and_then(|rest| rest.strip_suffix('"'))
		.unwrap_or(value)
}

/// Audible's `chapter_info`, as `audible-cli` and AAXtoMP3 write it.
///
/// The list is either nested under `content_metadata.chapter_info.chapters` —
/// the shape the store's own API returns — or hoisted to the document root by
/// a downloader that kept only the part it needed. Both are accepted because
/// both are what is actually on disk. Offsets are already milliseconds from
/// the start of the publication, which is why this format outranks the other
/// two: nothing has to be reconstructed.
///
/// Audible nests part chapters under their parent (`chapters[].chapters[]`).
/// The nesting is flattened, because a player shows one list and a parent mark
/// whose children are invisible is a chapter a listener cannot reach.
#[must_use]
pub fn parse_audible_json(text: &str) -> Vec<SidecarMark> {
	let Ok(document) = serde_json::from_str::<AudibleDocument>(text) else {
		return Vec::new();
	};

	let chapters = document
		.content_metadata
		.and_then(|metadata| metadata.chapter_info)
		.map(|info| info.chapters)
		.filter(|chapters| !chapters.is_empty())
		.or_else(|| document.chapter_info.map(|info| info.chapters))
		.filter(|chapters| !chapters.is_empty())
		.or(document.chapters)
		.unwrap_or_default();

	let mut marks = Vec::new();
	flatten(&chapters, &mut marks);
	finish(marks)
}

fn flatten(chapters: &[AudibleChapter], marks: &mut Vec<SidecarMark>) {
	for chapter in chapters {
		let start_ms = chapter.start_offset_ms.or_else(|| {
			chapter
				.start_offset_sec
				.and_then(|seconds| seconds.checked_mul(1000))
		});
		if let Some(start_ms) = start_ms {
			marks.push(SidecarMark {
				title: chapter.title.clone().unwrap_or_default(),
				start_ms,
			});
		}
		flatten(&chapter.chapters, marks);
	}
}

/// Sort ascending, drop negatives and exact duplicates, and name the untitled.
///
/// A duplicate start is two names for one moment and every player renders the
/// second one as an unreachable row; the first spelling wins because a list is
/// written in the order its author meant.
fn finish(mut marks: Vec<SidecarMark>) -> Vec<SidecarMark> {
	marks.retain(|mark| mark.start_ms >= 0);
	marks.sort_by_key(|mark| mark.start_ms);
	marks.dedup_by_key(|mark| mark.start_ms);
	for (index, mark) in marks.iter_mut().enumerate() {
		if mark.title.trim().is_empty() {
			mark.title = format!("Chapter {}", index + 1);
		}
	}
	marks
}

#[derive(Debug, Deserialize)]
struct AudibleDocument {
	content_metadata: Option<AudibleContentMetadata>,
	chapter_info: Option<AudibleChapterInfo>,
	chapters: Option<Vec<AudibleChapter>>,
}

#[derive(Debug, Deserialize)]
struct AudibleContentMetadata {
	chapter_info: Option<AudibleChapterInfo>,
}

#[derive(Debug, Deserialize)]
struct AudibleChapterInfo {
	#[serde(default)]
	chapters: Vec<AudibleChapter>,
}

#[derive(Debug, Deserialize)]
struct AudibleChapter {
	title: Option<String>,
	start_offset_ms: Option<i64>,
	start_offset_sec: Option<i64>,
	#[serde(default)]
	chapters: Vec<AudibleChapter>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fs;
	use tempfile::TempDir;

	fn mark(title: &str, start_ms: i64) -> SidecarMark {
		SidecarMark {
			title: title.to_string(),
			start_ms,
		}
	}

	#[test]
	fn chapters_txt_reads_every_timestamp_shape() {
		let marks = parse_chapters_txt(
			"00:00:00.000 Opening Credits\n\
			 0:03:12.5 Chapter One\n\
			 12:30 A Short Form\n\
			 01:00:00 The Second Hour\n",
		);

		assert_eq!(
			marks,
			vec![
				mark("Opening Credits", 0),
				mark("Chapter One", 192_500),
				mark("A Short Form", 750_000),
				mark("The Second Hour", 3_600_000),
			]
		);
	}

	/// A track listing with no timestamps is not a chapter list.
	#[test]
	fn chapters_txt_ignores_lines_that_are_not_marks() {
		let marks = parse_chapters_txt(
			"# Berserk, Volume 1\n\
			 ; generated by hand\n\
			 Chapter One .......... page 3\n\
			 00:00:01 Real Mark\n",
		);

		assert_eq!(marks, vec![mark("Real Mark", 1000)]);
	}

	#[test]
	fn chapters_txt_names_an_untitled_mark() {
		assert_eq!(
			parse_chapters_txt("00:00:00\n00:00:10 Named\n"),
			vec![mark("Chapter 1", 0), mark("Named", 10_000)]
		);
	}

	/// CD frames are 1/75 s, not hundredths: `00:02:37` is 2.493 s, and
	/// reading it as a fraction would put the mark 100 ms out.
	#[test]
	fn cue_converts_frames_not_hundredths() {
		let marks = parse_cue(
			"TITLE \"The Album\"\n\
			 FILE \"book.wav\" WAVE\n\
			 \x20 TRACK 01 AUDIO\n\
			 \x20   TITLE \"Chapter One\"\n\
			 \x20   INDEX 01 00:00:00\n\
			 \x20 TRACK 02 AUDIO\n\
			 \x20   TITLE \"Chapter Two\"\n\
			 \x20   INDEX 00 00:02:00\n\
			 \x20   INDEX 01 00:02:37\n",
		);

		assert_eq!(
			marks,
			vec![mark("Chapter One", 0), mark("Chapter Two", 2493)]
		);
	}

	/// The album `TITLE` precedes the first `TRACK` and is not a chapter name.
	#[test]
	fn cue_does_not_borrow_the_album_title() {
		let marks = parse_cue(
			"TITLE \"The Album\"\n\
			 TRACK 01 AUDIO\n\
			 INDEX 01 00:00:00\n",
		);

		assert_eq!(marks, vec![mark("Chapter 1", 0)]);
	}

	#[test]
	fn audible_json_reads_the_nested_store_shape() {
		let marks = parse_audible_json(
			r#"{
				"content_metadata": {
					"chapter_info": {
						"runtime_length_ms": 47476093,
						"chapters": [
							{ "title": "Opening Credits", "start_offset_ms": 0, "length_ms": 22007 },
							{ "title": "Chapter 1", "start_offset_ms": 22007, "length_ms": 1200000 }
						]
					}
				}
			}"#,
		);

		assert_eq!(
			marks,
			vec![mark("Opening Credits", 0), mark("Chapter 1", 22_007)]
		);
	}

	/// A downloader that kept only the chapter list writes it at the root.
	#[test]
	fn audible_json_reads_the_hoisted_shape() {
		let marks = parse_audible_json(
			r#"{ "chapters": [ { "title": "Part One", "start_offset_sec": 12 } ] }"#,
		);

		assert_eq!(marks, vec![mark("Part One", 12_000)]);
	}

	/// A part's sub-chapters are real marks; a flat list is what a player
	/// shows.
	#[test]
	fn audible_json_flattens_part_chapters() {
		let marks = parse_audible_json(
			r#"{
				"chapter_info": {
					"chapters": [
						{ "title": "Part One", "start_offset_ms": 0, "chapters": [
							{ "title": "Chapter 1", "start_offset_ms": 5000 },
							{ "title": "Chapter 2", "start_offset_ms": 9000 }
						] }
					]
				}
			}"#,
		);

		assert_eq!(
			marks,
			vec![
				mark("Part One", 0),
				mark("Chapter 1", 5000),
				mark("Chapter 2", 9000),
			]
		);
	}

	#[test]
	fn malformed_json_is_no_chapter_list_rather_than_an_error() {
		assert!(parse_audible_json("not json at all").is_empty());
		assert!(parse_audible_json(r#"{"chapters": []}"#).is_empty());
	}

	/// Two names for one moment is one mark: the second is an unreachable row
	/// in every player.
	#[test]
	fn duplicate_starts_collapse_and_order_is_ascending() {
		let marks =
			parse_chapters_txt("00:00:10 Second\n00:00:00 First\n00:00:10 Duplicate\n");

		assert_eq!(marks, vec![mark("First", 0), mark("Second", 10_000)]);
	}

	/// Audible's millisecond offsets outrank a hand-typed `chapters.txt` in
	/// the same folder.
	#[test]
	fn discover_prefers_the_higher_fidelity_sidecar() {
		let dir = TempDir::new().expect("temp dir");
		fs::write(dir.path().join("chapters.txt"), "00:00:00 Typed\n")
			.expect("write chapters.txt");
		fs::write(
			dir.path().join("metadata.json"),
			r#"{"chapters":[{"title":"Mastered","start_offset_ms":0}]}"#,
		)
		.expect("write metadata.json");

		let found = discover(dir.path()).expect("a sidecar beside the book");
		assert_eq!(found.kind, SidecarKind::AudibleJson);
		assert_eq!(found.marks, vec![mark("Mastered", 0)]);
	}

	/// A sidecar named after a single container is found beside it.
	#[test]
	fn discover_matches_a_container_by_stem() {
		let dir = TempDir::new().expect("temp dir");
		let book = dir.path().join("Berserk.m4b");
		fs::write(&book, b"not really an m4b").expect("write book");
		fs::write(
			dir.path().join("Berserk.chapters.txt"),
			"00:00:00 One\n00:01:00 Two\n",
		)
		.expect("write sidecar");

		let found = discover(&book).expect("a sidecar beside the container");
		assert_eq!(found.kind, SidecarKind::ChaptersTxt);
		assert_eq!(found.marks.len(), 2);
	}

	/// A `chapters.txt` that parses to nothing must not shadow a cue sheet
	/// that parses fine.
	#[test]
	fn discover_falls_through_an_unparseable_sidecar() {
		let dir = TempDir::new().expect("temp dir");
		fs::write(dir.path().join("chapters.txt"), "just some prose\n")
			.expect("write chapters.txt");
		fs::write(
			dir.path().join("album.cue"),
			"TRACK 01 AUDIO\n  TITLE \"From The Cue\"\n  INDEX 01 00:00:00\n",
		)
		.expect("write cue");

		let found = discover(dir.path()).expect("the cue sheet");
		assert_eq!(found.kind, SidecarKind::Cue);
		assert_eq!(found.marks, vec![mark("From The Cue", 0)]);
	}

	#[test]
	fn discover_returns_none_when_there_is_nothing_beside_the_book() {
		let dir = TempDir::new().expect("temp dir");
		assert!(discover(dir.path()).is_none());
	}
}
