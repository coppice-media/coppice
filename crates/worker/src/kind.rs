//! Job kinds and capability matching.
//!
//! A job kind is a registry entry, not a database enum, for the same reason a
//! quality check is one (`crates/ingest/src/quality/`): the set grows, and
//! every addition would otherwise be a migration. A kind declares its id, the
//! capability keys a worker must advertise to be offered it, and whether the
//! server can run it itself when no worker can.
//!
//! `transcode` and `challenge_solve` are the registered kinds today. `align`
//! (tier 2 read-aloud) shares its input, capability requirement, and result
//! contracts from this crate, but no aligner runner or server route is part of
//! this slice.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

// Keep the job contracts discoverable beside the other kind DTOs while their
// validator lives in its own module.
pub use crate::alignment::{
	validate_sync_map, validate_sync_map_for_input, AlignExecutionProvider,
	AlignGranularity, AlignInput, AlignPrecision, AlignResult, AudioClip, SyncCue,
	SyncMapProvenance, SyncMapV1, SyncMapValidationContext, SyncMapValidationError,
	TextFragment, TrackDurationsMs, SYNC_MAP_MIME, SYNC_MAP_SCHEMA_VERSION,
};
use serde_json::Value;

/// The `transcode` job kind: re-encode one audio track into a delivery codec.
pub const TRANSCODE: &str = "transcode";

/// The `align` job kind: forced alignment of an EPUB against its audiobook.
/// The shared DTO and capability contract live in [`crate::alignment`]; no
/// aligner runner is registered by this crate.
pub const ALIGN: &str = "align";

/// The `challenge_solve` job kind: drive a real browser through a Cloudflare
/// managed challenge and hand back the clearance it earned.
///
/// Registered with **no** local implementation, and never gaining one: a
/// server has no browser, and the whole reason this kind exists is that the
/// challenge is unsolvable by anything that cannot execute the challenge
/// script. A server with no browser worker parks the job in `needs_worker`,
/// which is the honest answer.
pub const CHALLENGE_SOLVE: &str = "challenge_solve";

/// The capability key a worker advertises when it can drive a browser.
pub const BROWSER: &str = "browser";

/// One transcode job's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscodeInput {
	/// The book whose track is being transcoded.
	pub media_id: String,
	/// The track's index within the publication, as the audio manifest reports
	/// it — the worker fetches `/api/v2/media/{media_id}/audio/track/{index}`.
	pub track_index: i32,
	/// The track's length. The worker sizes its `ffmpeg` timeout from it: the
	/// server has the manifest and the worker does not, and a constant ceiling
	/// is either absurd for a two-minute track or too tight for a ten-hour one.
	#[serde(default)]
	pub duration_ms: i64,
	pub output: TranscodeOutput,
}

/// What a transcode produces. Externally tagged, so the wire form is
/// `{"opus": {"bitrate": "64k"}}` — the shape the design names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscodeOutput {
	Opus { bitrate: String },
	Aac { bitrate: String },
}

impl TranscodeOutput {
	/// The output file extension. `ffmpeg` picks its muxer from the output
	/// path, so this also selects the container: Ogg for Opus, MP4 for AAC.
	#[must_use]
	pub fn extension(&self) -> &'static str {
		match self {
			Self::Opus { .. } => "ogg",
			Self::Aac { .. } => "m4a",
		}
	}

	#[must_use]
	pub fn bitrate(&self) -> &str {
		match self {
			Self::Opus { bitrate } | Self::Aac { bitrate } => bitrate,
		}
	}

	/// The media type of the produced bytes.
	#[must_use]
	pub fn mime(&self) -> &'static str {
		match self {
			Self::Opus { .. } => "audio/ogg",
			Self::Aac { .. } => "audio/mp4",
		}
	}
}

/// What a `transcode` job reports when it finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscodeResult {
	/// Size of the uploaded (or locally written) output, in bytes.
	pub bytes: u64,
	/// Lowercase hex SHA-256 of those bytes.
	pub sha256: String,
	/// The media type, so a consumer does not have to re-derive it from the
	/// input it may no longer have.
	pub mime: String,
}

/// One `challenge_solve` job's input: which source instance is gated, and
/// where a browser has to go to earn its clearance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeSolveInput {
	/// `provider_sources.id`. Carried through so the server can apply the
	/// result without keeping any state of its own about the job.
	pub source_instance_id: String,
	/// The host the challenge sits on, e.g. `readcomicsonline.ru`. Only
	/// cookies for this host are returned.
	pub host: String,
	/// The URL to navigate to.
	pub url: String,
}

