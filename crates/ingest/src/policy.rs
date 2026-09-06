//! Komf-style per-field metadata policy: for every canonical metadata field,
//! *which* providers may fill it, in *what order*, and *how* their offers
//! combine.
//!
//! Komf keys the same information the other way round — per provider, a
//! `priority` number plus a `seriesMetadata`/`bookMetadata` map of per-field
//! booleans, overridable per library through `libraryProviders`
//! ([komf `application.yml` reference][komf-config], [aggregation
//! rules][komf-aggregate]). A per-field ordered provider list is the transpose
//! of that table and carries exactly the same facts, with one advantage: the
//! answer to "who wrote this field" is a single lookup instead of a scan over
//! every provider's toggle map.
//!
//! The module is deliberately split in two halves:
//!
//! * [`MetadataPolicy::plan`] is pure. It turns candidates + the stored row +
//!   the user's locks into the same [`FieldPick`] recipe the editor submits by
//!   hand, so auto-apply and "apply best" share one resolver and one audit
//!   shape rather than growing a second, subtly different merge.
//! * [`effective_policy`] / [`set_library_override`] are the persistence half:
//!   the server default lives in code, a library may override it per field in
//!   `library_configs.metadata_policy`, and the override always wins.
//!
//! [komf-config]: https://github.com/Snd-R/komf/blob/master/README.md#example-applicationyml-config
//! [komf-aggregate]: https://github.com/Snd-R/komf/blob/master/README.md#metadata-aggregation

use std::{
	collections::{BTreeMap, BTreeSet},
	sync::LazyLock,
};

use models::entity::{
	ingest_metadata_candidate, library, library_config, media_metadata,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
	contract::{FieldPick, MetadataField},
	providers::{
		apply::{existing_list, field_is_present, is_list_field, STORABLE_FIELDS},
		builtin_embedded::EMBEDDED_PROVIDER_ID,
		llm::LLM_PROVIDER_ID,
	},
};

/// How the offers of the allowed providers for one field combine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PolicyStrategy {
	/// The first provider in the field's list that offers a value wins. This
	/// is Komf's behaviour: "all metadata will be fetched from the first
	/// positive match in configured providers by order of priority".
	#[default]
	First,
	/// Case-insensitive union of the stored value and every allowed provider's
	/// values, first spelling kept. Only meaningful for list fields
	/// (`AUTHORS`, `TAGS`, `GENRES`); falls back to [`Self::First`] elsewhere.
	MergeUnion,
	/// The longest offer wins: character count for text, item count for lists.
	/// Ties keep the higher-priority provider.
	Longest,
	/// Keep whatever the row already holds; only fill the field when it is
	/// empty. Komf's `postProcessing.seriesTitle: false` is this rule.
	PreferExisting,
	/// The offer advertising the largest cover wins, see
	/// [`advertised_cover_width`]. Offers advertising nothing rank last, so a
	/// field where nobody advertises a size degrades to [`Self::First`].
	HighestResolution,
}

impl PolicyStrategy {
	/// Whether this strategy can be honoured for `field` at all. A stored
	/// policy that pairs them anyway is not an error at plan time — it
	/// degrades to [`Self::First`] — but [`MetadataPolicy::validate`] refuses
	/// to write the pair in the first place.
	pub fn applies_to(self, field: MetadataField) -> bool {
		match self {
			Self::MergeUnion => is_list_field(field),
			Self::HighestResolution => field == MetadataField::CoverUrl,
			Self::First | Self::Longest | Self::PreferExisting => true,
		}
	}
}

fn lock_respected_default() -> bool {
	true
}

/// One field's rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldPolicy {
	/// Provider ids in priority order. Empty means no provider may fill this
	/// field, which is how Komf's per-provider `seriesMetadata: { x: false }`
	/// toggles are expressed once transposed.
	#[serde(default)]
	pub providers: Vec<String>,
	#[serde(default)]
	pub strategy: PolicyStrategy,
	/// Honour the user's `media_metadata.locked_fields`. A locked field is
	/// never written while this is `true`, whatever the strategy says.
	#[serde(default = "lock_respected_default")]
	pub lock_respected: bool,
}

impl Default for FieldPolicy {
	fn default() -> Self {
		Self {
			providers: Vec::new(),
			strategy: PolicyStrategy::First,
			lock_respected: true,
		}
	}
}

impl FieldPolicy {
	fn new(providers: Vec<String>, strategy: PolicyStrategy) -> Self {
		Self {
			providers,
			strategy,
			lock_respected: true,
		}
	}
}

/// The audio half of a library's ingest policy: the one quality-check weight
/// that is worth arguing about, plus the auto-fix toggles.
///
/// Six of the seven audio checks (`chapters_present`, `faststart`,
/// `tags_complete`, `cover_embedded`, `duration_consistent`, `bitrate_sane`)
/// measure a fact about the file that is either true or false, and their
/// weights are compile-time constants like every comic check's. `single_file`
/// is different: whether a book split across 40 MP3s is a *defect* or simply
/// how that library stores audiobooks is a library-level opinion, so its
/// weight is the one number an operator can move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPolicy {
	/// Weight of the `single_file` quality check, on the same 0..=100 scale
	/// as every other check's [`crate::contract::QualityCheck::weight`]. `0`
	/// keeps the finding as advisory prose without moving the score, which is
	/// how a library that deliberately stores per-chapter MP3s turns the
	/// penalty off without losing the report row.
	#[serde(default = "default_single_file_weight")]
	pub single_file_weight: u16,
	/// Assemble a split audiobook into the canonical single file
	/// (`STUMP_AUDIO_CANONICAL`) as part of ingest.
	#[serde(default)]
	pub auto_assemble: bool,
	/// Write chapter marks into an audiobook that has none, derived from its
	/// per-file boundaries.
	#[serde(default)]
	pub auto_chapters: bool,
	/// Keep the source files after an assemble. Turning this off is the only
	/// way ingest ever deletes an operator's audio, so it defaults on and is
	/// meaningless — and refused — without `auto_assemble`.
	#[serde(default = "keep_original_default")]
	pub keep_original: bool,
}

/// The weight `single_file` carries unless a library says otherwise. Heavy on
/// purpose: with the other six audio checks at 10..=15 the audio family sums
/// to 100, so a split book cannot score above ~70 however clean its tags are.
fn default_single_file_weight() -> u16 {
	30
}

fn keep_original_default() -> bool {
	true
}

impl Default for AudioPolicy {
	fn default() -> Self {
		*Self::server_default()
	}
}

impl AudioPolicy {
	/// The server-wide default. Both auto-fixes are off and the sources are
	/// kept: an ingest run analyses and reports, and only rewrites or removes
	/// an operator's audio files once they have asked for it per library.
	pub fn server_default() -> &'static Self {
		static DEFAULT: AudioPolicy = AudioPolicy {
			single_file_weight: 30,
			auto_assemble: false,
			auto_chapters: false,
			keep_original: true,
		};
		&DEFAULT
	}

	/// Refuse a document that could not do what it says.
	pub fn validate(&self) -> Result<(), PolicyError> {
		if self.single_file_weight > MAX_CHECK_WEIGHT {
			return Err(PolicyError::AudioWeightOutOfRange(self.single_file_weight));
		}
		if !self.keep_original && !self.auto_assemble {
			return Err(PolicyError::AudioKeepOriginalWithoutAssemble);
		}
		Ok(())
	}
}

/// The sum of the built-in quality-check weights, and therefore the largest
/// weight one check may carry: above it, a single check outvotes every other
/// check in the registry combined and the score stops meaning anything.
pub const MAX_CHECK_WEIGHT: u16 = 100;

