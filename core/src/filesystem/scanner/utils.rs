use std::{
	collections::{HashMap, HashSet, VecDeque},
	path::{Path, PathBuf},
	sync::Arc,
	time::Instant,
};

use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use models::txn::begin_write;
use models::{
	entity::{library_config, media, media_metadata, media_tag, series, tag},
	services::audio::{self as audio_service, AudioFacts},
	shared::enums::FileStatus,
};
use sea_orm::{
	prelude::*,
	sea_query::{OnConflict, Query},
	ActiveValue, DatabaseConnection, DatabaseTransaction, IntoActiveModel, Iterable, Set,
};
use tokio::task::spawn_blocking;

use crate::{
	config::StumpConfig,
	database::{get_insert_batch_size, SQLITE_BIND_LIMIT},
	error::{CoreError, CoreResult},
	event::CreatedMedia,
	filesystem::{
		media::{BuiltMedia, MediaBuilder},
		series::{BuiltSeries, SeriesBuilder},
	},
	job::JobServices,
	CoreEvent,
};
use stump_jobs::{JobContext, JobError, JobExecuteLog, JobProgress};
use stump_media::image::remove_thumbnails;
use stump_scanner::{BookVisitOperation, CustomVisitResult, TagCache};

pub(crate) const MAX_INSERT_CHUNK_SIZE: usize = 250;
pub(crate) async fn build_tag_cache(
	conn: &DatabaseConnection,
	all_tag_names: HashSet<String>,
) -> CoreResult<TagCache> {
	if all_tag_names.is_empty() {
		return Ok(TagCache::default());
	}

	let names: Vec<_> = all_tag_names.iter().cloned().collect();
	let mut map = HashMap::with_capacity(names.len());

	for chunk in names.chunks(SQLITE_BIND_LIMIT) {
		let rows = tag::Entity::find()
			.filter(tag::Column::Name.is_in(chunk.to_vec()))
			.all(conn)
			.await?;

		for row in rows {
			map.insert(row.name, row.id);
		}
	}

	let missing: Vec<tag::ActiveModel> = all_tag_names
		.iter()
		.filter(|name| !map.contains_key(*name))
		.map(|name| tag::ActiveModel {
			name: Set(name.clone()),
			..Default::default()
		})
		.collect();

	if !missing.is_empty() {
		let tag_cols = tag::Column::iter().count();
		let batch_size = get_insert_batch_size(tag_cols);
		for chunk in missing.chunks(batch_size) {
			let inserted = tag::Entity::insert_many(chunk.to_vec())
				.exec_with_returning_many(conn)
				.await
				.map_err(CoreError::from)?;

			for row in inserted {
				map.insert(row.name, row.id);
			}
		}
	}

	Ok(TagCache::from_entries(map))
}
pub(crate) enum BookVisitResult {
	Built(Box<BuiltMedia>),
	Custom(CustomVisitResult),
}

impl BookVisitResult {
	/// Returns the path or ID that provides context for a visit failure.
	fn error_ctx(&self) -> String {
		match self {
			Self::Built(result) => match result.media.path.clone().into_value() {
				Some(sea_orm::Value::String(Some(path))) => *path,
				_ => {
					tracing::warn!(?result, "Processed media has invalid path?");
					String::default()
				},
			},
			Self::Custom(result) => result.id.clone(),
		}
	}
}

/// build the `media_tag` lookup record for given book pulling from cache
pub(crate) fn build_tag_link_rows(
	media_id: &str,
	tag_names: &[String],
	cache: &TagCache,
) -> Vec<media_tag::ActiveModel> {
	tag_names
		.iter()
		.filter(|n| !n.is_empty())
		.filter_map(|n| cache.get(n))
		.map(|tag_id| media_tag::ActiveModel {
			media_id: Set(media_id.to_string()),
			tag_id: Set(tag_id),
			..Default::default()
		})
		.collect()
}
/// Inserts one batch of already-built media inside an open transaction.
///
/// The caller owns the transaction lifecycle; this helper keeps one-shot insertion on the same
/// media/tag/audio persistence path as regular scans.
pub(crate) async fn insert_media_in_txn(
	txn: &DatabaseTransaction,
	media_models: Vec<media::ActiveModel>,
	meta_models: Vec<media_metadata::ActiveModel>,
	tags_by_media: Vec<(String, Vec<String>)>,
	audio_by_media: Vec<(String, AudioFacts)>,
	tag_cache: &TagCache,
) -> CoreResult<()> {
	let media_batch_size = get_insert_batch_size(media::Column::iter().count());
	for batch in media_models.chunks(media_batch_size) {
		media::Entity::insert_many(batch.to_vec())
			.exec(txn)
			.await
			.map_err(CoreError::from)?;
	}

	let metadata_batch_size =
		get_insert_batch_size(media_metadata::Column::iter().count());
	for batch in meta_models.chunks(metadata_batch_size) {
		media_metadata::Entity::insert_many(batch.to_vec())
			.exec(txn)
			.await
			.map_err(CoreError::from)?;
	}

	for (media_id, facts) in audio_by_media {
		audio_service::replace_in(txn, &media_id, &facts)
			.await
			.map_err(CoreError::from)?;
	}

	let tag_links = tags_by_media
		.iter()
		.flat_map(|(media_id, tag_names)| {
			build_tag_link_rows(media_id, tag_names, tag_cache)
		})
		.collect::<Vec<_>>();
	let tag_batch_size = get_insert_batch_size(media_tag::Column::iter().count());
	for batch in tag_links.chunks(tag_batch_size) {
		media_tag::Entity::insert_many(batch.to_vec())
			.exec(txn)
			.await
			.map_err(CoreError::from)?;
	}

	Ok(())
}

pub(crate) async fn update_media(
	db: &DatabaseConnection,
	BuiltMedia {
		media,
		metadata,
		tags,
		audio,
	}: BuiltMedia,
) -> CoreResult<media::Model> {
	let txn = begin_write(db).await?;

	let updated_media = media.update(&txn).await?;

	if let Some(meta) = metadata {
		let on_conflict = OnConflict::new()
			.update_columns(media_metadata::Column::iter())
			.to_owned();
		media_metadata::Entity::insert(meta.into_active_model())
			.on_conflict(on_conflict)
			.exec(&txn)
			.await?;
	}

	// The audio facts are replaced wholesale in the same transaction as the
	// media row: a re-probe is the authority on a book's track split, and
	// `replace_in` is idempotent, so a rescan of an unchanged book writes the
	// same rows back.
	if let Some(facts) = audio.as_ref() {
		audio_service::replace_in(&txn, &updated_media.id, facts).await?;
	}

	ensure_tags_linked(&txn, &updated_media.id, &tags).await?;

	txn.commit().await?;

	Ok(updated_media)
}

