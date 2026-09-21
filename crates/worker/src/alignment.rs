//! Tier-2 read-aloud alignment contracts.
//!
//! This module owns the data that crosses the worker boundary for an align
//! job, plus the small, portable SyncMap artifact produced by one. It does not
//! run an aligner or inspect media: callers supply the known duration of every
//! audio track when validating a map.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The only SyncMap schema this crate currently understands.
pub const SYNC_MAP_SCHEMA_VERSION: u32 = 1;

/// The media type for an alignment result artifact.
pub const SYNC_MAP_MIME: &str = "application/vnd.stump.sync-map+json";

/// Granularity requested from an aligner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignGranularity {
	Sentence,
	Word,
}

/// Execution backend requested from an aligner worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignExecutionProvider {
	Cpu,
	Cuda,
	Rocm,
	Metal,
}

/// Numeric precision requested from an aligner worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignPrecision {
	Fp32,
	Fp16,
	Int8,
}

/// One tier-2 alignment request.
///
/// `text_digest` identifies the exact canonical text the worker must align.
/// An audiobook may consist of many files, so its identity is deliberately an
/// `audio_manifest_digest` rather than a digest of one arbitrary track. The
/// server computes both digests; a worker only consumes them as provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignInput {
	/// The EPUB/text media row.
	pub text_media_id: String,
	/// Lowercase hexadecimal SHA-256 of the canonical text content.
	pub text_digest: String,
	/// The audiobook media row, including folder audiobooks.
	pub audio_media_id: String,
	/// Lowercase hexadecimal SHA-256 of the sorted audiobook track manifest.
	pub audio_manifest_digest: String,
	/// Stable alignment algorithm identifier, for example `ctc`.
	pub algorithm: String,
	/// Model identifier carried by the worker.
	pub model: String,
	/// Immutable model revision, digest, or release identifier.
	pub model_revision: String,
	/// BCP-47 language tag used for text normalization and inference.
	pub language: String,
	pub granularity: AlignGranularity,
	pub execution_provider: AlignExecutionProvider,
	pub precision: AlignPrecision,
	/// Algorithm knobs are ordered on the wire for deterministic job JSON.
	#[serde(default)]
	pub options: BTreeMap<String, serde_json::Value>,
}

/// Metadata reported alongside an uploaded SyncMap artifact.
///
/// The MIME is fixed by the job kind. [`AlignResult::new`] should be used by
/// producers so a result cannot accidentally advertise an audio or arbitrary
/// JSON content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignResult {
	/// Size of the uploaded SyncMap JSON, in bytes.
	pub bytes: u64,
	/// Lowercase hexadecimal SHA-256 of the uploaded SyncMap JSON.
	pub sha256: String,
}

impl AlignResult {
	/// The fixed MIME advertised by every align result.
	pub const MIME: &'static str = SYNC_MAP_MIME;

	/// Construct result metadata for a SyncMap artifact.
	#[must_use]
	pub fn new(bytes: u64, sha256: impl Into<String>) -> Self {
		Self {
			bytes,
			sha256: sha256.into(),
		}
	}

	/// The fixed MIME advertised by every align result.
	#[must_use]
	pub const fn mime(&self) -> &'static str {
		Self::MIME
	}
}

impl Serialize for AlignResult {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use serde::ser::SerializeStruct;

		let mut state = serializer.serialize_struct("AlignResult", 3)?;
		state.serialize_field("bytes", &self.bytes)?;
		state.serialize_field("sha256", &self.sha256)?;
		state.serialize_field("mime", &Self::MIME)?;
		state.end()
	}
}

impl<'de> Deserialize<'de> for AlignResult {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		#[derive(Deserialize)]
		struct Wire {
			bytes: u64,
			sha256: String,
			mime: String,
		}