/// A whole policy document: the rules for the fields it mentions, plus the
/// audio section. A field with no rule is never touched by a policy run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPolicy {
	#[serde(default)]
	pub fields: BTreeMap<MetadataField, FieldPolicy>,
	/// The audio rules, when this document has an opinion about them.
	/// `None` inherits [`AudioPolicy::server_default`], which is why a
	/// document written before the audio lane existed still deserializes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub audio: Option<AudioPolicy>,
}

/// Provider ids in Komf's default priority order, restricted to the providers
/// this build can register. Komf's numbers are `mangaUpdates: 10`, `mal: 20`,
/// `aniList: 40`, `mangaDex: 90`, `comicVine: 110`; the providers Komf has no
/// equivalent for follow, comics before books, so that a build with every
/// provider configured still resolves manga exactly like Komf would.
///
/// A provider that does not support a book's media kind never produces a
/// candidate, so one total order over every provider is enough: the manga
/// providers are simply silent on an EPUB novel.
const KOMF_PRIORITY: &[&str] = &[
	"mangaupdates",
	"mal",
	"anilist",
	"mangadex",
	"comic_vine",
	"metron",
	"openlibrary",
	"googlebooks",
	"audible",
	"hardcover",
];

fn remote() -> Vec<String> {
	KOMF_PRIORITY.iter().map(|id| (*id).to_string()).collect()
}

/// Remote providers, then the file's own metadata as the fallback.
fn remote_then_embedded() -> Vec<String> {
	let mut providers = remote();
	providers.push(EMBEDDED_PROVIDER_ID.to_string());
	providers
}

/// The file's own metadata first: mechanical facts measured from the bytes
/// beat a catalog record for a different edition.
fn embedded_then_remote() -> Vec<String> {
	let mut providers = vec![EMBEDDED_PROVIDER_ID.to_string()];
	providers.extend(remote());
	providers
}

/// Descriptive prose and free-form lists may fall back to AI enrichment, last.
fn remote_then_embedded_then_llm() -> Vec<String> {
	let mut providers = remote_then_embedded();
	providers.push(LLM_PROVIDER_ID.to_string());
	providers
}

impl MetadataPolicy {
	/// The server-wide default, mirroring Komf's defaults
	/// ([reference config][komf-config]) with three documented divergences:
	///
	/// * `TAGS`/`GENRES` default to a union. Komf gates the same behaviour
	///   behind `mergeTags`/`mergeGenres`, off by default *because Komf's
	///   aggregation itself is off by default*; a per-field policy aggregates
	///   by construction, and the union dedupes case-insensitively.
	/// * `COVER_URL` picks the largest advertised cover rather than the
	///   priority winner: Stump keeps one cover per book, so a larger source
	///   image is strictly better input for thumbnail generation.
	/// * `PAGE_COUNT` allows only the embedded provider, and `SERIES_INDEX`
	///   and `ISBN` prefer it. These are measured from the file; Komf's own
	///   docs warn that a catalog provider "can mismatch issue numbers".
	///
	/// `SERIES` is [`PolicyStrategy::PreferExisting`] because Komf leaves the
	/// series title alone unless `postProcessing.seriesTitle` is turned on.
	///
	/// [komf-config]: https://github.com/Snd-R/komf/blob/master/README.md#example-applicationyml-config
	pub fn server_default() -> &'static Self {
		static DEFAULT: LazyLock<MetadataPolicy> = LazyLock::new(|| {
			let rules = [
				(
					MetadataField::Title,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::SortTitle,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::Series,
					FieldPolicy::new(
						remote_then_embedded(),
						PolicyStrategy::PreferExisting,
					),
				),
				(
					MetadataField::SeriesIndex,
					FieldPolicy::new(embedded_then_remote(), PolicyStrategy::First),
				),
				(
					MetadataField::Authors,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::Narrators,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::Publisher,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::PublishedDate,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::Language,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::Summary,
					FieldPolicy::new(
						remote_then_embedded_then_llm(),
						PolicyStrategy::First,
					),
				),
				(
					MetadataField::Tags,
					FieldPolicy::new(
						remote_then_embedded_then_llm(),
						PolicyStrategy::MergeUnion,
					),
				),
				(
					MetadataField::Genres,
					FieldPolicy::new(
						remote_then_embedded_then_llm(),
						PolicyStrategy::MergeUnion,
					),
				),
				(
					MetadataField::Isbn,
					FieldPolicy::new(embedded_then_remote(), PolicyStrategy::First),
				),
				(
					MetadataField::Identifiers,
					FieldPolicy::new(embedded_then_remote(), PolicyStrategy::First),
				),
				(
					MetadataField::AgeRating,
					FieldPolicy::new(remote_then_embedded(), PolicyStrategy::First),
				),
				(
					MetadataField::PageCount,
					FieldPolicy::new(
						vec![EMBEDDED_PROVIDER_ID.to_string()],
						PolicyStrategy::First,
					),
				),
				(
					MetadataField::CoverUrl,
					FieldPolicy::new(remote(), PolicyStrategy::HighestResolution),
				),
				(
					MetadataField::Status,
					FieldPolicy::new(remote(), PolicyStrategy::First),
				),
			];
			MetadataPolicy {
				fields: rules.into_iter().collect(),
				audio: Some(*AudioPolicy::server_default()),
			}
		});
		&DEFAULT
	}

	/// Per-field override: every field the override mentions replaces the
	/// base rule wholesale, everything else is inherited. Komf's
	/// `libraryProviders` works the same way — a library that names one
	/// provider set keeps the defaults for everything it does not mention.
	///
	/// The audio section follows the same rule at section granularity: an
	/// override that mentions `audio` replaces the whole section, one that
	/// does not keeps the base's.
	pub fn overlay(&self, over: &Self) -> Self {
		let mut fields = self.fields.clone();
		for (field, rule) in &over.fields {
			fields.insert(*field, rule.clone());
		}
		Self {
			fields,
			audio: over.audio.or(self.audio),
		}
	}

	/// This document's audio rules, falling back to the server default for a
	/// document that has no `audio` section.
	pub fn audio(&self) -> &AudioPolicy {
		self.audio
			.as_ref()
			.unwrap_or_else(|| AudioPolicy::server_default())
	}

	/// Reject a document before it is stored: an unknown provider id, a
	/// repeated provider, or a strategy that cannot apply to its field would
	/// all silently disable or degrade a field later.
	pub fn validate(&self, known_providers: &[String]) -> Result<(), PolicyError> {
		for (field, rule) in &self.fields {
			if !rule.strategy.applies_to(*field) {
				return Err(PolicyError::StrategyNotApplicable {
					field: *field,
					strategy: rule.strategy,
				});
			}
			let mut seen = BTreeSet::new();
			for provider_id in &rule.providers {
				if !known_providers.contains(provider_id) {
					return Err(PolicyError::UnknownProvider {
						field: *field,
						provider_id: provider_id.clone(),
					});
				}
				if !seen.insert(provider_id) {
					return Err(PolicyError::DuplicateProvider {
						field: *field,
						provider_id: provider_id.clone(),
					});
				}
			}
		}
		if let Some(audio) = &self.audio {
			audio.validate()?;
		}
		Ok(())
	}

	/// Resolve the policy against the candidates stored for one target.
	///
	/// This is the single resolver behind both auto-apply and the editor's
	/// "apply best": it returns the [`FieldPick`] recipe to hand to
	/// `apply::resolve_picks_for_context`, the locks it enforced (pass them
	/// straight back so the apply path enforces the same set), and one
	/// [`FieldDecision`] per field it considered so the editor can explain
	/// itself.
	pub fn plan(
		&self,
		candidates: &[PolicyCandidate],
		existing: Option<&media_metadata::Model>,
		locked: &[MetadataField],
	) -> PolicyPlan {
		let locked: BTreeSet<_> = locked.iter().copied().collect();
		let mut plan = PolicyPlan::default();
		for (field, rule) in &self.fields {
			let field = *field;
			if rule.lock_respected && locked.contains(&field) {
				plan.enforced_locks.push(field);
				plan.decisions.push(FieldDecision {
					field,
					strategy: rule.strategy,
					outcome: FieldOutcome::Locked,
					providers: Vec::new(),
					applied: false,
				});
				continue;
			}
			let offers = rule.offers(field, candidates);
			if offers.is_empty() {
				plan.decisions.push(FieldDecision {
					field,
					strategy: rule.strategy,
					outcome: FieldOutcome::Unmatched,
					providers: Vec::new(),
					applied: false,
				});
				continue;
			}
			// A field the staged apply path has no column for is still
			// resolved: the editor shows who would win, nothing is written.
			let storable = STORABLE_FIELDS.contains(&field);
			let strategy = if rule.strategy.applies_to(field) {
				rule.strategy
			} else {
				PolicyStrategy::First
			};
			match strategy {
				PolicyStrategy::PreferExisting
					if existing.is_some_and(|model| field_is_present(field, model)) =>
				{
					plan.decisions.push(FieldDecision {
						field,
						strategy: rule.strategy,
						outcome: FieldOutcome::Kept,
						providers: Vec::new(),
						applied: false,
					});
				},
				PolicyStrategy::MergeUnion => {
					let (value, providers) = union(field, &offers, existing);
					if storable {
						plan.picks.push(FieldPick::Manual { field, value });
					}
					plan.decisions.push(FieldDecision {
						field,
						strategy: rule.strategy,
						outcome: FieldOutcome::Merged,
						providers,
						applied: storable,
					});
				},
				PolicyStrategy::Longest
				| PolicyStrategy::HighestResolution
				| PolicyStrategy::First
				| PolicyStrategy::PreferExisting => {
					let winner = match strategy {
						PolicyStrategy::Longest => {
							best(&offers, |offer| value_length(offer.value))
						},
						PolicyStrategy::HighestResolution => best(&offers, |offer| {
							advertised_cover_width(offer.value).unwrap_or_default()
						}),
						_ => offers[0],
					};
					if storable {
						plan.picks.push(FieldPick::Candidate {
							field,
							candidate_id: winner.candidate_id.to_string(),
						});
					}
					plan.decisions.push(FieldDecision {
						field,
						strategy: rule.strategy,
						outcome: FieldOutcome::Candidate,
						providers: vec![winner.provider_id.to_string()],
						applied: storable,
					});
				},
			}
		}
		plan
	}
}

