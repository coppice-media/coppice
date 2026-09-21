//! Clean-room CrossPoint Reader wireless transfer client.
//!
//! The client implements only the observed Calibre-wireless-compatible wire
//! contract.  It deliberately owns no remote-file management operation: an
//! upload may create a new file, but a caller can never ask this module to
//! delete or overwrite one.  Endpoint validation is kept here so every caller
//! gets the same private-IPv4 and fixed-port policy.

use std::{
	fmt,
	net::{Ipv4Addr, SocketAddr},
	path::Path,
	str::FromStr,
	sync::Arc,
	time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::random;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use thiserror::Error;
use tokio::{
	fs::File,
	io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
	net::TcpStream,
	sync::{mpsc, Mutex},
	task::JoinHandle,
	time::timeout,
};

/// CrossPoint's HTTP status endpoint.
pub const HTTP_PORT: u16 = 80;
/// CrossPoint's Calibre-wireless WebSocket endpoint.
pub const WEBSOCKET_PORT: u16 = 81;
/// Maximum binary payload in one client frame, matching the pinned plugin.
pub const MAX_FRAME_BYTES: usize = 2048;
/// Conservative application bound.  The firmware has no documented limit.
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;
/// The firmware parses the decimal START size through a signed long.
pub const MAX_UPLOAD_BYTES: u64 = i32::MAX as u64;
/// Protocol control messages are tiny; this prevents an untrusted peer from
/// allocating based on a claimed WebSocket payload length.
const MAX_CONTROL_FRAME_BYTES: u64 = 64 * 1024;
const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Errors returned by endpoint verification and upload.
#[derive(Debug, Error)]
pub enum CrossPointError {
	/// The destination is not an explicitly supplied private IPv4 literal or
	/// does not use CrossPoint's fixed ports.
	#[error("invalid CrossPoint endpoint: {0}")]
	InvalidEndpoint(String),
	/// A remote filename/path would be ambiguous or permit traversal.
	#[error("invalid CrossPoint path: {0}")]
	InvalidPath(String),
	/// A local source is larger than the configured bounded upload limit.
	#[error("upload size {size} exceeds configured maximum {maximum}")]
	SizeLimit { size: u64, maximum: u64 },
	/// The source changed while it was being read.
	#[error("source size changed while uploading (declared {declared}, sent {sent})")]
	SourceChanged { declared: u64, sent: u64 },
	/// The HTTP status endpoint could not be reached or parsed.
	#[error("CrossPoint status request failed: {0}")]
	Status(String),
	/// The status identity does not match the user-confirmed device.
	#[error("CrossPoint identity mismatch: {0}")]
	IdentityMismatch(String),
	/// The WebSocket HTTP upgrade was not a valid RFC 6455 handshake.
	#[error("CrossPoint WebSocket handshake failed: {0}")]
	Handshake(String),
	/// A peer message violated the small upload state machine.
	#[error("CrossPoint protocol error: {0}")]
	Protocol(String),
	/// The device explicitly rejected the operation.
	#[error("CrossPoint device rejected upload: {0}")]
	Remote(String),
	/// An operation exceeded its deadline.
	#[error("CrossPoint operation timed out")]
	Timeout,
	/// Local source/socket I/O failed.
	#[error(transparent)]
	Io(#[from] std::io::Error),
}

/// Hardware identity captured during explicit target verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CrossPointModel {
	X3,
	X4,
}

impl fmt::Display for CrossPointModel {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::X3 => "X3",
			Self::X4 => "X4",
		})
	}
}

impl FromStr for CrossPointModel {
	type Err = CrossPointError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value.trim().to_ascii_uppercase().as_str() {
			"X3" => Ok(Self::X3),
			"X4" => Ok(Self::X4),
			other => Err(CrossPointError::IdentityMismatch(format!(
				"unsupported model {other:?}"
			))),
		}
	}
}

/// The model/serial pair a user confirmed for a registry device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossPointIdentity {
	pub model: CrossPointModel,
	pub serial: String,
}

impl CrossPointIdentity {
	/// Construct an identity, refusing firmware's unknown serial sentinel.
	pub fn new(
		model: CrossPointModel,
		serial: impl Into<String>,
	) -> Result<Self, CrossPointError> {
		let serial = serial.into().trim().to_string();
		if serial.is_empty() || serial.eq_ignore_ascii_case("not found") {
			return Err(CrossPointError::IdentityMismatch(
				"a concrete serial is required".to_string(),
			));
		}
		Ok(Self { model, serial })
	}
}

