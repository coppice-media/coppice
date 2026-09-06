//! Garbage collection for materialised provider series (Mode B hygiene).
//!
//! A virtual library lazily materialises `series`/`media` rows the first
//! time its books or pages are opened. Because those rows are only a cache
//! of a remote source, a materialised series that nobody reads must be
//! reclaimed: [`gc_materialised_series`] deletes provider-backed series that
//! (a) have no reading head for any user on any of their books and (b) were
//! created before the retention cutoff (`provider_gc_days`). Non-provider
//! series are never touched.
//!
//! Deletion mirrors the GraphQL `cleanLibrary` transaction: reading-list and
//! collection memberships are removed explicitly, remaining references
//! (tags, metadata, favorites, reading sessions) rely on their cascade, and
//! thumbnail files are swept best-effort after commit.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use models::txn::begin_write;
use models::{
	entity::{media, reading_head, series},
	services::lists,
};
use sea_orm::{
	prelude::*, sea_query::Expr, sea_query::Query, ColumnTrait, EntityTrait, QueryFilter,
};

use crate::ProviderError;

/// One deleted `series` row, with the ids needed for deletion events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcSeriesRef {
	pub id: String,
	pub library_id: String,
}

/// One deleted `media` row, with the ids needed for deletion events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcMediaRef {
	pub id: String,
	pub series_id: String,
	pub library_id: String,
}

/// What one GC pass reclaimed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcReport {
	/// Deleted `series` rows (all provider-backed).
	pub series: Vec<GcSeriesRef>,
	/// Deleted `media` rows (all provider-backed).
	pub media: Vec<GcMediaRef>,
}

/// Delete materialised provider series with no reading head older than
/// `cutoff`, returning what was removed so the caller can emit events.
pub async fn gc_materialised_series(
	conn: &DatabaseConnection,
	thumbnails_dir: Option<&Path>,
	cutoff: DateTime<Utc>,
) -> Result<GcReport, ProviderError> {
	// Series that some user has a reading head on are alive no matter how
	// old they are.
	let head_series_ids = Query::select()
		.column(media::Column::SeriesId)
		.distinct()
		.from(media::Entity)
		.inner_join(
			reading_head::Entity,
			Expr::col((media::Entity, media::Column::Id))
				.equals((reading_head::Entity, reading_head::Column::MediaId)),
		)
		.to_owned();

	let dead_series_rows = series::Entity::find()
		.filter(series::Column::SourceProvider.is_not_null())
		.filter(
			series::Column::CreatedAt
				.lt(sea_orm::prelude::DateTimeWithTimeZone::from(cutoff)),
		)
		.filter(series::Column::Id.not_in_subquery(head_series_ids))
		.all(conn)
		.await?;

	if dead_series_rows.is_empty() {
		return Ok(GcReport::default());
	}

	let dead_series: Vec<GcSeriesRef> = dead_series_rows
		.iter()
		.map(|series| GcSeriesRef {
			id: series.id.clone(),
			library_id: series.library_id.clone().unwrap_or_default(),
		})
		.collect();
	let dead_series_ids: Vec<String> =
		dead_series.iter().map(|series| series.id.clone()).collect();
	let library_of: HashMap<&str, &str> = dead_series
		.iter()
		.map(|series| (series.id.as_str(), series.library_id.as_str()))
		.collect();

	let dead_media_rows: Vec<(String, Option<String>)> = media::Entity::find()
		.filter(media::Column::SeriesId.is_in(dead_series_ids.clone()))
		.into_tuple()
		.all(conn)
		.await?;
	let dead_media: Vec<GcMediaRef> = dead_media_rows
		.into_iter()
		.map(|(id, series_id)| {
			let library_id = series_id
				.as_deref()
				.and_then(|series_id| library_of.get(series_id))
				.copied()
				.unwrap_or_default();
			GcMediaRef {
				id,
				series_id: series_id.unwrap_or_default(),
				library_id: library_id.to_string(),
			}
		})
		.collect();
	let dead_media_ids: Vec<String> =
		dead_media.iter().map(|media| media.id.clone()).collect();

	let txn = begin_write(conn).await?;
	if !dead_media_ids.is_empty() {
		lists::remove_memberships_for_media(&txn, &dead_media_ids).await?;
		media::Entity::delete_many()
			.filter(media::Column::Id.is_in(dead_media_ids.clone()))
			.exec(&txn)
			.await?;
	}
	lists::remove_memberships_for_series(&txn, &dead_series_ids).await?;
	series::Entity::delete_many()
		.filter(series::Column::Id.is_in(dead_series_ids.clone()))
		.exec(&txn)
		.await?;
	txn.commit().await?;

	if let Some(dir) = thumbnails_dir {
		let mut thumbnail_ids = dead_media_ids.clone();
		thumbnail_ids.extend(dead_series_ids.iter().cloned());
		if let Err(error) =
			stump_media::image::remove_thumbnails(&thumbnail_ids, dir).await
		{
			tracing::warn!(?error, "Failed to remove thumbnails for GC'd series");
		}
	}

	tracing::info!(
		series = dead_series.len(),
		media = dead_media.len(),
		"Provider GC reclaimed materialised series"
	);

	Ok(GcReport {
		series: dead_series,
		media: dead_media,
	})
}

