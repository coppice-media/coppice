//! The [ntfy](https://ntfy.sh) push channel.
//!
//! Publishes each [`Notification`] as an HTTP `POST` to `{server_url}/{topic}`.
//! Metadata rides in ntfy's `X-` headers (`X-Title`, `X-Tags`, `X-Click`,
//! `X-Message`); an attachment is delivered as the request body with
//! `X-Filename` + `Content-Type`, while the text body moves into `X-Message`
//! (ntfy's documented way to attach a file and keep the message).
//!
//! Self-hosted servers are supported through the `server_url` setting; an
//! optional `access_token` (secret) authenticates against protected topics.

use std::sync::LazyLock;

use rand::Rng;
use stump_api_types::settings::{SettingDefinition, SettingKind, SettingValues};
use serde_json::json;

use crate::{Channel, ChannelError, Notification, Recipient};

const CHANNEL_ID: &str = "ntfy";

static SETTINGS: LazyLock<Vec<SettingDefinition>> = LazyLock::new(|| {
	vec![
		SettingDefinition {
			key: "server_url",
			label: "Server URL",
			description:
				"Base URL of the ntfy server (https://ntfy.sh or your self-hosted instance)",
			kind: SettingKind::String,
			default: json!("https://ntfy.sh"),
			required: true,
			secret: false,
			help_url: Some("https://docs.ntfy.sh/config/"),
		},
		SettingDefinition {
			key: "topic",
			label: "Topic",
			description: "The ntfy topic notifications are published to",
			kind: SettingKind::String,
			default: json!(""),
			required: true,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "access_token",
			label: "Access token",
			description: "Optional token for password-protected topics",
			kind: SettingKind::String,
			default: json!(""),
			required: false,
			secret: true,
			help_url: Some("https://docs.ntfy.sh/publish/#access-tokens"),
		},
	]
});

/// The ntfy channel. Stateless; every user-specific value arrives through the
/// [`Recipient`]'s settings.
pub struct NtfyChannel {
	client: reqwest::Client,
}

impl Default for NtfyChannel {
	fn default() -> Self {
		Self::new()
	}
}

impl NtfyChannel {
	pub fn new() -> Self {
		Self {
			client: reqwest::Client::new(),
		}
	}

	/// A per-user default topic: `stump-<short user id>-<random>` so parallel
	/// installations sharing one ntfy server never collide.
	pub fn default_topic(user_id: &str) -> String {
		let short_id: String = user_id.chars().take(8).collect();
		let random: String = (0..6)
			.map(|_| {
				let index = rand::rng().random_range(0..26);
				(b'a' + index) as char
			})
			.collect();
		format!("stump-{short_id}-{random}")
	}
}

