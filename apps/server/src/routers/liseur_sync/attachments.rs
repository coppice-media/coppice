//! Annotation attachments: the binary side objects of a liseur-sync
//! annotation (a Kobo markup SVG, its page snapshot, a notebook export).
//!
//! Rows live in `annotation_attachments` and are owned by
//! [`models::services::annotation_attachment`]; the bytes live under
//! `<config_dir>/attachments/<annotation_id>/<sha256>.<ext>`. This module is
//! the only place that joins the two, and it exposes three surfaces:
//!
//! * the `PUT`/`GET` halves of the protocol crate's attachment routes,
//! * [`detach_all`]/[`unlink_all`], used when an annotation is tombstoned, and
//! * `GET /api/v2/annotations/{id}/attachments/{attachment_id}`, the
//!   authenticated download used by the web UI and by a paired device.
//!
//! Attachments never touch an annotation's revision or feed sequence: they are
//! keyed by annotation id and the compare-and-set contract is unaffected.

use std::path::{Path as FsPath, PathBuf};

use axum::{
	extract::{Path, Request, State},
	http::{header, HeaderMap, HeaderValue, StatusCode},
	middleware,
	response::{IntoResponse, Response},
	routing::get,
	Extension, Router,
};
use models::{
	entity::annotation_attachment::Model as Attachment,
	services::annotation_attachment as attachment_rows,
};
use sea_orm::ConnectionTrait;
use stump_auth::AuthContext;
use stump_liseur_sync::{
	AttachmentRecord, AttachmentUpload, AttachmentUploadResult, LiseurSyncError,
	LiseurTokenKind,
};
use tower_sessions::Session;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::{auth::auth_middleware, HostExtractor},
};

use super::storage::{self, internal};

/// `GET /api/v2/annotations/{id}/attachments/{attachment_id}` — the bytes of
/// one attachment, for the owner only.
///
/// The credential is either a Stump session/API key (the web UI) or the
/// liseur device secret already used on `/v1/*`, so a device can round-trip
/// what it uploaded without minting a second credential.
pub(crate) fn router(ctx: AppState) -> Router<AppState> {
	Router::new()
		.route(
			"/api/v2/annotations/{id}/attachments/{attachment_id}",
			get(download),
		)
		.layer(middleware::from_fn_with_state(ctx, device_or_stump_auth))
}

/// Accept the liseur device bearer, otherwise fall back to Stump's own
/// session/bearer middleware. A liseur secret is not a Stump JWT, so the
/// generic middleware would reject it outright.
async fn device_or_stump_auth(
	State(ctx): State<AppState>,
	host: HostExtractor,
	session: Session,
	mut request: Request,
	next: middleware::Next,
) -> Result<Response, Response> {
	let secret = request
		.headers()
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.strip_prefix("Bearer "))
		.filter(|value| !value.is_empty())
		.map(str::to_owned);

	if let Some(secret) = secret {
		if let Ok(token) = storage::authenticate(&ctx, &secret).await {
			if token.kind == LiseurTokenKind::Device {
				request.extensions_mut().insert(token.context.clone());
				request.extensions_mut().insert(token);
				return Ok(next.run(request).await);
			}
		}
	}

	auth_middleware(State(ctx), host, session, request, next)
		.await
		.map_err(IntoResponse::into_response)
}

