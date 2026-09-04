use crate::book::{KomgaBookId, KomgaMediaStatus, KomgaReadStatus};
use crate::common::{KomgaAuthor, KomgaThumbnailId, PatchValue};
use crate::library::KomgaLibraryId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaReadListId(pub String);

impl KomgaReadListId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl std::fmt::Display for KomgaReadListId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaReadListId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaReadListId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaReadList {
	pub id: KomgaReadListId,
	pub name: String,
	pub summary: String,
	pub ordered: bool,
	pub book_ids: Vec<KomgaBookId>,
	pub created_date: DateTime<Utc>,
	pub last_modified_date: DateTime<Utc>,
	pub filtered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaReadListThumbnail {
	pub id: KomgaThumbnailId,
	pub read_list_id: KomgaReadListId,
	pub r#type: String,
	pub selected: bool,
	pub media_type: String,
	pub file_size: i64,
	pub width: i32,
	pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaReadListCreateRequest {
	pub name: String,
	pub summary: String,
	pub ordered: bool,
	pub book_ids: Vec<KomgaBookId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaReadListUpdateRequest {
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub name: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub summary: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub ordered: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub book_ids: PatchValue<Vec<KomgaBookId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaReadListQuery {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub library_ids: Option<Vec<KomgaLibraryId>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub read_status: Option<Vec<KomgaReadStatus>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tags: Option<Vec<String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub media_status: Option<Vec<KomgaMediaStatus>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub deleted: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub authors: Option<Vec<KomgaAuthor>>,
}