/// Ensure each tag name in `tag_names` exists in the `tags` table and is linked to
/// `media_id` via `media_tags`. Creates tags that don't yet exist and adds missing
/// links; never removes existing links.
///
/// This is used during scans to intake tags from file metadata (e.g. ComicInfo.xml
/// `<Tags>`) without clobbering tags the user has manually assigned through the UI.
pub(crate) async fn ensure_tags_linked(
	txn: &DatabaseTransaction,
	media_id: &str,
	tag_names: &[String],
) -> CoreResult<()> {
	let desired: HashSet<String> = tag_names
		.iter()
		.filter(|n| !n.is_empty())
		.cloned()
		.collect();
	if desired.is_empty() {
		return Ok(());
	}

	let already_linked: Vec<tag::Model> =
		tag::Entity::find_for_media_id(media_id).all(txn).await?;
	let already_linked_names: HashSet<&str> =
		already_linked.iter().map(|t| t.name.as_str()).collect();

	let to_link: Vec<String> = desired
		.into_iter()
		.filter(|n| !already_linked_names.contains(n.as_str()))
		.collect();
	if to_link.is_empty() {
		return Ok(());
	}

	let existing_unlinked: Vec<tag::Model> = tag::Entity::find()
		.filter(tag::Column::Name.is_in(to_link.clone()))
		.all(txn)
		.await?;
	let existing_unlinked_names: HashSet<&str> =
		existing_unlinked.iter().map(|t| t.name.as_str()).collect();

	let to_create: Vec<tag::ActiveModel> = to_link
		.iter()
		.filter(|n| !existing_unlinked_names.contains(n.as_str()))
		.map(|name| tag::ActiveModel {
			name: Set(name.clone()),
			..Default::default()
		})
		.collect();

	let created = if to_create.is_empty() {
		Vec::new()
	} else {
		tag::Entity::insert_many(to_create)
			.exec_with_returning_many(txn)
			.await?
	};

	let new_link_ids: Vec<i32> = existing_unlinked
		.iter()
		.map(|t| t.id)
		.chain(created.iter().map(|t| t.id))
		.collect();

	if !new_link_ids.is_empty() {
		media_tag::Entity::insert_many(new_link_ids.into_iter().map(|tag_id| {
			media_tag::ActiveModel {
				media_id: Set(media_id.to_string()),
				tag_id: Set(tag_id),
				..Default::default()
			}
		}))
		.exec(txn)
		.await?;
	}

	Ok(())
}

pub(crate) async fn handle_book_visit_operation(
	db: &DatabaseConnection,
	result: BookVisitResult,
) -> CoreResult<()> {
	match result {
		BookVisitResult::Custom(custom) => {
			if let Some(mut meta) = custom.meta {
				let tags = meta.tags.take().unwrap_or_default();

				let txn = begin_write(db).await?;
				let active_model = media_metadata::ActiveModel {
					media_id: Set(Some(custom.id.clone())),
					..meta.into_active_model()
				};
				let updated_meta = active_model.update(&txn).await?;
				ensure_tags_linked(&txn, &custom.id, &tags).await?;
				txn.commit().await?;

				tracing::trace!(?updated_meta, "Metadata upserted");
			}

			if let Some(hashes) = custom.hashes {
				let affected_rows = media::Entity::update_many()
					.filter(media::Column::Id.eq(custom.id.clone()))
					.col_expr(media::Column::Hash, Expr::value(hashes.hash))
					.col_expr(
						media::Column::KoreaderHash,
						Expr::value(hashes.koreader_hash),
					)
					.exec(db)
					.await?
					.rows_affected;
				tracing::trace!(affected_rows, "Book updated with new hashes");
			}
		},
		BookVisitResult::Built(book) => {
			let updated_media = update_media(db, *book).await?;
			tracing::trace!(?updated_media, "Book updated");
		},
	}

	Ok(())
}

/// The outcome of marking series, and the books that belong to them, missing.
#[derive(Default)]
pub(crate) struct MissingSeriesOutput {
	pub updated_series: u64,
	pub updated_media: u64,
	pub logs: Vec<JobExecuteLog>,
}

/// Marks every series found at `paths` as missing, along with every book that
/// belongs to those series.
///
/// A book of a series that vanished from disk is just as gone as the series
/// itself, so the two statuses are written in the same pass: a scan can never
/// leave a `MISSING` series holding `READY` books, which is what previously let
/// a moved-away folder keep serving its books to every listing profile. The
/// inverse is handled by [`handle_restored_media`], which only promotes books
/// whose status `is_recovered_if_present`, so an `ERROR`/`UNSUPPORTED` book that
/// went missing does not come back as `READY`.
pub(crate) async fn handle_missing_series(
	client: &DatabaseConnection,
	paths: &[String],
) -> Result<MissingSeriesOutput, JobError> {
	let mut output = MissingSeriesOutput::default();

	if paths.is_empty() {
		tracing::debug!("No missing series to handle");
		return Ok(output);
	}

	for (index, chunk) in paths.chunks(SQLITE_BIND_LIMIT).enumerate() {
		let affected_series = series::Entity::update_many()
			.filter(series::Column::Path.is_in(chunk.to_vec()))
			.col_expr(
				series::Column::Status,
				Expr::value(FileStatus::Missing.to_string()),
			)
			.exec(client)
			.await
			.map_or_else(
				|error| {
					tracing::error!(chunk = index + 1, error = ?error, "Failed to update missing series");
					output.logs.push(JobExecuteLog::error(format!(
						"Failed to update missing series: {:?}",
						error.to_string()
					)));
					0
				},
				|res| res.rows_affected,
			);
		output.updated_series += affected_series;

		let affected_media = media::Entity::update_many()
			.filter(
				media::Column::SeriesId.in_subquery(
					Query::select()
						.column(series::Column::Id)
						.from(series::Entity)
						.and_where(series::Column::Path.is_in(chunk.to_vec()))
						.to_owned(),
				),
			)
			.col_expr(
				media::Column::Status,
				Expr::value(FileStatus::Missing.to_string()),
			)
			.exec(client)
			.await
			.map_or_else(
				|error| {
					tracing::error!(chunk = index + 1, error = ?error, "Failed to update missing media");
					output.logs.push(JobExecuteLog::error(format!(
						"Failed to update missing media: {:?}",
						error.to_string()
					)));
					0
				},
				|res| res.rows_affected,
			);
		output.updated_media += affected_media;
	}

	if output.updated_series > paths.len() as u64 {
		tracing::warn!(
			updated_series = output.updated_series,
			expected = paths.len(),
			"Updated more series than there were missing paths"
		);
	}

	tracing::debug!(
		updated_series = output.updated_series,
		updated_media = output.updated_media,
		"Marked series and their books as missing"
	);

	Ok(output)
}

