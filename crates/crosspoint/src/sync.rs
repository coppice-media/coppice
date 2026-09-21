//! Lossless CrossPoint rich-sync wire values and validation limits.
//!
//! The field names intentionally mirror crosspoint-sync's `/api/v1` contract.
//! Device ids in these values are advisory input only; the server must replace
//! them with the device bound to the path credential before persisting.

use serde::{Deserialize, Serialize};

/// Maximum items accepted by bookmark, clipping, and document batches.
pub const MAX_BATCH_ITEMS: usize = 50;
/// Maximum items accepted by the per-book statistics batch.
pub const MAX_STATS_BOOK_BATCH: usize = 20;
/// Maximum page size for bookmark/clipping delta reads.
pub const MAX_DELTA_PAGE: u64 = 100;
/// Maximum progress/document discovery page size.
pub const MAX_DOCUMENTS_PAGE: u64 = 500;
/// Maximum UTF-8 bytes in a progress string.
pub const MAX_PROGRESS_BYTES: usize = 4096;
/// Maximum UTF-8 bytes in a document field.
pub const MAX_DOCUMENT_BYTES: usize = 64;
/// Maximum UTF-8 bytes in a bookmark x-path.
pub const MAX_POSITION_XPATH_BYTES: usize = 120;
/// Maximum UTF-8 bytes in a position anchor.
pub const MAX_POSITION_ANCHOR_BYTES: usize = 48;
/// Maximum UTF-8 bytes in a bookmark summary.
pub const MAX_SUMMARY_BYTES: usize = 256;
/// Maximum UTF-8 bytes in a clipping text.
pub const MAX_CLIPPING_TEXT_BYTES: usize = 2048;
/// Maximum UTF-8 bytes in a clipping note.
pub const MAX_CLIPPING_NOTE_BYTES: usize = 4096;
/// Maximum clipping chapter length in Unicode scalar values.
pub const MAX_CLIPPING_CHAPTER_CHARS: usize = 64;

/// A CrossPoint compact EPUB position. Page values are layout hints; `pct_q`,
/// `spine`, and portable anchors remain useful across device settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
	#[serde(rename = "pctQ")]
	pub pct_q: u32,
	pub spine: u16,
	pub page: u16,
	pub pages: u16,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub para: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub li: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub anchor: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub xpath: Option<String>,
}

impl Position {
	/// Validate the bounded wire representation.
	pub fn is_valid(&self) -> bool {
		self.pct_q <= 1_000_000
			&& self
				.anchor
				.as_deref()
				.is_none_or(|v| v.len() <= MAX_POSITION_ANCHOR_BYTES)
			&& self
				.xpath
				.as_deref()
				.is_none_or(|v| v.len() <= MAX_POSITION_XPATH_BYTES)
	}
}

/// The optional metadata object accepted by progress PUTs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentMetadata {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub filename: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub authors: Option<String>,
	/// Connector sidecar ids, e.g. `bookfusion_id`.
	#[serde(flatten)]
	pub external_ids: std::collections::BTreeMap<String, String>,
}

impl DocumentMetadata {
	/// Returns false when every field is absent/empty or a field exceeds the
	/// protocol's 512-byte metadata bound.
	pub fn is_valid(&self) -> bool {
		self.filename
			.as_deref()
			.into_iter()
			.chain(self.title.as_deref())
			.chain(self.authors.as_deref())
			.chain(self.external_ids.values().map(String::as_str))
			.all(|value| !value.is_empty() && value.len() <= 512)
	}
}

/// KOSync-compatible progress plus an optional lossless position/metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Progress {
	pub document: String,
	pub progress: String,
	pub percentage: f32,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device_id: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub position: Option<Position>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub metadata: Option<DocumentMetadata>,
}

impl Progress {
	/// Validate fields that are independent of authentication and media
	/// visibility. Invalid optional position values are ignored by the
	/// server, as required by the upstream contract.
	pub fn is_valid(&self) -> bool {
		!self.document.is_empty()
			&& self.document.len() <= MAX_DOCUMENT_BYTES
			&& self.document.bytes().all(|byte| {
				byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
			}) && !self.progress.is_empty()
			&& self.progress.len() <= MAX_PROGRESS_BYTES
			&& self.percentage.is_finite()
			&& (0.0..=1.0).contains(&self.percentage)
	}
}

/// A bookmark delta or tombstone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
	pub id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub xpath: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub percentage: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub summary: Option<String>,
	#[serde(rename = "si", default, skip_serializing_if = "Option::is_none")]
	pub spine_index: Option<u16>,
	#[serde(rename = "pc", default, skip_serializing_if = "Option::is_none")]
	pub paragraph_count: Option<u16>,
	#[serde(rename = "pp", default, skip_serializing_if = "Option::is_none")]
	pub paragraph_pos: Option<u16>,
	#[serde(default)]
	pub deleted: i32,
	#[serde(default)]
	pub updated_at: i64,
}

impl Bookmark {
	pub fn is_tombstone(&self) -> bool {
		self.deleted == 1
	}
}

/// A clipping/highlight delta or tombstone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clipping {
	pub id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub spine: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub start_page: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub end_page: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub pages: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub start_word: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub end_word: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub words: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub para: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub chapter: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub note: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub color: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub created_at: Option<u64>,
	#[serde(default)]
	pub deleted: i32,
	#[serde(default)]
	pub updated_at: i64,
}

impl Clipping {
	pub fn is_tombstone(&self) -> bool {
		self.deleted == 1
	}
}

/// A global reading-statistics snapshot owned by one device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalStats {
	pub device_id: String,
	#[serde(default)]
	pub device: String,
	#[serde(default, rename = "v")]
	pub version: u32,
	#[serde(default)]
	pub sessions: u64,
	#[serde(default)]
	pub seconds: u64,
	#[serde(default)]
	pub pages: u64,
	#[serde(default)]
	pub completed: u64,
	#[serde(default)]
	pub tod: Vec<u64>,
	#[serde(default)]
	pub dow: Vec<u64>,
	#[serde(default)]
	pub anchor_day: i64,
	#[serde(default)]
	pub history_b64: String,
	#[serde(default)]
	pub streak: u64,
}

/// A per-book reading-statistics snapshot owned by one device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsBook {
	pub document: String,
	#[serde(default, rename = "v")]
	pub version: u32,
	#[serde(default)]
	pub sessions: u64,
	#[serde(default)]
	pub seconds: u64,
	#[serde(default)]
	pub pages: u64,
	#[serde(default)]
	pub completed: bool,
	#[serde(default)]
	pub avg_fwd: u64,
	#[serde(default)]
	pub pace_n: u64,
	#[serde(default)]
	pub eta: u64,
	#[serde(default)]
	pub start_manual: bool,
	#[serde(default)]
	pub finish_manual: bool,
	#[serde(default)]
	pub start_date: u64,
	#[serde(default)]
	pub finished_date: u64,
	#[serde(default)]
	pub tod: Vec<u64>,
	#[serde(default)]
	pub dow: Vec<u64>,
}

/// Raw global stats response item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatsDevice {
	pub device_id: String,
	pub device: String,
	pub updated_at: i64,
	pub stats: GlobalStats,
}

/// Raw/combined per-book stats response item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatsBookDevice {
	pub device_id: String,
	pub updated_at: i64,
	pub stats: StatsBook,
}

/// Stable protocol-level validation errors for callers that do not use Axum.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RichSyncError {
	#[error("invalid request")]
	InvalidRequest,
	#[error("invalid document")]
	InvalidDocument,
	#[error("device identity does not match the credential")]
	DeviceMismatch,
}
