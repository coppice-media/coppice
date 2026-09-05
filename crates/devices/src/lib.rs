//! Unified device registry.
//!
//! A *device* is a registered client (Kobo, KOReader, Mihon, Komelia, Liseur,
//! an OPDS reader, a script, a browser) owned by a user. The registry mints the
//! credential a device authenticates with (a prefixed API key, or a liseur-sync
//! token), tells the client which endpoints to configure, and records
//! last-seen / last-sync state every time a credential authenticates a
//! request.
//!
//! The crate is persistence-only: it depends on the SeaORM entities in
//! `models` and knows nothing about HTTP. The server calls
//! [`DeviceService::touch`] from its protocol auth paths and forwards the
//! returned [`DeviceSeen`] through the core event channel.

mod credential;
mod endpoint;
mod error;
mod event;
pub mod service;

#[cfg(test)]
mod tests;

pub use credential::{
	api_key_permissions_for, credential_kind_for, protocol_for, required_permissions,
	CredentialRef, IssuedCredential,
};
pub use endpoint::Endpoint;
pub use error::{DeviceError, DeviceResult};
pub use event::DeviceSeen;
pub use models::entity::device::Model as Device;
pub use models::entity::device_credential::Model as DeviceCredential;
pub use models::shared::enums::{
	DeviceCredentialKind as CredentialKind, DeviceKind, DeviceProtocol as Protocol,
};
pub use service::{DeviceService, SeenListener};

pub mod liseur_token {
	//! Secret generation, hashing, and expiry shared with the liseur-sync storage.
	pub use crate::credential::liseur::{hash_secret, new_secret, DEVICE_TOKEN_TTL_SECS};
}
