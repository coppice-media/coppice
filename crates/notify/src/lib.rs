//! Notification channels and routing for Stump. See `crates/notify/README.md`
//! and `docs/content/docs/developer/notifications.mdx`.
//!
//! `channel` holds the [`Channel`] trait and the neutral [`Notification`] /
//! [`Attachment`] types; `routing` resolves per-user rules to channel targets.
//! Concrete channels are feature-gated: `ntfy` (HTTP push), `email` (SMTP via
//! the `email` crate), and `webhook` (signed JSON POST). `registry` bundles
//! the enabled channels; the host resolves a [`Channel`] by id when dispatching.

pub mod channel;
#[cfg(feature = "email")]
pub mod email;
#[cfg(feature = "ntfy")]
pub mod ntfy;
pub mod registry;
pub mod routing;
#[cfg(feature = "webhook")]
pub mod webhook;

pub use channel::{
	Attachment, AttachmentContent, Channel, ChannelError, Inbound, Notification,
	NotificationKind, Recipient,
};
#[cfg(feature = "email")]
pub use email::EmailChannel;
#[cfg(feature = "ntfy")]
pub use ntfy::NtfyChannel;
pub use registry::ChannelRegistry;
pub use routing::{resolve_channels, resolve_targets, Audience, Rule, Target};
#[cfg(feature = "webhook")]
pub use webhook::WebhookChannel;
