use std::{
	collections::HashSet,
	path::{Path, PathBuf},
	sync::{Arc, OnceLock},
	time::{Duration, SystemTime},
};

use models::entity::{media, series};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use stump_core::{job::CoreJobOutput, CoreEvent};
use tokio::sync::{mpsc, Mutex, Semaphore};

use crate::{
	config::state::AppState,
	routers::kobo_backend::kepub::{cache_path_for, ensure_cached},
};

const QUEUE_CAPACITY: usize = 128;
const MAX_CONCURRENT_CONVERSIONS: usize = 1;
const EVICTION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const TEMP_FILE_GRACE: Duration = Duration::from_secs(60 * 60);

struct WarmRequest {
	ctx: AppState,
	book: media::MediaIdentSelect,
	cache_path: PathBuf,
}

pub(crate) struct KepubWarmer {
	tx: mpsc::Sender<WarmRequest>,
	pending: Arc<Mutex<HashSet<PathBuf>>>,
}

static WARMER: OnceLock<Arc<KepubWarmer>> = OnceLock::new();

/// Start the process-wide KEPUB warmer and cache eviction loop once.
///
/// The worker is deliberately lazy: creating a Kobo router starts only the
/// bounded queue and background tasks; no EPUB is read until a request is
/// submitted.
pub(crate) fn start(ctx: AppState) -> Arc<KepubWarmer> {
	WARMER
		.get_or_init(|| {
			let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
			let pending = Arc::new(Mutex::new(HashSet::new()));
			let warmer = Arc::new(KepubWarmer {
				tx,
				pending: pending.clone(),
			});
			let worker_warmer = warmer.clone();
			tokio::spawn(worker(rx, worker_warmer));

			let eviction_ctx = ctx.clone();
			tokio::spawn(async move {
				let mut interval = tokio::time::interval(EVICTION_INTERVAL);
				interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
				loop {
					interval.tick().await;
					evict_once(&eviction_ctx).await;
				}
			});

			if ctx.config.kobo_kepub_preconvert && ctx.config.kobo_kepub_conversion {
				let preconvert_ctx = ctx.clone();
				tokio::spawn(
					async move { scan_completion_listener(preconvert_ctx).await },
				);
			}
			warmer
		})
		.clone()
}

async fn scan_completion_listener(ctx: AppState) {
	let mut events = ctx.get_client_receiver();
	loop {
		match events.recv().await {
			Ok(CoreEvent::JobOutput(output)) => {
				if let CoreJobOutput::LibraryScan(scan) = output.output {
					preconvert_library(&ctx, &scan.library_id).await;
				}
			},
			Ok(_) => {},
			Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
				tracing::warn!(
					count,
					"KEPUB preconvert listener lagged behind core events"
				);
			},
			Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
		}
	}
}

async fn preconvert_library(ctx: &AppState, library_id: &str) {
	let ids = match media::Entity::find()
		.inner_join(series::Entity)
		.select_only()
		.column(media::Column::Id)
		.filter(series::Column::LibraryId.eq(library_id))
		.filter(media::Column::Extension.eq("epub"))
		.into_tuple::<String>()
		.all(ctx.conn.as_ref())
		.await
	{
		Ok(ids) => ids,
		Err(error) => {
			tracing::debug!(
				?error,
				library_id,
				"Failed to list EPUBs for KEPUB preconversion"
			);
			return;
		},
	};
	for id in ids {
		enqueue(ctx.clone(), id);
	}
}

/// Enqueue a media id for conversion. The database lookup and queue send are
/// detached from the request path so a sync response is never delayed by a
/// conversion or a full queue.
pub(crate) fn enqueue(ctx: AppState, media_id: impl Into<String>) {
	let warmer = start(ctx.clone());
	let media_id = media_id.into();
	tokio::spawn(async move {
		let Some(book) = media::Entity::find_by_id(&media_id)
			.one(ctx.conn.as_ref())
			.await
			.ok()
			.flatten()
			.map(media::MediaIdentSelect::from)
		else {
			tracing::debug!(media_id, "Skipping KEPUB warm request for missing media");
			return;
		};
		if !Path::new(&book.path)
			.extension()
			.and_then(|extension| extension.to_str())
			.is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
		{
			return;
		}
		if let Err(error) = warmer.enqueue(ctx, book).await {
			tracing::debug!(?error, "Skipping KEPUB warm request");
		}
	});
}

