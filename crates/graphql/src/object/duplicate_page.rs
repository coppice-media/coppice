use async_graphql::{SimpleObject, ID};
use chrono::{DateTime, FixedOffset};
use models::{entity::known_duplicate_page, shared::enums::DuplicatePageAction};
use stump_core::ingest::quality::duplicate_pages_across_books::dhash_hex;

/// A librarian decision about one recurring page hash inside a library.
#[derive(Debug, Clone, SimpleObject)]
pub struct KnownDuplicatePage {
	pub library_id: ID,
	/// The 64-bit dHash as 16 lowercase hexadecimal digits.
	pub dhash: String,
	pub action: DuplicatePageAction,
	pub created_by: Option<String>,
	pub created_at: DateTime<FixedOffset>,
}

impl From<known_duplicate_page::Model> for KnownDuplicatePage {
	fn from(model: known_duplicate_page::Model) -> Self {
		Self {
			library_id: ID(model.library_id),
			dhash: dhash_hex(model.dhash),
			action: model.action,
			created_by: model.created_by,
			created_at: model.created_at,
		}
	}
}

/// One physical page carrying a candidate duplicate hash.
#[derive(Debug, Clone, SimpleObject)]
pub struct DuplicatePageOccurrence {
	pub media_id: ID,
	pub media_name: String,
	/// Physical 1-based page inside the file.
	pub page: i32,
	/// The 1-based page number clients currently see for this physical page
	/// once duplicate-page skipping is applied, or `null` when the page is
	/// hidden.
	pub visible_page: Option<i32>,
	/// The exact hash of this page; near-duplicates inside a candidate group
	/// may differ from the group's representative hash by a few bits.
	pub dhash: String,
}

/// A page hash that recurs across several books of one library and has not
/// been reviewed yet.
#[derive(Debug, Clone, SimpleObject)]
pub struct DuplicatePageCandidate {
	/// Representative hash of the group (the most frequent member).
	pub dhash: String,
	/// Distinct books containing a page from this group.
	pub book_count: i32,
	/// Total pages across those books.
	pub page_count: i32,
	pub occurrences: Vec<DuplicatePageOccurrence>,
}
