//! Typed, bounded transform/transfer profiles shared by target settings and
//! delivery queue snapshots.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const DEFAULT_RETRY_COUNT: u32 = 3;
pub const DEFAULT_RETRY_DELAY_SECONDS: u64 = 2;
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// Hardware selector used before `/api/status` resolves `AUTO`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CrosspointTargetModel {
	#[default]
	Auto,
	X3,
	X4,
}

impl CrosspointTargetModel {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Auto => "AUTO",
			Self::X3 => "X3",
			Self::X4 => "X4",
		}
	}
}

/// Nullable patch accepted by GraphQL/UI and target settings. `None` means
/// preserve the current target default; normalization fills defaults at write.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrosspointTransferProfileInput {
	pub optimizer_enabled: Option<bool>,
	pub target_model: Option<CrosspointTargetModel>,
	pub jpeg_quality: Option<u8>,
	pub grayscale: Option<bool>,
	pub auto_crop: Option<bool>,
	pub split_large_paragraphs: Option<bool>,
	pub remove_fonts: Option<bool>,
	pub chunk_bytes: Option<u32>,
	pub retry_count: Option<u32>,
	pub retry_delay_seconds: Option<u64>,
	pub timeout_seconds: Option<u64>,
	pub max_upload_bytes: Option<u64>,
}

/// Fully normalized profile persisted on targets and copied into each queue
/// item. It is safe to use after a restart without rereading mutable defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrosspointTransferProfile {
	pub optimizer_enabled: bool,
	pub target_model: CrosspointTargetModel,
	pub jpeg_quality: u8,
	pub grayscale: bool,
	pub auto_crop: bool,
	pub split_large_paragraphs: bool,
	pub remove_fonts: bool,
	pub chunk_bytes: u32,
	/// Number of additional attempts after the first transfer.
	pub retry_count: u32,
	pub retry_delay_seconds: u64,
	pub timeout_seconds: u64,
	pub max_upload_bytes: u64,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CrosspointProfileError {
	#[error("jpegQuality must be between 1 and 100")]
	JpegQuality,
	#[error("chunkBytes must be between 1 and 2048")]
	ChunkBytes,
	#[error("retryCount must be between 0 and 7")]
	RetryCount,
	#[error("retryDelaySeconds must be at most 86400")]
	RetryDelay,
	#[error("timeoutSeconds must be between 5 and 600")]
	Timeout,
	#[error("maxUploadBytes must be between 1 and 2147483647")]
	MaxUploadBytes,
}

impl Default for CrosspointTransferProfile {
	fn default() -> Self {
		Self {
			optimizer_enabled: false,
			target_model: CrosspointTargetModel::Auto,
			jpeg_quality: 85,
			grayscale: true,
			auto_crop: false,
			split_large_paragraphs: true,
			remove_fonts: true,
			chunk_bytes: 2048,
			retry_count: DEFAULT_RETRY_COUNT,
			retry_delay_seconds: DEFAULT_RETRY_DELAY_SECONDS,
			timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
			max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
		}
	}
}

impl CrosspointTransferProfileInput {
	pub fn normalized(self) -> Result<CrosspointTransferProfile, CrosspointProfileError> {
		let defaults = CrosspointTransferProfile::default();
		let profile = CrosspointTransferProfile {
			optimizer_enabled: self
				.optimizer_enabled
				.unwrap_or(defaults.optimizer_enabled),
			target_model: self.target_model.unwrap_or(defaults.target_model),
			jpeg_quality: self.jpeg_quality.unwrap_or(defaults.jpeg_quality),
			grayscale: self.grayscale.unwrap_or(defaults.grayscale),
			auto_crop: self.auto_crop.unwrap_or(defaults.auto_crop),
			split_large_paragraphs: self
				.split_large_paragraphs
				.unwrap_or(defaults.split_large_paragraphs),
			remove_fonts: self.remove_fonts.unwrap_or(defaults.remove_fonts),
			chunk_bytes: self.chunk_bytes.unwrap_or(defaults.chunk_bytes),
			retry_count: self.retry_count.unwrap_or(defaults.retry_count),
			retry_delay_seconds: self
				.retry_delay_seconds
				.unwrap_or(defaults.retry_delay_seconds),
			timeout_seconds: self.timeout_seconds.unwrap_or(defaults.timeout_seconds),
			max_upload_bytes: self.max_upload_bytes.unwrap_or(defaults.max_upload_bytes),
		};
		profile.validate()
	}
	/// Normalize an input patch for persistence and GraphQL. The string error
	/// keeps this pure protocol crate independent from an HTTP/GraphQL error
	/// type.
	pub fn normalize(self) -> Result<CrosspointTransferProfile, String> {
		self.normalized().map_err(|error| error.to_string())
	}
}

