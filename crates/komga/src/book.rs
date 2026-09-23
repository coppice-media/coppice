use crate::common::{KomgaAuthor, KomgaThumbnailId, KomgaWebLink, PatchValue};
use crate::library::KomgaLibraryId;
use crate::search::BookCondition;
use crate::series::KomgaSeriesId;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaBookId(pub String);

impl KomgaBookId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl std::fmt::Display for KomgaBookId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaBookId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaBookId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBook {
	pub id: KomgaBookId,
	pub series_id: KomgaSeriesId,
	pub series_title: String,
	pub library_id: KomgaLibraryId,
	pub name: String,
	pub url: String,
	pub number: i32,
	pub created: DateTime<Utc>,
	pub last_modified: DateTime<Utc>,
	pub file_last_modified: DateTime<Utc>,
	pub size_bytes: i64,
	pub size: String,
	pub media: Media,
	pub metadata: KomgaBookMetadata,
	pub read_progress: Option<ReadProgress>,
	pub deleted: bool,
	pub file_hash: String,
	pub oneshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBookMetadata {
	pub title: String,
	pub summary: String,
	pub number: String,
	pub number_sort: f32,
	pub release_date: Option<NaiveDate>,
	pub authors: Vec<KomgaAuthor>,
	pub tags: Vec<String>,
	pub isbn: String,
	pub links: Vec<KomgaWebLink>,
	pub title_lock: bool,
	pub summary_lock: bool,
	pub number_lock: bool,
	pub number_sort_lock: bool,
	pub release_date_lock: bool,
	pub authors_lock: bool,
	pub tags_lock: bool,
	pub isbn_lock: bool,
	pub links_lock: bool,
	pub created: DateTime<Utc>,
	pub last_modified: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBookPage {
	pub number: i32,
	pub file_name: String,
	pub media_type: String,
	#[serde(default)]
	pub width: Option<i32>,
	#[serde(default)]
	pub height: Option<i32>,
	#[serde(default)]
	pub size_bytes: Option<i64>,
	pub size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBookThumbnail {
	pub id: KomgaThumbnailId,
	pub book_id: KomgaBookId,
	pub r#type: String,
	pub selected: bool,
	pub media_type: String,
	pub file_size: i64,
	pub width: i32,
	pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Media {
	pub status: KomgaMediaStatus,
	pub media_type: Option<String>,
	pub pages_count: i32,
	pub comment: String,
	pub epub_divina_compatible: bool,
	pub epub_is_kepub: bool,
	#[serde(deserialize_with = "deserialize_media_profile")]
	pub media_profile: Option<MediaProfile>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MediaProfile {
	Divina,
	Pdf,
	Epub,
}

/// File extensions that the Komga API can expose as readable books.
pub const SUPPORTED_MEDIA_EXTENSIONS: &[&str] =
	&["epub", "pdf", "cbz", "cbr", "zip", "rar"];

/// Extensions that map to each Komga media profile.
pub fn extensions_for_media_profile(profile: MediaProfile) -> &'static [&'static str] {
	match profile {
		MediaProfile::Epub => &["epub"],
		MediaProfile::Pdf => &["pdf"],
		MediaProfile::Divina => &["cbz", "cbr", "zip", "rar"],
	}
}

/// Resolve a persisted extension to the Komga reader profile that can open it.
pub fn media_profile_for_extension(extension: &str) -> Option<MediaProfile> {
	let extension = extension.trim().to_ascii_lowercase();
	[MediaProfile::Epub, MediaProfile::Pdf, MediaProfile::Divina]
		.into_iter()
		.find(|profile| {
			extensions_for_media_profile(*profile).contains(&extension.as_str())
		})
}

fn deserialize_media_profile<'de, D>(
	deserializer: D,
) -> Result<Option<MediaProfile>, D::Error>
where
	D: Deserializer<'de>,
{
	let value = Option::<String>::deserialize(deserializer)?;
	Ok(value.and_then(|value| match value.as_str() {
		"DIVINA" => Some(MediaProfile::Divina),
		"PDF" => Some(MediaProfile::Pdf),
		"EPUB" => Some(MediaProfile::Epub),
		_ => None,
	}))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgress {
	pub page: i32,
	pub completed: bool,
	pub read_date: DateTime<Utc>,
	pub device_id: String,
	pub device_name: String,
	pub created: DateTime<Utc>,
	pub last_modified: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KomgaMediaStatus {
	Ready,
	Unknown,
	Error,
	Unsupported,
	Outdated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KomgaReadStatus {
	Unread,
	InProgress,
	Read,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CopyMode {
	Move,
	Copy,
	Hardlink,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBookMetadataUpdateRequest {
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub title: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub title_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub summary: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub summary_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub number: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub number_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub number_sort: PatchValue<f32>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub number_sort_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub release_date: PatchValue<NaiveDate>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub release_date_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub authors: PatchValue<Vec<KomgaAuthor>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub authors_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub tags: PatchValue<Vec<String>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub tags_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub isbn: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub isbn_lock: PatchValue<bool>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub links: PatchValue<Vec<KomgaWebLink>>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub links_lock: PatchValue<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBookReadProgressUpdateRequest {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub page: Option<i32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub completed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBookSearch {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub condition: Option<BookCondition>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub full_text_search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaBookQuery {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub search_term: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub library_ids: Option<Vec<KomgaLibraryId>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub media_status: Option<Vec<KomgaMediaStatus>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub read_status: Option<Vec<KomgaReadStatus>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub released_after: Option<NaiveDate>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tags: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
	use super::{
		KomgaBookReadProgressUpdateRequest, KomgaMediaStatus, Media, MediaProfile,
	};
	use serde_json::json;

	#[test]
	fn progress_update_uses_komga_camel_case_names() {
		let request = KomgaBookReadProgressUpdateRequest {
			page: Some(2),
			completed: Some(true),
		};
		assert_eq!(
			serde_json::to_value(request).unwrap(),
			json!({"page": 2, "completed": true})
		);
	}

	#[test]
	fn unknown_media_profile_is_the_nullable_decoder_fallback() {
		let media: Media = serde_json::from_value(json!({
			"status": "READY",
			"mediaType": null,
			"pagesCount": 0,
			"comment": "",
			"epubDivinaCompatible": false,
			"epubIsKepub": false,
			"mediaProfile": "FUTURE_PROFILE"
		}))
		.unwrap();
		assert_eq!(media.status, KomgaMediaStatus::Ready);
		assert_eq!(media.media_profile, None);
		assert_ne!(Some(MediaProfile::Pdf), media.media_profile);
	}
}
