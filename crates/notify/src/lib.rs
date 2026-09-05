//! Notification channels and routing for Stump. See `crates/notify/README.md`
//! (pending) and `docs/content/docs/developer/notifications.mdx`.
//!
//! `channel` holds the [`Channel`] trait and the neutral [`Notification`] /
//! [`Attachment`] types; `routing` resolves per-user rules to channel targets.
//! Concrete channels (`ntfy`, `email`, `webhook`) are feature-gated modules and
//! land together with the dispatch job.

pub mod channel;
pub mod routing;

pub use channel::{
	Attachment, AttachmentContent, Channel, ChannelError, Inbound, Notification,
	NotificationKind, Recipient,
};
pub use routing::{resolve_channels, resolve_targets, Audience, Rule, Target};
