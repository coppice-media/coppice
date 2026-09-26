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
