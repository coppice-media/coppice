use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use stump_api_types::settings::{SettingDefinition, SettingValues};
use unicode_normalization::UnicodeNormalization;

use super::{enabled_setting, outcome, QUALITY_VERSION};
use crate::ingest::contract::{
	BookSnapshot, QualityCheck, QualityCheckError, QualityCheckOutcome,
};

/// The parser's deterministic confidence state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FilenameParseStatus {
	Pass,
	Warn,
	Fail,
}

/// One possible interpretation of a normalized file name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilenameTuple {
	pub series: Option<String>,
	pub title: String,
	pub number: Option<f64>,
	pub year: Option<i32>,
}

/// The result of the fixed filename/series grammar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedFilename {
	pub normalized: String,
	pub series: Option<String>,
	pub title: String,
	pub number: Option<f64>,
	pub year: Option<i32>,
	pub status: FilenameParseStatus,
	/// Rejected interpretations, in deterministic input order.
	pub alternatives: Vec<FilenameTuple>,
}

impl ParsedFilename {
	pub fn tuple(&self) -> FilenameTuple {
		FilenameTuple {
			series: self.series.clone(),
			title: self.title.clone(),
			number: self.number,
			year: self.year,
		}
	}
}

/// Parse a source path using the staged-ingest filename grammar.
///
/// This function intentionally returns a value for malformed input rather than a
/// `Result`: malformed names are quality findings, not worker failures.
pub fn parse_filename(input: &str) -> ParsedFilename {
	let raw_path = input.replace('\\', "/").nfkc().collect::<String>();
	let (parent, filename) = split_parent_and_filename(&raw_path);
	let filename_without_extension = trim_extension(filename);
	let normalized_filename = normalize_text(filename_without_extension);
	let normalized_parent = parent
		.map(|parent| normalize_text(&parent))
		.filter(|p| !p.is_empty());
	let normalized_path = normalized_parent
		.as_ref()
		.map(|parent| format!("{parent}/{normalized_filename}"))
		.unwrap_or_else(|| normalized_filename.clone());

	let boundaries: Vec<usize> = normalized_filename
		.match_indices(" - ")
		.map(|(index, _)| index)
		.collect();
	let (prefix, mut title_source) = if let Some(index) = boundaries.first().copied() {
		(
			Some(normalized_filename[..index].trim().to_string()),
			normalized_filename[index + 3..].trim().to_string(),
		)
	} else {
		(None, normalized_filename.trim().to_string())
	};

	let prefix = prefix.filter(|value| !value.is_empty());
	let mut ambiguous = boundaries.len() > 1;
	let mut conflict = false;
	if let (Some(parent), Some(prefix)) = (&normalized_parent, &prefix) {
		if !parent.eq_ignore_ascii_case(prefix) {
			conflict = true;
			ambiguous = true;
		}
	}

	let series = prefix.clone().or(normalized_parent.clone());
	let mut malformed = false;

	let marker_matches = marker_regex()
		.find_iter(&title_source)
		.map(|m| {
			let marker = m.as_str();
			let digits = marker
				.chars()
				.skip_while(|ch| !ch.is_ascii_digit())
				.take_while(|ch| ch.is_ascii_digit())
				.collect::<String>();
			let number = digits.parse::<f64>().ok();
			if number.is_none() {
				malformed = true;
			}
			(m.start(), m.end(), number)
		})
		.collect::<Vec<_>>();
	if has_malformed_marker(&title_source) {
		malformed = true;
	}

	let marker_spans = marker_matches
		.iter()
		.map(|(start, end, _)| (*start, *end))
		.collect::<Vec<_>>();
	title_source = remove_spans(&title_source, &marker_spans);
	title_source = strip_malformed_markers(&title_source);

	let year_matches = find_years(&title_source);
	if year_matches
		.iter()
		.any(|(_, _, year)| !(1900..=2099).contains(year))
	{
		malformed = true;
	}
	if year_matches.len() > 1 {
		ambiguous = true;
	}
	let year_spans = year_matches
		.iter()
		.map(|(start, end, _)| (*start, *end))
		.collect::<Vec<_>>();
	title_source = remove_spans(&title_source, &year_spans);
	let title = clean_title(&title_source);
	if title.is_empty() {
		malformed = true;
	}

	let number = marker_matches.first().and_then(|(_, _, number)| *number);
	let year = year_matches
		.first()
		.map(|(_, _, year)| *year)
		.filter(|year| (1900..=2099).contains(year));

	let mut alternatives = Vec::new();
	for (_, _, marker_number) in marker_matches.iter().skip(1) {
		alternatives.push(FilenameTuple {
			series: series.clone(),
			title: title.clone(),
			number: *marker_number,
			year,
		});
	}
	if let (Some(parent), Some(prefix)) = (normalized_parent, prefix) {
		if !parent.eq_ignore_ascii_case(&prefix) {
			alternatives.push(FilenameTuple {
				series: Some(parent),
				title: title.clone(),
				number,
				year,
			});
		}
	}
	if year_matches.len() > 1 {
		for (_, _, alternate_year) in year_matches.iter().skip(1) {
			alternatives.push(FilenameTuple {
				series: series.clone(),
				title: title.clone(),
				number,
				year: Some(*alternate_year),
			});
		}
	}
	if boundaries.len() > 1 {
		// Retain the text after the first boundary as the chosen title, but
		// expose the alternate split as evidence rather than silently guessing.
		for index in boundaries.iter().skip(1) {
			let alternate_title = clean_title(&normalized_filename[index + 3..]);
			if !alternate_title.is_empty() {
				alternatives.push(FilenameTuple {
					series: Some(normalized_filename[..*index].trim().to_string()),
					title: alternate_title,
					number,
					year,
				});
			}
		}
	}

	let status = if malformed {
		FilenameParseStatus::Fail
	} else if ambiguous || marker_matches.len() > 1 || conflict {
		FilenameParseStatus::Warn
	} else {
		FilenameParseStatus::Pass
	};

	ParsedFilename {
		normalized: normalized_path,
		series,
		title,
		number,
		year,
		status,
		alternatives,
	}
}

