//! Which media rows are editions of the same work, and how sure we are.
//!
//! There is no pair table. A pair is two [`liseur_sync_media_link`] rows for
//! one user naming the same `work_id`, which is the representation the
//! liseur-sync lane already writes (`POST /v1/works/resolve`); the unique index
//! on `(user_id, media_id)` is what makes "a media row belongs to one work"
//! true rather than aspirational.
//!
//! What this module adds is the difference between a *guess* and a *fact*.
//! Pairing heuristics run on demand — every book-page query recomputes them —
//! so their verdicts must be cached, and a verdict the user rejected must stay
//! rejected or it comes straight back on the next query. Hence
//! [`PairStatus`] on the link row, and [`PairEvidence`] so the console can say
//! *why* and so a weak signal never silently replaces a strong one.
//!
//! The decision half (which candidates to even consider, and the provider
//! calls that expand an identifier into an edition list) lives in
//! `stump_library::editions`; only the persistence and the state machine are
//! here, because `stump_ingest` needs to record a pair when two files of one
//! archive drop commit and it cannot depend on `stump_library`.

use chrono::Utc;
use sea_orm::{prelude::*, ActiveValue::Set, ConnectionTrait, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use uuid::Uuid;

use crate::entity::{
	liseur_sync_media_link as media_link, liseur_sync_work as work, media_metadata,
};

/// Whether a link is an asserted edition of the work, a guess, or a guess the
/// user refused.
#[derive(
	Eq,
	Copy,
	Hash,
	Debug,
	Clone,
	Default,
	PartialEq,
	Serialize,
	Deserialize,
	EnumString,
	Display,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PairStatus {
	/// An edition of the work. The schema default, so every link the liseur
	/// lane wrote — a client asserting a work for a file whose edition digest
	/// it computed — keeps meaning what it always did. This is what
	/// `Media.editions` returns.
	#[default]
	Confirmed,
	/// A heuristic match awaiting confirmation. Never presented as an edition.
	Suggested,
	/// The user said no. Kept as a row rather than deleted, because the
	/// suggestion is recomputed on every book-page query.
	Rejected,
}

impl PairStatus {
	/// Parse a stored value. An unrecognised string reads as
	/// [`PairStatus::Confirmed`]: that is the column default, so a row whose
	/// value this build does not know is a row some other writer asserted,
	/// and treating an assertion as a guess would silently unpair books.
	#[must_use]
	pub fn from_stored(value: &str) -> Self {
		value.parse().unwrap_or(Self::Confirmed)
	}
}

/// Why pairing believes two media rows are the same work.
///
/// Ordered by strength through [`PairEvidence::rank`], which is what keeps a
/// title guess from overwriting an identifier match on the same link.
#[derive(
	Eq, Copy, Hash, Debug, Clone, PartialEq, Serialize, Deserialize, EnumString, Display,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PairEvidence {
	/// An operator paired them by hand.
	Manual,
	/// Both media rows already linked to the same work.
	WorkId,
	/// An identifier of one matched an identifier of the other through a
	/// provider's edition list (Audnexus `/books/{asin}`, Open Library
	/// works→editions).
	ProviderEditionList,
	/// Both files arrived in one ingest drop group.
	SameDrop,
	/// Normalised `title + first author` matched. The weakest signal, and the
	/// reason confirmation exists at all.
	TitleAuthor,
}

impl PairEvidence {
	/// Higher wins. `Manual` outranks everything because a person looked at
	/// both books; `TitleAuthor` is last because two different books share a
	/// title more often than they share an identifier.
	#[must_use]
	pub fn rank(self) -> u8 {
		match self {
			Self::Manual => 4,
			Self::WorkId => 3,
			Self::ProviderEditionList => 2,
			Self::SameDrop => 1,
			Self::TitleAuthor => 0,
		}
	}

	fn from_stored(value: &str) -> Option<Self> {
		value.parse().ok()
	}
}

/// What a pairing write did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairOutcome {
	/// Both media rows now link to `work_id` at `status`.
	Written { work_id: String, status: PairStatus },
	/// Nothing was written.
	Unchanged(PairUnchanged),
}