		let wire = Wire::deserialize(deserializer)?;
		if wire.mime != Self::MIME {
			return Err(serde::de::Error::custom(format!(
				"align result MIME must be {}",
				Self::MIME
			)));
		}
		Ok(Self::new(wire.bytes, wire.sha256))
	}
}

/// A text target in a SyncMap cue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextFragment {
	/// Zero-based EPUB spine item index.
	pub spine_index: i32,
	/// Zero-based sentence/fragment ordinal within the spine item.
	pub ordinal: i32,
	/// Stable fragment/element id injected into that XHTML resource.
	pub element_id: String,
}

/// One timed range on one audiobook track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioClip {
	/// Zero-based audiobook track index.
	pub track_index: i32,
	/// Start relative to this track, in integer milliseconds.
	pub begin_ms: i64,
	/// End relative to this track, in integer milliseconds.
	pub end_ms: i64,
}

/// A text-fragment/audio-clip correspondence.
///
/// The two component structs are flattened to keep the wire artifact compact:
/// `{"spine_index":0,"ordinal":0,"element_id":"s1","track_index":0,...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncCue {
	#[serde(flatten)]
	pub text: TextFragment,
	#[serde(flatten)]
	pub audio: AudioClip,
	/// Alignment confidence in the inclusive range `0..=1`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub confidence: Option<f64>,
}

/// Generator, input identity, and execution provenance carried by a SyncMap.
///
/// Keeping the digests here makes a map self-verifying when it is imported
/// without the originating job row. `implementation` and
/// `version` identify the producer independently of the algorithm and model
/// names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMapProvenance {
	/// Lowercase hexadecimal SHA-256 of the canonical text content.
	pub text_digest: String,
	/// Lowercase hexadecimal SHA-256 of the sorted audiobook track manifest.
	pub audio_manifest_digest: String,
	pub implementation: String,
	pub version: String,
	pub algorithm: String,
	pub model: String,
	pub model_revision: String,
	pub language: String,
	pub granularity: AlignGranularity,
	pub execution_provider: AlignExecutionProvider,
	pub precision: AlignPrecision,
	/// Ordered options make the same request serialize byte-for-byte alike.
	#[serde(default)]
	pub options: BTreeMap<String, serde_json::Value>,
}

/// The compact, validated v1 timing artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncMapV1 {
	/// Must equal [`SYNC_MAP_SCHEMA_VERSION`].
	pub schema: u32,
	/// Input media identities are retained for local provenance; the exact
	/// content digests live in [`SyncMapProvenance`].
	pub text_media_id: String,
	pub audio_media_id: String,
	pub provenance: SyncMapProvenance,
	/// Cues are ordered by text target and non-decreasing audio track/time.
	pub cues: Vec<SyncCue>,
}

/// Per-track duration bounds supplied by the caller that already probed the
/// audiobook. Values and cue times are integer milliseconds.
pub type SyncMapValidationContext = BTreeMap<i32, i64>;

/// Alias documenting the unit of the validation context at call sites.
pub type TrackDurationsMs = SyncMapValidationContext;

