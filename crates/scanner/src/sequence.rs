//! Chapter/volume sequence parsing and gap detection for file names.
//!
//! The rules mirror the Kavita-recommended `CBZ-Missing-Sequence-Checker`
//! behaviour documented at
//! <https://wiki.kavitareader.com/guides/external-tools/cbz-missing-sequence-checker/>:
//! the identifier vocabulary (`Chapter`, `Chap`, `Ch`, `C`, `Episode`, `Ep`,
//! `Volume`, `Vol`, `v`), omnibus ranges (`Chapter 01-03` covers 1, 2 and 3),
//! decimal/interstitial chapters that must not open a gap, `()`/`[]` release
//! metadata stripping, the changing-numeric-token fallback for names without an
//! identifier, and gap-only reporting. Implemented from that behaviour
//! description alone; no upstream source was consulted.
//!
//! This module is the single sequence parser in the workspace: `stump_tools`
//! (the `missing-sequence` tool) and `stump_core` (the
//! `missing_chapters_in_series` quality check) both use it.

use std::{collections::BTreeSet, fmt};

/// Widest omnibus range accepted from one file name. A wider `a-b` pair is
/// release metadata (a year span, a resolution), not chapters.
pub const MAX_RANGE_LEN: i64 = 500;

/// Widest integer span a folder may cover before gap enumeration is refused.
/// Without it a stray date or ISBN token would report millions of gaps.
pub const MAX_SEQUENCE_SPAN: i64 = 10_000;

/// Numbering space a parsed identifier belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SequenceKind {
	/// `Chapter`, `Chap`, `Ch`, `C`, `Episode`, `Ep`.
	Chapter,
	/// `Volume`, `Vol`, `v`.
	Volume,
}

impl SequenceKind {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Chapter => "chapter",
			Self::Volume => "volume",
		}
	}
}

/// A sequence number stored as integer thousandths, so decimal chapters
/// (`112.1`) sort, compare and format exactly without floating point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SequenceNumber(i64);

impl SequenceNumber {
	/// Fixed-point scale: three decimal places.
	pub const SCALE: i64 = 1_000;

	pub const fn from_integer(value: i64) -> Self {
		Self(value * Self::SCALE)
	}

	pub const fn from_thousandths(value: i64) -> Self {
		Self(value)
	}

	pub const fn thousandths(self) -> i64 {
		self.0
	}

	/// The integer chapter a number belongs to: `112.1` belongs to `112`, so
	/// an interstitial chapter never opens a gap of its own.
	pub const fn integer_part(self) -> i64 {
		self.0.div_euclid(Self::SCALE)
	}

	pub const fn is_integer(self) -> bool {
		self.0 % Self::SCALE == 0
	}
}

impl fmt::Display for SequenceNumber {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let whole = self.integer_part();
		let fraction = self.0.rem_euclid(Self::SCALE);
		if fraction == 0 {
			return write!(f, "{whole}");
		}
		let mut digits = format!("{fraction:03}");
		while digits.ends_with('0') {
			digits.pop();
		}
		write!(f, "{whole}.{digits}")
	}
}

/// Inclusive run of found integer numbers, rendered `48-80` or `81`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceRange {
	pub start: i64,
	pub end: i64,
}

impl fmt::Display for SequenceRange {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.start == self.end {
			write!(f, "{}", self.start)
		} else {
			write!(f, "{}-{}", self.start, self.end)
		}
	}
}

/// How one file's number was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SequenceSource {
	/// An explicit `Chapter`/`Volume`-style identifier carried the number.
	Identifier,
	/// No file in the group had an identifier, so the changing numeric token
	/// across the group was used.
	Fallback,
	/// Neither route produced a number.
	#[default]
	Unidentified,
}

impl SequenceSource {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Identifier => "identifier",
			Self::Fallback => "fallback",
			Self::Unidentified => "unidentified",
		}
	}
}

