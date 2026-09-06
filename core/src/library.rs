//! Library create/update/delete as one shared path.
//!
//! Both the GraphQL mutations (`crates/graphql/src/mutation/library.rs`) and the
//! provider adapters (`apps/server/src/routers/komga_backend.rs` on behalf of the
//! Komga profile) call into this module, so validation, persistence, watcher and
//! scheduled-scan wiring, core-event emission, and post-commit side effects can
//! never drift between surfaces.
use models::txn::begin_write;
use models::{
	entity::{
		library, library_config, library_tag, media, scheduled_job, series, tag,
		user::AuthUser,
	},
	services::{
		library::{add_trailing_slash, normalize_path, path_within_roots},
		lists, tags as tag_service,
	},
	shared::enums::{FileStatus, ScheduledJobKind},
};
use sea_orm::{
	prelude::*, sea_query::Query, ActiveModelTrait, ColumnTrait, ConnectionTrait,
	DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter, QuerySelect, Set,
	Statement, Value,
};
use stump_media::image::{remove_thumbnails, ImageProcessorOptionsExt};
use uuid::Uuid;

use crate::{
	context::Ctx,
	error::{CoreError, CoreResult},
	event::{CoreEvent, LibraryCreated, LibraryDeleted, LibraryUpdated},
	job::stump_job::StumpJob,
};

/// Whether an update should add, remove, or leave the library filesystem watcher.
///
/// The GraphQL surface derives this from the submitted config (`watch` flag);
/// the Komga surface only expresses it when the patch touches configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchUpdate {
	/// Attach the watcher to the library root after the update.
	Add,
	/// Detach the watcher from the library's previous root after the update.
	Remove,
	/// Leave watcher state untouched.
	Keep,
}

/// Values for a library to be created.
pub struct NewLibrary {
	pub name: String,
	pub path: String,
	pub description: Option<String>,
	pub emoji: Option<String>,
	/// Configuration row values; `library_id` is wired by the service.
	pub config: library_config::ActiveModel,
	/// Tag names to create (if missing) and link to the new library.
	pub tags: Vec<String>,
	/// Enqueue an initial scan once the rows are committed.
	pub scan_after_persist: bool,
}

/// Values for an update to an existing library.
///
/// `name`, `path`, `description`, and `emoji` are the resolved final values;
/// partial-update callers merge them with the existing row before calling.
pub struct UpdatedLibrary {
	pub name: String,
	pub path: String,
	pub description: Option<String>,
	pub emoji: Option<String>,
	/// Full replacement config row values when present; the row is left
	/// untouched when `None`.
	pub config: Option<library_config::ActiveModel>,
	/// Resync library tags when present.
	pub tags: Option<Vec<String>>,
	/// Enqueue a scan once the rows are committed.
	pub scan_after_persist: bool,
	pub watch: WatchUpdate,
}

/// Creates a library (config row + library row + tags) and, on success, wires
/// the initial scan, the watcher, and the `LibraryCreated` core event.
pub async fn create_library(ctx: &Ctx, params: NewLibrary) -> CoreResult<library::Model> {
	if params.scan_after_persist {
		ctx.require_background_jobs()?;
	}

	enforce_valid_library_path(
		ctx.conn.as_ref(),
		&params.path,
		None,
		&ctx.config.server.library_roots,
	)
	.await?;
	enforce_unique_library_name(ctx.conn.as_ref(), &params.name, None).await?;
	validate_config(&params.config)?;

	let watch = matches!(&params.config.watch, Set(true));

	let txn = begin_write(ctx.conn.as_ref()).await?;

	let id = Uuid::new_v4().to_string();
	let created_config = library_config::ActiveModel {
		library_id: Set(Some(id.clone())),
		..params.config
	}
	.insert(&txn)
	.await?;

	let created_library = library::ActiveModel {
		id: Set(created_config.library_id.clone().ok_or_else(|| {
			CoreError::InternalError("Library config not created correctly".into())
		})?),
		config_id: Set(created_config.id),
		status: Set(FileStatus::Ready),
		name: Set(params.name),
		description: Set(params.description),
		path: Set(params.path),
		emoji: Set(params.emoji),
		..Default::default()
	}
	.insert(&txn)
	.await?;

	if !params.tags.is_empty() {
		let (to_connect, _) = tag_service::sync_tags(&txn, &params.tags, &[]).await?;

		if !to_connect.is_empty() {
			library_tag::Entity::insert_many(
				to_connect
					.into_iter()
					.map(|tag_id| library_tag::ActiveModel {
						library_id: Set(created_library.id.clone()),
						tag_id: Set(tag_id),
						..Default::default()
					})
					.collect::<Vec<library_tag::ActiveModel>>(),
			)
			.on_conflict_do_nothing()
			.exec(&txn)
			.await?;
		}
	}

	txn.commit().await?;

	if params.scan_after_persist {
		ctx.enqueue(StumpJob::library_scan(
			created_library.id.clone(),
			created_library.path.clone(),
			None,
		))
		.await?;
	}

	if ctx.background_jobs_enabled() && watch {
		ctx.add_watcher(created_library.path.clone().into()).await?;
	}

	ctx.send_core_event(CoreEvent::LibraryCreated(LibraryCreated {
		id: created_library.id.clone(),
		name: created_library.name.clone(),
		path: created_library.path.clone(),
	}));

	Ok(created_library)
}