/// What a `challenge_solve` job reports when it finishes.
///
/// The worker reports observations, not policy: the cookies it was handed and
/// the user agent they were issued to. Deciding what to store, and how long a
/// clearance without its own expiry should be trusted, is the server's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChallengeSolveOutput {
	/// The host's cookies after the challenge cleared, `name` → `value`.
	/// Sorted, so the `Cookie` header a server builds from two solves of the
	/// same jar is byte-identical.
	pub cookies: BTreeMap<String, String>,
	/// The user agent the browser sent. A `cf_clearance` cookie is only
	/// accepted with the user agent it was issued to, so replaying the cookie
	/// without this is replaying nothing.
	pub user_agent: String,
	/// The clearance cookie's own expiry, in Unix seconds. `None` for a
	/// session cookie — the browser would drop it on exit, and the server
	/// substitutes its own horizon.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expires_at: Option<i64>,
}

/// The cookie Cloudflare issues for a solved managed challenge. Its presence
/// is what makes an output worth storing.
pub const CLEARANCE_COOKIE: &str = "cf_clearance";

impl ChallengeSolveOutput {
	/// The `Cookie` header value: `name=value` pairs joined with `; `, in the
	/// jar's sorted order.
	#[must_use]
	pub fn cookie_header(&self) -> String {
		let mut header = String::new();
		for (name, value) in &self.cookies {
			if !header.is_empty() {
				header.push_str("; ");
			}
			header.push_str(name);
			header.push('=');
			header.push_str(value);
		}
		header
	}

	/// Whether the jar carries a Cloudflare clearance. A solve that cleared
	/// the interstitial without one earned nothing replayable: the host let
	/// *that browser* through for another reason.
	#[must_use]
	pub fn has_clearance(&self) -> bool {
		self.cookies.contains_key(CLEARANCE_COOKIE)
	}
}

/// The capability requirement of a `challenge_solve` job: `{"browser": true}`.
#[must_use]
pub fn challenge_solve_requires() -> Value {
	serde_json::json!({ BROWSER: true })
}

/// The capability requirement of a `transcode` job: `{"transcode": true}`.
#[must_use]
pub fn transcode_requires() -> Value {
	serde_json::json!({ TRANSCODE: true })
}

/// The capability requirement of an `align` job.
///
/// The documented `device` capability remains a scalar, while model,
/// algorithm, and precision capabilities are arrays. The existing structural
/// matcher treats requirement arrays as containment, so a worker can advertise
/// several supported values while a job asks for exactly the values it needs.
#[must_use]
pub fn align_requires(input: &crate::alignment::AlignInput) -> Value {
	serde_json::json!({
		ALIGN: {
			"device": input.execution_provider,
			"models": [&input.model],
			"algorithms": [&input.algorithm],
			"precisions": [input.precision],
		}
	})
}

/// Whether `capabilities` satisfies `requires`.
///
/// The rule is deliberately structural rather than per-kind: a requirement is a
/// JSON subset test, so a new job kind adds a vocabulary, never a matcher.
///
/// * `true` requires the key to be advertised as anything truthy — the common
///   case, `{"transcode": true}` against `{"transcode": {"ffmpeg": "7.1"}}`.
/// * an object requires every one of its entries to be satisfied recursively.
/// * an array requires the advertised array to contain every element, so
///   `{"transcode": {"hwaccel": ["nvenc"]}}` picks the GPU box out of a pool.
/// * any other scalar requires equality.
/// * `false` and `null` require nothing, so a caller can write a requirement
///   out in full and switch parts of it off.
#[must_use]
pub fn satisfies(capabilities: &Value, requires: &Value) -> bool {
	match requires {
		Value::Null | Value::Bool(false) => true,
		Value::Bool(true) => is_truthy(capabilities),
		Value::Object(required) => {
			let Some(advertised) = capabilities.as_object() else {
				return false;
			};
			required.iter().all(|(key, want)| {
				satisfies(advertised.get(key).unwrap_or(&Value::Null), want)
			})
		},
		Value::Array(required) => {
			let Some(advertised) = capabilities.as_array() else {
				return false;
			};
			required.iter().all(|want| advertised.contains(want))
		},
		other => capabilities == other,
	}
}