/// A typed failure from [`SyncMapV1::validate`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SyncMapValidationError {
	#[error("unsupported SyncMap schema {found}; expected {expected}")]
	UnsupportedSchema { expected: u32, found: u32 },
	#[error("{field} is not a lowercase hexadecimal SHA-256 digest")]
	MalformedDigest { field: &'static str },
	#[error("{field} is empty")]
	EmptyField { field: &'static str },
	#[error("{field} does not match the align request")]
	InputMismatch { field: &'static str },
	#[error("SyncMap has no cues")]
	EmptyMap,
	#[error("cue {cue_index} repeats text target spine={spine_index} element_id={element_id:?}")]
	DuplicateTarget {
		cue_index: usize,
		spine_index: i32,
		element_id: String,
	},
	#[error("cue {cue_index} has a negative spine index {spine_index}")]
	NegativeSpineIndex { cue_index: usize, spine_index: i32 },
	#[error("cue {cue_index} has a negative text ordinal {ordinal}")]
	NegativeOrdinal { cue_index: usize, ordinal: i32 },
	#[error("cue {cue_index} has a negative track index {track_index}")]
	NegativeTrackIndex { cue_index: usize, track_index: i32 },
	#[error("cue {cue_index} has an empty text element_id")]
	EmptyTarget { cue_index: usize },
	#[error("cue {cue_index} has a non-positive audio interval [{begin_ms}, {end_ms})")]
	NonPositiveInterval {
		cue_index: usize,
		begin_ms: i64,
		end_ms: i64,
	},
	#[error("cue {cue_index} moves text backward from {previous:?} to {current:?}")]
	BackwardTextOrder {
		cue_index: usize,
		previous: TextFragment,
		current: TextFragment,
	},
	#[error("cue {cue_index} moves audio backward from track {previous_track} to {current_track}")]
	BackwardAudioOrder {
		cue_index: usize,
		previous_track: i32,
		current_track: i32,
	},
	#[error("cue {cue_index} overlaps an earlier cue on track {track_index}")]
	SameTrackOverlap { cue_index: usize, track_index: i32 },
	#[error("cue {cue_index} refers to unknown audio track {track_index}")]
	UnknownTrack { cue_index: usize, track_index: i32 },
	#[error(
		"cue {cue_index} ends at {end_ms} ms beyond track {track_index} duration {duration_ms} ms"
	)]
	ClipOutOfBounds {
		cue_index: usize,
		track_index: i32,
		end_ms: i64,
		duration_ms: i64,
	},
	#[error("cue {cue_index} has invalid confidence {value}")]
	InvalidConfidence { cue_index: usize, value: f64 },
	#[error("track {track_index} has invalid duration {duration_ms} ms")]
	InvalidTrackDuration { track_index: i32, duration_ms: i64 },
}