impl FieldPolicy {
	/// Every candidate value offered for `field`, ordered by the rule's
	/// provider priority and, within one provider, by descending candidate
	/// confidence (candidate id breaks the remaining ties so a plan is
	/// reproducible).
	fn offers<'a>(
		&self,
		field: MetadataField,
		candidates: &'a [PolicyCandidate],
	) -> Vec<Offer<'a>> {
		let mut offers = Vec::new();
		for provider_id in &self.providers {
			let mut matched = candidates
				.iter()
				.filter(|candidate| candidate.provider_id == *provider_id)
				.filter_map(|candidate| {
					let value = candidate.fields.get(&field)?;
					offered(value).then_some(Offer {
						candidate_id: &candidate.candidate_id,
						provider_id: &candidate.provider_id,
						confidence: candidate.confidence,
						value,
					})
				})
				.collect::<Vec<_>>();
			matched.sort_by(|left, right| {
				right
					.confidence
					.total_cmp(&left.confidence)
					.then_with(|| left.candidate_id.cmp(right.candidate_id))
			});
			offers.append(&mut matched);
		}
		offers
	}
}

/// One provider's value for one field, already known to be non-empty.
#[derive(Debug, Clone, Copy)]
struct Offer<'a> {
	candidate_id: &'a str,
	provider_id: &'a str,
	confidence: f64,
	value: &'a Value,
}

/// The first offer with the greatest score: `max_by_key` would return the
/// last, which would silently invert the priority order on a tie.
fn best<'a, F>(offers: &[Offer<'a>], score: F) -> Offer<'a>
where
	F: Fn(&Offer<'a>) -> u32,
{
	let mut best = offers[0];
	let mut best_score = score(&best);
	for offer in &offers[1..] {
		let current = score(offer);
		if current > best_score {
			best = *offer;
			best_score = current;
		}
	}
	best
}

/// Whether a candidate actually offers something for a field: a null, blank
/// string, empty list, or empty object is absence, not evidence.
fn offered(value: &Value) -> bool {
	match value {
		Value::Null => false,
		Value::String(value) => !value.trim().is_empty(),
		Value::Array(values) => values.iter().any(offered),
		Value::Object(values) => !values.is_empty(),
		Value::Bool(_) | Value::Number(_) => true,
	}
}

/// Comparable "size" of an offer for [`PolicyStrategy::Longest`]: characters
/// for text, non-empty items for a list, `0` for anything unordered.
fn value_length(value: &Value) -> u32 {
	match value {
		Value::String(value) => {
			value.trim().chars().count().min(u32::MAX as usize) as u32
		},
		Value::Array(values) => {
			values.iter().filter(|value| offered(value)).count() as u32
		},
		_ => 0,
	}
}

/// Case-insensitive union of the stored list and every offer, in first-seen
/// order: the value already on the row comes first, so a policy run never
/// drops what the user has, and the first spelling of a duplicate wins.
fn union(
	field: MetadataField,
	offers: &[Offer<'_>],
	existing: Option<&media_metadata::Model>,
) -> (Value, Vec<String>) {
	let mut seen: BTreeSet<String> = BTreeSet::new();
	let mut values: Vec<String> = Vec::new();
	let mut providers: Vec<String> = Vec::new();
	let mut push = |value: &str, seen: &mut BTreeSet<String>| {
		let value = value.trim();
		if value.is_empty() {
			return false;
		}
		if seen.insert(value.to_lowercase()) {
			values.push(value.to_string());
			true
		} else {
			false
		}
	};
	if let Some(stored) = existing.and_then(|model| existing_list(field, model)) {
		for value in stored {
			push(&value, &mut seen);
		}
	}
	for offer in offers {
		let mut contributed = false;
		match offer.value {
			Value::Array(items) => {
				for item in items {
					if let Some(item) = item.as_str() {
						contributed |= push(item, &mut seen);
					}
				}
			},
			Value::String(value) => contributed |= push(value, &mut seen),
			_ => {},
		}
		if contributed && !providers.iter().any(|id| id == offer.provider_id) {
			providers.push(offer.provider_id.to_string());
		}
	}
	(json!(values), providers)
}

/// Best-effort *advertised* width in pixels of a cover offer.
///
/// A provider hands out a URL, not an image, so the only size information
/// available without downloading every candidate is what the value itself
/// advertises. Recognised forms:
///
/// | Form | Example | Width |
/// | --- | --- | --- |
/// | object with a numeric `width` | `{"url":"…","width":1200}` | `1200` |
/// | `<width>x<height>` token | `…/cover_1200x1800.jpg` | `1200` |
/// | size component before the extension | MangaDex `…/abc.jpg.512.jpg` | `512` |
/// | `width=` or `w=` query parameter | `…/cover.jpg?w=800` | `800` |
///
/// `None` means the value advertises nothing, which ranks below every known
/// width; a field where no offer advertises a size therefore resolves in
/// priority order. Guessing from a provider's own size keyword (`super_url`,
/// `extraLarge`, `-L`) is deliberately not attempted: those are already each
/// provider's largest variant, so they carry no comparable pixel count.
pub fn advertised_cover_width(value: &Value) -> Option<u32> {
	if let Some(width) = value.get("width").and_then(Value::as_u64) {
		return u32::try_from(width).ok();
	}
	let url = value
		.as_str()
		.or_else(|| value.get("url").and_then(Value::as_str))?;
	dimension_token(url)
		.or_else(|| size_component(url))
		.or_else(|| query_width(url))
}

/// The 2..=5 digit run ending at the end of `text`.
fn trailing_number(text: &str) -> Option<u32> {
	let digits = text.len() - text.trim_end_matches(|c: char| c.is_ascii_digit()).len();
	(2..=5)
		.contains(&digits)
		.then(|| &text[text.len() - digits..])?
		.parse()
		.ok()
}

/// The 2..=5 digit run starting at the start of `text`.
fn leading_number(text: &str) -> Option<u32> {
	let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
	(2..=5)
		.contains(&digits)
		.then(|| &text[..digits])?
		.parse()
		.ok()
}

fn dimension_token(url: &str) -> Option<u32> {
	url.char_indices()
		.filter(|(_, character)| *character == 'x' || *character == 'X')
		.find_map(|(index, _)| {
			let width = trailing_number(&url[..index])?;
			leading_number(&url[index + 1..]).map(|_| width)
		})
}

/// The size component MangaDex appends between the file name and the
/// extension (`…/abc.jpg.512.jpg`). A plain `<name>.<ext>` has nothing
/// between the two to read.
fn size_component(url: &str) -> Option<u32> {
	let segment = url.split('?').next()?.rsplit('/').next()?;
	let components: Vec<&str> = segment.split('.').collect();
	if components.len() < 3 {
		return None;
	}
	let size = components[components.len() - 2];
	size.chars()
		.all(|character| character.is_ascii_digit())
		.then(|| leading_number(size))?
}

fn query_width(url: &str) -> Option<u32> {
	let (_, query) = url.split_once('?')?;
	query.split('&').find_map(|parameter| {
		let value = parameter
			.strip_prefix("width=")
			.or_else(|| parameter.strip_prefix("w="))?;
		leading_number(value)
	})
}

/// One candidate row reduced to what the policy needs.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyCandidate {
	pub candidate_id: String,
	pub provider_id: String,
	pub confidence: f64,
	pub fields: BTreeMap<MetadataField, Value>,
}

impl PolicyCandidate {
	/// A stored candidate row whose `fields` document parses. A row written by
	/// an older provider version that no longer deserializes is skipped rather
	/// than failing the run: candidates are evidence, and stale evidence is
	/// simply not evidence.
	pub fn from_model(model: &ingest_metadata_candidate::Model) -> Option<Self> {
		Some(Self {
			candidate_id: model.id.clone(),
			provider_id: model.provider_id.clone(),
			confidence: model.confidence,
			fields: serde_json::from_value(model.fields.clone()).ok()?,
		})
	}

	pub fn from_models(models: &[ingest_metadata_candidate::Model]) -> Vec<Self> {
		models.iter().filter_map(Self::from_model).collect()
	}
}

/// What the policy decided for one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FieldOutcome {
	/// One candidate won; the pick references it, so the audit row keeps the
	/// provenance.
	Candidate,
	/// Several providers were combined; the pick carries the merged value.
	Merged,
	/// The stored value stands (`PREFER_EXISTING` on a populated field).
	Kept,
	/// The field is locked and the rule respects locks.
	Locked,
	/// No allowed provider offered anything.
	Unmatched,
}

