use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::types::{ConfidenceFactor, ExternalMetadata, MatchCandidate};

/// Minimum pairwise confidence required before two provider records can be
/// automatically grouped as the same work.
pub const AUTO_MATCH_THRESHOLD: f32 = 0.90;

const TITLE_WEIGHT: f32 = 0.75;
const ALTERNATIVE_TITLE_WEIGHT: f32 = 0.70;
const YEAR_MATCH_WEIGHT: f32 = 0.08;
const AUTHOR_MATCH_WEIGHT: f32 = 0.10;
const PUBLISHER_MATCH_WEIGHT: f32 = 0.04;
const FORMAT_MATCH_WEIGHT: f32 = 0.03;
const MIN_TITLE_SIMILARITY: f64 = 0.65;
const AUTHOR_SIMILARITY_THRESHOLD: f64 = 0.92;
const PUBLISHER_SIMILARITY_THRESHOLD: f64 = 0.92;

/// A provider result plus optional evidence not represented in the shared
/// metadata DTO (notably the provider's medium/format) and a caller-supplied
/// work URL when the provider has a non-standard link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossProviderCandidate {
	pub candidate: MatchCandidate,
	#[serde(default)]
	pub format: Option<String>,
	#[serde(default)]
	pub provider_url: Option<String>,
}

impl CrossProviderCandidate {
	pub fn new(candidate: MatchCandidate) -> Self {
		let provider_url = match &candidate.metadata {
			ExternalMetadata::Media(metadata) => metadata.provider_url.clone(),
			ExternalMetadata::Series(_) => None,
		};
		Self {
			candidate,
			format: None,
			provider_url,
		}
	}

	/// Supply a provider-specific format or medium label for pair scoring.
	pub fn with_format(mut self, format: impl Into<String>) -> Self {
		self.format = Some(format.into());
		self
	}

	/// Override the standard provider link used when returning merged work
	/// references.
	pub fn with_provider_url(mut self, provider_url: impl Into<String>) -> Self {
		self.provider_url = Some(provider_url.into());
		self
	}
}

impl From<MatchCandidate> for CrossProviderCandidate {
	fn from(candidate: MatchCandidate) -> Self {
		Self::new(candidate)
	}
}

/// Pairwise cross-provider confidence and the evidence that contributed to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossProviderScore {
	pub confidence: f32,
	#[serde(default)]
	pub confidence_factors: Vec<ConfidenceFactor>,
}

impl CrossProviderScore {
	pub fn is_auto_match(&self) -> bool {
		self.confidence >= AUTO_MATCH_THRESHOLD
	}
}

/// Provider-qualified external identity. An external ID is meaningful only
/// together with its provider; IDs from different providers are never joined
/// or compared as if they shared a namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderReference {
	pub provider: String,
	pub external_id: String,
	pub url: Option<String>,
}

/// A candidate scored against a reference, borrowing the original candidate
/// so ranking does not copy provider metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredCrossProviderCandidate<'a> {
	pub candidate: &'a CrossProviderCandidate,
	pub score: CrossProviderScore,
}

/// A deterministic group of provider records believed to describe one work.
/// `confidence` is the weakest pairwise score in the group; singleton groups
/// have no cross-provider evidence and therefore score zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedWorkCandidate {
	pub candidates: Vec<CrossProviderCandidate>,
	pub confidence: f32,
	#[serde(default)]
	pub confidence_factors: Vec<ConfidenceFactor>,
	#[serde(default)]
	pub external_references: Vec<ProviderReference>,
}

impl MergedWorkCandidate {
	pub fn is_auto_match(&self) -> bool {
		self.confidence >= AUTO_MATCH_THRESHOLD
	}
}

/// Cross-provider work scoring and conservative same-work grouping. The
/// existing [`super::MatchScorer`] remains the provider-search scorer; this
/// scorer compares records from distinct providers using normalized work
/// metadata.
#[derive(Debug, Clone, Copy, Default)]
pub struct CrossProviderScorer;