/// A verified, fixed-port CrossPoint destination.
///
/// Construction rejects DNS names, IPv6, public/loopback/link-local/multicast
/// addresses, and non-standard ports.  The host is retained as an IPv4 value,
/// not a string, so later requests cannot be redirected through a hostname.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossPointEndpoint {
	host: Ipv4Addr,
	http_port: u16,
	ws_port: u16,
}

impl CrossPointEndpoint {
	/// Parse an explicitly supplied private IPv4 literal using ports 80/81.
	pub fn new(host: &str) -> Result<Self, CrossPointError> {
		let ip = host.parse::<Ipv4Addr>().map_err(|_| {
			CrossPointError::InvalidEndpoint(
				"host must be a dotted private IPv4 literal".to_string(),
			)
		})?;
		Self::from_ipv4(ip)
	}

	/// Validate an IPv4 address and CrossPoint's fixed HTTP/WebSocket ports.
	pub fn from_ipv4(host: Ipv4Addr) -> Result<Self, CrossPointError> {
		if !is_allowed_private_ipv4(host) {
			return Err(CrossPointError::InvalidEndpoint(format!(
				"{host} is not an allowed private IPv4 address"
			)));
		}
		Ok(Self {
			host,
			http_port: HTTP_PORT,
			ws_port: WEBSOCKET_PORT,
		})
	}

	/// The explicitly verified IPv4 literal.
	pub fn host(&self) -> Ipv4Addr {
		self.host
	}

	/// Fixed HTTP port (always 80).
	pub fn http_port(&self) -> u16 {
		self.http_port
	}

	/// Fixed WebSocket port (always 81).
	pub fn ws_port(&self) -> u16 {
		self.ws_port
	}

	pub(crate) fn http_url(&self) -> String {
		format!("http://{}:{}/api/status", self.host, self.http_port)
	}

	pub(crate) fn ws_addr(&self) -> SocketAddr {
		SocketAddr::from((self.host, self.ws_port))
	}
}

/// Parsed `/api/status` facts used for identity binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossPointStatus {
	pub version: Option<String>,
	pub ip: Option<Ipv4Addr>,
	pub mode: Option<String>,
	pub model: CrossPointModel,
	pub serial: String,
}

#[derive(Debug, Deserialize)]
struct StatusPayload {
	version: Option<String>,
	ip: Option<String>,
	mode: Option<String>,
	#[serde(alias = "deviceModel", alias = "device_model")]
	device: Option<String>,
	serial: Option<String>,
}

/// Upload settings.  Queue retries belong to `stump_core`; this object only
/// governs one bounded protocol attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferConfig {
	pub timeout: Duration,
	pub chunk_size: usize,
	pub max_upload_bytes: u64,
}

impl Default for TransferConfig {
	fn default() -> Self {
		Self {
			timeout: Duration::from_secs(30),
			chunk_size: MAX_FRAME_BYTES,
			max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
		}
	}
}

impl TransferConfig {
	/// Validate and normalize settings supplied by an operator/config row.
	pub fn validated(mut self) -> Result<Self, CrossPointError> {
		if self.timeout.is_zero() {
			return Err(CrossPointError::Protocol(
				"transfer timeout must be positive".to_string(),
			));
		}
		if self.chunk_size == 0 {
			return Err(CrossPointError::Protocol(
				"chunk size must be positive".to_string(),
			));
		}
		self.chunk_size = self.chunk_size.min(MAX_FRAME_BYTES);
		if self.max_upload_bytes == 0 || self.max_upload_bytes > MAX_UPLOAD_BYTES {
			return Err(CrossPointError::SizeLimit {
				size: self.max_upload_bytes,
				maximum: MAX_UPLOAD_BYTES,
			});
		}
		Ok(self)
	}
}

/// Result of an accepted upload.  The protocol has no remote hash/receipt, so
/// this reports only exact local bytes sent after the device replied `DONE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadReceipt {
	pub filename: String,
	pub path: String,
	pub bytes: u64,
}

/// Stateless verifier and one-at-a-time transfer client.
#[derive(Debug, Clone)]
pub struct CrossPointClient {
	config: TransferConfig,
}

impl Default for CrossPointClient {
	fn default() -> Self {
		Self::new(TransferConfig::default())
			.expect("default CrossPoint transfer config is valid")
	}
}

