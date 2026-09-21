use async_graphql::{Context, Json, MaybeUndefined, Object, Result, SimpleObject, ID};
use chrono::Utc;
use itertools::chain;
use metadata_integrations::MetadataField;
use models::txn::begin_write;
use models::{
	entity::{
		last_library_visit,
		library::{self, LibraryIdentSelect},
		library_config, library_exclusion, library_scan_record, media, media_metadata,
		metadata_provider_config, series, series_metadata, user,
	},
	services::lists,
	shared::enums::{FileStatus, MetadataResetImpact, UserPermission},
};
use sea_orm::{
	prelude::*,
	sea_query::{Expr, OnConflict, Query},
	Condition, IntoActiveModel, QuerySelect, Set, TransactionTrait,
};
use stump_core::{
	filesystem::{
		image::{
			generate_book_thumbnail, GenerateThumbnailOptions,
			PlaceholderGenerationJobConfig, PlaceholderGenerationJobScope,
			ThumbnailGenerationJobParams,
		},
		media::analysis::{AnalysisJobConfig, MediaAnalysisJobScope},
		metadata::{MetadataFetchJobParams, MetadataFetchScope},
	},
	job::stump_job::StumpJob,
	CoreEvent, MediaDeleted, SeriesDeleted,
};
use stump_media::image::remove_thumbnails;
use stump_scanner::ScanOptions;

use crate::{
	data::CoreContext,
	error_message,
	guard::PermissionGuard,
	input::{
		library::{
			CreateOrUpdateLibraryInput, PatchLibraryConfigInput, PatchLibraryInput,
		},
		thumbnail::UpdateThumbnailInput,
	},
	object::{library::Library, library_config::LibraryConfig},
};

#[derive(Default, SimpleObject)]
struct CleanLibraryResponse {
	deleted_media_count: usize,
	deleted_series_count: usize,
	is_empty: bool,
}

#[derive(Default)]
pub struct LibraryMutation;

#[Object]
impl LibraryMutation {
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn analyze_library(
		&self,
		ctx: &Context<'_>,
		id: ID,
		#[graphql(default = false)] force_reanalysis: bool,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let model = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.into_model::<LibraryIdentSelect>()
			.one(conn)
			.await?
			.ok_or("Library not found")?;

		core.enqueue(StumpJob::analyze_media(AnalysisJobConfig {
			force_reanalysis,
			scope: MediaAnalysisJobScope::Library(model.id),
		}))
		.await
		.map_err(crate::error::map_core_error)?;

		Ok(true)
	}

	/// Delete media and series from a library that match one of the following conditions:
	///
	/// - A series that is missing from disk (status is not `Ready`)
	/// - A media that is missing from disk (status is not `Ready`)
	/// - A series that is not associated with any media (i.e., no media in the series)
	///
	/// This operation will also remove any associated thumbnails of the deleted media and series.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn clean_library(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<CleanLibraryResponse> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		// This is primarily for access control assertion
		let _library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.into_model::<library::LibraryIdentSelect>()
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		let thumbnails_dir = core.config.get_thumbnails_dir();

		let txn = begin_write(core.conn.as_ref()).await?;

		let deleted_media = media::Entity::find()
			.filter(
				media::Column::Status.ne(FileStatus::Ready.to_string()).and(
					media::Column::SeriesId.in_subquery(
						Query::select()
							.column(series::Column::Id)
							.from(series::Entity)
							.and_where(series::Column::LibraryId.eq(id.to_string()))
							.to_owned(),
					),
				),
			)
			.all(&txn)
			.await?;
		let deleted_media_ids = deleted_media
			.iter()
			.map(|media| media.id.clone())
			.collect::<Vec<_>>();
		tracing::trace!(?deleted_media_ids, "Found media ids to delete");

		lists::remove_memberships_for_media(&txn, &deleted_media_ids).await?;
		media::Entity::delete_many()
			.filter(
				media::Column::Status.ne(FileStatus::Ready.to_string()).and(
					media::Column::SeriesId.in_subquery(
						Query::select()
							.column(series::Column::Id)
							.from(series::Entity)
							.and_where(series::Column::LibraryId.eq(id.to_string()))
							.to_owned(),
					),
				),
			)
			.exec(&txn)
			.await?;