/// Why a pairing write was a no-op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairUnchanged {
	/// A rejection is sticky.
	Rejected { work_id: String },
	/// Already in the requested state.
	AlreadySet { work_id: String, status: PairStatus },
	/// One side is a confirmed edition of a *different* work. Pairing never
	/// re-homes a confirmed link: the liseur lane owns those, and moving one
	/// would silently repoint annotations and sessions at another work.
	WorkConflict { media_id: String, work_id: String },
	/// Neither side has a link to change.
	NoLink,
}

impl PairOutcome {
	/// The work the two media rows share after the write, when they do.
	#[must_use]
	pub fn work_id(&self) -> Option<&str> {
		match self {
			Self::Written { work_id, .. }
			| Self::Unchanged(
				PairUnchanged::Rejected { work_id }
				| PairUnchanged::AlreadySet { work_id, .. },
			) => Some(work_id),
			_ => None,
		}
	}
}

/// Record a heuristic pair, or refresh the evidence of one already recorded.
///
/// Idempotent: re-running with the same arguments is a no-op, a confirmed link
/// is never downgraded to a suggestion, and a rejected pair stays rejected.
/// `anchor_media_id` is the book whose page (or whose ingest commit) triggered
/// the pairing; its work wins when both sides already have one.
pub async fn suggest_pair<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	anchor_media_id: &str,
	other_media_id: &str,
	evidence: PairEvidence,
) -> Result<PairOutcome, DbErr> {
	write_pair(
		conn,
		user_id,
		anchor_media_id,
		other_media_id,
		PairStatus::Suggested,
		evidence,
	)
	.await
}

/// Promote a pair to a confirmed one, creating the work and the links when the
/// operator paired two books that no heuristic had suggested.
pub async fn confirm_pair<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	anchor_media_id: &str,
	other_media_id: &str,
) -> Result<PairOutcome, DbErr> {
	write_pair(
		conn,
		user_id,
		anchor_media_id,
		other_media_id,
		PairStatus::Confirmed,
		PairEvidence::Manual,
	)
	.await
}

/// Refuse a pair.
///
/// Only a `suggested` link is rejected: a confirmed link is an assertion this
/// module did not make (a liseur client resolved the work from the file, or an
/// operator confirmed it), and unlinking it is the liseur lane's business, not
/// a side effect of dismissing a suggestion.
pub async fn reject_pair<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_a: &str,
	media_b: &str,
) -> Result<PairOutcome, DbErr> {
	let links = [
		link_for_media(conn, user_id, media_a).await?,
		link_for_media(conn, user_id, media_b).await?,
	];
	let mut work_id = None;
	for link in links.into_iter().flatten() {
		if PairStatus::from_stored(&link.pair_status) != PairStatus::Suggested {
			continue;
		}
		work_id = Some(link.work_id.clone());
		let mut model = media_link::ActiveModel::from(link);
		model.pair_status = Set(PairStatus::Rejected.to_string());
		model.update(conn).await?;
	}

	Ok(match work_id {
		Some(work_id) => PairOutcome::Written {
			work_id,
			status: PairStatus::Rejected,
		},
		None => PairOutcome::Unchanged(PairUnchanged::NoLink),
	})
}

