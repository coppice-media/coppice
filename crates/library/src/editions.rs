//! Which audiobook is which ebook, and how their chapters line up.
//!
//! Tier 1 of read-aloud: no compute, no worker, no storage beyond a chapter
//! map. See `docs/content/docs/developer/read-aloud.mdx` sections 2 and 3.
//!
//! ## Pairing
//!
//! Neither ISBN nor ASIN is a safe key on its own — print, ebook and audio
//! editions carry different ISBNs, and an audiobook usually carries an Audible
//! ASIN and no ISBN — so the key is the **work**, and three rules find it, in
//! descending order of trust:
//!
//! 1. Both media rows already link to the same work
//!    ([`models::domain::edition_pair`], the liseur-sync model). Nothing to
//!    compute.
//! 2. An identifier of one matches an identifier of the other *through a
//!    provider's edition list*: Audnexus carries the print edition's ISBN on
//!    an audiobook record, and Open Library expands one ISBN into every
//!    edition of its work. This is the rule that pairs a translated or
//!    retitled edition, where no title comparison would.
//! 3. Normalised `title + first author` match. The weakest rule, and the
//!    reason confirmation exists: two different books share a title far more
//!    often than they share an identifier.
//!
//! Rules 2 and 3 produce **suggestions**, cached as `suggested` link rows so
//! the console can offer them and so a rejection sticks. Only a `confirmed`
//! link is an edition.
//! A media row already linked to a different work is not re-homed by a
//! suggestion or confirmation: each work owns its Liseur history, so a
//! cross-work identity merge must be explicit and is refused here.
//!
//! ## Chapter map
//!
//! Both sides already have chapters: the EPUB's linear spine (with the TOC
//! labels the Readium positions generator resolves) and the audiobook's
//! `media_audio_chapters`. [`build_chapter_map`] pairs them by normalised
//! title, then by the ordinal inside the title, then positionally, and skips
//! front and back matter rather than forcing it onto a counterpart it does
//! not have. The map is computed when a pair is confirmed and is editable
//! afterwards, because no heuristic gets every book right.

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::Utc;
use metadata_integrations::editions::{EditionIdentifier, EditionLookup};
use models::{
	domain::edition_pair::{self, PairEvidence, PairOutcome, PairStatus, PairUnchanged},
	entity::{
		media, media_audio, media_audio_chapter, media_chapter_map, media_metadata,
		metadata_provider_config, user::AuthUser,
	},
};
use sea_orm::{
	prelude::*,
	sea_query::{Alias, ExprTrait, Func, OnConflict, SimpleExpr},
	ActiveValue::Set,
	ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
	QuerySelect,
};
use stump_media::{media::readium::ReadiumManifestGenerator, FileError};
use uuid::Uuid;

/// Candidate books considered for a title-based suggestion. A book-page query
/// must stay a book-page query: the cap is what keeps "The Complete Works"
/// from scanning a whole library.
const MAX_TITLE_CANDIDATES: u64 = 64;

/// Candidates whose own identifiers are expanded through the providers to
/// upgrade a title match into an identifier match. Each expansion is an
/// outbound request against a one-per-second (Open Library) or
/// sixty-per-minute (Audnexus) budget.
const MAX_CANDIDATE_EXPANSIONS: usize = 4;

/// Confidence of a chapter-map entry matched by normalised title.
const CONFIDENCE_TITLE: f64 = 1.0;
/// Matched by the ordinal inside the title ("Chapter 12" ↔ "12").
const CONFIDENCE_ORDINAL: f64 = 0.85;
/// Matched positionally with the same number of chapters left on both sides.
const CONFIDENCE_POSITIONAL: f64 = 0.5;
/// Matched positionally with the counts disagreeing, so at least one entry is
/// certainly wrong. Kept, and visibly weak, so the console can show what to
/// fix instead of silently dropping half the book.
const CONFIDENCE_POSITIONAL_UNEVEN: f64 = 0.3;

/// Titles that name front or back matter rather than a chapter. Compared
/// against the normalised title, so "Copyright Page" and "copyright page"
/// are one entry.
const MATTER_TITLES: &[&str] = &[
	"cover",
	"title page",
	"titlepage",
	"half title",
	"copyright",
	"copyright page",
	"colophon",
	"dedication",
	"epigraph",
	"contents",
	"table of contents",
	"toc",
	"nav",
	"navigation",
	"acknowledgements",
	"acknowledgments",
	"credits",
	"opening credits",
	"end credits",
	"index",
	"bibliography",
	"imprint",
	"advertisement",
	"back cover",
	"frontmatter",
	"backmatter",
];

