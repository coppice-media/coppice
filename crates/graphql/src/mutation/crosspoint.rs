//! GraphQL mutations for CrossPoint target verification and delivery control.

use std::{collections::HashSet, str::FromStr};

use async_graphql::{Context, Error, Object, Result, ID};
use chrono::Utc;
use models::entity::{crosspoint_delivery_queue, crosspoint_device_target};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use stump_crosspoint::{
	profile::{
		CrosspointTargetModel as ProtocolTargetModel,
		CrosspointTransferProfile as ProtocolProfile,
		CrosspointTransferProfileInput as ProtocolProfileInput,
	},
	storage::{DeliveryStatus, CROSSPOINT_HTTP_PORT, CROSSPOINT_WS_PORT},
	transfer::{CrossPointClient, CrossPointEndpoint, CrossPointModel, TransferConfig},
};

use crate::{
	data::CoreContext,
	input::crosspoint::{CrosspointTargetInput, CrosspointTransferProfileInput},
	object::crosspoint::{
		CrosspointDelivery, CrosspointTarget, CrosspointTargetVerification,
	},
	query::crosspoint::{owned_crosspoint_device, visible_media_ids_for_device},
};

#[derive(Default)]
pub struct CrosspointMutation;

#[Object]
impl CrosspointMutation {
	/// Probe a private IPv4 address using CrossPoint's fixed HTTP port. The
	/// returned identity is deliberately not persisted until the user submits
	/// [`Self::update_crosspoint_target`].
	async fn verify_crosspoint_target(
		&self,
		ctx: &Context<'_>,
		device_id: ID,
		host: String,
	) -> Result<CrosspointTargetVerification> {
		let (_core, _user, _device) = owned_crosspoint_device(ctx, &device_id).await?;
		let endpoint = CrossPointEndpoint::new(host.trim())
			.map_err(|error| Error::new(error.to_string()))?;
		let client = CrossPointClient::new(TransferConfig::default())
			.map_err(|error| Error::new(error.to_string()))?;
		let status = client
			.probe_target(&endpoint)
			.await
			.map_err(|error| Error::new(error.to_string()))?;

		Ok(CrosspointTargetVerification {
			device_id,
			host_or_ip: endpoint.host().to_string(),
			http_port: endpoint.http_port() as i32,
			ws_port: endpoint.ws_port() as i32,
			model: status.model.to_string(),
			serial: status.serial,
			verified: true,
		})
	}