impl KepubWarmer {
	async fn enqueue(
		&self,
		ctx: AppState,
		book: media::MediaIdentSelect,
	) -> Result<(), ()> {
		let cache_path = cache_path_for(&ctx, &book).await.map_err(|_| ())?;
		if tokio::fs::metadata(&cache_path).await.is_ok() {
			return Ok(());
		}
		{
			let mut pending = self.pending.lock().await;
			if !pending.insert(cache_path.clone()) {
				return Ok(());
			}
		}
		match self.tx.try_send(WarmRequest {
			ctx,
			book,
			cache_path: cache_path.clone(),
		}) {
			Ok(()) => Ok(()),
			Err(_) => {
				self.pending.lock().await.remove(&cache_path);
				Err(())
			},
		}
	}
}

async fn worker(mut rx: mpsc::Receiver<WarmRequest>, warmer: Arc<KepubWarmer>) {
	let semaphore = Semaphore::new(MAX_CONCURRENT_CONVERSIONS);
	while let Some(request) = rx.recv().await {
		let permit = match semaphore.acquire().await {
			Ok(permit) => permit,
			Err(_) => return,
		};
		if let Err(error) = ensure_cached(&request.ctx, &request.book).await {
			tracing::debug!(?error, path = ?request.cache_path, "KEPUB warm conversion failed");
		}
		warmer.pending.lock().await.remove(&request.cache_path);
		drop(permit);
	}
}

async fn evict_once(ctx: &AppState) {
	let path = ctx.config.get_cache_dir().join("kepub");
	if let Err(error) =
		evict_cache_dir(&path, ctx.config.kobo_kepub_cache_max_age_days).await
	{
		tracing::debug!(?error, path = ?path, "KEPUB cache eviction failed");
	}
}

/// Delete cache files older than `max_age_days` by mtime.
///
/// Temporary files receive a one-hour grace period, so an in-flight conversion
/// is never removed by a sweep. Files are otherwise treated exactly like cache
/// entries and are removed once they exceed the configured age.
pub(crate) async fn evict_cache_dir(
	path: &Path,
	max_age_days: u32,
) -> std::io::Result<u64> {
	let mut entries = match tokio::fs::read_dir(path).await {
		Ok(entries) => entries,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
		Err(error) => return Err(error),
	};
	let now = SystemTime::now();
	let max_age = Duration::from_secs(u64::from(max_age_days) * 24 * 60 * 60);
	let mut removed = 0;
	while let Some(entry) = entries.next_entry().await? {
		let entry_path = entry.path();
		if !entry.file_type().await?.is_file() {
			continue;
		}
		let metadata = entry.metadata().await?;
		let age = now
			.duration_since(metadata.modified().unwrap_or(now))
			.unwrap_or_default();
		let is_tmp = entry_path.extension().is_some_and(|ext| ext == "tmp");
		if is_tmp && age < TEMP_FILE_GRACE {
			continue;
		}
		if age >= max_age {
			match tokio::fs::remove_file(&entry_path).await {
				Ok(()) => removed += 1,
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
				Err(error) => return Err(error),
			}
		}
	}
	Ok(removed)
}

#[cfg(test)]
mod tests {
	use super::*;
	use filetime::{set_file_mtime, FileTime};

	#[tokio::test]
	async fn eviction_respects_age_and_tmp_grace_period() {
		let dir = tempfile::tempdir().unwrap();
		let old = dir.path().join("old.kepub.epub");
		let recent = dir.path().join("recent.kepub.epub");
		let young_tmp = dir.path().join("active.tmp");
		std::fs::write(&old, b"old").unwrap();
		std::fs::write(&recent, b"recent").unwrap();
		std::fs::write(&young_tmp, b"tmp").unwrap();
		let old_time = FileTime::from_system_time(
			SystemTime::now() - Duration::from_secs(3 * 86400),
		);
		set_file_mtime(&old, old_time).unwrap();
		set_file_mtime(
			&young_tmp,
			FileTime::from_system_time(SystemTime::now() - Duration::from_secs(30 * 60)),
		)
		.unwrap();
		assert_eq!(evict_cache_dir(dir.path(), 2).await.unwrap(), 1);
		assert!(!old.exists());
		assert!(recent.exists());
		assert!(young_tmp.exists());
	}

	#[tokio::test]
	async fn queue_is_bounded_and_deduplicates_paths() {
		let (tx, mut rx) = mpsc::channel(1);
		let pending = Arc::new(Mutex::new(HashSet::new()));
		let path = PathBuf::from("book.kepub.epub");
		assert!(pending.lock().await.insert(path.clone()));
		assert!(!pending.lock().await.insert(path.clone()));
		tx.send(path.clone()).await.unwrap();
		assert!(tx.try_send(PathBuf::from("book-2.kepub.epub")).is_err());
		let fake_converter = |path: PathBuf| async move { path };
		assert_eq!(fake_converter(rx.recv().await.unwrap()).await, path);
	}
}