		let deleted_series_ids = series::Entity::find()
			.select_only()
			.column(series::Column::Id)
			.filter(series::Column::LibraryId.eq(id.to_string()))
			.filter(
				Condition::any()
					.add(series::Column::Status.ne(FileStatus::Ready.to_string()))
					// TODO: Double check that this query is correct
					.add(
						series::Column::Id.not_in_subquery(
							Query::select()
								.column(media::Column::SeriesId)
								.distinct()
								.from(media::Entity)
								.to_owned(),
						),
					),
			)
			.into_tuple::<String>()
			.all(&txn)
			.await?;
		tracing::trace!(?deleted_series_ids, "Found series ids to delete");

		lists::remove_memberships_for_series(&txn, &deleted_series_ids).await?;
		series::Entity::delete_many()
			.filter(series::Column::LibraryId.eq(id.to_string()))
			.filter(
				Condition::any()
					.add(series::Column::Status.ne(FileStatus::Ready.to_string()))
					.add(
						series::Column::Id.not_in_subquery(
							Query::select()
								.column(media::Column::SeriesId)
								.distinct()
								.from(media::Entity)
								.to_owned(),
						),
					),
			)
			.exec(&txn)
			.await?;

		let is_library_empty = series::Entity::find()
			.filter(series::Column::LibraryId.eq(id.to_string()))
			.count(&txn)
			.await? == 0;

		txn.commit().await?;

		if !deleted_media_ids.is_empty() {
			if let Err(error) =
				remove_thumbnails(&deleted_media_ids, &thumbnails_dir).await
			{
				tracing::error!(?error, "Failed to remove thumbnails for library media");
			}
		}

		if !deleted_series_ids.is_empty() {
			if let Err(error) =
				remove_thumbnails(&deleted_series_ids, &thumbnails_dir).await
			{
				tracing::error!(?error, "Failed to remove thumbnails for library series");
			}
		}

		let event_library_id = id.to_string();
		for media in deleted_media {
			if let Some(series_id) = media.series_id {
				core.send_core_event(CoreEvent::MediaDeleted(MediaDeleted {
					id: media.id,
					series_id,
					library_id: event_library_id.clone(),
				}));
			}
		}
		for series_id in &deleted_series_ids {
			core.send_core_event(CoreEvent::SeriesDeleted(SeriesDeleted {
				id: series_id.clone(),
				library_id: event_library_id.clone(),
			}));
		}