/// The outcome of marking series ready again after they reappeared on disk.
#[derive(Default)]
pub(crate) struct RecoveredSeriesOutput {
	pub updated_series: u64,
	pub logs: Vec<JobExecuteLog>,
}

/// Marks every series with `ids` as ready again, because the walk found it back
/// on disk.
///
/// The update is guarded by the same predicate the walk uses to decide a series
/// is recovered (`FileStatus::is_recovered_if_present`), so the caller may pass
/// any scanned series: a `READY` series is not rewritten, and an `ERROR` series
/// is not promoted by a folder move. Only the series row is touched — the walk
/// is the only step that knows which of its books came back, so those are
/// restored by [`handle_restored_media`].
pub(crate) async fn handle_recovered_series(
	client: &DatabaseConnection,
	ids: &[String],
) -> Result<RecoveredSeriesOutput, JobError> {
	let mut output = RecoveredSeriesOutput::default();

	if ids.is_empty() {
		tracing::debug!("No recovered series to handle");
		return Ok(output);
	}

	for (index, chunk) in ids.chunks(SQLITE_BIND_LIMIT).enumerate() {
		let affected_series = series::Entity::update_many()
			.filter(series::Column::Id.is_in(chunk.to_vec()))
			.filter(
				series::Column::Status.is_in([FileStatus::Missing, FileStatus::Unknown]),
			)
			.col_expr(
				series::Column::Status,
				Expr::value(FileStatus::Ready.to_string()),
			)
			.exec(client)
			.await
			.map_or_else(
				|error| {
					tracing::error!(chunk = index + 1, error = ?error, "Failed to recover series");
					output.logs.push(JobExecuteLog::error(format!(
						"Failed to recover series: {:?}",
						error.to_string()
					)));
					0
				},
				|res| res.rows_affected,
			);
		output.updated_series += affected_series;
	}

	tracing::debug!(
		updated_series = output.updated_series,
		"Marked series as recovered"
	);

	Ok(output)
}

#[derive(Default)]
pub(crate) struct MediaOperationOutput {
	pub created_media: u64,
	pub updated_media: u64,
	pub logs: Vec<JobExecuteLog>,
}

/// Marks the books at `paths` as missing. A book is missing when its row still
/// exists but its file no longer does, whatever status the row held before.
pub(crate) async fn handle_missing_media(
	conn: &DatabaseConnection,
	series_id: &str,
	paths: Vec<PathBuf>,
) -> MediaOperationOutput {
	let mut output = MediaOperationOutput::default();

	if paths.is_empty() {
		tracing::debug!("No missing media to handle");
		return output;
	}

	let path_strings: Vec<String> = paths
		.iter()
		.map(|p| p.to_string_lossy().to_string())
		.collect();

	for (i, chunk) in path_strings.chunks(SQLITE_BIND_LIMIT).enumerate() {
		let _affected_rows = media::Entity::update_many()
			.filter(media::Column::SeriesId.eq(series_id.to_string()))
			.filter(media::Column::Path.is_in(chunk.to_vec()))
			.col_expr(
				media::Column::Status,
				Expr::value(FileStatus::Missing.to_string()),
			)
			.exec(conn)
			.await
			.map_or_else(
				|error| {
					tracing::error!(
						chunk = i + 1,
						?error,
						"Failed to update missing media chunk"
					);
					output.logs.push(JobExecuteLog::error(format!(
						"Failed to update missing media: {:?}",
						error.to_string()
					)));
					0
				},
				|res| {
					output.updated_media += res.rows_affected;
					res.rows_affected
				},
			);
	}

	output
}

/// Marks the books with `ids` as ready again. The caller passes only the books
/// the walk found back on disk whose status `is_recovered_if_present`, so a book
/// that failed to build is never silently promoted to `READY`.
pub(crate) async fn handle_restored_media(
	conn: &DatabaseConnection,
	series_id: &str,
	ids: Vec<String>,
) -> MediaOperationOutput {
	let mut output = MediaOperationOutput::default();

	if ids.is_empty() {
		tracing::debug!("No restored media to handle");
		return output;
	}

	let id_strings: Vec<String> = ids.iter().map(|id| id.to_string()).collect();

	for (i, chunk) in id_strings.chunks(SQLITE_BIND_LIMIT).enumerate() {
		let _affected_series = media::Entity::update_many()
			.filter(media::Column::SeriesId.eq(series_id.to_string()))
			.filter(media::Column::Id.is_in(chunk.to_vec()))
			.col_expr(
				media::Column::Status,
				Expr::value(FileStatus::Ready.to_string()),
			)
			.exec(conn)
			.await
			.map_or_else(
				|error| {
					tracing::error!(
						chunk = i + 1,
						?error,
						"Failed to update restored media chunk"
					);
					output.logs.push(JobExecuteLog::error(format!(
						"Failed to update restored media: {:?}",
						error.to_string()
					)));
					0
				},
				|res| {
					output.updated_media += res.rows_affected;
					res.rows_affected
				},
			);
	}

	output
}