/// One field's resolution, for the editor to explain and for tests to pin.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecision {
	pub field: MetadataField,
	pub strategy: PolicyStrategy,
	pub outcome: FieldOutcome,
	/// Providers that contributed, in the order they were used.
	pub providers: Vec<String>,
	/// Whether this decision produced a pick. `false` for a lock, a kept
	/// value, an unmatched field, and for the candidate-only fields the
	/// staged apply path has no column for (`COVER_URL`, `IDENTIFIERS`,
	/// `STATUS`).
	pub applied: bool,
}

/// The auto-apply payload: what to apply, which locks to enforce, and why.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PolicyPlan {
	pub picks: Vec<FieldPick>,
	pub decisions: Vec<FieldDecision>,
	pub enforced_locks: Vec<MetadataField>,
}

impl PolicyPlan {
	pub fn decision(&self, field: MetadataField) -> Option<&FieldDecision> {
		self.decisions
			.iter()
			.find(|decision| decision.field == field)
	}
}

/// The fields a user has locked on a metadata row, in the ingest vocabulary.
/// Public metadata fields with no ingest equivalent are dropped: they cannot
/// be the target of a policy rule either.
pub fn locked_fields(existing: Option<&media_metadata::Model>) -> Vec<MetadataField> {
	let Some(value) = existing.and_then(|model| model.locked_fields.as_ref()) else {
		return Vec::new();
	};
	let public: Vec<metadata_integrations::MetadataField> =
		serde_json::from_value(value.clone()).unwrap_or_default();
	let mut fields: Vec<_> = public
		.into_iter()
		.filter_map(MetadataField::from_public)
		.collect();
	fields.sort_unstable();
	fields.dedup();
	fields
}

/// The effective policy for one library: the server default with the library's
/// override applied, plus the override itself so a client can tell inherited
/// rules from overridden ones.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectivePolicy {
	pub library_id: String,
	pub policy: MetadataPolicy,
	pub library_override: Option<MetadataPolicy>,
}

impl EffectivePolicy {
	/// Whether this field's rule comes from the library override.
	pub fn is_overridden(&self, field: MetadataField) -> bool {
		self.library_override
			.as_ref()
			.is_some_and(|policy| policy.fields.contains_key(&field))
	}

	/// Whether the audio section comes from the library override. Section
	/// granularity, not per key: the override stores or omits the whole
	/// `audio` object.
	pub fn audio_overridden(&self) -> bool {
		self.library_override
			.as_ref()
			.is_some_and(|policy| policy.audio.is_some())
	}
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PolicyError {
	#[error("library {0} does not exist")]
	UnknownLibrary(String),
	#[error("library {0} has no configuration row")]
	MissingLibraryConfig(String),
	#[error("stored metadata policy for library {library_id} is invalid: {message}")]
	Invalid { library_id: String, message: String },
	#[error("{provider_id} is not a known provider (field {field:?})")]
	UnknownProvider {
		field: MetadataField,
		provider_id: String,
	},
	#[error("provider {provider_id} is listed twice for field {field:?}")]
	DuplicateProvider {
		field: MetadataField,
		provider_id: String,
	},
	#[error("strategy {strategy:?} cannot be used for field {field:?}")]
	StrategyNotApplicable {
		field: MetadataField,
		strategy: PolicyStrategy,
	},
	#[error(
		"audio singleFileWeight {0} exceeds {MAX_CHECK_WEIGHT}, the sum of the \
		 built-in quality-check weights"
	)]
	AudioWeightOutOfRange(u16),
	#[error(
		"audio keepOriginal can only be turned off when autoAssemble is on: \
		 there would be nothing to replace the source files with"
	)]
	AudioKeepOriginalWithoutAssemble,
	#[error("database operation failed: {0}")]
	Database(String),
}

fn db_error(error: sea_orm::DbErr) -> PolicyError {
	PolicyError::Database(error.to_string())
}

async fn library_config_for(
	conn: &DatabaseConnection,
	library_id: &str,
) -> Result<library_config::Model, PolicyError> {
	let library = library::Entity::find_by_id(library_id)
		.one(conn)
		.await
		.map_err(db_error)?
		.ok_or_else(|| PolicyError::UnknownLibrary(library_id.to_string()))?;
	library_config::Entity::find_by_id(library.config_id)
		.one(conn)
		.await
		.map_err(db_error)?
		.ok_or_else(|| PolicyError::MissingLibraryConfig(library_id.to_string()))
}

