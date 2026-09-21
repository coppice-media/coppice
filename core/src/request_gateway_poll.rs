//! Durable poll/retry/import step for an opaque request grab.

#[cfg(feature = "ingest")]
use std::path::Path;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use models::entity::{
	book_request, book_request_gateway_setting, book_request_grab, book_request_handoff,
	book_request_release, library, media, series,
};
use sea_orm::{
	ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
	Set,
};

use crate::{request_gateway::GatewayClient, CoreError, CoreResult, Ctx};

const REQUEST_GATEWAY_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn poll_book_request_grab(ctx: &Ctx, grab_id: &str) -> CoreResult<()> {
	let conn = ctx.conn.as_ref();
	let grab = book_request_grab::Entity::find_by_id(grab_id)
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("request grab not found".into()))?;
	if matches!(grab.status.as_str(), "COMPLETED" | "FAILED") {
		return Ok(());
	}
	let request = book_request::Entity::find_by_id(&grab.request_id)
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("book request not found".into()))?;
	let settings = book_request_gateway_setting::Entity::find_by_id("default")
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("request gateway is not configured".into()))?;
	let key = ctx.get_encryption_key().await?;
	let token =
		crate::utils::encryption::decrypt_string(&settings.encrypted_token, &key)?;
	let client = GatewayClient::new(&settings.endpoint, token, REQUEST_GATEWAY_TIMEOUT)
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	let status = client
		.status(&grab.opaque_id)
		.await
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	let now = Utc::now().fixed_offset();
	let normalized = status.status.to_ascii_uppercase();
	let mut active = grab.clone().into_active_model();
	active.last_polled_at = Set(Some(now));
	if matches!(normalized.as_str(), "FAILED" | "ERROR") {
		if grab.attempts < grab.max_attempts {
			let release = book_request_release::Entity::find_by_id(&grab.release_id)
				.one(conn)
				.await?
				.ok_or_else(|| CoreError::NotFound("request release not found".into()))?;
			let retry = client
				.grab(&release.remote_id, &request.id)
				.await
				.map_err(|error| CoreError::BadRequest(error.to_string()))?;
			active.opaque_id = Set(retry.opaque_id);
			active.status = Set(if retry.status.is_empty() {
				"RETRYING".into()
			} else {
				retry.status
			});
			active.attempts = Set(grab.attempts + 1);
			active.failure_code = Set(crate::request_gateway::safe_failure_code(
				status.failure_code.as_deref(),
			));
			active.failure_message = Set(crate::request_gateway::safe_failure_message(
				status.failure_message.as_deref(),
			));
			active.next_poll_at = Set(Some(now + ChronoDuration::seconds(30)));
			active.update(conn).await?;
			let mut request_active = request.clone().into_active_model();
			request_active.status = Set("GRABBED".into());
			request_active.retries = Set(grab.attempts);
			request_active.failure_code = Set(crate::request_gateway::safe_failure_code(
				status.failure_code.as_deref(),
			));
			request_active.failure_message =
				Set(crate::request_gateway::safe_failure_message(
					status.failure_message.as_deref(),
				));
			request_active.update(conn).await?;
			return Ok(());
		}
		active.status = Set("FAILED".into());
		active.failure_code = Set(crate::request_gateway::safe_failure_code(
			status.failure_code.as_deref(),
		)
		.or_else(|| Some("GATEWAY_GRAB_FAILED".into())));
		active.failure_message = Set(crate::request_gateway::safe_failure_message(
			status.failure_message.as_deref(),
		));
		active.finished_at = Set(Some(now));
		active.update(conn).await?;
		let mut request = request.into_active_model();
		request.status = Set("FAILED".into());
		request.failure_code = Set(crate::request_gateway::safe_failure_code(
			status.failure_code.as_deref(),
		)
		.or_else(|| Some("GATEWAY_GRAB_FAILED".into())));
		request.failure_message = Set(crate::request_gateway::safe_failure_message(
			status.failure_message.as_deref(),
		));
		request.update(conn).await?;
		return Ok(());
	}
	if matches!(normalized.as_str(), "COMPLETE" | "COMPLETED" | "DOWNLOADED") {
		let Some(handoff) = status.handoff else {
			active.status = Set("COMPLETED_PENDING_HANDOFF".into());
			active.failure_code = Set(Some("HANDOFF_MISSING".into()));
			active.failure_message =
				Set(Some("gateway completed without a handoff manifest".into()));
			active.next_poll_at = Set(Some(now + ChronoDuration::seconds(30)));
			active.update(conn).await?;
			let mut request = request.into_active_model();
			request.status = Set("IMPORTING".into());
			request.failure_code = Set(Some("HANDOFF_MISSING".into()));
			request.failure_message =
				Set(Some("gateway completed without a handoff manifest".into()));
			request.update(conn).await?;
			return Ok(());
		};
		let handoff_result = stage_handoff(ctx, &request, &grab, &handoff).await;
		match handoff_result {
			Ok(()) => {
				active.status = Set("COMPLETED".into());
				active.finished_at = Set(Some(now));
				active.update(conn).await?;
				let mut request = request.into_active_model();
				request.status = Set("QUEUED".into());
				request.completed_at = Set(Some(now));
				request.failure_code = Set(None);
				request.failure_message = Set(None);
				request.update(conn).await?;
			},
			Err(error) => {
				active.status = Set("FAILED".into());
				active.failure_code = Set(Some("HANDOFF_IMPORT_FAILED".into()));
				active.failure_message = Set(Some(error.to_string()));
				active.finished_at = Set(Some(now));
				active.update(conn).await?;
				let mut request = request.into_active_model();
				request.status = Set("FAILED".into());
				request.failure_code = Set(Some("HANDOFF_IMPORT_FAILED".into()));
				request.failure_message = Set(Some(error.to_string()));
				request.update(conn).await?;
			},
		}
		return Ok(());
	}
	active.status = Set(normalized);
	active.next_poll_at = Set(Some(now + ChronoDuration::seconds(30)));
	active.update(conn).await?;
	Ok(())
}