impl CrossPointClient {
	/// Create a client with bounded, validated settings.
	pub fn new(config: TransferConfig) -> Result<Self, CrossPointError> {
		Ok(Self {
			config: config.validated()?,
		})
	}

	pub fn config(&self) -> &TransferConfig {
		&self.config
	}

	/// Fetch `/api/status` without following redirects and verify model,
	/// serial, and (when present) the returned IP against the endpoint.
	pub async fn verify_target(
		&self,
		endpoint: &CrossPointEndpoint,
		identity: &CrossPointIdentity,
	) -> Result<CrossPointStatus, CrossPointError> {
		let status = self.probe_target(endpoint).await?;
		if status.model != identity.model {
			return Err(CrossPointError::IdentityMismatch(format!(
				"expected model {}, status reported {}",
				identity.model, status.model
			)));
		}
		if status.serial != identity.serial {
			return Err(CrossPointError::IdentityMismatch(format!(
				"expected serial {:?}, status reported {:?}",
				identity.serial, status.serial
			)));
		}
		Ok(status)
	}

	/// Probe `/api/status` and return the device facts without requiring a
	/// previously persisted identity.  Callers must bind the returned model
	/// and serial to a user-confirmed registry device before uploading.
	pub async fn probe_target(
		&self,
		endpoint: &CrossPointEndpoint,
	) -> Result<CrossPointStatus, CrossPointError> {
		let client = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.connect_timeout(self.config.timeout)
			.timeout(self.config.timeout)
			.build()
			.map_err(|error| CrossPointError::Status(error.to_string()))?;
		let response = client
			.get(endpoint.http_url())
			.header(reqwest::header::ACCEPT, "application/json")
			.send()
			.await
			.map_err(|error| CrossPointError::Status(error.to_string()))?;
		if response.status() != StatusCode::OK {
			return Err(CrossPointError::Status(format!(
				"status endpoint returned {}",
				response.status()
			)));
		}
		if response
			.content_length()
			.is_some_and(|length| length > MAX_CONTROL_FRAME_BYTES)
		{
			return Err(CrossPointError::Status(
				"status response exceeds bounded size".to_string(),
			));
		}
		let body = response
			.bytes()
			.await
			.map_err(|error| CrossPointError::Status(error.to_string()))?;
		if body.len() as u64 > MAX_CONTROL_FRAME_BYTES {
			return Err(CrossPointError::Status(
				"status response exceeds bounded size".to_string(),
			));
		}
		let payload: StatusPayload = serde_json::from_slice(&body).map_err(|error| {
			CrossPointError::Status(format!("invalid status JSON: {error}"))
		})?;
		let model = payload
			.device
			.as_deref()
			.ok_or_else(|| {
				CrossPointError::Status("status has no device model".to_string())
			})?
			.parse::<CrossPointModel>()?;
		let serial = payload
			.serial
			.as_deref()
			.map(str::trim)
			.filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("not found"))
			.ok_or_else(|| {
				CrossPointError::IdentityMismatch(
					"status has no concrete serial".to_string(),
				)
			})?
			.to_string();
		let ip = match payload.ip.as_deref() {
			Some(value) => {
				let status_ip = value.parse::<Ipv4Addr>().map_err(|_| {
					CrossPointError::Status("status returned a non-IPv4 ip".to_string())
				})?;
				if status_ip != endpoint.host {
					return Err(CrossPointError::IdentityMismatch(format!(
						"endpoint is {}, status reported {}",
						endpoint.host, status_ip
					)));
				}
				Some(status_ip)
			},
			None => None,
		};
		Ok(CrossPointStatus {
			version: payload.version,
			ip,
			mode: payload.mode,
			model,
			serial,
		})
	}

	/// Verify the target identity, then transfer one local file.
	pub async fn upload_for_identity(
		&self,
		endpoint: &CrossPointEndpoint,
		identity: &CrossPointIdentity,
		source: impl AsRef<Path>,
		filename: &str,
		target_path: &str,
	) -> Result<UploadReceipt, CrossPointError> {
		self.verify_target(endpoint, identity).await?;
		self.upload(endpoint, source, filename, target_path).await
	}

	/// Upload one file using the already-validated endpoint.
	///
	/// Callers that have a registry identity should prefer
	/// [`Self::upload_for_identity`], which rechecks `/api/status` immediately
	/// before opening the WebSocket.
	pub async fn upload(
		&self,
		endpoint: &CrossPointEndpoint,
		source: impl AsRef<Path>,
		filename: &str,
		target_path: &str,
	) -> Result<UploadReceipt, CrossPointError> {
		validate_filename(filename)?;
		let target_path = validate_target_path(target_path)?;
		let source = source.as_ref();
		let metadata = tokio::fs::metadata(source).await?;
		if !metadata.is_file() {
			return Err(CrossPointError::Io(std::io::Error::new(
				std::io::ErrorKind::InvalidInput,
				"source is not a regular file",
			)));
		}
		let declared = metadata.len();
		if declared > self.config.max_upload_bytes || declared > MAX_UPLOAD_BYTES {
			return Err(CrossPointError::SizeLimit {
				size: declared,
				maximum: self.config.max_upload_bytes.min(MAX_UPLOAD_BYTES),
			});
		}
		timeout(
			self.config.timeout,
			self.upload_inner(endpoint, source, filename, &target_path, declared),
		)
		.await
		.map_err(|_| CrossPointError::Timeout)?
	}

	async fn upload_inner(
		&self,
		endpoint: &CrossPointEndpoint,
		source: &Path,
		filename: &str,
		target_path: &str,
		declared: u64,
	) -> Result<UploadReceipt, CrossPointError> {
		let mut socket = WebSocket::connect(endpoint, self.config.timeout).await?;
		let start = format!("START:{filename}:{declared}:{target_path}");
		let result = async {
			socket.send_text(&start, self.config.timeout).await?;
			if declared == 0 {
				match socket.recv_text(self.config.timeout).await? {
					Some(text) if text == "DONE" => {
						return Ok(UploadReceipt {
							filename: filename.to_string(),
							path: target_path.to_string(),
							bytes: 0,
						});
					},
					Some(text) if text.starts_with("ERROR:") => {
						return Err(remote_error(&text));
					},
					Some(text) => {
						return Err(CrossPointError::Protocol(format!(
							"expected DONE for zero-byte upload, got {text:?}"
						)));
					},
					None => {
						return Err(CrossPointError::Protocol(
							"peer closed before zero-byte completion".to_string(),
						));
					},
				}
			}

			match socket.recv_text(self.config.timeout).await? {
				Some(text) if text == "READY" => {},
				Some(text) if text.starts_with("ERROR:") => {
					return Err(remote_error(&text))
				},
				Some(text) => {
					return Err(CrossPointError::Protocol(format!(
						"expected READY, got {text:?}"
					)))
				},
				None => {
					return Err(CrossPointError::Protocol(
						"peer closed before READY".to_string(),
					))
				},
			}

			let mut file = File::open(source).await?;
			let mut sent = 0_u64;
			let mut buffer = vec![0_u8; self.config.chunk_size];
			while sent < declared {
				let remaining = (declared - sent) as usize;
				let read_len = remaining.min(buffer.len());
				let read = file.read(&mut buffer[..read_len]).await?;
				if read == 0 {
					return Err(CrossPointError::SourceChanged { declared, sent });
				}
				socket
					.send_binary(&buffer[..read], self.config.timeout)
					.await?;
				sent += read as u64;
				// Firmware emits PROGRESS frames while data is flowing.  Drain
				// anything already decoded so a large upload cannot fill the
				// peer's receive window.  A text error is terminal immediately.
				socket.drain_available(self.config.timeout).await?;
			}
			// Detect a local source growing after the initial metadata read;
			// silently truncating that file would make the queue's source
			// snapshot lie.
			let mut extra = [0_u8; 1];
			if file.read(&mut extra).await? != 0 {
				return Err(CrossPointError::SourceChanged { declared, sent });
			}

			loop {
				match socket.recv_text(self.config.timeout).await? {
					Some(text) if text == "DONE" => {
						return Ok(UploadReceipt {
							filename: filename.to_string(),
							path: target_path.to_string(),
							bytes: sent,
						});
					},
					Some(text) if text.starts_with("ERROR:") => {
						return Err(remote_error(&text))
					},
					Some(text) if text.starts_with("PROGRESS:") => {
						validate_progress(&text, sent, declared)?;
					},
					Some(text) => {
						return Err(CrossPointError::Protocol(format!(
							"unexpected completion message {text:?}"
						)))
					},
					None => {
						return Err(CrossPointError::Protocol(
							"peer closed before DONE".to_string(),
						))
					},
				}
			}
		}
		.await;
		socket.close(self.config.timeout).await;
		result
	}
}

