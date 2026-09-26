//! Stable identities for both Liseur CAS mirrors and native records exported to Liseur.

pub const STUMP_NATIVE_ANNOTATION_ID_PREFIX: &str = "stump-native:";

pub fn stump_native_annotation_id(kind: &str, native_id: &str) -> String {
	format!("{STUMP_NATIVE_ANNOTATION_ID_PREFIX}{kind}:{native_id}")
}

pub fn parse_stump_native_annotation_id(id: &str) -> Option<(&str, &str)> {
	let (kind, native_id) = id
		.strip_prefix(STUMP_NATIVE_ANNOTATION_ID_PREFIX)?
		.split_once(':')?;
	if native_id.is_empty() || !matches!(kind, "annotation" | "bookmark") {
		return None;
	}
	Some((kind, native_id))
}

pub fn is_stump_native_annotation_id(id: &str) -> bool {
	id.starts_with(STUMP_NATIVE_ANNOTATION_ID_PREFIX)
}

pub const LISEUR_SYNC_PROJECTION_ID_PREFIX: &str = "liseur-sync:";

pub fn liseur_sync_projection_id(user_id: &str, annotation_id: &str) -> String {
	format!("{LISEUR_SYNC_PROJECTION_ID_PREFIX}{user_id}:{annotation_id}")
}

pub fn is_liseur_sync_projection_id(id: &str) -> bool {
	id.starts_with(LISEUR_SYNC_PROJECTION_ID_PREFIX)
}

/// `ORDER BY` terms that pick the book a Liseur annotation projects onto
/// from its work's `liseur_sync_media_links` rows: the edition the annotation
/// was made on, then any edition that is not an audiobook (a work also links
/// its audiobook, and a Readium locator cannot open one), then the oldest
/// link. `link` is the links table alias in the caller's query; `edition_sha`
/// is the SQL expression holding the annotation's edition digest, `''` when
/// it has none.
///
/// Every consumer that attaches a Liseur record to a media row — the
/// native projection writer, the annotation hub, and Home edits — ranks
/// with this so they name the same book.
pub fn projection_link_order_by(link: &str, edition_sha: &str) -> String {
	format!(
		"CASE WHEN {edition_sha} <> '' AND {link}.edition_sha = {edition_sha} THEN 0 ELSE 1 END, \
		 CASE WHEN EXISTS (SELECT 1 FROM media_audio \
		                   WHERE media_audio.media_id = {link}.media_id) THEN 1 ELSE 0 END, \
		 {link}.created_at ASC, {link}.id ASC"
	)
}