#[cfg(feature = "ingest")]
async fn stage_handoff(
	ctx: &Ctx,
	request: &book_request::Model,
	grab: &book_request_grab::Model,
	handoff: &crate::request_gateway::GatewayHandoff,
) -> CoreResult<()> {
	let settings = book_request_gateway_setting::Entity::find_by_id("default")
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| CoreError::NotFound("request gateway is not configured".into()))?;
	let root = settings.handoff_root.ok_or_else(|| {
		CoreError::BadRequest("gateway handoff root is not configured".into())
	})?;
	if !Path::new(&root).is_absolute() {
		return Err(CoreError::BadRequest(
			"gateway handoff root must be absolute".into(),
		));
	}
	let relative =
		stump_ingest::staging::normalize_relative_path(Some(&handoff.relative_path))
			.map_err(|error| CoreError::BadRequest(error.to_string()))?
			.ok_or_else(|| CoreError::BadRequest("handoff path is empty".into()))?;
	let root = tokio::fs::canonicalize(&root).await?;
	let path = root.join(&relative);
	let canonical = tokio::fs::canonicalize(&path).await?;
	if !canonical.starts_with(&root) {
		return Err(CoreError::Forbidden(
			"handoff path escapes configured root".into(),
		));
	}
	let metadata = tokio::fs::metadata(&canonical).await?;
	if !metadata.is_file() {
		return Err(CoreError::BadRequest(
			"handoff target is not a regular file".into(),
		));
	}
	let (sha256, bytes) = stump_ingest::staging::hash_file(&canonical)
		.await
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	if handoff
		.sha256
		.as_deref()
		.is_some_and(|expected| expected != sha256)
		|| handoff
			.byte_size
			.is_some_and(|expected| expected != bytes as i64)
	{
		return Err(CoreError::BadRequest(
			"handoff digest or size verification failed".into(),
		));
	}
	let library_id = resolve_library_id(ctx, request).await?;
	let filename = handoff
		.filename
		.as_deref()
		.or_else(|| canonical.file_name().and_then(|value| value.to_str()))
		.ok_or_else(|| CoreError::BadRequest("handoff filename is invalid".into()))?;
	let reader = tokio::fs::File::open(&canonical).await?;
	let staged = ctx
		.ingest()
		.store
		.stage_upload(
			&library_id,
			Some(&request.requester_id),
			None,
			filename,
			reader,
			Some(&format!("request:{}", request.id)),
		)
		.await
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	ctx.ingest()
		.coordinator
		.enqueue(vec![staged.item.id.clone()], false)
		.await
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	let now = Utc::now().fixed_offset();
	book_request_handoff::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		request_id: Set(request.id.clone()),
		grab_id: Set(grab.id.clone()),
		library_id: Set(library_id),
		relative_path: Set(relative),
		sha256: Set(sha256),
		byte_size: Set(bytes as i64),
		status: Set("QUEUED".into()),
		drop_item_id: Set(Some(staged.item.id)),
		shelf_id: Set(request.destination_shelf_id.clone()),
		device_id: Set(request.destination_device_id.clone()),
		error_code: Set(None),
		error_message: Set(None),
		created_at: Set(now),
		updated_at: Set(now),
	}
	.insert(ctx.conn.as_ref())
	.await?;
	Ok(())
}

#[cfg(not(feature = "ingest"))]
async fn stage_handoff(
	_ctx: &Ctx,
	_request: &book_request::Model,
	_grab: &book_request_grab::Model,
	_handoff: &crate::request_gateway::GatewayHandoff,
) -> CoreResult<()> {
	Err(CoreError::FeatureDisabled("ingest"))
}

async fn resolve_library_id(
	ctx: &Ctx,
	request: &book_request::Model,
) -> CoreResult<String> {
	if let Some(media_id) = request.internal_media_id.as_deref() {
		let media = media::Entity::find_by_id(media_id)
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| {
				CoreError::NotFound("request media no longer exists".into())
			})?;
		let series_id = media.series_id.ok_or_else(|| {
			CoreError::BadRequest("request media has no library series".into())
		})?;
		let series = series::Entity::find_by_id(series_id)
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| {
				CoreError::NotFound("request series no longer exists".into())
			})?;
		return series
			.library_id
			.ok_or_else(|| CoreError::NotFound("request series has no library".into()));
	}
	if let Some(work_id) = request.internal_work_id.as_deref() {
		let link = models::entity::liseur_sync_media_link::Entity::find()
			.filter(models::entity::liseur_sync_media_link::Column::WorkId.eq(work_id))
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| {
				CoreError::NotFound("request work has no local edition".into())
			})?;
		let media = media::Entity::find_by_id(link.media_id)
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| {
				CoreError::NotFound("request work edition no longer exists".into())
			})?;
		let series_id = media.series_id.ok_or_else(|| {
			CoreError::BadRequest("request work edition has no library series".into())
		})?;
		let series = series::Entity::find_by_id(series_id)
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| {
				CoreError::NotFound("request series no longer exists".into())
			})?;
		return series
			.library_id
			.ok_or_else(|| CoreError::NotFound("request series has no library".into()));
	}
	library::Entity::find()
		.order_by_asc(library::Column::Id)
		.one(ctx.conn.as_ref())
		.await?
		.map(|row| row.id)
		.ok_or_else(|| {
			CoreError::NotFound(
				"create a library before importing an external request".into(),
			)
		})
}
