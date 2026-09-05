//! `SeriesFilterV2Dto` and the smart-filter string encoding
//! (`Kavita.Services/Helpers/SmartFilter/SmartFilterHelper.cs`).

use serde::{Deserialize, Serialize};

use crate::dto::MangaFormat;

int_enum! {
	/// `FilterComparison`.
	FilterComparison {
		Equal = 0,
		GreaterThan = 1,
		GreaterThanEqual = 2,
		LessThan = 3,
		LessThanEqual = 4,
		Contains = 5,
		MustContains = 6,
		Matches = 7,
		NotContains = 8,
		NotEqual = 9,
		BeginsWith = 10,
		EndsWith = 11,
		IsBefore = 12,
		IsAfter = 13,
		IsInLast = 14,
		IsNotInLast = 15,
		IsEmpty = 16,
		IsNotEmpty = 17,
	}
}

int_enum! {
	/// `FilterCombination`.
	FilterCombination {
		Or = 0,
		And = 1,
	}
}

int_enum! {
	/// `SeriesFilterField`.
	SeriesFilterField {
		Summary = 0,
		SeriesName = 1,
		PublicationStatus = 2,
		Languages = 3,
		AgeRating = 4,
		UserRating = 5,
		Tags = 6,
		CollectionTags = 7,
		Translators = 8,
		Characters = 9,
		Publisher = 10,
		Editor = 11,
		CoverArtist = 12,
		Letterer = 13,
		Colorist = 14,
		Inker = 15,
		Penciller = 16,
		Writers = 17,
		Genres = 18,
		Libraries = 19,
		ReadProgress = 20,
		Formats = 21,
		ReleaseYear = 22,
		ReadTime = 23,
		Path = 24,
		FilePath = 25,
		WantToRead = 26,
		ReadingDate = 27,
		AverageRating = 28,
		Imprint = 29,
		Team = 30,
		Location = 31,
		ReadLast = 32,
		FileSize = 33,
		CollapseSeriesRelationships = 34,
	}
}

int_enum! {
	/// `SeriesSortField`.
	SeriesSortField {
		SortName = 1,
		CreatedDate = 2,
		LastModifiedDate = 3,
		LastChapterAdded = 4,
		TimeToRead = 5,
		ReleaseYear = 6,
		ReadProgress = 7,
		AverageRating = 8,
		Random = 9,
		UserRating = 10,
		UnreadChapterCount = 11,
	}
}

int_enum! {
	/// `FilterEntityType`.
	FilterEntityType {
		Series = 0,
		ReadingList = 1,
		Person = 2,
		Annotation = 3,
	}
}

/// The field of a filter statement. Kavita binds any integer here and only
/// fails later for values outside `SeriesFilterField`; the Kavita Tachiyomi
/// extension sends fields newer than 0.9.1.4 (`People`) on prefixed
/// searches, so unknown values are carried through and ignored rather than
/// rejecting the whole request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementField {
	Known(SeriesFilterField),
	Unknown(i32),
}

impl StatementField {
	pub fn value(self) -> i32 {
		match self {
			Self::Known(field) => field.value(),
			Self::Unknown(value) => value,
		}
	}

	pub fn known(self) -> Option<SeriesFilterField> {
		match self {
			Self::Known(field) => Some(field),
			Self::Unknown(_) => None,
		}
	}
}

impl From<SeriesFilterField> for StatementField {
	fn from(field: SeriesFilterField) -> Self {
		Self::Known(field)
	}
}

impl From<i32> for StatementField {
	fn from(value: i32) -> Self {
		SeriesFilterField::try_from(value).map_or(Self::Unknown(value), Self::Known)
	}
}

impl Serialize for StatementField {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_i32(self.value())
	}
}