/// Builds a series from the given path
///
/// # Arguments
/// * `for_library` - The library ID to associate the series with
/// * `path` - The path to the series on disk
async fn build_series(for_library: &str, path: &Path) -> CoreResult<BuiltSeries> {
	let path = path.to_path_buf();
	let for_library = for_library.to_string();

	// Spawn a blocking task to handle the IO-intensive operations:
	spawn_blocking(move || SeriesBuilder::new(&path, &for_library).build())
		.await
		.map_err(|e| CoreError::Unknown(e.to_string()))?
}

/// a type alias for the unordered stream of futures returned by concurernt builds of
/// managed entities
pub(crate) type BuiltEntityFutures<T, R = PathBuf> =
	FuturesUnordered<BoxFuture<'static, Result<T, (CoreError, R)>>>;

/// Safely builds a series from a list of paths concurrently, with a maximum concurrency limit
/// derived from available CPU threads
///
/// # Arguments
/// * `for_library` - The library ID to associate the series with
/// * `paths` - A list of paths to build series from
/// * `reporter` - A function to report progress to the UI
pub(crate) async fn safely_build_series(
	for_library: &str,
	paths: Vec<PathBuf>,
	config: Arc<StumpConfig>,
	reporter: impl Fn(usize),
) -> (Vec<BuiltSeries>, Vec<JobExecuteLog>) {
	let mut logs = vec![];
	let mut created_series = Vec::with_capacity(paths.len());

	let concurrency = config.jobs.cpu_concurrency_limit();
	let total_series = paths.len();
	tracing::debug!(total_series, concurrency, "Processing series");

	let start = Instant::now();
	let mut futures: BuiltEntityFutures<BuiltSeries> = FuturesUnordered::new();
	let mut cursor = 0usize;

	for path in paths {
		if futures.len() >= concurrency {
			if let Some(result) = futures.next().await {
				match result {
					Ok(series) => {
						created_series.push(series);
					},
					Err((error, path)) => {
						logs.push(
							JobExecuteLog::error(format!(
								"Failed to build series: {:?}",
								error.to_string()
							))
							.with_ctx(format!("Path: {path:?}")),
						);
					},
				}
				reporter(cursor);
				cursor += 1;
			}
		}

		let for_library = for_library.to_string();
		futures.push(Box::pin(async move {
			tracing::trace!(?path, "Starting series build");
			build_series(&for_library, &path)
				.await
				.map_err(|e| (e, path.clone()))
		}));
	}

	while let Some(result) = futures.next().await {
		match result {
			Ok(series) => {
				created_series.push(series);
			},
			Err((error, path)) => {
				logs.push(
					JobExecuteLog::error(format!(
						"Failed to build series: {:?}",
						error.to_string()
					))
					.with_ctx(format!("Path: {path:?}")),
				);
			},
		}
		reporter(cursor);
		cursor += 1;
	}

	let success_count = created_series.len();
	let error_count = logs.len();
	tracing::debug!(elapsed = ?start.elapsed(), success_count, error_count, "Finished batch of series");

	(created_series, logs)
}

pub(crate) async fn safely_insert_series(
	series: Vec<BuiltSeries>,
	conn: &DatabaseConnection,
) -> Result<Vec<series::Model>, JobError> {
	let mut output = Vec::with_capacity(series.len());

	let txn = begin_write(conn).await?;

	for BuiltSeries { series, metadata } in series {
		let created_series = series.insert(&txn).await?;

		// I opted to not kill the transaction if metadata insertion fails, I figure this
		// is a best-effort operation and we can always try again later after fixing a bad
		// metadata entry vs killing the entire series creation process over a single bad entry
		if let Some(mut meta) = metadata {
			meta.series_id = Set(created_series.id.clone());
			if let Err(error) = meta.insert(&txn).await {
				tracing::error!(?error, "Failed to insert series metadata");
			}
		}

		output.push(created_series);
	}

	txn.commit().await?;
	tracing::debug!(series_count = output.len(), "Inserted series into database");
	Ok(output)
}

// TODO(granular-scans): intake ScanOptions
pub(crate) struct MediaBuildOperation {
	pub series_id: String,
	pub library_config: library_config::Model,
}

/// Builds a media from the given path
///
/// # Arguments
/// * `path` - The path to the media on disk
/// * `series_id` - The series ID to associate the media with
/// * `existing_book` - An optional existing media to rebuild
/// * `library_config` - The library configuration
/// * `config` - The core configuration
async fn build_book(
	path: &Path,
	series_id: &str,
	existing_book: Option<media::ModelWithMetadata>,
	library_config: library_config::Model,
	config: &StumpConfig,
) -> CoreResult<BuiltMedia> {
	let path = path.to_path_buf();
	let series_id = series_id.to_string();
	let library_config = library_config.clone();
	let config = config.clone();

	// Spawn a blocking task to handle the IO-intensive operations:
	spawn_blocking({
		move || {
			let builder = MediaBuilder::new(&path, &series_id, library_config, &config);
			if let Some(existing_book) = existing_book {
				builder.rebuild(&existing_book)
			} else {
				builder.build()
			}
		}
	})
	.await
	.map_err(|e| CoreError::Unknown(e.to_string()))?
}

struct BookVisitCtx {
	operation: BookVisitOperation,
	path: PathBuf,
	series_id: String,
	existing_book: Option<media::ModelWithMetadata>,
}

async fn handle_book(
	BookVisitCtx {
		path,
		operation,
		series_id,
		existing_book,
	}: BookVisitCtx,
	library_config: library_config::Model,
	config: &StumpConfig,
) -> CoreResult<BookVisitResult> {
	let path = path.to_path_buf();
	let series_id = series_id.to_string();
	let library_config = library_config.clone();
	let config = config.clone();

	// Spawn a blocking task to handle the IO-intensive operations:
	spawn_blocking({
		move || {
			let builder = MediaBuilder::new(&path, &series_id, library_config, &config);
			match (operation, existing_book) {
				(BookVisitOperation::Rebuild, Some(book)) => builder
					.rebuild(&book)
					.map(|b| BookVisitResult::Built(Box::new(b))),
				(BookVisitOperation::Custom(custom), Some(book)) => {
					builder.custom_visit(custom).map(|result| {
						BookVisitResult::Custom(CustomVisitResult {
							id: book.media.id,
							..result
						})
					})
				},
				// If the existing book is None, it means the book doesn't yet exist so we
				// always just do a full build. However, we really shouldn't be in this state
				// since media creation is handled in a separate flow than visit
				(_, None) => builder.build().map(|b| BookVisitResult::Built(Box::new(b))),
			}
		}
	})
	.await
	.map_err(|e| CoreError::Unknown(e.to_string()))?
}

