//! The wire frames of the worker socket.
//!
//! One WebSocket, text frames, one JSON object per frame, discriminated by a
//! `"type"` field. There are five frames from the worker and two from the
//! server, and that is the whole protocol:
//!
//! ```text
//! worker → server   hello { capabilities, name?, version? }
//!                   claim { job_id }
//!                   progress { job_id, fraction, message? }
//!                   result { job_id, output }
//!                   fail { job_id, error }
//!
//! server → worker   job { id, kind, input, priority }
//!                   cancel { job_id }
//! ```
//!
//! There is no acknowledgement frame and no heartbeat frame. An offer is a
//! `job`, an acceptance is the `claim` that answers it, and liveness is the
//! WebSocket's own ping/pong — adding a third layer of the same thing would
//! only give the two ends a way to disagree about who is alive.
//!
//! Reconnect is not a frame either: the server re-sends a `job` for every row
//! still assigned to the worker that said `hello`, so a worker's resume path
//! and its cold-start path are one code path.
//!
//! Frames are `#[serde(deny_unknown_fields)]`-free on purpose: a newer worker
//! may send a field an older server does not know, and dropping the connection
//! over it would make every protocol addition a flag day.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A frame sent by a worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerFrame {
	/// The first frame on every connection: what this worker can do.
	///
	/// A connection that sends anything else first is refused, so the hub never
	/// has to reason about a worker whose capabilities it does not know.
	Hello {
		capabilities: Value,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		name: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		version: Option<String>,
	},
	/// Accepts an offered job. A claim for a job offered to somebody else, or
	/// for a job that has already finished, is ignored.
	Claim { job_id: String },
	/// Progress on a claimed job. `fraction` is clamped to `0.0..=1.0`.
	Progress {
		job_id: String,
		fraction: f64,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		message: Option<String>,
	},
	/// The job produced `output`. Any bytes it produced were uploaded first.
	Result { job_id: String, output: Value },
	/// The job could not be produced. The worker stays connected and available.
	Fail { job_id: String, error: String },
}

/// A frame sent by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
	/// An offer. The worker answers with `claim` (or says nothing, and the
	/// offer is withdrawn when the connection drops).
	Job {
		id: String,
		kind: String,
		input: Value,
		priority: i32,
	},
	/// Stop working on `job_id`; the row is already terminal server-side.
	Cancel { job_id: String },
}

impl WorkerFrame {
	/// The job this frame is about, when it is about one.
	#[must_use]
	pub fn job_id(&self) -> Option<&str> {
		match self {
			Self::Hello { .. } => None,
			Self::Claim { job_id }
			| Self::Progress { job_id, .. }
			| Self::Result { job_id, .. }
			| Self::Fail { job_id, .. } => Some(job_id),
		}
	}
}

/// Parse a text frame from a worker.
pub fn parse_worker_frame(text: &str) -> Result<WorkerFrame, serde_json::Error> {
	serde_json::from_str(text)
}

/// Parse a text frame from the server.
pub fn parse_server_frame(text: &str) -> Result<ServerFrame, serde_json::Error> {
	serde_json::from_str(text)
}

/// Render a frame as the text a socket carries. Serialising these types cannot
/// fail (no maps with non-string keys, no non-finite floats after clamping), so
/// the fallible form would only push an `unwrap` onto every caller.
#[must_use]
pub fn encode<T: Serialize>(frame: &T) -> String {
	serde_json::to_string(frame).unwrap_or_else(|error| {
		tracing::error!(?error, "Failed to encode a worker protocol frame");
		String::from("{}")
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	/// The tag values are the protocol. A rename here silently breaks every
	/// deployed worker, so they are asserted as literal text rather than
	/// round-tripped through the same enum that would rename with them.
	#[test]
	fn frame_tags_are_the_documented_names() {
		let hello = encode(&WorkerFrame::Hello {
			capabilities: json!({ "transcode": { "ffmpeg": "7.1" } }),
			name: Some("gpu-box".into()),
			version: None,
		});
		assert_eq!(
			hello,
			r#"{"type":"hello","capabilities":{"transcode":{"ffmpeg":"7.1"}},"name":"gpu-box"}"#
		);

		assert_eq!(
			encode(&WorkerFrame::Progress {
				job_id: "j1".into(),
				fraction: 0.5,
				message: None,
			}),
			r#"{"type":"progress","job_id":"j1","fraction":0.5}"#
		);

		assert_eq!(
			encode(&ServerFrame::Job {
				id: "j1".into(),
				kind: "transcode".into(),
				input: json!({ "media_id": "m1" }),
				priority: -10,
			}),
			r#"{"type":"job","id":"j1","kind":"transcode","input":{"media_id":"m1"},"priority":-10}"#
		);

		assert_eq!(
			encode(&ServerFrame::Cancel {
				job_id: "j1".into()
			}),
			r#"{"type":"cancel","job_id":"j1"}"#
		);
	}

	/// A worker from a later release may add fields; an older server must keep
	/// talking to it rather than dropping the connection.
	#[test]
	fn unknown_fields_are_ignored() {
		let frame = parse_worker_frame(
			r#"{"type":"claim","job_id":"j1","lease_secs":30,"note":"hi"}"#,
		)
		.expect("claim parses");
		assert_eq!(
			frame,
			WorkerFrame::Claim {
				job_id: "j1".into()
			}
		);
	}

	#[test]
	fn an_unknown_frame_type_is_an_error_not_a_silent_no_op() {
		assert!(parse_worker_frame(r#"{"type":"align","job_id":"j1"}"#).is_err());
	}
}
