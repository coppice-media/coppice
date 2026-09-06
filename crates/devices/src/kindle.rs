//! Send-to-Kindle addressing: the per-device Amazon address and the summary
//! one completed delivery records.
//!
//! A Kindle speaks no sync protocol. Amazon gives each device an address
//! (`<name>@kindle.com`) and delivers whatever an *approved sender* mails to
//! it, so the address is the whole transport and it lives on the device row
//! next to the transform profile and the library scope.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use crate::error::{DeviceError, DeviceResult};

/// The `protocol` a send-to-Kindle summary reports.
///
/// Deliberately not a [`DeviceProtocol`](models::shared::enums::DeviceProtocol)
/// variant: that enum names the protocols a *credential* authenticates a
/// request on, and a send-to-Kindle delivery has neither — Stump mailed the
/// book out on its own initiative.
pub const KINDLE_EMAIL_PROTOCOL: &str = "kindle-email";

/// The longest address accepted: the RFC 5321 forward-path limit.
const MAX_EMAIL_CHARS: usize = 254;

/// What one completed delivery writes to `devices.last_sync_summary`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindleSendSummary {
	/// Always [`KINDLE_EMAIL_PROTOCOL`], so a stored summary says which lane
	/// wrote it exactly like the Kobo and KOReader summaries do.
	pub protocol: String,
	/// Size of the attachment that actually went out.
	pub bytes: u64,
	/// Extension of that attachment: `azw3` when the book was converted for
	/// the Kindle, otherwise the book's own format.
	pub format: String,
}

impl KindleSendSummary {
	pub fn new(bytes: u64, format: impl Into<String>) -> Self {
		Self {
			protocol: KINDLE_EMAIL_PROTOCOL.to_string(),
			bytes,
			format: format.into(),
		}
	}
}

impl From<&KindleSendSummary> for JsonValue {
	/// The stored form of the summary. Infallible by construction — every
	/// field is a string or a number — so a recorded delivery can never be
	/// lost to a serialisation error.
	fn from(summary: &KindleSendSummary) -> Self {
		json!({
			"protocol": summary.protocol,
			"bytes": summary.bytes,
			"format": summary.format,
		})
	}
}

/// Validates and trims an address for
/// [`DeviceService::set_kindle_email`](crate::DeviceService::set_kindle_email).
///
/// The check is syntactic and deliberately domain-agnostic: Amazon runs
/// regional Send-to-Kindle domains (`@kindle.com`, `@free.kindle.com`, …) and
/// an operator may point the device at their own forwarder, so pinning the
/// domain would reject working setups. A malformed address is caught here
/// rather than at send time, where it would surface as an SMTP rejection long
/// after the operator typed it.
pub fn normalize_kindle_email(email: &str) -> DeviceResult<String> {
	let address = email.trim();
	let invalid = |reason: &str| {
		Err(DeviceError::InvalidEmail(format!(
			"{address:?} is not a usable address: {reason}"
		)))
	};

	if address.is_empty() {
		return invalid("it is empty");
	}
	if address.chars().count() > MAX_EMAIL_CHARS {
		return invalid("it is longer than 254 characters");
	}
	if address.chars().any(char::is_whitespace) {
		return invalid("it contains whitespace");
	}
	let Some((local, domain)) = address.split_once('@') else {
		return invalid("it has no @");
	};
	if local.is_empty() {
		return invalid("it has nothing before the @");
	}
	if domain.contains('@') {
		return invalid("it has more than one @");
	}
	// A bare host is never deliverable from a public SMTP relay, and it is the
	// typo an operator actually makes ("al@kindle" for "al@kindle.com").
	if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
		return invalid("its domain is not a hostname");
	}

	Ok(address.to_string())
}