/// Safely builds media from a list of paths concurrently, with a maximum concurrency limit
/// as defined by the core configuration. The media is then inserted into the database.
///
/// # Arguments
/// * `MediaBuildOperation` - The operation configuration for building media
/// * `worker_ctx` - The worker context
/// * `paths` - A list of paths to build media from
pub(crate) async fn safely_build_and_insert_media(
	MediaBuildOperation {
		series_id,
		library_config,
	}: MediaBuildOperation,
	worker_ctx: &JobContext<JobServices>,
	paths: Vec<PathBuf>,
) -> Result<MediaOperationOutput, JobError> {
	if paths.is_empty() {
		tracing::trace!("No media to create?");
		return Ok(MediaOperationOutput::default());
	}

	let mut output = MediaOperationOutput::default();

	let Some(library_id) = library_config.library_id.clone() else {
		tracing::error!(?library_config, "Library config has no library ID?");
		output.logs.push(JobExecuteLog::error(format!(
			"Library config has no library ID: {:?}",
			library_config.id
		)));
		return Ok(output);
	};

	let concurrency = worker_ctx.config().jobs.cpu_concurrency_limit();
	let book_count = paths.len();
	tracing::debug!(book_count, concurrency, "Processing media");

	let start = Instant::now();
	let mut books = VecDeque::with_capacity(book_count);

	worker_ctx.report_progress(JobProgress::msg("Building media from disk"));

	let config_arc = Arc::clone(&worker_ctx.services().config);
	let mut futures: BuiltEntityFutures<BuiltMedia> = FuturesUnordered::new();
	let mut cursor = 0i32;

	for path in paths {
		if futures.len() >= concurrency {
			if let Some(result) = futures.next().await {
				match result {
					Ok(book) => {
						books.push_back(book);
					},
					Err((error, path)) => {
						tracing::error!(error = ?error, ?path, "Failed to build book");
						output.logs.push(
							JobExecuteLog::error(format!(
								"Failed to build book: {:?}",
								error.to_string()
							))
							.with_ctx(format!("Path: {path:?}")),
						);
					},
				}
				cursor += 1;
				worker_ctx.report_progress(JobProgress::subtask_position(
					cursor,
					book_count as i32,
				));
			}
		}

		let series_id = series_id.clone();
		let library_config = library_config.clone();
		let config = Arc::clone(&config_arc);

		futures.push(Box::pin(async move {
			tracing::trace!(?path, "Starting media build");
			build_book(&path, &series_id, None, library_config, &config)
				.await
				.map_err(|e| (e, path.clone()))
		}));
	}

	while let Some(result) = futures.next().await {
		match result {
			Ok(book) => {
				books.push_back(book);
			},
			Err((error, path)) => {
				tracing::error!(error = ?error, ?path, "Failed to build book");
				output.logs.push(
					JobExecuteLog::error(format!(
						"Failed to build book: {:?}",
						error.to_string()
					))
					.with_ctx(format!("Path: {path:?}")),
				);
			},
		}
		cursor += 1;
		worker_ctx
			.report_progress(JobProgress::subtask_position(cursor, book_count as i32));
	}

	let success_count = books.len();
	let error_count = output.logs.len();
	tracing::debug!(
		elapsed = ?start.elapsed(),
		success_count, error_count,
		"Built books from disk"
	);

	worker_ctx.report_progress(JobProgress::msg("Inserting books into database"));
	let task_count = books.len() as i32;
	let start = Instant::now();

	let media_cols_count = media::Column::iter().count();
	let media_metadata_cols_count = media_metadata::Column::iter().count();
	let media_tag_cols_count = media_tag::Column::iter().count();

	let all_tag_names: HashSet<_> =
		books.iter().flat_map(|b| b.tags.iter().cloned()).collect();
	let tag_cache = build_tag_cache(worker_ctx.conn(), all_tag_names).await?;

	let mut insert_cursor = 0i32;

	while !books.is_empty() {
		let txn = begin_write(worker_ctx.conn()).await?;

		let chunk_count = MAX_INSERT_CHUNK_SIZE.min(books.len());

		let mut media_models = Vec::with_capacity(chunk_count);
		let mut meta_models: Vec<media_metadata::ActiveModel> = Vec::new();
		let mut tags_by_media: Vec<(String, Vec<String>)> = Vec::new();
		let mut audio_by_media: Vec<(String, AudioFacts)> = Vec::new();
		let mut inserted_ids: Vec<String> = Vec::with_capacity(chunk_count);

		for _ in 0..chunk_count {
			let Some(BuiltMedia {
				media,
				metadata,
				tags,
				audio,
			}) = books.pop_front()
			else {
				break;
			};

			let media_id = match media.id.clone() {
				ActiveValue::Set(id) | ActiveValue::Unchanged(id) => id,
				ActiveValue::NotSet => {
					// this should not really happen but i want the log without killing
					// the entire batch
					tracing::warn!(?media, "Media built without an id, skipping");
					continue;
				},
			};

			inserted_ids.push(media_id.clone());
			media_models.push(media);

			if let Some(meta) = metadata {
				meta_models.push(meta);
			}

			if let Some(facts) = audio {
				audio_by_media.push((media_id.clone(), facts));
			}

			if !tags.is_empty() {
				tags_by_media.push((media_id, tags));
			}
		}

		let media_batch_size = get_insert_batch_size(media_cols_count);
		for batch in media_models.chunks(media_batch_size) {
			media::Entity::insert_many(batch.to_vec())
				.exec(&txn)
				.await
				.map_err(CoreError::from)?;
		}

		if !meta_models.is_empty() {
			let meta_batch_size = get_insert_batch_size(media_metadata_cols_count);
			for batch in meta_models.chunks(meta_batch_size) {
				media_metadata::Entity::insert_many(batch.to_vec())
					.exec(&txn)
					.await
					.map_err(CoreError::from)?;
			}
		}

		// After the media rows exist, so the `media_audio.media_id` foreign
		// key resolves, and inside the same transaction: a book is never
		// visible without its tracks.
		for (media_id, facts) in &audio_by_media {
			audio_service::replace_in(&txn, media_id, facts)
				.await
				.map_err(CoreError::from)?;
		}

		let mut tag_links: Vec<media_tag::ActiveModel> = Vec::new();
		for (media_id, tag_names) in &tags_by_media {
			tag_links.extend(build_tag_link_rows(media_id, tag_names, &tag_cache));
		}
		if !tag_links.is_empty() {
			let tag_batch_size = get_insert_batch_size(media_tag_cols_count);
			for batch in tag_links.chunks(tag_batch_size) {
				media_tag::Entity::insert_many(batch.to_vec())
					.exec(&txn)
					.await
					.map_err(CoreError::from)?;
			}
		}

		txn.commit().await?;

		// TODO(metadata-fetching): Track inserted_ids as needing fetching (assuming enabled)
		for media_id in inserted_ids {
			output.created_media += 1;
			insert_cursor += 1;
			worker_ctx.report_progress(JobProgress::subtask_position(
				insert_cursor,
				task_count,
			));
			worker_ctx.emit_event(CoreEvent::CreatedMedia(CreatedMedia {
				id: media_id,
				series_id: series_id.clone(),
				library_id: library_id.clone(),
			}));
		}
	}

	let success_count = output.created_media;
	let error_count = output.logs.len() - error_count; // Subtract the errors from the previous step
	tracing::debug!(success_count, error_count, elapsed = ?start.elapsed(), "Inserted books into database");

	Ok(output)
}

