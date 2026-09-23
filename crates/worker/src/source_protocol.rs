//! Shared source-worker control protocol.
//!
//! Source workers use a separate JSON protocol from compute workers.  The
//! control socket carries only these frames; media bytes use the independent
//! tunnel data socket owned by [`crate::source_hub`].

use std::{
	collections::HashSet,
	time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// The maximum number of observations in one manifest frame.
pub const MAX_SOURCE_MANIFEST_ITEMS: usize = 256;
/// The maximum encoded size of one manifest frame (one MiB).
pub const MAX_SOURCE_MANIFEST_FRAME_BYTES: usize = 1024 * 1024;
/// Compatibility aliases used by callers that do not include `SOURCE` in the
/// constant name.
pub const MAX_MANIFEST_ITEMS: usize = MAX_SOURCE_MANIFEST_ITEMS;
pub const MAX_MANIFEST_FRAME_BYTES: usize = MAX_SOURCE_MANIFEST_FRAME_BYTES;

/// A source worker's byte transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTransport {
	Direct,
	Tunnel,
}

/// Whether a read covers the entire object or one exact range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceReadMode {
	Full,
	Range,
}

/// A source worker's hello payload, kept separate so the server can pass the
/// decoded hello to [`crate::SourceHub::attach`] without reconstructing a
/// protocol frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceWorkerHello {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub version: Option<String>,
	#[serde(default)]
	pub roots: Vec<SourceRootHello>,
}

impl SourceWorkerHello {
	/// Validate bounded worker and root metadata before it reaches persistence.
	pub fn validate(&self) -> Result<(), SourceProtocolError> {
		if self.name.as_ref().is_some_and(|name| name.len() > 256)
			|| self
				.version
				.as_ref()
				.is_some_and(|version| version.len() > 128)
			|| self.roots.len() > 128
		{
			return Err(SourceProtocolError::InvalidHello(
				"source hello metadata exceeds protocol limits",
			));
		}
		let mut root_ids = HashSet::with_capacity(self.roots.len());
		for root in &self.roots {
			root.validate()?;
			if !root_ids.insert(root.root_id.as_str()) {
				return Err(SourceProtocolError::InvalidHello(
					"source hello contains a duplicate root id",
				));
			}
		}
		Ok(())
	}
}

/// A root advertised by a source worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRootHello {
	pub root_id: String,
	pub label: String,
	pub kind: String,
	pub privacy_mode: String,
	pub transport: SourceTransport,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub direct_base_url: Option<String>,
}

impl SourceRootHello {
	pub fn validate(&self) -> Result<(), SourceProtocolError> {
		let valid_id = !self.root_id.is_empty()
			&& self.root_id.len() <= 128
			&& self.root_id.bytes().all(|byte| {
				byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
			});
		if !valid_id
			|| self.label.trim().is_empty()
			|| self.label.len() > 256
			|| self.kind.trim().is_empty()
			|| self.kind.len() > 64
			|| self.privacy_mode != "catalog"
		{
			return Err(SourceProtocolError::InvalidHello(
				"source root metadata is invalid",
			));
		}
		match self.transport {
			SourceTransport::Direct
				if self
					.direct_base_url
					.as_ref()
					.is_none_or(|url| url.is_empty() || url.len() > 2048) =>
			{
				Err(SourceProtocolError::InvalidHello(
					"direct source root requires a bounded base URL",
				))
			},
			SourceTransport::Tunnel if self.direct_base_url.is_some() => {
				Err(SourceProtocolError::InvalidHello(
					"tunnel source root cannot advertise a direct base URL",
				))
			},
			_ => Ok(()),
		}
	}
}

/// One privacy-minimized source observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceManifestItem {
	pub worker_item_id: String,
	pub worker_content_version: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub relative_path: Option<String>,
	pub size: u64,
	/// Either an RFC3339 string or an epoch number.  The worker does not need
	/// to interpret this value and preserving the JSON value keeps both wire
	/// representations lossless.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub modified_at: Option<Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub media_type: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub quick_fingerprint: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub sha256: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub metadata: Option<Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub retention: Option<Value>,
}

/// A bounded page of observations for one root/revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceManifestChunk {
	pub batch_id: String,
	pub root_id: String,
	pub revision: u64,
	pub sequence: u64,
	pub terminal: bool,
	#[serde(default)]
	pub items: Vec<SourceManifestItem>,
}