/// The library's stored override, when it has one.
pub async fn library_override(
	conn: &DatabaseConnection,
	library_id: &str,
) -> Result<Option<MetadataPolicy>, PolicyError> {
	let config = library_config_for(conn, library_id).await?;
	let Some(document) = config.metadata_policy.as_deref() else {
		return Ok(None);
	};
	serde_json::from_str(document)
		.map(Some)
		.map_err(|error| PolicyError::Invalid {
			library_id: library_id.to_string(),
			message: error.to_string(),
		})
}

/// The policy a run for this library must obey.
pub async fn effective_policy(
	conn: &DatabaseConnection,
	library_id: &str,
) -> Result<EffectivePolicy, PolicyError> {
	let library_override = library_override(conn, library_id).await?;
	let policy = match &library_override {
		Some(over) => MetadataPolicy::server_default().overlay(over),
		None => MetadataPolicy::server_default().clone(),
	};
	Ok(EffectivePolicy {
		library_id: library_id.to_string(),
		policy,
		library_override,
	})
}

/// Store (or, with `None`, clear) the library's override and return the new
/// effective policy. `known_providers` is the registry catalog: a rule naming
/// a provider this build cannot register is refused rather than stored.
pub async fn set_library_override(
	conn: &DatabaseConnection,
	library_id: &str,
	policy: Option<&MetadataPolicy>,
	known_providers: &[String],
) -> Result<EffectivePolicy, PolicyError> {
	if let Some(policy) = policy {
		policy.validate(known_providers)?;
	}
	let document = policy
		.map(|policy| {
			serde_json::to_string(policy).map_err(|error| PolicyError::Invalid {
				library_id: library_id.to_string(),
				message: error.to_string(),
			})
		})
		.transpose()?;
	let config = library_config_for(conn, library_id).await?;
	let mut active = config.into_active_model();
	active.metadata_policy = Set(document);
	active.update(conn).await.map_err(db_error)?;
	effective_policy(conn, library_id).await
}

#[cfg(test)]
mod tests {
	use super::*;

	fn candidate(
		id: &str,
		provider_id: &str,
		confidence: f64,
		fields: &[(MetadataField, Value)],
	) -> PolicyCandidate {
		PolicyCandidate {
			candidate_id: id.to_string(),
			provider_id: provider_id.to_string(),
			confidence,
			fields: fields.iter().cloned().collect(),
		}
	}

	fn policy(rules: &[(MetadataField, &[&str], PolicyStrategy)]) -> MetadataPolicy {
		MetadataPolicy {
			fields: rules
				.iter()
				.map(|(field, providers, strategy)| {
					(
						*field,
						FieldPolicy::new(
							providers.iter().map(|id| (*id).to_string()).collect(),
							*strategy,
						),
					)
				})
				.collect(),
			audio: None,
		}
	}

	fn metadata() -> media_metadata::Model {
		media_metadata::Model {
			id: 1,
			..Default::default()
		}
	}

	#[test]
	fn priority_order_decides_the_winner() {
		let policy = policy(&[(
			MetadataField::Title,
			&["mangaupdates", "anilist"],
			PolicyStrategy::First,
		)]);
		// Deliberately the *more* confident candidate from the lower-priority
		// provider: priority is an ordering, not a score.
		let candidates = [
			candidate(
				"anilist-1",
				"anilist",
				0.99,
				&[(MetadataField::Title, json!("Berserk Deluxe"))],
			),
			candidate(
				"mu-1",
				"mangaupdates",
				0.40,
				&[(MetadataField::Title, json!("Berserk"))],
			),
		];

		let plan = policy.plan(&candidates, None, &[]);

		assert_eq!(
			plan.picks,
			vec![FieldPick::Candidate {
				field: MetadataField::Title,
				candidate_id: "mu-1".to_string(),
			}]
		);
		let decision = plan.decision(MetadataField::Title).unwrap();
		assert_eq!(decision.outcome, FieldOutcome::Candidate);
		assert_eq!(decision.providers, vec!["mangaupdates".to_string()]);
		assert!(decision.applied);
	}

	#[test]
	fn a_provider_outside_the_list_never_wins() {
		let policy = policy(&[(
			MetadataField::Title,
			&["mangaupdates"],
			PolicyStrategy::First,
		)]);
		let candidates = [candidate(
			"llm-1",
			LLM_PROVIDER_ID,
			1.0,
			&[(MetadataField::Title, json!("Hallucinated Title"))],
		)];

		let plan = policy.plan(&candidates, None, &[]);

		assert!(plan.picks.is_empty());
		assert_eq!(
			plan.decision(MetadataField::Title).unwrap().outcome,
			FieldOutcome::Unmatched
		);
	}

	#[test]
	fn union_dedupes_case_insensitively_and_keeps_stored_values() {
		let policy = policy(&[(
			MetadataField::Genres,
			&["mangaupdates", "anilist", "mangadex"],
			PolicyStrategy::MergeUnion,
		)]);
		let candidates = [
			candidate(
				"mu-1",
				"mangaupdates",
				0.9,
				&[(MetadataField::Genres, json!(["Action", "seinen", "  "]))],
			),
			candidate(
				"anilist-1",
				"anilist",
				0.8,
				&[(MetadataField::Genres, json!(["ACTION", "Adventure"]))],
			),
			candidate(
				"mangadex-1",
				"mangadex",
				0.7,
				&[(MetadataField::Genres, json!(["action", "Drama"]))],
			),
		];
		let existing = media_metadata::Model {
			genres: Some("Drama, action".to_string()),
			..metadata()
		};

		let plan = policy.plan(&candidates, Some(&existing), &[]);

		assert_eq!(
			plan.picks,
			vec![FieldPick::Manual {
				field: MetadataField::Genres,
				// Stored spellings first and kept, then each new value once:
				// "Action"/"ACTION" both fold onto the stored "action", and the
				// blank entry is not a value.
				value: json!(["Drama", "action", "seinen", "Adventure"]),
			}]
		);
		let decision = plan.decision(MetadataField::Genres).unwrap();
		assert_eq!(decision.outcome, FieldOutcome::Merged);
		// MangaDex re-offered only values the row already had, so it is not
		// listed as a contributor.
		assert_eq!(
			decision.providers,
			vec!["mangaupdates".to_string(), "anilist".to_string()]
		);
	}

	#[test]
	fn union_falls_back_to_priority_for_a_scalar_field() {
		let policy = policy(&[(
			MetadataField::Summary,
			&["mangaupdates", "anilist"],
			PolicyStrategy::MergeUnion,
		)]);
		let candidates = [
			candidate(
				"anilist-1",
				"anilist",
				0.9,
				&[(MetadataField::Summary, json!("Second"))],
			),
			candidate(
				"mu-1",
				"mangaupdates",
				0.1,
				&[(MetadataField::Summary, json!("First"))],
			),
		];

		let plan = policy.plan(&candidates, None, &[]);

		assert_eq!(
			plan.picks,
			vec![FieldPick::Candidate {
				field: MetadataField::Summary,
				candidate_id: "mu-1".to_string(),
			}]
		);
	}

	#[test]
	fn longest_beats_priority_and_ties_keep_priority() {
		let policy = policy(&[(
			MetadataField::Summary,
			&["mangaupdates", "anilist", "mangadex"],
			PolicyStrategy::Longest,
		)]);
		let candidates = [
			candidate(
				"mu-1",
				"mangaupdates",
				0.9,
				&[(MetadataField::Summary, json!("Short."))],
			),
			candidate(
				"anilist-1",
				"anilist",
				0.5,
				&[(
					MetadataField::Summary,
					json!("A considerably longer synopsis."),
				)],
			),
		];

		let plan = policy.plan(&candidates, None, &[]);
		assert_eq!(
			plan.picks,
			vec![FieldPick::Candidate {
				field: MetadataField::Summary,
				candidate_id: "anilist-1".to_string(),
			}]
		);

		// Same length: the higher-priority provider keeps the field.
		let tied = [
			candidate(
				"mu-1",
				"mangaupdates",
				0.1,
				&[(MetadataField::Summary, json!("Equal length text"))],
			),
			candidate(
				"anilist-1",
				"anilist",
				0.9,
				&[(MetadataField::Summary, json!("Equal length text"))],
			),
		];
		let plan = policy.plan(&tied, None, &[]);
		assert_eq!(
			plan.picks,
			vec![FieldPick::Candidate {
				field: MetadataField::Summary,
				candidate_id: "mu-1".to_string(),
			}]
		);
	}

