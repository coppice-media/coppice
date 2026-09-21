//! Typed GraphQL inputs for CrossPoint target settings.

use async_graphql::InputObject;

use crate::object::crosspoint::CrosspointTargetModel;

impl From<CrosspointTargetModel> for stump_crosspoint::profile::CrosspointTargetModel {
	fn from(value: CrosspointTargetModel) -> Self {
		match value {
			CrosspointTargetModel::Auto => Self::Auto,
			CrosspointTargetModel::X3 => Self::X3,
			CrosspointTargetModel::X4 => Self::X4,
		}
	}
}

/// A bounded, nullable patch for a CrossPoint transfer profile.
///
/// The protocol crate owns defaults and validation. This type only bridges
/// GraphQL's `Int` scalar to the protocol's unsigned fields; callers must run
/// [`Self::into_protocol`] before persisting or queueing the profile.
#[derive(Clone, Debug, Default, InputObject)]
pub struct CrosspointTransferProfileInput {
	pub optimizer_enabled: Option<bool>,
	pub target_model: Option<CrosspointTargetModel>,
	pub jpeg_quality: Option<i32>,
	pub grayscale: Option<bool>,
	pub auto_crop: Option<bool>,
	pub split_large_paragraphs: Option<bool>,
	pub remove_fonts: Option<bool>,
	pub chunk_bytes: Option<i32>,
	pub retry_count: Option<i32>,
	pub retry_delay_seconds: Option<i32>,
	pub timeout_seconds: Option<i32>,
	pub max_upload_bytes: Option<i32>,
}

impl CrosspointTransferProfileInput {
	/// Convert this GraphQL patch into the protocol crate's typed patch.
	///
	/// Values outside GraphQL's signed `Int` range are rejected by
	/// async-graphql before this function is called. Negative values are
	/// intentionally mapped to the nearest invalid protocol value so the
	/// canonical profile validator returns the field-specific bounds error.
	pub fn into_protocol(
		self,
	) -> stump_crosspoint::profile::CrosspointTransferProfileInput {
		let jpeg_quality = self.jpeg_quality.map(|value| {
			if !(1..=100).contains(&value) {
				0
			} else {
				value as u8
			}
		});
		let chunk_bytes = self
			.chunk_bytes
			.map(|value| if value < 0 { 0 } else { value as u32 });
		let retry_count =
			self.retry_count
				.map(|value| if value < 0 { u32::MAX } else { value as u32 });
		let retry_delay_seconds =
			self.retry_delay_seconds.map(
				|value| {
					if value < 0 {
						u64::MAX
					} else {
						value as u64
					}
				},
			);
		let timeout_seconds =
			self.timeout_seconds
				.map(|value| if value < 0 { 0 } else { value as u64 });
		let max_upload_bytes =
			self.max_upload_bytes
				.map(|value| if value < 0 { 0 } else { value as u64 });

		stump_crosspoint::profile::CrosspointTransferProfileInput {
			optimizer_enabled: self.optimizer_enabled,
			target_model: self.target_model.map(Into::into),
			jpeg_quality,
			grayscale: self.grayscale,
			auto_crop: self.auto_crop,
			split_large_paragraphs: self.split_large_paragraphs,
			remove_fonts: self.remove_fonts,
			chunk_bytes,
			retry_count,
			retry_delay_seconds,
			timeout_seconds,
			max_upload_bytes,
		}
	}
}

/// Verified CrossPoint endpoint settings and an optional typed profile patch.
///
/// HTTP/WebSocket ports are accepted for the existing Home client contract but
/// are policy inputs, not routing inputs: mutations require exactly firmware
/// ports 80 and 81 and persist those constants rather than trusting a caller.
#[derive(Clone, Debug, InputObject)]
pub struct CrosspointTargetInput {
	pub host_or_ip: String,
	pub http_port: i32,
	pub ws_port: i32,
	pub root_path: String,
	pub profile: Option<CrosspointTransferProfileInput>,
}