impl CrossProviderScorer {
	/// Score two provider records. A provider match or a series/media kind
	/// mismatch is never considered a cross-provider work match.
	pub fn score_pair(
		&self,
		left: &CrossProviderCandidate,
		right: &CrossProviderCandidate,
	) -> CrossProviderScore {
		if left
			.candidate
			.provider
			.trim()
			.eq_ignore_ascii_case(right.candidate.provider.trim())
		{
			return CrossProviderScore {
				confidence: 0.0,
				confidence_factors: vec![factor("provider", 0.0, false)],
			};
		}

		let left_is_series =
			matches!(&left.candidate.metadata, ExternalMetadata::Series(_));
		let right_is_series =
			matches!(&right.candidate.metadata, ExternalMetadata::Series(_));
		if left_is_series != right_is_series {
			return CrossProviderScore {
				confidence: 0.0,
				confidence_factors: vec![
					factor("provider", 0.0, true),
					factor("metadata_kind", 0.0, false),
				],
			};
		}

		let left_evidence = evidence(&left.candidate.metadata);
		let right_evidence = evidence(&right.candidate.metadata);
		let title = title_evidence(left_evidence, right_evidence);
		let title_matched = title.similarity >= MIN_TITLE_SIMILARITY;
		let title_weight = if title.similarity == 1.0 && title.alternative {
			ALTERNATIVE_TITLE_WEIGHT
		} else {
			TITLE_WEIGHT
		};
		let title_contribution = if title_matched {
			title_weight * title.similarity as f32
		} else {
			0.0
		};

		let title_factor = if title.similarity == 1.0 {
			if title.alternative {
				"title_alternative"
			} else {
				"title_exact"
			}
		} else if title.alternative {
			"title_alternative_similarity"
		} else {
			"title_similarity"
		};

		let mut factors = vec![
			factor("provider", 0.0, true),
			factor(title_factor, title_contribution, title_matched),
		];
		let mut confidence = title_contribution;

		if let (Some(left_year), Some(right_year)) =
			(left_evidence.year, right_evidence.year)
		{
			let (contribution, matched) = match left_year.abs_diff(right_year) {
				0..=1 => (YEAR_MATCH_WEIGHT, true),
				2..=3 => (-0.03, false),
				_ => (-0.12, false),
			};
			confidence += contribution;
			factors.push(factor("year", contribution, matched));
		}

		let left_authors = normalized_people(left_evidence.authors);
		let right_authors = normalized_people(right_evidence.authors);
		if !left_authors.is_empty() && !right_authors.is_empty() {
			let matched = left_authors.iter().any(|left_author| {
				right_authors
					.iter()
					.any(|right_author| names_match(left_author, right_author))
			});
			let contribution = if matched { AUTHOR_MATCH_WEIGHT } else { -0.08 };
			confidence += contribution;
			factors.push(factor("author", contribution, matched));
		}

		if let (Some(left_publisher), Some(right_publisher)) =
			(left_evidence.publisher, right_evidence.publisher)
		{
			if let Some(matched) = values_match(
				left_publisher,
				right_publisher,
				PUBLISHER_SIMILARITY_THRESHOLD,
			) {
				let contribution = if matched {
					PUBLISHER_MATCH_WEIGHT
				} else {
					-0.03
				};
				confidence += contribution;
				factors.push(factor("publisher", contribution, matched));
			}
		}

		if let (Some(left_format), Some(right_format)) =
			(left.format.as_deref(), right.format.as_deref())
		{
			let left_format = normalize_format(left_format);
			let right_format = normalize_format(right_format);
			if !left_format.is_empty() && !right_format.is_empty() {
				let matched = left_format == right_format;
				let contribution = if matched { FORMAT_MATCH_WEIGHT } else { -0.20 };
				confidence += contribution;
				factors.push(factor("format", contribution, matched));
			}
		}

		CrossProviderScore {
			confidence: confidence.clamp(0.0, 1.0),
			confidence_factors: factors,
		}
	}