/// An identifier match in one file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedIdentifier {
	pub kind: SequenceKind,
	/// First covered number; equal to `end` unless the name carries a range.
	pub start: SequenceNumber,
	pub end: SequenceNumber,
}

/// One analysed file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceEntry {
	/// File name as supplied, unmodified.
	pub name: String,
	pub kind: Option<SequenceKind>,
	pub source: SequenceSource,
	/// Inclusive number span the file covers, `None` when unidentified.
	pub span: Option<(SequenceNumber, SequenceNumber)>,
}

/// Sequence analysis of one folder (or one series).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SequenceAnalysis {
	/// Numbering space the group was sequenced in; `None` in fallback mode,
	/// where names carry bare numbers of unknown kind.
	pub kind: Option<SequenceKind>,
	/// `Identifier`, `Fallback`, or `Unidentified` when nothing parsed.
	pub source: SequenceSource,
	/// One entry per input name, in input order.
	pub entries: Vec<SequenceEntry>,
	/// Contiguous runs of covered integers, ascending.
	pub found_ranges: Vec<SequenceRange>,
	/// Integers absent between the lowest and highest covered number.
	pub missing: Vec<i64>,
	/// Interstitial numbers observed (`112.1`); recorded as evidence, never
	/// as gaps.
	pub decimals: Vec<SequenceNumber>,
	/// Names that produced no number, in input order.
	pub unidentified: Vec<String>,
	/// Set when the covered span exceeds [`MAX_SEQUENCE_SPAN`], in which case
	/// `missing` is deliberately left empty.
	pub span_exceeded: bool,
}

impl SequenceAnalysis {
	pub fn has_gaps(&self) -> bool {
		!self.missing.is_empty()
	}

	/// `["48-80", "82-92"]`, the found-ranges rendering used by reports.
	pub fn found_ranges_display(&self) -> Vec<String> {
		self.found_ranges
			.iter()
			.map(SequenceRange::to_string)
			.collect()
	}

	/// `"81"` / `"18, 22-24"`: [`Self::missing`] as one compact line, for
	/// report surfaces that can only show a scalar.
	pub fn missing_summary(&self) -> String {
		to_ranges(self.missing.iter().copied())
			.iter()
			.map(SequenceRange::to_string)
			.collect::<Vec<_>>()
			.join(", ")
	}

	/// Number of names that yielded a number.
	pub fn identified_count(&self) -> usize {
		self.entries
			.iter()
			.filter(|entry| entry.span.is_some())
			.count()
	}
}

/// Parse the chapter/volume identifier of a single file name.
///
/// Chapters outrank volumes (`Series v02 Ch 010` sequences by chapter) and the
/// right-most identifier of the winning kind is used.
pub fn parse_identifier(file_name: &str) -> Option<ParsedIdentifier> {
	best_identifier(&clean_name(file_name), None)
}

/// Parse one numbering space of a file name, ignoring the other: `Series v03
/// Ch 010` yields volume `3` for [`SequenceKind::Volume`] and chapter `10` for
/// [`SequenceKind::Chapter`]. The right-most match wins.
pub fn parse_identifier_of(
	file_name: &str,
	kind: SequenceKind,
) -> Option<ParsedIdentifier> {
	best_identifier(&clean_name(file_name), Some(kind))
}

/// Analyse a group of file names that belong to one folder or series.
pub fn analyze_sequence<S: AsRef<str>>(file_names: &[S]) -> SequenceAnalysis {
	let cleaned = file_names
		.iter()
		.map(|name| clean_name(name.as_ref()))
		.collect::<Vec<_>>();
	let identifiers = cleaned
		.iter()
		.map(|name| best_identifier(name, None))
		.collect::<Vec<_>>();

	let kind = folder_kind(&identifiers);
	let entries = match kind {
		Some(kind) => identifier_entries(file_names, &identifiers, kind),
		None => fallback_entries(file_names, &cleaned),
	};

	let source = entries
		.iter()
		.map(|entry| entry.source)
		.find(|source| *source != SequenceSource::Unidentified)
		.unwrap_or(SequenceSource::Unidentified);

	let mut analysis = SequenceAnalysis {
		kind,
		source,
		unidentified: entries
			.iter()
			.filter(|entry| entry.span.is_none())
			.map(|entry| entry.name.clone())
			.collect(),
		entries,
		..Default::default()
	};
	fill_gaps(&mut analysis);
	analysis
}