/// Updates a library (config row when provided, library row, tags) and, on
/// success, wires the follow-up scan, the watcher, and the `LibraryUpdated`
/// core event.
pub async fn update_library(
	ctx: &Ctx,
	user: &AuthUser,
	id: &str,
	params: UpdatedLibrary,
) -> CoreResult<library::Model> {
	let (existing_library, existing_config) = library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(id.to_owned()))
		.find_also_related(library_config::Entity)
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| CoreError::NotFound("Library not found".into()))?;
	let existing_config = existing_config.ok_or_else(|| {
		CoreError::InternalError("Library is missing associated config!".into())
	})?;

	if params.scan_after_persist {
		ctx.require_background_jobs()?;
	}

	enforce_valid_library_path(
		ctx.conn.as_ref(),
		&params.path,
		Some(&existing_library.path),
		&ctx.config.server.library_roots,
	)
	.await?;
	enforce_unique_library_name(
		ctx.conn.as_ref(),
		&params.name,
		Some(existing_library.id.as_str()),
	)
	.await?;
	if let Some(config) = &params.config {
		validate_config(config)?;
	}

	let txn = begin_write(ctx.conn.as_ref()).await?;

	if let Some(config) = params.config {
		library_config::ActiveModel {
			id: Set(existing_config.id),
			library_id: Set(existing_config.library_id.clone()),
			..config
		}
		.update(&txn)
		.await?;
	}

	let updated_library = library::ActiveModel {
		id: Set(existing_library.id.clone()),
		name: Set(params.name),
		description: Set(params.description),
		path: Set(params.path),
		emoji: Set(params.emoji),
		..Default::default()
	}
	.update(&txn)
	.await?;

	if let Some(tags) = &params.tags {
		let existing_tags = linked_tags(&txn, &existing_library.id).await?;
		let (to_connect, to_disconnect) =
			tag_service::sync_tags(&txn, tags, &existing_tags).await?;

		if !to_disconnect.is_empty() {
			library_tag::Entity::delete_many()
				.filter(
					library_tag::Column::TagId.is_in(to_disconnect).and(
						library_tag::Column::LibraryId.eq(updated_library.id.clone()),
					),
				)
				.exec(&txn)
				.await?;
		}

		if !to_connect.is_empty() {
			let library_id = updated_library.id.clone();
			library_tag::Entity::insert_many(
				to_connect
					.into_iter()
					.map(|tag_id| library_tag::ActiveModel {
						library_id: Set(library_id.clone()),
						tag_id: Set(tag_id),
						..Default::default()
					})
					.collect::<Vec<library_tag::ActiveModel>>(),
			)
			.on_conflict_do_nothing()
			.exec(&txn)
			.await?;
		}
	}

	txn.commit().await?;

	if params.scan_after_persist {
		ctx.enqueue(StumpJob::library_scan(
			updated_library.id.clone(),
			updated_library.path.clone(),
			None,
		))
		.await?;
	}

	if ctx.background_jobs_enabled() {
		match params.watch {
			WatchUpdate::Add => {
				ctx.add_watcher(updated_library.path.clone().into()).await?;
			},
			WatchUpdate::Remove => {
				ctx.remove_watcher(existing_library.path.clone().into())
					.await?;
			},
			WatchUpdate::Keep => {},
		}
	}

	ctx.send_core_event(CoreEvent::LibraryUpdated(LibraryUpdated {
		id: updated_library.id.clone(),
		name: updated_library.name.clone(),
		path: updated_library.path.clone(),
	}));

	Ok(updated_library)
}

