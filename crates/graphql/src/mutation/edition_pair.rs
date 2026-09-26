//! Confirming, refusing, and correcting an edition pair.
//!
//! Confirming is the only place a chapter map is built, because the map is
//! the one artefact tier 1 stores and an operator has to be able to edit it
//! afterwards — no heuristic gets every book right, and a book with a wrong
//! map resumes the listener in the wrong chapter.
//!
//! Pairing decisions are per user (the link row is), so confirming needs no
//! library permission: it is a statement about the caller's own library view.
//! The chapter map is *not* per user — it is a fact about two files — so
//! editing it does.

use async_graphql::{Context, Object, Result, ID};
use models::{
	domain::edition_pair::{PairOutcome, PairStatus, PairUnchanged},
	shared::enums::UserPermission,
};
use stump_library::editions;

use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	object::edition_pair::{ChapterMapEntry, EditionPairResult},
};

#[derive(Default)]
pub struct EditionPairMutation;

#[Object]
impl EditionPairMutation {
	/// Confirm that two books are editions of the same work, and build their
	/// chapter map.
	///
	/// Works with or without a prior suggestion: an operator who knows the
	/// pair can create it directly, which also mints the work row when
	/// neither book has one. Re-confirming is a no-op; two existing work
	/// identities are refused rather than silently merged or re-homed.
	async fn confirm_edition_pair(
		&self,
		ctx: &Context<'_>,
		media_id_a: ID,
		media_id_b: ID,
	) -> Result<EditionPairResult> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let (audio_id, ebook_id) =
			resolve_pair_sides(conn, media_id_a.as_str(), media_id_b.as_str()).await?;

		// Build the two chapter lists before confirming: reading the EPUB
		// spine can fail (a DRM'd or truncated file), and failing after the
		// link is written would leave a confirmed pair with no map and no
		// error anybody saw.
		let chapters = match (&audio_id, &ebook_id) {
			(Some(audio_id), Some(ebook_id)) => {
				let ebook_path = media_path(conn, ebook_id).await?;
				let ebook = tokio::task::spawn_blocking(move || {
					editions::ebook_map_chapters(&ebook_path)
				})
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))??;
				let audio = editions::audio_map_chapters(conn, audio_id).await?;
				Some((ebook, audio))
			},
			// Two books of the same kind can still be editions of one work
			// (two translations, a reflowable and a fixed-layout EPUB);
			// there is simply no time↔text conversion to map.
			_ => None,
		};

		let outcome = editions::confirm_edition_pair(
			conn,
			&user.id,
			media_id_a.as_str(),
			media_id_b.as_str(),
			chapters
				.as_ref()
				.map(|(ebook, audio)| (ebook.as_slice(), audio.as_slice())),
		)
		.await?;

		let entries = match (&outcome, &audio_id, &ebook_id) {
			(PairOutcome::Written { .. }, Some(audio_id), Some(ebook_id)) => {
				editions::chapter_map(conn, ebook_id, audio_id).await?.len()
			},
			_ => 0,
		};

		Ok(result(outcome, i32::try_from(entries).unwrap_or(i32::MAX)))
	}

	/// Refuse a suggested pair. The refusal is a row, not a deletion:
	/// suggestions are recomputed on every book-page query, so a forgotten
	/// refusal comes straight back.
	async fn reject_edition_pair(
		&self,
		ctx: &Context<'_>,
		media_id_a: ID,
		media_id_b: ID,
	) -> Result<EditionPairResult> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let outcome = editions::reject_edition_pair(
			conn,
			&user.id,
			media_id_a.as_str(),
			media_id_b.as_str(),
		)
		.await?;
		Ok(result(outcome, 0))
	}

	/// Point one ebook spine item at a different audio chapter, or add an
	/// entry the heuristics did not produce. A hand-set entry has confidence
	/// `1.0` unless one is given: a person looked at it.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn set_chapter_map_entry(
		&self,
		ctx: &Context<'_>,
		ebook_media_id: ID,
		audio_media_id: ID,
		ebook_spine_index: i32,
		audio_chapter_index: i32,
		confidence: Option<f64>,
	) -> Result<ChapterMapEntry> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let model = editions::set_chapter_map_entry(
			conn,
			ebook_media_id.as_str(),
			audio_media_id.as_str(),
			ebook_spine_index,
			audio_chapter_index,
			confidence,
		)
		.await?;
		Ok(ChapterMapEntry::from(model))
	}

	/// Drop one entry, for a spine item that turns out to have no
	/// counterpart. Returns `false` when there was nothing to drop.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn clear_chapter_map_entry(
		&self,
		ctx: &Context<'_>,
		ebook_media_id: ID,
		audio_media_id: ID,
		ebook_spine_index: i32,
	) -> Result<bool> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		Ok(editions::clear_chapter_map_entry(
			conn,
			ebook_media_id.as_str(),
			audio_media_id.as_str(),
			ebook_spine_index,
		)
		.await?)
	}
}

