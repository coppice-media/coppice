use async_graphql::{Context, Error, Object, Result, ID};
use models::shared::enums::DeviceKind;
use stump_devices::{
	CredentialIssuance, Device as DeviceModel, IssuedCredential, LibraryScope,
};

use crate::{
	data::CoreContext,
	object::device::{Device, DeviceWithCredential},
};

#[derive(Default)]
pub struct DeviceMutation;

#[Object]
impl DeviceMutation {
	/// Registers a device for the current user and mints its credential(s). The
	/// registry enforces the permissions the kind needs (`ACCESS_API_KEYS` plus
	/// the protocol permission for API-key kinds); secrets are returned once.
	/// `allowKomfMetadataEditing` defaults to false and only enables Komelia
	/// metadata edits when the creator holds `EditMetadata`.
	async fn create_device(
		&self,
		ctx: &Context<'_>,
		kind: DeviceKind,
		name: Option<String>,
		#[graphql(default = false)] allow_komf_metadata_editing: bool,
	) -> Result<DeviceWithCredential> {
		let req_ctx = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let issuance = credential_issuance(req_ctx);
		let devices = core.devices();
		let (device, credential) = if kind == DeviceKind::Komelia {
			devices
				.create_komelia_device(
					&req_ctx.user,
					issuance,
					name,
					allow_komf_metadata_editing,
				)
				.await?
		} else {
			if allow_komf_metadata_editing {
				return Err(Error::new(
					"Komf metadata editing is only available for Komelia devices",
				));
			}
			devices
				.create_device(&req_ctx.user, issuance, kind, name)
				.await?
		};
		with_credential(ctx, device, credential).await
	}

	async fn rename_device(
		&self,
		ctx: &Context<'_>,
		id: String,
		name: String,
	) -> Result<Device> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		Ok(Device::from(core.devices().rename(user, &id, &name).await?))
	}

	/// Discards the device's credential and mints a replacement; the new secret
	/// is returned once.
	async fn rotate_device_credential(
		&self,
		ctx: &Context<'_>,
		id: String,
	) -> Result<DeviceWithCredential> {
		let req_ctx = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let devices = core.devices();
		let device = devices.get(&req_ctx.user, &id).await?;
		let credential = devices
			.rotate_credential(&req_ctx.user, credential_issuance(req_ctx), &id)
			.await?;
		with_credential(ctx, device, credential).await
	}

	/// Invalidates the device's credential; the device stays listed as revoked.
	async fn revoke_device(&self, ctx: &Context<'_>, id: String) -> Result<Device> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		Ok(Device::from(core.devices().revoke(user, &id).await?))
	}

	/// Replaces the device's transform profile (`null` clears it).
	///
	/// The document is interpreted before it is stored — a preset name, a
	/// `{"preset": …}` selector, or a full/partial profile object including
	/// the `audio` section — so an unknown preset or an unusable Opus
	/// bitrate is a mutation error here rather than a warning logged once
	/// per delivery request for the rest of the device's life.
	async fn set_device_transform_profile(
		&self,
		ctx: &Context<'_>,
		id: String,
		profile: Option<serde_json::Value>,
	) -> Result<Device> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		if let Some(profile) = profile.as_ref() {
			if let Some(result) =
				stump_media::transform::TransformProfile::from_device_profile(profile)
			{
				result.map_err(|error| Error::new(error.to_string()))?;
			}
		}

		Ok(Device::from(
			core.devices()
				.set_transform_profile(user, &id, profile)
				.await?,
		))
	}

	/// Restricts the device to `library_ids`, or clears the restriction when
	/// `library_ids` is null so the device inherits its user's visibility. The
	/// scope is intersected with that visibility, so it can only narrow what
	/// the device sees and never reveal a library the user cannot see; an
	/// empty list is a device that sees nothing.
	async fn set_device_library_scope(
		&self,
		ctx: &Context<'_>,
		id: String,
		library_ids: Option<Vec<ID>>,
	) -> Result<Device> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let library_ids =
			library_ids.map(|ids| ids.into_iter().map(|id| id.to_string()).collect());

		Ok(Device::from(
			core.devices()
				.set_library_scope(user, &id, LibraryScope::from(library_ids))
				.await?,
		))
	}

	/// Sets the device's Amazon *Send to Kindle* address, or clears it when
	/// `email` is null. A device with an address is a `sendToKindle` target;
	/// one without cannot be sent to. The address is validated syntactically
	/// before it is stored, but never verified with Amazon — only a delivery
	/// can do that.
	async fn set_device_kindle_email(
		&self,
		ctx: &Context<'_>,
		id: String,
		email: Option<String>,
	) -> Result<Device> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		Ok(Device::from(
			core.devices()
				.set_kindle_email(user, &id, email.as_deref())
				.await?,
		))
	}
}

pub(super) fn credential_issuance(
	req_ctx: &stump_auth::AuthContext,
) -> CredentialIssuance {
	if req_ctx.api_key().is_some() {
		CredentialIssuance::DelegatedCredential
	} else {
		CredentialIssuance::InteractiveSession
	}
}

async fn with_credential(
	ctx: &Context<'_>,
	device: DeviceModel,
	credential: IssuedCredential,
) -> Result<DeviceWithCredential> {
	let stump_auth::AuthContext { user, .. } = ctx.data::<stump_auth::AuthContext>()?;
	let core = ctx.data::<CoreContext>()?;
	let origin = ctx.data::<stump_api_types::RequestOrigin>()?;
	let credentials = credential.credentials();
	let endpoints = core
		.devices()
		.endpoints_with_secret(user, &device, &credential, origin)
		.await?;

	Ok(DeviceWithCredential {
		device: Device::from(device),
		credential,
		credentials,
		endpoints,
	})
}