	/// Rank candidates against a reference, with the strongest match first.
	/// Ties are broken by provider and then provider-local external ID.
	pub fn rank_candidates<'a>(
		&self,
		reference: &CrossProviderCandidate,
		candidates: &'a [CrossProviderCandidate],
	) -> Vec<ScoredCrossProviderCandidate<'a>> {
		let mut scored: Vec<_> = candidates
			.iter()
			.map(|candidate| ScoredCrossProviderCandidate {
				candidate,
				score: self.score_pair(reference, candidate),
			})
			.collect();
		scored.sort_by(|left, right| {
			right
				.score
				.confidence
				.partial_cmp(&left.score.confidence)
				.unwrap_or(Ordering::Equal)
				.then_with(|| {
					left.candidate
						.candidate
						.provider
						.cmp(&right.candidate.candidate.provider)
				})
				.then_with(|| {
					left.candidate
						.candidate
						.external_id
						.cmp(&right.candidate.candidate.external_id)
				})
		});
		scored
	}

	/// Group only candidates for which every cross-provider pair reaches the
	/// auto-match threshold. Complete-link grouping prevents a weak transitive
	/// bridge from joining otherwise unrelated provider records.
	pub fn merge_same_work_candidates(
		&self,
		mut candidates: Vec<CrossProviderCandidate>,
	) -> Vec<MergedWorkCandidate> {
		candidates.sort_by(|left, right| candidate_order(left, right));
		let mut groups: Vec<Vec<usize>> = Vec::new();

		for candidate_index in 0..candidates.len() {
			let mut best_group: Option<(usize, CrossProviderScore)> = None;
			for (group_index, group) in groups.iter().enumerate() {
				let mut weakest_pair: Option<CrossProviderScore> = None;
				let mut complete_match = true;
				for &member_index in group {
					let pair = self.score_pair(
						&candidates[candidate_index],
						&candidates[member_index],
					);
					if !pair.is_auto_match() {
						complete_match = false;
						break;
					}
					if weakest_pair
						.as_ref()
						.map_or(true, |weakest| pair.confidence < weakest.confidence)
					{
						weakest_pair = Some(pair);
					}
				}
				if !complete_match {
					continue;
				}
				let Some(weakest_pair) = weakest_pair else {
					continue;
				};
				if best_group
					.as_ref()
					.map_or(true, |(_, best)| weakest_pair.confidence > best.confidence)
				{
					best_group = Some((group_index, weakest_pair));
				}
			}

			if let Some((group_index, _)) = best_group {
				groups[group_index].push(candidate_index);
			} else {
				groups.push(vec![candidate_index]);
			}
		}

		let mut candidates: Vec<Option<CrossProviderCandidate>> =
			candidates.into_iter().map(Some).collect();
		let mut merged = Vec::with_capacity(groups.len());
		for group in groups {
			let score = weakest_group_score(self, &candidates, &group);
			let mut members = Vec::with_capacity(group.len());
			for index in group {
				members.push(
					candidates[index]
						.take()
						.expect("each candidate belongs to exactly one group"),
				);
			}
			let mut external_references: Vec<_> =
				members.iter().map(provider_reference).collect();
			external_references.sort_by(|left, right| {
				left.provider
					.cmp(&right.provider)
					.then_with(|| left.external_id.cmp(&right.external_id))
			});
			external_references.dedup_by(|left, right| {
				left.provider == right.provider && left.external_id == right.external_id
			});
			merged.push(MergedWorkCandidate {
				candidates: members,
				confidence: score.confidence,
				confidence_factors: score.confidence_factors,
				external_references,
			});
		}

		merged.sort_by(|left, right| {
			right
				.confidence
				.partial_cmp(&left.confidence)
				.unwrap_or(Ordering::Equal)
				.then_with(|| work_order(left, right))
		});
		merged
	}
}

#[derive(Clone, Copy)]
struct Evidence<'a> {
	title: Option<&'a str>,
	alternative_titles: &'a [String],
	year: Option<i32>,
	authors: &'a [String],
	publisher: Option<&'a str>,
}

struct NormalizedTitle {
	value: String,
	alternative: bool,
}

struct TitleEvidence {
	similarity: f64,
	alternative: bool,
}

fn evidence(metadata: &ExternalMetadata) -> Evidence<'_> {
	match metadata {
		ExternalMetadata::Series(series) => Evidence {
			title: Some(&series.title),
			alternative_titles: &series.alternative_titles,
			year: series.year,
			authors: series.authors.as_deref().unwrap_or_default(),
			publisher: series.publisher.as_deref(),
		},
		ExternalMetadata::Media(media) => Evidence {
			title: media.title.as_deref(),
			alternative_titles: &[],
			year: media.year,
			authors: media.writers.as_deref().unwrap_or_default(),
			publisher: media.publisher.as_deref(),
		},
	}
}