/// Which of the two is the audio edition and which the text one. Either may be
/// `None`: a pair of two ebooks has no time side.
async fn resolve_pair_sides(
	conn: &sea_orm::DatabaseConnection,
	media_id_a: &str,
	media_id_b: &str,
) -> Result<(Option<String>, Option<String>)> {
	use models::entity::media_audio;
	use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

	let audio_ids: Vec<String> = media_audio::Entity::find()
		.filter(
			media_audio::Column::MediaId
				.is_in([media_id_a.to_owned(), media_id_b.to_owned()]),
		)
		.all(conn)
		.await?
		.into_iter()
		.map(|audio| audio.media_id)
		.collect();

	let audio = audio_ids.first().cloned();
	let ebook = [media_id_a, media_id_b]
		.into_iter()
		.find(|id| !audio_ids.iter().any(|audio_id| audio_id == id))
		.map(str::to_owned);
	Ok((audio, ebook))
}

async fn media_path(
	conn: &sea_orm::DatabaseConnection,
	media_id: &str,
) -> Result<String> {
	use models::entity::media;
	use sea_orm::EntityTrait;

	Ok(media::Entity::find_by_id(media_id.to_owned())
		.one(conn)
		.await?
		.ok_or("Media not found")?
		.path)
}

fn result(outcome: PairOutcome, chapter_map_entries: i32) -> EditionPairResult {
	match outcome {
		PairOutcome::Written { work_id, status } => EditionPairResult {
			work_id: Some(ID::from(work_id)),
			status: Some(status),
			changed: true,
			message: None,
			chapter_map_entries,
		},
		PairOutcome::Unchanged(PairUnchanged::Rejected { work_id }) => {
			EditionPairResult {
				work_id: Some(ID::from(work_id)),
				status: Some(PairStatus::Rejected),
				changed: false,
				message: Some(
					"This pair was rejected; pairing does not re-suggest it".into(),
				),
				chapter_map_entries,
			}
		},
		PairOutcome::Unchanged(PairUnchanged::AlreadySet { work_id, status }) => {
			EditionPairResult {
				work_id: Some(ID::from(work_id)),
				status: Some(status),
				changed: false,
				message: None,
				chapter_map_entries,
			}
		},
		PairOutcome::Unchanged(PairUnchanged::WorkConflict { media_id, work_id }) => {
			EditionPairResult {
				work_id: None,
				status: None,
				changed: false,
				message: Some(format!(
					"{media_id} resolves to work {work_id}; pairing cannot merge work identities"
				)),
				chapter_map_entries: 0,
			}
		},
		PairOutcome::Unchanged(PairUnchanged::NoLink) => EditionPairResult {
			work_id: None,
			status: None,
			changed: false,
			message: Some("There was no suggestion to change".into()),
			chapter_map_entries: 0,
		},
	}
}
