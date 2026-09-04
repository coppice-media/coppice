use crate::book::{KomgaMediaStatus, KomgaReadStatus, MediaProfile};
use crate::collection::KomgaCollectionId;
use crate::library::KomgaLibraryId;
use crate::read_list::KomgaReadListId;
use crate::series::{KomgaSeriesId, KomgaSeriesStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operator")]
pub enum Equality<T> {
	#[serde(rename = "is")]
	Is { value: T },
	#[serde(rename = "isNot")]
	IsNot { value: T },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operator")]
pub enum EqualityNullable<T> {
	#[serde(rename = "is")]
	Is { value: T },
	#[serde(rename = "isNot")]
	IsNot { value: T },
	#[serde(rename = "isNull")]
	IsNull,
	#[serde(rename = "isNotNull")]
	IsNotNull,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operator")]
pub enum StringOperator {
	#[serde(rename = "is")]
	Is { value: String },
	#[serde(rename = "isNot")]
	IsNot { value: String },
	#[serde(rename = "contains")]
	Contains { value: String },
	#[serde(rename = "doesNotContain")]
	DoesNotContain { value: String },
	#[serde(rename = "beginsWith")]
	BeginsWith { value: String },
	#[serde(rename = "doesNotBeginWith")]
	DoesNotBeginWith { value: String },
	#[serde(rename = "endsWith")]
	EndsWith { value: String },
	#[serde(rename = "doesNotEndWith")]
	DoesNotEndWith { value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operator")]
pub enum Numeric<T> {
	#[serde(rename = "is")]
	Is { value: T },
	#[serde(rename = "isNot")]
	IsNot { value: T },
	#[serde(rename = "greaterThan")]
	GreaterThan { value: T },
	#[serde(rename = "lessThan")]
	LessThan { value: T },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operator")]
pub enum NumericNullable<T> {
	#[serde(rename = "is")]
	Is { value: T },
	#[serde(rename = "isNot")]
	IsNot { value: T },
	#[serde(rename = "greaterThan")]
	GreaterThan { value: T },
	#[serde(rename = "lessThan")]
	LessThan { value: T },
	#[serde(rename = "isNull")]
	IsNull,
	#[serde(rename = "isNotNull")]
	IsNotNull,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operator")]
pub enum DateOperator {
	#[serde(rename = "before")]
	Before {
		#[serde(rename = "dateTime")]
		date_time: DateTime<Utc>,
	},
	#[serde(rename = "after")]
	After {
		#[serde(rename = "dateTime")]
		date_time: DateTime<Utc>,
	},
	#[serde(rename = "isInTheLast")]
	IsInTheLast {
		#[serde(with = "kotlin_duration")]
		duration: Duration,
	},
	#[serde(rename = "isNotInTheLast")]
	IsNotInTheLast {
		#[serde(with = "kotlin_duration")]
		duration: Duration,
	},
	#[serde(rename = "isNull")]
	IsNull,
	#[serde(rename = "isNotNull")]
	IsNotNull,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operator")]
pub enum BooleanOperator {
	#[serde(rename = "isTrue")]
	IsTrue,
	#[serde(rename = "isFalse")]
	IsFalse,
}

pub type StringOp = StringOperator;
pub type Date = DateOperator;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum BookCondition {
	#[serde(rename = "AnyOfBook")]
	AnyOfBook {
		#[serde(rename = "anyOf", default)]
		conditions: Vec<BookCondition>,
	},
	#[serde(rename = "AllOfBook")]
	AllOfBook {
		#[serde(rename = "allOf", default)]
		conditions: Vec<BookCondition>,
	},
	#[serde(rename = "LibraryId")]
	LibraryId {
		#[serde(rename = "libraryId")]
		operator: Equality<KomgaLibraryId>,
	},
	#[serde(rename = "ReadListId")]
	ReadListId {
		#[serde(rename = "readListId")]
		operator: Equality<KomgaReadListId>,
	},
	#[serde(rename = "SeriesId")]
	SeriesId {
		#[serde(rename = "seriesId")]
		operator: Equality<KomgaSeriesId>,
	},
	#[serde(rename = "Deleted")]
	Deleted {
		#[serde(rename = "deleted")]
		operator: BooleanOperator,
	},
	#[serde(rename = "OneShot")]
	OneShot {
		#[serde(rename = "oneShot")]
		operator: BooleanOperator,
	},
	#[serde(rename = "Title")]
	Title {
		#[serde(rename = "title")]
		operator: StringOperator,
	},
	#[serde(rename = "ReleaseDate")]
	ReleaseDate {
		#[serde(rename = "releaseDate")]
		operator: DateOperator,
	},
	#[serde(rename = "NumberSort")]
	NumberSort {
		#[serde(rename = "numberSort")]
		operator: Numeric<f32>,
	},
	#[serde(rename = "Tag")]
	Tag {
		#[serde(rename = "tag")]
		operator: EqualityNullable<String>,
	},
	#[serde(rename = "ReadStatus")]
	ReadStatus {
		#[serde(rename = "readStatus")]
		operator: Equality<KomgaReadStatus>,
	},
	#[serde(rename = "MediaStatus")]
	MediaStatus {
		#[serde(rename = "mediaStatus")]
		operator: Equality<KomgaMediaStatus>,
	},
	#[serde(rename = "MediaProfile")]
	MediaProfile {
		#[serde(rename = "mediaProfile")]
		operator: Equality<MediaProfile>,
	},
	#[serde(rename = "Author")]
	Author {
		#[serde(rename = "author")]
		operator: Equality<AuthorMatch>,
	},
	#[serde(rename = "Poster")]
	Poster {
		#[serde(rename = "poster")]
		operator: Equality<PosterMatch>,
	},
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum SeriesCondition {
	#[serde(rename = "AnyOfSeries")]
	AnyOfSeries {
		#[serde(rename = "anyOf", default)]
		conditions: Vec<SeriesCondition>,
	},
	#[serde(rename = "AllOfSeries")]
	AllOfSeries {
		#[serde(rename = "allOf", default)]
		conditions: Vec<SeriesCondition>,
	},
	#[serde(rename = "LibraryId")]
	LibraryId {
		#[serde(rename = "libraryId")]
		operator: Equality<KomgaLibraryId>,
	},
	#[serde(rename = "CollectionId")]
	CollectionId {
		#[serde(rename = "collectionId")]
		operator: Equality<KomgaCollectionId>,
	},
	#[serde(rename = "Deleted")]
	Deleted {
		#[serde(rename = "deleted")]
		operator: BooleanOperator,
	},
	#[serde(rename = "Complete")]
	Complete {
		#[serde(rename = "complete")]
		operator: BooleanOperator,
	},
	#[serde(rename = "OneShot")]
	OneShot {
		#[serde(rename = "oneShot")]
		operator: BooleanOperator,
	},
	#[serde(rename = "Title")]
	Title {
		#[serde(rename = "title")]
		operator: StringOperator,
	},
	#[serde(rename = "TitleSort")]
	TitleSort {
		#[serde(rename = "titleSort")]
		operator: StringOperator,
	},
	#[serde(rename = "ReleaseDate")]
	ReleaseDate {
		#[serde(rename = "releaseDate")]
		operator: DateOperator,
	},
	#[serde(rename = "Tag")]
	Tag {
		#[serde(rename = "tag")]
		operator: EqualityNullable<String>,
	},
	#[serde(rename = "SharingLabel")]
	SharingLabel {
		#[serde(rename = "sharingLabel")]
		operator: EqualityNullable<String>,
	},
	#[serde(rename = "Publisher")]
	Publisher {
		#[serde(rename = "publisher")]
		operator: Equality<String>,
	},
	#[serde(rename = "Language")]
	Language {
		#[serde(rename = "language")]
		operator: Equality<String>,
	},
	#[serde(rename = "Genre")]
	Genre {
		#[serde(rename = "genre")]
		operator: EqualityNullable<String>,
	},
	#[serde(rename = "AgeRating")]
	AgeRating {
		#[serde(rename = "ageRating")]
		operator: NumericNullable<i32>,
	},
	#[serde(rename = "ReadStatus")]
	ReadStatus {
		#[serde(rename = "readStatus")]
		operator: Equality<KomgaReadStatus>,
	},
	#[serde(rename = "SeriesStatus")]
	SeriesStatus {
		#[serde(rename = "seriesStatus")]
		operator: Equality<KomgaSeriesStatus>,
	},
	#[serde(rename = "Author")]
	Author {
		#[serde(rename = "author")]
		operator: Equality<AuthorMatch>,
	},
}

fn parse_condition_field<T, E>(value: serde_json::Value, field: &str) -> Result<T, E>
where
	T: serde::de::DeserializeOwned,
	E: serde::de::Error,
{
	serde_json::from_value(value)
		.map_err(|error| E::custom(format!("invalid {field} condition: {error}")))
}

fn parse_book_condition<E>(value: serde_json::Value) -> Result<BookCondition, E>
where
	E: serde::de::Error,
{
	let serde_json::Value::Object(mut object) = value else {
		return Err(E::custom("book condition must be an object"));
	};

	let distinguishing_keys = [
		"anyOf",
		"allOf",
		"libraryId",
		"readListId",
		"seriesId",
		"deleted",
		"oneShot",
		"title",
		"releaseDate",
		"numberSort",
		"tag",
		"readStatus",
		"mediaStatus",
		"mediaProfile",
		"author",
		"poster",
	];
	let unknown_key = object
		.keys()
		.find(|key| *key != "type" && !distinguishing_keys.contains(&key.as_str()));
	if let Some(key) = unknown_key {
		return Err(E::custom(format!("unknown book condition field: {key}")));
	}
	let present_keys = distinguishing_keys
		.iter()
		.filter(|key| object.contains_key(**key))
		.collect::<Vec<_>>();
	if present_keys.is_empty() {
		let Some(type_value) = object.get("type") else {
			return Err(E::custom(
				"book condition must contain a distinguishing field",
			));
		};
		let actual_type: String = parse_condition_field(type_value.clone(), "type")?;
		return match actual_type.as_str() {
			"AnyOfBook" => Ok(BookCondition::AnyOfBook {
				conditions: Vec::new(),
			}),
			"AllOfBook" => Ok(BookCondition::AllOfBook {
				conditions: Vec::new(),
			}),
			_ => Err(E::custom(format!(
				"book condition type {actual_type} requires a distinguishing field"
			))),
		};
	}
	if present_keys.len() > 1 {
		return Err(E::custom(format!(
			"book condition must contain exactly one distinguishing field, found {}",
			present_keys.len()
		)));
	}
	let key = *present_keys[0];
	let expected_type = match key {
		"anyOf" => "AnyOfBook",
		"allOf" => "AllOfBook",
		"libraryId" => "LibraryId",
		"readListId" => "ReadListId",
		"seriesId" => "SeriesId",
		"deleted" => "Deleted",
		"oneShot" => "OneShot",
		"title" => "Title",
		"releaseDate" => "ReleaseDate",
		"numberSort" => "NumberSort",
		"tag" => "Tag",
		"readStatus" => "ReadStatus",
		"mediaStatus" => "MediaStatus",
		"mediaProfile" => "MediaProfile",
		"author" => "Author",
		"poster" => "Poster",
		_ => unreachable!(),
	};
	if let Some(type_value) = object.get("type") {
		let actual_type: String = parse_condition_field(type_value.clone(), "type")?;
		if actual_type != expected_type {
			return Err(E::custom(format!(
				"book condition field {key} contradicts type {actual_type}"
			)));
		}
	}
	let payload = object
		.remove(key)
		.expect("distinguishing field was present");
	Ok(match key {
		"anyOf" => BookCondition::AnyOfBook {
			conditions: parse_condition_field(payload, key)?,
		},
		"allOf" => BookCondition::AllOfBook {
			conditions: parse_condition_field(payload, key)?,
		},
		"libraryId" => BookCondition::LibraryId {
			operator: parse_condition_field(payload, key)?,
		},
		"readListId" => BookCondition::ReadListId {
			operator: parse_condition_field(payload, key)?,
		},
		"seriesId" => BookCondition::SeriesId {
			operator: parse_condition_field(payload, key)?,
		},
		"deleted" => BookCondition::Deleted {
			operator: parse_condition_field(payload, key)?,
		},
		"oneShot" => BookCondition::OneShot {
			operator: parse_condition_field(payload, key)?,
		},
		"title" => BookCondition::Title {
			operator: parse_condition_field(payload, key)?,
		},
		"releaseDate" => BookCondition::ReleaseDate {
			operator: parse_condition_field(payload, key)?,
		},
		"numberSort" => BookCondition::NumberSort {
			operator: parse_condition_field(payload, key)?,
		},
		"tag" => BookCondition::Tag {
			operator: parse_condition_field(payload, key)?,
		},
		"readStatus" => BookCondition::ReadStatus {
			operator: parse_condition_field(payload, key)?,
		},
		"mediaStatus" => BookCondition::MediaStatus {
			operator: parse_condition_field(payload, key)?,
		},
		"mediaProfile" => BookCondition::MediaProfile {
			operator: parse_condition_field(payload, key)?,
		},
		"author" => BookCondition::Author {
			operator: parse_condition_field(payload, key)?,
		},
		"poster" => BookCondition::Poster {
			operator: parse_condition_field(payload, key)?,
		},
		_ => unreachable!(),
	})
}

fn parse_series_condition<E>(value: serde_json::Value) -> Result<SeriesCondition, E>
where
	E: serde::de::Error,
{
	let serde_json::Value::Object(mut object) = value else {
		return Err(E::custom("series condition must be an object"));
	};

	let distinguishing_keys = [
		"anyOf",
		"allOf",
		"libraryId",
		"collectionId",
		"deleted",
		"complete",
		"oneShot",
		"title",
		"titleSort",
		"releaseDate",
		"tag",
		"sharingLabel",
		"publisher",
		"language",
		"genre",
		"ageRating",
		"readStatus",
		"seriesStatus",
		"author",
	];
	let unknown_key = object
		.keys()
		.find(|key| *key != "type" && !distinguishing_keys.contains(&key.as_str()));
	if let Some(key) = unknown_key {
		return Err(E::custom(format!("unknown series condition field: {key}")));
	}
	let present_keys = distinguishing_keys
		.iter()
		.filter(|key| object.contains_key(**key))
		.collect::<Vec<_>>();
	if present_keys.is_empty() {
		let Some(type_value) = object.get("type") else {
			return Err(E::custom(
				"series condition must contain a distinguishing field",
			));
		};
		let actual_type: String = parse_condition_field(type_value.clone(), "type")?;
		return match actual_type.as_str() {
			"AnyOfSeries" => Ok(SeriesCondition::AnyOfSeries {
				conditions: Vec::new(),
			}),
			"AllOfSeries" => Ok(SeriesCondition::AllOfSeries {
				conditions: Vec::new(),
			}),
			_ => Err(E::custom(format!(
				"series condition type {actual_type} requires a distinguishing field"
			))),
		};
	}
	if present_keys.len() > 1 {
		return Err(E::custom(format!(
			"series condition must contain exactly one distinguishing field, found {}",
			present_keys.len()
		)));
	}
	let key = *present_keys[0];
	let expected_type = match key {
		"anyOf" => "AnyOfSeries",
		"allOf" => "AllOfSeries",
		"libraryId" => "LibraryId",
		"collectionId" => "CollectionId",
		"deleted" => "Deleted",
		"complete" => "Complete",
		"oneShot" => "OneShot",
		"title" => "Title",
		"titleSort" => "TitleSort",
		"releaseDate" => "ReleaseDate",
		"tag" => "Tag",
		"sharingLabel" => "SharingLabel",
		"publisher" => "Publisher",
		"language" => "Language",
		"genre" => "Genre",
		"ageRating" => "AgeRating",
		"readStatus" => "ReadStatus",
		"seriesStatus" => "SeriesStatus",
		_ => unreachable!(),
	};
	if let Some(type_value) = object.get("type") {
		let actual_type: String = parse_condition_field(type_value.clone(), "type")?;
		if actual_type != expected_type {
			return Err(E::custom(format!(
				"series condition field {key} contradicts type {actual_type}"
			)));
		}
	}
	let payload = object
		.remove(key)
		.expect("distinguishing field was present");
	Ok(match key {
		"anyOf" => SeriesCondition::AnyOfSeries {
			conditions: parse_condition_field(payload, key)?,
		},
		"allOf" => SeriesCondition::AllOfSeries {
			conditions: parse_condition_field(payload, key)?,
		},
		"libraryId" => SeriesCondition::LibraryId {
			operator: parse_condition_field(payload, key)?,
		},
		"collectionId" => SeriesCondition::CollectionId {
			operator: parse_condition_field(payload, key)?,
		},
		"deleted" => SeriesCondition::Deleted {
			operator: parse_condition_field(payload, key)?,
		},
		"complete" => SeriesCondition::Complete {
			operator: parse_condition_field(payload, key)?,
		},
		"oneShot" => SeriesCondition::OneShot {
			operator: parse_condition_field(payload, key)?,
		},
		"title" => SeriesCondition::Title {
			operator: parse_condition_field(payload, key)?,
		},
		"titleSort" => SeriesCondition::TitleSort {
			operator: parse_condition_field(payload, key)?,
		},
		"releaseDate" => SeriesCondition::ReleaseDate {
			operator: parse_condition_field(payload, key)?,
		},
		"tag" => SeriesCondition::Tag {
			operator: parse_condition_field(payload, key)?,
		},
		"sharingLabel" => SeriesCondition::SharingLabel {
			operator: parse_condition_field(payload, key)?,
		},
		"publisher" => SeriesCondition::Publisher {
			operator: parse_condition_field(payload, key)?,
		},
		"language" => SeriesCondition::Language {
			operator: parse_condition_field(payload, key)?,
		},
		"genre" => SeriesCondition::Genre {
			operator: parse_condition_field(payload, key)?,
		},
		"ageRating" => SeriesCondition::AgeRating {
			operator: parse_condition_field(payload, key)?,
		},
		"readStatus" => SeriesCondition::ReadStatus {
			operator: parse_condition_field(payload, key)?,
		},
		"seriesStatus" => SeriesCondition::SeriesStatus {
			operator: parse_condition_field(payload, key)?,
		},
		"author" => SeriesCondition::Author {
			operator: parse_condition_field(payload, key)?,
		},
		_ => unreachable!(),
	})
}

impl<'de> Deserialize<'de> for BookCondition {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		parse_book_condition(serde_json::Value::deserialize(deserializer)?)
	}
}

impl<'de> Deserialize<'de> for SeriesCondition {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		parse_series_condition(serde_json::Value::deserialize(deserializer)?)
	}
}

impl BookCondition {
	pub fn any_of(conditions: impl IntoIterator<Item = Self>) -> Self {
		Self::AnyOfBook {
			conditions: conditions.into_iter().collect(),
		}
	}

	pub fn all_of(conditions: impl IntoIterator<Item = Self>) -> Self {
		Self::AllOfBook {
			conditions: conditions.into_iter().collect(),
		}
	}
}

impl SeriesCondition {
	pub fn any_of(conditions: impl IntoIterator<Item = Self>) -> Self {
		Self::AnyOfSeries {
			conditions: conditions.into_iter().collect(),
		}
	}

	pub fn all_of(conditions: impl IntoIterator<Item = Self>) -> Self {
		Self::AllOfSeries {
			conditions: conditions.into_iter().collect(),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorMatch {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PosterMatch {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub r#type: Option<PosterMatchType>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub selected: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PosterMatchType {
	Generated,
	Sidecar,
	UserUploaded,
}

pub type AnyOfBook = BookCondition;
pub type AllOfBook = BookCondition;
pub type AnyOfSeries = SeriesCondition;
pub type AllOfSeries = SeriesCondition;
pub type LibraryId = BookCondition;
pub type CollectionId = SeriesCondition;
pub type ReadListId = BookCondition;
pub type SeriesId = BookCondition;
pub type Deleted = BookCondition;
pub type Complete = SeriesCondition;
pub type OneShot = BookCondition;
pub type Title = BookCondition;
pub type TitleSort = SeriesCondition;
pub type ReleaseDate = BookCondition;
pub type NumberSort = BookCondition;
pub type Tag = BookCondition;
pub type SharingLabel = SeriesCondition;
pub type Publisher = SeriesCondition;
pub type Language = SeriesCondition;
pub type Genre = SeriesCondition;
pub type AgeRating = SeriesCondition;
pub type ReadStatus = BookCondition;
pub type MediaStatus = BookCondition;
pub type MediaProfileCondition = BookCondition;
pub type Author = BookCondition;
pub type Poster = BookCondition;
pub type Is<T> = Equality<T>;
pub type IsNot<T> = Equality<T>;
pub type GreaterThan<T> = Numeric<T>;
pub type LessThan<T> = Numeric<T>;
pub type IsTrue = BooleanOperator;
pub type IsFalse = BooleanOperator;
pub type IsNull = EqualityNullable<String>;
pub type IsNotNull = EqualityNullable<String>;
pub type IsNullT<T> = NumericNullable<T>;
pub type IsNotNullT<T> = NumericNullable<T>;
pub type After = DateOperator;
pub type Before = DateOperator;
pub type IsInTheLast = DateOperator;
pub type IsNotInTheLast = DateOperator;

mod kotlin_duration {
	use super::*;

	pub fn serialize<S>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_str(&format_duration(*value))
	}

	pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
	where
		D: Deserializer<'de>,
	{
		let value = String::deserialize(deserializer)?;
		parse_duration(&value).map_err(serde::de::Error::custom)
	}

	fn format_duration(value: Duration) -> String {
		if value.is_zero() {
			return "PT0S".to_owned();
		}
		let mut seconds = value.as_secs();
		let nanos = value.subsec_nanos();
		let days = seconds / 86_400;
		seconds %= 86_400;
		let hours = seconds / 3_600;
		seconds %= 3_600;
		let minutes = seconds / 60;
		seconds %= 60;
		let mut output = String::from("P");
		if days > 0 {
			output.push_str(&days.to_string());
			output.push('D');
		}
		if hours > 0 || minutes > 0 || seconds > 0 || nanos > 0 {
			output.push('T');
			if hours > 0 {
				output.push_str(&hours.to_string());
				output.push('H');
			}
			if minutes > 0 {
				output.push_str(&minutes.to_string());
				output.push('M');
			}
			if nanos > 0 {
				let mut fraction = format!("{nanos:09}");
				while fraction.ends_with('0') {
					fraction.pop();
				}
				output.push_str(&seconds.to_string());
				output.push('.');
				output.push_str(&fraction);
				output.push('S');
			} else if seconds > 0 {
				output.push_str(&seconds.to_string());
				output.push('S');
			}
		}
		if !output.contains('T') {
			output.push_str("T0S");
		}
		output
	}

	fn parse_duration(value: &str) -> Result<Duration, String> {
		let value = value
			.strip_prefix('P')
			.ok_or("duration must start with P")?;
		let (date, time) = value.split_once('T').unwrap_or((value, ""));
		let days = parse_component(date, 'D')?.unwrap_or(0.0);
		let hours = parse_component(time, 'H')?.unwrap_or(0.0);
		let minutes = parse_component(time, 'M')?.unwrap_or(0.0);
		let seconds = parse_component(time, 'S')?.unwrap_or(0.0);
		if [days, hours, minutes, seconds]
			.iter()
			.any(|part| *part < 0.0)
		{
			return Err("negative durations are not supported".to_owned());
		}
		let total_nanos = days * 86_400.0 * 1e9
			+ hours * 3_600.0 * 1e9
			+ minutes * 60.0 * 1e9
			+ seconds * 1e9;
		if !total_nanos.is_finite() || total_nanos < 0.0 {
			return Err("invalid duration".to_owned());
		}
		Ok(Duration::from_nanos(total_nanos.round() as u64))
	}

	fn parse_component(value: &str, suffix: char) -> Result<Option<f64>, String> {
		let Some(index) = value.find(suffix) else {
			return Ok(None);
		};
		let number = &value[..index];
		if number.is_empty() {
			return Err("duration component is missing a number".to_owned());
		}
		number
			.parse::<f64>()
			.map(Some)
			.map_err(|_| "invalid duration component".to_owned())
	}
}

#[cfg(test)]
mod tests {
	use super::{BookCondition, Equality, SeriesCondition};
	use crate::book::{KomgaMediaStatus, KomgaReadStatus, MediaProfile};
	use crate::library::KomgaLibraryId;
	use crate::series::KomgaSeriesStatus;
	use serde_json::json;

	#[test]
	fn liseur_untagged_book_condition_decodes() {
		let liseur = json!({
			"condition": {
				"allOf": [
					{"mediaProfile": {"operator": "is", "value": "EPUB"}},
					{"mediaStatus": {"operator": "is", "value": "READY"}}
				]
			}
		});
		let expected = BookCondition::all_of([
			BookCondition::MediaProfile {
				operator: Equality::Is {
					value: MediaProfile::Epub,
				},
			},
			BookCondition::MediaStatus {
				operator: Equality::Is {
					value: KomgaMediaStatus::Ready,
				},
			},
		]);
		assert_eq!(
			serde_json::from_value::<BookCondition>(liseur["condition"].clone()).unwrap(),
			expected
		);
	}

	#[test]
	fn tagged_and_untagged_book_conditions_decode_identically() {
		let untagged = json!({
			"allOf": [
				{"mediaProfile": {"operator": "is", "value": "EPUB"}},
				{"mediaStatus": {"operator": "is", "value": "READY"}}
			]
		});
		let tagged = json!({
			"type": "AllOfBook",
			"allOf": [
				{"type": "MediaProfile", "mediaProfile": {"operator": "is", "value": "EPUB"}},
				{"type": "MediaStatus", "mediaStatus": {"operator": "is", "value": "READY"}}
			]
		});
		assert_eq!(
			serde_json::from_value::<BookCondition>(untagged).unwrap(),
			serde_json::from_value::<BookCondition>(tagged).unwrap()
		);
	}

	#[test]
	fn untagged_nested_series_conditions_decode() {
		let value = json!({
			"allOf": [{
				"anyOf": [
					{"title": {"operator": "is", "value": "Discworld"}},
					{"seriesStatus": {"operator": "is", "value": "ONGOING"}}
				]
			}]
		});
		let expected = SeriesCondition::all_of([SeriesCondition::any_of([
			SeriesCondition::Title {
				operator: super::StringOperator::Is {
					value: "Discworld".to_owned(),
				},
			},
			SeriesCondition::SeriesStatus {
				operator: Equality::Is {
					value: KomgaSeriesStatus::Ongoing,
				},
			},
		])]);
		assert_eq!(
			serde_json::from_value::<SeriesCondition>(value).unwrap(),
			expected
		);
	}

	#[test]
	fn invalid_condition_discriminators_error() {
		let invalid = [
			json!({}),
			json!({
				"title": {"operator": "is", "value": "Discworld"},
				"deleted": {"operator": "isTrue"}
			}),
			json!({
				"type": "Title",
				"deleted": {"operator": "isTrue"}
			}),
			json!({"type": "MediaProfile"}),
		];
		for value in invalid {
			assert!(serde_json::from_value::<BookCondition>(value).is_err());
		}
	}

	#[test]
	fn tagged_empty_composite_conditions_decode() {
		assert_eq!(
			serde_json::from_value::<BookCondition>(json!({"type": "AllOfBook"}))
				.unwrap(),
			BookCondition::AllOfBook {
				conditions: Vec::new()
			}
		);
		assert_eq!(
			serde_json::from_value::<BookCondition>(json!({"type": "AnyOfBook"}))
				.unwrap(),
			BookCondition::AnyOfBook {
				conditions: Vec::new()
			}
		);
		assert_eq!(
			serde_json::from_value::<SeriesCondition>(json!({"type": "AllOfSeries"}))
				.unwrap(),
			SeriesCondition::AllOfSeries {
				conditions: Vec::new()
			}
		);
		assert_eq!(
			serde_json::from_value::<SeriesCondition>(json!({"type": "AnyOfSeries"}))
				.unwrap(),
			SeriesCondition::AnyOfSeries {
				conditions: Vec::new()
			}
		);
	}

	#[test]
	fn conditions_preserve_nested_type_and_operator_discriminators() {
		let condition = BookCondition::all_of([BookCondition::ReadStatus {
			operator: Equality::Is {
				value: KomgaReadStatus::InProgress,
			},
		}]);
		let serialized = serde_json::to_value(&condition).unwrap();
		assert_eq!(
			serialized,
			json!({
				"type": "AllOfBook",
				"allOf": [{
					"type": "ReadStatus",
					"readStatus": {"operator": "is", "value": "IN_PROGRESS"}
				}]
			})
		);
		assert_eq!(
			serde_json::from_value::<BookCondition>(serialized).unwrap(),
			condition
		);
	}

	#[test]
	fn series_condition_uses_camel_case_field_names() {
		let condition = SeriesCondition::LibraryId {
			operator: Equality::Is {
				value: KomgaLibraryId::from("library"),
			},
		};
		let serialized = serde_json::to_value(&condition).unwrap();
		assert_eq!(
			serialized,
			json!({
				"type": "LibraryId",
				"libraryId": {"operator": "is", "value": "library"}
			})
		);
		assert_eq!(
			serde_json::from_value::<SeriesCondition>(serialized).unwrap(),
			condition
		);
	}
}