fn title_evidence(left: Evidence<'_>, right: Evidence<'_>) -> TitleEvidence {
	let left_titles = normalized_titles(left);
	let right_titles = normalized_titles(right);
	let mut best = TitleEvidence {
		similarity: 0.0,
		alternative: false,
	};
	for left_title in &left_titles {
		for right_title in &right_titles {
			let similarity = pair_title_similarity(&left_title.value, &right_title.value);
			let alternative = left_title.alternative || right_title.alternative;
			if similarity > best.similarity
				|| (similarity == best.similarity && best.alternative && !alternative)
			{
				best = TitleEvidence {
					similarity,
					alternative,
				};
			}
		}
	}
	best
}

fn normalized_titles(evidence: Evidence<'_>) -> Vec<NormalizedTitle> {
	let mut titles = Vec::with_capacity(evidence.alternative_titles.len() + 1);
	if let Some(title) = evidence.title {
		push_normalized_title(&mut titles, title, false);
	}
	for alternative in evidence.alternative_titles {
		push_normalized_title(&mut titles, alternative, true);
	}
	titles
}

fn push_normalized_title(
	titles: &mut Vec<NormalizedTitle>,
	value: &str,
	alternative: bool,
) {
	let value = normalize_text(value);
	if !value.is_empty() {
		titles.push(NormalizedTitle { value, alternative });
	}
}

fn pair_title_similarity(left: &str, right: &str) -> f64 {
	if left == right {
		return 1.0;
	}
	strsim::sorensen_dice(left, right)
		.max(strsim::jaro_winkler(left, right))
		.max(super::title_similarity(left, right))
		.max(super::title_similarity(right, left))
}

/// Lowercase Unicode letters/digits and treat punctuation and spacing alike.
fn normalize_text(value: &str) -> String {
	let mut normalized = String::with_capacity(value.len());
	let mut pending_space = false;
	for character in value.chars().flat_map(char::to_lowercase) {
		if character.is_alphanumeric() {
			if pending_space && !normalized.is_empty() {
				normalized.push(' ');
			}
			normalized.push(character);
			pending_space = false;
		} else {
			pending_space = true;
		}
	}
	normalized
}

fn normalized_people(people: &[String]) -> Vec<String> {
	people
		.iter()
		.map(|person| {
			let normalized = normalize_text(person);
			let mut words: Vec<_> = normalized.split_whitespace().collect();
			words.sort_unstable();
			words.join(" ")
		})
		.filter(|person| !person.is_empty())
		.collect()
}

fn names_match(left: &str, right: &str) -> bool {
	left == right || strsim::jaro_winkler(left, right) >= AUTHOR_SIMILARITY_THRESHOLD
}

fn values_match(left: &str, right: &str, threshold: f64) -> Option<bool> {
	let left = normalize_text(left);
	let right = normalize_text(right);
	if left.is_empty() || right.is_empty() {
		None
	} else {
		Some(left == right || strsim::jaro_winkler(&left, &right) >= threshold)
	}
}

fn normalize_format(format: &str) -> String {
	let normalized = normalize_text(format);
	match normalized.as_str() {
		"comic book" | "comic books" | "graphic novel" | "graphic novels" => {
			"comic".into()
		},
		"manga series" => "manga".into(),
		"light novels" => "light novel".into(),
		"web novels" => "web novel".into(),
		"oneshot" => "one shot".into(),
		_ => normalized,
	}
}

fn factor(name: &str, weight: f32, matched: bool) -> ConfidenceFactor {
	ConfidenceFactor {
		factor: name.into(),
		weight,
		matched,
	}
}

fn candidate_order(
	left: &CrossProviderCandidate,
	right: &CrossProviderCandidate,
) -> Ordering {
	left.candidate
		.provider
		.cmp(&right.candidate.provider)
		.then_with(|| left.candidate.external_id.cmp(&right.candidate.external_id))
		.then_with(|| {
			metadata_title(&left.candidate.metadata)
				.cmp(metadata_title(&right.candidate.metadata))
		})
		.then_with(|| left.format.cmp(&right.format))
		.then_with(|| left.provider_url.cmp(&right.provider_url))
}

fn metadata_title(metadata: &ExternalMetadata) -> &str {
	match metadata {
		ExternalMetadata::Series(series) => &series.title,
		ExternalMetadata::Media(media) => media.title.as_deref().unwrap_or_default(),
	}
}

