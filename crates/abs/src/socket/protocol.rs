//! Engine.IO v4 and Socket.IO v5 framing, by hand.
//!
//! The official app connects with `socket.io-client ^4.1.3` and forces
//! `transports: ['websocket'], upgrade: false` (`plugins/server.js:36-42`),
//! which reduces the two protocols to a small, closed set of text frames on
//! one websocket. They are implemented here rather than pulled in as a
//! dependency: the whole surface is the packet types below, and a socket.io
//! server crate would bring its own connection model, its own namespaces and
//! its own polling transport for a lane that needs none of them.
//!
//! References (MIT-licensed specification text, not Audiobookshelf's
//! GPL-3.0 source): `socketio/engine.io-protocol` `README.md` for the packet
//! types and the heartbeat, `socketio/socket.io-protocol` `Readme.md` for the
//! packet encoding of revision 5. Every frame below was also observed on the
//! wire against `abs-ref` 2.36.0.
//!
//! ```text
//! server → 0{"sid":…,"upgrades":[],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}
//! client → 40                      CONNECT to the default namespace
//! server → 40{"sid":…}             CONNECT acknowledged
//! client → 42["auth","<jwt>"]      EVENT
//! server → 42["init",{…}]          EVENT
//! server → 2   client → 3          heartbeat, server-initiated
//! ```

use serde_json::Value;

/// Engine.IO packet types (`engine.io-protocol` §"Packet encoding").
pub(crate) const EIO_OPEN: char = '0';
pub(crate) const EIO_CLOSE: char = '1';
pub(crate) const EIO_PING: char = '2';
pub(crate) const EIO_PONG: char = '3';
pub(crate) const EIO_MESSAGE: char = '4';

/// Socket.IO packet types, carried inside an Engine.IO `message`
/// (`socket.io-protocol` §"Exchange protocol").
pub(crate) const SIO_CONNECT: char = '0';
pub(crate) const SIO_DISCONNECT: char = '1';
pub(crate) const SIO_EVENT: char = '2';

/// What a decoded client frame asks of the server.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ClientFrame {
	/// Engine.IO `3`: the answer to our heartbeat.
	Pong,
	/// Engine.IO `2`: some clients ping too; the answer is a `3`.
	Ping,
	/// Engine.IO `1`, or Socket.IO `41`: the client is leaving.
	Close,
	/// Socket.IO `40[namespace]`: join a namespace.
	Connect { namespace: String },
	/// Socket.IO `42[…]`: an event with its name and arguments.
	Event { name: String, args: Vec<Value> },
	/// A frame this lane has no meaning for (binary attachments, acks,
	/// namespaces other than the default). Ignored, never fatal: an unknown
	/// frame must not drop a working socket.
	Ignored,
}

/// The default (and only) namespace this lane serves.
pub(crate) const DEFAULT_NAMESPACE: &str = "/";

/// Decode one text frame from the client.
pub(crate) fn decode(frame: &str) -> ClientFrame {
	let mut chars = frame.chars();
	let Some(kind) = chars.next() else {
		return ClientFrame::Ignored;
	};
	let rest = &frame[kind.len_utf8()..];
	match kind {
		EIO_PING => ClientFrame::Ping,
		EIO_PONG => ClientFrame::Pong,
		EIO_CLOSE => ClientFrame::Close,
		EIO_MESSAGE => decode_socket_io(rest),
		_ => ClientFrame::Ignored,
	}
}

fn decode_socket_io(payload: &str) -> ClientFrame {
	let mut chars = payload.chars();
	let Some(kind) = chars.next() else {
		return ClientFrame::Ignored;
	};
	let rest = &payload[kind.len_utf8()..];
	match kind {
		SIO_CONNECT => ClientFrame::Connect {
			namespace: namespace_of(rest),
		},
		SIO_DISCONNECT => ClientFrame::Close,
		SIO_EVENT => decode_event(rest),
		_ => ClientFrame::Ignored,
	}
}

/// A Socket.IO packet may name a namespace before its payload, terminated by
/// a comma (`42/admin,["x"]`). Absent, it is the default namespace.
fn namespace_of(rest: &str) -> String {
	let trimmed = rest.trim();
	if !trimmed.starts_with('/') {
		return DEFAULT_NAMESPACE.to_owned();
	}
	match trimmed.split_once(',') {
		Some((namespace, _)) => namespace.to_owned(),
		None => match trimmed.find(['[', '{']) {
			Some(index) => trimmed[..index].to_owned(),
			None => trimmed.to_owned(),
		},
	}
}

fn decode_event(rest: &str) -> ClientFrame {
	// Skip an optional namespace and an optional numeric ack id, both of
	// which sit between the packet type and the JSON array.
	let Some(start) = rest.find('[') else {
		return ClientFrame::Ignored;
	};
	if rest[..start].starts_with('/') && !rest[..start].contains(',') {
		// A namespaced event with no payload separator is malformed.
		return ClientFrame::Ignored;
	}
	let Ok(Value::Array(mut args)) = serde_json::from_str::<Value>(&rest[start..]) else {
		return ClientFrame::Ignored;
	};
	if args.is_empty() {
		return ClientFrame::Ignored;
	}
	let name = match args.remove(0) {
		Value::String(name) => name,
		_ => return ClientFrame::Ignored,
	};
	ClientFrame::Event { name, args }
}