#[cfg(test)]
mod tests {
	use ::tests::db;
	use models::{
		domain::reading_state::SourceProtocol,
		entity::{library, library_config, reading_head, series_metadata},
		shared::enums::{
			LibraryPattern, LibraryType, LibraryViewMode, ReadingDirection,
			ReadingImageScaleFit, ReadingMode,
		},
	};
	use sea_orm::{sea_query::Expr, ActiveValue::Set, EntityTrait, QueryFilter};

	use super::*;

	async fn seed_library(db: &DatabaseConnection, id: &str, provider: Option<&str>) {
		let config = library_config::ActiveModel {
			library_id: Set(None),
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
		.insert(db)
		.await
		.expect("config insert");

		library::ActiveModel {
			id: Set(id.to_string()),
			name: Set(id.to_string()),
			path: Set(format!("provider://{id}")),
			config_id: Set(config.id),
			source_provider: Set(provider.map(str::to_string)),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("library insert");
	}

	async fn seed_series(
		db: &DatabaseConnection,
		id: &str,
		library_id: &str,
		provider: Option<&str>,
		age_days: i64,
	) {
		// `before_save` stamps created_at on insert, so age is applied after.
		series::ActiveModel {
			id: Set(id.to_string()),
			name: Set(id.to_string()),
			path: Set(match provider {
				Some(provider) => format!("provider://{provider}/{id}"),
				None => format!("/tmp/{id}"),
			}),
			library_id: Set(Some(library_id.to_string())),
			source_provider: Set(provider.map(str::to_string)),
			remote_id: Set(provider.map(|_| id.to_string())),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("series insert");
		let age = (Utc::now() - chrono::Duration::days(age_days)).fixed_offset();
		series::Entity::update_many()
			.filter(series::Column::Id.eq(id))
			.col_expr(series::Column::CreatedAt, Expr::value(age))
			.exec(db)
			.await
			.expect("age update");
		series_metadata::ActiveModel {
			series_id: Set(id.to_string()),
			title_sort_lock: Set(false),
			reading_direction_lock: Set(false),
			language_lock: Set(false),
			alternate_titles_lock: Set(false),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("metadata insert");
	}

	#[tokio::test]
	async fn gc_deletes_unread_provider_series_and_keeps_read_ones() {
		let db = db::test_database().await;

		seed_library(&db, "prov-lib", Some("mock-en")).await;
		seed_library(&db, "local-lib", None).await;
		seed_series(&db, "old-unread", "prov-lib", Some("mock-en"), 60).await;
		seed_series(&db, "old-read", "prov-lib", Some("mock-en"), 60).await;
		let reader = ::tests::fake_data::User::new("reader").insert(&db).await;
		seed_read_chapter(&db, &reader.id, "old-read").await;
		seed_series(&db, "young", "prov-lib", Some("mock-en"), 1).await;
		seed_series(&db, "old-local", "local-lib", None, 400).await;

		let report =
			gc_materialised_series(&db, None, Utc::now() - chrono::Duration::days(30))
				.await
				.expect("gc pass");

		assert_eq!(
			report
				.series
				.iter()
				.map(|series| series.id.as_str())
				.collect::<Vec<_>>(),
			vec!["old-unread"]
		);

		for survivor in ["old-read", "young", "old-local"] {
			assert!(
				series::Entity::find_by_id(survivor)
					.one(&db)
					.await
					.expect("lookup")
					.is_some(),
				"{survivor} must survive GC"
			);
		}
		assert!(
			series::Entity::find_by_id("old-unread")
				.one(&db)
				.await
				.expect("lookup")
				.is_none(),
			"old unread provider series must be deleted"
		);
	}

	/// One materialised chapter under `series_id` with a reading head for
	/// `user_id` on it.
	async fn seed_read_chapter(db: &DatabaseConnection, user_id: &str, series_id: &str) {
		let media_id = format!("{series_id}-ch1");
		media::ActiveModel {
			id: Set(media_id.clone()),
			name: Set("ch1".to_string()),
			path: Set(format!("provider://mock-en/{series_id}/ch1")),
			extension: Set("cbz".to_string()),
			size: Set(0),
			pages: Set(0),
			series_id: Set(Some(series_id.to_string())),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("media insert");

		let now = Utc::now().fixed_offset();
		reading_head::ActiveModel {
			user_id: Set(user_id.to_string()),
			media_id: Set(media_id),
			progression: Set(0.5),
			completed: Set(false),
			created_at: Set(now),
			updated_at: Set(now),
			changed_at: Set(now),
			source_protocol: Set(SourceProtocol::Komga),
			revision: Set(1),
			event_id: Set(1),
			..Default::default()
		}
		.insert(db)
		.await
		.expect("head insert");
	}

	#[tokio::test]
	async fn gc_keeps_series_with_reading_head_on_any_book() {
		let db = db::test_database().await;

		seed_library(&db, "prov-lib", Some("mock-en")).await;
		seed_series(&db, "old-with-head", "prov-lib", Some("mock-en"), 90).await;
		let reader = ::tests::fake_data::User::new("reader").insert(&db).await;
		seed_read_chapter(&db, &reader.id, "old-with-head").await;

		let report =
			gc_materialised_series(&db, None, Utc::now() - chrono::Duration::days(30))
				.await
				.expect("gc pass");

		assert!(report.series.is_empty(), "head keeps the series alive");
		assert!(series::Entity::find_by_id("old-with-head")
			.one(&db)
			.await
			.expect("lookup")
			.is_some());
	}
}
