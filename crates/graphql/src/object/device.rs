use async_graphql::{ComplexObject, Context, Result, SimpleObject};
use models::{
	entity::device,
	shared::enums::{DeviceCredentialKind, DeviceProtocol},
};
use stump_devices::{service::secret_hint, Endpoint, IssuedCredential};

use crate::data::CoreContext;

/// A registered client (Kobo, KOReader, Mihon, Komelia, Liseur, an OPDS
/// reader, a script, or a browser) owned by a user.
#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct Device {
	#[graphql(flatten)]
	pub model: device::Model,
}

impl From<device::Model> for Device {
	fn from(model: device::Model) -> Self {
		Self { model }
	}
}

/// The credential a device authenticates with, redacted.
#[derive(Debug, Clone, SimpleObject)]
pub struct DeviceCredentialSummary {
	pub kind: DeviceCredentialKind,
	/// The protocol the credential was minted for
	pub protocol: DeviceProtocol,
	/// The visible part of the secret, e.g. `stump_abcdefgh_…`
	pub secret_hint: String,
}

#[ComplexObject]
impl Device {
	/// The device's credential, or `null` once it has been revoked or when the
	/// device was registered by a protocol (KOReader, OPDS) and never minted one.
	async fn credential(
		&self,
		ctx: &Context<'_>,
	) -> Result<Option<DeviceCredentialSummary>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.devices()
			.credential(&self.model.id)
			.await?
			.map(|credential| DeviceCredentialSummary {
				kind: credential.credential_kind,
				protocol: credential.protocol,
				secret_hint: secret_hint(&credential),
			}))
	}

	/// The libraries this device may see, or `null` when the device inherits
	/// its user's visibility. The scope is intersected with that visibility,
	/// so it only ever narrows; an empty list is a device that sees nothing.
	async fn library_scope(&self) -> Option<Vec<String>> {
		self.model.library_scope_ids()
	}
}

/// A device together with a freshly minted credential. The secret inside
/// `credential` and the endpoints is shown exactly once.
#[derive(Debug, Clone, SimpleObject)]
pub struct DeviceWithCredential {
	pub device: Device,
	pub credential: IssuedCredential,
	pub endpoints: Vec<Endpoint>,
}