/// Prefixes that name front or back matter whose title carries a name
/// ("About the Author", "Also by Shirley Jackson").
const MATTER_PREFIXES: &[&str] = &[
	"about the",
	"also by",
	"praise for",
	"more from",
	"excerpt from",
];

/// One edition of the anchor book's work, as the book page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditionLink {
	pub media_id: String,
	pub work_id: String,
	pub status: PairStatus,
	/// `None` only for a link written by the liseur lane before pairing
	/// existed.
	pub evidence: Option<PairEvidence>,
}

/// One entry of a chapter map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChapterMapEntry {
	pub ebook_spine_index: i32,
	pub audio_chapter_index: i32,
	pub confidence: f64,
}

/// One chapter of either edition, reduced to what matching needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapChapter {
	/// 0-based spine index or `media_audio_chapters.index`.
	pub index: i32,
	pub title: Option<String>,
}

/// Strip a title to its comparable core: lower-case, no punctuation, no
/// leading article, no subtitle after a colon, single-spaced.
///
/// This is what makes "The Lottery", "the lottery", "Lottery, The" and
/// "The Lottery: A Story" one title, and it is deliberately not fuzzy: a
/// similarity score would pair "The Hobbit" with "The Hobbit: Illustrated
/// Edition" *and* with "The Hobbits", and only one of those is right.
#[must_use]
pub fn normalize_title(title: &str) -> String {
	let title = title.split(':').next().unwrap_or(title);
	let mut words: Vec<String> = title
		.split_whitespace()
		.map(|word| {
			word.chars()
				.filter(|c| c.is_alphanumeric() || c.is_whitespace())
				.flat_map(char::to_lowercase)
				.collect::<String>()
		})
		.filter(|word| !word.is_empty())
		.collect();

	// "Lottery, The" is a sort title; the trailing article is noise either
	// way, and so is a leading one.
	if let Some(last) = words.last() {
		if is_article(last) && words.len() > 1 {
			words.pop();
		}
	}
	if words.len() > 1 && is_article(&words[0]) {
		words.remove(0);
	}
	words.join(" ")
}

fn is_article(word: &str) -> bool {
	matches!(word, "the" | "a" | "an")
}

/// An author's comparable name tokens, lower-cased and stripped of
/// punctuation.
#[must_use]
pub fn author_tokens(author: &str) -> BTreeSet<String> {
	author
		.split_whitespace()
		.map(|word| {
			word.chars()
				.filter(|c| c.is_alphanumeric())
				.flat_map(char::to_lowercase)
				.collect::<String>()
		})
		.filter(|word| !word.is_empty())
		.collect()
}

/// Whether two credit strings name the same author.
///
/// Compared as token sets with subset tolerance, because a library holds the
/// same person under several spellings: an audiobook tagged `Jackson` and an
/// ebook tagged `Shirley Jackson` are one author, and so are `Shirley
/// Jackson` and `Jackson, Shirley` (whose first list entry is `Jackson`).
/// `Shirley Jackson` and `Peter Jackson` share only a surname and neither set
/// contains the other, so they disagree — which is the case exact equality
/// gets right and pure surname matching gets wrong.
#[must_use]
pub fn authors_agree(left: &str, right: &str) -> bool {
	let left = author_tokens(left);
	let right = author_tokens(right);
	if left.is_empty() || right.is_empty() {
		return false;
	}
	left.is_subset(&right) || right.is_subset(&left)
}

/// The first credited author of a comma-joined `media_metadata` credit list.
#[must_use]
pub fn first_author(writers: Option<&str>) -> Option<String> {
	writers
		.and_then(|writers| writers.split(',').next())
		.map(str::trim)
		.filter(|author| !author.is_empty())
		.map(str::to_owned)
}

/// Whether a chapter title names front or back matter.
#[must_use]
pub fn is_matter(title: &str) -> bool {
	let normalized = normalize_matter_title(title);
	if normalized.is_empty() {
		return false;
	}
	MATTER_TITLES.contains(&normalized.as_str())
		|| MATTER_PREFIXES
			.iter()
			.any(|prefix| normalized.starts_with(prefix))
}

/// Matter titles are compared without the article-stripping
/// [`normalize_title`] does, because "The End" and "End" are different things
/// and "the" is part of "about the author".
fn normalize_matter_title(title: &str) -> String {
	title
		.split_whitespace()
		.map(|word| {
			word.chars()
				.filter(|c| c.is_alphanumeric())
				.flat_map(char::to_lowercase)
				.collect::<String>()
		})
		.filter(|word| !word.is_empty())
		.collect::<Vec<_>>()
		.join(" ")
}