		Ok(CleanLibraryResponse {
			deleted_media_count: deleted_media_ids.len(),
			deleted_series_count: deleted_series_ids.len(),
			is_empty: is_library_empty,
		})
	}

	/// Clear the scan history for a specific library
	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::ReadJobs, UserPermission::ManageLibrary])"
	)]
	async fn clear_scan_history(&self, ctx: &Context<'_>, id: ID) -> Result<u64> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		// This is primarily for access control assertion
		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.into_model::<library::LibraryIdentSelect>()
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		let affected_records = library_scan_record::Entity::delete_many()
			.filter(library_scan_record::Column::LibraryId.eq(library.id.clone()))
			.exec(core.conn.as_ref())
			.await?
			.rows_affected;

		Ok(affected_records)
	}

	/// Create a new library with the provided configuration. If `scan_after_persist` is `true`,
	/// the library will be scanned immediately after creation.
	#[tracing::instrument(skip(self, ctx))]
	#[graphql(guard = "PermissionGuard::one(UserPermission::CreateLibrary)")]
	async fn create_library(
		&self,
		ctx: &Context<'_>,
		mut input: CreateOrUpdateLibraryInput,
	) -> Result<Library> {
		let core = ctx.data::<CoreContext>()?;

		let created_library = stump_library::library::create_library(
			core,
			stump_library::library::NewLibrary {
				name: input.name,
				path: input.path,
				description: input.description,
				emoji: input.emoji,
				config: input.config.take().unwrap_or_default().into_active_model(),
				tags: input.tags.take().unwrap_or_default(),
				scan_after_persist: input.scan_after_persist,
			},
		)
		.await
		.map_err(crate::error::map_core_error)?;

		Ok(Library::from(created_library))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn reset_library_metadata(
		&self,
		ctx: &Context<'_>,
		id: ID,
		impact: MetadataResetImpact,
	) -> Result<Library> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.one(conn)
			.await?
			.ok_or("Library not found")?;

		let tx = begin_write(conn).await?;

		let series_ids: Vec<String> = series::Entity::find()
			.select_only()
			.column(series::Column::Id)
			.filter(series::Column::LibraryId.eq(library.id.clone()))
			.into_tuple()
			.all(&tx)
			.await?;

		if matches!(
			impact,
			MetadataResetImpact::Series | MetadataResetImpact::Everything
		) {
			let metadata_models = series_metadata::Entity::find()
				.filter(series_metadata::Column::SeriesId.is_in(series_ids.clone()))
				.all(&tx)
				.await?;
			tracing::trace!(
				count = metadata_models.len(),
				"Found series metadata to delete"
			);

			for metadata in metadata_models {
				metadata.delete(&tx).await?;
			}
		}

		if matches!(
			impact,
			MetadataResetImpact::Books | MetadataResetImpact::Everything
		) {
			let media_metadata_models = media_metadata::Entity::find()
				.filter(
					media_metadata::Column::MediaId.in_subquery(
						Query::select()
							.column(media::Column::Id)
							.from(media::Entity)
							.and_where(media::Column::SeriesId.is_in(series_ids))
							.to_owned(),
					),
				)
				.all(&tx)
				.await?;
			tracing::trace!(
				count = media_metadata_models.len(),
				"Found media metadata to delete"
			);

			for media_metadata in media_metadata_models {
				media_metadata.delete(&tx).await?;
			}
		}

		tx.commit().await?;

		tracing::debug!(?impact, library_id = ?library.id, "Reset metadata for library");

		Ok(library.into())
	}

	/// Update an existing library with the provided configuration. If `scan_after_persist` is `true`,
	/// the library will be scanned immediately after updating.
	#[graphql(
		guard = "PermissionGuard::one(UserPermission::EditLibrary)",
		deprecation = "Use `patchLibrary` instead"
	)]
	async fn update_library(
		&self,
		ctx: &Context<'_>,
		id: ID,
		mut input: CreateOrUpdateLibraryInput,
	) -> Result<Library> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let watch = if input.config.as_ref().is_some_and(|config| config.watch) {
			stump_library::library::WatchUpdate::Add
		} else {
			stump_library::library::WatchUpdate::Remove
		};

		let updated_library = stump_library::library::update_library(
			core,
			user,
			&id.to_string(),
			stump_library::library::UpdatedLibrary {
				name: input.name,
				path: input.path,
				description: input.description,
				emoji: input.emoji,
				config: Some(input.config.take().unwrap_or_default().into_active_model()),
				tags: input.tags.take(),
				scan_after_persist: input.scan_after_persist,
				watch,
			},
		)
		.await
		.map_err(crate::error::map_core_error)?;

		Ok(Library::from(updated_library))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::EditLibrary)")]
	async fn patch_library(
		&self,
		ctx: &Context<'_>,
		id: ID,
		input: PatchLibraryInput,
	) -> Result<Library> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let (existing_library, existing_config) = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.find_also_related(library_config::Entity)
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;
		let Some(existing_config) = existing_config else {
			return Err("Library is missing associated config!".into());
		};

		let final_name = input
			.name
			.clone()
			.unwrap_or_else(|| existing_library.name.clone());
		let final_path = input
			.path
			.clone()
			.unwrap_or_else(|| existing_library.path.clone());
		let scan_after_persist = input.scan_after_persist;
		let config_in_patch = input.config.is_some();
		let watch_update = match input.config.as_ref().and_then(|config| config.watch) {
			Some(true) if !existing_config.watch => {
				stump_library::library::WatchUpdate::Add
			},
			Some(false) if existing_config.watch => {
				stump_library::library::WatchUpdate::Remove
			},
			_ => stump_library::library::WatchUpdate::Keep,
		};
		let tags = match input.tags.clone() {
			MaybeUndefined::Null => Some(vec![]),
			MaybeUndefined::Value(tags) => Some(tags),
			MaybeUndefined::Undefined => None,
		};

		let (library, config) = input.apply(existing_library, existing_config)?;
		let updated_library = stump_library::library::patch_library(
			core,
			user,
			&id.to_string(),
			stump_library::library::PatchedLibrary {
				library,
				config: config_in_patch.then_some(config),
				name: final_name,
				path: final_path,
				tags,
				scan_after_persist,
				watch: watch_update,
			},
		)
		.await
		.map_err(crate::error::map_core_error)?;

		Ok(Library::from(updated_library))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::EditLibrary)")]
	async fn patch_library_config(
		&self,
		ctx: &Context<'_>,
		id: ID,
		input: PatchLibraryConfigInput,
	) -> Result<LibraryConfig> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let (_library, existing_config) = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.find_also_related(library_config::Entity)
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;
		let Some(existing_config) = existing_config else {
			return Err("Library is missing associated config!".into());
		};

		let watch = input.watch;
		let config = input.apply_to_model(existing_config)?;
		let updated_config = stump_library::library::patch_library_config(
			core,
			user,
			&id.to_string(),
			stump_library::library::PatchedLibraryConfig { config, watch },
		)
		.await
		.map_err(crate::error::map_core_error)?;

		Ok(LibraryConfig::from(updated_config))
	}

	/// Update the emoji for a library
	#[graphql(guard = "PermissionGuard::new(&[UserPermission::EditLibrary])")]
	async fn update_library_emoji(
		&self,
		ctx: &Context<'_>,
		id: ID,
		emoji: Option<String>,
	) -> Result<Library> {
		let core = ctx.data::<CoreContext>()?;
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;

		let existing_library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		let mut active_model = existing_library.into_active_model();
		active_model.emoji = Set(emoji);
		let updated_library = active_model.update(core.conn.as_ref()).await?;
		Ok(updated_library.into())
	}

	/// Update the thumbnail for a library. This will replace the existing thumbnail with the the one
	/// associated with the provided input (book). If the book does not have a thumbnail, one
	/// will be generated based on the library's thumbnail configuration.
	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::EditLibrary, UserPermission::EditThumbnails])"
	)]
	async fn update_library_thumbnail(
		&self,
		ctx: &Context<'_>,
		id: ID,
		input: UpdateThumbnailInput,
	) -> Result<Library> {
		let core = ctx.data::<CoreContext>()?;
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;

		let (library, config) = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.find_also_related(library_config::Entity)
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		let page = input.params.page();

		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(input.media_id))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Book not found")?;

		if book.extension == "epub" && page > 1 {
			return Err("Cannot set thumbnail from EPUB chapter".into());
		}

		let image_options = config
			.ok_or("Library config not found")?
			.thumbnail_config
			.unwrap_or_default()
			.with_page(page);

		let (_, path_buf, _) = generate_book_thumbnail(
			&book.into(),
			core.conn.as_ref(),
			GenerateThumbnailOptions {
				image_options,
				core_config: core.config.as_ref().clone(),
				force_regen: true,
				filename: Some(id.to_string()),
			},
		)
		.await?;
		tracing::debug!(path = ?path_buf, "Generated library thumbnail");

		Ok(library.into())
	}

	/// Exclude users from a library, preventing them from seeing the library in the UI. This operates as a
	/// full replacement of the excluded users list, so any users not included in the provided list will be
	/// removed from the exclusion list if they were previously excluded.
	///
	/// The server owner cannot be excluded from a library, nor can the user performing the action exclude
	/// themselves.
	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::ManageLibrary, UserPermission::ReadUsers])"
	)]
	async fn update_library_excluded_users(
		&self,
		ctx: &Context<'_>,
		id: ID,
		user_ids: Vec<String>,
	) -> Result<Library> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		if user_ids.contains(&user.id) {
			return Err("Cannot exclude self from library".into());
		}

		let server_owner_id = if user.is_server_owner {
			user.id.clone()
		} else {
			user::Entity::find()
				.select_only()
				.columns(vec![user::Column::Id, user::Column::Username])
				.filter(user::Column::IsServerOwner.eq(true))
				.into_model::<user::UserIdentSelect>()
				.one(core.conn.as_ref())
				.await?
				.ok_or("Server owner not found")?
				.id
		};

		if user_ids.contains(&server_owner_id) {
			tracing::error!(?user, library = ?id, "Attempted to exclude server owner from library");
			return Err(error_message::FORBIDDEN_ACTION.into());
		}

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		let existing_exclusions = library_exclusion::Entity::find()
			.filter(library_exclusion::Column::LibraryId.eq(library.id.clone()))
			.all(core.conn.as_ref())
			.await?;

		let to_add = user_ids
			.iter()
			.filter(|id| {
				!existing_exclusions
					.iter()
					.any(|exclusion| exclusion.user_id == **id)
			})
			.map(|id| library_exclusion::ActiveModel {
				library_id: Set(library.id.clone()),
				user_id: Set(id.clone()),
				..Default::default()
			})
			.collect::<Vec<_>>();

		let to_remove = existing_exclusions
			.iter()
			.filter(|exclusion| !user_ids.contains(&exclusion.user_id))
			.map(|exclusion| exclusion.id)
			.collect::<Vec<_>>();

		if to_add.is_empty() && to_remove.is_empty() {
			tracing::warn!("No changes to library exclusions");
			return Ok(Library::from(library));
		}

		let txn = begin_write(core.conn.as_ref()).await?;

		if !to_add.is_empty() {
			library_exclusion::Entity::insert_many(to_add)
				.on_conflict_do_nothing()
				.exec(&txn)
				.await?;
		}

		if !to_remove.is_empty() {
			library_exclusion::Entity::delete_many()
				.filter(library_exclusion::Column::Id.is_in(to_remove))
				.exec(&txn)
				.await?;
		}

		txn.commit().await?;

		// Note: We return the full node so the ID may be pulled to properly update the cache.
		Ok(Library::from(library))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn delete_library_scan_history(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<Library> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		library_scan_record::Entity::delete_many()
			.filter(library_scan_record::Column::LibraryId.eq(library.id.clone()))
			.exec(core.conn.as_ref())
			.await?;

		// Note: We return the full node so the ID may be pulled to properly update the cache.
		Ok(Library::from(library))
	}

	/// Delete a library, including all associated media and series via cascading deletes. This
	/// operation cannot be undone.
	#[graphql(guard = "PermissionGuard::one(UserPermission::DeleteLibrary)")]
	async fn delete_library(&self, ctx: &Context<'_>, id: ID) -> Result<Library> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let library = stump_library::library::delete_library(core, user, &id.to_string())
			.await
			.map_err(crate::error::map_core_error)?;

		// Note: We return the full node so the ID may be pulled to properly update the cache.
		// For obvious reasons, certain fields will error if accessed.
		Ok(Library::from(library))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn generate_library_thumbnails(
		&self,
		ctx: &Context<'_>,
		id: ID,
		#[graphql(default = false)] force_regenerate: bool,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let (library, config) = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.find_also_related(library_config::Entity)
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;
		let config = config.ok_or("Library config not found")?;

		if let Err(error) = core
			.enqueue(StumpJob::thumbnail_generation(
				config.thumbnail_config.unwrap_or_default(),
				ThumbnailGenerationJobParams::books_in_library(
					library.id,
					force_regenerate,
				),
			))
			.await
		{
			tracing::error!(?error, "Failed to enqueue thumbnail generation job");
			return Err(crate::error::map_core_error(error));
		}

		Ok(true)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn process_library_thumbnails(
		&self,
		ctx: &Context<'_>,
		id: ID,
		#[graphql(default = false)] force_regenerate: bool,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		if let Err(error) = core
			.enqueue(StumpJob::placeholder_generation(
				PlaceholderGenerationJobConfig {
					scope: PlaceholderGenerationJobScope::BooksInLibrary(
						library.id.clone(),
					),
					force_regenerate,
				},
			))
			.await
		{
			tracing::error!(?error, "Failed to enqueue placeholder generation job");
			return Err(crate::error::map_core_error(error));
		}

		Ok(true)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn delete_library_thumbnails(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let library = library::Entity::find_for_user(user)
			.select_only()
			.columns(LibraryIdentSelect::columns())
			.filter(library::Column::Id.eq(id.to_string()))
			.into_model::<library::LibraryIdentSelect>()
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;
		let library_id = library.id;

		let series = series::Entity::find()
			.filter(series::Column::LibraryId.eq(library_id.clone()))
			.select_only()
			.columns(series::SeriesIdentSelect::columns())
			.into_model::<series::SeriesIdentSelect>()
			.all(core.conn.as_ref())
			.await?;

		let books = media::Entity::find()
			.filter(
				media::Column::SeriesId.in_subquery(
					Query::select()
						.column(series::Column::Id)
						.from(series::Entity)
						.and_where(series::Column::LibraryId.eq(library_id.clone()))
						.to_owned(),
				),
			)
			.select_only()
			.columns(media::MediaIdentSelect::columns())
			.into_model::<media::MediaIdentSelect>()
			.all(core.conn.as_ref())
			.await?;

		let ids = chain(
			[library_id.clone()],
			series
				.iter()
				.map(|s| s.id.clone())
				.chain(books.iter().map(|b| b.id.clone())),
		)
		.collect::<Vec<_>>();

		let thumbnails_dir = core.config.get_thumbnails_dir();
		if let Err(error) = remove_thumbnails(&ids, &thumbnails_dir).await {
			tracing::error!(?error, "Failed to remove library thumbnails");
			return Err(error.into());
		}

		let updated_at = Some(DateTimeWithTimeZone::from(Utc::now()));
		let txn = core.conn.as_ref().begin().await?;

		library::Entity::update_many()
			.filter(library::Column::Id.eq(library_id.clone()))
			.col_expr(library::Column::ThumbnailPath, Expr::value(None::<String>))
			.col_expr(
				library::Column::ThumbnailMeta,
				Expr::value(None::<models::shared::image::ImageMetadata>),
			)
			.col_expr(library::Column::UpdatedAt, Expr::value(updated_at))
			.exec(&txn)
			.await?;

		series::Entity::update_many()
			.filter(series::Column::LibraryId.eq(library_id.clone()))
			.col_expr(series::Column::ThumbnailPath, Expr::value(None::<String>))
			.col_expr(
				series::Column::ThumbnailMeta,
				Expr::value(None::<models::shared::image::ImageMetadata>),
			)
			.col_expr(series::Column::UpdatedAt, Expr::value(updated_at))
			.exec(&txn)
			.await?;

		media::Entity::update_many()
			.filter(
				media::Column::SeriesId.in_subquery(
					Query::select()
						.column(series::Column::Id)
						.from(series::Entity)
						.and_where(series::Column::LibraryId.eq(library_id))
						.to_owned(),
				),
			)
			.col_expr(media::Column::ThumbnailPath, Expr::value(None::<String>))
			.col_expr(
				media::Column::ThumbnailMeta,
				Expr::value(None::<models::shared::image::ImageMetadata>),
			)
			.col_expr(media::Column::UpdatedAt, Expr::value(updated_at))
			.exec(&txn)
			.await?;

		txn.commit().await?;

		Ok(true)
	}

	/// Start a job which will search external metadata providers
	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataFetchRecordManage)")]
	#[tracing::instrument(skip(self, ctx))]
	async fn fetch_library_metadata(
		&self,
		ctx: &Context<'_>,
		id: ID,
		#[graphql(default = false)] force_refetch: bool,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let (library, config) = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.find_also_related(library_config::Entity)
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		let library_type = config.ok_or("Library config not found")?.library_type;

		let has_relevant_provider = metadata_provider_config::Entity::find()
			.filter(metadata_provider_config::Column::Enabled.eq(true))
			.all(core.conn.as_ref())
			.await
			.unwrap_or_default()
			.into_iter()
			.any(|config| library_type.has_provider_overlap(&config.provider_type));

		if !has_relevant_provider {
			tracing::debug!(
				?library_type,
				"No compatible metadata providers for this library type"
			);
			return Ok(false);
		}

		core.enqueue(StumpJob::metadata_fetch(MetadataFetchJobParams {
			force_refetch,
			scope: MetadataFetchScope::MediaInLibrary(library.id),
		}))
		.await
		.map_err(crate::error::map_core_error)?;
		tracing::debug!("Enqueued library metadata fetch job");

		Ok(true)
	}

	/// Bulk-set locked metadata fields for all series metadata in a library
	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn set_library_series_locked_fields(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
		locked_fields: Vec<MetadataField>,
	) -> Result<u64> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(library_id.to_string()))
			.one(conn)
			.await?
			.ok_or("Library not found")?;

		let locked_json = serde_json::to_value(&locked_fields)?;
		let library_id_str = library.id.clone();

		let series_ids: Vec<String> = series::Entity::find()
			.filter(series::Column::LibraryId.eq(&library_id_str))
			.select_only()
			.column(series::Column::Id)
			.into_tuple()
			.all(conn)
			.await?;

		if series_ids.is_empty() {
			return Ok(0);
		}

		let result = series_metadata::Entity::update_many()
			.col_expr(
				series_metadata::Column::LockedFields,
				sea_orm::sea_query::Expr::value(locked_json.to_string()),
			)
			.filter(series_metadata::Column::SeriesId.is_in(series_ids))
			.exec(conn)
			.await?;

		tracing::debug!(
			library_id = ?library_id_str,
			?locked_fields,
			updated = result.rows_affected,
			"Set locked fields for series metadata in library"
		);

		Ok(result.rows_affected)
	}

	/// Bulk-set locked metadata fields for all media metadata in a library
	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn set_library_media_locked_fields(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
		locked_fields: Vec<MetadataField>,
	) -> Result<u64> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(library_id.to_string()))
			.one(conn)
			.await?
			.ok_or("Library not found")?;

		let locked_json = serde_json::to_value(&locked_fields)?;
		let library_id_str = library.id.clone();

		let media_ids: Vec<String> = media::Entity::find()
			.filter(
				media::Column::SeriesId.in_subquery(
					Query::select()
						.column(series::Column::Id)
						.from(series::Entity)
						.and_where(series::Column::LibraryId.eq(&library_id_str))
						.to_owned(),
				),
			)
			.select_only()
			.column(media::Column::Id)
			.into_tuple()
			.all(conn)
			.await?;

		if media_ids.is_empty() {
			return Ok(0);
		}

		let result = media_metadata::Entity::update_many()
			.col_expr(
				media_metadata::Column::LockedFields,
				sea_orm::sea_query::Expr::value(locked_json.to_string()),
			)
			.filter(media_metadata::Column::MediaId.is_in(media_ids))
			.exec(conn)
			.await?;

		tracing::debug!(
			library_id = ?library_id_str,
			?locked_fields,
			updated = result.rows_affected,
			"Set locked fields for media metadata in library"
		);

		Ok(result.rows_affected)
	}

	/// Enqueue a scan job for a library. This will index the filesystem from the library's root path
	/// and update the database accordingly.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ScanLibrary)")]
	async fn scan_library(
		&self,
		ctx: &Context<'_>,
		id: ID,
		options: Option<Json<ScanOptions>>,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.into_model::<library::LibraryIdentSelect>()
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		core.enqueue(StumpJob::library_scan(
			library.id,
			library.path,
			options.map(|o| o.0),
		))
		.await
		.map_err(crate::error::map_core_error)?;
		tracing::debug!("Enqueued library scan job");

		Ok(true)
	}

	/// "Visit" a library, which will upsert a record of the user's last visit to the library.
	/// This is used to inform the UI of the last library which was visited by the user
	async fn visit_library(&self, ctx: &Context<'_>, id: ID) -> Result<Library> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let library = library::Entity::find_for_user(user)
			.filter(library::Column::Id.eq(id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Library not found")?;

		let active_model = last_library_visit::ActiveModel {
			library_id: Set(library.id.clone()),
			user_id: Set(user.id.clone()),
			timestamp: Set(Utc::now().into()),
			..Default::default()
		};

		last_library_visit::Entity::insert(active_model)
			.on_conflict(
				OnConflict::new()
					.update_column(last_library_visit::Column::Timestamp)
					.to_owned(),
			)
			.exec(core.conn.as_ref())
			.await?;

		Ok(Library::from(library))
	}
}