async fn write_pair<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	anchor_media_id: &str,
	other_media_id: &str,
	status: PairStatus,
	evidence: PairEvidence,
) -> Result<PairOutcome, DbErr> {
	let anchor = link_for_media(conn, user_id, anchor_media_id).await?;
	let other = link_for_media(conn, user_id, other_media_id).await?;

	// The work the pair will share. An existing link decides it (the anchor's
	// first), so pairing joins the work the rest of the tree already knows
	// instead of minting a rival one.
	let work_id = match (anchor.as_ref(), other.as_ref()) {
		(Some(link), _) | (None, Some(link)) => link.work_id.clone(),
		(None, None) => {
			create_work(conn, user_id, anchor_media_id, other_media_id).await?
		},
	};

	// A rejection is sticky *for the work it was made about*: the suggestion
	// is recomputed on every book-page query, so a deleted or ignored
	// rejection comes straight back. A rejection about a different work is
	// not a verdict on this pair.
	for link in [anchor.as_ref(), other.as_ref()].into_iter().flatten() {
		if link.work_id == work_id
			&& PairStatus::from_stored(&link.pair_status) == PairStatus::Rejected
		{
			return Ok(PairOutcome::Unchanged(PairUnchanged::Rejected {
				work_id: link.work_id.clone(),
			}));
		}
	}

	// A confirmed link to some *other* work is not ours to move.
	for link in [anchor.as_ref(), other.as_ref()].into_iter().flatten() {
		if link.work_id != work_id
			&& PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
		{
			return Ok(PairOutcome::Unchanged(PairUnchanged::WorkConflict {
				media_id: link.media_id.clone(),
				work_id: link.work_id.clone(),
			}));
		}
	}

	let mut wrote = false;
	for (media_id, link) in
		[(anchor_media_id, anchor), (other_media_id, other)].into_iter()
	{
		wrote |= upsert_link(conn, user_id, media_id, &work_id, link, status, evidence)
			.await?;
	}

	Ok(if wrote {
		PairOutcome::Written { work_id, status }
	} else {
		PairOutcome::Unchanged(PairUnchanged::AlreadySet { work_id, status })
	})
}

/// Write one side of a pair. Returns whether the row changed.
async fn upsert_link<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
	work_id: &str,
	existing: Option<media_link::Model>,
	status: PairStatus,
	evidence: PairEvidence,
) -> Result<bool, DbErr> {
	let Some(existing) = existing else {
		media_link::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			user_id: Set(user_id.to_owned()),
			media_id: Set(media_id.to_owned()),
			work_id: Set(work_id.to_owned()),
			// Pairing identifies editions by metadata, never by content
			// digest; an empty digest is the honest value and is what the
			// liseur lane already writes for a media row it could not hash.
			edition_sha: Set(String::new()),
			resolution_status: Set("unverified".to_owned()),
			created_at: Set(Utc::now().to_rfc3339()),
			pair_status: Set(status.to_string()),
			pair_evidence: Set(Some(evidence.to_string())),
		}
		.insert(conn)
		.await?;
		return Ok(true);
	};

	let current_status = PairStatus::from_stored(&existing.pair_status);
	let current_evidence = existing
		.pair_evidence
		.as_deref()
		.and_then(PairEvidence::from_stored);

	// Never downgrade: a confirmed edition does not become a suggestion
	// because a heuristic re-ran, and stronger evidence is not replaced by
	// weaker evidence for the same verdict.
	let promote = status == PairStatus::Confirmed
		&& current_status != PairStatus::Confirmed
		|| current_status == PairStatus::Suggested && status == PairStatus::Suggested;
	let next_status = if promote { status } else { current_status };
	let next_evidence = match current_evidence {
		Some(current) if current.rank() >= evidence.rank() => current,
		_ => evidence,
	};
	let same_work = existing.work_id == work_id;

	if same_work
		&& next_status == current_status
		&& Some(next_evidence) == current_evidence
	{
		return Ok(false);
	}

	let mut model = media_link::ActiveModel::from(existing);
	model.work_id = Set(work_id.to_owned());
	model.pair_status = Set(next_status.to_string());
	model.pair_evidence = Set(Some(next_evidence.to_string()));
	model.update(conn).await?;
	Ok(true)
}

