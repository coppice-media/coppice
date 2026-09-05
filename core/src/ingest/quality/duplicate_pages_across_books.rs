use models::entity::{media, page_hash, series};
use sea_orm::{
	ColumnTrait, DatabaseConnection, EntityTrait, JoinType, QueryFilter, QuerySelect,
	RelationTrait,
};
use serde_json::{json, Value};
use stump_api_types::settings::{SettingDefinition, SettingKind, SettingValues};

use crate::ingest::contract::{
	BookSnapshot, IngestMediaKind, QualityCheck, QualityCheckError, QualityStatus,
};

use super::{disabled_outcome, enabled_setting, outcome, QUALITY_VERSION};

/// Default number of distinct books a page hash must appear in before the
/// check flags it. A page shared by one or two books is usually intentional
/// (a re-used cover, a credits page); three or more suggests scraped filler.
pub(crate) const MIN_DUPLICATE_BOOKS_DEFAULT: i64 = 3;

/// Upper bound on the duplicate groups recorded in evidence so reports stay
/// bounded regardless of how repetitive a library is.
const MAX_EVIDENCE_GROUPS: usize = 20;

static DUPLICATE_PAGES_SETTINGS: std::sync::LazyLock<Vec<SettingDefinition>> =
	std::sync::LazyLock::new(|| {
		vec![
			SettingDefinition {
				key: "enabled",
				label: "Enabled",
				description: "Run this quality check during staged analysis",
				kind: SettingKind::Bool,
				default: Value::Bool(true),
				required: false,
				secret: false,
				help_url: None,
			},
			SettingDefinition {
				key: "minBooks",
				label: "Minimum duplicate books",
				description:
					"Flag a page only when its hash appears in at least this many books",
				kind: SettingKind::Int,
				default: Value::Number(MIN_DUPLICATE_BOOKS_DEFAULT.into()),
				required: false,
				secret: false,
				help_url: None,
			},
		]
	});

fn min_books(settings: &SettingValues) -> i64 {
	settings
		.get("minBooks")
		.and_then(Value::as_i64)
		.unwrap_or(MIN_DUPLICATE_BOOKS_DEFAULT)
		.max(1)
}

/// Formats a stored `i64` dHash as the lowercase hexadecimal form used by
/// every duplicate-page API surface (quality evidence, GraphQL, editor).
pub(crate) fn dhash_hex(dhash: i64) -> String {
	format!("{:016x}", dhash as u64)
}

fn parse_dhash_hex(value: &str) -> Option<i64> {
	u64::from_str_radix(value.trim().trim_start_matches("0x"), 16)
		.ok()
		.map(|hash| hash as i64)
}

/// Flags staged books whose pages also appear in at least `minBooks` other
/// books of the same library, by perceptual dHash (`page_hashes` table).
///
/// Pages whose hash matches are listed in the evidence; the check never
/// mutates anything — hiding reviewed duplicates is the librarian's job via
/// `knownDuplicatePages`.
pub struct DuplicatePagesAcrossBooksCheck {
	conn: std::sync::Arc<DatabaseConnection>,
}

impl DuplicatePagesAcrossBooksCheck {
	pub fn new(conn: std::sync::Arc<DatabaseConnection>) -> Self {
		Self { conn }
	}
}