/// The number inside a chapter title: `Chapter 12` → 12, `12` → 12,
/// `12. The Lottery` → 12. `None` when the title has no leading ordinal,
/// which is what keeps "Part 2 of 3" style prose from matching a chapter.
#[must_use]
pub fn title_ordinal(title: &str) -> Option<u32> {
	let cleaned = normalize_matter_title(title);
	let mut words = cleaned.split_whitespace();
	let first = words.next()?;
	if let Ok(number) = first.parse::<u32>() {
		return Some(number);
	}
	// `chapter`/`ch`/`track`/`part` followed by the number.
	matches!(first, "chapter" | "ch" | "track" | "part" | "book")
		.then(|| words.next().and_then(|word| word.parse::<u32>().ok()))
		.flatten()
}

/// Pair an ebook's spine against an audiobook's chapter marks.
///
/// Front and back matter is dropped from both sides first, so an "Opening
/// Credits" track never becomes the copyright page and a dedication never
/// eats chapter 1. What is left is matched by normalised title, then by the
/// ordinal inside the title, then positionally — each pass only over what the
/// earlier passes left unclaimed, so a strong match is never displaced by a
/// weak one.
#[must_use]
pub fn build_chapter_map(
	ebook: &[MapChapter],
	audio: &[MapChapter],
) -> Vec<ChapterMapEntry> {
	let mut ebook_left: Vec<&MapChapter> = ebook
		.iter()
		.filter(|chapter| !chapter_is_matter(chapter))
		.collect();
	let mut audio_left: Vec<&MapChapter> = audio
		.iter()
		.filter(|chapter| !chapter_is_matter(chapter))
		.collect();
	let mut entries: Vec<ChapterMapEntry> = Vec::new();

	claim(
		&mut ebook_left,
		&mut audio_left,
		&mut entries,
		CONFIDENCE_TITLE,
		|chapter| {
			chapter
				.title
				.as_deref()
				.map(normalize_title)
				.filter(|title| !title.is_empty())
		},
	);

	claim(
		&mut ebook_left,
		&mut audio_left,
		&mut entries,
		CONFIDENCE_ORDINAL,
		|chapter| {
			chapter
				.title
				.as_deref()
				.and_then(title_ordinal)
				.map(|ordinal| ordinal.to_string())
		},
	);

	// Whatever is left, in order. Equal counts means the two editions simply
	// title their chapters differently; unequal counts means at least one
	// entry is wrong, and the confidence says so.
	let confidence = if ebook_left.len() == audio_left.len() {
		CONFIDENCE_POSITIONAL
	} else {
		CONFIDENCE_POSITIONAL_UNEVEN
	};
	for (ebook_chapter, audio_chapter) in ebook_left.iter().zip(audio_left.iter()) {
		entries.push(ChapterMapEntry {
			ebook_spine_index: ebook_chapter.index,
			audio_chapter_index: audio_chapter.index,
			confidence,
		});
	}

	entries.sort_by_key(|entry| entry.ebook_spine_index);
	entries
}

fn chapter_is_matter(chapter: &MapChapter) -> bool {
	chapter.title.as_deref().is_some_and(is_matter)
}

/// Match one pass' worth of chapters by a key, removing what it claims.
fn claim<'a, K>(
	ebook_left: &mut Vec<&'a MapChapter>,
	audio_left: &mut Vec<&'a MapChapter>,
	entries: &mut Vec<ChapterMapEntry>,
	confidence: f64,
	key: K,
) where
	K: Fn(&MapChapter) -> Option<String>,
{
	let mut audio_by_key: HashMap<String, i32> = HashMap::new();
	let mut ambiguous: HashSet<String> = HashSet::new();
	for chapter in audio_left.iter() {
		if let Some(chapter_key) = key(chapter) {
			if audio_by_key
				.insert(chapter_key.clone(), chapter.index)
				.is_some()
			{
				// Two audio chapters with the same key cannot both be the
				// match; guessing one is worse than falling through to the
				// next pass.
				ambiguous.insert(chapter_key);
			}
		}
	}

	let mut claimed_audio: HashSet<i32> = HashSet::new();
	let mut claimed_ebook: HashSet<i32> = HashSet::new();
	for chapter in ebook_left.iter() {
		let Some(chapter_key) = key(chapter) else {
			continue;
		};
		if ambiguous.contains(&chapter_key) {
			continue;
		}
		let Some(audio_index) = audio_by_key.get(&chapter_key).copied() else {
			continue;
		};
		if !claimed_audio.insert(audio_index) {
			continue;
		}
		claimed_ebook.insert(chapter.index);
		entries.push(ChapterMapEntry {
			ebook_spine_index: chapter.index,
			audio_chapter_index: audio_index,
			confidence,
		});
	}

	ebook_left.retain(|chapter| !claimed_ebook.contains(&chapter.index));
	audio_left.retain(|chapter| !claimed_audio.contains(&chapter.index));
}