	/// Verify and persist a CrossPoint target plus its canonical typed profile.
	/// The status probe is repeated here so a client cannot forge a prior
	/// verification result between the two mutations.
	async fn update_crosspoint_target(
		&self,
		ctx: &Context<'_>,
		device_id: ID,
		input: CrosspointTargetInput,
	) -> Result<CrosspointTarget> {
		let (core, user, device) = owned_crosspoint_device(ctx, &device_id).await?;
		let device_id = device_id.to_string();
		let existing = crosspoint_device_target::Entity::find_by_id(device_id.clone())
			.one(core.conn.as_ref())
			.await?;
		if let Some(target) = existing.as_ref() {
			if target.user_id != user.id {
				return Err(Error::new(
					"CrossPoint target is not owned by the authenticated user",
				));
			}
		}

		let CrosspointTargetInput {
			host_or_ip,
			http_port,
			ws_port,
			root_path,
			profile,
		} = input;
		validate_fixed_ports(http_port, ws_port)?;
		let endpoint = CrossPointEndpoint::new(host_or_ip.trim())
			.map_err(|error| Error::new(error.to_string()))?;
		let root_path = normalize_remote_path(&root_path, "target root_path", true)?;
		// Normalize and validate the profile before opening the status socket.
		let profile = normalize_profile(profile, existing.as_ref())?;

		let client = CrossPointClient::new(TransferConfig::default())
			.map_err(|error| Error::new(error.to_string()))?;
		let status = client
			.probe_target(&endpoint)
			.await
			.map_err(|error| Error::new(error.to_string()))?;
		let detected_model = match status.model {
			CrossPointModel::X3 => ProtocolTargetModel::X3,
			CrossPointModel::X4 => ProtocolTargetModel::X4,
		};
		profile
			.clone()
			.resolve_for_target_model(detected_model)
			.map_err(|error| Error::new(error))?;

		let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();
		let profile_json = serde_json::to_value(&profile).map_err(|error| {
			Error::new(format!("failed to serialize CrossPoint profile: {error}"))
		})?;
		let profile_digest = profile.digest();
		let fingerprint = serde_json::json!({
			"model": status.model.to_string(),
			"serial": status.serial,
		});
		let row = match existing {
			Some(existing) => {
				let mut active: crosspoint_device_target::ActiveModel = existing.into();
				active.user_id = Set(user.id.clone());
				active.host_or_ip = Set(endpoint.host().to_string());
				active.http_port = Set(CROSSPOINT_HTTP_PORT as i32);
				active.ws_port = Set(CROSSPOINT_WS_PORT as i32);
				active.root_path = Set(root_path);
				active.discovery_method = Set("manual".to_string());
				active.verified_at = Set(Some(now));
				active.fingerprint = Set(Some(fingerprint));
				active.profile_json = Set(profile_json);
				active.profile_digest = Set(profile_digest);
				active.updated_at = Set(now);
				active.revoked_at = Set(None);
				active.update(core.conn.as_ref()).await?
			},
			None => {
				crosspoint_device_target::ActiveModel {
					device_id: Set(device.id.clone()),
					user_id: Set(user.id.clone()),
					host_or_ip: Set(endpoint.host().to_string()),
					http_port: Set(CROSSPOINT_HTTP_PORT as i32),
					ws_port: Set(CROSSPOINT_WS_PORT as i32),
					root_path: Set(root_path),
					discovery_method: Set("manual".to_string()),
					verified_at: Set(Some(now)),
					fingerprint: Set(Some(fingerprint)),
					profile_json: Set(profile_json),
					profile_digest: Set(profile_digest),
					created_at: Set(now),
					updated_at: Set(now),
					revoked_at: Set(None),
				}
				.insert(core.conn.as_ref())
				.await?
			},
		};
		CrosspointTarget::from_model(row).map_err(|error| Error::new(error))
	}

	/// Queue one or more visible media items using the target's immutable
	/// profile snapshot. Duplicate media ids are collapsed before enqueueing;
	/// the service's idempotency key handles repeated requests safely.
	async fn queue_crosspoint_deliveries(
		&self,
		ctx: &Context<'_>,
		device_id: ID,
		media_ids: Vec<ID>,
		target_path: String,
	) -> Result<Vec<CrosspointDelivery>> {
		let (core, user, device) = owned_crosspoint_device(ctx, &device_id).await?;
		let device_id = device_id.to_string();
		let target = crosspoint_device_target::Entity::find_by_id(device_id.clone())
			.one(core.conn.as_ref())
			.await?
			.ok_or_else(|| Error::new("CrossPoint target has not been verified"))?;
		if target.user_id != user.id {
			return Err(Error::new(
				"CrossPoint target is not owned by the authenticated user",
			));
		}
		if target.revoked_at.is_some() {
			return Err(Error::new("CrossPoint target has been revoked"));
		}
		validate_fixed_ports(target.http_port, target.ws_port)?;
		CrossPointEndpoint::new(&target.host_or_ip)
			.map_err(|error| Error::new(error.to_string()))?;
		validate_verified_fingerprint(&target)?;
		let _ = persisted_profile(&target)?;
		let target_path = normalize_remote_path(&target_path, "destination path", false)?;

		let mut requested = Vec::with_capacity(media_ids.len());
		let mut seen = HashSet::with_capacity(media_ids.len());
		for media_id in media_ids {
			let media_id = media_id.to_string();
			if seen.insert(media_id.clone()) {
				requested.push(media_id);
			}
		}
		if requested.is_empty() {
			return Ok(Vec::new());
		}
		let visible =
			visible_media_ids_for_device(&core, user, &device, &requested).await?;
		let visible = visible.into_iter().collect::<HashSet<_>>();
		if visible.len() != requested.len()
			|| requested.iter().any(|id| !visible.contains(id))
		{
			return Err(Error::new(
                "one or more media items are outside the authenticated user's device scope",
            ));
		}

		let service = core
			.crosspoint_delivery()
			.map_err(|error| Error::new(error.to_string()))?;
		let mut deliveries = Vec::with_capacity(requested.len());
		for media_id in requested {
			let row = service
				.enqueue_for_media(
					user.id.clone(),
					device.id.clone(),
					media_id,
					&target_path,
					None,
				)
				.await
				.map_err(|error| Error::new(error.to_string()))?;
			deliveries.push(CrosspointDelivery::from(row));
		}
		Ok(deliveries)
	}