/// Mint a work for a pair neither side belongs to yet, titled from the
/// anchor's metadata so the console and the liseur lane show something
/// recognisable rather than a bare uuid.
async fn create_work<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	anchor_media_id: &str,
	fallback_media_id: &str,
) -> Result<String, DbErr> {
	let mut title = String::new();
	let mut author = String::new();
	for media_id in [anchor_media_id, fallback_media_id] {
		let metadata = media_metadata::Entity::find()
			.filter(media_metadata::Column::MediaId.eq(media_id))
			.one(conn)
			.await?;
		if let Some(metadata) = metadata {
			if title.is_empty() {
				title = metadata.title.unwrap_or_default();
			}
			if author.is_empty() {
				author = metadata
					.writers
					.and_then(|writers| {
						writers
							.split(',')
							.next()
							.map(|first| first.trim().to_owned())
					})
					.unwrap_or_default();
			}
		}
		if !title.is_empty() && !author.is_empty() {
			break;
		}
	}

	let id = Uuid::new_v4().to_string();
	work::ActiveModel {
		id: Set(id.clone()),
		user_id: Set(user_id.to_owned()),
		title: Set(title),
		author: Set(author),
		// Not a client assertion waiting to be reconciled: it was derived
		// from metadata this server already holds.
		pending: Set(false),
		created_at: Set(Utc::now().to_rfc3339()),
	}
	.insert(conn)
	.await?;
	Ok(id)
}

/// The work link of one media row, if it has one.
pub async fn link_for_media<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
) -> Result<Option<media_link::Model>, DbErr> {
	media_link::Entity::find()
		.filter(media_link::Column::UserId.eq(user_id))
		.filter(media_link::Column::MediaId.eq(media_id))
		.one(conn)
		.await
}

/// The other media rows sharing this media row's work, filtered by the
/// **pair's** status.
///
/// A pair is only as confirmed as its weaker half: if the anchor's own link is
/// a suggestion, every other member of the work is a suggestion *to the
/// anchor* even where that member's link is confirmed in its own right. That
/// is what lets a newly suggested book see the pair it was suggested into,
/// and what stops `Media.editions` from presenting an unconfirmed guess as an
/// edition.
///
/// `None` for `status` is every state except [`PairStatus::Rejected`], which
/// is never surfaced.
pub async fn linked_media<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
	status: Option<PairStatus>,
) -> Result<Vec<media_link::Model>, DbErr> {
	let Some(link) = link_for_media(conn, user_id, media_id).await? else {
		return Ok(Vec::new());
	};
	let anchor_status = PairStatus::from_stored(&link.pair_status);
	if anchor_status == PairStatus::Rejected {
		return Ok(Vec::new());
	}

	let links = media_link::Entity::find()
		.filter(media_link::Column::UserId.eq(user_id))
		.filter(media_link::Column::WorkId.eq(link.work_id.clone()))
		.filter(media_link::Column::MediaId.ne(media_id))
		.order_by_asc(media_link::Column::CreatedAt)
		.order_by_asc(media_link::Column::Id)
		.all(conn)
		.await?;

	Ok(links
		.into_iter()
		.filter(|candidate| {
			let pair_status = pair_status(
				anchor_status,
				PairStatus::from_stored(&candidate.pair_status),
			);
			match status {
				Some(wanted) => pair_status == wanted,
				None => pair_status != PairStatus::Rejected,
			}
		})
		.collect())
}

/// The status of a pair, given the status of its two links: the weaker one.
/// `rejected` beats `suggested` beats `confirmed`, because a pair a user
/// refused is refused from both sides and a pair only one side has confirmed
/// is still a guess.
#[must_use]
pub fn pair_status(left: PairStatus, right: PairStatus) -> PairStatus {
	match (left, right) {
		(PairStatus::Rejected, _) | (_, PairStatus::Rejected) => PairStatus::Rejected,
		(PairStatus::Suggested, _) | (_, PairStatus::Suggested) => PairStatus::Suggested,
		_ => PairStatus::Confirmed,
	}
}

/// Media ids the user has already decided about, in either direction, so the
/// candidate search can skip them.
pub async fn decided_media_ids<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
) -> Result<Vec<String>, DbErr> {
	let Some(link) = link_for_media(conn, user_id, media_id).await? else {
		return Ok(Vec::new());
	};

	media_link::Entity::find()
		.select_only()
		.column(media_link::Column::MediaId)
		.filter(media_link::Column::UserId.eq(user_id))
		.filter(media_link::Column::WorkId.eq(link.work_id))
		.into_tuple()
		.all(conn)
		.await
}
