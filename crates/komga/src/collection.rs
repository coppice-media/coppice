use crate::book::KomgaReadStatus;
use crate::common::KomgaAuthor;
use crate::common::{KomgaThumbnailId, PatchValue};
use crate::library::KomgaLibraryId;
use crate::search::SeriesCondition;
use crate::series::{KomgaSeriesId, KomgaSeriesStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaCollectionId(pub String);

impl KomgaCollectionId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl std::fmt::Display for KomgaCollectionId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaCollectionId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaCollectionId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaCollection {
	pub id: KomgaCollectionId,
	pub name: String,
	pub ordered: bool,
	pub series_ids: Vec<KomgaSeriesId>,
	pub created_date: DateTime<Utc>,
	pub last_modified_date: DateTime<Utc>,
	pub filtered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaCollectionThumbnail {
	pub id: KomgaThumbnailId,
	pub collection_id: KomgaCollectionId,
	pub r#type: String,
	pub selected: bool,
	pub media_type: String,
	pub file_size: i64,
	pub width: i32,
	pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaCollectionCreateRequest {
	pub name: String,
	pub ordered: bool,
	pub series_ids: Vec<KomgaSeriesId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaCollectionUpdateRequest {
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub name: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub ordered: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub series_ids: PatchValue<Vec<KomgaSeriesId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaCollectionQuery {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub library_ids: Option<Vec<KomgaLibraryId>>,
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
	pub authors: Option<Vec<KomgaAuthor>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub deleted: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub complete: Option<bool>,
}

// A collection search condition is the same series-condition wire format.  The
// alias keeps the protocol shape explicit without adding a second discriminator.
pub type KomgaCollectionSearch = SeriesCondition;