/// Advertised-but-empty is not advertised: `{"transcode": false}` and
/// `{"transcode": null}` both mean "this worker cannot transcode", and an
/// empty object means it advertised the key and nothing behind it, which is
/// still a yes — a worker with `ffmpeg` and no hwaccel says
/// `{"transcode": {}}`.
fn is_truthy(value: &Value) -> bool {
	!matches!(value, Value::Null | Value::Bool(false))
}

/// Where a locally-run job writes the bytes it produces, and what it was asked
/// for. The path is the same one a remote worker's upload lands at, so the
/// caller that publishes the output does not care who ran the job.
#[derive(Debug, Clone)]
pub struct LocalJob {
	pub id: String,
	pub kind: String,
	pub input: Value,
	pub output_path: PathBuf,
}

/// The server-side implementation of a job kind, for the kinds that have one.
/// Registered by the host (`apps/server`) rather than by this crate: running a
/// transcode needs `ffmpeg`, the media row and the server's configuration,
/// none of which a protocol crate should know about.
#[cfg(feature = "server")]
#[async_trait::async_trait]
pub trait LocalRunner: Send + Sync {
	/// Produce the job's bytes at `job.output_path` and return its result JSON.
	async fn run(&self, job: LocalJob) -> Result<Value, String>;
}

/// The job kinds this server knows about, and how each is run when no worker
/// can be found.
///
/// Cheap to clone: the map holds `Arc`s, so the host builds the registry once
/// at boot and the service takes a copy.
#[cfg(feature = "server")]
#[derive(Clone, Default)]
pub struct KindRegistry {
	local: BTreeMap<String, std::sync::Arc<dyn LocalRunner>>,
}

#[cfg(feature = "server")]
impl KindRegistry {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Register the local fallback for `kind`, replacing any previous one.
	#[must_use]
	pub fn with_local(
		mut self,
		kind: impl Into<String>,
		runner: std::sync::Arc<dyn LocalRunner>,
	) -> Self {
		self.local.insert(kind.into(), runner);
		self
	}

	/// The local fallback for `kind`, when it has one.
	#[must_use]
	pub fn local(&self, kind: &str) -> Option<std::sync::Arc<dyn LocalRunner>> {
		self.local.get(kind).cloned()
	}
}