/// The Engine.IO handshake. `upgrades` is empty because this endpoint only
/// speaks websocket: there is no polling transport to upgrade from, and the
/// app disables upgrading anyway.
pub(crate) fn open_frame(
	sid: &str,
	ping_interval_ms: u64,
	ping_timeout_ms: u64,
	max_payload: u64,
) -> String {
	format!(
		"{EIO_OPEN}{}",
		serde_json::json!({
			"sid": sid,
			"upgrades": [],
			"pingInterval": ping_interval_ms,
			"pingTimeout": ping_timeout_ms,
			"maxPayload": max_payload,
		})
	)
}

/// The Socket.IO CONNECT acknowledgement, which carries the *socket* id —
/// a different id from the Engine.IO session id, as in abs-ref.
pub(crate) fn connect_frame(socket_id: &str) -> String {
	format!(
		"{EIO_MESSAGE}{SIO_CONNECT}{}",
		serde_json::json!({ "sid": socket_id })
	)
}

/// A server event: `42["name", payload]`, or `42["name"]` without one.
pub(crate) fn event_frame(name: &str, payload: Option<Value>) -> String {
	let args = match payload {
		Some(payload) => serde_json::json!([name, payload]),
		None => serde_json::json!([name]),
	};
	format!("{EIO_MESSAGE}{SIO_EVENT}{args}")
}

pub(crate) fn ping_frame() -> String {
	EIO_PING.to_string()
}

pub(crate) fn pong_frame() -> String {
	EIO_PONG.to_string()
}

pub(crate) fn close_frame() -> String {
	EIO_CLOSE.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn the_open_frame_is_what_a_websocket_only_client_expects() {
		let frame = open_frame("abc", 25_000, 20_000, 1_000_000);
		let (kind, payload) = frame.split_at(1);
		assert_eq!(kind, "0");
		let payload: Value = serde_json::from_str(payload).expect("json payload");
		assert_eq!(payload["sid"], "abc");
		// A non-empty `upgrades` would make socket.io-client try to switch
		// transports on a server that has no second transport.
		assert_eq!(payload["upgrades"], json!([]));
		assert_eq!(payload["pingInterval"], 25_000);
		assert_eq!(payload["pingTimeout"], 20_000);
	}

	#[test]
	fn the_connect_reply_carries_a_socket_id() {
		assert_eq!(connect_frame("s-1"), r#"40{"sid":"s-1"}"#);
	}

	#[test]
	fn an_event_frame_is_a_json_array_of_name_and_payload() {
		assert_eq!(
			event_frame("auth_failed", Some(json!({ "message": "Invalid token" }))),
			r#"42["auth_failed",{"message":"Invalid token"}]"#
		);
		assert_eq!(event_frame("init", None), r#"42["init"]"#);
	}

	#[test]
	fn the_clients_handshake_frames_decode() {
		assert_eq!(
			decode("40"),
			ClientFrame::Connect {
				namespace: DEFAULT_NAMESPACE.to_owned()
			}
		);
		assert_eq!(
			decode(r#"42["auth","token-value"]"#),
			ClientFrame::Event {
				name: "auth".to_owned(),
				args: vec![json!("token-value")],
			}
		);
		assert_eq!(decode("3"), ClientFrame::Pong);
		assert_eq!(decode("2"), ClientFrame::Ping);
		assert_eq!(decode("1"), ClientFrame::Close);
		assert_eq!(decode("41"), ClientFrame::Close);
	}

	#[test]
	fn a_namespaced_connect_is_read_as_that_namespace() {
		// Only the default namespace is served; the caller answers the
		// others, so the decoder must report which one was asked for.
		assert_eq!(
			decode("40/admin,"),
			ClientFrame::Connect {
				namespace: "/admin".to_owned()
			}
		);
		assert_eq!(
			decode(r#"40/admin,{"token":"x"}"#),
			ClientFrame::Connect {
				namespace: "/admin".to_owned()
			}
		);
	}

	#[test]
	fn an_event_with_an_ack_id_still_decodes() {
		// `42<ack>[…]` is how a client asks for an acknowledgement; the
		// event still has to be read.
		assert_eq!(
			decode(r#"427["auth","tok"]"#),
			ClientFrame::Event {
				name: "auth".to_owned(),
				args: vec![json!("tok")],
			}
		);
	}

	#[test]
	fn frames_this_lane_has_no_meaning_for_are_ignored_not_fatal() {
		// Binary attachments, acks and garbage must never drop a socket
		// that is otherwise working.
		assert_eq!(decode(""), ClientFrame::Ignored);
		assert_eq!(decode("4"), ClientFrame::Ignored);
		assert_eq!(decode("43[]"), ClientFrame::Ignored);
		assert_eq!(decode("42"), ClientFrame::Ignored);
		assert_eq!(decode("42[]"), ClientFrame::Ignored);
		assert_eq!(decode("42[42]"), ClientFrame::Ignored);
		assert_eq!(decode("42not-json"), ClientFrame::Ignored);
		assert_eq!(decode("9"), ClientFrame::Ignored);
	}
}
