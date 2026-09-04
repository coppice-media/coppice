use std::collections::HashMap;

/// A scan-wide lookup from tag names to database IDs.
///
/// The scanner only owns this in-memory representation. Database-backed loading and insertion stay
/// in the core adapter until per-file scan orchestration is extracted.
#[derive(Debug, Default, Clone)]
pub struct TagCache(HashMap<String, i32>);

impl TagCache {
	pub fn from_entries(entries: impl IntoIterator<Item = (String, i32)>) -> Self {
		Self(entries.into_iter().collect())
	}

	/// Returns the cached ID for the given tag name.
	pub fn get(&self, name: &str) -> Option<i32> {
		self.0.get(name).copied()
	}
}
