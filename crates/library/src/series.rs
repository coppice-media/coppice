//! Series reshape: move books between series, merge two series into one, and
//! split books out into a series of their own.
//!
//! The scanner recognises the books it already knows by `series_id` **and**
//! `path` ([`crate::filesystem::scanner`]'s store lists the existing media of a
//! series with `WHERE series_id = ?`), so a database-only regrouping does not
//! survive the next scan: the file still sitting in the old directory is
//! re-created as a second book row, and the row that changed series is reported
//! missing because its file is not under the new series' directory. Every
//! reshape here therefore moves the file into the target series' directory and
//! writes the new path together with the new `series_id`, which leaves the next
//! scan with nothing to do.
//!
//! The renames run while the write transaction is open — the same window
//! [`crate::ingest`] uses when it commits a staged file into a library — so a
//! failure rolls the rows back *and* renames back whatever already moved. The
//! database and the filesystem cannot end up disagreeing.
use std::{
	collections::HashSet,
	path::{Path, PathBuf},
};

use models::txn::begin_write;
use models::{
	entity::{library, media, series, user::AuthUser},
	services::lists,
};
use sea_orm::{
	prelude::*, ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait,
	IntoActiveModel, PaginatorTrait, QueryFilter, Set,
};
use stump_media::{image::remove_thumbnails, move_file};
use tokio::fs;
use uuid::Uuid;

use stump_core::{
	error::{CoreError, CoreResult},
	event::{CoreEvent, CreatedManySeries, CreatedOrUpdatedManyMedia, SeriesDeleted},
	Ctx,
};

/// One book that changed series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedMedia {
	pub id: String,
	/// Where the file was before the reshape.
	pub from: String,
	/// Where the file is now, and what the book's `path` column holds.
	pub to: String,
	/// The file was not on disk, so only the grouping changed. The book keeps
	/// whatever status the scanner gave it; its path now points into the
	/// target series' directory, where a replacement file would be picked up.
	pub file_missing: bool,
}

/// What [`move_media_to_series`] or [`split_series`] did.
#[derive(Debug)]
pub struct Reshaped {
	/// The series the books belong to now.
	pub series: series::Model,
	pub moved: Vec<MovedMedia>,
}

/// What [`merge_series`] did.
#[derive(Debug)]
pub struct Merged {
	pub kept: series::Model,
	pub dropped_series_id: String,
	pub moved: Vec<MovedMedia>,
	/// The emptied series directory was removed from disk. It is left in place
	/// when it still holds files the scanner ignored.
	pub dropped_directory: bool,
}

/// Moves `media_ids` into an existing series, on disk and in the database.
///
/// Every book must be visible to `user` and live in the same library as the
/// target series; books whose file is a provider stream are rejected, since
/// nothing can be moved for them (`mergeProviderSeries` is their reshape).
pub async fn move_media_to_series(
	ctx: &Ctx,
	user: &AuthUser,
	media_ids: &[String],
	series_id: &str,
) -> CoreResult<Reshaped> {
	let conn = ctx.conn.as_ref();
	let target = local_series(conn, user, series_id).await?;
	let library_id = library_of(&target)?;

	let books = selected_books(conn, user, media_ids).await?;
	let source_library = source_library(conn, &books).await?;
	if source_library != library_id {
		return Err(CoreError::BadRequest(
			"Books can only be moved between series of the same library".into(),
		));
	}

	let moves = plan_moves(books, Path::new(&target.path)).await?;
	let committed = Plan {
		series_id: target.id.clone(),
		create: None,
		drop: None,
		moves,
	}
	.commit(conn)
	.await?;

	ctx.send_core_event(CoreEvent::CreatedOrUpdatedManyMedia(
		CreatedOrUpdatedManyMedia {
			count: committed.moves.len() as u64,
			series_id: target.id.clone(),
			library_id,
		},
	));

	Ok(Reshaped {
		series: target,
		moved: committed.moved(),
	})
}

