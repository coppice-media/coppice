use async_graphql::{Context, Object, Result, SimpleObject, ID};
use chrono::Utc;
use models::{
	entity::{favorite_series, library, library_config, media, series},
	shared::enums::UserPermission,
};
use sea_orm::{
	prelude::*,
	sea_query::{OnConflict, Query},
	ActiveValue::Set,
};
use stump_core::filesystem::{
	image::{generate_book_thumbnail, GenerateThumbnailOptions},
	media::analysis::{AnalysisJobConfig, MediaAnalysisJobScope},
};
use stump_core::job::stump_job::StumpJob;

use crate::{
	data::CoreContext, guard::PermissionGuard, input::thumbnail::UpdateThumbnailInput,
	object::series::Series,
};

#[derive(Default)]
pub struct SeriesMutation;

#[Object]
impl SeriesMutation {
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn analyze_series(
		&self,
		ctx: &Context<'_>,
		id: ID,
		#[graphql(default = false)] force_reanalysis: bool,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let model =
			series::Entity::find_series_ident_for_user_and_id(user, id.to_string())
				.into_model::<series::SeriesIdentSelect>()
				.one(conn)
				.await?
				.ok_or("Series not found")?;

		core.enqueue(StumpJob::analyze_media(AnalysisJobConfig {
			force_reanalysis,
			scope: MediaAnalysisJobScope::Series(model.id),
		}))
		.await
		.map_err(crate::error::map_core_error)?;

		Ok(true)
	}

	async fn favorite_series(
		&self,
		ctx: &Context<'_>,
		id: ID,
		is_favorite: bool,
	) -> Result<Series> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let model = series::ModelWithMetadata::find_for_user(user)
			.filter(
				series::Column::Id
					.eq(id.to_string())
					.and(series::Column::DeletedAt.is_null()),
			)
			.into_model::<series::ModelWithMetadata>()
			.one(conn)
			.await?
			.ok_or("Series not found")?;

		if is_favorite {
			let last_insert_id =
				favorite_series::Entity::insert(favorite_series::ActiveModel {
					user_id: Set(user.id.clone()),
					series_id: Set(model.series.id.clone()),
					favorited_at: Set(DateTimeWithTimeZone::from(Utc::now())),
				})
				.on_conflict(OnConflict::new().do_nothing().to_owned())
				.exec(core.conn.as_ref())
				.await?
				.last_insert_id;
			tracing::debug!(?last_insert_id, "Added favorite series");
		} else {
			let affected_rows =
				favorite_series::Entity::delete_many()
					.filter(favorite_series::Column::UserId.eq(user.id.clone()).and(
						favorite_series::Column::SeriesId.eq(model.series.id.clone()),
					))
					.exec(core.conn.as_ref())
					.await?
					.rows_affected;
			tracing::debug!(?affected_rows, "Removed favorite series");
		}