	/// Requeue a failed delivery without changing its immutable source/profile
	/// snapshot. Cancelled, active, and completed rows cannot be resurrected.
	async fn retry_crosspoint_delivery(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<CrosspointDelivery> {
		let (core, row) = owned_queue(ctx, &id).await?;
		if row.status != DeliveryStatus::Failed.as_str() {
			return Err(Error::new(
				"only failed CrossPoint deliveries can be retried",
			));
		}
		let service = core
			.crosspoint_delivery()
			.map_err(|error| Error::new(error.to_string()))?;
		if !service
			.retry(&row.id)
			.await
			.map_err(|error| Error::new(error.to_string()))?
		{
			return Err(Error::new(
				"CrossPoint delivery changed before it could be retried",
			));
		}
		let row = crosspoint_delivery_queue::Entity::find_by_id(row.id)
			.one(core.conn.as_ref())
			.await?
			.ok_or_else(|| Error::new("CrossPoint delivery disappeared after retry"))?;
		Ok(CrosspointDelivery::from(row))
	}

	/// Cancel only queued local work. No remote cancellation request is sent;
	/// active transfers remain owned by the worker and terminal history stays.
	async fn cancel_crosspoint_delivery(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<CrosspointDelivery> {
		let (core, row) = owned_queue(ctx, &id).await?;
		if row.status == DeliveryStatus::Cancelled.as_str() {
			return Ok(CrosspointDelivery::from(row));
		}
		if row.status != DeliveryStatus::Queued.as_str() {
			return Err(Error::new(
				"only queued CrossPoint deliveries can be cancelled",
			));
		}
		let service = core
			.crosspoint_delivery()
			.map_err(|error| Error::new(error.to_string()))?;
		if !service
			.cancel(&row.id)
			.await
			.map_err(|error| Error::new(error.to_string()))?
		{
			return Err(Error::new(
				"CrossPoint delivery changed before it could be cancelled",
			));
		}
		let row = crosspoint_delivery_queue::Entity::find_by_id(row.id)
			.one(core.conn.as_ref())
			.await?
			.ok_or_else(|| {
				Error::new("CrossPoint delivery disappeared after cancellation")
			})?;
		Ok(CrosspointDelivery::from(row))
	}
}

async fn owned_queue(
	ctx: &Context<'_>,
	id: &ID,
) -> Result<(CoreContext, crosspoint_delivery_queue::Model)> {
	let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
	let core = ctx.data::<CoreContext>()?;
	let row = crosspoint_delivery_queue::Entity::find_by_id(id.to_string())
		.one(core.conn.as_ref())
		.await?
		.ok_or_else(|| Error::new("CrossPoint delivery was not found"))?;
	if row.user_id != user.id {
		return Err(Error::new(
			"CrossPoint delivery is not owned by the authenticated user",
		));
	}
	let device_id = ID(row.device_id.clone());
	let _ = owned_crosspoint_device(ctx, &device_id).await?;
	Ok((core.clone(), row))
}

fn normalize_profile(
	input: Option<CrosspointTransferProfileInput>,
	existing: Option<&crosspoint_device_target::Model>,
) -> Result<ProtocolProfile> {
	let base = existing
		.map(persisted_profile)
		.transpose()?
		.unwrap_or_default();
	let patch = input
		.map(CrosspointTransferProfileInput::into_protocol)
		.unwrap_or_default();
	ProtocolProfileInput {
		optimizer_enabled: patch.optimizer_enabled.or(Some(base.optimizer_enabled)),
		target_model: patch.target_model.or(Some(base.target_model)),
		jpeg_quality: patch.jpeg_quality.or(Some(base.jpeg_quality)),
		grayscale: patch.grayscale.or(Some(base.grayscale)),
		auto_crop: patch.auto_crop.or(Some(base.auto_crop)),
		split_large_paragraphs: patch
			.split_large_paragraphs
			.or(Some(base.split_large_paragraphs)),
		remove_fonts: patch.remove_fonts.or(Some(base.remove_fonts)),
		chunk_bytes: patch.chunk_bytes.or(Some(base.chunk_bytes)),
		retry_count: patch.retry_count.or(Some(base.retry_count)),
		retry_delay_seconds: patch.retry_delay_seconds.or(Some(base.retry_delay_seconds)),
		timeout_seconds: patch.timeout_seconds.or(Some(base.timeout_seconds)),
		max_upload_bytes: patch.max_upload_bytes.or(Some(base.max_upload_bytes)),
	}
	.normalized()
	.map_err(|error| Error::new(error.to_string()))
}

fn persisted_profile(
	target: &crosspoint_device_target::Model,
) -> Result<ProtocolProfile> {
	let profile: ProtocolProfile = serde_json::from_value(target.profile_json.clone())
		.map_err(|error| {
			Error::new(format!("invalid CrossPoint target profile: {error}"))
		})?;
	let profile = profile.validate().map_err(|error| {
		Error::new(format!("invalid CrossPoint target profile: {error}"))
	})?;
	if profile.digest() != target.profile_digest {
		return Err(Error::new(
			"CrossPoint target profile digest does not match its JSON",
		));
	}
	Ok(profile)
}

fn validate_fixed_ports(http_port: i32, ws_port: i32) -> Result<()> {
	if http_port != CROSSPOINT_HTTP_PORT as i32 || ws_port != CROSSPOINT_WS_PORT as i32 {
		return Err(Error::new(
			"CrossPoint target ports must be HTTP 80 and WebSocket 81",
		));
	}
	Ok(())
}

fn validate_verified_fingerprint(target: &crosspoint_device_target::Model) -> Result<()> {
	if target.verified_at.is_none() {
		return Err(Error::new("CrossPoint target has not been verified"));
	}
	let fingerprint = target
		.fingerprint
		.as_ref()
		.ok_or_else(|| Error::new("CrossPoint target has no verified fingerprint"))?;
	let model = fingerprint
		.get("model")
		.and_then(serde_json::Value::as_str)
		.ok_or_else(|| Error::new("CrossPoint target fingerprint has no model"))?;
	let serial = fingerprint
		.get("serial")
		.and_then(serde_json::Value::as_str)
		.filter(|serial| !serial.trim().is_empty())
		.ok_or_else(|| Error::new("CrossPoint target fingerprint has no serial"))?;
	CrossPointModel::from_str(model).map_err(|error| Error::new(error.to_string()))?;
	if serial.eq_ignore_ascii_case("not found") {
		return Err(Error::new(
			"CrossPoint target fingerprint has no concrete serial",
		));
	}
	Ok(())
}

fn normalize_remote_path(
	value: &str,
	field: &str,
	require_absolute: bool,
) -> Result<String> {
	let value = value.trim();
	let value = if value.is_empty() { "/" } else { value };
	if require_absolute && !value.starts_with('/') {
		return Err(Error::new(format!("{field} must be absolute")));
	}
	if value.contains('\\') || value.chars().any(char::is_control) {
		return Err(Error::new(format!(
			"{field} contains a forbidden separator or control character"
		)));
	}
	for component in value.split('/') {
		if component.is_empty() {
			continue;
		}
		if component == "." || component == ".." || component.contains(':') {
			return Err(Error::new(format!(
				"{field} contains an unsafe path component"
			)));
		}
	}
	Ok(value.to_string())
}
