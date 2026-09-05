//! Registry of the concrete channels a host offers its users.
//!
//! `ntfy` and `webhook` need no host configuration; the email channel wraps
//! the server-wide emailer config, so the host builds the registry through
//! [`ChannelRegistry::from_channels`] once it has loaded that config.

use std::sync::Arc;

use crate::Channel;

/// The channels available on this host, addressable by their stable
/// [`Channel::id`].
#[derive(Clone, Default)]
pub struct ChannelRegistry {
	channels: Arc<[Arc<dyn Channel>]>,
}

impl ChannelRegistry {
	pub fn from_channels(channels: Vec<Arc<dyn Channel>>) -> Self {
		Self {
			channels: channels.into(),
		}
	}

	/// All channels in registration order; used to render the settings UI.
	pub fn all(&self) -> &[Arc<dyn Channel>] {
		&self.channels
	}

	pub fn get(&self, id: &str) -> Option<Arc<dyn Channel>> {
		self.channels
			.iter()
			.find(|channel| channel.id() == id)
			.cloned()
	}
}