fn normalize_text(value: &str) -> String {
	value
		.nfkc()
		.collect::<String>()
		.replace(['_', '.'], " ")
		.split_whitespace()
		.collect::<Vec<_>>()
		.join(" ")
}

fn split_parent_and_filename(path: &str) -> (Option<String>, &str) {
	if let Some(index) = path.rfind('/') {
		let parent = path[..index]
			.rsplit('/')
			.next()
			.filter(|value| !value.is_empty())
			.map(ToOwned::to_owned);
		(parent, &path[index + 1..])
	} else {
		(None, path)
	}
}

fn trim_extension(filename: &str) -> &str {
	match filename.rfind('.') {
		Some(index) if index > 0 && index + 1 < filename.len() => &filename[..index],
		_ => filename,
	}
}

fn clean_title(value: &str) -> String {
	let mut title = value.split_whitespace().collect::<Vec<_>>().join(" ");
	while title.ends_with('-') {
		title.pop();
		title = title.trim_end().to_string();
	}
	title
}

fn remove_spans(value: &str, spans: &[(usize, usize)]) -> String {
	if spans.is_empty() {
		return value.to_string();
	}
	let mut result = String::with_capacity(value.len());
	let mut cursor = 0;
	for (start, end) in spans {
		if *start > cursor {
			result.push_str(&value[cursor..*start]);
		}
		cursor = *end;
	}
	if cursor < value.len() {
		result.push_str(&value[cursor..]);
	}
	result
}

static MARKER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(?i)(?:#\s*[0-9]+|\bv\s*[0-9]+|\b(?:vol|volume|issue|chapter)\s+[0-9]+)")
		.expect("filename marker regex is valid")
});

fn marker_regex() -> &'static Regex {
	&MARKER_REGEX
}

fn find_years(value: &str) -> Vec<(usize, usize, i32)> {
	let mut cursor = 0;
	let mut matches = Vec::new();
	for token in value.split_whitespace() {
		let Some(relative_start) = value[cursor..].find(token) else {
			continue;
		};
		let start = cursor + relative_start;
		let end = start + token.len();
		cursor = end;
		if token.len() == 4 && token.chars().all(|ch| ch.is_ascii_digit()) {
			if let Ok(year) = token.parse::<i32>() {
				matches.push((start, end, year));
			}
		}
	}
	matches
}

fn strip_malformed_markers(value: &str) -> String {
	remove_spans(value, &malformed_marker_spans(value))
}

fn malformed_marker_spans(value: &str) -> Vec<(usize, usize)> {
	let tokens = value.split_whitespace().collect::<Vec<_>>();
	let mut spans = Vec::new();
	let mut cursor = 0;
	for (index, token) in tokens.iter().enumerate() {
		let Some(relative_start) = value[cursor..].find(token) else {
			continue;
		};
		let start = cursor + relative_start;
		let end = start + token.len();
		cursor = end;
		if is_malformed_marker_token(token, tokens.get(index + 1).copied()) {
			spans.push((start, end));
		}
	}
	spans
}