fn remote_error(text: &str) -> CrossPointError {
	let message = text.strip_prefix("ERROR:").unwrap_or(text).trim();
	if message.to_ascii_lowercase().contains("already exists") {
		CrossPointError::Remote(
			"file already exists (overwrite is never attempted)".to_string(),
		)
	} else {
		CrossPointError::Remote(message.to_string())
	}
}

fn validate_progress(
	text: &str,
	sent: u64,
	declared: u64,
) -> Result<(), CrossPointError> {
	let mut fields = text.split(':');
	if fields.next() != Some("PROGRESS") {
		return Err(CrossPointError::Protocol("malformed PROGRESS".to_string()));
	}
	let received = fields
		.next()
		.ok_or_else(|| CrossPointError::Protocol("malformed PROGRESS".to_string()))?
		.parse::<u64>()
		.map_err(|_| {
			CrossPointError::Protocol("malformed PROGRESS byte count".to_string())
		})?;
	let total = fields
		.next()
		.ok_or_else(|| CrossPointError::Protocol("malformed PROGRESS".to_string()))?
		.parse::<u64>()
		.map_err(|_| CrossPointError::Protocol("malformed PROGRESS total".to_string()))?;
	if fields.next().is_some() || total != declared || received > sent || received > total
	{
		return Err(CrossPointError::Protocol(
			"inconsistent PROGRESS values".to_string(),
		));
	}
	Ok(())
}