/// Visits the media on disk and updates the database with the latest information. This is done
/// concurrently with a maximum concurrency limit as defined by the core configuration.
///
/// # Arguments
/// * `MediaBuildOperation` - The operation configuration for visiting media
/// * `worker_ctx` - The worker context
/// * `params` - A list of paths and operations to visit
pub(crate) async fn visit_and_update_media(
	MediaBuildOperation {
		series_id,
		library_config,
	}: MediaBuildOperation,
	worker_ctx: &JobContext<JobServices>,
	params: Vec<(PathBuf, BookVisitOperation)>,
) -> Result<MediaOperationOutput, JobError> {
	let mut output = MediaOperationOutput::default();

	if params.is_empty() {
		tracing::trace!("No media to visit?");
		return Ok(output);
	}

	let conn = worker_ctx.conn();
	let paths_to_operation = params
		.iter()
		.map(|(p, o)| (p.to_string_lossy().to_string(), *o))
		.collect::<HashMap<_, _>>();
	let paths = paths_to_operation.keys().cloned().collect::<Vec<String>>();
	let paths_len = paths.len();

	let mut media = Vec::with_capacity(paths_len);
	for chunk in paths.chunks(SQLITE_BIND_LIMIT) {
		let batch = media::ModelWithMetadata::find()
			.filter(media::Column::Path.is_in(chunk.to_vec()))
			.filter(media::Column::SeriesId.eq(series_id.to_string()))
			.into_model::<media::ModelWithMetadata>()
			.all(conn)
			.await?;
		media.extend(batch);
	}

	if media.len() != paths_len {
		output.logs.push(JobExecuteLog::warn(
			"Not all media paths were found in the database",
		));
	}

	let concurrency = worker_ctx.config().jobs.cpu_concurrency_limit();
	let book_count = media.len();
	tracing::debug!(book_count, concurrency, "Processing media visit");

	let start = Instant::now();
	let mut build_results = VecDeque::with_capacity(book_count);

	worker_ctx.report_progress(JobProgress::msg("Visiting media on disk"));

	let config_arc = Arc::clone(&worker_ctx.services().config);
	let mut futures: BuiltEntityFutures<BookVisitResult, String> =
		FuturesUnordered::new();
	let mut cursor = 0i32;
	// A book rebuilt from a changed file whose thumbnail was only ever
	// generated on request (no stored `thumbnail_path`) must render its new
	// first page next time, exactly as the raw-page fallback used to.
	let mut rebuilt_on_demand_thumbnails = Vec::new();
	for book in media {
		let path = book.media.path.clone();
		let Some(operation) = paths_to_operation.get(&path) else {
			tracing::warn!(?path, "No operation found for media?");
			continue;
		};
		if matches!(operation, BookVisitOperation::Rebuild)
			&& book.media.thumbnail_path.is_none()
		{
			rebuilt_on_demand_thumbnails.push(book.media.id.clone());
		}

		if futures.len() >= concurrency {
			if let Some(future_result) = futures.next().await {
				match future_result {
					Ok(result) => {
						build_results.push_back(result);
					},
					Err((error, path)) => {
						output.logs.push(
							JobExecuteLog::error(format!(
								"Failed to handle book: {:?}",
								error.to_string()
							))
							.with_ctx(format!("Path: {path:?}")),
						);
					},
				}
				cursor += 1;
				worker_ctx.report_progress(JobProgress::subtask_position(
					cursor,
					book_count as i32,
				));
			}
		}

		let ctx = BookVisitCtx {
			operation: *operation,
			existing_book: Some(book),
			series_id: series_id.clone(),
			path: PathBuf::from(path.as_str()),
		};
		let library_config = library_config.clone();
		let config = Arc::clone(&config_arc);
		futures.push(Box::pin(async move {
			tracing::trace!(?path, "Starting media visit");
			handle_book(ctx, library_config, &config)
				.await
				.map_err(|e| (e, path.clone()))
		}));
	}

	while let Some(future_result) = futures.next().await {
		match future_result {
			Ok(result) => {
				build_results.push_back(result);
			},
			Err((error, path)) => {
				output.logs.push(
					JobExecuteLog::error(format!(
						"Failed to handle book: {:?}",
						error.to_string()
					))
					.with_ctx(format!("Path: {path:?}")),
				);
			},
		}
		cursor += 1;
		worker_ctx
			.report_progress(JobProgress::subtask_position(cursor, book_count as i32));
	}

	let success_count = build_results.len();
	let error_count = output.logs.len();
	tracing::debug!(elapsed = ?start.elapsed(), success_count, error_count, "Handled books from disk");

	worker_ctx.report_progress(JobProgress::msg("Updating media in database"));
	let task_count = build_results.len() as i32;
	let start = Instant::now();

	let mut update_cursor = 0i32;

	while let Some(result) = build_results.pop_front() {
		let error_ctx = result.error_ctx();
		match handle_book_visit_operation(worker_ctx.conn(), result).await {
			Ok(_) => {
				output.updated_media += 1;
			},
			Err(e) => {
				tracing::error!(error = ?e, ?error_ctx, "Failed to update media");
				output.logs.push(
					JobExecuteLog::error(format!(
						"Failed to update media: {:?}",
						e.to_string()
					))
					.with_ctx(error_ctx),
				);
			},
		}

		update_cursor += 1;
		worker_ctx
			.report_progress(JobProgress::subtask_position(update_cursor, task_count));
	}

	let success_count = output.updated_media;
	let error_count = output.logs.len() - error_count; // Subtract the errors from the previous step
	tracing::debug!(elapsed = ?start.elapsed(), success_count, error_count, "Updated books in database");

	if !rebuilt_on_demand_thumbnails.is_empty() {
		if let Err(error) = remove_thumbnails(
			&rebuilt_on_demand_thumbnails,
			&config_arc.get_thumbnails_dir(),
		)
		.await
		{
			tracing::warn!(
				?error,
				"Could not drop on-demand thumbnails of rebuilt books; they may be stale until regenerated"
			);
		}
	}

	Ok(output)
}