impl SourceManifestChunk {
	/// Return the size of this chunk as a complete source JSON frame.
	#[must_use]
	pub fn encoded_len(&self) -> usize {
		serde_json::to_vec(&SourceWorkerFrame::ManifestChunk(self.clone()))
			.map(|encoded| encoded.len())
			.unwrap_or(usize::MAX)
	}

	/// Validate the protocol's hard per-frame manifest limits.
	pub fn validate(&self) -> Result<(), SourceProtocolError> {
		if self.batch_id.is_empty()
			|| self.batch_id.len() > 256
			|| self.root_id.is_empty()
			|| self.root_id.len() > 128
		{
			return Err(SourceProtocolError::InvalidManifest(
				"manifest identity is missing or too long",
			));
		}
		for item in &self.items {
			if item.worker_item_id.is_empty()
				|| item.worker_item_id.len() > 256
				|| item.worker_content_version.is_empty()
				|| item.worker_content_version.len() > 512
				|| item
					.relative_path
					.as_ref()
					.is_some_and(|path| path.len() > 2048)
			{
				return Err(SourceProtocolError::InvalidManifest(
					"manifest item identity is missing or too long",
				));
			}
		}
		if self.items.len() > MAX_SOURCE_MANIFEST_ITEMS {
			return Err(SourceProtocolError::ManifestItemLimit {
				actual: self.items.len(),
				max: MAX_SOURCE_MANIFEST_ITEMS,
			});
		}
		let encoded = self.encoded_len();
		if encoded > MAX_SOURCE_MANIFEST_FRAME_BYTES {
			return Err(SourceProtocolError::ManifestFrameLimit {
				actual: encoded,
				max: MAX_SOURCE_MANIFEST_FRAME_BYTES,
			});
		}
		Ok(())
	}
}

/// A request to issue a one-use source read grant.  The grant id itself is
/// generated by the server hub and is intentionally absent here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReadRequest {
	pub root_id: String,
	pub worker_item_id: String,
	pub worker_content_version: String,
	/// Server-verified whole-object identity. Reads linked to a verified media
	/// location require the worker to recheck this digest before serving.
	pub expected_sha256: Option<String>,
	pub mode: SourceReadMode,
	pub offset: u64,
	pub length: u64,
	pub transport: SourceTransport,
	/// Unix seconds or Unix milliseconds.  The hub accepts both forms so a
	/// caller can use either `timestamp()` or `timestamp_millis()` without a
	/// lossy conversion at the protocol boundary.
	pub expires_at: i64,
	pub max_bytes: u64,
}

impl SourceReadRequest {
	/// Validate range/byte-budget invariants before a grant is sent.
	pub fn validate(&self) -> Result<(), SourceProtocolError> {
		if self.root_id.is_empty()
			|| self.worker_item_id.is_empty()
			|| self.worker_content_version.is_empty()
		{
			return Err(SourceProtocolError::GrantFieldMissing);
		}
		if self
			.expected_sha256
			.as_deref()
			.is_some_and(|digest| !is_sha256(digest))
		{
			return Err(SourceProtocolError::InvalidExpectedDigest);
		}
		if self.length > self.max_bytes || self.offset.checked_add(self.length).is_none()
		{
			return Err(SourceProtocolError::GrantLengthExceedsMaximum {
				length: self.length,
				max_bytes: self.max_bytes,
			});
		}
		Ok(())
	}
}

/// A server-issued, exact, one-use source read authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReadGrant {
	pub grant_id: String,
	pub root_id: String,
	pub worker_item_id: String,
	pub worker_content_version: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expected_sha256: Option<String>,
	pub mode: SourceReadMode,
	pub offset: u64,
	pub length: u64,
	pub transport: SourceTransport,
	pub expires_at: i64,
	pub max_bytes: u64,
}

impl SourceReadGrant {
	/// Validate grant byte-budget invariants.
	pub fn validate(&self) -> Result<(), SourceProtocolError> {
		if self.grant_id.is_empty()
			|| self.root_id.is_empty()
			|| self.worker_item_id.is_empty()
			|| self.worker_content_version.is_empty()
		{
			return Err(SourceProtocolError::GrantFieldMissing);
		}
		if self
			.expected_sha256
			.as_deref()
			.is_some_and(|digest| !is_sha256(digest))
		{
			return Err(SourceProtocolError::InvalidExpectedDigest);
		}
		if self.length > self.max_bytes || self.offset.checked_add(self.length).is_none()
		{
			return Err(SourceProtocolError::GrantLengthExceedsMaximum {
				length: self.length,
				max_bytes: self.max_bytes,
			});
		}
		Ok(())
	}