impl SyncMapV1 {
	/// Validate identity, ordering, intervals, confidence, and caller-supplied
	/// per-track duration bounds without probing media again.
	pub fn validate(
		&self,
		track_durations_ms: &SyncMapValidationContext,
	) -> Result<(), SyncMapValidationError> {
		if self.schema != SYNC_MAP_SCHEMA_VERSION {
			return Err(SyncMapValidationError::UnsupportedSchema {
				expected: SYNC_MAP_SCHEMA_VERSION,
				found: self.schema,
			});
		}
		if !is_lowercase_sha256(&self.provenance.text_digest) {
			return Err(SyncMapValidationError::MalformedDigest {
				field: "provenance.text_digest",
			});
		}
		if !is_lowercase_sha256(&self.provenance.audio_manifest_digest) {
			return Err(SyncMapValidationError::MalformedDigest {
				field: "provenance.audio_manifest_digest",
			});
		}
		for (field, value) in [
			("text_media_id", self.text_media_id.as_str()),
			("audio_media_id", self.audio_media_id.as_str()),
			(
				"provenance.implementation",
				self.provenance.implementation.as_str(),
			),
			("provenance.version", self.provenance.version.as_str()),
			("provenance.algorithm", self.provenance.algorithm.as_str()),
			("provenance.model", self.provenance.model.as_str()),
			(
				"provenance.model_revision",
				self.provenance.model_revision.as_str(),
			),
			("provenance.language", self.provenance.language.as_str()),
		] {
			if value.trim().is_empty() {
				return Err(SyncMapValidationError::EmptyField { field });
			}
		}
		if self.cues.is_empty() {
			return Err(SyncMapValidationError::EmptyMap);
		}

		// A duration map is caller-owned evidence. Reject malformed bounds once,
		// even when no cue happens to reference that track.
		for (&track_index, &duration_ms) in track_durations_ms {
			if duration_ms <= 0 {
				return Err(SyncMapValidationError::InvalidTrackDuration {
					track_index,
					duration_ms,
				});
			}
		}

		let mut targets = BTreeSet::new();
		let mut previous_text: Option<&TextFragment> = None;
		let mut previous_audio: Option<AudioClip> = None;

		for (cue_index, cue) in self.cues.iter().enumerate() {
			if cue.text.spine_index < 0 {
				return Err(SyncMapValidationError::NegativeSpineIndex {
					cue_index,
					spine_index: cue.text.spine_index,
				});
			}
			if cue.text.ordinal < 0 {
				return Err(SyncMapValidationError::NegativeOrdinal {
					cue_index,
					ordinal: cue.text.ordinal,
				});
			}
			if cue.audio.track_index < 0 {
				return Err(SyncMapValidationError::NegativeTrackIndex {
					cue_index,
					track_index: cue.audio.track_index,
				});
			}
			if cue.text.element_id.is_empty() {
				return Err(SyncMapValidationError::EmptyTarget { cue_index });
			}
			let target = (cue.text.spine_index, cue.text.element_id.as_str());
			if !targets.insert(target) {
				return Err(SyncMapValidationError::DuplicateTarget {
					cue_index,
					spine_index: cue.text.spine_index,
					element_id: cue.text.element_id.clone(),
				});
			}

			if cue.audio.begin_ms < 0 || cue.audio.end_ms <= cue.audio.begin_ms {
				return Err(SyncMapValidationError::NonPositiveInterval {
					cue_index,
					begin_ms: cue.audio.begin_ms,
					end_ms: cue.audio.end_ms,
				});
			}

			let Some(&duration_ms) = track_durations_ms.get(&cue.audio.track_index)
			else {
				return Err(SyncMapValidationError::UnknownTrack {
					cue_index,
					track_index: cue.audio.track_index,
				});
			};
			if cue.audio.end_ms > duration_ms {
				return Err(SyncMapValidationError::ClipOutOfBounds {
					cue_index,
					track_index: cue.audio.track_index,
					end_ms: cue.audio.end_ms,
					duration_ms,
				});
			}

			if let Some(confidence) = cue.confidence {
				if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
					return Err(SyncMapValidationError::InvalidConfidence {
						cue_index,
						value: confidence,
					});
				}
			}

			if let Some(previous) = previous_text {
				if (cue.text.spine_index, cue.text.ordinal)
					< (previous.spine_index, previous.ordinal)
				{
					return Err(SyncMapValidationError::BackwardTextOrder {
						cue_index,
						previous: previous.clone(),
						current: cue.text.clone(),
					});
				}
			}

			if let Some(previous) = previous_audio {
				if cue.audio.track_index < previous.track_index {
					return Err(SyncMapValidationError::BackwardAudioOrder {
						cue_index,
						previous_track: previous.track_index,
						current_track: cue.audio.track_index,
					});
				}
				if cue.audio.track_index == previous.track_index {
					if cue.audio.begin_ms < previous.begin_ms {
						return Err(SyncMapValidationError::BackwardAudioOrder {
							cue_index,
							previous_track: previous.track_index,
							current_track: cue.audio.track_index,
						});
					}
					if cue.audio.begin_ms < previous.end_ms {
						return Err(SyncMapValidationError::SameTrackOverlap {
							cue_index,
							track_index: cue.audio.track_index,
						});
					}
				}
			}

			previous_text = Some(&cue.text);
			previous_audio = Some(cue.audio);
		}