/// Strip the directory, the extension, and `()`/`[]` release metadata,
/// normalize `_` to spaces, and collapse the resulting whitespace. Callers
/// that need the leftover title text should start from this, so the workspace
/// keeps one cleaning convention.
pub fn clean_name(file_name: &str) -> String {
	let name = file_name
		.rsplit(['/', '\\'])
		.next()
		.unwrap_or(file_name)
		.trim();
	let stem = strip_extension(name);
	let mut cleaned = String::with_capacity(stem.len());
	let mut depth = 0usize;
	for character in stem.chars() {
		match character {
			'(' | '[' => {
				depth += 1;
				push_space(&mut cleaned);
			},
			')' | ']' => {
				depth = depth.saturating_sub(1);
				push_space(&mut cleaned);
			},
			_ if depth > 0 => {},
			_ if character.is_whitespace() || character == '_' => {
				push_space(&mut cleaned)
			},
			_ => cleaned.push(character),
		}
	}
	while cleaned.ends_with(' ') {
		cleaned.pop();
	}
	cleaned
}

/// Append one separating space, never a leading or a repeated one.
fn push_space(cleaned: &mut String) {
	if !cleaned.is_empty() && !cleaned.ends_with(' ') {
		cleaned.push(' ');
	}
}

/// Drop a trailing extension only when it looks like one: short, alphanumeric,
/// and containing a letter, so the `5` of `Ch 5.5` survives.
fn strip_extension(name: &str) -> &str {
	match name.rsplit_once('.') {
		Some((stem, extension))
			if !stem.is_empty()
				&& (1..=5).contains(&extension.len())
				&& extension.chars().all(|c| c.is_ascii_alphanumeric())
				&& extension.chars().any(|c| c.is_ascii_alphabetic()) =>
		{
			stem
		},
		_ => name,
	}
}

enum Token {
	/// Byte range of an ASCII letter run inside the cleaned name.
	Word(usize, usize),
	Number(SequenceNumber, SequenceNumber),
}

fn tokenize(cleaned: &str) -> Vec<Token> {
	let bytes = cleaned.as_bytes();
	let mut tokens = Vec::new();
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index].is_ascii_alphabetic() {
			let start = index;
			while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
				index += 1;
			}
			tokens.push(Token::Word(start, index));
		} else if bytes[index].is_ascii_digit() {
			let (span, next) = scan_span(bytes, index);
			if let Some((start, end)) = span {
				tokens.push(Token::Number(start, end));
			}
			index = next.max(index + 1);
		} else {
			index += 1;
		}
	}
	tokens
}

/// Scan one number, absorbing an omnibus range (`01-03`, `01 - 03`, `01~03`)
/// when both sides parse and the run is no wider than [`MAX_RANGE_LEN`].
fn scan_span(
	bytes: &[u8],
	index: usize,
) -> (Option<(SequenceNumber, SequenceNumber)>, usize) {
	let (start, after_start) = scan_decimal(bytes, index);
	let Some(start) = start else {
		return (None, after_start);
	};
	let mut probe = after_start;
	while probe < bytes.len() && bytes[probe] == b' ' {
		probe += 1;
	}
	if probe < bytes.len() && matches!(bytes[probe], b'-' | b'~') {
		probe += 1;
		while probe < bytes.len() && bytes[probe] == b' ' {
			probe += 1;
		}
		let (end, after_end) = scan_decimal(bytes, probe);
		if let Some(end) = end {
			if end >= start && end.integer_part() - start.integer_part() <= MAX_RANGE_LEN
			{
				return (Some((start, end)), after_end);
			}
		}
	}
	(Some((start, start)), after_start)
}

