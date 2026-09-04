use crate::book::KomgaReadStatus;
use crate::collection::KomgaCollectionId;
use crate::common::{
	KomgaAuthor, KomgaReadingDirection, KomgaThumbnailId, KomgaWebLink, PatchValue,
};
use crate::library::KomgaLibraryId;

use crate::search::SeriesCondition;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaSeriesId(pub String);

impl KomgaSeriesId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl std::fmt::Display for KomgaSeriesId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaSeriesId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaSeriesId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeries {
	pub id: KomgaSeriesId,
	pub library_id: KomgaLibraryId,
	pub name: String,
	pub url: String,
	pub books_count: i32,
	pub books_read_count: i32,
	pub books_unread_count: i32,
	pub books_in_progress_count: i32,
	pub metadata: KomgaSeriesMetadata,
	pub deleted: bool,
	pub oneshot: bool,
	pub books_metadata: KomgaSeriesBookMetadata,
	pub created: DateTime<Utc>,
	pub last_modified: DateTime<Utc>,
	pub file_last_modified: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesMetadata {
	pub status: KomgaSeriesStatus,
	pub status_lock: bool,
	pub title: String,
	pub alternate_titles: Vec<KomgaAlternativeTitle>,
	pub alternate_titles_lock: bool,
	pub title_lock: bool,
	pub title_sort: String,
	pub title_sort_lock: bool,
	pub summary: String,
	pub summary_lock: bool,
	#[serde(deserialize_with = "deserialize_reading_direction")]
	pub reading_direction: Option<KomgaReadingDirection>,
	pub reading_direction_lock: bool,
	pub publisher: String,
	pub publisher_lock: bool,
	pub age_rating: Option<i32>,
	pub age_rating_lock: bool,
	pub language: Option<String>,
	pub language_lock: bool,
	pub genres: Vec<String>,
	pub genres_lock: bool,
	pub tags: Vec<String>,
	pub tags_lock: bool,
	pub total_book_count: Option<i32>,
	pub total_book_count_lock: bool,
	pub sharing_labels: Vec<String>,
	pub sharing_labels_lock: bool,
	pub links: Vec<KomgaWebLink>,
	pub links_lock: bool,
}

fn deserialize_reading_direction<'de, D>(
	deserializer: D,
) -> Result<Option<KomgaReadingDirection>, D::Error>
where
	D: Deserializer<'de>,
{
	let value = Option::<String>::deserialize(deserializer)?;
	match value.as_deref() {
		None | Some("") => Ok(None),
		Some("LEFT_TO_RIGHT") => Ok(Some(KomgaReadingDirection::LeftToRight)),
		Some("RIGHT_TO_LEFT") => Ok(Some(KomgaReadingDirection::RightToLeft)),
		Some("VERTICAL") => Ok(Some(KomgaReadingDirection::Vertical)),
		Some("WEBTOON") => Ok(Some(KomgaReadingDirection::Webtoon)),
		Some(value) => Err(serde::de::Error::custom(format!(
			"unknown Komga reading direction {value:?}"
		))),
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesBookMetadata {
	pub authors: Vec<KomgaAuthor>,
	pub tags: Vec<String>,
	pub release_date: Option<NaiveDate>,
	pub summary: String,
	pub summary_number: String,
	pub created: DateTime<Utc>,
	pub last_modified: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaAlternativeTitle {
	pub label: String,
	pub title: String,
}

/// Komga `ThumbnailSeries.Type` (`SIDECAR | USER_UPLOADED`). Unlike book
/// thumbnails there is no `GENERATED` variant: Komga never lists a generated
/// series cover, and Komelia decodes this field with an unguarded `valueOf`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KomgaSeriesThumbnailType {
	Sidecar,
	UserUploaded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesThumbnail {
	pub id: KomgaThumbnailId,
	pub series_id: KomgaSeriesId,
	pub r#type: KomgaSeriesThumbnailType,
	pub selected: bool,
	pub media_type: String,
	pub file_size: i64,
	pub width: i32,
	pub height: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KomgaSeriesStatus {
	Ended,
	Ongoing,
	Abandoned,
	Hiatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesMetadataUpdateRequest {
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub status: PatchValue<KomgaSeriesStatus>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub status_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub title: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub title_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub title_sort: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub title_sort_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub summary: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub summary_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub publisher: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub publisher_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub reading_direction: PatchValue<KomgaReadingDirection>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub reading_direction_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub age_rating: PatchValue<i32>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub age_rating_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub language: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub language_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub genres: PatchValue<Vec<String>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub genres_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub tags: PatchValue<Vec<String>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub tags_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub total_book_count: PatchValue<i32>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub total_book_count_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub sharing_labels: PatchValue<Vec<String>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub sharing_labels_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub links: PatchValue<Vec<KomgaWebLink>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub links_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub alternate_titles: PatchValue<Vec<KomgaAlternativeTitle>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub alternate_titles_lock: PatchValue<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesSearch {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub condition: Option<SeriesCondition>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub full_text_search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaSeriesQuery {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub search_term: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub search_regex: Option<SearchRegex>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub library_ids: Option<Vec<KomgaLibraryId>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub collection_ids: Option<Vec<KomgaCollectionId>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub status: Option<Vec<KomgaSeriesStatus>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub read_status: Option<Vec<KomgaReadStatus>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub publishers: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub languages: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub genres: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tags: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub age_ratings: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub release_years: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub sharing_labels: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub authors: Option<Vec<KomgaAuthor>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub deleted: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub complete: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub oneshot: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchRegex {
	pub regex: String,
	pub search_field: SearchField,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SearchField {
	Title,
	TitleSort,
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Komga's `ThumbnailSeries.Type` has no `GENERATED` variant, and Komelia
	/// decodes it with an unguarded `valueOf`; emitting one crashed every
	/// download at the series import step.
	#[test]
	fn series_thumbnail_type_matches_komga_enum_exactly() {
		assert_eq!(
			serde_json::to_string(&KomgaSeriesThumbnailType::Sidecar).unwrap(),
			"\"SIDECAR\""
		);
		assert_eq!(
			serde_json::to_string(&KomgaSeriesThumbnailType::UserUploaded).unwrap(),
			"\"USER_UPLOADED\""
		);
		assert!(
			serde_json::from_str::<KomgaSeriesThumbnailType>("\"GENERATED\"").is_err()
		);
	}

	use serde_json::json;

	#[test]
	fn blank_reading_direction_decodes_to_none() {
		let metadata: KomgaSeriesMetadata = serde_json::from_value(json!({
			"status": "ONGOING",
			"statusLock": false,
			"title": "Fixture",
			"alternateTitles": [],
			"alternateTitlesLock": false,
			"titleLock": false,
			"titleSort": "Fixture",
			"titleSortLock": false,
			"summary": "",
			"summaryLock": false,
			"readingDirection": "",
			"readingDirectionLock": false,
			"publisher": "",
			"publisherLock": false,
			"ageRating": null,
			"ageRatingLock": false,
			"language": "en",
			"languageLock": false,
			"genres": [],
			"genresLock": false,
			"tags": [],
			"tagsLock": false,
			"totalBookCount": null,
			"totalBookCountLock": false,
			"sharingLabels": [],
			"sharingLabelsLock": false,
			"links": [],
			"linksLock": false
		}))
		.unwrap();
		assert_eq!(metadata.status, KomgaSeriesStatus::Ongoing);
		assert!(metadata.reading_direction.is_none());
	}
}