		Ok(())
	}

	/// Validate the map and require every request-controlled identity and
	/// generator option to match the job that produced it.
	pub fn validate_for(
		&self,
		input: &AlignInput,
		track_durations_ms: &SyncMapValidationContext,
	) -> Result<(), SyncMapValidationError> {
		self.validate(track_durations_ms)?;
		for (field, matches) in [
			("text_media_id", self.text_media_id == input.text_media_id),
			(
				"provenance.text_digest",
				self.provenance.text_digest == input.text_digest,
			),
			(
				"audio_media_id",
				self.audio_media_id == input.audio_media_id,
			),
			(
				"provenance.audio_manifest_digest",
				self.provenance.audio_manifest_digest == input.audio_manifest_digest,
			),
			(
				"provenance.algorithm",
				self.provenance.algorithm == input.algorithm,
			),
			("provenance.model", self.provenance.model == input.model),
			(
				"provenance.model_revision",
				self.provenance.model_revision == input.model_revision,
			),
			(
				"provenance.language",
				self.provenance.language == input.language,
			),
			(
				"provenance.granularity",
				self.provenance.granularity == input.granularity,
			),
			(
				"provenance.execution_provider",
				self.provenance.execution_provider == input.execution_provider,
			),
			(
				"provenance.precision",
				self.provenance.precision == input.precision,
			),
			(
				"provenance.options",
				self.provenance.options == input.options,
			),
		] {
			if !matches {
				return Err(SyncMapValidationError::InputMismatch { field });
			}
		}
		Ok(())
	}
}

/// Validate a v1 SyncMap against durations already known by the caller.
pub fn validate_sync_map(
	map: &SyncMapV1,
	track_durations_ms: &SyncMapValidationContext,
) -> Result<(), SyncMapValidationError> {
	map.validate(track_durations_ms)
}

/// Validate structure, duration bounds, and identity against the producing job.
pub fn validate_sync_map_for_input(
	map: &SyncMapV1,
	input: &AlignInput,
	track_durations_ms: &SyncMapValidationContext,
) -> Result<(), SyncMapValidationError> {
	map.validate_for(input, track_durations_ms)
}