fn is_malformed_marker_token(token: &str, next: Option<&str>) -> bool {
	let lower = token.to_ascii_lowercase();
	if lower == "#" {
		return next.is_none_or(|next| !next.chars().all(|ch| ch.is_ascii_digit()));
	}
	if lower.starts_with('#') {
		let digits = lower.trim_start_matches('#');
		return digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit());
	}
	if matches!(lower.as_str(), "v" | "vol" | "volume" | "issue" | "chapter") {
		return next.is_none_or(|next| !next.chars().all(|ch| ch.is_ascii_digit()));
	}
	lower.starts_with('v')
		&& lower.len() > 1
		&& lower
			.as_bytes()
			.get(1)
			.is_some_and(|byte| byte.is_ascii_digit())
		&& !lower[1..].chars().all(|ch| ch.is_ascii_digit())
}

fn has_malformed_marker(value: &str) -> bool {
	!malformed_marker_spans(value).is_empty()
}

#[derive(Default)]
pub struct FilenameSeriesParseCheck;

impl FilenameSeriesParseCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for FilenameSeriesParseCheck {
	fn id(&self) -> &'static str {
		"filename_series_parse"
	}

	fn name(&self) -> &'static str {
		"Filename and series parsing"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		10
	}

	fn settings(&self) -> &[SettingDefinition] {
		super::enabled_settings()
	}

	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(outcome(
				self.id(),
				self.name(),
				crate::ingest::contract::QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({"disabled": true}),
			));
		}
		let input = if book.relative_path.trim().is_empty() {
			book.source_filename.clone()
		} else {
			format!(
				"{}/{}",
				book.relative_path.trim_end_matches('/'),
				book.source_filename
			)
		};
		let parsed = parse_filename(&input);
		let status = match parsed.status {
			FilenameParseStatus::Pass => crate::ingest::contract::QualityStatus::Pass,
			FilenameParseStatus::Warn => crate::ingest::contract::QualityStatus::Warn,
			FilenameParseStatus::Fail => crate::ingest::contract::QualityStatus::Fail,
		};
		let normalized_score = match status {
			crate::ingest::contract::QualityStatus::Pass => 1.0,
			crate::ingest::contract::QualityStatus::Warn => 0.5,
			crate::ingest::contract::QualityStatus::Fail => 0.0,
			crate::ingest::contract::QualityStatus::NotApplicable => 0.0,
		};
		Ok(outcome(
			self.id(),
			self.name(),
			status,
			normalized_score,
			serde_json::to_value(parsed).map_err(|error| {
				QualityCheckError::Internal {
					check_id: self.id().to_string(),
					message: error.to_string(),
				}
			})?,
		))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_standalone_title() {
		let parsed = parse_filename("Dune_01.cbz");
		assert_eq!(parsed.status, FilenameParseStatus::Pass);
		assert_eq!(parsed.title, "Dune 01");
		assert_eq!(parsed.series, None);
	}

	#[test]
	fn parses_series_number_and_year() {
		let parsed = parse_filename("Wayfarers - The Long Way v2 2020.epub");
		assert_eq!(parsed.status, FilenameParseStatus::Pass);
		assert_eq!(parsed.series.as_deref(), Some("Wayfarers"));
		assert_eq!(parsed.title, "The Long Way");
		assert_eq!(parsed.number, Some(2.0));
		assert_eq!(parsed.year, Some(2020));
	}

	#[test]
	fn parent_and_prefix_conflict_warns() {
		let parsed = parse_filename("Other/Wayfarers - Title #1.cbz");
		assert_eq!(parsed.status, FilenameParseStatus::Warn);
		assert_eq!(parsed.alternatives.len(), 1);
	}

	#[test]
	fn multiple_markers_warn() {
		let parsed = parse_filename("Title #1 v2.cbz");
		assert_eq!(parsed.status, FilenameParseStatus::Warn);
		assert_eq!(parsed.alternatives.len(), 1);
	}

	#[test]
	fn malformed_marker_fails() {
		let parsed = parse_filename("Title #.cbz");
		assert_eq!(parsed.status, FilenameParseStatus::Fail);
	}

	#[test]
	fn empty_title_fails() {
		let parsed = parse_filename("Series - #1.cbz");
		assert_eq!(parsed.status, FilenameParseStatus::Fail);
	}
}