fn validate_filename(filename: &str) -> Result<(), CrossPointError> {
	validate_component(filename, "filename")
}

fn validate_component(component: &str, label: &str) -> Result<(), CrossPointError> {
	if component.is_empty() || component == "." || component == ".." {
		return Err(CrossPointError::InvalidPath(format!(
			"{label} must be one non-empty component"
		)));
	}
	if component.contains('/') || component.contains('\\') || component.contains(':') {
		return Err(CrossPointError::InvalidPath(format!(
			"{label} cannot contain separators or ':'"
		)));
	}
	if component.chars().any(|character| character.is_control()) {
		return Err(CrossPointError::InvalidPath(format!(
			"{label} cannot contain control characters"
		)));
	}
	Ok(())
}

fn validate_target_path(path: &str) -> Result<String, CrossPointError> {
	if path.is_empty() {
		return Ok("/".to_string());
	}
	if !path.starts_with('/') || path.contains('\\') {
		return Err(CrossPointError::InvalidPath(
			"target path must be absolute and use '/' separators".to_string(),
		));
	}
	let mut normalized = Vec::new();
	for component in path.split('/').skip(1) {
		if component.is_empty() {
			continue;
		}
		validate_component(component, "target path component")?;
		normalized.push(component);
	}
	if normalized.is_empty() {
		Ok("/".to_string())
	} else {
		Ok(format!("/{}", normalized.join("/")))
	}
}

/// RFC1918 only.  `Ipv4Addr::is_private` is intentionally not the sole check:
/// keeping the accepted ranges explicit prevents future std changes from
/// widening this LAN-only policy.
fn is_allowed_private_ipv4(ip: Ipv4Addr) -> bool {
	let octets = ip.octets();
	matches!(
		octets,
		[10, _, _, _] | [172, 16..=31, _, _] | [192, 168, _, _]
	)
}

struct WebSocket {
	writer: Arc<Mutex<WriteHalf<TcpStream>>>,
	inbound: mpsc::Receiver<Result<ServerFrame, CrossPointError>>,
	reader_task: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum ServerFrame {
	Text(String),
	Binary,
	Ping(Vec<u8>),
	Pong,
	Close(Vec<u8>),
}

impl WebSocket {
	async fn connect(
		endpoint: &CrossPointEndpoint,
		io_timeout: Duration,
	) -> Result<Self, CrossPointError> {
		let mut stream = timeout(io_timeout, TcpStream::connect(endpoint.ws_addr()))
			.await
			.map_err(|_| CrossPointError::Timeout)??;
		stream.set_nodelay(true)?;
		let key_bytes = random::<[u8; 16]>();
		let key = BASE64.encode(key_bytes);
		let request = format!(
            "GET / HTTP/1.1\r\nHost: {}:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            endpoint.host(),
            endpoint.ws_port(),
        );
		timeout(io_timeout, stream.write_all(request.as_bytes()))
			.await
			.map_err(|_| CrossPointError::Timeout)??;
		let response = read_http_upgrade(&mut stream, io_timeout).await?;
		validate_upgrade(&response, &key)?;

		let (reader, writer) = tokio::io::split(stream);
		let writer = Arc::new(Mutex::new(writer));
		let (tx, rx) = mpsc::channel(128);
		let reader_task = tokio::spawn(read_server_frames(reader, tx, io_timeout));
		Ok(Self {
			writer,
			inbound: rx,
			reader_task: Some(reader_task),
		})
	}