/// Deletes a library together with its series/media list memberships, removes
/// the thumbnails of its media and series, and emits the per-media/series
/// deletion events plus a final `LibraryDeleted` core event.
pub async fn delete_library(
	ctx: &Ctx,
	user: &AuthUser,
	id: &str,
) -> CoreResult<library::Model> {
	let library = library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(id.to_owned()))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| CoreError::NotFound("Library not found".into()))?;

	let txn = begin_write(ctx.conn.as_ref()).await?;
	let library_series = Query::select()
		.column(series::Column::Id)
		.from(series::Entity)
		.and_where(series::Column::LibraryId.eq(library.id.clone()))
		.to_owned();
	let media_rows = media::Entity::find()
		.filter(media::Column::SeriesId.in_subquery(library_series))
		.all(&txn)
		.await?;
	let media_ids = media_rows
		.iter()
		.map(|media| media.id.clone())
		.collect::<Vec<_>>();
	let series_ids: Vec<String> = series::Entity::find()
		.select_only()
		.column(series::Column::Id)
		.filter(series::Column::LibraryId.eq(library.id.clone()))
		.into_tuple()
		.all(&txn)
		.await?;

	lists::remove_memberships_for_media(&txn, &media_ids).await?;
	lists::remove_memberships_for_series(&txn, &series_ids).await?;
	library.clone().delete(&txn).await?;
	txn.commit().await?;

	if !media_ids.is_empty() {
		if let Err(error) =
			remove_thumbnails(&media_ids, &ctx.config.get_thumbnails_dir()).await
		{
			tracing::error!(
				?error,
				"Failed to remove thumbnails for deleted library media"
			);
		}
	}
	if !series_ids.is_empty() {
		if let Err(error) =
			remove_thumbnails(&series_ids, &ctx.config.get_thumbnails_dir()).await
		{
			tracing::error!(
				?error,
				"Failed to remove thumbnails for deleted library series"
			);
		}
	}
	for media in media_rows {
		if let Some(series_id) = media.series_id {
			ctx.send_core_event(CoreEvent::MediaDeleted(crate::event::MediaDeleted {
				id: media.id,
				series_id,
				library_id: library.id.clone(),
			}));
		}
	}
	for series_id in &series_ids {
		ctx.send_core_event(CoreEvent::SeriesDeleted(crate::event::SeriesDeleted {
			id: series_id.clone(),
			library_id: library.id.clone(),
		}));
	}

	ctx.send_core_event(CoreEvent::LibraryDeleted(LibraryDeleted {
		id: library.id.clone(),
		name: library.name.clone(),
		path: library.path.clone(),
	}));

	Ok(library)
}

/// Creates, updates, or removes the scheduled `LibraryScan` job row owning a
/// library's periodic scan.
///
/// `cron: None` (Komga's `DISABLED`) removes the generated row; otherwise the
/// single-library schedule is upserted. Only rows whose config is exactly
/// `{"libraryIds": [<id>]}` are managed, so manually created schedules that
/// scan multiple libraries (or all libraries) are never touched.
///
/// The scheduler is refreshed so the change takes effect without a restart.
pub async fn sync_library_scan_schedule(
	ctx: &Ctx,
	library_id: &str,
	library_name: &str,
	cron: Option<&str>,
) -> CoreResult<()> {
	let rows = scheduled_job::Entity::find()
		.filter(scheduled_job::Column::Kind.eq(ScheduledJobKind::LibraryScan))
		.all(ctx.conn.as_ref())
		.await?;

	let owning_row = rows.into_iter().find(|row| {
		row.library_scan_config()
			.map(|config| {
				config.library_ids.len() == 1 && config.library_ids[0] == library_id
			})
			.unwrap_or(false)
	});

	match (cron, owning_row) {
		(None, Some(row)) => {
			row.delete(ctx.conn.as_ref()).await?;
		},
		(None, None) => {},
		(Some(schedule), Some(row)) => {
			let mut active = row.into_active_model();
			active.schedule = Set(schedule.to_owned());
			active.name = Set(library_scan_job_name(library_name));
			active.enabled = Set(true);
			active.update(ctx.conn.as_ref()).await?;
		},
		(Some(schedule), None) => {
			scheduled_job::ActiveModel {
				name: Set(library_scan_job_name(library_name)),
				kind: Set(ScheduledJobKind::LibraryScan),
				schedule: Set(schedule.to_owned()),
				config: Set(Some(serde_json::json!({ "libraryIds": [library_id] }))),
				enabled: Set(true),
				..Default::default()
			}
			.insert(ctx.conn.as_ref())
			.await?;
		},
	}

	if ctx.background_jobs_enabled() {
		ctx.start_scheduler().await?;
	}

	Ok(())
}

