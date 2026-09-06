use crate::common::PatchValue;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct KomgaLibraryId(pub String);

impl KomgaLibraryId {
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}
}

impl fmt::Display for KomgaLibraryId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(f)
	}
}

impl From<String> for KomgaLibraryId {
	fn from(value: String) -> Self {
		Self(value)
	}
}

impl From<&str> for KomgaLibraryId {
	fn from(value: &str) -> Self {
		Self(value.to_owned())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaLibrary {
	pub id: KomgaLibraryId,
	pub name: String,
	pub root: String,
	pub import_comic_info_book: bool,
	pub import_comic_info_series: bool,
	pub import_comic_info_collection: bool,
	pub import_comic_info_read_list: bool,
	pub import_comic_info_series_append_volume: bool,
	pub import_epub_book: bool,
	pub import_epub_series: bool,
	pub import_mylar_series: bool,
	pub import_local_artwork: bool,
	pub import_barcode_isbn: bool,
	pub scan_force_modified_time: bool,
	pub scan_interval: ScanInterval,
	pub scan_on_startup: bool,
	pub scan_cbx: bool,
	pub scan_pdf: bool,
	pub scan_epub: bool,
	pub scan_directory_exclusions: Vec<String>,
	pub repair_extensions: bool,
	pub convert_to_cbz: bool,
	pub empty_trash_after_scan: bool,
	pub series_cover: SeriesCover,
	pub hash_files: bool,
	pub hash_pages: bool,
	pub hash_koreader: bool,
	pub analyze_dimensions: bool,
	pub oneshots_directory: Option<String>,
	pub unavailable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SeriesCover {
	First,
	FirstUnreadOrFirst,
	FirstUnreadOrLast,
	Last,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScanInterval {
	Disabled,
	Hourly,
	Every6h,
	Every12h,
	Daily,
	Weekly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaLibraryCreateRequest {
	pub name: String,
	pub root: String,
	#[serde(default = "default_true")]
	pub import_comic_info_book: bool,
	#[serde(default = "default_true")]
	pub import_comic_info_series: bool,
	#[serde(default = "default_true")]
	pub import_comic_info_collection: bool,
	#[serde(default = "default_true")]
	pub import_comic_info_read_list: bool,
	#[serde(default = "default_true")]
	pub import_comic_info_series_append_volume: bool,
	#[serde(default = "default_true")]
	pub import_epub_book: bool,
	#[serde(default = "default_true")]
	pub import_epub_series: bool,
	#[serde(default = "default_true")]
	pub import_mylar_series: bool,
	#[serde(default = "default_true")]
	pub import_local_artwork: bool,
	#[serde(default = "default_true")]
	pub import_barcode_isbn: bool,
	#[serde(default)]
	pub scan_force_modified_time: bool,
	#[serde(default = "default_scan_interval")]
	pub scan_interval: ScanInterval,
	#[serde(default)]
	pub scan_on_startup: bool,
	#[serde(default = "default_true")]
	pub scan_cbx: bool,
	#[serde(default = "default_true")]
	pub scan_pdf: bool,
	#[serde(default = "default_true")]
	pub scan_epub: bool,
	#[serde(default)]
	pub scan_directory_exclusions: Vec<String>,
	#[serde(default)]
	pub repair_extensions: bool,
	#[serde(default)]
	pub convert_to_cbz: bool,
	#[serde(default)]
	pub empty_trash_after_scan: bool,
	#[serde(default = "default_series_cover")]
	pub series_cover: SeriesCover,
	#[serde(default = "default_true")]
	pub hash_files: bool,
	#[serde(default)]
	pub hash_koreader: bool,
	#[serde(default)]
	pub hash_pages: bool,
	#[serde(default = "default_true")]
	pub analyze_dimensions: bool,
	#[serde(default)]
	pub oneshots_directory: Option<String>,
}

impl Default for KomgaLibraryCreateRequest {
	fn default() -> Self {
		Self {
			name: String::new(),
			root: String::new(),
			import_comic_info_book: true,
			import_comic_info_series: true,
			import_comic_info_collection: true,
			import_comic_info_read_list: true,
			import_comic_info_series_append_volume: true,
			import_epub_book: true,
			import_epub_series: true,
			import_mylar_series: true,
			import_local_artwork: true,
			import_barcode_isbn: true,
			scan_force_modified_time: false,
			scan_interval: ScanInterval::Every6h,
			scan_on_startup: false,
			scan_cbx: true,
			scan_pdf: true,
			scan_epub: true,
			scan_directory_exclusions: Vec::new(),
			repair_extensions: false,
			convert_to_cbz: false,
			empty_trash_after_scan: false,
			hash_files: true,
			hash_koreader: false,
			hash_pages: false,
			analyze_dimensions: true,
			series_cover: default_series_cover(),
			oneshots_directory: None,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KomgaLibraryUpdateRequest {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub root: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_comic_info_book: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_comic_info_series: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_comic_info_series_append_volume: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_comic_info_collection: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_comic_info_read_list: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_epub_book: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_epub_series: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_mylar_series: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_local_artwork: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub import_barcode_isbn: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scan_force_modified_time: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub repair_extensions: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub convert_to_cbz: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub empty_trash_after_scan: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub series_cover: Option<SeriesCover>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub hash_files: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub hash_koreader: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub hash_pages: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub analyze_dimensions: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scan_on_startup: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scan_cbx: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scan_epub: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scan_pdf: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scan_interval: Option<ScanInterval>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub oneshots_directory: PatchValue<String>,
	#[serde(default, skip_serializing_if = "PatchValue::is_unset")]
	pub scan_directory_exclusions: PatchValue<Vec<String>>,
}

fn default_true() -> bool {
	true
}

fn default_scan_interval() -> ScanInterval {
	ScanInterval::Every6h
}

fn default_series_cover() -> SeriesCover {
	SeriesCover::First
}