/// Parse `digits[.digits]` at `index`; the returned index always advances past
/// the digit run so callers cannot loop.
fn scan_decimal(bytes: &[u8], index: usize) -> (Option<SequenceNumber>, usize) {
	let start = index;
	let mut index = index;
	while index < bytes.len() && bytes[index].is_ascii_digit() {
		index += 1;
	}
	if index == start {
		return (None, index);
	}
	// A run this long is an identifier, hash, or ISBN, never a chapter.
	if index - start > 9 {
		return (None, index);
	}
	let mut value = 0i64;
	for byte in &bytes[start..index] {
		value = value * 10 + i64::from(byte - b'0');
	}
	let mut number = value * SequenceNumber::SCALE;
	if index + 1 < bytes.len()
		&& bytes[index] == b'.'
		&& bytes[index + 1].is_ascii_digit()
	{
		index += 1;
		let fraction_start = index;
		while index < bytes.len() && bytes[index].is_ascii_digit() {
			index += 1;
		}
		let mut scale = SequenceNumber::SCALE / 10;
		for byte in &bytes[fraction_start..index] {
			if scale == 0 {
				break;
			}
			number += i64::from(byte - b'0') * scale;
			scale /= 10;
		}
	}
	(Some(SequenceNumber::from_thousandths(number)), index)
}

/// The documented identifier vocabulary.
fn identifier_kind(word: &str) -> Option<SequenceKind> {
	const CHAPTER: [&str; 6] = ["chapter", "chap", "ch", "c", "episode", "ep"];
	const VOLUME: [&str; 3] = ["volume", "vol", "v"];
	if CHAPTER
		.iter()
		.any(|identifier| word.eq_ignore_ascii_case(identifier))
	{
		Some(SequenceKind::Chapter)
	} else if VOLUME
		.iter()
		.any(|identifier| word.eq_ignore_ascii_case(identifier))
	{
		Some(SequenceKind::Volume)
	} else {
		None
	}
}

/// Right-most identifier match in a cleaned name. With `only` set, other
/// numbering spaces are ignored; without it, chapters outrank volumes.
fn best_identifier(
	cleaned: &str,
	only: Option<SequenceKind>,
) -> Option<ParsedIdentifier> {
	let tokens = tokenize(cleaned);
	let mut best: Option<ParsedIdentifier> = None;
	for (index, token) in tokens.iter().enumerate() {
		let Token::Word(start, end) = token else {
			continue;
		};
		let Some(kind) = identifier_kind(&cleaned[*start..*end])
			.filter(|kind| only.is_none_or(|only| only == *kind))
		else {
			continue;
		};
		let Some(Token::Number(number_start, number_end)) = tokens.get(index + 1) else {
			continue;
		};
		let candidate = ParsedIdentifier {
			kind,
			start: *number_start,
			end: *number_end,
		};
		best = match best {
			// Chapters outrank volumes; within one kind the right-most
			// identifier wins.
			Some(previous)
				if previous.kind == SequenceKind::Chapter
					&& kind == SequenceKind::Volume =>
			{
				Some(previous)
			},
			_ => Some(candidate),
		};
	}
	best
}

fn folder_kind(identifiers: &[Option<ParsedIdentifier>]) -> Option<SequenceKind> {
	let mut volume = None;
	for identifier in identifiers.iter().flatten() {
		match identifier.kind {
			SequenceKind::Chapter => return Some(SequenceKind::Chapter),
			SequenceKind::Volume => volume = Some(SequenceKind::Volume),
		}
	}
	volume
}