	#[test]
	fn locks_are_respected_unless_the_rule_opts_out() {
		let mut policy = policy(&[
			(
				MetadataField::Title,
				&["mangaupdates"],
				PolicyStrategy::First,
			),
			(
				MetadataField::Summary,
				&["mangaupdates"],
				PolicyStrategy::First,
			),
		]);
		policy
			.fields
			.get_mut(&MetadataField::Summary)
			.unwrap()
			.lock_respected = false;
		let candidates = [candidate(
			"mu-1",
			"mangaupdates",
			0.9,
			&[
				(MetadataField::Title, json!("Berserk")),
				(MetadataField::Summary, json!("A synopsis.")),
			],
		)];
		let locked = [MetadataField::Title, MetadataField::Summary];

		let plan = policy.plan(&candidates, Some(&metadata()), &locked);

		// The locked field with `lockRespected` is neither picked nor cleared,
		// and it is reported back so the apply path enforces the same set.
		assert_eq!(plan.enforced_locks, vec![MetadataField::Title]);
		assert_eq!(
			plan.decision(MetadataField::Title).unwrap().outcome,
			FieldOutcome::Locked
		);
		assert_eq!(
			plan.picks,
			vec![FieldPick::Candidate {
				field: MetadataField::Summary,
				candidate_id: "mu-1".to_string(),
			}]
		);
	}

	#[test]
	fn locked_fields_reads_the_public_lock_list() {
		let existing = media_metadata::Model {
			// `WRITERS` and `ARTISTS` both fold onto the ingest `AUTHORS`
			// field, and `FORMAT` has no ingest equivalent at all.
			locked_fields: Some(json!(["WRITERS", "ARTISTS", "FORMAT", "COVER"])),
			..metadata()
		};

		assert_eq!(
			locked_fields(Some(&existing)),
			vec![MetadataField::Authors, MetadataField::CoverUrl]
		);
		assert!(locked_fields(None).is_empty());
	}

	#[test]
	fn prefer_existing_keeps_a_populated_field_and_fills_an_empty_one() {
		let policy = policy(&[(
			MetadataField::Series,
			&["mangaupdates"],
			PolicyStrategy::PreferExisting,
		)]);
		let candidates = [candidate(
			"mu-1",
			"mangaupdates",
			0.9,
			&[(MetadataField::Series, json!("Berserk"))],
		)];

		let populated = media_metadata::Model {
			series: Some("My Own Ordering".to_string()),
			..metadata()
		};
		let plan = policy.plan(&candidates, Some(&populated), &[]);
		assert!(plan.picks.is_empty());
		assert_eq!(
			plan.decision(MetadataField::Series).unwrap().outcome,
			FieldOutcome::Kept
		);

		let plan = policy.plan(&candidates, Some(&metadata()), &[]);
		assert_eq!(
			plan.picks,
			vec![FieldPick::Candidate {
				field: MetadataField::Series,
				candidate_id: "mu-1".to_string(),
			}]
		);
	}

	#[test]
	fn cover_picks_the_largest_advertised_resolution() {
		let policy = policy(&[(
			MetadataField::CoverUrl,
			&["mangaupdates", "anilist", "mangadex"],
			PolicyStrategy::HighestResolution,
		)]);
		let candidates = [
			candidate(
				"mu-1",
				"mangaupdates",
				0.9,
				&[(
					MetadataField::CoverUrl,
					json!("https://cdn.test/cover.jpg?w=800"),
				)],
			),
			candidate(
				"anilist-1",
				"anilist",
				0.9,
				&[(
					MetadataField::CoverUrl,
					json!("https://cdn.test/cover_1200x1800.jpg"),
				)],
			),
			candidate(
				"mangadex-1",
				"mangadex",
				0.9,
				&[(
					MetadataField::CoverUrl,
					json!("https://uploads.test/covers/abc.jpg.512.jpg"),
				)],
			),
		];

		let plan = policy.plan(&candidates, None, &[]);

		// 1200 > 800 > 512, so the second-priority provider wins the cover.
		let decision = plan.decision(MetadataField::CoverUrl).unwrap();
		assert_eq!(decision.providers, vec!["anilist".to_string()]);
		assert_eq!(decision.outcome, FieldOutcome::Candidate);
		// `COVER_URL` has no column in the staged apply path: the winner is
		// reported, nothing is written.
		assert!(!decision.applied);
		assert!(plan.picks.is_empty());
	}

	#[test]
	fn advertised_cover_width_reads_only_documented_forms() {
		let width = |value: Value| advertised_cover_width(&value);
		assert_eq!(width(json!({"url": "x", "width": 1600})), Some(1600));
		assert_eq!(width(json!("https://cdn.test/c_1200x1800.jpg")), Some(1200));
		assert_eq!(width(json!("https://cdn.test/a.jpg.512.jpg")), Some(512));
		assert_eq!(width(json!("https://cdn.test/c.jpg?w=800")), Some(800));
		assert_eq!(width(json!("https://cdn.test/c.jpg?width=640")), Some(640));
		// A provider's own size keyword is not a pixel count.
		assert_eq!(width(json!("https://cdn.test/super_url/cover.jpg")), None);
		assert_eq!(width(json!("https://covers.test/b/id/6498519-L.jpg")), None);
		assert_eq!(width(json!(42)), None);
	}

	#[test]
	fn unknown_cover_sizes_fall_back_to_priority() {
		let policy = policy(&[(
			MetadataField::CoverUrl,
			&["mangaupdates", "anilist"],
			PolicyStrategy::HighestResolution,
		)]);
		let candidates = [
			candidate(
				"anilist-1",
				"anilist",
				0.9,
				&[(MetadataField::CoverUrl, json!("https://cdn.test/a.jpg"))],
			),
			candidate(
				"mu-1",
				"mangaupdates",
				0.1,
				&[(MetadataField::CoverUrl, json!("https://cdn.test/b.jpg"))],
			),
		];

		let plan = policy.plan(&candidates, None, &[]);

		assert_eq!(
			plan.decision(MetadataField::CoverUrl).unwrap().providers,
			vec!["mangaupdates".to_string()]
		);
	}

	#[test]
	fn blank_offers_are_not_evidence() {
		let policy = policy(&[
			(
				MetadataField::Title,
				&["mangaupdates"],
				PolicyStrategy::First,
			),
			(
				MetadataField::Genres,
				&["mangaupdates"],
				PolicyStrategy::MergeUnion,
			),
		]);
		let candidates = [candidate(
			"mu-1",
			"mangaupdates",
			0.9,
			&[
				(MetadataField::Title, json!("   ")),
				(MetadataField::Genres, json!([])),
			],
		)];

		let plan = policy.plan(&candidates, None, &[]);

		assert!(plan.picks.is_empty());
		assert_eq!(
			plan.decision(MetadataField::Title).unwrap().outcome,
			FieldOutcome::Unmatched
		);
		assert_eq!(
			plan.decision(MetadataField::Genres).unwrap().outcome,
			FieldOutcome::Unmatched
		);
	}