/// The editions of this book's work the user should see: confirmed editions
/// first, then suggestions worth confirming.
///
/// Computed on demand — there is no pairing job — and cached as link rows, so
/// a second call is a single indexed read and a rejection is never
/// re-suggested. Provider lookups are best-effort: an upstream that is down
/// or rate-limited costs the *evidence* of a suggestion, never the page.
pub async fn pair_editions<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	media_id: &str,
	lookups: &[Box<dyn EditionLookup + Send + Sync>],
) -> Result<Vec<EditionLink>, DbErr> {
	let Some(anchor) =
		media::ModelWithMetadata::find_by_id_for_user(media_id.to_owned(), user)
			.into_model::<media::ModelWithMetadata>()
			.one(conn)
			.await?
	else {
		return Ok(Vec::new());
	};

	let anchor_is_audio = is_audio(conn, media_id).await?;
	let already_decided = edition_pair::decided_media_ids(conn, &user.id, media_id)
		.await?
		.into_iter()
		.collect::<HashSet<_>>();

	for candidate in candidates(
		conn,
		user,
		&anchor,
		anchor_is_audio,
		&already_decided,
		lookups,
	)
	.await?
	{
		let outcome = edition_pair::suggest_pair(
			conn,
			&user.id,
			media_id,
			&candidate.media_id,
			candidate.evidence,
		)
		.await?;
		if let PairOutcome::Unchanged(PairUnchanged::WorkConflict {
			media_id: conflicting,
			work_id,
		}) = &outcome
		{
			tracing::debug!(
				%conflicting,
				%work_id,
				"Edition suggestion skipped: media already belongs to another work"
			);
		}
	}

	// The pair's status, not the candidate's link status: a pair is only as
	// confirmed as its weaker half, so a book just suggested into a work sees
	// the whole work as suggestions rather than as settled editions.
	let anchor_status = edition_pair::link_for_media(conn, &user.id, media_id)
		.await?
		.map_or(PairStatus::Confirmed, |link| {
			PairStatus::from_stored(&link.pair_status)
		});
	let links = edition_pair::linked_media(conn, &user.id, media_id, None).await?;
	let mut editions: Vec<EditionLink> = links
		.into_iter()
		.map(|link| EditionLink {
			media_id: link.media_id,
			work_id: link.work_id,
			status: edition_pair::pair_status(
				anchor_status,
				PairStatus::from_stored(&link.pair_status),
			),
			evidence: link
				.pair_evidence
				.as_deref()
				.and_then(|value| value.parse().ok()),
		})
		.collect();

	// Confirmed editions are what a reader acts on; suggestions are what an
	// operator reviews. Within a group, the stronger evidence first.
	editions.sort_by(|a, b| {
		status_rank(a.status)
			.cmp(&status_rank(b.status))
			.then_with(|| evidence_rank(b.evidence).cmp(&evidence_rank(a.evidence)))
			.then_with(|| a.media_id.cmp(&b.media_id))
	});
	Ok(editions)
}

fn status_rank(status: PairStatus) -> u8 {
	match status {
		PairStatus::Confirmed => 0,
		PairStatus::Suggested => 1,
		PairStatus::Rejected => 2,
	}
}

fn evidence_rank(evidence: Option<PairEvidence>) -> u8 {
	evidence.map_or(0, PairEvidence::rank)
}

struct Candidate {
	media_id: String,
	evidence: PairEvidence,
}

/// Whether a media row is an audio publication. The `media_audio` row is the
/// only reliable tell: a folder book has no audio extension of its own, and a
/// `.m4b` that failed to probe is not a playable publication.
async fn is_audio<C: ConnectionTrait>(conn: &C, media_id: &str) -> Result<bool, DbErr> {
	Ok(media_audio::Entity::find()
		.filter(media_audio::Column::MediaId.eq(media_id))
		.one(conn)
		.await?
		.is_some())
}