/// Merges `drop` into `keep`: every book of `drop` moves into the kept series'
/// directory, then the emptied series row (and its now empty directory) is
/// removed.
///
/// Reading progress needs no repair — the book rows survive the merge, only
/// their series does not.
pub async fn merge_series(
	ctx: &Ctx,
	user: &AuthUser,
	keep: &str,
	drop: &str,
) -> CoreResult<Merged> {
	if keep == drop {
		return Err(CoreError::BadRequest(
			"A series cannot be merged into itself".into(),
		));
	}

	let conn = ctx.conn.as_ref();
	let kept = local_series(conn, user, keep).await?;
	let dropped = local_series(conn, user, drop).await?;
	let library_id = library_of(&kept)?;
	if library_of(&dropped)? != library_id {
		return Err(CoreError::BadRequest(
			"Series can only be merged within the same library".into(),
		));
	}

	// Every row has to move, including the ones the scanner marked missing:
	// `media.series_id` is `ON DELETE CASCADE`, so a book left behind would be
	// deleted with the series instead of merged into the kept one.
	let books = media::Entity::find()
		.filter(media::Column::SeriesId.eq(dropped.id.clone()))
		.all(conn)
		.await?;

	let moves = plan_moves(books, Path::new(&kept.path)).await?;
	let committed = Plan {
		series_id: kept.id.clone(),
		create: None,
		drop: Some(dropped.id.clone()),
		moves,
	}
	.commit(conn)
	.await?;

	if let Err(error) =
		remove_thumbnails(&[dropped.id.clone()], &ctx.config.get_thumbnails_dir()).await
	{
		tracing::error!(?error, "Failed to remove the merged series' thumbnail");
	}

	// An empty directory left behind would be scanned back into a series row.
	let dropped_directory = match fs::remove_dir(&dropped.path).await {
		Ok(()) => true,
		Err(error) => {
			tracing::debug!(
				path = %dropped.path,
				?error,
				"The merged series' directory was left in place"
			);
			false
		},
	};

	ctx.send_core_event(CoreEvent::SeriesDeleted(SeriesDeleted {
		id: dropped.id.clone(),
		library_id: library_id.clone(),
	}));
	ctx.send_core_event(CoreEvent::CreatedOrUpdatedManyMedia(
		CreatedOrUpdatedManyMedia {
			count: committed.moves.len() as u64,
			series_id: kept.id.clone(),
			library_id,
		},
	));

	Ok(Merged {
		kept,
		dropped_series_id: dropped.id,
		moved: committed.moved(),
		dropped_directory,
	})
}

/// Splits `media_ids` out into a new series named `name`, created as a
/// directory next to the other series of their library.
pub async fn split_series(
	ctx: &Ctx,
	user: &AuthUser,
	media_ids: &[String],
	name: &str,
) -> CoreResult<Reshaped> {
	let conn = ctx.conn.as_ref();
	let name = directory_name(name)?;

	let books = selected_books(conn, user, media_ids).await?;
	let library_id = source_library(conn, &books).await?;
	let library = library::Entity::find_by_id(library_id.clone())
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("Library not found".into()))?;

	let path = Path::new(&library.path).join(&name);
	let path_string = path.to_string_lossy().into_owned();
	if fs::metadata(&path).await.is_ok() {
		return Err(CoreError::BadRequest(format!(
			"{name} already exists in this library; pick another name"
		)));
	}
	if series::Entity::find()
		.filter(series::Column::Path.eq(path_string.clone()))
		.count(conn)
		.await?
		> 0
	{
		return Err(CoreError::BadRequest(format!(
			"A series is already registered at {path_string}"
		)));
	}

	let moves = plan_moves(books, &path).await?;
	let id = Uuid::new_v4().to_string();
	fs::create_dir(&path).await?;

	let plan = Plan {
		series_id: id.clone(),
		create: Some(series::ActiveModel {
			id: Set(id),
			name: Set(name),
			path: Set(path_string),
			library_id: Set(Some(library_id.clone())),
			..Default::default()
		}),
		drop: None,
		moves,
	};
	let committed = match plan.commit(conn).await {
		Ok(committed) => committed,
		Err(error) => {
			// Nothing was written and no file moved, so the directory this call
			// created is the only trace left.
			if let Err(cleanup) = fs::remove_dir(&path).await {
				tracing::error!(
					path = %path.display(),
					?cleanup,
					"Failed to remove the directory of a series that was not created"
				);
			}
			return Err(error);
		},
	};

	let created = committed.created.clone().ok_or_else(|| {
		CoreError::InternalError("The new series was not created".into())
	})?;

	ctx.send_core_event(CoreEvent::CreatedManySeries(CreatedManySeries {
		count: 1,
		library_id: library_id.clone(),
	}));
	ctx.send_core_event(CoreEvent::CreatedOrUpdatedManyMedia(
		CreatedOrUpdatedManyMedia {
			count: committed.moves.len() as u64,
			series_id: created.id.clone(),
			library_id,
		},
	));

	Ok(Reshaped {
		series: created,
		moved: committed.moved(),
	})
}