impl CrosspointTransferProfile {
	pub fn validate(self) -> Result<Self, CrosspointProfileError> {
		if !(1..=100).contains(&self.jpeg_quality) {
			return Err(CrosspointProfileError::JpegQuality);
		}
		if !(1..=2048).contains(&self.chunk_bytes) {
			return Err(CrosspointProfileError::ChunkBytes);
		}
		if !(0..=7).contains(&self.retry_count) {
			return Err(CrosspointProfileError::RetryCount);
		}
		if self.retry_delay_seconds > 86_400 {
			return Err(CrosspointProfileError::RetryDelay);
		}
		if !(5..=600).contains(&self.timeout_seconds) {
			return Err(CrosspointProfileError::Timeout);
		}
		if !(1..=i32::MAX as u64).contains(&self.max_upload_bytes) {
			return Err(CrosspointProfileError::MaxUploadBytes);
		}
		Ok(self)
	}

	/// Resolve `AUTO` after the verified target model is known. An explicit
	/// model mismatch is rejected instead of silently retargeting a device.
	pub fn resolve_for_model(
		mut self,
		model: crate::transfer::CrossPointModel,
	) -> Result<Self, String> {
		let model = match model {
			crate::transfer::CrossPointModel::X3 => CrosspointTargetModel::X3,
			crate::transfer::CrossPointModel::X4 => CrosspointTargetModel::X4,
		};
		match self.target_model {
			CrosspointTargetModel::Auto => self.target_model = model,
			selected if selected != model => {
				return Err(format!(
					"profile targets {}, verified device is {}",
					selected.as_str(),
					model.as_str()
				));
			},
			_ => {},
		}
		Ok(self)
	}

	/// Resolve against the protocol-neutral enum (useful before transfer
	/// module code has parsed `/api/status`).
	pub fn resolve_for_target_model(
		mut self,
		model: CrosspointTargetModel,
	) -> Result<Self, String> {
		match self.target_model {
			CrosspointTargetModel::Auto => self.target_model = model,
			selected if selected != model => {
				return Err(format!(
					"profile targets {}, verified device is {}",
					selected.as_str(),
					model.as_str()
				));
			},
			_ => {},
		}
		Ok(self)
	}

	/// One initial attempt plus the configured additional retries.
	pub const fn max_attempts(&self) -> u32 {
		self.retry_count.saturating_add(1)
	}

	/// Stable SHA-256 digest over canonical serde JSON (field order is fixed by
	/// the struct declaration).
	pub fn digest(&self) -> String {
		let json = serde_json::to_vec(self).expect("profile is serializable");
		let mut digest = Sha256::new();
		digest.update(json);
		format!("{:x}", digest.finalize())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_and_retry_semantics_are_explicit() {
		let profile = CrosspointTransferProfile::default();
		assert_eq!(profile.retry_count, 3);
		assert_eq!(profile.max_attempts(), 4);
		assert_eq!(profile.retry_delay_seconds, 2);
		assert_eq!(profile.max_upload_bytes, DEFAULT_MAX_UPLOAD_BYTES);
	}

	#[test]
	fn input_normalization_rejects_out_of_bounds_values() {
		let error = CrosspointTransferProfileInput {
			chunk_bytes: Some(2049),
			..Default::default()
		}
		.normalized()
		.unwrap_err();
		assert_eq!(error, CrosspointProfileError::ChunkBytes);
	}
}
