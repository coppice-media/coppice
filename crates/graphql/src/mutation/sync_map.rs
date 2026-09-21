//! GraphQL writes for importing a validated map and explicitly requesting
//! alignment.

use async_graphql::{Context, Error, Json, Object, Result, ID};
use models::shared::enums::UserPermission;
use stump_library::sync_maps;
use stump_worker::SyncMapV1;

use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	object::worker::WorkerJob,
	object::{sync_map::AlignGranularity, sync_map::SyncMap},
};

#[derive(Default)]
pub struct SyncMapMutation;

#[Object]
impl SyncMapMutation {
	/// Validate and persist an operator-supplied `SyncMapV1` for a confirmed
	/// ebook↔audiobook pair.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn import_sync_map(
		&self,
		ctx: &Context<'_>,
		ebook_media_id: ID,
		audio_media_id: ID,
		map: Json<serde_json::Value>,
	) -> Result<SyncMap> {
		let typed: SyncMapV1 = serde_json::from_value(map.0)
			.map_err(|error| Error::new(format!("invalid SyncMap JSON: {error}")))?;
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let model = sync_maps::import_sync_map_for_user(
			conn,
			&auth.user.id,
			ebook_media_id.as_str(),
			audio_media_id.as_str(),
			typed,
		)
		.await
		.map_err(|error| Error::new(error.to_string()))?;
		Ok(SyncMap::from(model))
	}

	/// Enqueue alignment for a confirmed pair, reusing any active identical
	/// request rather than creating a second worker job.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn enqueue_alignment(
		&self,
		ctx: &Context<'_>,
		ebook_media_id: ID,
		audio_media_id: ID,
		#[graphql(default)] granularity: AlignGranularity,
	) -> Result<WorkerJob> {
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let jobs = core.worker_jobs();
		let model = sync_maps::enqueue_alignment_for_user(
			core.conn.as_ref(),
			jobs.as_ref(),
			&auth.user.id,
			ebook_media_id.as_str(),
			audio_media_id.as_str(),
			granularity.into(),
		)
		.await
		.map_err(|error| Error::new(error.to_string()))?;
		Ok(WorkerJob::from(model))
	}
}
