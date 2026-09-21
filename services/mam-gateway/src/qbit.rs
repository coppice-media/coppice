use crate::{
	config::{GatewayConfig, Secret},
	error::{GatewayError, GatewayResult},
	http_util::{map_request_error, read_bounded},
};
use bytes::Bytes;
use reqwest::{header, Client, StatusCode};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

#[derive(Clone, Debug)]
pub struct QbitSnapshot {
	pub progress: f64,
	pub completed: bool,
	pub failed: bool,
	pub(crate) content_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitResult {
	Submitted,
	AlreadyPresent,
}

#[derive(Clone)]
pub struct QbitClient {
	http: Client,
	base_url: Url,
	username: Option<Secret>,
	password: Option<Secret>,
	sid: Arc<Mutex<Option<Secret>>>,
	timeout_body_bytes: usize,
}

impl QbitClient {
	pub fn new(config: Arc<GatewayConfig>) -> GatewayResult<Self> {
		let http = Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.no_proxy()
			.timeout(config.request_timeout)
			.build()
			.map_err(|_| GatewayError::Config)?;
		Ok(Self {
			http,
			base_url: config.qbit_base_url.clone(),
			username: config.qbit_username.clone(),
			password: config.qbit_password.clone(),
			sid: Arc::new(Mutex::new(None)),
			timeout_body_bytes: config.max_qbit_body_bytes,
		})
	}

	/// Submit one gateway-fetched torrent artifact. A tag preflight makes
	/// retries idempotent even when the first qBittorrent response is lost.
	pub async fn submit(&self, torrent: Bytes, tag: &str) -> GatewayResult<SubmitResult> {
		if let Some(_) = self.find_by_tag(tag).await? {
			return Ok(SubmitResult::AlreadyPresent);
		}
		let mut relogin = false;
		loop {
			let sid = self.ensure_session().await?;
			let part =
				reqwest::multipart::Part::stream(reqwest::Body::from(torrent.clone()))
					.file_name("coppice.torrent")
					.mime_str("application/x-bittorrent")
					.map_err(|_| GatewayError::Internal)?;
			let form = reqwest::multipart::Form::new()
				.part("torrents", part)
				.text("tags", tag.to_owned())
				.text("paused", "false");
			let mut request = self
				.http
				.post(self.endpoint("/api/v2/torrents/add"))
				.multipart(form);
			if let Some(sid) = sid {
				request = request.header(header::COOKIE, format!("SID={}", sid.expose()));
			}
			let response = request
				.send()
				.await
				.map_err(|error| map_request_error(&error))?;
			let status = response.status();
			if status == StatusCode::FORBIDDEN && !relogin && self.has_credentials() {
				self.clear_session().await;
				relogin = true;
				continue;
			}
			if status.is_redirection() {
				return Err(GatewayError::RedirectRejected);
			}
			if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
				return Err(GatewayError::DownloadClientRejected);
			}
			if !status.is_success() {
				return Err(GatewayError::DownloadClientRejected);
			}
			let body = read_bounded(response, self.timeout_body_bytes).await?;
			let text =
				std::str::from_utf8(&body).map_err(|_| GatewayError::UpstreamInvalid)?;
			if text.trim() == "Ok." {
				return Ok(SubmitResult::Submitted);
			}
			return Err(GatewayError::DownloadClientRejected);
		}
	}