fn weakest_group_score(
	scorer: &CrossProviderScorer,
	candidates: &[Option<CrossProviderCandidate>],
	group: &[usize],
) -> CrossProviderScore {
	let mut weakest: Option<CrossProviderScore> = None;
	for (offset, &left_index) in group.iter().enumerate() {
		for &right_index in &group[offset + 1..] {
			let score = scorer.score_pair(
				candidates[left_index]
					.as_ref()
					.expect("candidate group index is valid"),
				candidates[right_index]
					.as_ref()
					.expect("candidate group index is valid"),
			);
			if weakest
				.as_ref()
				.map_or(true, |current| score.confidence < current.confidence)
			{
				weakest = Some(score);
			}
		}
	}
	weakest.unwrap_or(CrossProviderScore {
		confidence: 0.0,
		confidence_factors: Vec::new(),
	})
}

fn provider_reference(candidate: &CrossProviderCandidate) -> ProviderReference {
	let provider = &candidate.candidate.provider;
	let external_id = &candidate.candidate.external_id;
	let url = candidate
		.provider_url
		.as_deref()
		.filter(|url| !url.trim().is_empty())
		.map(str::to_string)
		.or_else(|| canonical_provider_url(provider, external_id));
	ProviderReference {
		provider: provider.clone(),
		external_id: external_id.clone(),
		url,
	}
}

fn canonical_provider_url(provider: &str, external_id: &str) -> Option<String> {
	if external_id.is_empty()
		|| !external_id
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
	{
		return None;
	}
	if provider.eq_ignore_ascii_case("anilist") && is_decimal_id(external_id) {
		Some(format!("https://anilist.co/manga/{external_id}"))
	} else if provider.eq_ignore_ascii_case("mal") && is_decimal_id(external_id) {
		Some(format!("https://myanimelist.net/manga/{external_id}"))
	} else if provider.eq_ignore_ascii_case("mangadex") {
		Some(format!("https://mangadex.org/title/{external_id}"))
	} else if provider.eq_ignore_ascii_case("mangaupdates") && is_decimal_id(external_id)
	{
		Some(format!(
			"https://www.mangaupdates.com/series.html?id={external_id}"
		))
	} else if provider.eq_ignore_ascii_case("metron") && is_decimal_id(external_id) {
		Some(format!("https://metron.cloud/series/{external_id}/"))
	} else {
		None
	}
}

fn is_decimal_id(external_id: &str) -> bool {
	!external_id.is_empty() && external_id.bytes().all(|byte| byte.is_ascii_digit())
}