#[async_trait::async_trait]
impl Channel for NtfyChannel {
	fn id(&self) -> &'static str {
		CHANNEL_ID
	}

	fn label(&self) -> &'static str {
		"ntfy"
	}

	fn settings(&self) -> &[SettingDefinition] {
		SETTINGS.as_slice()
	}

	fn default_settings(&self, user_id: &str) -> SettingValues {
		let mut values = SettingValues::new();
		for definition in self.settings() {
			values.insert(
				definition.key.to_string(),
				definition.default.clone(),
			);
		}
		values.insert("topic".to_string(), json!(Self::default_topic(user_id)));
		values
	}

	async fn send(
		&self,
		recipient: &Recipient,
		notification: &Notification,
	) -> Result<(), ChannelError> {
		let server = recipient
			.required_str(CHANNEL_ID, "server_url")?
			.trim_end_matches('/')
			.to_string();
		let topic = recipient.required_str(CHANNEL_ID, "topic")?.to_string();
		let url = format!("{server}/{topic}");

		let mut request = self
			.client
			.post(&url)
			.header("X-Title", &notification.title)
			.header("X-Tags", notification.kind.tags())
			.header("X-Message", &notification.body);

		if let Some(link) = &notification.link {
			request = request.header("X-Click", link);
		}

		if let Some(token) = recipient.optional_str("access_token") {
			request = request.bearer_auth(token);
		}

		let request = match &notification.attachment {
			Some(attachment) => {
				let bytes = attachment.load()?;
				request
					.header("X-Filename", &attachment.filename)
					.header(reqwest::header::CONTENT_TYPE, &attachment.mime)
					.body(bytes.into_owned())
			},
			None => request.body(notification.body.clone()),
		};

		let response = request
			.send()
			.await
			.map_err(|error| ChannelError::Transport(error.to_string()))?;

		let status = response.status();
		if status.is_success() {
			Ok(())
		} else if status.is_client_error() {
			let body = response.text().await.unwrap_or_default();
			Err(ChannelError::Rejected(format!(
				"ntfy returned {status}: {body}"
			)))
		} else {
			let body = response.text().await.unwrap_or_default();
			Err(ChannelError::Transport(format!(
				"ntfy returned {status}: {body}"
			)))
		}
	}
}

 #[cfg(test)]
 mod http_tests {
 	use super::*;
 	use crate::{Attachment, NotificationKind};
 	use std::io::{Read, Write};
 	use std::net::TcpListener;
 	use std::sync::mpsc;

 	/// Minimal HTTP mock: answers each request with `200 OK` and forwards the
 	/// raw request text (headers + body) to the test over a channel.
 	struct MockServer {
 		url: String,
 		requests: mpsc::Receiver<String>,
 	}

 	impl MockServer {
 		fn spawn(connections: usize) -> Self {
 			let listener =
 				TcpListener::bind("127.0.0.1:0").expect("failed to bind mock server");
 			let url = format!("http://{}", listener.local_addr().unwrap());
 			let (sender, receiver) = mpsc::channel();
 			std::thread::spawn(move || {
 				for _ in 0..connections {
 					let Ok((mut stream, _)) = listener.accept() else {
 						break;
 					};
 					let mut buf = Vec::new();
 					let mut chunk = [0u8; 1024];
 					while stream.read(&mut chunk).is_ok_and(|n| n > 0) {
 						buf.extend_from_slice(&chunk[..chunk.len().min(1024)]);
 						let header_end = buf
 							.windows(4)
 							.position(|window| window == b"\r\n\r\n");
 						let Some(position) = header_end else {
 							continue;
 						};
 						let length = content_length(&buf);
 						if buf.len() >= position + 4 + length {
 							break;
 						}
 					}
 					if sender
 						.send(String::from_utf8_lossy(&buf).to_string())
 						.is_err()
 					{
 						break;
 					}
 					let _ = stream
 						.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
 					let _ = stream.flush();
 				}
 			});
 			Self { url, requests: receiver }
 		}

 		fn request(&self) -> String {
 			self.requests
 				.recv_timeout(std::time::Duration::from_secs(5))
 				.expect("mock server received a request")
 		}
 	}

 	fn content_length(buf: &[u8]) -> usize {
 		let headers = String::from_utf8_lossy(buf);
 		headers
 			.lines()
 			.find_map(|line| {
 				let (name, value) = line.split_once(':')?;
 				name.trim()
 					.eq_ignore_ascii_case("content-length")
 					.then(|| value.trim().parse::<usize>().ok())?
 			})
 			.unwrap_or(0)
 	}

 	fn recipient(server_url: &str, topic: &str, token: Option<&str>) -> Recipient {
 		let mut settings = SettingValues::new();
 		settings.insert("server_url".into(), json!(server_url));
 		settings.insert("topic".into(), json!(topic));
 		if let Some(token) = token {
 			settings.insert("access_token".into(), json!(token));
 		}
 		Recipient::new("user-1", "reader", settings)
 	}

 	#[tokio::test]
 	async fn posts_message_headers_and_body() {
 		let server = MockServer::spawn(1);
 		let notification = Notification::new(
 			NotificationKind::ScanFinished,
 			"Scan finished",
 			"3 series added",
 		)
 		.with_link("https://stump.example/libraries");

 		NtfyChannel::new()
 			.send(
 				&recipient(&server.url, "stump-topic", Some("tok-123")),
 				&notification,
 			)
 			.await
 			.expect("delivery");

 		let request = server.request();
 		assert!(request.starts_with("POST /stump-topic HTTP/1.1"));
 		assert!(request.contains("X-Title: Scan finished"));
 		assert!(request.contains("X-Tags: books"));
 		assert!(request.contains("X-Click: https://stump.example/libraries"));
 		assert!(request.contains("X-Message: 3 series added"));
 		assert!(request.contains("Authorization: Bearer tok-123"));
 		assert!(request.ends_with("3 series added"));
 	}

 	#[tokio::test]
 	async fn posts_attachment_as_body_with_filename_header() {
 		let server = MockServer::spawn(1);
 		let notification = Notification::new(NotificationKind::Test, "Here", "your book")
 			.with_attachment(Attachment::bytes(
 				"book.cbz",
 				"application/zip",
 				b"zip-bytes".to_vec(),
 			));

 		NtfyChannel::new()
 			.send(&recipient(&server.url, "files", None), &notification)
 			.await
 			.expect("delivery");

 		let request = server.request();
 		assert!(request.starts_with("POST /files HTTP/1.1"));
 		assert!(request.contains("X-Filename: book.cbz"));
 		assert!(request.contains("Content-Type: application/zip"));
 		assert!(request.contains("X-Message: your book"));
 		assert!(request.ends_with("zip-bytes"));
 	}

 	#[tokio::test]
 	async fn client_errors_are_not_retryable() {
 		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
 		let url = format!("http://{}", listener.local_addr().unwrap());
 		std::thread::spawn(move || {
 			let (mut stream, _) = listener.accept().unwrap();
 			let mut buffer = vec![0u8; 512];
 			let _ = stream.read(&mut buffer);
 			let _ = stream
 				.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
 		});

 		let error = NtfyChannel::new()
 			.send(
 				&recipient(&url, "private", None),
 				&Notification::new(NotificationKind::Test, "t", "b"),
 			)
 			.await
 			.unwrap_err();
 		assert!(matches!(error, ChannelError::Rejected(_)));
 		assert!(!error.is_retryable());
 	}
 }