async fn download(
	Path((annotation_id, attachment_id)): Path<(String, String)>,
	State(ctx): State<AppState>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<Response> {
	let attachment = attachment_rows::find(
		ctx.conn.as_ref(),
		&auth.user.id,
		&annotation_id,
		&attachment_id,
	)
	.await?
	.ok_or_else(|| APIError::NotFound("attachment not found".to_string()))?;

	let path = resolve_path(&ctx, &attachment);
	let bytes = tokio::fs::read(&path).await.map_err(|error| {
		APIError::NotFound(format!("attachment bytes are missing: {error}"))
	})?;

	let mut headers = HeaderMap::new();
	headers.insert(
		header::CONTENT_TYPE,
		HeaderValue::from_str(&attachment.media_type).map_err(|_| {
			APIError::InternalServerError("stored media type is invalid".to_string())
		})?,
	);
	// The path is content-addressed, so the bytes behind this URL can never
	// change: revalidation is pointless.
	headers.insert(
		header::CACHE_CONTROL,
		HeaderValue::from_static("private, max-age=31536000, immutable"),
	);
	headers.insert(
		header::CONTENT_DISPOSITION,
		HeaderValue::from_str(&format!(
			"inline; filename=\"{}.{}\"",
			attachment.kind,
			attachment_rows::extension_for(&attachment.media_type)
		))
		.map_err(|_| {
			APIError::InternalServerError("stored kind is invalid".to_string())
		})?,
	);

	Ok((StatusCode::OK, headers, bytes).into_response())
}

/// Absolute path of an attachment's bytes.
fn resolve_path(ctx: &AppState, attachment: &Attachment) -> PathBuf {
	ctx.config
		.get_attachments_dir()
		.join(&attachment.storage_path)
}

/// The liseur-sync backend's `attachments` callback.
pub(super) async fn list(
	ctx: &AppState,
	user_id: &str,
	annotation_id: &str,
) -> Result<Vec<AttachmentRecord>, LiseurSyncError> {
	require_live_annotation(ctx.conn.as_ref(), user_id, annotation_id).await?;
	Ok(
		attachment_rows::list(ctx.conn.as_ref(), user_id, annotation_id)
			.await
			.map_err(internal)?
			.into_iter()
			.map(record)
			.collect(),
	)
}

/// The liseur-sync backend's `put_attachment` callback.
///
/// The row is written after the bytes land, so a stored row never points at a
/// missing file. A retry of the same digest writes neither.
pub(super) async fn put(
	ctx: &AppState,
	user_id: &str,
	annotation_id: &str,
	upload: AttachmentUpload,
) -> Result<AttachmentUploadResult, LiseurSyncError> {
	require_live_annotation(ctx.conn.as_ref(), user_id, annotation_id).await?;

	let byte_size = upload.bytes.len() as i64;
	let new = attachment_rows::NewAttachment {
		user_id: user_id.to_owned(),
		annotation_id: annotation_id.to_owned(),
		kind: upload.kind,
		media_type: upload.media_type,
		byte_size,
		sha256: upload.sha256,
	};
	let root = ctx.config.get_attachments_dir();
	let relative =
		attachment_rows::relative_path(&new.annotation_id, &new.sha256, &new.media_type);
	write_bytes(&root.join(&relative), &upload.bytes).await?;

	let stored = attachment_rows::upsert(ctx.conn.as_ref(), new).await;
	let stored = match stored {
		Ok(stored) => stored,
		Err(error) => {
			// The bytes are not referenced by any row; do not leave them.
			remove_file(&root.join(&relative)).await;
			return Err(internal(error));
		},
	};
	if let attachment_rows::Upserted::Replaced { previous, .. } = &stored {
		if previous.storage_path != relative {
			remove_file(&root.join(&previous.storage_path)).await;
		}
	}

	let attachment = stored.attachment();
	Ok(AttachmentUploadResult {
		id: attachment.id.clone(),
		sha256: attachment.sha256.clone(),
		byte_size: attachment.byte_size,
	})
}

/// Drop every attachment row of an annotation inside an open transaction and
/// return the rows whose bytes [`unlink_all`] must remove after the commit.
pub(super) async fn detach_all<C: ConnectionTrait>(
	txn: &C,
	user_id: &str,
	annotation_id: &str,
) -> Result<Vec<Attachment>, LiseurSyncError> {
	attachment_rows::delete_for_annotation(txn, user_id, annotation_id)
		.await
		.map_err(internal)
}

/// Remove the bytes of already-deleted rows. Failures are logged, never
/// propagated: the rows are gone and the annotation is tombstoned either way.
pub(super) async fn unlink_all(ctx: &AppState, attachments: &[Attachment]) {
	if attachments.is_empty() {
		return;
	}
	let root = ctx.config.get_attachments_dir();
	for attachment in attachments {
		remove_file(&root.join(&attachment.storage_path)).await;
	}
	// The per-annotation directory is only ours; drop it once it is empty.
	if let Some(annotation_id) = attachments.first().map(|row| &row.annotation_id) {
		let _ = tokio::fs::remove_dir(root.join(annotation_id)).await;
	}
}

/// Attachments hang off a live annotation; a tombstone has none.
async fn require_live_annotation<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation_id: &str,
) -> Result<(), LiseurSyncError> {
	let row = conn
		.query_one(storage::db_statement(
			conn,
			"SELECT deleted FROM liseur_sync_annotations
             WHERE user_id = $1 AND annotation_id = $2",
			vec![user_id.to_owned().into(), annotation_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	let deleted: bool = match row {
		Some(row) => row.try_get("", "deleted").map_err(internal)?,
		None => {
			return Err(LiseurSyncError::NotFound("annotation not found".into()));
		},
	};
	if deleted {
		Err(LiseurSyncError::NotFound("annotation not found".into()))
	} else {
		Ok(())
	}
}

fn record(attachment: Attachment) -> AttachmentRecord {
	AttachmentRecord {
		id: attachment.id,
		annotation_id: attachment.annotation_id,
		kind: attachment.kind,
		media_type: attachment.media_type,
		byte_size: attachment.byte_size,
		sha256: attachment.sha256,
		created_at: attachment.created_at,
	}
}

/// Write `bytes` at `path`, creating the annotation directory. The write goes
/// through a sibling temporary file so a reader never observes a partial
/// content-addressed file.
async fn write_bytes(path: &FsPath, bytes: &[u8]) -> Result<(), LiseurSyncError> {
	if let Some(parent) = path.parent() {
		tokio::fs::create_dir_all(parent).await.map_err(internal)?;
	}
	let temporary = path.with_extension("partial");
	tokio::fs::write(&temporary, bytes)
		.await
		.map_err(internal)?;
	tokio::fs::rename(&temporary, path).await.map_err(internal)
}

async fn remove_file(path: &FsPath) {
	if let Err(error) = tokio::fs::remove_file(path).await {
		if error.kind() != std::io::ErrorKind::NotFound {
			tracing::warn!(?path, ?error, "failed to remove attachment bytes");
		}
	}
}