	/// Whether this grant is expired at `now` (or now, when omitted).
	#[must_use]
	pub fn is_expired_at(&self, now: SystemTime) -> bool {
		let now_ms = now
			.duration_since(UNIX_EPOCH)
			.map(|duration| duration.as_millis() as i128)
			.unwrap_or(i128::MIN);
		expires_at_millis(self.expires_at) <= now_ms
	}

	/// Whether this grant is expired according to the local wall clock.
	#[must_use]
	pub fn is_expired(&self) -> bool {
		self.is_expired_at(SystemTime::now())
	}
}

impl From<SourceReadGrant> for SourceReadRequest {
	fn from(grant: SourceReadGrant) -> Self {
		Self {
			root_id: grant.root_id,
			worker_item_id: grant.worker_item_id,
			worker_content_version: grant.worker_content_version,
			expected_sha256: grant.expected_sha256,
			mode: grant.mode,
			offset: grant.offset,
			length: grant.length,
			transport: grant.transport,
			expires_at: grant.expires_at,
			max_bytes: grant.max_bytes,
		}
	}
}

/// A frame sent by a source worker on the authenticated JSON control socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceWorkerFrame {
	Hello {
		#[serde(default, skip_serializing_if = "Option::is_none")]
		name: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		version: Option<String>,
		#[serde(default)]
		roots: Vec<SourceRootHello>,
	},
	ManifestChunk(SourceManifestChunk),
	ReadReady {
		grant_id: String,
	},
	ReadFailed {
		grant_id: String,
		error: String,
	},
}

impl SourceWorkerFrame {
	/// Extract a hello payload without changing the wire representation.
	#[must_use]
	pub fn hello(
		name: Option<String>,
		version: Option<String>,
		roots: Vec<SourceRootHello>,
	) -> Self {
		Self::Hello {
			name,
			version,
			roots,
		}
	}

	/// Convert a hello frame into the attach-friendly payload.
	pub fn into_hello(self) -> Result<SourceWorkerHello, SourceProtocolError> {
		match self {
			Self::Hello {
				name,
				version,
				roots,
			} => Ok(SourceWorkerHello {
				name,
				version,
				roots,
			}),
			_ => Err(SourceProtocolError::ExpectedHello),
		}
	}

	/// Validate limits carried by this frame.
	pub fn validate(&self) -> Result<(), SourceProtocolError> {
		match self {
			Self::Hello {
				name,
				version,
				roots,
			} => SourceWorkerHello {
				name: name.clone(),
				version: version.clone(),
				roots: roots.clone(),
			}
			.validate(),
			Self::ManifestChunk(chunk) => chunk.validate(),
			Self::ReadReady { grant_id } | Self::ReadFailed { grant_id, .. }
				if grant_id.is_empty() || grant_id.len() > 256 =>
			{
				Err(SourceProtocolError::GrantFieldMissing)
			},
			_ => Ok(()),
		}
	}
}

/// A frame sent by the server on the authenticated JSON control socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceServerFrame {
	ManifestAck {
		root_id: String,
		revision: u64,
		sequence: u64,
	},
	Read(SourceReadGrant),
}