#[async_trait::async_trait]
impl QualityCheck for DuplicatePagesAcrossBooksCheck {
	fn id(&self) -> &'static str {
		"duplicate_pages_across_books"
	}

	fn name(&self) -> &'static str {
		"Duplicate pages across books"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		10
	}

	fn settings(&self) -> &[SettingDefinition] {
		&DUPLICATE_PAGES_SETTINGS
	}

	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}

		// Reflowable books have no real pages to hash; the analysis job does
		// not write page hashes for them either.
		if !book.media_kind.is_paged() || book.pages.is_empty() {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				json!({"reason": "not_paged"}),
			));
		}

		let own_hashes = own_page_hashes(book);
		if own_hashes.is_empty() {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				json!({"reason": "no_hashable_pages"}),
			));
		}

		let rows = page_hash::Entity::find()
			.find_also_related(media::Entity)
			.join(JoinType::InnerJoin, media::Relation::Series.def())
			.filter(series::Column::LibraryId.eq(book.library_id.clone()))
			// For library rework the snapshot's target id is the media row
			// under analysis; never report a book as its own duplicate. For
			// staged runs no media row carries a drop-item id, so this is a
			// no-op.
			.filter(page_hash::Column::MediaId.ne(book.drop_item_id.clone()))
			.filter(media::Column::DeletedAt.is_null())
			.all(self.conn.as_ref())
			.await
			.map_err(|error| QualityCheckError::Internal {
				check_id: self.id().to_string(),
				message: error.to_string(),
			})?;

		let min_books = min_books(settings);
		let tolerance = crate::filesystem::media::visible_pages::DUPLICATE_PAGE_TOLERANCE;

		let mut evidence_groups = Vec::new();
		let mut matched_books = std::collections::BTreeSet::new();
		for (own_page, own_hash) in own_hashes.iter() {
			let mut distinct = std::collections::BTreeSet::new();
			let mut books = Vec::new();
			for (hash, media) in rows.iter().filter_map(|(hash, media)| {
				media
					.as_ref()
					.map(|media| (hash, media))
					.filter(|(hash, _)| {
						stump_media::hamming(*own_hash as u64, hash.dhash as u64)
							<= tolerance
					})
			}) {
				if distinct.insert(hash.media_id.as_str()) {
					books.push(json!({
						"media_id": media.id,
						"name": media.name,
						"page": hash.page,
					}));
					matched_books.insert(media.id.clone());
				}
			}
			if (books.len() as i64) >= min_books {
				evidence_groups.push(json!({
					"dhash": dhash_hex(*own_hash),
					"own_page": own_page,
					"books": books,
				}));
			}
		}
		if evidence_groups.is_empty() {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Pass,
				1.0,
				json!({
					"tolerance": tolerance,
					"min_books": min_books,
					"hashed_pages": own_hashes.len(),
					"duplicate_groups": [],
					"matched_books": [],
					"truncated": false,
				}),
			));
		}

		evidence_groups.sort_by_key(|group| {
			group
				.get("books")
				.and_then(Value::as_array)
				.map(Vec::len)
				.unwrap_or(0)
		});
		evidence_groups.reverse();
		let truncated = evidence_groups.len() > MAX_EVIDENCE_GROUPS;
		evidence_groups.truncate(MAX_EVIDENCE_GROUPS);

		let matched_books = matched_books.into_iter().collect::<Vec<_>>();
		Ok(outcome(
			self.id(),
			self.name(),
			QualityStatus::Warn,
			0.5,
			json!({
				"tolerance": tolerance,
				"min_books": min_books,
				"hashed_pages": own_hashes.len(),
				"duplicate_groups": evidence_groups,
				"matched_books": matched_books,
				"truncated": truncated,
			}),
		))
	}
}

/// dHashes of the snapshot's image pages, computed from the staged copy.
/// Pages that fail to hash are skipped: they also stay visible at serve time
/// (the `visible_pages` filter never hides a page without a stored hash).
fn own_page_hashes(book: &BookSnapshot) -> Vec<(i32, i64)> {
	let mut hashes = Vec::new();
	for entry in &book.pages {
		if !entry.is_image {
			continue;
		}
		let page = (entry.index as i32).saturating_add(1);
		let bytes = match super::canonical_page(book, entry.index as usize) {
			Ok(Some((_, bytes))) => bytes,
			_ => continue,
		};
		if let Ok(hash) = stump_media::page_dhash(&bytes) {
			hashes.push((page, hash as i64));
		}
	}
	hashes
}

/// Parses the hexadecimal dHash form shared by the API surfaces; exposed for
/// sibling modules and tests.
pub fn parse_dhash(value: &str) -> Option<i64> {
	parse_dhash_hex(value)
}

#[cfg(test)]
mod tests {
	use super::parse_dhash;

	#[test]
	fn dhash_hex_roundtrips() {
		for value in [0i64, 1i64, -1i64, 0x0f0f_0f0f_0f0f_0f0f, i64::MIN, i64::MAX] {
			let text = super::dhash_hex(value);
			assert_eq!(parse_dhash(&text), Some(value), "{text}");
			assert_eq!(text.len(), 16, "{text}");
		}
	}
}