fn identifier_entries<S: AsRef<str>>(
	file_names: &[S],
	identifiers: &[Option<ParsedIdentifier>],
	kind: SequenceKind,
) -> Vec<SequenceEntry> {
	file_names
		.iter()
		.zip(identifiers)
		.map(|(name, identifier)| {
			match identifier.filter(|identifier| identifier.kind == kind) {
				Some(identifier) => SequenceEntry {
					name: name.as_ref().to_string(),
					kind: Some(kind),
					source: SequenceSource::Identifier,
					span: Some((identifier.start, identifier.end)),
				},
				None => unidentified_entry(name.as_ref()),
			}
		})
		.collect()
}

fn fallback_entries<S: AsRef<str>>(
	file_names: &[S],
	cleaned: &[String],
) -> Vec<SequenceEntry> {
	let spans = cleaned
		.iter()
		.map(|name| numbers_in(name))
		.collect::<Vec<_>>();
	let Some(position) = changing_position(&spans) else {
		return file_names
			.iter()
			.map(|name| unidentified_entry(name.as_ref()))
			.collect();
	};
	file_names
		.iter()
		.zip(&spans)
		.map(|(name, numbers)| {
			match numbers
				.len()
				.checked_sub(position + 1)
				.and_then(|index| numbers.get(index))
			{
				Some(span) => SequenceEntry {
					name: name.as_ref().to_string(),
					kind: None,
					source: SequenceSource::Fallback,
					span: Some(*span),
				},
				None => unidentified_entry(name.as_ref()),
			}
		})
		.collect()
}

fn unidentified_entry(name: &str) -> SequenceEntry {
	SequenceEntry {
		name: name.to_string(),
		kind: None,
		source: SequenceSource::Unidentified,
		span: None,
	}
}

fn numbers_in(cleaned: &str) -> Vec<(SequenceNumber, SequenceNumber)> {
	tokenize(cleaned)
		.into_iter()
		.filter_map(|token| match token {
			Token::Number(start, end) => Some((start, end)),
			Token::Word(..) => None,
		})
		.collect()
}

/// Pick the numeric token position that changes across the group, counted from
/// the right so trailing chapter numbers align across differing prefixes.
/// Static tokens (a repeated year or series number) lose on distinct count;
/// ties go to the right-most position.
fn changing_position(spans: &[Vec<(SequenceNumber, SequenceNumber)>]) -> Option<usize> {
	let longest = spans.iter().map(Vec::len).max().unwrap_or(0);
	let mut best: Option<(usize, usize, usize)> = None;
	for position in 0..longest {
		let mut distinct = BTreeSet::new();
		let mut coverage = 0usize;
		for numbers in spans {
			if let Some((start, _)) = numbers
				.len()
				.checked_sub(position + 1)
				.and_then(|index| numbers.get(index))
			{
				coverage += 1;
				distinct.insert(start.thousandths());
			}
		}
		if coverage == 0 {
			continue;
		}
		let candidate = (distinct.len(), coverage, position);
		best = match best {
			Some(current) if (current.0, current.1) >= (candidate.0, candidate.1) => {
				Some(current)
			},
			_ => Some(candidate),
		};
	}
	best.map(|(_, _, position)| position)
}

fn fill_gaps(analysis: &mut SequenceAnalysis) {
	let mut covered = BTreeSet::new();
	for entry in &analysis.entries {
		let Some((start, end)) = entry.span else {
			continue;
		};
		for value in start.integer_part()..=end.integer_part() {
			covered.insert(value);
		}
		if !start.is_integer() {
			analysis.decimals.push(start);
		}
		if end != start && !end.is_integer() {
			analysis.decimals.push(end);
		}
	}
	analysis.decimals.sort_unstable();
	analysis.decimals.dedup();

	let (Some(&lowest), Some(&highest)) = (covered.first(), covered.last()) else {
		return;
	};
	analysis.found_ranges = to_ranges(covered.iter().copied());

	if highest - lowest > MAX_SEQUENCE_SPAN {
		analysis.span_exceeded = true;
		return;
	}
	analysis.missing = (lowest..=highest)
		.filter(|value| !covered.contains(value))
		.collect();
}