/// Rules 2 and 3: identifier candidates through the providers' edition lists,
/// then title candidates. Rule 1 needs no computation — it is already a link
/// row.
async fn candidates<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	anchor: &media::ModelWithMetadata,
	anchor_is_audio: bool,
	already_decided: &HashSet<String>,
	lookups: &[Box<dyn EditionLookup + Send + Sync>],
) -> Result<Vec<Candidate>, DbErr> {
	let anchor_identifiers = identifiers_of(anchor.metadata.as_ref());
	let expanded = expand(&anchor_identifiers, lookups).await;

	let mut found: HashMap<String, PairEvidence> = HashMap::new();

	for candidate in
		identifier_candidates(conn, user, &anchor.media.id, &expanded).await?
	{
		if already_decided.contains(&candidate) {
			continue;
		}
		found.insert(candidate, PairEvidence::ProviderEditionList);
	}

	let title = anchor
		.metadata
		.as_ref()
		.and_then(|metadata| metadata.title.clone())
		.unwrap_or_else(|| anchor.media.name.clone());
	let normalized_title = normalize_title(&title);
	if normalized_title.is_empty() {
		return Ok(into_candidates(found));
	}
	let anchor_author = anchor
		.metadata
		.as_ref()
		.and_then(|metadata| first_author(metadata.writers.as_deref()));

	let mut expansions = 0usize;
	for candidate in title_candidates(
		conn,
		user,
		&anchor.media.id,
		anchor_is_audio,
		&normalized_title,
	)
	.await?
	{
		if already_decided.contains(&candidate.media.id)
			|| found.contains_key(&candidate.media.id)
		{
			continue;
		}
		let candidate_title = candidate
			.metadata
			.as_ref()
			.and_then(|metadata| metadata.title.clone())
			.unwrap_or_else(|| candidate.media.name.clone());
		if normalize_title(&candidate_title) != normalized_title {
			continue;
		}
		// An author on both sides must agree. An author missing on either
		// side is not a disagreement — a stripped audiobook tag is common,
		// and the suggestion still needs confirmation.
		let candidate_author = candidate
			.metadata
			.as_ref()
			.and_then(|metadata| first_author(metadata.writers.as_deref()));
		if let (Some(left), Some(right)) = (&anchor_author, &candidate_author) {
			if !authors_agree(left, right) {
				continue;
			}
		}

		// A title match is worth upgrading: if this candidate's own
		// identifiers expand into the anchor's set, the pair rests on an
		// identifier rather than on a shared title.
		let mut evidence = PairEvidence::TitleAuthor;
		let candidate_identifiers = identifiers_of(candidate.metadata.as_ref());
		if !candidate_identifiers.is_empty() && expansions < MAX_CANDIDATE_EXPANSIONS {
			expansions += 1;
			let candidate_expanded = expand(&candidate_identifiers, lookups).await;
			if candidate_expanded.intersection(&expanded).next().is_some() {
				evidence = PairEvidence::ProviderEditionList;
			}
		}
		found.insert(candidate.media.id.clone(), evidence);
	}

	Ok(into_candidates(found))
}

fn into_candidates(found: HashMap<String, PairEvidence>) -> Vec<Candidate> {
	let mut candidates: Vec<Candidate> = found
		.into_iter()
		.map(|(media_id, evidence)| Candidate { media_id, evidence })
		.collect();
	candidates.sort_by(|a, b| a.media_id.cmp(&b.media_id));
	candidates
}

/// The identifiers a `media_metadata` row carries, normalised.
///
/// `identifier_amazon` and `identifier_mobi_asin` are both ASIN carriers —
/// the first is what a scraped record writes, the second what a MOBI/AZW file
/// carries — and both are read, because a library holds books from both.
#[must_use]
pub fn identifiers_of(
	metadata: Option<&media_metadata::Model>,
) -> BTreeSet<EditionIdentifier> {
	let Some(metadata) = metadata else {
		return BTreeSet::new();
	};
	[
		metadata
			.identifier_isbn
			.as_deref()
			.map(|isbn| EditionIdentifier::Isbn(isbn.to_owned())),
		metadata
			.identifier_amazon
			.as_deref()
			.map(|asin| EditionIdentifier::Asin(asin.to_owned())),
		metadata
			.identifier_mobi_asin
			.as_deref()
			.map(|asin| EditionIdentifier::Asin(asin.to_owned())),
	]
	.into_iter()
	.flatten()
	.filter_map(|identifier| identifier.normalized())
	.collect()
}

/// The identifier set plus everything the providers say names the same work.
async fn expand(
	identifiers: &BTreeSet<EditionIdentifier>,
	lookups: &[Box<dyn EditionLookup + Send + Sync>],
) -> BTreeSet<EditionIdentifier> {
	let mut expanded = identifiers.clone();
	for identifier in identifiers {
		for lookup in lookups {
			match lookup.sibling_identifiers(identifier).await {
				Ok(siblings) => expanded.extend(siblings),
				Err(error) => tracing::warn!(
					provider = lookup.provider_id(),
					?error,
					"Edition list lookup failed; pairing continues without it"
				),
			}
		}
	}
	expanded
}

