//! Turn a remote series into ordinary `series`/`media` rows.
//!
//! Every chapter becomes one `media` row with `extension = "cbz"`, a
//! `provider://` path, and the chapter's known page count (0 until the page
//! list is first resolved). Re-running for the same series only adds chapters
//! that are not stored yet, which is also how a refresh works.

use models::{
	entity::{media, media_metadata, series, series_metadata},
	shared::enums::FileStatus,
};
use rust_decimal::Decimal;
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait,
	QueryFilter, QuerySelect,
};

use crate::{
	host::{ProviderError, ProviderHost},
	source::{RemoteChapter, RemoteSeries},
	virtual_path::{self, VirtualPath},
};

/// The rows touched by [`add_series`] / [`refresh_series`].
#[derive(Debug, Clone, PartialEq)]
pub struct Materialized {
	pub series: series::Model,
	/// Newly inserted chapters, in remote order.
	pub created: Vec<media::Model>,
	/// Chapters already present before this run.
	pub existing: usize,
}

/// Materialise (or extend) `remote_id` from `source_id` into `library_id`.
pub async fn add_series(
	host: &ProviderHost,
	library_id: &str,
	source_id: &str,
	remote_id: &str,
) -> Result<Materialized, ProviderError> {
	let source = host.source(source_id)?;
	let details = source.details(remote_id).await?;
	let chapters = source.chapters(remote_id).await?;
	let conn = host.conn();

	let existing_series = series::Entity::find()
		.filter(series::Column::SourceProvider.eq(source_id))
		.filter(series::Column::RemoteId.eq(remote_id))
		.one(conn)
		.await?;
	let series_row = match existing_series {
		Some(existing) => update_series(conn, existing, &details).await?,
		None => insert_series(conn, library_id, source_id, &details).await?,
	};
	upsert_series_metadata(conn, &series_row, source_id, &details).await?;
	let (created, existing) =
		insert_chapters(conn, &series_row, source_id, remote_id, &chapters).await?;
	Ok(Materialized {
		series: series_row,
		created,
		existing,
	})
}

/// Re-fetch details and chapters for a materialised series.
pub async fn refresh_series(
	host: &ProviderHost,
	series_id: &str,
) -> Result<Materialized, ProviderError> {
	let existing = series::Entity::find_by_id(series_id)
		.one(host.conn())
		.await?
		.ok_or_else(|| ProviderError::Other(format!("Series {series_id} not found")))?;
	let (Some(source_id), Some(remote_id), Some(library_id)) = (
		existing.source_provider.clone(),
		existing.remote_id.clone(),
		existing.library_id.clone(),
	) else {
		return Err(ProviderError::NotVirtual(existing.path));
	};
	add_series(host, &library_id, &source_id, &remote_id).await
}

async fn insert_series<C: ConnectionTrait>(
	conn: &C,
	library_id: &str,
	source_id: &str,
	details: &RemoteSeries,
) -> Result<series::Model, ProviderError> {
	let row = series::ActiveModel {
		id: Set(virtual_path::series_id(source_id, &details.remote_id)),
		name: Set(details.title.clone()),
		description: Set(details.description.clone()),
		path: Set(VirtualPath::series(source_id, &details.remote_id).to_string()),
		status: Set(FileStatus::Ready),
		library_id: Set(Some(library_id.to_string())),
		source_provider: Set(Some(source_id.to_string())),
		remote_id: Set(Some(details.remote_id.clone())),
		..Default::default()
	}
	.insert(conn)
	.await?;
	Ok(row)
}

async fn update_series<C: ConnectionTrait>(
	conn: &C,
	existing: series::Model,
	details: &RemoteSeries,
) -> Result<series::Model, ProviderError> {
	let unchanged = existing.name == details.title
		&& existing.description == details.description
		&& existing.status == FileStatus::Ready;
	if unchanged {
		return Ok(existing);
	}
	let mut active: series::ActiveModel = existing.into();
	active.name = Set(details.title.clone());
	active.description = Set(details.description.clone());
	active.status = Set(FileStatus::Ready);
	Ok(active.update(conn).await?)
}