/// Group ascending, de-duplicated integers into inclusive runs: `[48..=80,
/// 82..=92]` for chapters 48-92 without 81.
pub fn to_ranges<I: IntoIterator<Item = i64>>(values: I) -> Vec<SequenceRange> {
	let mut ranges: Vec<SequenceRange> = Vec::new();
	for value in values {
		match ranges.last_mut() {
			Some(range) if range.end + 1 == value => range.end = value,
			_ => ranges.push(SequenceRange {
				start: value,
				end: value,
			}),
		}
	}
	ranges
}

#[cfg(test)]
mod tests {
	use super::*;

	fn analyze(names: &[&str]) -> SequenceAnalysis {
		analyze_sequence(names)
	}

	#[test]
	fn identifier_vocabulary_is_recognized() {
		for (name, kind, number) in [
			("Series Chapter 12.cbz", SequenceKind::Chapter, 12),
			("Series Chap 12.cbz", SequenceKind::Chapter, 12),
			("Series Ch.12.cbz", SequenceKind::Chapter, 12),
			("Series c012.cbz", SequenceKind::Chapter, 12),
			("Series Episode 12.cbz", SequenceKind::Chapter, 12),
			("Series Ep 12.cbz", SequenceKind::Chapter, 12),
			("Series Volume 12.cbz", SequenceKind::Volume, 12),
			("Series Vol 12.cbz", SequenceKind::Volume, 12),
			("Series v12.cbz", SequenceKind::Volume, 12),
		] {
			let parsed = parse_identifier(name).expect(name);
			assert_eq!(parsed.kind, kind, "{name}");
			assert_eq!(parsed.start, SequenceNumber::from_integer(number), "{name}");
			assert_eq!(parsed.end, SequenceNumber::from_integer(number), "{name}");
		}
	}

	#[test]
	fn chapters_outrank_volumes_and_right_most_wins() {
		let parsed = parse_identifier("Series v02 Ch 010.cbz").expect("parsed");
		assert_eq!(parsed.kind, SequenceKind::Chapter);
		assert_eq!(parsed.start, SequenceNumber::from_integer(10));

		let parsed = parse_identifier("Chapter 1 Special Chapter 7.cbz").expect("parsed");
		assert_eq!(parsed.start, SequenceNumber::from_integer(7));
	}

	#[test]
	fn each_numbering_space_can_be_parsed_on_its_own() {
		let name = "Series v03 Ch 010.5 (2025).cbz";
		assert_eq!(
			parse_identifier_of(name, SequenceKind::Volume)
				.expect("volume")
				.start,
			SequenceNumber::from_integer(3)
		);
		assert_eq!(
			parse_identifier_of(name, SequenceKind::Chapter)
				.expect("chapter")
				.start,
			SequenceNumber::from_thousandths(10_500)
		);
		assert!(parse_identifier_of("Series v03.cbz", SequenceKind::Chapter).is_none());
	}

	#[test]
	fn clean_name_drops_directory_extension_and_metadata() {
		assert_eq!(
			clean_name("/library/XYZ/XYZ_Vol. 3 (2025) [Digital].cbz"),
			"XYZ Vol. 3"
		);
		// A numeric "extension" is a decimal chapter, not a suffix.
		assert_eq!(clean_name("Series Ch 5.5"), "Series Ch 5.5");
	}

	#[test]
	fn identifier_words_need_an_adjacent_number() {
		assert!(parse_identifier("Prologue.cbz").is_none());
		assert!(parse_identifier("Chapter Two.cbz").is_none());
		assert!(parse_identifier("Marvel v Capcom.cbz").is_none());
	}

	#[test]
	fn omnibus_range_covers_every_chapter() {
		let analysis = analyze(&["Series Chapter 01-03.cbz"]);
		assert_eq!(analysis.found_ranges_display(), vec!["1-3"]);
		assert!(analysis.missing.is_empty());
		let (start, end) = analysis.entries[0].span.expect("span");
		assert_eq!(start, SequenceNumber::from_integer(1));
		assert_eq!(end, SequenceNumber::from_integer(3));
	}