	#[test]
	fn same_provider_offers_are_ordered_by_confidence() {
		let policy = policy(&[(
			MetadataField::Title,
			&["mangaupdates"],
			PolicyStrategy::First,
		)]);
		let candidates = [
			candidate(
				"mu-low",
				"mangaupdates",
				0.4,
				&[(MetadataField::Title, json!("Low"))],
			),
			candidate(
				"mu-high",
				"mangaupdates",
				0.8,
				&[(MetadataField::Title, json!("High"))],
			),
		];

		let plan = policy.plan(&candidates, None, &[]);

		assert_eq!(
			plan.picks,
			vec![FieldPick::Candidate {
				field: MetadataField::Title,
				candidate_id: "mu-high".to_string(),
			}]
		);
	}

	#[test]
	fn overlay_replaces_only_the_fields_the_override_names() {
		let base = policy(&[
			(
				MetadataField::Title,
				&["mangaupdates", "anilist"],
				PolicyStrategy::First,
			),
			(
				MetadataField::Summary,
				&["mangaupdates"],
				PolicyStrategy::First,
			),
		]);
		let over =
			policy(&[(MetadataField::Title, &["anilist"], PolicyStrategy::Longest)]);

		let merged = base.overlay(&over);

		assert_eq!(
			merged.fields[&MetadataField::Title],
			FieldPolicy::new(vec!["anilist".to_string()], PolicyStrategy::Longest)
		);
		assert_eq!(
			merged.fields[&MetadataField::Summary],
			base.fields[&MetadataField::Summary]
		);
	}

	#[test]
	fn server_default_mirrors_komf_ordering_and_documented_divergences() {
		let default = MetadataPolicy::server_default();

		// Komf's priority numbers, transposed: the first four providers of a
		// descriptive field are Komf's order for the providers this build has.
		let title = &default.fields[&MetadataField::Title];
		assert_eq!(
			title.providers[..4],
			[
				"mangaupdates".to_string(),
				"mal".to_string(),
				"anilist".to_string(),
				"mangadex".to_string()
			]
		);
		assert_eq!(title.strategy, PolicyStrategy::First);
		assert!(title.lock_respected);

		// Komf's postProcessing.seriesTitle defaults to false.
		assert_eq!(
			default.fields[&MetadataField::Series].strategy,
			PolicyStrategy::PreferExisting
		);
		// Komf's mergeTags/mergeGenres, on because per-field aggregation is.
		assert_eq!(
			default.fields[&MetadataField::Tags].strategy,
			PolicyStrategy::MergeUnion
		);
		assert_eq!(
			default.fields[&MetadataField::Genres].strategy,
			PolicyStrategy::MergeUnion
		);
		assert_eq!(
			default.fields[&MetadataField::CoverUrl].strategy,
			PolicyStrategy::HighestResolution
		);
		// Measured facts: only the file may set the page count, and it leads
		// for the number and the ISBN.
		assert_eq!(
			default.fields[&MetadataField::PageCount].providers,
			vec![EMBEDDED_PROVIDER_ID.to_string()]
		);
		for field in [MetadataField::SeriesIndex, MetadataField::Isbn] {
			assert_eq!(
				default.fields[&field].providers.first().map(String::as_str),
				Some(EMBEDDED_PROVIDER_ID)
			);
		}
		// AI enrichment is a last resort, and only for prose and free lists.
		for field in [
			MetadataField::Summary,
			MetadataField::Tags,
			MetadataField::Genres,
		] {
			assert_eq!(
				default.fields[&field].providers.last().map(String::as_str),
				Some(LLM_PROVIDER_ID)
			);
		}
		assert!(!default.fields[&MetadataField::Title]
			.providers
			.iter()
			.any(|id| id == LLM_PROVIDER_ID));

		// Every rule the default ships must be one the writer would accept.
		let known = known_provider_ids();
		default.validate(&known).unwrap();
	}

	fn known_provider_ids() -> Vec<String> {
		let mut ids = remote_then_embedded_then_llm();
		ids.sort();
		ids.dedup();
		ids
	}

	#[test]
	fn validate_refuses_documents_that_would_silently_degrade() {
		let known = known_provider_ids();

		let unknown = policy(&[(
			MetadataField::Title,
			&["mangaupdatez"],
			PolicyStrategy::First,
		)]);
		assert_eq!(
			unknown.validate(&known),
			Err(PolicyError::UnknownProvider {
				field: MetadataField::Title,
				provider_id: "mangaupdatez".to_string(),
			})
		);

		let duplicated = policy(&[(
			MetadataField::Title,
			&["anilist", "anilist"],
			PolicyStrategy::First,
		)]);
		assert_eq!(
			duplicated.validate(&known),
			Err(PolicyError::DuplicateProvider {
				field: MetadataField::Title,
				provider_id: "anilist".to_string(),
			})
		);

		let wrong_strategy = policy(&[(
			MetadataField::Title,
			&["anilist"],
			PolicyStrategy::HighestResolution,
		)]);
		assert_eq!(
			wrong_strategy.validate(&known),
			Err(PolicyError::StrategyNotApplicable {
				field: MetadataField::Title,
				strategy: PolicyStrategy::HighestResolution,
			})
		);
	}