// TODO(tests): sort out tests later. I had to remove them for now because
// mocking apalis state and all that was too much

/// The scan write path for an audiobook, against the real migrated schema.
///
/// The unit tests in `models` exercise `AudioBook`'s lookup helpers on
/// hand-built rows, so nothing covered an actual `replace_in` round trip —
/// which is how a `NOT NULL constraint failed: media_audio_tracks.id` shipped
/// (`Entity::insert_many` never runs `ActiveModelBehavior::before_save`).
#[cfg(test)]
mod audio_persistence {
	use super::*;
	use migrations::{Migrator, MigratorTrait};
	use models::{
		domain::audio::AudioChapterSource,
		services::audio::{ChapterFacts, TrackFacts},
	};
	use sea_orm::{ActiveModelTrait, Database};

	/// `durations` becomes one track per part; the chapter marks tile them.
	fn facts(durations: &[i64]) -> AudioFacts {
		let mut start_ms = 0_i64;
		let (tracks, chapters) = durations
			.iter()
			.enumerate()
			.map(|(index, duration_ms)| {
				let start = start_ms;
				start_ms += duration_ms;
				(
					TrackFacts {
						path: format!("/books/Book Vol. 1/{:02}.mp3", index + 1),
						duration_ms: *duration_ms,
						byte_size: 2_108,
						mime: "audio/mpeg".to_string(),
					},
					ChapterFacts {
						title: Some(format!("Part {}", index + 1)),
						start_ms: start,
						end_ms: Some(start_ms),
					},
				)
			})
			.collect::<(Vec<_>, Vec<_>)>();

		AudioFacts {
			duration_ms: start_ms,
			codec: "mp3".to_string(),
			sample_rate: Some(44_100),
			channels: Some(2),
			bitrate: Some(64_000),
			chapter_source: AudioChapterSource::PerTrack,
			tracks,
			chapters,
		}
	}

	/// A scanned audiobook is only usable if its tracks reach the database:
	/// `start_offset_ms` is what maps a publication-relative position to one
	/// file, and a re-probe is the authority on the split, so a rescan that
	/// splits differently must leave no row of the old split behind.
	#[tokio::test]
	async fn update_media_persists_and_replaces_audio_facts() {
		let db = Database::connect("sqlite::memory:").await.unwrap();
		Migrator::up(&db, None).await.unwrap();

		let inserted = media::ActiveModel {
			id: Set("media-1".to_string()),
			name: Set("Book Vol. 1".to_string()),
			size: Set(6_324),
			extension: Set("mp3".to_string()),
			pages: Set(-1),
			path: Set("/books/Book Vol. 1".to_string()),
			status: Set(FileStatus::Ready),
			created_at: Set(chrono::Utc::now().into()),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();

		let built = |audio| BuiltMedia {
			media: media::ActiveModel {
				id: Set(inserted.id.clone()),
				name: Set("Book Vol. 1".to_string()),
				..Default::default()
			},
			metadata: None,
			tags: Vec::new(),
			audio,
		};

		update_media(&db, built(Some(facts(&[2_000, 3_000, 1_500]))))
			.await
			.unwrap();
		let book = audio_service::book(&db, &inserted.id)
			.await
			.unwrap()
			.expect("the scan wrote media_audio");

		assert_eq!(book.audio.duration_ms, 6_500);
		assert_eq!(book.audio.chapter_source, AudioChapterSource::PerTrack);
		// Contiguous indexes and the running sum of the durations, which is
		// what `AudioBook::track_at` bisects.
		assert_eq!(
			book.tracks
				.iter()
				.map(|track| track.index)
				.collect::<Vec<_>>(),
			[0, 1, 2]
		);
		assert_eq!(
			book.tracks
				.iter()
				.map(|track| track.start_offset_ms)
				.collect::<Vec<_>>(),
			[0, 2_000, 5_000]
		);
		assert_eq!(book.chapters.len(), 3);

		// A re-probe that splits the same book into two parts must replace the
		// three-track split outright: a leftover track would claim an index
		// the new split no longer has.
		update_media(&db, built(Some(facts(&[4_000, 2_500]))))
			.await
			.unwrap();
		let rescanned = audio_service::book(&db, &inserted.id)
			.await
			.unwrap()
			.expect("the rescan kept media_audio");

		assert_eq!(rescanned.audio.duration_ms, 6_500);
		assert_eq!(
			rescanned
				.tracks
				.iter()
				.map(|track| (track.index, track.start_offset_ms))
				.collect::<Vec<_>>(),
			[(0, 0), (1, 4_000)]
		);
		assert_eq!(rescanned.chapters.len(), 2);
	}
}

/// The status a scan leaves behind for a series that vanished from disk, and for
/// the same series once it is put back.
///
/// A `MISSING` series used to keep `READY` books: every listing funnel filters
/// on the book status, not the series status, so a moved-away folder went on
/// serving books whose files were gone (and reading one 404s at the file read).
#[cfg(test)]
mod missing_series_lifecycle {
	use super::super::store::SeaOrmScanSource;
	use super::*;
	use globset::GlobSet;
	use migrations::{Migrator, MigratorTrait};
	use sea_orm::{ActiveModelTrait, Database};
	use stump_scanner::{walk_library, walk_series, ScanOptions, WalkerCtx};
	use tempfile::TempDir;

