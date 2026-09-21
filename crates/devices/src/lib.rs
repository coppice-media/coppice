//! Unified device registry.
//!
//! A *device* is a registered client (Kobo, KOReader, Coppice, Mihon,
//! Komelia, Liseur, Kavita, an OPDS reader, a script, a browser) owned by a
//! user. The registry credentials a device authenticates with (a prefixed API
//! key, including Kavita's download-only key, a liseur-sync token, or both for
//! Coppice) tell the client which endpoints to configure, and record last-seen
//! / last-sync state every time a credential authenticates.
//! The crate is persistence-only: it depends on the SeaORM entities in
//! `models` and knows nothing about HTTP. The server calls
//! [`DeviceService::touch`] from its protocol auth paths and forwards the
//! returned [`DeviceSeen`] through the core event channel.
//!
//! Decisions, layout, and verification commands: `crates/devices/README.md`.

mod credential;
mod endpoint;
mod error;
mod event;
pub mod kindle;
pub mod scope;
pub mod service;
pub mod telemetry;

#[cfg(test)]
mod tests;

pub use credential::{
	api_key_permissions_for, credential_kind_for, protocol_for, required_permissions,
	CredentialRef, IssuedCredential,
};
pub use endpoint::Endpoint;
pub use error::{DeviceError, DeviceResult};
pub use event::DeviceSeen;
pub use kindle::{normalize_kindle_email, KindleSendSummary, KINDLE_EMAIL_PROTOCOL};
pub use models::entity::device::Model as Device;
pub use models::entity::device_credential::Model as DeviceCredential;
pub use models::shared::enums::{
	DeviceCredentialKind as CredentialKind, DeviceKind, DeviceProtocol as Protocol,
};
pub use scope::LibraryScope;
pub use service::{secret_hint, DeviceAuth, DeviceService, SeenListener};
pub use telemetry::{
	DeviceTelemetryCounters, DeviceTelemetryCountersPatch, DeviceTelemetryPatch,
	DeviceTelemetrySnapshot,
};

pub mod liseur_token {
	//! Secret generation, hashing, and expiry shared with the liseur-sync storage.
	pub use crate::credential::liseur::{hash_secret, new_secret, DEVICE_TOKEN_TTL_SECS};
}
