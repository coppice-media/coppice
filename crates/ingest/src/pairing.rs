//! The one thing ingest knows about editions: two files in one drop are two
//! editions of one book.
//!
//! A librarian who downloads "Dark Eden" gets `Audiobook.rar` and `Ebooks.rar`
//! in one folder; a `.rar` of a book usually holds its EPUB *and* its MOBI.
//! Nothing later in the pipeline can recover that: once both are library rows,
//! only their titles relate them, and a title match is the weakest edition
//! signal there is ([`PairEvidence::TitleAuthor`]). At commit time the drop
//! group still says it, so that is where it is recorded — as
//! [`PairEvidence::SameDrop`], which ranks above a title guess and below an
//! identifier match.
//!
//! Two rules make this safe to run on every commit:
//!
//! * **Suggested, never confirmed.** `suggest_pair` writes
//!   [`PairStatus::Suggested`], so an archive that happened to hold two
//!   unrelated books produces a suggestion a user declines, never a wrong
//!   edition list. It never downgrades a confirmed link.
//! * **Complementary kinds only.** An audiobook pairs with an ebook. Two EPUBs
//!   in one archive are usually one book in two formats *of the same edition*
//!   (`.epub` + `.mobi`), which is a format duplicate rather than an edition
//!   pair, and three volumes of a series in one archive are not editions of
//!   anything. Pairing the audio lane with the text lane is the case that is
//!   both common and unambiguous.
//!
//! The persistence lives in `models::domain::edition_pair` rather than in
//! `stump_library::editions` for a mechanical reason: `stump_library` depends
//! on `stump_core`, which depends on this crate, so a call the other way is a
//! dependency cycle.

use models::domain::edition_pair::{self, PairEvidence, PairOutcome};
use sea_orm::ConnectionTrait;

use crate::{contract::IngestMediaKind, IngestResult};

/// Whether two drop items of one group are an audio/text edition pair.
///
/// Order-independent: it answers the same for (audio, epub) and (epub, audio),
/// because a commit sees whichever of the two landed second.
pub fn is_edition_pair(left: IngestMediaKind, right: IngestMediaKind) -> bool {
	let (audio, text) = if left.is_audio() {
		(left, right)
	} else {
		(right, left)
	};
	audio.is_audio()
		&& matches!(
			text,
			// `Unknown` is the kind a MOBI/AZW3 gets: the processor reads it,
			// the page lane does not, and an audiobook beside one is exactly
			// the `Audiobook.rar` + `Ebooks.rar` case this exists for.
			IngestMediaKind::Epub
				| IngestMediaKind::Pdf
				| IngestMediaKind::ComicArchive
				| IngestMediaKind::ComicRarArchive
				| IngestMediaKind::Unknown
		)
}

/// Record that two committed media rows arrived in one drop group.
///
/// Returns the outcome so a caller can log a conflict without inspecting the
/// link rows itself. A pairing failure is never allowed to fail the commit: the
/// books are in the library either way, and a missing suggestion is a missing
/// convenience, not a lost file.
pub async fn suggest_same_drop_pair<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	anchor_media_id: &str,
	sibling_media_id: &str,
) -> IngestResult<PairOutcome> {
	Ok(edition_pair::suggest_pair(
		conn,
		user_id,
		anchor_media_id,
		sibling_media_id,
		PairEvidence::SameDrop,
	)
	.await?)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_audiobook_pairs_with_a_text_edition_in_either_order() {
		for text in [
			IngestMediaKind::Epub,
			IngestMediaKind::Pdf,
			IngestMediaKind::Unknown,
			IngestMediaKind::ComicArchive,
		] {
			assert!(
				is_edition_pair(IngestMediaKind::Audio, text),
				"audio + {text:?}"
			);
			assert!(
				is_edition_pair(text, IngestMediaKind::Audio),
				"{text:?} + audio"
			);
		}
	}

	/// Two EPUBs in one archive are a format duplicate or two different
	/// books; neither is an edition pair, and suggesting one would put a
	/// wrong edition list in front of the user on every drop of a series.
	#[test]
	fn two_text_editions_or_two_audiobooks_are_not_a_pair() {
		assert!(!is_edition_pair(
			IngestMediaKind::Epub,
			IngestMediaKind::Unknown
		));
		assert!(!is_edition_pair(
			IngestMediaKind::Epub,
			IngestMediaKind::Epub
		));
		assert!(!is_edition_pair(
			IngestMediaKind::Audio,
			IngestMediaKind::Audio
		));
	}
}