fn work_order(left: &MergedWorkCandidate, right: &MergedWorkCandidate) -> Ordering {
	left.external_references
		.first()
		.map(|reference| (&reference.provider, &reference.external_id))
		.cmp(
			&right
				.external_references
				.first()
				.map(|reference| (&reference.provider, &reference.external_id)),
		)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::ExternalSeriesMetadata;

	fn provider_candidate(
		provider: &str,
		external_id: &str,
		title: &str,
		alternative_titles: &[&str],
		year: Option<i32>,
		authors: &[&str],
		publisher: Option<&str>,
		format: &str,
	) -> CrossProviderCandidate {
		let candidate = MatchCandidate {
			provider: provider.into(),
			external_id: external_id.into(),
			metadata: ExternalMetadata::Series(ExternalSeriesMetadata {
				provider: provider.into(),
				external_id: external_id.into(),
				title: title.into(),
				alternative_titles: alternative_titles
					.iter()
					.map(|title| (*title).into())
					.collect(),
				year,
				authors: (!authors.is_empty())
					.then(|| authors.iter().map(|author| (*author).into()).collect()),
				publisher: publisher.map(str::to_string),
				..Default::default()
			}),
			confidence: 0.0,
			confidence_factors: Vec::new(),
		};
		CrossProviderCandidate::new(candidate).with_format(format)
	}

	// Values follow the shared DTOs produced by the five provider mappers:
	// AniList title/synonyms and staff, MangaUpdates detail aliases/publishers,
	// MAL alternative titles and role-split authors, MangaDex localized titles
	// and relationships, and Metron series title/year/publisher records.
	fn anilist_fruit_basket() -> CrossProviderCandidate {
		provider_candidate(
			"anilist",
			"30013",
			"Fruits Basket",
			&["Furūtsu Basuketto"],
			Some(1998),
			&["Natsuki Takaya"],
			None,
			"MANGA",
		)
	}

	fn mangaupdates_fruit_basket() -> CrossProviderCandidate {
		provider_candidate(
			"mangaupdates",
			"1106",
			"Fruits Basket",
			&["Furūtsu Basuketto"],
			Some(1998),
			&["Takaya, Natsuki"],
			Some("Hakusensha"),
			"manga series",
		)
	}

	fn mal_fruit_basket() -> CrossProviderCandidate {
		provider_candidate(
			"mal",
			"11",
			"Fruits Basket",
			&["Furūtsu Basuketto"],
			Some(1998),
			&["Takaya Natsuki"],
			Some("Hakusensha"),
			"Manga",
		)
	}

	fn mangadex_fruit_basket() -> CrossProviderCandidate {
		provider_candidate(
			"mangadex",
			"a96676e5-8ae2-425e-b549-7f15dd34a6d8",
			"Fruits Basket",
			&["フルーツバスケット"],
			Some(1998),
			&["Takaya, Natsuki"],
			None,
			"manga",
		)
	}

	fn metron_sandman() -> CrossProviderCandidate {
		provider_candidate(
			"metron",
			"51229",
			"The Sandman",
			&["Sandman"],
			Some(1989),
			&["Neil Gaiman"],
			Some("DC Comics"),
			"Graphic Novel",
		)
	}

	#[test]
	fn alternative_titles_and_punctuation_normalize_before_comparison() {
		let scorer = CrossProviderScorer;
		let left = provider_candidate(
			"anilist",
			"85495",
			"Koe no Katachi",
			&["A Silent Voice"],
			Some(2013),
			&["Yoshitoki Oima"],
			None,
			"Manga",
		);
		let right = provider_candidate(
			"mal",
			"44347",
			"  A SILENT-VOICE! ",
			&[],
			Some(2013),
			&["Oima, Yoshitoki"],
			None,
			"manga series",
		);

		let score = scorer.score_pair(&left, &right);
		assert!(score.is_auto_match(), "score was {}", score.confidence);
		assert!(score
			.confidence_factors
			.iter()
			.any(|factor| factor.factor == "title_alternative" && factor.matched));
		assert!(score
			.confidence_factors
			.iter()
			.any(|factor| factor.factor == "author" && factor.matched));
	}

	#[test]
	fn ranks_candidates_best_first_using_year_author_publisher_and_format() {
		let scorer = CrossProviderScorer;
		let reference = mal_fruit_basket();
		let close_match = mangaupdates_fruit_basket();
		let wrong_year = provider_candidate(
			"mangadex",
			"a96676e5-8ae2-425e-b549-7f15dd34a6d8",
			"Fruits Basket",
			&[],
			Some(2022),
			&["Natsuki Takaya"],
			Some("Hakusensha"),
			"Manga",
		);
		let unrelated = metron_sandman();
		let candidates = [unrelated, wrong_year, close_match];
		let ranked = scorer.rank_candidates(&reference, &candidates);

		assert_eq!(ranked[0].candidate.candidate.provider, "mangaupdates");
		assert!(ranked[0].score.confidence > ranked[1].score.confidence);
		assert!(ranked[1].score.confidence > ranked[2].score.confidence);
		assert!(ranked[0].score.is_auto_match());
		assert!(!ranked[1].score.is_auto_match());
	}

	#[test]
	fn merges_provider_shaped_candidates_and_keeps_provider_qualified_links() {
		let scorer = CrossProviderScorer;
		let grouped = scorer.merge_same_work_candidates(vec![
			metron_sandman(),
			mal_fruit_basket(),
			anilist_fruit_basket(),
			mangadex_fruit_basket(),
			mangaupdates_fruit_basket(),
		]);

		assert_eq!(grouped.len(), 2);
		assert!(grouped[0].is_auto_match());
		assert_eq!(grouped[0].candidates.len(), 4);
		assert_eq!(grouped[0].external_references.len(), 4);
		assert_eq!(grouped[1].candidates.len(), 1);
		assert!(!grouped[1].is_auto_match());

		let references = &grouped[0].external_references;
		assert_eq!(
			references
				.iter()
				.map(|reference| reference.provider.as_str())
				.collect::<Vec<_>>(),
			["anilist", "mal", "mangadex", "mangaupdates"]
		);
		assert_eq!(references[0].external_id, "30013");
		assert_eq!(
			references[0].url.as_deref(),
			Some("https://anilist.co/manga/30013")
		);
		assert_eq!(references[1].external_id, "11");
		assert_eq!(
			references[1].url.as_deref(),
			Some("https://myanimelist.net/manga/11")
		);
		assert_eq!(
			references[2].url.as_deref(),
			Some("https://mangadex.org/title/a96676e5-8ae2-425e-b549-7f15dd34a6d8")
		);
		assert_eq!(
			references[3].url.as_deref(),
			Some("https://www.mangaupdates.com/series.html?id=1106")
		);
	}

	#[test]
	fn same_provider_and_same_numeric_ids_do_not_create_cross_provider_matches() {
		let scorer = CrossProviderScorer;
		let same_provider_a = anilist_fruit_basket();
		let same_provider_b = provider_candidate(
			"anilist",
			"30014",
			"Fruits Basket",
			&[],
			Some(1998),
			&["Natsuki Takaya"],
			None,
			"Manga",
		);
		let same_provider_score = scorer.score_pair(&same_provider_a, &same_provider_b);
		assert_eq!(same_provider_score.confidence, 0.0);
		assert!(!same_provider_score.is_auto_match());
		assert_eq!(
			scorer
				.merge_same_work_candidates(vec![same_provider_a, same_provider_b])
				.len(),
			2
		);

		let same_id_other_provider = provider_candidate(
			"mal",
			"30013",
			"The Sandman",
			&[],
			Some(1989),
			&["Neil Gaiman"],
			Some("DC Comics"),
			"Comic",
		);
		let id_score =
			scorer.score_pair(&anilist_fruit_basket(), &same_id_other_provider);
		assert!(!id_score.is_auto_match());
		assert!(id_score.confidence < AUTO_MATCH_THRESHOLD);
	}

	#[test]
	fn year_and_format_conflicts_block_automatic_merges() {
		let scorer = CrossProviderScorer;
		let reference = mal_fruit_basket();
		let different_format = provider_candidate(
			"mangaupdates",
			"1107",
			"Fruits Basket",
			&[],
			Some(1998),
			&["Natsuki Takaya"],
			Some("Hakusensha"),
			"Light Novel",
		);
		let different_year = provider_candidate(
			"mangadex",
			"a96676e5-8ae2-425e-b549-7f15dd34a6d8",
			"Fruits Basket",
			&[],
			Some(2022),
			&["Natsuki Takaya"],
			Some("Hakusensha"),
			"Manga",
		);

		let format_score = scorer.score_pair(&reference, &different_format);
		let year_score = scorer.score_pair(&reference, &different_year);
		assert!(!format_score.is_auto_match());
		assert!(!year_score.is_auto_match());
		assert!(format_score
			.confidence_factors
			.iter()
			.any(|factor| factor.factor == "format" && !factor.matched));
		assert!(year_score
			.confidence_factors
			.iter()
			.any(|factor| factor.factor == "year" && !factor.matched));
	}

	#[test]
	fn series_and_media_records_are_not_merged_as_the_same_candidate_kind() {
		let series = mal_fruit_basket();
		let media = CrossProviderCandidate::new(MatchCandidate {
			provider: "mangadex".into(),
			external_id: "a96676e5-8ae2-425e-b549-7f15dd34a6d8".into(),
			metadata: ExternalMetadata::Media(crate::types::ExternalMediaMetadata {
				provider: "mangadex".into(),
				external_id: "a96676e5-8ae2-425e-b549-7f15dd34a6d8".into(),
				title: Some("Fruits Basket".into()),
				..Default::default()
			}),
			confidence: 0.0,
			confidence_factors: Vec::new(),
		});

		let score = CrossProviderScorer.score_pair(&series, &media);
		assert_eq!(score.confidence, 0.0);
		assert!(score
			.confidence_factors
			.iter()
			.any(|factor| factor.factor == "metadata_kind" && !factor.matched));
	}
}