	const LIBRARY_ID: &str = "library-1";
	const SERIES_ID: &str = "series-1";
	const MEDIA_ID: &str = "media-1";

	fn walker_ctx(series_id: Option<&str>) -> WalkerCtx {
		WalkerCtx {
			ignore_rules: GlobSet::empty(),
			max_depth: None,
			options: ScanOptions::default(),
			dir_mtimes: Arc::new(HashMap::new()),
			library_id: LIBRARY_ID.to_string(),
			series_id: series_id.map(str::to_string),
			oneshots_directory: None,
		}
	}

	/// The pair a client actually sees: the series status and its book's status.
	async fn statuses(conn: &DatabaseConnection) -> (FileStatus, FileStatus) {
		let series = series::Entity::find_by_id(SERIES_ID)
			.one(conn)
			.await
			.unwrap()
			.expect("the series row survives a scan");
		let book = media::Entity::find_by_id(MEDIA_ID)
			.one(conn)
			.await
			.unwrap()
			.expect("the book row survives a scan");
		(series.status, book.status)
	}

	#[tokio::test]
	async fn missing_series_takes_its_books_with_it_and_gives_them_back() {
		let temp = TempDir::new().unwrap();
		let library_root = temp.path().join("library");
		let series_path = library_root.join("Some Series");
		std::fs::create_dir_all(&series_path).unwrap();
		let book_path = series_path.join("book.cbz");
		std::fs::write(&book_path, b"pretend this is a cbz").unwrap();
		// Outside the library root, so the walk sees the series as gone rather
		// than as a new series at a new path.
		let stash = temp.path().join("stash");
		std::fs::create_dir_all(&stash).unwrap();
		let stashed_series = stash.join("Some Series");

		let db = Arc::new(Database::connect("sqlite::memory:").await.unwrap());
		Migrator::up(db.as_ref(), None).await.unwrap();

		tests::fake_data::Library {
			id: Some(LIBRARY_ID.to_string()),
			name: Some("Missing Series Library".to_string()),
			path: Some(library_root.to_string_lossy().to_string()),
		}
		.insert(db.as_ref())
		.await;

		tests::fake_data::Series {
			id: Some(SERIES_ID.to_string()),
			name: Some("Some Series".to_string()),
			path: Some(series_path.to_string_lossy().to_string()),
			library_id: Some(LIBRARY_ID.to_string()),
		}
		.insert(db.as_ref())
		.await;

		media::ActiveModel {
			id: Set(MEDIA_ID.to_string()),
			name: Set("book".to_string()),
			size: Set(21),
			extension: Set("cbz".to_string()),
			pages: Set(1),
			path: Set(book_path.to_string_lossy().to_string()),
			status: Set(FileStatus::Ready),
			series_id: Set(Some(SERIES_ID.to_string())),
			created_at: Set(chrono::Utc::now().into()),
			..Default::default()
		}
		.insert(db.as_ref())
		.await
		.unwrap();

		let source = SeaOrmScanSource::new(Arc::clone(&db));
		let db = db.as_ref();
		let library_root_str = library_root.to_string_lossy().to_string();

		// The folder is moved out from under the scanner.
		std::fs::rename(&series_path, &stashed_series).unwrap();

		let walked = walk_library(&library_root_str, &source, walker_ctx(None))
			.await
			.unwrap();
		assert_eq!(walked.missing_series, vec![series_path.clone()]);
		// Nothing re-walks a missing series, so this is the only step that can
		// mark its books.
		assert!(walked.series_to_visit.is_empty());

		let missing_paths = walked
			.missing_series
			.iter()
			.map(|path| path.to_string_lossy().to_string())
			.collect::<Vec<_>>();
		let missing = handle_missing_series(db, &missing_paths).await.unwrap();
		assert_eq!(missing.updated_series, 1);
		assert_eq!(missing.updated_media, 1);
		assert!(missing.logs.is_empty());
		assert_eq!(
			statuses(db).await,
			(FileStatus::Missing, FileStatus::Missing)
		);

		// ...and put back.
		std::fs::rename(&stashed_series, &series_path).unwrap();

		let rewalked = walk_library(&library_root_str, &source, walker_ctx(None))
			.await
			.unwrap();
		assert_eq!(rewalked.recovered_series, vec![SERIES_ID.to_string()]);
		assert_eq!(rewalked.series_to_visit, vec![series_path.clone()]);
		assert!(rewalked.missing_series.is_empty());

		let recovered = handle_recovered_series(db, &rewalked.recovered_series)
			.await
			.unwrap();
		assert_eq!(recovered.updated_series, 1);
		// `SeriesScanJob` calls this for every series it scans, so a series that
		// is already `READY` must not be rewritten.
		assert_eq!(
			handle_recovered_series(db, &[SERIES_ID.to_string()])
				.await
				.unwrap()
				.updated_series,
			0
		);
		// The series is back but its book is still MISSING until the series walk
		// reports it, which is the half that was never verified.
		assert_eq!(statuses(db).await, (FileStatus::Ready, FileStatus::Missing));

		let walked_series =
			walk_series(&series_path, &source, walker_ctx(Some(SERIES_ID)))
				.await
				.unwrap();
		assert!(!walked_series.series_is_missing);
		assert_eq!(walked_series.recovered_media, vec![MEDIA_ID.to_string()]);
		assert!(walked_series.missing_media.is_empty());
		// The file is unchanged since the row was written, so no rebuild is
		// queued: restoring the status is the whole recovery.
		assert!(walked_series.media_to_create.is_empty());

		let restored =
			handle_restored_media(db, SERIES_ID, walked_series.recovered_media).await;
		assert_eq!(restored.updated_media, 1);
		assert!(restored.logs.is_empty());
		assert_eq!(statuses(db).await, (FileStatus::Ready, FileStatus::Ready));
	}
}