	#[test]
	fn decimal_chapters_do_not_open_gaps() {
		let analysis = analyze(&[
			"Series Chapter 112.cbz",
			"Series Chapter 112.1.cbz",
			"Series Chapter 113.cbz",
		]);
		assert!(analysis.missing.is_empty());
		assert_eq!(analysis.found_ranges_display(), vec!["112-113"]);
		assert_eq!(
			analysis.decimals,
			vec![SequenceNumber::from_thousandths(112_100)]
		);

		// A lone interstitial belongs to its integer chapter, so a folder
		// holding only 27 and 27.2 is complete.
		let analysis = analyze(&["Series Ch 27.cbz", "Series Ch 27.2.cbz"]);
		assert!(analysis.missing.is_empty());
		assert_eq!(analysis.found_ranges_display(), vec!["27"]);
	}

	#[test]
	fn release_metadata_is_stripped_before_parsing() {
		let analysis = analyze(&[
			"XYZ (2025) Chapter 5 (Digital).cbz",
			"XYZ (2025) Chapter 6 [Ripper].cbz",
		]);
		assert_eq!(analysis.kind, Some(SequenceKind::Chapter));
		assert_eq!(analysis.found_ranges_display(), vec!["5-6"]);
		assert!(analysis.missing.is_empty());
	}

	#[test]
	fn fallback_tracks_the_changing_numeric_token() {
		let analysis =
			analyze(&["XYZ 2025 017.cbz", "XYZ 2025 018.cbz", "XYZ 2025 020.cbz"]);
		assert_eq!(analysis.source, SequenceSource::Fallback);
		assert_eq!(analysis.kind, None);
		assert_eq!(analysis.found_ranges_display(), vec!["17-18", "20"]);
		assert_eq!(analysis.missing, vec![19]);
	}

	#[test]
	fn unidentified_names_are_listed_and_never_sequenced() {
		let analysis = analyze(&["XYZ 017.cbz", "Prologue.cbz"]);
		assert_eq!(analysis.unidentified, vec!["Prologue.cbz".to_string()]);
		assert_eq!(analysis.identified_count(), 1);
		assert!(analysis.missing.is_empty());

		let analysis = analyze(&["Series Chapter 1.cbz", "Prologue.cbz"]);
		assert_eq!(analysis.unidentified, vec!["Prologue.cbz".to_string()]);
	}

	#[test]
	fn wide_spans_refuse_to_enumerate_gaps() {
		let analysis = analyze(&["Series Ch 1.cbz", "Series Ch 20250101.cbz"]);
		assert!(analysis.span_exceeded);
		assert!(analysis.missing.is_empty());
		assert_eq!(analysis.found_ranges_display(), vec!["1", "20250101"]);
	}

	#[test]
	fn numbers_format_without_trailing_zeroes() {
		assert_eq!(SequenceNumber::from_integer(81).to_string(), "81");
		assert_eq!(
			SequenceNumber::from_thousandths(112_100).to_string(),
			"112.1"
		);
		assert_eq!(
			SequenceRange { start: 48, end: 80 }.to_string(),
			"48-80".to_string()
		);
		assert_eq!(SequenceRange { start: 81, end: 81 }.to_string(), "81");
	}

	#[test]
	fn missing_summary_groups_consecutive_gaps() {
		let analysis = analyze(&["S Ch 1.cbz", "S Ch 5.cbz", "S Ch 7.cbz"]);
		assert_eq!(analysis.missing, vec![2, 3, 4, 6]);
		assert_eq!(analysis.missing_summary(), "2-4, 6");
		assert_eq!(analysis.found_ranges_display(), vec!["1", "5", "7"]);

		let analysis = analyze(&["S Ch 1.cbz", "S Ch 2.cbz"]);
		assert_eq!(analysis.missing_summary(), "");
	}
}