/// Books carrying one of the expanded identifiers. No title comparison, which
/// is the whole point: this is what pairs a retitled or translated edition.
///
/// The stored value is normalised in SQL rather than compared verbatim,
/// because an ISBN arrives hyphenated as often as not (`978-0-374-52953-6`
/// from an OPF, `9780374529536` from Open Library) and the hyphenation is the
/// publisher's choice, not part of the number. That costs the index on
/// `identifier_isbn` — a function on a column cannot use one — but this runs
/// once per book page, and an unhyphenated-only comparison would silently
/// pair nothing on a library imported from calibre.
async fn identifier_candidates<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	anchor_id: &str,
	expanded: &BTreeSet<EditionIdentifier>,
) -> Result<Vec<String>, DbErr> {
	let mut isbns: Vec<String> = Vec::new();
	let mut asins: Vec<String> = Vec::new();
	for identifier in expanded {
		match identifier {
			EditionIdentifier::Isbn(isbn) => isbns.push(isbn.clone()),
			EditionIdentifier::Asin(asin) => asins.push(asin.clone()),
			// A work key names no edition and no media row carries it.
			EditionIdentifier::OpenLibraryWork(_) => {},
		}
	}
	if isbns.is_empty() && asins.is_empty() {
		return Ok(Vec::new());
	}

	let mut identifier_match = Condition::any();
	if !isbns.is_empty() {
		identifier_match = identifier_match.add(stored_isbn().is_in(isbns));
	}
	if !asins.is_empty() {
		identifier_match = identifier_match
			.add(
				stored_identifier(media_metadata::Column::IdentifierAmazon)
					.is_in(asins.clone()),
			)
			.add(
				stored_identifier(media_metadata::Column::IdentifierMobiAsin)
					.is_in(asins),
			);
	}

	media::Entity::find_for_user(user)
		.select_only()
		.column(media::Column::Id)
		.filter(media::Column::Id.ne(anchor_id))
		.filter(media::Column::DeletedAt.is_null())
		.filter(identifier_match)
		.limit(MAX_TITLE_CANDIDATES)
		.into_tuple()
		.all(conn)
		.await
}

/// `UPPER(column)`, the form [`EditionIdentifier::normalized`] produces for an
/// ASIN or an Open Library key.
fn stored_identifier(column: media_metadata::Column) -> SimpleExpr {
	Func::upper(Expr::col((media_metadata::Entity, column))).into()
}

/// `UPPER(REPLACE(REPLACE(identifier_isbn, '-', ''), ' ', ''))`: the stored
/// ISBN reduced to the digits-and-`X` form
/// [`EditionIdentifier::normalized`] produces. SQLite has `REPLACE` and
/// `UPPER` natively, so this needs no extension and no migration.
fn stored_isbn() -> SimpleExpr {
	let strip = |value: SimpleExpr, separator: &str| -> SimpleExpr {
		Func::cust(Alias::new("REPLACE"))
			.args([value, Expr::val(separator).into(), Expr::val("").into()])
			.into()
	};
	let column: SimpleExpr = Expr::col((
		media_metadata::Entity,
		media_metadata::Column::IdentifierIsbn,
	))
	.into();
	Func::upper(strip(strip(column, "-"), " ")).into()
}

/// Books of the complementary kind whose title *might* normalise to the
/// anchor's.
///
/// The SQL filter is a cheap `LIKE` on the longest word of the normalised
/// title, because normalisation (articles, punctuation, subtitles) cannot be
/// expressed in the index; the exact comparison happens in Rust over the rows
/// this returns. Restricting to the other kind is what keeps the candidate set
/// small and is also the only pairing that means anything: two EPUBs of one
/// work have nothing to synchronise.
async fn title_candidates<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	anchor_id: &str,
	anchor_is_audio: bool,
	normalized_title: &str,
) -> Result<Vec<media::ModelWithMetadata>, DbErr> {
	let Some(anchor_word) = normalized_title
		.split_whitespace()
		.max_by_key(|word| word.len())
	else {
		return Ok(Vec::new());
	};

	let audio_ids = media_audio::Entity::find()
		.select_only()
		.column(media_audio::Column::MediaId)
		.into_tuple::<String>()
		.all(conn)
		.await?;

	let kind_filter = if anchor_is_audio {
		// The complement of an audiobook is a text edition; an audio row
		// would be a second recording, which tier 1 has nothing to say about.
		media::Column::Id.is_not_in(audio_ids)
	} else {
		media::Column::Id.is_in(audio_ids)
	};

	let like = format!("%{}%", anchor_word.replace('%', "\\%").replace('_', "\\_"));
	media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::Id.ne(anchor_id))
		.filter(media::Column::DeletedAt.is_null())
		.filter(kind_filter)
		.filter(
			Condition::any()
				.add(media_metadata::Column::Title.like(like.clone()))
				.add(media::Column::Name.like(like)),
		)
		.order_by_asc(media::Column::Id)
		.limit(MAX_TITLE_CANDIDATES)
		.into_model::<media::ModelWithMetadata>()
		.all(conn)
		.await
}