/// Errors raised before a source frame is handed to application code.
#[derive(Debug, Error)]
pub enum SourceProtocolError {
	#[error("manifest contains {actual} items; maximum is {max}")]
	ManifestItemLimit { actual: usize, max: usize },
	#[error("manifest frame is {actual} bytes; maximum is {max}")]
	ManifestFrameLimit { actual: usize, max: usize },
	#[error("source hello is invalid: {0}")]
	InvalidHello(&'static str),
	#[error("source manifest is invalid: {0}")]
	InvalidManifest(&'static str),
	#[error("grant length {length} exceeds byte budget {max_bytes}")]
	GrantLengthExceedsMaximum { length: u64, max_bytes: u64 },
	#[error("grant has an empty required field")]
	GrantFieldMissing,
	#[error("expected source digest must be 64 lowercase hexadecimal characters")]
	InvalidExpectedDigest,
	#[error("expected source worker hello frame")]
	ExpectedHello,
	#[error("invalid source JSON: {0}")]
	Json(#[from] serde_json::Error),
}

fn is_sha256(value: &str) -> bool {
	value.len() == 64
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Parse and validate one worker control frame.
pub fn parse_source_worker_frame(
	text: &str,
) -> Result<SourceWorkerFrame, SourceProtocolError> {
	let frame: SourceWorkerFrame = serde_json::from_str(text)?;
	frame.validate()?;
	Ok(frame)
}

/// Parse and validate one server control frame.
pub fn parse_source_server_frame(
	text: &str,
) -> Result<SourceServerFrame, SourceProtocolError> {
	let frame: SourceServerFrame = serde_json::from_str(text)?;
	if let SourceServerFrame::Read(grant) = &frame {
		grant.validate()?;
	}
	Ok(frame)
}

/// Render a source control frame as one JSON text message.
#[must_use]
pub fn encode_source_frame<T: Serialize>(frame: &T) -> String {
	serde_json::to_string(frame).unwrap_or_else(|error| {
		tracing::error!(?error, "Failed to encode a source worker protocol frame");
		String::from("{}")
	})
}

/// Convert either accepted expiry representation into epoch milliseconds.
#[must_use]
pub fn expires_at_millis(expires_at: i64) -> i128 {
	// Current Unix milliseconds are thirteen digits; seconds are ten digits.
	// Treat values with an absolute magnitude below 10^11 as seconds.  This
	// also makes zero/negative values unambiguously expired.
	if expires_at.unsigned_abs() < 100_000_000_000 {
		expires_at as i128 * 1_000
	} else {
		expires_at as i128
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn item(id: &str) -> SourceManifestItem {
		SourceManifestItem {
			worker_item_id: id.to_owned(),
			worker_content_version: "v1".to_owned(),
			relative_path: None,
			size: 4,
			modified_at: None,
			media_type: Some("text/plain".to_owned()),
			quick_fingerprint: None,
			sha256: None,
			metadata: None,
			retention: None,
		}
	}

	#[test]
	fn source_frames_are_separate_from_compute_frames() {
		let frame = SourceWorkerFrame::ManifestChunk(SourceManifestChunk {
			batch_id: "b".to_owned(),
			root_id: "r".to_owned(),
			revision: 3,
			sequence: 0,
			terminal: true,
			items: vec![item("i")],
		});
		let encoded = encode_source_frame(&frame);
		assert!(encoded.contains(r#""type":"manifest_chunk""#));
		assert_eq!(parse_source_worker_frame(&encoded).unwrap(), frame);
	}

	#[test]
	fn manifest_item_and_encoded_limits_are_enforced() {
		let mut chunk = SourceManifestChunk {
			batch_id: "b".to_owned(),
			root_id: "r".to_owned(),
			revision: 1,
			sequence: 0,
			terminal: false,
			items: (0..=MAX_SOURCE_MANIFEST_ITEMS)
				.map(|index| item(&index.to_string()))
				.collect(),
		};
		assert!(matches!(
			chunk.validate(),
			Err(SourceProtocolError::ManifestItemLimit { .. })
		));

		chunk.items = vec![SourceManifestItem {
			metadata: Some(json!({"large": "x".repeat(MAX_SOURCE_MANIFEST_FRAME_BYTES)})),
			..item("large")
		}];
		assert!(matches!(
			chunk.validate(),
			Err(SourceProtocolError::ManifestFrameLimit { .. })
		));
	}

	#[test]
	fn expiry_accepts_seconds_and_milliseconds() {
		let now_seconds = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs() as i64;
		let grant = SourceReadGrant {
			grant_id: "g".into(),
			root_id: "r".into(),
			worker_item_id: "i".into(),
			worker_content_version: "v".into(),
			expected_sha256: None,
			mode: SourceReadMode::Full,
			offset: 0,
			length: 1,
			transport: SourceTransport::Tunnel,
			expires_at: now_seconds + 60,
			max_bytes: 1,
		};
		assert!(!grant.is_expired());
		let mut invalid = grant;
		invalid.expected_sha256 = Some("ABC".into());
		assert!(matches!(
			invalid.validate(),
			Err(SourceProtocolError::InvalidExpectedDigest)
		));
		assert!(expires_at_millis(now_seconds) < expires_at_millis(now_seconds + 60));
	}
}
