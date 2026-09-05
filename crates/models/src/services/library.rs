/// Normalises a path by removing trailing slashes.
///
/// Shared by library validation (core) and the Komga filesystem browser, which
/// must agree on how configured roots and request paths compare.
pub(crate) fn normalize_path(path: &str) -> &str {
	let trimmed = path.trim_end_matches(['/', '\\']);
	if trimmed.is_empty() || path == "/" {
		"/"
	} else {
		trimmed
	}
}

/// Adds a single trailing slash to a path.
pub(crate) fn add_trailing_slash(path: &str) -> String {
	if path.contains('/') {
		if path.ends_with('/') {
			path.to_string()
		} else {
			format!("{path}/")
		}
	} else {
		format!("{}\\", path)
	}
}

/// Enforces the optional `library_roots` configuration constraint: when roots
/// are configured, a library root (or filesystem-browser request path) must
/// live inside one of them. The comparison is textual, matching how the
/// configured roots and persisted library paths are stored; callers that need
/// symlink-resolved guarantees canonicalize before calling.
pub fn path_within_roots(path: &str, roots: &[String]) -> bool {
	if roots.is_empty() {
		return true;
	}
	let normalized = normalize_path(path);
	roots
		.iter()
		.map(|root| normalize_path(root))
		.any(|root| normalized == root || normalized.starts_with(&add_trailing_slash(root)))
}