/// One book's file move. `from == to` when the file already sits in the target
/// directory and only the grouping changes.
struct PlannedMove {
	media: media::Model,
	from: PathBuf,
	to: PathBuf,
	file_missing: bool,
}

impl PlannedMove {
	fn renames(&self) -> bool {
		!self.file_missing && self.from != self.to
	}
}

/// A reshape that has been validated and can be applied in one transaction.
struct Plan {
	/// The series every moved book belongs to afterwards.
	series_id: String,
	/// The series row to insert first, for a split.
	create: Option<series::ActiveModel>,
	/// The emptied series row to delete afterwards, for a merge.
	drop: Option<String>,
	moves: Vec<PlannedMove>,
}

/// What a [`Plan`] wrote.
struct Committed {
	created: Option<series::Model>,
	moves: Vec<PlannedMove>,
}

impl Committed {
	fn moved(self) -> Vec<MovedMedia> {
		self.moves
			.into_iter()
			.map(|plan| MovedMedia {
				id: plan.media.id,
				from: plan.from.to_string_lossy().into_owned(),
				to: plan.to.to_string_lossy().into_owned(),
				file_missing: plan.file_missing,
			})
			.collect()
	}
}

impl Plan {
	/// Writes the rows and renames the files as one step: the transaction is
	/// committed only once every rename landed, and a failure on either side
	/// leaves the library exactly as it was.
	async fn commit(self, conn: &DatabaseConnection) -> CoreResult<Committed> {
		let txn = begin_write(conn).await?;

		let created = match self.write(&txn).await {
			Ok(created) => created,
			Err(error) => return Err(rollback(txn, error).await),
		};
		if let Err(error) = apply_moves(&self.moves).await {
			return Err(rollback(txn, error).await);
		}
		if let Err(error) = txn.commit().await {
			undo_moves(&self.moves).await;
			return Err(error.into());
		}

		Ok(Committed {
			created,
			moves: self.moves,
		})
	}

	async fn write(
		&self,
		txn: &DatabaseTransaction,
	) -> CoreResult<Option<series::Model>> {
		let created = match self.create.clone() {
			Some(series) => Some(series.insert(txn).await?),
			None => None,
		};

		for plan in &self.moves {
			let mut book = plan.media.clone().into_active_model();
			book.series_id = Set(Some(self.series_id.clone()));
			book.path = Set(plan.to.to_string_lossy().into_owned());
			book.update(txn).await?;
		}

		if let Some(drop) = &self.drop {
			let remaining = media::Entity::find()
				.filter(media::Column::SeriesId.eq(drop.clone()))
				.count(txn)
				.await?;
			if remaining > 0 {
				return Err(CoreError::InternalError(format!(
					"{remaining} books are still attached to the series being merged away"
				)));
			}
			lists::remove_memberships_for_series(txn, std::slice::from_ref(drop)).await?;
			series::Entity::delete_by_id(drop.clone()).exec(txn).await?;
		}

		Ok(created)
	}
}

/// Renames every planned file, renaming back whatever already landed when one
/// fails, so the filesystem matches the transaction that is about to roll back.
async fn apply_moves(moves: &[PlannedMove]) -> CoreResult<()> {
	for (index, plan) in moves.iter().enumerate() {
		if !plan.renames() {
			continue;
		}
		if let Err(error) = move_file(&plan.from, &plan.to).await {
			undo_moves(&moves[..index]).await;
			return Err(error.into());
		}
	}
	Ok(())
}