	async fn send_text(
		&self,
		text: &str,
		io_timeout: Duration,
	) -> Result<(), CrossPointError> {
		self.send_frame(0x1, text.as_bytes(), io_timeout).await
	}

	async fn send_binary(
		&self,
		payload: &[u8],
		io_timeout: Duration,
	) -> Result<(), CrossPointError> {
		if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
			return Err(CrossPointError::Protocol(format!(
				"binary frame must be 1..={MAX_FRAME_BYTES} bytes"
			)));
		}
		self.send_frame(0x2, payload, io_timeout).await
	}

	async fn send_frame(
		&self,
		opcode: u8,
		payload: &[u8],
		io_timeout: Duration,
	) -> Result<(), CrossPointError> {
		let length = payload.len();
		let mut frame = Vec::with_capacity(length + 14);
		frame.push(0x80 | opcode);
		if length <= 125 {
			frame.push(0x80 | length as u8);
		} else if length <= u16::MAX as usize {
			frame.push(0x80 | 126);
			frame.extend_from_slice(&(length as u16).to_be_bytes());
		} else {
			frame.push(0x80 | 127);
			frame.extend_from_slice(&(length as u64).to_be_bytes());
		}
		let mask = random::<[u8; 4]>();
		frame.extend_from_slice(&mask);
		frame.extend(
			payload
				.iter()
				.enumerate()
				.map(|(index, byte)| byte ^ mask[index % 4]),
		);
		let mut writer = self.writer.lock().await;
		timeout(io_timeout, writer.write_all(&frame))
			.await
			.map_err(|_| CrossPointError::Timeout)??;
		Ok(())
	}

	async fn recv_text(
		&mut self,
		io_timeout: Duration,
	) -> Result<Option<String>, CrossPointError> {
		loop {
			let frame = timeout(io_timeout, self.inbound.recv())
				.await
				.map_err(|_| CrossPointError::Timeout)?
				.transpose()?;
			let Some(frame) = frame else { return Ok(None) };
			match frame {
				ServerFrame::Text(text) => return Ok(Some(text)),
				ServerFrame::Ping(payload) => {
					self.send_frame(0xA, &payload, io_timeout).await?;
				},
				ServerFrame::Pong => {},
				ServerFrame::Binary => {
					return Err(CrossPointError::Protocol(
						"unexpected binary frame from server".to_string(),
					))
				},
				ServerFrame::Close(payload) => {
					return Err(CrossPointError::Protocol(format!(
						"server closed WebSocket ({})",
						close_reason(&payload)
					)))
				},
			}
		}
	}

	async fn drain_available(
		&mut self,
		io_timeout: Duration,
	) -> Result<(), CrossPointError> {
		while let Ok(frame) = self.inbound.try_recv() {
			match frame? {
				ServerFrame::Ping(payload) => {
					self.send_frame(0xA, &payload, io_timeout).await?
				},
				ServerFrame::Pong => {},
				ServerFrame::Text(text) if text.starts_with("PROGRESS:") => {},
				ServerFrame::Text(text) if text.starts_with("ERROR:") => {
					return Err(remote_error(&text))
				},
				ServerFrame::Text(text) => {
					return Err(CrossPointError::Protocol(format!(
						"unexpected message during transfer {text:?}"
					)))
				},
				ServerFrame::Binary => {
					return Err(CrossPointError::Protocol(
						"unexpected binary frame during transfer".to_string(),
					))
				},
				ServerFrame::Close(payload) => {
					return Err(CrossPointError::Protocol(format!(
						"server closed WebSocket ({})",
						close_reason(&payload)
					)))
				},
			}
		}
		Ok(())
	}

	async fn close(&mut self, io_timeout: Duration) {
		let _ = self.send_frame(0x8, &[], io_timeout).await;
		if let Some(task) = self.reader_task.take() {
			task.abort();
		}
	}
}