/// Confirm a pair and compute its chapter map.
///
/// The map is built here rather than lazily at read time because it is the
/// only artefact tier 1 stores, and because a map an operator can edit has to
/// exist as rows first. `chapters` are the two chapter lists as the callers
/// see them: the EPUB's spine (with TOC labels) and `media_audio_chapters`.
pub async fn confirm_edition_pair<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	anchor_media_id: &str,
	other_media_id: &str,
	chapters: Option<(&[MapChapter], &[MapChapter])>,
) -> Result<PairOutcome, DbErr> {
	let outcome =
		edition_pair::confirm_pair(conn, user_id, anchor_media_id, other_media_id)
			.await?;

	if let (PairOutcome::Written { .. }, Some((ebook, audio))) = (&outcome, chapters) {
		let (ebook_media_id, audio_media_id) = if is_audio(conn, anchor_media_id).await? {
			(other_media_id, anchor_media_id)
		} else {
			(anchor_media_id, other_media_id)
		};
		let entries = build_chapter_map(ebook, audio);
		replace_chapter_map(conn, ebook_media_id, audio_media_id, &entries).await?;
	}

	Ok(outcome)
}

/// Refuse a suggestion. The rejection is a row, not a deletion: the suggestion
/// is recomputed on every book-page query.
pub async fn reject_edition_pair<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_a: &str,
	media_b: &str,
) -> Result<PairOutcome, DbErr> {
	edition_pair::reject_pair(conn, user_id, media_a, media_b).await
}

/// The stored chapter map of a pair, in spine order.
pub async fn chapter_map<C: ConnectionTrait>(
	conn: &C,
	ebook_media_id: &str,
	audio_media_id: &str,
) -> Result<Vec<media_chapter_map::Model>, DbErr> {
	media_chapter_map::Entity::find()
		.filter(media_chapter_map::Column::EbookMediaId.eq(ebook_media_id))
		.filter(media_chapter_map::Column::AudioMediaId.eq(audio_media_id))
		.order_by_asc(media_chapter_map::Column::EbookSpineIndex)
		.all(conn)
		.await
}

/// Replace a pair's whole map. Recomputing is idempotent through the unique
/// index on `(ebook, audio, spine index)`, and entries the new map does not
/// mention are removed rather than left behind as stale rows.
pub async fn replace_chapter_map<C: ConnectionTrait>(
	conn: &C,
	ebook_media_id: &str,
	audio_media_id: &str,
	entries: &[ChapterMapEntry],
) -> Result<(), DbErr> {
	media_chapter_map::Entity::delete_many()
		.filter(media_chapter_map::Column::EbookMediaId.eq(ebook_media_id))
		.filter(media_chapter_map::Column::AudioMediaId.eq(audio_media_id))
		.exec(conn)
		.await?;
	if entries.is_empty() {
		return Ok(());
	}

	let now = Utc::now().to_rfc3339();
	let rows = entries.iter().map(|entry| media_chapter_map::ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		ebook_media_id: Set(ebook_media_id.to_owned()),
		audio_media_id: Set(audio_media_id.to_owned()),
		ebook_spine_index: Set(entry.ebook_spine_index),
		audio_chapter_index: Set(entry.audio_chapter_index),
		confidence: Set(entry.confidence),
		created_at: Set(now.clone()),
	});
	media_chapter_map::Entity::insert_many(rows)
		.exec(conn)
		.await?;
	Ok(())
}

/// Edit one entry of a map, which is how an operator fixes what the heuristics
/// got wrong. A hand-set entry is `1.0`: a person looked at it.
pub async fn set_chapter_map_entry<C: ConnectionTrait>(
	conn: &C,
	ebook_media_id: &str,
	audio_media_id: &str,
	ebook_spine_index: i32,
	audio_chapter_index: i32,
	confidence: Option<f64>,
) -> Result<media_chapter_map::Model, DbErr> {
	media_chapter_map::Entity::insert(media_chapter_map::ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		ebook_media_id: Set(ebook_media_id.to_owned()),
		audio_media_id: Set(audio_media_id.to_owned()),
		ebook_spine_index: Set(ebook_spine_index),
		audio_chapter_index: Set(audio_chapter_index),
		confidence: Set(confidence.unwrap_or(1.0).clamp(0.0, 1.0)),
		created_at: Set(Utc::now().to_rfc3339()),
	})
	.on_conflict(
		OnConflict::columns([
			media_chapter_map::Column::EbookMediaId,
			media_chapter_map::Column::AudioMediaId,
			media_chapter_map::Column::EbookSpineIndex,
		])
		.update_columns([
			media_chapter_map::Column::AudioChapterIndex,
			media_chapter_map::Column::Confidence,
		])
		.to_owned(),
	)
	.exec_with_returning(conn)
	.await
}

