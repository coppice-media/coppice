use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use models::{
	entity::{library, media, scanned_directory, series},
	shared::enums::FileStatus,
};
use sea_orm::{prelude::*, FromQueryResult, QuerySelect};
use stump_scanner::{
	MediaIdentity, ScanError, ScanResult, ScanSource, ScanStatus, SeriesIdentity,
};

/// SeaORM implementation of the read-only scanner source boundary.
#[derive(Clone)]
pub(crate) struct SeaOrmScanSource {
	conn: Arc<DatabaseConnection>,
}

impl SeaOrmScanSource {
	pub(crate) fn new(conn: Arc<DatabaseConnection>) -> Self {
		Self { conn }
	}
}

#[derive(Debug, FromQueryResult)]
struct SeriesScanRow {
	id: String,
	path: String,
	status: FileStatus,
}

#[derive(Debug, FromQueryResult)]
struct MediaScanRow {
	id: String,
	path: String,
	modified_at: Option<DateTimeWithTimeZone>,
	hash: Option<String>,
	status: FileStatus,
}

fn map_status(status: FileStatus) -> ScanStatus {
	match status {
		FileStatus::Unknown => ScanStatus::Unknown,
		FileStatus::Ready => ScanStatus::Ready,
		FileStatus::Unsupported => ScanStatus::Unsupported,
		FileStatus::Error => ScanStatus::Error,
		FileStatus::Missing => ScanStatus::Missing,
	}
}

#[async_trait]
impl ScanSource for SeaOrmScanSource {
	async fn existing_series(&self, library_id: &str) -> ScanResult<Vec<SeriesIdentity>> {
		series::Entity::find()
			.select_only()
			.columns([
				series::Column::Id,
				series::Column::Path,
				series::Column::Status,
			])
			.filter(series::Column::LibraryId.eq(library_id))
			.into_model::<SeriesScanRow>()
			.all(self.conn.as_ref())
			.await
			.map(|rows| {
				rows.into_iter()
					.map(|row| SeriesIdentity {
						id: row.id,
						path: row.path,
						status: map_status(row.status),
					})
					.collect()
			})
			.map_err(ScanError::source)
	}

	async fn existing_media(&self, series_id: &str) -> ScanResult<Vec<MediaIdentity>> {
		media::Entity::find()
			.select_only()
			.columns([
				media::Column::Id,
				media::Column::Path,
				media::Column::ModifiedAt,
				media::Column::Hash,
				media::Column::Status,
			])
			.filter(media::Column::SeriesId.eq(series_id))
			.into_model::<MediaScanRow>()
			.all(self.conn.as_ref())
			.await
			.map(|rows| {
				rows.into_iter()
					.map(|row| MediaIdentity {
						id: row.id,
						path: row.path,
						modified_at: row.modified_at,
						hash: row.hash,
						status: map_status(row.status),
					})
					.collect()
			})
			.map_err(ScanError::source)
	}

	async fn stored_dir_mtimes(
		&self,
		library_id: &str,
	) -> ScanResult<HashMap<String, u64>> {
		let Some(library) = library::Entity::find_by_id(library_id)
			.one(self.conn.as_ref())
			.await
			.map_err(ScanError::source)?
		else {
			return Ok(HashMap::new());
		};

		scanned_directory::Entity::find()
			.filter(scanned_directory::Column::Path.starts_with(library.path))
			.all(self.conn.as_ref())
			.await
			.map(|rows| {
				rows.into_iter()
					.map(|row| (row.path, row.last_mtime.max(0) as u64))
					.collect()
			})
			.map_err(ScanError::source)
	}
}