	#[test]
	fn a_partial_document_deserializes_with_locks_respected() {
		let policy: MetadataPolicy =
			serde_json::from_str(r#"{"fields":{"TITLE":{"providers":["anilist"]}}}"#)
				.unwrap();

		let rule = &policy.fields[&MetadataField::Title];
		assert_eq!(rule.strategy, PolicyStrategy::First);
		assert!(rule.lock_respected);
	}

	async fn library_fixture() -> DatabaseConnection {
		use migrations::MigratorTrait;
		use models::{entity::library, shared::enums::FileStatus};
		use sea_orm::Database;

		let conn = Database::connect("sqlite::memory:").await.unwrap();
		migrations::Migrator::up(&conn, None).await.unwrap();
		let config = <library_config::ActiveModel as std::default::Default>::default()
			.insert(&conn)
			.await
			.unwrap();
		library::ActiveModel {
			id: Set("library".to_string()),
			name: Set("Library".to_string()),
			path: Set("/tmp/library".to_string()),
			status: Set(FileStatus::Ready),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();
		conn
	}

	#[tokio::test]
	async fn per_library_override_beats_the_server_default() {
		let conn = library_fixture().await;

		// No override: the effective policy is exactly the server default.
		let effective = effective_policy(&conn, "library").await.unwrap();
		assert_eq!(effective.library_override, None);
		assert_eq!(&effective.policy, MetadataPolicy::server_default());
		assert!(!effective.is_overridden(MetadataField::Title));

		let over =
			policy(&[(MetadataField::Title, &["anilist"], PolicyStrategy::Longest)]);
		let effective =
			set_library_override(&conn, "library", Some(&over), &known_provider_ids())
				.await
				.unwrap();

		// The named field is replaced, every other field is inherited.
		assert_eq!(
			effective.policy.fields[&MetadataField::Title],
			FieldPolicy::new(vec!["anilist".to_string()], PolicyStrategy::Longest)
		);
		assert_eq!(
			effective.policy.fields[&MetadataField::Summary],
			MetadataPolicy::server_default().fields[&MetadataField::Summary]
		);
		assert!(effective.is_overridden(MetadataField::Title));
		assert!(!effective.is_overridden(MetadataField::Summary));

		// It survives a reload, and the winning provider changes with it.
		let reloaded = effective_policy(&conn, "library").await.unwrap();
		assert_eq!(reloaded.policy, effective.policy);
		let candidates = [
			candidate(
				"mu-1",
				"mangaupdates",
				0.9,
				&[(MetadataField::Title, json!("Berserk"))],
			),
			candidate(
				"anilist-1",
				"anilist",
				0.1,
				&[(MetadataField::Title, json!("Berserk Deluxe Edition"))],
			),
		];
		assert_eq!(
			reloaded.policy.plan(&candidates, None, &[]).picks[0],
			FieldPick::Candidate {
				field: MetadataField::Title,
				candidate_id: "anilist-1".to_string(),
			}
		);
		assert_eq!(
			MetadataPolicy::server_default()
				.plan(&candidates, None, &[])
				.picks[0],
			FieldPick::Candidate {
				field: MetadataField::Title,
				candidate_id: "mu-1".to_string(),
			}
		);

		// Clearing the override falls back to the default.
		let cleared = set_library_override(&conn, "library", None, &known_provider_ids())
			.await
			.unwrap();
		assert_eq!(cleared.library_override, None);
		assert_eq!(&cleared.policy, MetadataPolicy::server_default());
	}

	#[tokio::test]
	async fn a_rejected_override_is_not_stored() {
		let conn = library_fixture().await;
		let invalid = policy(&[(
			MetadataField::Title,
			&["not-a-provider"],
			PolicyStrategy::First,
		)]);

		let error =
			set_library_override(&conn, "library", Some(&invalid), &known_provider_ids())
				.await
				.unwrap_err();

		assert!(matches!(error, PolicyError::UnknownProvider { .. }));
		assert_eq!(library_override(&conn, "library").await.unwrap(), None);
	}

	#[tokio::test]
	async fn an_unknown_library_is_an_error_not_a_default() {
		let conn = library_fixture().await;

		assert_eq!(
			effective_policy(&conn, "nope").await.unwrap_err(),
			PolicyError::UnknownLibrary("nope".to_string())
		);
	}

	#[tokio::test]
	async fn a_corrupt_stored_document_is_reported_not_ignored() {
		let conn = library_fixture().await;
		let config = library_config_for(&conn, "library").await.unwrap();
		let mut active = config.into_active_model();
		active.metadata_policy = Set(Some("{not json".to_string()));
		active.update(&conn).await.unwrap();

		assert!(matches!(
			effective_policy(&conn, "library").await.unwrap_err(),
			PolicyError::Invalid { .. }
		));
	}

	#[test]
	fn audio_server_default_never_rewrites_or_deletes() {
		let audio = AudioPolicy::server_default();
		assert_eq!(audio.single_file_weight, 30);
		assert!(!audio.auto_assemble);
		assert!(!audio.auto_chapters);
		assert!(audio.keep_original);
		assert_eq!(&AudioPolicy::default(), audio);
		// The server default document carries the section, so a caller that
		// reads `.audio()` on it never falls through to the static.
		assert_eq!(MetadataPolicy::server_default().audio(), audio);
		assert!(audio.validate().is_ok());
	}

	#[test]
	fn a_document_without_an_audio_section_inherits_the_default() {
		// A `library_configs.metadata_policy` written before the audio lane
		// existed has no `audio` key; it must still deserialize and resolve.
		let stored: MetadataPolicy =
			serde_json::from_str(r#"{"fields":{"TITLE":{"providers":["anilist"]}}}"#)
				.unwrap();
		assert_eq!(stored.audio, None);
		assert_eq!(stored.audio(), AudioPolicy::server_default());

		// A section that names only one key fills the rest from the default.
		let partial: MetadataPolicy =
			serde_json::from_str(r#"{"audio":{"autoAssemble":true}}"#).unwrap();
		let audio = partial.audio.expect("section present");
		assert!(audio.auto_assemble);
		assert_eq!(audio.single_file_weight, 30);
		assert!(audio.keep_original);

		// And an absent section is omitted on the way out rather than
		// written as an explicit null.
		assert_eq!(
			serde_json::to_string(&MetadataPolicy::default()).unwrap(),
			r#"{"fields":{}}"#
		);
	}

	#[test]
	fn audio_validation_refuses_a_meaningless_document() {
		let known = known_provider_ids();
		let with_audio = |audio: AudioPolicy| MetadataPolicy {
			audio: Some(audio),
			..MetadataPolicy::default()
		};

		assert_eq!(
			with_audio(AudioPolicy {
				single_file_weight: 101,
				..AudioPolicy::default()
			})
			.validate(&known),
			Err(PolicyError::AudioWeightOutOfRange(101))
		);
		// Exactly the sum of the built-in weights is still legal.
		assert!(with_audio(AudioPolicy {
			single_file_weight: MAX_CHECK_WEIGHT,
			..AudioPolicy::default()
		})
		.validate(&known)
		.is_ok());
		// Zero is legal: an advisory finding that does not move the score.
		assert!(with_audio(AudioPolicy {
			single_file_weight: 0,
			..AudioPolicy::default()
		})
		.validate(&known)
		.is_ok());

		assert_eq!(
			with_audio(AudioPolicy {
				keep_original: false,
				..AudioPolicy::default()
			})
			.validate(&known),
			Err(PolicyError::AudioKeepOriginalWithoutAssemble)
		);
		assert!(with_audio(AudioPolicy {
			auto_assemble: true,
			keep_original: false,
			..AudioPolicy::default()
		})
		.validate(&known)
		.is_ok());
	}

	#[test]
	fn overlay_replaces_the_audio_section_wholesale() {
		let base = MetadataPolicy {
			audio: Some(AudioPolicy {
				single_file_weight: 30,
				auto_assemble: false,
				auto_chapters: true,
				keep_original: true,
			}),
			..MetadataPolicy::default()
		};
		let over = MetadataPolicy {
			audio: Some(AudioPolicy {
				single_file_weight: 5,
				auto_assemble: true,
				auto_chapters: false,
				keep_original: true,
			}),
			..MetadataPolicy::default()
		};

		// Every key of the section moves together, including the ones whose
		// value happens to equal the base's.
		assert_eq!(base.overlay(&over).audio, over.audio);
		// An override silent about audio keeps the base's section.
		assert_eq!(base.overlay(&MetadataPolicy::default()).audio, base.audio);
	}

	#[tokio::test]
	async fn a_library_overrides_only_the_audio_section() {
		let conn = library_fixture().await;

		let effective = effective_policy(&conn, "library").await.unwrap();
		assert_eq!(effective.policy.audio(), AudioPolicy::server_default());
		assert!(!effective.audio_overridden());

		let over = MetadataPolicy {
			audio: Some(AudioPolicy {
				single_file_weight: 0,
				auto_assemble: true,
				auto_chapters: true,
				keep_original: false,
			}),
			..MetadataPolicy::default()
		};
		let effective =
			set_library_override(&conn, "library", Some(&over), &known_provider_ids())
				.await
				.unwrap();

		let audio = effective.policy.audio();
		assert_eq!(audio.single_file_weight, 0);
		assert!(audio.auto_assemble && audio.auto_chapters && !audio.keep_original);
		assert!(effective.audio_overridden());
		// An audio-only override leaves every field rule inherited.
		assert_eq!(
			effective.policy.fields,
			MetadataPolicy::server_default().fields
		);
		assert!(!effective.is_overridden(MetadataField::Title));

		// It survives a reload, and clearing it restores the default.
		let reloaded = effective_policy(&conn, "library").await.unwrap();
		assert_eq!(reloaded.policy.audio(), audio);

		let cleared = set_library_override(&conn, "library", None, &known_provider_ids())
			.await
			.unwrap();
		assert_eq!(cleared.policy.audio(), AudioPolicy::server_default());
		assert!(!cleared.audio_overridden());
	}

	#[tokio::test]
	async fn a_rejected_audio_section_is_not_stored() {
		let conn = library_fixture().await;
		let invalid = MetadataPolicy {
			audio: Some(AudioPolicy {
				single_file_weight: 4_000,
				..AudioPolicy::default()
			}),
			..MetadataPolicy::default()
		};

		let error =
			set_library_override(&conn, "library", Some(&invalid), &known_provider_ids())
				.await
				.unwrap_err();

		assert_eq!(error, PolicyError::AudioWeightOutOfRange(4_000));
		assert_eq!(library_override(&conn, "library").await.unwrap(), None);
	}
}