async fn undo_moves(moves: &[PlannedMove]) {
	for plan in moves.iter().rev().filter(|plan| plan.renames()) {
		if let Err(error) = move_file(&plan.to, &plan.from).await {
			tracing::error!(
				from = %plan.to.display(),
				to = %plan.from.display(),
				?error,
				"Failed to move a book back after a reshape was rolled back"
			);
		}
	}
}

async fn rollback(txn: DatabaseTransaction, error: CoreError) -> CoreError {
	if let Err(rollback) = txn.rollback().await {
		tracing::error!(?rollback, "Failed to roll a reshape back");
	}
	error
}

/// Resolves where each book's file lands in `target`, rejecting a selection
/// that would overwrite a file or collapse two books onto one name.
async fn plan_moves(
	books: Vec<media::Model>,
	target: &Path,
) -> CoreResult<Vec<PlannedMove>> {
	let mut moves = Vec::with_capacity(books.len());
	let mut names = HashSet::with_capacity(books.len());

	for media in books {
		let from = PathBuf::from(&media.path);
		let file_name = from
			.file_name()
			.ok_or_else(|| {
				CoreError::BadRequest(format!(
					"{} has no file name in its path ({})",
					media.name, media.path
				))
			})?
			.to_owned();
		let to = target.join(&file_name);

		if !names.insert(file_name.clone()) {
			return Err(CoreError::BadRequest(format!(
				"Two of the selected books are both named {}; rename one first",
				file_name.to_string_lossy()
			)));
		}

		let mut plan = PlannedMove {
			media,
			from,
			to,
			file_missing: false,
		};
		if plan.from != plan.to {
			if fs::metadata(&plan.from).await.is_err() {
				plan.file_missing = true;
			} else if fs::metadata(&plan.to).await.is_ok() {
				return Err(CoreError::BadRequest(format!(
					"{} already exists; rename one of the two books first",
					plan.to.display()
				)));
			}
		}
		moves.push(plan);
	}

	Ok(moves)
}

/// A series `user` may reshape: visible to them, not soft deleted, and backed
/// by files rather than a provider stream.
async fn local_series(
	conn: &DatabaseConnection,
	user: &AuthUser,
	id: &str,
) -> CoreResult<series::Model> {
	let series = series::Entity::find_for_user(user)
		.filter(
			series::Column::Id
				.eq(id.to_owned())
				.and(series::Column::DeletedAt.is_null()),
		)
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("Series not found".into()))?;

	if series.source_provider.is_some() {
		return Err(CoreError::BadRequest(format!(
			"{} is a provider series and has no files to move",
			series.name
		)));
	}

	Ok(series)
}

fn library_of(series: &series::Model) -> CoreResult<String> {
	series.library_id.clone().ok_or_else(|| {
		CoreError::BadRequest(format!("{} does not belong to a library", series.name))
	})
}

/// The books `media_ids` names, in the order they were given, rejecting ids
/// `user` cannot see and provider-backed books.
async fn selected_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_ids: &[String],
) -> CoreResult<Vec<media::Model>> {
	let mut wanted = Vec::with_capacity(media_ids.len());
	for id in media_ids {
		if !wanted.contains(id) {
			wanted.push(id.clone());
		}
	}
	if wanted.is_empty() {
		return Err(CoreError::BadRequest("No books were selected".into()));
	}

	let found = media::Entity::find_for_user(user)
		.filter(media::Column::Id.is_in(wanted.clone()))
		.all(conn)
		.await?;
	if found.len() != wanted.len() {
		return Err(CoreError::NotFound(format!(
			"{} of the {} selected books could not be found",
			wanted.len() - found.len(),
			wanted.len()
		)));
	}
	if let Some(remote) = found.iter().find(|media| media.source_provider.is_some()) {
		return Err(CoreError::BadRequest(format!(
			"{} is a provider stream and has no file to move",
			remote.name
		)));
	}

	// The caller's order is the one the UI showed.
	let mut books = Vec::with_capacity(found.len());
	for id in &wanted {
		if let Some(index) = found.iter().position(|media| &media.id == id) {
			books.push(found[index].clone());
		}
	}

	Ok(books)
}