/// The persisted name for a library scan schedule generated by the library
/// service (as opposed to manually created schedules).
pub fn library_scan_job_name(library_name: &str) -> String {
	format!("Scan library: {library_name}")
}

async fn linked_tags(
	txn: &DatabaseTransaction,
	library_id: &str,
) -> CoreResult<Vec<tag::Model>> {
	Ok(tag::Entity::find()
		.filter(
			tag::Column::Id.in_subquery(
				Query::select()
					.column(library_tag::Column::TagId)
					.from(library_tag::Entity)
					.and_where(library_tag::Column::LibraryId.eq(library_id.to_owned()))
					.to_owned(),
			),
		)
		.all(txn)
		.await?)
}

fn validate_config(config: &library_config::ActiveModel) -> CoreResult<()> {
	if let Set(Some(options)) = &config.thumbnail_config {
		options
			.validate()
			.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	}
	Ok(())
}

/// A helper function to enforce that a library path is valid and does not conflict with
/// other libraries.
pub(crate) async fn enforce_valid_library_path(
	conn: &DatabaseConnection,
	path: &str,
	existing_path: Option<&str>,
	library_roots: &[String],
) -> CoreResult<()> {
	if !path_within_roots(path, library_roots) {
		return Err(CoreError::BadRequest(
			"Path is outside of the configured library roots".to_owned(),
		));
	}

	match tokio::fs::metadata(path).await {
		Ok(metadata) => {
			if !metadata.is_dir() {
				return Err(CoreError::BadRequest("Path is not a directory".into()));
			}
		},
		Err(error) => return Err(CoreError::BadRequest(error.to_string())),
	}

	if let Some(existing_path) = existing_path {
		if existing_path == path {
			return Ok(());
		}
	}

	// example: new_path = "/books", existing_library = "/books/fiction"
	// check if any libraries start with "/books/" (can't use "/books" else it flags e.g. "/books2")
	let mut child_query = library::Entity::find().filter(
		library::Column::Path.starts_with(add_trailing_slash(normalize_path(path))),
	);

	if let Some(existing_path) = existing_path {
		child_query =
			child_query.filter(library::Column::Path.ne(normalize_path(existing_path)));
	}

	let child_libraries_count = child_query.count(conn).await?;

	if child_libraries_count > 0 {
		return Err(CoreError::BadRequest(
			"Path is a parent of another library on the filesystem".into(),
		));
	}

	// example: new_path = "/data/books/fiction", existing_library = "/data/books"
	// check if new_path matches the pattern "/data/books/_%".
	let (parent_sql, parent_values): (String, Vec<sea_orm::Value>) = if let Some(ep) =
		existing_path
	{
		(
			r#"SELECT COUNT(*) AS count FROM libraries WHERE $1 LIKE "path" || '/_%' AND "path" != $2"#.to_string(),
			vec![path.into(), ep.into()],
		)
	} else {
		(
			r#"SELECT COUNT(*) AS count FROM libraries WHERE $1 LIKE "path" || '/_%'"#
				.to_string(),
			vec![path.into()],
		)
	};

	let parent_libraries_count: i64 = conn
		.query_one(db_statement(conn, parent_sql, parent_values))
		.await?
		.ok_or_else(|| {
			CoreError::InternalError("Failed to count parent libraries".into())
		})?
		.try_get("", "count")?;

	if parent_libraries_count > 0 {
		return Err(CoreError::BadRequest(
			"Path is a child of another library on the filesystem".into(),
		));
	}

	Ok(())
}

/// Rejects a library whose name is already taken by another library, mirroring
/// Komga's `DuplicateNameException` handling (SQLite enforces the same
/// constraint, but the explicit check yields an actionable error).
async fn enforce_unique_library_name(
	conn: &DatabaseConnection,
	name: &str,
	exclude_id: Option<&str>,
) -> CoreResult<()> {
	let mut query = library::Entity::find().filter(library::Column::Name.eq(name));
	if let Some(exclude_id) = exclude_id {
		query = query.filter(library::Column::Id.ne(exclude_id.to_owned()));
	}
	if query.count(conn).await? > 0 {
		return Err(CoreError::BadRequest("Library name already exists".into()));
	}
	Ok(())
}

