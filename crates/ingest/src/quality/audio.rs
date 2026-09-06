//! What the seven audio quality checks share.
//!
//! A comic check reads an archive's entry list; an audio check reads a
//! *probe*. Demuxing an audiobook is the expensive part — symphonia has to
//! walk the container, and a folder book means one walk per part — so it is
//! done once here and handed to every check as a [`ProbedAudio`], rather than
//! seven times per book.
//!
//! Two rules make the family behave like the comic family:
//!
//! * **Applicability is by kind, never by guesswork.** A check reports
//!   `NOT_APPLICABLE` for anything that is not [`IngestMediaKind::Audio`], and
//!   the scorer excludes those from both sums — so a CBZ's score is untouched
//!   by the audio checks existing, and an audiobook's score is untouched by
//!   the comic checks existing.
//! * **An unreadable file is one finding, not seven.** A probe that fails
//!   makes every check `Fail` with the same evidence rather than raising, so a
//!   corrupt book still gets a report and a librarian still gets a row.
//!
//! # Why the ids are what they are
//!
//! Each one is a property a *listener* notices, and each names a tool that
//! fixes it:
//!
//! | Check | What a listener sees when it fails | Fix |
//! | --- | --- | --- |
//! | `single_file` | 43 "songs" on their phone, no book | `audio-assemble` |
//! | `chapters_present` | a time scrubber instead of a chapter list | `audio-chapters` |
//! | `faststart` | the book will not start until it has downloaded | `audio-assemble` |
//! | `tags_complete` | blank rows in every library and car stereo | `meta-edit` |
//! | `cover_embedded` | a placeholder in the grid | `meta-edit` |
//! | `duration_consistent` | a progress bar that lies | `audio-assemble` |
//! | `bitrate_sane` | speech that sounds underwater, or 4 GB of nothing | `audio-assemble` |

use std::{
	io::{Read, Seek, SeekFrom},
	path::Path,
};

use serde_json::json;
use stump_media::audio::{self, ProbedAudio};

use crate::contract::{
	BookSnapshot, IngestMediaKind, QualityCheckOutcome, QualityStatus,
};

use super::outcome;

/// The tool ids an audio finding points at. Same strings the `stump_tools`
/// registry answers to, so a fix cannot name a tool that does not exist.
pub(crate) const TOOL_ASSEMBLE: &str = "audio-assemble";
pub(crate) const TOOL_CHAPTERS: &str = "audio-chapters";
pub(crate) const TOOL_META: &str = "meta-edit";

/// What a check needs to know about the publication, or why it could not be
/// read.
pub(crate) enum AudioSubject {
	/// Not an audiobook: report `NOT_APPLICABLE`.
	NotAudio,
	/// The publication, demuxed.
	Probed(Box<ProbedAudio>),
	/// The publication could not be demuxed at all.
	Unreadable(String),
}

/// Probe the snapshot once.
pub(crate) fn subject(book: &BookSnapshot) -> AudioSubject {
	if !matches!(book.media_kind, IngestMediaKind::Audio) {
		return AudioSubject::NotAudio;
	}
	match audio::probe(&book.staged_path) {
		Ok(probed) => AudioSubject::Probed(Box::new(probed)),
		Err(error) => AudioSubject::Unreadable(error.to_string()),
	}
}

/// The `NOT_APPLICABLE` outcome every audio check gives a comic.
pub(crate) fn not_audio(check_id: &str, label: &str) -> QualityCheckOutcome {
	outcome(
		check_id,
		label,
		QualityStatus::NotApplicable,
		0.0,
		json!({ "applicable": false, "reason": "not_an_audiobook" }),
	)
}

/// The `FAIL` outcome every audio check gives a publication it cannot demux.
///
/// A failure rather than a `NOT_APPLICABLE`: the file claims to be an
/// audiobook by its extension and is not readable as one, which is exactly
/// the kind of thing a quality report exists to surface.
pub(crate) fn unreadable(
	check_id: &str,
	label: &str,
	error: &str,
) -> QualityCheckOutcome {
	outcome(
		check_id,
		label,
		QualityStatus::Fail,
		0.0,
		json!({ "readable": false, "error": error }),
	)
}

/// Whether an MP4 puts its `moov` box ahead of its media data.
///
/// `Ok(None)` for a container that is not MP4 at all: an MP3 or an Ogg has no
/// index to misplace, so the question does not apply rather than failing.
///
/// Only the top-level box order is read — four bytes of size and four of type
/// per box, seeking over each — so this costs a handful of reads even on a
/// one-gigabyte book.
pub(crate) fn moov_before_mdat(path: &Path) -> std::io::Result<Option<bool>> {
	/// `size` + `type`.
	const HEADER: u64 = 8;

	let mut file = std::fs::File::open(path)?;
	let length = file.seek(SeekFrom::End(0))?;
	let mut offset = 0u64;
	let mut moov = None;
	let mut mdat = None;
	let mut index = 0usize;
	let mut is_mp4 = false;

	while offset + HEADER <= length {
		file.seek(SeekFrom::Start(offset))?;
		let mut header = [0u8; 8];
		file.read_exact(&mut header)?;
		let declared = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
		let fourcc = [header[4], header[5], header[6], header[7]];
		let size = match declared {
			1 => {
				let mut large = [0u8; 8];
				file.read_exact(&mut large)?;
				u64::from_be_bytes(large)
			},
			0 => length - offset,
			other => u64::from(other),
		};
		if size < HEADER || offset + size > length {
			// Not a box tree, or a truncated one; either way not an MP4 whose
			// layout can be judged.
			return Ok(None);
		}

		match &fourcc {
			b"ftyp" => is_mp4 = true,
			b"moov" => moov = moov.or(Some(index)),
			b"mdat" => mdat = mdat.or(Some(index)),
			_ => {},
		}
		index += 1;
		offset += size;
	}

	if !is_mp4 {
		return Ok(None);
	}
	Ok(match (moov, mdat) {
		(Some(moov), Some(mdat)) => Some(moov < mdat),
		// An MP4 with no media data is not a book; an MP4 with no `moov` is
		// not playable. Neither is a faststart question.
		_ => None,
	})
}

/// `h:mm:ss`, for evidence a librarian reads rather than sorts.
pub(crate) fn human_duration(duration_ms: i64) -> String {
	let seconds = duration_ms.max(0) / 1_000;
	format!(
		"{}:{:02}:{:02}",
		seconds / 3_600,
		(seconds % 3_600) / 60,
		seconds % 60
	)
}