fn is_lowercase_sha256(value: &str) -> bool {
	value.len() == 64
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	const DIGEST: &str =
		"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

	fn provenance() -> SyncMapProvenance {
		SyncMapProvenance {
			text_digest: DIGEST.into(),
			audio_manifest_digest: DIGEST.into(),
			implementation: "stump-test-aligner".into(),
			version: "r1".into(),
			algorithm: "ctc".into(),
			model: "wav2vec2-base-960h".into(),
			model_revision: "r1".into(),
			language: "en".into(),
			granularity: AlignGranularity::Sentence,
			execution_provider: AlignExecutionProvider::Cuda,
			precision: AlignPrecision::Fp16,
			options: BTreeMap::from([(String::from("beam"), json!(8))]),
		}
	}

	fn cue(
		spine_index: i32,
		ordinal: i32,
		element_id: &str,
		track_index: i32,
		begin_ms: i64,
		end_ms: i64,
	) -> SyncCue {
		SyncCue {
			text: TextFragment {
				spine_index,
				ordinal,
				element_id: element_id.into(),
			},
			audio: AudioClip {
				track_index,
				begin_ms,
				end_ms,
			},
			confidence: Some(0.9),
		}
	}

	fn map(cues: Vec<SyncCue>) -> SyncMapV1 {
		SyncMapV1 {
			schema: SYNC_MAP_SCHEMA_VERSION,
			text_media_id: "ebook".into(),
			audio_media_id: "audio".into(),
			provenance: provenance(),
			cues,
		}
	}

	#[test]
	fn sync_map_wire_json_is_compact_and_snake_case() {
		let value =
			serde_json::to_value(map(vec![cue(0, 0, "s1", 0, 0, 1_000)])).unwrap();
		assert_eq!(
			value,
			json!({
				"schema": 1,
				"text_media_id": "ebook",
				"audio_media_id": "audio",
				"provenance": {
					"text_digest": DIGEST,
					"audio_manifest_digest": DIGEST,
					"implementation": "stump-test-aligner",
					"version": "r1",
					"algorithm": "ctc",
					"model": "wav2vec2-base-960h",
					"model_revision": "r1",
					"language": "en",
					"granularity": "sentence",
					"execution_provider": "cuda",
					"precision": "fp16",
					"options": {"beam": 8}
				},
				"cues": [{
					"spine_index": 0,
					"ordinal": 0,
					"element_id": "s1",
					"track_index": 0,
					"begin_ms": 0,
					"end_ms": 1_000,
					"confidence": 0.9
				}]
			})
		);
	}

	#[test]
	fn align_input_and_requirement_have_literal_wire_shapes() {
		let input = AlignInput {
			text_media_id: "ebook".into(),
			text_digest: DIGEST.into(),
			audio_media_id: "audio-folder".into(),
			audio_manifest_digest: DIGEST.into(),
			algorithm: "ctc".into(),
			model: "wav2vec2-base-960h".into(),
			model_revision: "r1".into(),
			language: "en".into(),
			granularity: AlignGranularity::Sentence,
			execution_provider: AlignExecutionProvider::Cuda,
			precision: AlignPrecision::Fp16,
			options: BTreeMap::from([(String::from("beam"), json!(8))]),
		};
		assert_eq!(
			serde_json::to_value(&input).unwrap(),
			json!({
				"text_media_id": "ebook",
				"text_digest": DIGEST,
				"audio_media_id": "audio-folder",
				"audio_manifest_digest": DIGEST,
				"algorithm": "ctc",
				"model": "wav2vec2-base-960h",
				"model_revision": "r1",
				"language": "en",
				"granularity": "sentence",
				"execution_provider": "cuda",
				"precision": "fp16",
				"options": {"beam": 8}
			})
		);
		let requires = crate::kind::align_requires(&input);
		assert_eq!(
			requires,
			json!({
				"align": {
					"device": "cuda",
					"models": ["wav2vec2-base-960h"],
					"algorithms": ["ctc"],
					"precisions": ["fp16"]
				}
			})
		);
		assert!(crate::satisfies(
			&json!({
				"align": {
					"device": "cuda",
					"models": ["wav2vec2-base-960h", "other-model"],
					"algorithms": ["ctc", "storyteller"],
					"precisions": ["fp16", "int8"]
				}
			}),
			&requires
		));
		assert!(!crate::satisfies(
			&json!({
				"align": {
					"device": "cpu",
					"models": ["wav2vec2-base-960h"],
					"algorithms": ["ctc"],
					"precisions": ["fp16"]
				}
			}),
			&requires
		));
	}

	#[test]
	fn align_result_serializes_and_requires_the_fixed_sync_map_mime() {
		let result = AlignResult::new(42, DIGEST);
		assert_eq!(result.mime(), SYNC_MAP_MIME);
		assert_eq!(
			serde_json::to_value(&result).unwrap(),
			json!({
				"bytes": 42,
				"sha256": DIGEST,
				"mime": SYNC_MAP_MIME
			})
		);
		assert!(serde_json::from_value::<AlignResult>(json!({
			"bytes": 42,
			"sha256": DIGEST,
			"mime": SYNC_MAP_MIME
		}))
		.is_ok());
		assert!(serde_json::from_value::<AlignResult>(json!({
			"bytes": 42,
			"sha256": DIGEST,
			"mime": "application/json"
		}))
		.is_err());
		assert!(serde_json::from_value::<AlignResult>(json!({
			"bytes": 42,
			"sha256": DIGEST
		}))
		.is_err());
	}

	#[test]
	fn valid_map_allows_a_track_clock_reset_when_track_advances() {
		let value = map(vec![
			cue(0, 0, "s1", 0, 8_000, 9_000),
			cue(0, 1, "s2", 1, 0, 500),
		]);
		assert!(value
			.validate(&BTreeMap::from([(0, 9_000), (1, 2_000)]))
			.is_ok());
	}

	#[test]
	fn text_order_uses_numeric_ordinal_not_element_id_lexicography() {
		let value = map(vec![
			cue(0, 1, "s2", 0, 0, 500),
			cue(0, 2, "s10", 0, 500, 1_000),
		]);
		assert!(value.validate(&BTreeMap::from([(0, 2_000)])).is_ok());
	}

	#[test]
	fn validator_matches_the_map_to_its_align_request() {
		let input = AlignInput {
			text_media_id: "ebook".into(),
			text_digest: DIGEST.into(),
			audio_media_id: "audio".into(),
			audio_manifest_digest: DIGEST.into(),
			algorithm: "ctc".into(),
			model: "wav2vec2-base-960h".into(),
			model_revision: "r1".into(),
			language: "en".into(),
			granularity: AlignGranularity::Sentence,
			execution_provider: AlignExecutionProvider::Cuda,
			precision: AlignPrecision::Fp16,
			options: BTreeMap::from([(String::from("beam"), json!(8))]),
		};
		let durations = BTreeMap::from([(0, 2_000)]);
		let value = map(vec![cue(0, 0, "s1", 0, 0, 1_000)]);
		assert!(value.validate_for(&input, &durations).is_ok());

		let mut mismatch = value;
		mismatch.provenance.model_revision = "other".into();
		assert!(matches!(
			mismatch.validate_for(&input, &durations),
			Err(SyncMapValidationError::InputMismatch {
				field: "provenance.model_revision"
			})
		));
	}

	#[test]
	fn validator_rejects_schema_digest_empty_duplicate_overlap_order_bounds_and_confidence(
	) {
		let durations = BTreeMap::from([(0, 2_000), (1, 2_000)]);
		let mut invalid = map(vec![cue(0, 0, "s1", 0, 0, 1_000)]);
		invalid.schema = 2;
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::UnsupportedSchema { .. })
		));

		let mut invalid = map(vec![cue(0, 0, "s1", 0, 0, 1_000)]);
		invalid.provenance.text_digest = DIGEST.to_ascii_uppercase();
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::MalformedDigest { .. })
		));

		let mut invalid = map(vec![cue(0, 0, "s1", 0, 0, 1_000)]);
		invalid.provenance.implementation.clear();
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::EmptyField {
				field: "provenance.implementation"
			})
		));

		let invalid = map(Vec::new());
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::EmptyMap)
		));

		let invalid = map(vec![cue(-1, 0, "s1", 0, 0, 1_000)]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::NegativeSpineIndex { .. })
		));

		let mut invalid = map(vec![cue(0, 0, "s1", 0, 0, 1_000)]);
		invalid.cues[0].text.ordinal = -1;
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::NegativeOrdinal { .. })
		));

		let invalid = map(vec![cue(0, 0, "s1", -1, 0, 1_000)]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::NegativeTrackIndex { .. })
		));

		let invalid = map(vec![
			cue(0, 9, "s10", 0, 0, 500),
			cue(0, 1, "s2", 0, 500, 1_000),
		]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::BackwardTextOrder { .. })
		));

		let invalid = map(vec![cue(0, 0, "s1", 0, 500, 500)]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::NonPositiveInterval { .. })
		));

		let invalid = map(vec![
			cue(0, 0, "s1", 0, 0, 1_000),
			cue(0, 1, "s1", 0, 1_000, 1_500),
		]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::DuplicateTarget { .. })
		));

		let invalid = map(vec![
			cue(0, 0, "s1", 0, 0, 1_000),
			cue(0, 1, "s2", 0, 900, 1_500),
		]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::SameTrackOverlap { .. })
		));

		let invalid = map(vec![cue(0, 0, "s1", 1, 0, 500), cue(0, 1, "s2", 0, 0, 500)]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::BackwardAudioOrder { .. })
		));

		let invalid = map(vec![cue(0, 0, "s1", 0, 1_500, 2_500)]);
		assert!(matches!(
			invalid.validate(&durations),
			Err(SyncMapValidationError::ClipOutOfBounds { .. })
		));

		for confidence in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
			let mut invalid = map(vec![cue(0, 0, "s1", 0, 0, 1_000)]);
			invalid.cues[0].confidence = Some(confidence);
			assert!(matches!(
				invalid.validate(&durations),
				Err(SyncMapValidationError::InvalidConfidence { .. })
			));
		}
	}
}