/// The library every selected book lives in, rejecting a selection spread over
/// several libraries (their files sit under different roots).
async fn source_library(
	conn: &DatabaseConnection,
	books: &[media::Model],
) -> CoreResult<String> {
	let mut series_ids = Vec::with_capacity(books.len());
	for media in books {
		let series_id = media.series_id.clone().ok_or_else(|| {
			CoreError::BadRequest(format!("{} does not belong to a series", media.name))
		})?;
		if !series_ids.contains(&series_id) {
			series_ids.push(series_id);
		}
	}

	let sources = series::Entity::find()
		.filter(series::Column::Id.is_in(series_ids))
		.all(conn)
		.await?;

	let mut library_id: Option<String> = None;
	for series in &sources {
		if series.source_provider.is_some() {
			return Err(CoreError::BadRequest(format!(
				"{} is a provider series and has no files to move",
				series.name
			)));
		}
		let series_library = library_of(series)?;
		match &library_id {
			Some(known) if known != &series_library => {
				return Err(CoreError::BadRequest(
					"The selected books belong to more than one library".into(),
				))
			},
			Some(_) => {},
			None => library_id = Some(series_library),
		}
	}

	library_id
		.ok_or_else(|| CoreError::BadRequest("The selected books have no library".into()))
}

/// Validates a name that becomes a directory of the library root.
fn directory_name(name: &str) -> CoreResult<String> {
	let name = name.trim();
	if name.is_empty() {
		return Err(CoreError::BadRequest("The series needs a name".into()));
	}
	if name == "." || name == ".." || name.contains(['/', '\\']) {
		return Err(CoreError::BadRequest(
			"A series name cannot contain a path separator".into(),
		));
	}
	Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
	use migrations::{Migrator, MigratorTrait};
	use models::shared::enums::FileStatus;
	use sea_orm::{Database, QueryOrder};

	use super::*;
	use stump_core::Ctx;

	struct Fixture {
		ctx: Ctx,
		user: AuthUser,
		library: library::Model,
		root: tempfile::TempDir,
	}

	impl Fixture {
		async fn new() -> Self {
			let root = tempfile::tempdir().expect("tempdir");
			let conn = Database::connect("sqlite::memory:").await.expect("connect");
			Migrator::up(&conn, None).await.expect("migrate");

			let owner = ::tests::fake_data::User::new("owner").insert(&conn).await;
			let library = ::tests::fake_data::Library {
				path: Some(root.path().to_string_lossy().into_owned()),
				..Default::default()
			}
			.insert(&conn)
			.await;

			Self {
				ctx: Ctx::for_testing(conn),
				user: AuthUser {
					id: owner.id,
					..Default::default()
				},
				library,
				root,
			}
		}

		/// A series directory with `books` in it, registered like a scan would.
		async fn series(&self, name: &str, books: &[&str]) -> series::Model {
			let path = self.root.path().join(name);
			std::fs::create_dir_all(&path).expect("series directory");

			let series = series::ActiveModel {
				id: Set(Uuid::new_v4().to_string()),
				name: Set(name.to_owned()),
				path: Set(path.to_string_lossy().into_owned()),
				library_id: Set(Some(self.library.id.clone())),
				..Default::default()
			}
			.insert(self.ctx.conn.as_ref())
			.await
			.expect("series row");

			for book in books {
				let file = path.join(book);
				std::fs::write(&file, b"book").expect("book file");
				media::ActiveModel {
					id: Set(Uuid::new_v4().to_string()),
					name: Set((*book).to_owned()),
					size: Set(4),
					extension: Set("cbz".to_owned()),
					pages: Set(1),
					path: Set(file.to_string_lossy().into_owned()),
					status: Set(FileStatus::Ready),
					series_id: Set(Some(series.id.clone())),
					..Default::default()
				}
				.insert(self.ctx.conn.as_ref())
				.await
				.expect("media row");
			}

			series
		}

		async fn books_of(&self, series_id: &str) -> Vec<media::Model> {
			media::Entity::find()
				.filter(media::Column::SeriesId.eq(series_id.to_owned()))
				.order_by_asc(media::Column::Name)
				.all(self.ctx.conn.as_ref())
				.await
				.expect("books")
		}
	}

	/// A reshape that only rewrote `series_id` would be undone by the next
	/// scan, so the file has to follow the book: it lands in the target
	/// directory and the row's path says so.
	#[tokio::test]
	async fn moving_a_book_moves_its_file_into_the_target_directory() {
		let fixture = Fixture::new().await;
		let source = fixture
			.series("Saga", &["Saga 001.cbz", "Saga 002.cbz"])
			.await;
		let target = fixture.series("Saga Specials", &[]).await;
		let book = fixture.books_of(&source.id).await.remove(0);

		let reshaped = move_media_to_series(
			&fixture.ctx,
			&fixture.user,
			&[book.id.clone()],
			&target.id,
		)
		.await
		.expect("move");

		let expected = Path::new(&target.path).join("Saga 001.cbz");
		assert_eq!(reshaped.moved.len(), 1);
		assert_eq!(reshaped.moved[0].to, expected.to_string_lossy());
		assert!(expected.exists(), "the file moved");
		assert!(
			!Path::new(&book.path).exists(),
			"the file no longer sits in the old series"
		);

		let moved = fixture.books_of(&target.id).await;
		assert_eq!(moved.len(), 1);
		assert_eq!(moved[0].path, expected.to_string_lossy());
		assert_eq!(
			fixture.books_of(&source.id).await.len(),
			1,
			"the rest stays"
		);
	}

	/// Overwriting a book of the target series would destroy a file, so the
	/// whole reshape is refused and nothing moves.
	#[tokio::test]
	async fn a_name_collision_refuses_the_move() {
		let fixture = Fixture::new().await;
		let source = fixture.series("Saga", &["Saga 001.cbz"]).await;
		let target = fixture.series("Saga Reprint", &["Saga 001.cbz"]).await;
		let book = fixture.books_of(&source.id).await.remove(0);

		let error = move_media_to_series(
			&fixture.ctx,
			&fixture.user,
			&[book.id.clone()],
			&target.id,
		)
		.await
		.expect_err("the target already has that file");
		assert!(matches!(error, CoreError::BadRequest(_)), "{error:?}");

		assert!(Path::new(&book.path).exists(), "the file stayed put");
		assert_eq!(fixture.books_of(&source.id).await.len(), 1);
		assert_eq!(fixture.books_of(&target.id).await.len(), 1);
	}

	/// A merge empties one series into another and takes the leftover row and
	/// directory with it, or the next scan would recreate the series.
	#[tokio::test]
	async fn merging_moves_every_book_and_deletes_the_emptied_series() {
		let fixture = Fixture::new().await;
		let keep = fixture.series("Saga", &["Saga 001.cbz"]).await;
		let drop = fixture
			.series("Saga (2012)", &["Saga 002.cbz", "Saga 003.cbz"])
			.await;

		let merged = merge_series(&fixture.ctx, &fixture.user, &keep.id, &drop.id)
			.await
			.expect("merge");

		assert_eq!(merged.moved.len(), 2);
		assert!(merged.dropped_directory, "the emptied directory is gone");
		assert!(!Path::new(&drop.path).exists());
		for book in ["Saga 001.cbz", "Saga 002.cbz", "Saga 003.cbz"] {
			assert!(Path::new(&keep.path).join(book).exists(), "{book} moved");
		}

		assert_eq!(fixture.books_of(&keep.id).await.len(), 3);
		assert!(
			series::Entity::find_by_id(drop.id)
				.one(fixture.ctx.conn.as_ref())
				.await
				.expect("lookup")
				.is_none(),
			"the merged-away series row is gone"
		);
	}

	/// `media.series_id` cascades on delete, so a book the merge could not move
	/// would be deleted with the series. A missing file must therefore still
	/// follow the merge, with its path rewritten into the kept series.
	#[tokio::test]
	async fn a_book_whose_file_is_gone_still_survives_the_merge() {
		let fixture = Fixture::new().await;
		let keep = fixture.series("Saga", &[]).await;
		let drop = fixture.series("Saga (2012)", &["Saga 002.cbz"]).await;
		let book = fixture.books_of(&drop.id).await.remove(0);
		std::fs::remove_file(&book.path).expect("simulate a missing file");

		let merged = merge_series(&fixture.ctx, &fixture.user, &keep.id, &drop.id)
			.await
			.expect("merge");

		assert!(merged.moved[0].file_missing, "reported, not moved");
		let survivors = fixture.books_of(&keep.id).await;
		assert_eq!(survivors.len(), 1, "the row was not cascade-deleted");
		assert_eq!(
			survivors[0].path,
			Path::new(&keep.path).join("Saga 002.cbz").to_string_lossy()
		);
	}

	/// A split creates the directory the new series is scanned from and moves
	/// the selected books into it.
	#[tokio::test]
	async fn splitting_creates_a_directory_and_moves_the_selection() {
		let fixture = Fixture::new().await;
		let source = fixture
			.series("Saga", &["Saga 001.cbz", "Omake 001.cbz"])
			.await;
		let omake = fixture
			.books_of(&source.id)
			.await
			.into_iter()
			.find(|book| book.name.starts_with("Omake"))
			.expect("book");

		let reshaped = split_series(
			&fixture.ctx,
			&fixture.user,
			&[omake.id.clone()],
			"  Saga Omake  ",
		)
		.await
		.expect("split");

		assert_eq!(reshaped.series.name, "Saga Omake", "the name is trimmed");
		let expected = fixture.root.path().join("Saga Omake");
		assert_eq!(reshaped.series.path, expected.to_string_lossy());
		assert!(expected.join("Omake 001.cbz").exists());
		assert_eq!(fixture.books_of(&reshaped.series.id).await.len(), 1);
		assert_eq!(fixture.books_of(&source.id).await.len(), 1);
	}

	/// A name that escapes the library root would write outside the library.
	#[tokio::test]
	async fn a_split_name_cannot_escape_the_library_root() {
		let fixture = Fixture::new().await;
		let source = fixture.series("Saga", &["Saga 001.cbz"]).await;
		let book = fixture.books_of(&source.id).await.remove(0);

		for name in ["../escape", "nested/name", "  "] {
			let error =
				split_series(&fixture.ctx, &fixture.user, &[book.id.clone()], name)
					.await
					.expect_err("rejected");
			assert!(
				matches!(error, CoreError::BadRequest(_)),
				"{name}: {error:?}"
			);
		}
		assert_eq!(fixture.books_of(&source.id).await.len(), 1);
	}

	/// Reshaping across libraries would move a file out of its library root,
	/// where the scanner would report it missing forever.
	#[tokio::test]
	async fn books_cannot_be_moved_into_another_library() {
		let fixture = Fixture::new().await;
		let source = fixture.series("Saga", &["Saga 001.cbz"]).await;
		let book = fixture.books_of(&source.id).await.remove(0);

		let elsewhere = ::tests::fake_data::Library {
			path: Some(
				fixture
					.root
					.path()
					.join("other-root")
					.to_string_lossy()
					.into_owned(),
			),
			..Default::default()
		}
		.insert(fixture.ctx.conn.as_ref())
		.await;
		let foreign = series::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			name: Set("Elsewhere".to_owned()),
			path: Set(fixture
				.root
				.path()
				.join("other-root/Elsewhere")
				.to_string_lossy()
				.into_owned()),
			library_id: Set(Some(elsewhere.id)),
			..Default::default()
		}
		.insert(fixture.ctx.conn.as_ref())
		.await
		.expect("series row");

		let error = move_media_to_series(
			&fixture.ctx,
			&fixture.user,
			&[book.id.clone()],
			&foreign.id,
		)
		.await
		.expect_err("cross-library move");
		assert!(matches!(error, CoreError::BadRequest(_)), "{error:?}");
		assert!(Path::new(&book.path).exists());
	}

	/// Merging a series into itself would delete the row it just filled.
	#[tokio::test]
	async fn a_series_cannot_be_merged_into_itself() {
		let fixture = Fixture::new().await;
		let series = fixture.series("Saga", &["Saga 001.cbz"]).await;

		let error = merge_series(&fixture.ctx, &fixture.user, &series.id, &series.id)
			.await
			.expect_err("self merge");
		assert!(matches!(error, CoreError::BadRequest(_)), "{error:?}");
		assert_eq!(fixture.books_of(&series.id).await.len(), 1);
	}
}