async fn upsert_series_metadata<C: ConnectionTrait>(
	conn: &C,
	series_row: &series::Model,
	source_id: &str,
	details: &RemoteSeries,
) -> Result<(), ProviderError> {
	let joined = |values: &[String]| {
		if values.is_empty() {
			None
		} else {
			Some(values.join(", "))
		}
	};
	let fields = series_metadata::ActiveModel {
		series_id: Set(series_row.id.clone()),
		title: Set(Some(details.title.clone())),
		summary: Set(details.description.clone()),
		status: Set(Some(details.status.as_metadata_status().to_string())),
		genres: Set(joined(&details.genres)),
		writers: Set(joined(&details.authors)),
		comic_image: Set(details.thumbnail_url.clone()),
		language: Set(details.original_language.clone()),
		links: Set(details.url.clone()),
		metadata_source: Set(Some(source_id.to_string())),
		metadata_external_id: Set(Some(details.remote_id.clone())),
		age_rating: Set(details.nsfw.then_some(18)),
		..Default::default()
	};
	let exists = series_metadata::Entity::find_by_id(&series_row.id)
		.one(conn)
		.await?
		.is_some();
	if exists {
		fields.update(conn).await?;
	} else {
		let mut fields = fields;
		fields.title_sort_lock = Set(false);
		fields.reading_direction_lock = Set(false);
		fields.language_lock = Set(false);
		fields.alternate_titles_lock = Set(false);
		fields.insert(conn).await?;
	}
	Ok(())
}