	pub async fn find_by_tag(&self, tag: &str) -> GatewayResult<Option<QbitSnapshot>> {
		let mut relogin = false;
		loop {
			let sid = self.ensure_session().await?;
			let mut request = self
				.http
				.get(self.endpoint("/api/v2/torrents/info"))
				.query(&[("tag", tag)]);
			if let Some(sid) = sid {
				request = request.header(header::COOKIE, format!("SID={}", sid.expose()));
			}
			let response = request
				.send()
				.await
				.map_err(|error| map_request_error(&error))?;
			let status = response.status();
			if status == StatusCode::FORBIDDEN && !relogin && self.has_credentials() {
				self.clear_session().await;
				relogin = true;
				continue;
			}
			if status.is_redirection() {
				return Err(GatewayError::RedirectRejected);
			}
			if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
				return Err(GatewayError::DownloadClientRejected);
			}
			if !status.is_success() {
				return Err(GatewayError::DownloadClientRejected);
			}
			let body = read_bounded(response, self.timeout_body_bytes).await?;
			let torrents: Vec<TorrentInfo> = serde_json::from_slice(&body)
				.map_err(|_| GatewayError::UpstreamInvalid)?;
			return Ok(torrents.first().map(TorrentInfo::snapshot));
		}
	}

	async fn ensure_session(&self) -> GatewayResult<Option<Secret>> {
		if !self.has_credentials() {
			return Ok(None);
		}
		if let Some(sid) = self.sid.lock().await.clone() {
			return Ok(Some(sid));
		}
		self.login().await
	}

	async fn login(&self) -> GatewayResult<Option<Secret>> {
		self.login_with_cookie().await
	}

	async fn login_with_cookie(&self) -> GatewayResult<Option<Secret>> {
		let username = self.username.as_ref().ok_or(GatewayError::Config)?;
		let password = self.password.as_ref().ok_or(GatewayError::Config)?;
		let form = [
			("username", username.expose()),
			("password", password.expose()),
		];
		let response = self
			.http
			.post(self.endpoint("/api/v2/auth/login"))
			.form(&form)
			.send()
			.await
			.map_err(|error| map_request_error(&error))?;
		let status = response.status();
		if status.is_redirection() {
			return Err(GatewayError::RedirectRejected);
		}
		if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
			return Err(GatewayError::DownloadClientRejected);
		}
		if !status.is_success() {
			return Err(GatewayError::DownloadClientRejected);
		}
		let sid = response
			.headers()
			.get_all(header::SET_COOKIE)
			.iter()
			.filter_map(|value| value.to_str().ok())
			.find_map(parse_sid)
			.ok_or(GatewayError::DownloadClientRejected)?;
		let body = read_bounded(response, self.timeout_body_bytes).await?;
		let text =
			std::str::from_utf8(&body).map_err(|_| GatewayError::UpstreamInvalid)?;
		if text.trim() != "Ok." {
			return Err(GatewayError::DownloadClientRejected);
		}
		let sid = Secret::new(sid)?;
		*self.sid.lock().await = Some(sid.clone());
		Ok(Some(sid))
	}

	async fn clear_session(&self) {
		*self.sid.lock().await = None;
	}

	fn has_credentials(&self) -> bool {
		self.username.is_some() && self.password.is_some()
	}

	fn endpoint(&self, path: &str) -> Url {
		self.base_url
			.join(path)
			.expect("validated qBittorrent base URL accepts fixed paths")
	}
}

#[derive(Debug, Deserialize)]
struct TorrentInfo {
	#[serde(default)]
	state: String,
	#[serde(default)]
	progress: f64,
	#[serde(default)]
	content_path: Option<String>,
}

impl TorrentInfo {
	fn snapshot(&self) -> QbitSnapshot {
		let progress = self.progress.clamp(0.0, 1.0);
		let failed = matches!(self.state.as_str(), "error" | "missingFiles" | "unknown");
		let completed = progress >= 1.0
			|| matches!(
				self.state.as_str(),
				"uploading" | "stalledUP" | "pausedUP" | "checkingUP" | "queuedUP"
			);
		QbitSnapshot {
			progress,
			completed,
			failed,
			content_path: self.content_path.clone(),
		}
	}
}

fn parse_sid(value: &str) -> Option<String> {
	let value = value.strip_prefix("SID=")?;
	let sid = value.split(';').next()?.trim();
	(!sid.is_empty()
		&& sid.len() <= 256
		&& sid.bytes().all(|byte| {
			byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
		}))
	.then_some(sid.to_owned())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn qbit_states_are_reduced_to_safe_progress() {
		let complete = TorrentInfo {
			state: "uploading".to_owned(),
			progress: 0.2,
			content_path: None,
		}
		.snapshot();
		assert!(complete.completed);
		assert_eq!(complete.progress, 0.2);

		let failed = TorrentInfo {
			state: "missingFiles".to_owned(),
			progress: 0.0,
			content_path: None,
		}
		.snapshot();
		assert!(failed.failed);
	}

	#[test]
	fn sid_parser_never_accepts_cookie_attributes_as_the_value() {
		assert_eq!(parse_sid("SID=abc123; HttpOnly"), Some("abc123".to_owned()));
		assert_eq!(parse_sid("SID=; HttpOnly"), None);
		assert_eq!(parse_sid("other=value"), None);
	}
}