#[cfg(all(test, feature = "server"))]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn a_bare_key_requirement_matches_any_advertised_shape() {
		let requires = transcode_requires();
		assert!(satisfies(
			&json!({ "transcode": { "ffmpeg": "7.1" } }),
			&requires
		));
		assert!(satisfies(&json!({ "transcode": {} }), &requires));
		assert!(!satisfies(&json!({ "align": {} }), &requires));
		assert!(!satisfies(&json!({ "transcode": false }), &requires));
		assert!(!satisfies(&json!({ "transcode": null }), &requires));
	}

	/// The point of a nested requirement: pick the box with the encoder out of
	/// a pool where every worker advertises `transcode`.
	#[test]
	fn an_array_requirement_is_containment_not_equality() {
		let requires = json!({ "transcode": { "hwaccel": ["nvenc"] } });
		assert!(satisfies(
			&json!({ "transcode": { "hwaccel": ["vaapi", "nvenc"] } }),
			&requires
		));
		assert!(!satisfies(
			&json!({ "transcode": { "hwaccel": ["vaapi"] } }),
			&requires
		));
		assert!(!satisfies(&json!({ "transcode": {} }), &requires));
	}

	#[test]
	fn a_scalar_requirement_is_equality_and_false_requires_nothing() {
		assert!(satisfies(
			&json!({ "align": { "device": "cuda" } }),
			&json!({ "align": { "device": "cuda" } })
		));
		assert!(!satisfies(
			&json!({ "align": { "device": "cpu" } }),
			&json!({ "align": { "device": "cuda" } })
		));
		assert!(satisfies(&json!({}), &json!({ "align": false })));
		assert!(satisfies(&json!({}), &json!({})));
	}

	#[test]
	fn transcode_output_round_trips_in_the_documented_shape() {
		let input = TranscodeInput {
			media_id: "m1".into(),
			track_index: 2,
			duration_ms: 1_800_000,
			output: TranscodeOutput::Opus {
				bitrate: "64k".into(),
			},
		};
		let encoded = serde_json::to_value(&input).expect("encode");
		assert_eq!(
			encoded,
			json!({
				"media_id": "m1",
				"track_index": 2,
				"duration_ms": 1_800_000,
				"output": { "opus": { "bitrate": "64k" } }
			})
		);
		assert_eq!(
			serde_json::from_value::<TranscodeInput>(encoded).expect("decode"),
			input
		);
		assert_eq!(input.output.extension(), "ogg");
		assert_eq!(
			TranscodeOutput::Aac {
				bitrate: "96k".into()
			}
			.extension(),
			"m4a"
		);
	}

	/// The two halves of a `challenge_solve` live in different crates — the
	/// runner in a `browser` build of this one, the side effect in
	/// `stump_provider` — and only ever meet as the JSON in a `job` and a
	/// `result` frame. This is that JSON.
	#[test]
	fn challenge_solve_round_trips_in_the_documented_shape() {
		let input = ChallengeSolveInput {
			source_instance_id: "en.readcomicsonline".into(),
			host: "readcomicsonline.ru".into(),
			url: "https://readcomicsonline.ru".into(),
		};
		let encoded = serde_json::to_value(&input).expect("encode");
		assert_eq!(
			encoded,
			json!({
				"source_instance_id": "en.readcomicsonline",
				"host": "readcomicsonline.ru",
				"url": "https://readcomicsonline.ru"
			})
		);
		assert_eq!(
			serde_json::from_value::<ChallengeSolveInput>(encoded).expect("decode"),
			input
		);

		let output = ChallengeSolveOutput {
			cookies: BTreeMap::from([
				(CLEARANCE_COOKIE.to_string(), "abc".to_string()),
				("__cf_bm".to_string(), "bm".to_string()),
			]),
			user_agent: "Mozilla/5.0 (X11; Linux x86_64)".into(),
			expires_at: Some(1_800_000_000),
		};
		let encoded = serde_json::to_value(&output).expect("encode");
		assert_eq!(
			encoded,
			json!({
				"cookies": { "__cf_bm": "bm", "cf_clearance": "abc" },
				"user_agent": "Mozilla/5.0 (X11; Linux x86_64)",
				"expires_at": 1_800_000_000i64
			})
		);
		assert_eq!(
			serde_json::from_value::<ChallengeSolveOutput>(encoded).expect("decode"),
			output
		);
		// Sorted, so two solves of the same jar build the same header.
		assert_eq!(output.cookie_header(), "__cf_bm=bm; cf_clearance=abc");
		assert!(output.has_clearance());

		// A session clearance omits the field entirely rather than sending a
		// null an older server would have to interpret.
		let session = ChallengeSolveOutput {
			expires_at: None,
			..output
		};
		assert_eq!(
			serde_json::to_value(&session)
				.expect("encode")
				.get("expires_at"),
			None
		);
	}

	/// Routing for the kind: a browser worker is offered it, the `ffmpeg` box
	/// in the same pool is not. A `challenge_solve` sent to a worker with no
	/// browser would fail every time, and the job's whole point is that it
	/// parks instead.
	#[test]
	fn only_a_browser_worker_satisfies_a_challenge_solve() {
		let requires = challenge_solve_requires();
		assert_eq!(requires, json!({ "browser": true }));
		assert!(satisfies(
			&json!({ "browser": { "chrome": "Google Chrome 152.0.7977.82", "headless": false } }),
			&requires
		));
		// A worker that advertises both serves both.
		assert!(satisfies(
			&json!({ "transcode": { "ffmpeg": "7.1" }, "browser": {} }),
			&requires
		));
		assert!(!satisfies(
			&json!({ "transcode": { "ffmpeg": "7.1" } }),
			&requires
		));
		assert!(!satisfies(&json!({ "browser": false }), &requires));
	}

	/// A jar with no clearance in it is not worth storing: the host let *that
	/// browser* through for some other reason, and replaying `__cf_bm` alone
	/// gets the next request nowhere.
	#[test]
	fn a_jar_without_the_clearance_cookie_is_not_a_clearance() {
		let output = ChallengeSolveOutput {
			cookies: BTreeMap::from([("__cf_bm".to_string(), "bm".to_string())]),
			user_agent: "Mozilla/5.0".into(),
			expires_at: None,
		};
		assert!(!output.has_clearance());
		assert_eq!(ChallengeSolveOutput::default().cookie_header(), "");
	}
}