/// Build a raw SQL [`Statement`] using the actual database backend of `conn`.
/// Write SQL with `$1`, `$2`, … placeholders — they work for both PostgreSQL
/// and SQLite (SQLite treats them as named parameters).
fn db_statement(
	conn: &DatabaseConnection,
	sql: impl Into<String>,
	values: impl IntoIterator<Item = Value>,
) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use migrations::{Migrator, MigratorTrait};
	use sea_orm::{ConnectOptions, Database, TransactionTrait};

	use super::*;
	use crate::context::Ctx;

	/// A file-backed database with room for two live connections. `sqlite::memory:`
	/// cannot be used here: every pooled connection would get its own private
	/// database, and sea-orm caps an unconfigured SQLite pool at one connection
	/// (`sea-orm-1.1.16/src/driver/sqlx_sqlite.rs:85-87`), so nothing could
	/// contend for the write lock.
	async fn contended_database(dir: &std::path::Path) -> DatabaseConnection {
		let url = format!("sqlite://{}/stump.db?mode=rwc", dir.display());

		// sea-orm-migration does not wrap SQLite migrations in a transaction
		// (`sea-orm-migration-1.1.16/src/migrator.rs:268-271`): every DDL
		// statement runs on whichever connection the pool hands out, so the
		// drop-and-rename in `m20260909_000000_add_ingest_media_targets` can
		// straddle two connections and hit a stale schema cache. Migrate on the
		// capped-at-one pool `Database::connect(&str)` gives us, then open the
		// pool this test contends on.
		let migrator = Database::connect(&url).await.expect("connect migrator");
		Migrator::up(&migrator, None).await.expect("migrate");
		migrator.close().await.expect("close migrator");

		let mut options = ConnectOptions::new(url);
		options.max_connections(4).sqlx_logging(false);
		let conn = Database::connect(options).await.expect("connect");
		conn.execute_unprepared("PRAGMA journal_mode=WAL")
			.await
			.expect("wal");
		conn
	}

	/// `DELETE /api/v1/libraries/{id}` right after a scan used to answer
	/// `500 database is locked`: [`delete_library`] reads the library's media and
	/// series inside its transaction before it writes, and SQLite will not run
	/// the busy handler when a deferred transaction has to promote a read
	/// snapshot to the write lock. It must now wait for the writer instead.
	#[tokio::test]
	async fn delete_library_waits_out_a_concurrent_writer() {
		let dir = tempfile::tempdir().expect("tempdir");
		let conn = contended_database(dir.path()).await;

		let owner = ::tests::fake_data::User::new("owner").insert(&conn).await;
		let library = ::tests::fake_data::Library::default().insert(&conn).await;
		let series = ::tests::fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&conn)
		.await;
		::tests::fake_data::Media {
			series_id: series.id.clone(),
			..Default::default()
		}
		.insert(&conn)
		.await;

		let ctx = Ctx::for_testing(conn);
		let user = AuthUser {
			id: owner.id,
			..Default::default()
		};

		// Stand in for a running scan job: hold the write lock on another
		// pooled connection with a write to an unrelated table, so only the
		// lock itself is contended.
		let holder = ctx.conn.begin().await.expect("begin holder");
		tag::ActiveModel {
			name: Set("scan-in-flight".to_owned()),
			..Default::default()
		}
		.insert(&holder)
		.await
		.expect("holder write takes the lock");

		let delete = {
			let ctx = ctx.clone();
			let id = library.id.clone();
			tokio::spawn(async move { delete_library(&ctx, &user, &id).await })
		};

		tokio::time::sleep(Duration::from_millis(250)).await;
		assert!(
			!delete.is_finished(),
			"the delete must wait for the write lock, not fail"
		);

		holder.commit().await.expect("release the write lock");
		let deleted = delete
			.await
			.expect("join")
			.expect("delete_library under contention");

		assert_eq!(deleted.id, library.id);
		assert!(
			library::Entity::find_by_id(library.id)
				.one(ctx.conn.as_ref())
				.await
				.expect("lookup")
				.is_none(),
			"the library row is gone"
		);
	}
}
