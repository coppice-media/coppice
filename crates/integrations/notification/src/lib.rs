//! Discord/Telegram notification clients (dormant: no dependant in this tree).
//!
//! See `crates/integrations/notification/README.md` for the crate contract and
//! decisions.

mod discord_client;
mod error;
mod event;
mod telegram_client;

pub use discord_client::DiscordClient;
pub use event::NotificationEvent;
pub use telegram_client::TelegramClient;

use self::error::NotificationResult;

pub const NOTIFIER_ID: &str = "Stump Notifier";
pub const FAVICON_URL: &str = "https://stumpapp.dev/favicon.png";

#[async_trait::async_trait]
pub trait NotificationClient {
	// TODO: MessageConfig struct? So we can style according to NotificationEvent?
	fn payload_from_event(
		event: NotificationEvent,
	) -> NotificationResult<serde_json::Value>;
	async fn send_message(&self, event: NotificationEvent) -> NotificationResult<()>;
}