async fn insert_chapters<C: ConnectionTrait>(
	conn: &C,
	series_row: &series::Model,
	source_id: &str,
	remote_id: &str,
	chapters: &[RemoteChapter],
) -> Result<(Vec<media::Model>, usize), ProviderError> {
	let known: Vec<String> = media::Entity::find()
		.select_only()
		.column(media::Column::RemoteChapterId)
		.filter(media::Column::SeriesId.eq(series_row.id.clone()))
		.filter(media::Column::SourceProvider.eq(source_id))
		.into_tuple::<Option<String>>()
		.all(conn)
		.await?
		.into_iter()
		.flatten()
		.collect();
	let mut created = Vec::new();
	let mut existing = 0usize;
	for chapter in chapters {
		if known.iter().any(|id| id == &chapter.remote_id) {
			existing += 1;
			continue;
		}
		let name = chapter.display_name();
		let row = media::ActiveModel {
			id: Set(virtual_path::media_id(source_id, &chapter.remote_id)),
			name: Set(name.clone()),
			size: Set(0),
			extension: Set("cbz".to_string()),
			pages: Set(chapter.page_count.map(|count| count as i32).unwrap_or(0)),
			modified_at: Set(chapter.uploaded_at.map(Into::into)),
			path: Set(VirtualPath::chapter(source_id, remote_id, &chapter.remote_id).to_string()),
			status: Set(FileStatus::Ready),
			series_id: Set(Some(series_row.id.clone())),
			source_provider: Set(Some(source_id.to_string())),
			remote_id: Set(Some(remote_id.to_string())),
			remote_chapter_id: Set(Some(chapter.remote_id.clone())),
			..Default::default()
		}
		.insert(conn)
		.await?;
		media_metadata::ActiveModel {
			media_id: Set(Some(row.id.clone())),
			title: Set(Some(name)),
			series: Set(Some(series_row.name.clone())),
			number: Set(chapter
				.number
				.and_then(|number| Decimal::try_from(number).ok())),
			volume: Set(chapter
				.volume
				.as_deref()
				.and_then(|volume| volume.parse::<i32>().ok())),
			language: Set(chapter.lang.clone()),
			publisher: Set(chapter.scanlator.clone()),
			page_count: Set(chapter.page_count.map(|count| count as i32)),
			year: Set(chapter.uploaded_at.map(|at| {
				use chrono::Datelike;
				at.year()
			})),
			month: Set(chapter.uploaded_at.map(|at| {
				use chrono::Datelike;
				at.month() as i32
			})),
			day: Set(chapter.uploaded_at.map(|at| {
				use chrono::Datelike;
				at.day() as i32
			})),
			links: Set(chapter.url.clone()),
			metadata_source: Set(Some(source_id.to_string())),
			metadata_external_id: Set(Some(chapter.remote_id.clone())),
			..Default::default()
		}
		.insert(conn)
		.await?;
		created.push(row);
	}
	Ok((created, existing))
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use models::entity::{library, library_config, media, media_metadata, series_metadata};
	use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
	use stump_media::{virtual_media::VirtualMediaResolver, ContentType};

	use super::*;
	use crate::{
		host::{ProviderHostConfig, VirtualArchive},
		mock::{MockSource, ALPHA_CHAPTERS, MOCK_SOURCE_ID, PAGES_PER_CHAPTER, SERIES_ALPHA},
	};

	async fn library(conn: &DatabaseConnection) -> library::Model {
		let config = library_config::ActiveModel {
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap();
		library::ActiveModel {
			name: Set("Remote".to_string()),
			path: Set("/remote".to_string()),
			config_id: Set(config.id),
			..Default::default()
		}
		.insert(conn)
		.await
		.unwrap()
	}

	async fn host(cache_max_bytes: u64) -> (Arc<ProviderHost>, Arc<MockSource>, tempfile::TempDir) {
		let conn = ::tests::db::test_database().await;
		let dir = tempfile::tempdir().unwrap();
		let host = ProviderHost::open(
			conn,
			Vec::new(),
			ProviderHostConfig {
				cache_dir: dir.path().to_path_buf(),
				cache_max_bytes,
				catalog_url: Some("http://127.0.0.1:9/".to_string()),
			},
		)
		.await
		.unwrap();
		let source = MockSource::new();
		host.register_source(source.clone());
		(host, source, dir)
	}

	#[tokio::test]
	async fn add_series_creates_rows_and_refresh_is_idempotent() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;

		let first = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		assert_eq!(first.created.len(), ALPHA_CHAPTERS.len());
		assert_eq!(first.existing, 0);
		let series_row = &first.series;
		assert_eq!(series_row.name, "Alpha Adventures");
		assert_eq!(series_row.path, "provider://mock-en/alpha");
		assert_eq!(series_row.id, virtual_path::series_id(MOCK_SOURCE_ID, SERIES_ALPHA));
		assert_eq!(series_row.source_provider.as_deref(), Some(MOCK_SOURCE_ID));
		assert_eq!(series_row.remote_id.as_deref(), Some(SERIES_ALPHA));
		assert_eq!(series_row.library_id.as_deref(), Some(library.id.as_str()));

		let metadata = series_metadata::Entity::find_by_id(&series_row.id)
			.one(host.conn())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(metadata.status.as_deref(), Some("Continuing"));
		assert_eq!(metadata.genres.as_deref(), Some("Action, Comedy"));
		assert_eq!(
			metadata.comic_image.as_deref(),
			Some("http://mock.invalid/covers/alpha.png")
		);

		let chapter3 = &first.created[0];
		assert_eq!(chapter3.name, "Vol. 1 Ch. 3 - Chapter 3");
		assert_eq!(chapter3.extension, "cbz");
		assert_eq!(chapter3.pages, PAGES_PER_CHAPTER as i32);
		assert_eq!(chapter3.path, "provider://mock-en/alpha/alpha-ch3");
		assert_eq!(chapter3.remote_chapter_id.as_deref(), Some("alpha-ch3"));
		// The mock omits the page count for chapter 2: unknown until first open.
		assert_eq!(first.created[1].pages, 0);
		let chapter_metadata = media_metadata::Entity::find()
			.filter(media_metadata::Column::MediaId.eq(chapter3.id.clone()))
			.one(host.conn())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(chapter_metadata.number, Some(Decimal::from(3)));
		assert_eq!(chapter_metadata.volume, Some(1));
		assert_eq!(chapter_metadata.publisher.as_deref(), Some("Mock Scans"));

		let again = refresh_series(&host, &series_row.id).await.unwrap();
		assert!(again.created.is_empty());
		assert_eq!(again.existing, ALPHA_CHAPTERS.len());
		assert_eq!(again.series.id, series_row.id);
		let count = media::Entity::find()
			.filter(media::Column::SeriesId.eq(series_row.id.clone()))
			.all(host.conn())
			.await
			.unwrap()
			.len();
		assert_eq!(count, ALPHA_CHAPTERS.len());
		assert_eq!(source.detail_fetches(), 2);
	}

	#[tokio::test]
	async fn unknown_series_and_disabled_sources_error() {
		let (host, _source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		assert!(matches!(
			add_series(&host, &library.id, MOCK_SOURCE_ID, "nope").await,
			Err(ProviderError::Source(crate::source::SourceError::NotFound(_)))
		));
		assert!(matches!(
			add_series(&host, &library.id, "missing-source", SERIES_ALPHA).await,
			Err(ProviderError::UnknownSource(_))
		));
	}

	#[tokio::test]
	async fn pages_resolve_through_cache_and_store_page_count_once() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		let chapter2 = &materialized.created[1];
		assert_eq!(chapter2.pages, 0);

		let resolver: &dyn VirtualMediaResolver = host.as_ref();
		assert!(resolver.owns(&chapter2.path));
		assert!(!resolver.owns("/library/book.cbz"));
		assert_eq!(resolver.get_page_count(&chapter2.path).await.unwrap(), 3);
		let stored = media::Entity::find_by_id(&chapter2.id)
			.one(host.conn())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(stored.pages, PAGES_PER_CHAPTER as i32);

		let (content_type, bytes) = resolver.get_page(&chapter2.path, 2).await.unwrap();
		assert_eq!(content_type, ContentType::PNG);
		assert_eq!(bytes, MockSource::page_bytes("alpha-ch2", 1));
		assert_eq!(source.page_fetches(), 1);
		let (_, again) = resolver.get_page(&chapter2.path, 2).await.unwrap();
		assert_eq!(again, bytes);
		assert_eq!(source.page_fetches(), 1, "second read is a cache hit");

		let types = resolver.page_content_types(&chapter2.path, &[1, 2]).unwrap();
		assert_eq!(types[&2], ContentType::PNG);
		assert_eq!(types[&1], ContentType::PNG, "manifest extension answers uncached pages");

		assert!(matches!(
			resolver.get_page(&chapter2.path, 9).await,
			Err(stump_media::FileError::PageNotFound { page: 9, available: 3 })
		));
		assert!(resolver.get_page("/library/book.cbz", 1).await.is_err());

		// The series path serves the cover through the same cache.
		let (cover_type, cover) = resolver.get_page(&materialized.series.path, 1).await.unwrap();
		assert_eq!(cover_type, ContentType::PNG);
		assert_eq!(cover, crate::mock::PNG_PIXEL);
		assert_eq!(resolver.get_page_count(&materialized.series.path).await.unwrap(), 1);
	}

	#[tokio::test]
	async fn archive_packs_every_page_as_stored_cbz() {
		let (host, source, _dir) = host(u64::MAX).await;
		let library = library(host.conn()).await;
		let materialized = add_series(&host, &library.id, MOCK_SOURCE_ID, SERIES_ALPHA)
			.await
			.unwrap();
		let chapter = &materialized.created[2];
		let VirtualArchive {
			file_name,
			content_type,
			bytes,
		} = host
			.build_archive(MOCK_SOURCE_ID, "alpha-ch1", &chapter.name)
			.await
			.unwrap();
		assert_eq!(file_name, "Vol. 1 Ch. 1 - Chapter 1.cbz");
		assert_eq!(content_type, ContentType::COMIC_ZIP);
		assert_eq!(source.page_fetches(), PAGES_PER_CHAPTER as usize);

		let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
		assert_eq!(archive.len(), PAGES_PER_CHAPTER as usize);
		let names: Vec<String> = (0..archive.len())
			.map(|index| archive.by_index(index).unwrap().name().to_string())
			.collect();
		assert_eq!(names, vec!["0001.png", "0002.png", "0003.png"]);
		let mut first = archive.by_name("0001.png").unwrap();
		assert_eq!(first.compression(), zip::CompressionMethod::Stored);
		let mut contents = Vec::new();
		std::io::Read::read_to_end(&mut first, &mut contents).unwrap();
		assert_eq!(contents, MockSource::page_bytes("alpha-ch1", 0));

		// Pages fetched for the archive are cache hits afterwards.
		let again = host.build_archive(MOCK_SOURCE_ID, "alpha-ch1", "x").await.unwrap();
		assert_eq!(again.file_name, "x.cbz");
		assert_eq!(source.page_fetches(), PAGES_PER_CHAPTER as usize);
	}

	#[tokio::test]
	async fn enable_and_disable_instances_round_trip_through_the_registry() {
		fn build(row: &models::entity::provider_source::Model) -> Result<Arc<dyn crate::source::Source>, ProviderError> {
			assert_eq!(row.implementation, "mock");
			Ok(MockSource::new())
		}
		let conn = ::tests::db::test_database().await;
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join(crate::catalog::INDEX_FILE_NAME),
			include_bytes!("../tests/fixtures/keiyoushi-index.json"),
		)
		.unwrap();
		let factory = crate::host::SourceFactory {
			implementation: "mock",
			name: "Mock",
			catalog_pkg: "eu.kanade.tachiyomi.extension.all.mangadex",
			base_url: "https://mangadex.org",
			build,
		};
		let host = ProviderHost::open(
			conn,
			vec![factory],
			ProviderHostConfig {
				cache_dir: dir.path().to_path_buf(),
				cache_max_bytes: 1024,
				catalog_url: Some("http://127.0.0.1:9/".to_string()),
			},
		)
		.await
		.unwrap();
		assert!(host.sources().is_empty());

		let row = host
			.enable_catalog_source("2499283573021220255", Some("user"))
			.await
			.unwrap();
		assert_eq!(row.id, "mock-en");
		assert_eq!(row.lang, "en");
		assert_eq!(row.catalog_id.as_deref(), Some("2499283573021220255"));
		assert!(row.enabled);
		assert_eq!(host.sources().len(), 1);

		assert!(matches!(
			host.enable_catalog_source("5498091984644576825", None).await,
			Err(ProviderError::NotImplemented(_))
		));
		assert!(matches!(
			host.enable_catalog_source("0", None).await,
			Err(ProviderError::CatalogSourceNotFound(_))
		));

		assert!(host.disable_source("mock-en").await.unwrap());
		assert!(host.sources().is_empty());
		assert!(matches!(host.source("mock-en"), Err(ProviderError::UnknownSource(_))));
		assert!(!host.disable_source("never").await.unwrap());

		// Re-enabling reuses the row and reload picks it up from the database.
		let row = host.enable_implementation("mock", "EN", None).await.unwrap();
		assert_eq!(row.id, "mock-en");
		assert!(row.updated_at.is_some());
		host.clear_sources();
		host.reload_sources().await.unwrap();
		assert_eq!(host.sources().len(), 1);
	}
}