		Ok(model.into())
	}

	/// Update the thumbnail for a series. This will replace the existing thumbnail with the the one
	/// associated with the provided input (book). If the book does not have a thumbnail, one
	/// will be generated based on the library's thumbnail configuration.
	#[graphql(guard = "PermissionGuard::one(UserPermission::EditThumbnails)")]
	async fn update_series_thumbnail(
		&self,
		ctx: &Context<'_>,
		id: ID,
		input: UpdateThumbnailInput,
	) -> Result<Series> {
		let core = ctx.data::<CoreContext>()?;
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;

		let series = series::ModelWithMetadata::find_for_user(user)
			.filter(series::Column::Id.eq(id.to_string()))
			.into_model::<series::ModelWithMetadata>()
			.one(core.conn.as_ref())
			.await?
			.ok_or("Series not found")?;
		let series_id = series.series.id.clone();

		let (_library, config) = library::Entity::find_for_user(user)
			.filter(
				library::Column::Id.in_subquery(
					Query::select()
						.column(series::Column::LibraryId)
						.from(series::Entity)
						.and_where(series::Column::Id.eq(series_id))
						.to_owned(),
				),
			)
			.find_also_related(library_config::Entity)
			.one(core.conn.as_ref())
			.await?
			.ok_or("Associated library for series not found")?;

		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(input.media_id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Media not found")?;

		let page = input.params.page();

		if book.extension == "epub" && page > 1 {
			return Err("Cannot set thumbnail from EPUB chapter".into());
		}

		let image_options = config
			.ok_or("Library config not found")?
			.thumbnail_config
			.unwrap_or_default()
			.with_page(page);

		let (_, path_buf, _) = generate_book_thumbnail(
			&book.clone().into(),
			core.conn.as_ref(),
			GenerateThumbnailOptions {
				image_options,
				core_config: core.config.as_ref().clone(),
				force_regen: true,
				filename: Some(id.to_string()),
			},
		)
		.await?;
		tracing::debug!(path = ?path_buf, "Generated series thumbnail");

		Ok(series.into())
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ScanLibrary)")]
	async fn scan_series(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let model =
			series::Entity::find_series_ident_for_user_and_id(user, id.to_string())
				.into_model::<series::SeriesIdentSelect>()
				.one(conn)
				.await?
				.ok_or("Series not found")?;

		core.enqueue(StumpJob::series_scan(model.id, model.path, None))
			.await
			.map_err(crate::error::map_core_error)?;

		Ok(true)
	}

	/// Move books into another series of the same library.
	///
	/// The files move with them, into the target series' directory, so the next
	/// scan sees the new grouping instead of undoing it. Provider-backed series
	/// have no files and are rejected.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn move_media_to_series(
		&self,
		ctx: &Context<'_>,
		media_ids: Vec<ID>,
		series_id: ID,
	) -> Result<Series> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let reshaped = stump_core::series::move_media_to_series(
			core,
			user,
			&id_strings(&media_ids),
			&series_id.to_string(),
		)
		.await
		.map_err(crate::error::map_core_error)?;

		reshaped_series(ctx, &reshaped.series.id).await
	}

	/// Merge `drop` into `keep`: every book of `drop` moves into the kept
	/// series' directory, then the emptied series (and its directory) is
	/// removed. Reading progress follows the books.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn merge_series(
		&self,
		ctx: &Context<'_>,
		keep: ID,
		drop: ID,
	) -> Result<SeriesMergeResult> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let merged = stump_core::series::merge_series(
			core,
			user,
			&keep.to_string(),
			&drop.to_string(),
		)
		.await
		.map_err(crate::error::map_core_error)?;

		let missing_files = merged
			.moved
			.iter()
			.filter(|media| media.file_missing)
			.count();
		Ok(SeriesMergeResult {
			kept: reshaped_series(ctx, &merged.kept.id).await?,
			dropped_series_id: merged.dropped_series_id,
			moved: (merged.moved.len() - missing_files) as i32,
			missing_files: missing_files as i32,
			dropped_directory: merged.dropped_directory,
		})
	}

	/// Split books out into a new series, created as a directory of their
	/// library so the next scan finds it where it is.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn split_series(
		&self,
		ctx: &Context<'_>,
		media_ids: Vec<ID>,
		name: String,
	) -> Result<Series> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let reshaped =
			stump_core::series::split_series(core, user, &id_strings(&media_ids), &name)
				.await
				.map_err(crate::error::map_core_error)?;

		reshaped_series(ctx, &reshaped.series.id).await
	}
}

/// What one `mergeSeries` call did.
#[derive(SimpleObject)]
pub struct SeriesMergeResult {
	/// The series the books belong to now.
	pub kept: Series,
	pub dropped_series_id: String,
	/// Books whose file moved into the kept series' directory.
	pub moved: i32,
	/// Books that were regrouped without moving a file, because the file is
	/// not on disk. They keep the status the scanner gave them.
	pub missing_files: i32,
	/// The emptied series directory was removed from disk. It is left in place
	/// when it still holds files the scanner ignored.
	pub dropped_directory: bool,
}

fn id_strings(ids: &[ID]) -> Vec<String> {
	ids.iter().map(|id| id.to_string()).collect()
}

/// Re-reads a reshaped series with its metadata, so the client sees the same
/// shape it gets from a query.
async fn reshaped_series(ctx: &Context<'_>, id: &str) -> Result<Series> {
	let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
	let conn = ctx.data::<CoreContext>()?.conn.as_ref();

	let model = series::ModelWithMetadata::find_for_user(user)
		.filter(series::Column::Id.eq(id.to_owned()))
		.into_model::<series::ModelWithMetadata>()
		.one(conn)
		.await?
		.ok_or("Series not found")?;

	Ok(model.into())
}