impl Drop for WebSocket {
	fn drop(&mut self) {
		if let Some(task) = self.reader_task.take() {
			task.abort();
		}
	}
}

async fn read_server_frames(
	mut reader: ReadHalf<TcpStream>,
	sender: mpsc::Sender<Result<ServerFrame, CrossPointError>>,
	io_timeout: Duration,
) {
	loop {
		let frame = read_server_frame(&mut reader, io_timeout).await;
		let done = frame.is_err() || sender.send(frame).await.is_err();
		if done {
			break;
		}
	}
}

async fn read_server_frame(
	reader: &mut ReadHalf<TcpStream>,
	io_timeout: Duration,
) -> Result<ServerFrame, CrossPointError> {
	let mut header = [0_u8; 2];
	read_exact_timeout(reader, &mut header, io_timeout).await?;
	let fin = header[0] & 0x80 != 0;
	let reserved = header[0] & 0x70;
	let opcode = header[0] & 0x0f;
	let masked = header[1] & 0x80 != 0;
	if !fin || reserved != 0 {
		return Err(CrossPointError::Protocol(
			"fragmented or extension WebSocket frame".to_string(),
		));
	}
	if masked {
		return Err(CrossPointError::Protocol(
			"server WebSocket frames must not be masked".to_string(),
		));
	}
	let mut length = (header[1] & 0x7f) as u64;
	if length == 126 {
		let mut bytes = [0_u8; 2];
		read_exact_timeout(reader, &mut bytes, io_timeout).await?;
		length = u16::from_be_bytes(bytes) as u64;
	} else if length == 127 {
		let mut bytes = [0_u8; 8];
		read_exact_timeout(reader, &mut bytes, io_timeout).await?;
		length = u64::from_be_bytes(bytes);
		if length & (1_u64 << 63) != 0 {
			return Err(CrossPointError::Protocol(
				"invalid WebSocket length".to_string(),
			));
		}
	}
	if length > MAX_CONTROL_FRAME_BYTES {
		return Err(CrossPointError::Protocol(
			"server control frame exceeds bounded size".to_string(),
		));
	}
	if opcode >= 0x8 && (!fin || length > 125) {
		return Err(CrossPointError::Protocol(
			"invalid WebSocket control frame".to_string(),
		));
	}
	let mut payload = vec![0_u8; length as usize];
	if !payload.is_empty() {
		read_exact_timeout(reader, &mut payload, io_timeout).await?;
	}
	match opcode {
		0x1 => String::from_utf8(payload)
			.map(ServerFrame::Text)
			.map_err(|_| {
				CrossPointError::Protocol("server text is not UTF-8".to_string())
			}),
		0x2 => Ok(ServerFrame::Binary),
		0x8 => Ok(ServerFrame::Close(payload)),
		0x9 => Ok(ServerFrame::Ping(payload)),
		0xA => Ok(ServerFrame::Pong),
		_ => Err(CrossPointError::Protocol(format!(
			"unsupported WebSocket opcode {opcode:#x}"
		))),
	}
}

async fn read_exact_timeout<R: AsyncRead + Unpin>(
	reader: &mut R,
	buffer: &mut [u8],
	io_timeout: Duration,
) -> Result<(), CrossPointError> {
	timeout(io_timeout, reader.read_exact(buffer))
		.await
		.map_err(|_| CrossPointError::Timeout)??;
	Ok(())
}

async fn read_http_upgrade(
	stream: &mut TcpStream,
	io_timeout: Duration,
) -> Result<Vec<u8>, CrossPointError> {
	let mut response = Vec::with_capacity(1024);
	let mut buffer = [0_u8; 1024];
	loop {
		let read = timeout(io_timeout, stream.read(&mut buffer))
			.await
			.map_err(|_| CrossPointError::Timeout)??;
		if read == 0 {
			return Err(CrossPointError::Handshake(
				"peer closed during HTTP upgrade".to_string(),
			));
		}
		response.extend_from_slice(&buffer[..read]);
		if response.windows(4).any(|window| window == b"\r\n\r\n") {
			return Ok(response);
		}
		if response.len() > 16 * 1024 {
			return Err(CrossPointError::Handshake(
				"HTTP upgrade headers exceed bounded size".to_string(),
			));
		}
	}
}

