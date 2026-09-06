//! Virtual library rows: Mode B "libraries" that live over a provider source.
//!
//! A virtual library is an ordinary `libraries` row whose `path` is a
//! `provider://` URI and whose `source_provider` names the backing
//! [`Source`](crate::Source) instance. Its id is deterministic
//! ([`virtual_path::library_id`]), so re-running creation for the same
//! source is idempotent. The filesystem scanner skips these rows; the
//! Komga/OPDS branches route them to live browse instead.

use models::{
	entity::{library, library_config},
	shared::enums::{
		FileStatus, LibraryPattern, LibraryType, LibraryViewMode, ReadingDirection,
		ReadingImageScaleFit, ReadingMode,
	},
};
use sea_orm::{prelude::*, ActiveValue::Set, EntityTrait};

use crate::{host::ProviderError, virtual_path};

/// Create (or fetch) the virtual library backing `source_id`.
///
/// `name` defaults to the source instance id when not supplied. The row is
/// created with a config whose `watch` is off: virtual libraries have no
/// filesystem root to watch, and their contents arrive through browse and
/// materialisation instead of scans.
pub async fn create_virtual_library<C: ConnectionTrait>(
	conn: &C,
	source_id: &str,
	name: Option<String>,
) -> Result<library::Model, ProviderError> {
	let id = virtual_path::library_id(source_id);
	if let Some(existing) = library::Entity::find_by_id(&id).one(conn).await? {
		return Ok(existing);
	}

	let config = library_config::ActiveModel {
		library_id: Set(Some(id.clone())),
		convert_rar_to_zip: Set(false),
		hard_delete_conversions: Set(false),
		default_reading_dir: Set(ReadingDirection::default()),
		default_reading_mode: Set(ReadingMode::default()),
		default_reading_image_scale_fit: Set(ReadingImageScaleFit::default()),
		generate_file_hashes: Set(false),
		generate_koreader_hashes: Set(false),
		process_metadata: Set(true),
		watch: Set(false),
		library_pattern: Set(LibraryPattern::default()),
		library_type: Set(LibraryType::default()),
		default_library_view_mode: Set(LibraryViewMode::default()),
		hide_series_view: Set(false),
		skip_book_overview: Set(false),
		process_thumbnail_colors_even_without_config: Set(false),
		..Default::default()
	}
	.insert(conn)
	.await?;

	let row = library::ActiveModel {
		id: Set(id),
		name: Set(name
			.filter(|name| !name.trim().is_empty())
			.unwrap_or_else(|| source_id.to_string())),
		path: Set(format!("{}{}", virtual_path::SCHEME, source_id)),
		status: Set(FileStatus::Ready),
		config_id: Set(config.id),
		source_provider: Set(Some(source_id.to_string())),
		..Default::default()
	}
	.insert(conn)
	.await?;

	tracing::info!(
		library = row.id,
		source = source_id,
		"Created virtual library"
	);
	Ok(row)
}

#[cfg(test)]
mod tests {
	use ::tests::db;
	use models::entity::library;
	use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

	use super::*;

	#[tokio::test]
	async fn create_is_idempotent_and_marks_source_provider() {
		let db = db::test_database().await;

		let first =
			create_virtual_library(&db, "mock-en", Some("Manga Mocks".to_string()))
				.await
				.expect("create");
		let again =
			create_virtual_library(&db, "mock-en", Some("Manga Mocks".to_string()))
				.await
				.expect("recreate");

		assert_eq!(first.id, again.id, "library id is deterministic per source");
		assert_eq!(first.source_provider.as_deref(), Some("mock-en"));
		assert_eq!(first.name, "Manga Mocks");

		let count = library::Entity::find()
			.filter(library::Column::SourceProvider.eq("mock-en"))
			.count(&db)
			.await
			.expect("count");
		assert_eq!(count, 1, "no duplicate rows for the same source");
	}
}