/// Remove one entry, for a spine item that has no counterpart after all.
pub async fn clear_chapter_map_entry<C: ConnectionTrait>(
	conn: &C,
	ebook_media_id: &str,
	audio_media_id: &str,
	ebook_spine_index: i32,
) -> Result<bool, DbErr> {
	let result = media_chapter_map::Entity::delete_many()
		.filter(media_chapter_map::Column::EbookMediaId.eq(ebook_media_id))
		.filter(media_chapter_map::Column::AudioMediaId.eq(audio_media_id))
		.filter(media_chapter_map::Column::EbookSpineIndex.eq(ebook_spine_index))
		.exec(conn)
		.await?;
	Ok(result.rows_affected > 0)
}

/// The audio chapter spans of a publication, with the last chapter's open end
/// resolved against the publication duration.
pub async fn audio_chapter_spans<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
) -> Result<Vec<models::domain::reading_state::AudioChapterSpan>, DbErr> {
	let duration_ms = media_audio::Entity::find()
		.filter(media_audio::Column::MediaId.eq(media_id))
		.one(conn)
		.await?
		.map(|audio| audio.duration_ms)
		.unwrap_or_default();

	let chapters = media_audio_chapter::Entity::find()
		.filter(media_audio_chapter::Column::MediaId.eq(media_id))
		.order_by_asc(media_audio_chapter::Column::Index)
		.all(conn)
		.await?;

	Ok(chapters
		.iter()
		.enumerate()
		.map(|(position, chapter)| {
			let next_start = chapters
				.get(position + 1)
				.map(|next| next.start_ms)
				.unwrap_or(duration_ms);
			models::domain::reading_state::AudioChapterSpan {
				index: chapter.index,
				start_ms: chapter.start_ms,
				end_ms: chapter.end_ms.unwrap_or(next_start).max(chapter.start_ms),
			}
		})
		.collect())
}

/// The audio chapters of a publication as matchable chapters.
pub async fn audio_map_chapters<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
) -> Result<Vec<MapChapter>, DbErr> {
	Ok(media_audio_chapter::Entity::find()
		.filter(media_audio_chapter::Column::MediaId.eq(media_id))
		.order_by_asc(media_audio_chapter::Column::Index)
		.all(conn)
		.await?
		.into_iter()
		.map(|chapter| MapChapter {
			index: chapter.index,
			title: chapter.title,
		})
		.collect())
}

/// The linear spine of an EPUB, with the character weights the Readium
/// positions generator uses.
///
/// This is the one place the conversion touches a file. It is the positions
/// generator's own enumeration, so a mapped locator's `total_progression`
/// agrees with `positions.json` instead of being a second, subtly different
/// notion of "how far through the book".
pub fn ebook_spine(
	path: &str,
) -> Result<Vec<models::domain::reading_state::SpineItem>, FileError> {
	Ok(ReadiumManifestGenerator::new(path, "")
		.enumerate_spine_for_positions()?
		.into_iter()
		.map(|item| models::domain::reading_state::SpineItem {
			index: i32::try_from(item.spine_index).unwrap_or(i32::MAX),
			href: item.package_path,
			char_count: i64::try_from(item.size).unwrap_or(i64::MAX),
		})
		.collect())
}

/// The spine of an EPUB as matchable chapters, labelled from the TOC.
pub fn ebook_map_chapters(path: &str) -> Result<Vec<MapChapter>, FileError> {
	Ok(ReadiumManifestGenerator::new(path, "")
		.enumerate_spine_for_positions()?
		.into_iter()
		.map(|item| MapChapter {
			index: i32::try_from(item.spine_index).unwrap_or(i32::MAX),
			title: item.title,
		})
		.collect())
}

/// The edition lookups the operator has enabled.
///
/// Both providers with an edition graph (Audnexus through `AUDIBLE`, and
/// `OPEN_LIBRARY`) are keyless, so this needs neither the token cache nor the
/// encryption key: an enabled row is enough. A provider the operator has not
/// enabled is not consulted, which is also how pairing stays offline on a
/// server with no providers configured.
pub async fn enabled_edition_lookups<C: ConnectionTrait>(
	conn: &C,
) -> Result<Vec<Box<dyn EditionLookup + Send + Sync>>, DbErr> {
	Ok(metadata_provider_config::Entity::find()
		.filter(metadata_provider_config::Column::Enabled.eq(true))
		.all(conn)
		.await?
		.into_iter()
		.filter_map(|config| {
			metadata_integrations::editions::create_edition_lookup(
				&config.provider_type.to_string(),
			)
		})
		.collect())
}

#[cfg(test)]
mod tests;