impl<'de> Deserialize<'de> for StatementField {
	fn deserialize<D: serde::Deserializer<'de>>(
		deserializer: D,
	) -> Result<Self, D::Error> {
		i32::deserialize(deserializer).map(Self::from)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesFilterStatementDto {
	pub comparison: FilterComparison,
	pub field: StatementField,
	#[serde(default)]
	pub value: String,
}

impl SeriesFilterStatementDto {
	pub fn new(
		comparison: FilterComparison,
		field: SeriesFilterField,
		value: impl Into<String>,
	) -> Self {
		Self {
			comparison,
			field: StatementField::Known(field),
			value: value.into(),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesSortOptionDto {
	pub sort_field: SeriesSortField,
	pub is_ascending: bool,
}

impl Default for SeriesSortOptionDto {
	fn default() -> Self {
		Self {
			sort_field: SeriesSortField::SortName,
			is_ascending: true,
		}
	}
}

/// `SeriesFilterV2Dto`, the body of `POST /api/Series/all-v2` and `/v2` and
/// the decoded form returned by `POST /api/Filter/decode`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesFilterV2Dto {
	#[serde(default)]
	pub id: i32,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub statements: Vec<SeriesFilterStatementDto>,
	#[serde(default = "default_combination")]
	pub combination: FilterCombination,
	/// `null` when the client (Turnleaf) or the encoded filter carries no sort;
	/// Kavita then orders by sort name ascending (`Sort(userId, null)`).
	#[serde(default)]
	pub sort_options: Option<SeriesSortOptionDto>,
	#[serde(default)]
	pub entity_type: FilterEntityType,
	#[serde(default)]
	pub limit_to: i32,
}

fn default_combination() -> FilterCombination {
	FilterCombination::And
}

impl Default for FilterEntityType {
	fn default() -> Self {
		Self::Series
	}
}

impl SeriesFilterV2Dto {
	pub fn effective_sort(&self) -> SeriesSortOptionDto {
		self.sort_options.clone().unwrap_or_default()
	}
}

impl Default for SeriesFilterV2Dto {
	fn default() -> Self {
		Self {
			id: 0,
			name: None,
			statements: Vec::new(),
			combination: FilterCombination::And,
			sort_options: None,
			entity_type: FilterEntityType::Series,
			limit_to: 0,
		}
	}
}

impl SeriesFilterStatementDto {
	/// Kavita parses list-valued statements as comma-separated integers.
	pub fn int_values(&self) -> Result<Vec<i32>, String> {
		self.value
			.split(',')
			.map(str::trim)
			.filter(|value| !value.is_empty())
			.map(|value| {
				value
					.parse::<i32>()
					.map_err(|_| format!("'{value}' is not a valid integer"))
			})
			.collect()
	}

	pub fn formats(&self) -> Result<Vec<MangaFormat>, String> {
		self.int_values()?
			.into_iter()
			.map(|value| {
				MangaFormat::try_from(value)
					.map_err(|value| format!("{value} is not a valid MangaFormat"))
			})
			.collect()
	}

	pub fn float_value(&self) -> Result<f32, String> {
		self.value
			.trim()
			.parse::<f32>()
			.map_err(|_| format!("'{}' is not a valid number", self.value))
	}
}

const SORT_OPTIONS_KEY: &str = "sortOptions=";
const NAME_KEY: &str = "name=";
const ENTITY_TYPE_KEY: &str = "entityType=";
const SORT_FIELD_KEY: &str = "sortField=";
const IS_ASCENDING_KEY: &str = "isAscending=";
const STATEMENTS_KEY: &str = "stmts=";
const LIMIT_TO_KEY: &str = "limitTo=";
const COMBINATION_KEY: &str = "combination=";
const STATEMENT_COMPARISON_KEY: &str = "comparison=";
const STATEMENT_FIELD_KEY: &str = "field=";
const STATEMENT_VALUE_KEY: &str = "value=";
/// `SmartFilterHelper.StatementSeparator` (U+FFFD).
const STATEMENT_SEPARATOR: char = '\u{fffd}';
/// `SmartFilterHelper.InnerStatementSeparator` (U+00A6).
const INNER_STATEMENT_SEPARATOR: char = '\u{a6}';

fn escape(value: &str) -> String {
	urlencoding::encode(value).into_owned()
}

fn unescape(value: &str) -> String {
	urlencoding::decode(value)
		.map(|decoded| decoded.into_owned())
		.unwrap_or_else(|_| value.to_owned())
}

/// `SmartFilterHelper.Encode(SeriesFilterV2Dto)`.
pub fn encode_series_filter(filter: &SeriesFilterV2Dto) -> String {
	let name = match filter.name.as_deref() {
		Some(name) if !name.trim().is_empty() => format!("{NAME_KEY}{}&", escape(name)),
		_ => String::new(),
	};
	let entity_type =
		format!("{ENTITY_TYPE_KEY}{}&", entity_type_name(filter.entity_type));
	let statements = if filter.statements.is_empty() {
		String::new()
	} else {
		let joined = filter
			.statements
			.iter()
			.map(|statement| {
				escape(&format!(
					"{STATEMENT_COMPARISON_KEY}{}{INNER_STATEMENT_SEPARATOR}{STATEMENT_FIELD_KEY}{}{INNER_STATEMENT_SEPARATOR}{STATEMENT_VALUE_KEY}{}",
					statement.comparison.value(),
					statement.field.value(),
					escape(&statement.value)
				))
			})
			.collect::<Vec<_>>()
			.join(&STATEMENT_SEPARATOR.to_string());
		format!("{STATEMENTS_KEY}{}", escape(&joined))
	};
	let sort_options = match filter.sort_options.as_ref() {
		Some(sort) => format!(
			"{SORT_OPTIONS_KEY}{}",
			escape(&format!(
				"{SORT_FIELD_KEY}{}{INNER_STATEMENT_SEPARATOR}{IS_ASCENDING_KEY}{}",
				sort.sort_field.value(),
				if sort.is_ascending { "True" } else { "False" }
			))
		),
		None => String::new(),
	};
	format!(
		"{name}{entity_type}{statements}&{sort_options}&{LIMIT_TO_KEY}{}&{COMBINATION_KEY}{}",
		filter.limit_to,
		filter.combination.value()
	)
}

fn entity_type_name(entity_type: FilterEntityType) -> &'static str {
	match entity_type {
		FilterEntityType::Series => "Series",
		FilterEntityType::ReadingList => "ReadingList",
		FilterEntityType::Person => "Person",
		FilterEntityType::Annotation => "Annotation",
	}
}

/// `SmartFilterHelper.PeekEntityType`: a missing or unparsable entity type
/// means a series filter.
pub fn peek_entity_type(encoded: &str) -> FilterEntityType {
	let Some((_, rest)) = encoded.split_once(ENTITY_TYPE_KEY) else {
		return FilterEntityType::Series;
	};
	let raw = rest.split('&').next().unwrap_or_default();
	match raw {
		"Series" | "0" => FilterEntityType::Series,
		"ReadingList" | "1" => FilterEntityType::ReadingList,
		"Person" | "2" => FilterEntityType::Person,
		"Annotation" | "3" => FilterEntityType::Annotation,
		_ => FilterEntityType::Series,
	}
}

fn parse_enum_token<T: TryFrom<i32, Error = i32>>(
	token: &str,
	what: &str,
) -> Result<T, String> {
	let raw = token.split('=').nth(1).unwrap_or_default().trim();
	let value = raw
		.parse::<i32>()
		.map_err(|_| format!("'{raw}' is not a valid {what}"))?;
	T::try_from(value).map_err(|value| format!("{value} is not a valid {what}"))
}

/// `SmartFilterHelper.DecodeFilter` for the series entity type.
pub fn decode_series_filter(encoded: &str) -> Result<SeriesFilterV2Dto, String> {
	let mut filter = SeriesFilterV2Dto::default();
	if encoded.trim().is_empty() {
		return Ok(filter);
	}
	for part in encoded.split('&') {
		if let Some(rest) = part.strip_prefix(SORT_OPTIONS_KEY) {
			filter.sort_options = Some(decode_sort_options(rest)?);
		} else if let Some(rest) = part.strip_prefix(LIMIT_TO_KEY) {
			filter.limit_to = rest
				.parse::<i32>()
				.map_err(|_| format!("'{rest}' is not a valid limitTo"))?;
		} else if part.starts_with(COMBINATION_KEY) {
			filter.combination = parse_enum_token(part, "FilterCombination")?;
		} else if let Some(rest) = part.strip_prefix(STATEMENTS_KEY) {
			filter.statements = decode_statements(rest)?;
		} else if let Some(rest) = part.strip_prefix(NAME_KEY) {
			filter.name = Some(unescape(rest));
		} else if part.starts_with(ENTITY_TYPE_KEY) {
			filter.entity_type = peek_entity_type(part);
		}
	}
	Ok(filter)
}

fn decode_statements(encoded: &str) -> Result<Vec<SeriesFilterStatementDto>, String> {
	let mut statements = Vec::new();
	for raw in unescape(encoded).split(STATEMENT_SEPARATOR) {
		let decoded = unescape(raw);
		let parts = decoded.split(INNER_STATEMENT_SEPARATOR).collect::<Vec<_>>();
		if parts.len() < 3 {
			continue;
		}
		statements.push(SeriesFilterStatementDto {
			comparison: parse_enum_token(parts[0], "FilterComparison")?,
			field: StatementField::from(
				parts[1]
					.split('=')
					.nth(1)
					.unwrap_or_default()
					.trim()
					.parse::<i32>()
					.map_err(|_| {
						format!("'{}' is not a valid SeriesFilterField", parts[1])
					})?,
			),
			value: unescape(parts[2].split_once('=').map_or("", |(_, value)| value)),
		});
	}
	Ok(statements)
}

fn decode_sort_options(encoded: &str) -> Result<SeriesSortOptionDto, String> {
	let decoded = unescape(encoded);
	let parts = decoded.split(INNER_STATEMENT_SEPARATOR).collect::<Vec<_>>();
	let is_ascending = parts
		.iter()
		.find(|part| part.starts_with(IS_ASCENDING_KEY))
		.map(|part| part.trim().replacen(IS_ASCENDING_KEY, "", 1))
		.is_some_and(|value| value.eq_ignore_ascii_case("true"));
	let Some(sort_field) = parts.iter().find(|part| part.starts_with(SORT_FIELD_KEY))
	else {
		return Ok(SeriesSortOptionDto {
			sort_field: SeriesSortField::SortName,
			is_ascending: false,
		});
	};
	Ok(SeriesSortOptionDto {
		sort_field: parse_enum_token(sort_field, "SeriesSortField")?,
		is_ascending,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sample() -> SeriesFilterV2Dto {
		SeriesFilterV2Dto {
			id: 0,
			name: Some("Unread manga".to_owned()),
			statements: vec![
				SeriesFilterStatementDto::new(
					FilterComparison::Equal,
					SeriesFilterField::Libraries,
					"1,2",
				),
				SeriesFilterStatementDto::new(
					FilterComparison::LessThan,
					SeriesFilterField::ReadProgress,
					"100",
				),
			],
			combination: FilterCombination::And,
			sort_options: Some(SeriesSortOptionDto {
				sort_field: SeriesSortField::LastChapterAdded,
				is_ascending: false,
			}),
			entity_type: FilterEntityType::Series,
			limit_to: 10,
		}
	}

	#[test]
	fn encode_matches_smart_filter_helper_layout() {
		// Verified against kavita-ref 0.9.1.4: POST /api/Filter/decode restores
		// both statements (value "1,2" intact), the name, sort and limit.
		let encoded = encode_series_filter(&sample());
		assert_eq!(
			encoded,
			"name=Unread%20manga&entityType=Series&stmts=comparison%253D0%25C2%25A6field%253D19%25C2%25A6value%253D1%25252C2%EF%BF%BDcomparison%253D3%25C2%25A6field%253D20%25C2%25A6value%253D100&sortOptions=sortField%3D4%C2%A6isAscending%3DFalse&limitTo=10&combination=1"
		);
		let unsorted = SeriesFilterV2Dto::default();
		assert_eq!(
			encode_series_filter(&unsorted),
			"entityType=Series&&&limitTo=0&combination=1"
		);
	}

	#[test]
	fn decode_round_trips_encode() {
		let decoded = decode_series_filter(&encode_series_filter(&sample())).unwrap();
		assert_eq!(decoded, sample());
	}

	#[test]
	fn decode_of_kavita_web_ui_filter() {
		// Decoded identically by kavita-ref 0.9.1.4 (library 1, sort name ascending).
		let encoded = "stmts=comparison%253D0%25C2%25A6field%253D19%25C2%25A6value%253D1&sortOptions=sortField%3D1%C2%A6isAscending%3DTrue&limitTo=0&combination=1";
		let decoded = decode_series_filter(encoded).unwrap();
		assert_eq!(decoded.statements.len(), 1);
		assert_eq!(
			decoded.statements[0].field,
			StatementField::Known(SeriesFilterField::Libraries)
		);
		assert_eq!(decoded.statements[0].value, "1");
		assert!(decoded.effective_sort().is_ascending);
		assert_eq!(decoded.combination, FilterCombination::And);
		assert_eq!(decoded.entity_type, FilterEntityType::Series);
	}

	#[test]
	fn decode_without_sort_field_falls_back_like_kavita() {
		let decoded = decode_series_filter(
			"sortOptions=isAscending%3DTrue&limitTo=0&combination=1",
		)
		.unwrap();
		let sort = decoded.sort_options.unwrap();
		assert_eq!(sort.sort_field, SeriesSortField::SortName);
		assert!(!sort.is_ascending);
		// kavita-ref answers an empty encoded filter with `sortOptions: null`.
		let empty = decode_series_filter("").unwrap();
		assert!(empty.statements.is_empty());
		assert_eq!(empty.sort_options, None);
	}

	#[test]
	fn body_accepts_null_sort_options_and_missing_fields() {
		let filter: SeriesFilterV2Dto = serde_json::from_str(
			r#"{"statements":[],"combination":1,"sortOptions":null,"limitTo":0}"#,
		)
		.unwrap();
		assert_eq!(filter.sort_options, None);
		assert_eq!(filter.effective_sort(), SeriesSortOptionDto::default());
		let filter: SeriesFilterV2Dto = serde_json::from_str("{}").unwrap();
		assert_eq!(filter.combination, FilterCombination::And);
		assert!(serde_json::from_str::<SeriesFilterV2Dto>(
			r#"{"statements":[{"comparison":99,"field":1,"value":"x"}]}"#
		)
		.is_err());
	}

	#[test]
	fn statement_value_parsers() {
		let statement = SeriesFilterStatementDto::new(
			FilterComparison::Contains,
			SeriesFilterField::Formats,
			"1, 3",
		);
		assert_eq!(statement.int_values().unwrap(), vec![1, 3]);
		assert_eq!(
			statement.formats().unwrap(),
			vec![MangaFormat::Archive, MangaFormat::Epub]
		);
		assert!(SeriesFilterStatementDto {
			value: "x".to_owned(),
			..statement
		}
		.int_values()
		.is_err());
	}
}