fn validate_upgrade(response: &[u8], key: &str) -> Result<(), CrossPointError> {
	let header_end = response
		.windows(4)
		.position(|window| window == b"\r\n\r\n")
		.ok_or_else(|| {
			CrossPointError::Handshake("missing HTTP header terminator".to_string())
		})?;
	let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| {
		CrossPointError::Handshake("upgrade headers are not UTF-8".to_string())
	})?;
	let mut lines = headers.split("\r\n");
	let status = lines.next().ok_or_else(|| {
		CrossPointError::Handshake("missing HTTP status line".to_string())
	})?;
	if !status.starts_with("HTTP/1.1 101 ") {
		return Err(CrossPointError::Handshake(format!(
			"expected HTTP/1.1 101, got {status:?}"
		)));
	}
	let mut upgrade = false;
	let mut connection = false;
	let mut accept = None;
	for line in lines {
		let Some((name, value)) = line.split_once(':') else {
			return Err(CrossPointError::Handshake(
				"malformed HTTP header".to_string(),
			));
		};
		match name.trim().to_ascii_lowercase().as_str() {
			"upgrade" => upgrade = value.trim().eq_ignore_ascii_case("websocket"),
			"connection" => {
				connection = value
					.split(',')
					.any(|token| token.trim().eq_ignore_ascii_case("upgrade"));
			},
			"sec-websocket-accept" => accept = Some(value.trim()),
			_ => {},
		}
	}
	if !upgrade || !connection {
		return Err(CrossPointError::Handshake(
			"missing Upgrade/Connection headers".to_string(),
		));
	}
	let mut digest = Sha1::new();
	digest.update(key.as_bytes());
	digest.update(WS_GUID);
	let expected = BASE64.encode(digest.finalize());
	if accept != Some(expected.as_str()) {
		return Err(CrossPointError::Handshake(
			"Sec-WebSocket-Accept does not match request key".to_string(),
		));
	}
	Ok(())
}

fn close_reason(payload: &[u8]) -> String {
	if payload.len() < 2 {
		return "no reason".to_string();
	}
	String::from_utf8_lossy(&payload[2..]).into_owned()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_private_ipv4_literals_are_allowed() {
		for value in ["10.0.0.1", "172.16.4.5", "172.31.255.254", "192.168.1.20"] {
			assert!(CrossPointEndpoint::new(value).is_ok(), "{value}");
		}
		for value in [
			"127.0.0.1",
			"169.254.1.2",
			"224.0.0.1",
			"0.0.0.0",
			"8.8.8.8",
			"172.32.0.1",
			"192.0.2.1",
			"crosspoint.local",
			"::1",
		] {
			assert!(CrossPointEndpoint::new(value).is_err(), "{value}");
		}
	}

	#[test]
	fn paths_normalize_only_safe_components() {
		assert_eq!(validate_target_path("/").unwrap(), "/");
		assert_eq!(
			validate_target_path("//Books///Fiction/").unwrap(),
			"/Books/Fiction"
		);
		for path in [
			"Books",
			"/../Books",
			"/Books/../Secrets",
			"/Books/./Fiction",
			"/Books/a:b",
		] {
			assert!(validate_target_path(path).is_err(), "{path}");
		}
		for name in ["book.epub", "chapter 1.epub"] {
			assert!(validate_filename(name).is_ok(), "{name}");
		}
		for name in ["", ".", "..", "a/b", "a\\b", "a:b", "a\n"] {
			assert!(validate_filename(name).is_err(), "{name:?}");
		}
	}

	#[test]
	fn config_caps_frame_and_rejects_i32_overflow() {
		let config = TransferConfig {
			chunk_size: 65_536,
			..Default::default()
		}
		.validated()
		.unwrap();
		assert_eq!(config.chunk_size, MAX_FRAME_BYTES);
		assert!(TransferConfig {
			max_upload_bytes: MAX_UPLOAD_BYTES + 1,
			..Default::default()
		}
		.validated()
		.is_err());
	}

	#[test]
	fn websocket_accept_validation_is_strict() {
		let key = "dGhlIHNhbXBsZSBub25jZQ==";
		let mut digest = Sha1::new();
		digest.update(key.as_bytes());
		digest.update(WS_GUID);
		let accept = BASE64.encode(digest.finalize());
		let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        );
		assert!(validate_upgrade(response.as_bytes(), key).is_ok());
		assert!(
			validate_upgrade(response.replace(&accept, "wrong").as_bytes(), key).is_err()
		);
	}
}
