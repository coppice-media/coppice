//! [`TransformProfile`] — the per-device description of how comic pages are
//! prepared for delivery — plus the named device presets.

use serde::{Deserialize, Serialize};

use super::error::TransformResult;
use crate::transform::error::TransformError;

/// Output image format for transformed pages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TransformFormat {
	/// JPEG with a quality in `1..=100` and a chroma subsampling mode.
	Jpeg {
		quality: u8,
		subsampling: JpegSubsampling,
	},
	/// Lossless PNG (large; rarely useful for delivery, kept for
	/// completeness).
	Png,
	/// WebP with a quality in `1..=100`.
	Webp { quality: u8 },
}

impl Default for TransformFormat {
	fn default() -> Self {
		Self::Jpeg {
			quality: 85,
			subsampling: JpegSubsampling::Auto,
		}
	}
}

/// Chroma subsampling for JPEG output.
///
/// `Auto` encodes grayscale pages as JPEG grayscale and colour pages as 4:2:0,
/// which is what e-ink delivery wants. The `image`-crate fallback encoder
/// ignores the chroma mode for colour pages (it always emits 4:2:0) and honours
/// `Gray` for luma pages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JpegSubsampling {
	/// Grayscale for luma pages, 4:2:0 for colour pages.
	#[default]
	Auto,
	/// 4:4:4 (no chroma subsampling).
	S444,
	/// 4:2:2.
	S422,
	/// 4:2:0.
	S420,
	/// JPEG grayscale.
	Gray,
}

/// The container a transformed comic is delivered in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComicContainer {
	/// A stored (uncompressed) CBZ with an optional `ComicInfo.xml`.
	Cbz,
	/// A fixed-layout EPUB ("KEPUB"): one XHTML page per image, sized to the
	/// panel, with Kobo's `kobostylehacks` wrapper applied through
	/// `stump_kepub`'s public content transform.
	#[default]
	KepubFixedLayout,
}

/// How comic pages are resized, toned, and encoded for a device.
///
/// Every field has a serde default, so a device's stored `transform_profile`
/// JSON may specify only the fields it overrides (e.g. `{"grayscale": false}`).
/// A bare string is a preset name (`"libra"`), and `{"preset": "clara"}` selects
/// a preset — see [`TransformProfile::from_device_profile`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransformProfile {
	/// Largest allowed page width; pages are only ever downscaled.
	pub max_width: Option<u32>,
	/// Largest allowed page height; pages are only ever downscaled.
	pub max_height: Option<u32>,
	/// Convert pages to 8-bit grayscale.
	pub grayscale: bool,
	/// Input levels at or below this value become pure black (0..255).
	pub black_level: Option<u8>,
	/// Gamma to apply after the black level (`> 0`; 2.0 brightens midtones).
	pub gamma: Option<f32>,
	/// Encoded format of the transformed pages.
	pub format: TransformFormat,
	/// Split pages taller than `MAX_TALL_PAGE_RATIO`× their width into
	/// separate panels.
	pub split_tall_pages: bool,
	/// Container produced for the transformed comic.
	pub container: ComicContainer,
}

/// Pages with a height more than this multiple of their width are split when
/// `split_tall_pages` is enabled.
pub const MAX_TALL_PAGE_RATIO: f32 = 3.0;

impl TransformProfile {
	/// Resolve the profile named `name`, or `None` for an unknown name.
	///
	/// Preset names are stable, lowercase identifiers: `clara`, `libra`,
	/// `sage`, `elipsa`, `nia`, `clara-colour`, `libra-colour`, `sage-colour`,
	/// `koreader`, `phone`.
	pub fn preset(name: &str) -> Option<Self> {
		let (max_width, max_height) = match name {
			"clara" => (1072, 1448),
			"libra" => (1264, 1680),
			"sage" => (1440, 1920),
			"elipsa" => (1404, 1872),
			"nia" => (758, 1024),
			"clara-colour" => (1072, 1448),
			"libra-colour" => (1264, 1680),
			"sage-colour" => (1440, 1920),
			"koreader" => (1920, 2560),
			"phone" => return Some(Self::phone()),
			_ => return None,
		};

		let grayscale = !matches!(name, "clara-colour" | "libra-colour" | "sage-colour");
		let format = if grayscale {
			TransformFormat::Jpeg {
				quality: 85,
				subsampling: JpegSubsampling::Auto,
			}
		} else {
			TransformFormat::Jpeg {
				quality: 90,
				subsampling: JpegSubsampling::Auto,
			}
		};

		Some(Self {
			max_width: Some(max_width),
			max_height: Some(max_height),
			grayscale,
			black_level: None,
			gamma: None,
			format,
			split_tall_pages: false,
			container: ComicContainer::KepubFixedLayout,
		})
	}

	/// The `koreader` preset: WebP output (KOReader renders it natively), a
	/// generous size cap, colour preserved, CBZ container.
	pub fn koreader() -> Self {
		Self {
			max_width: Some(1920),
			max_height: Some(2560),
			grayscale: false,
			black_level: None,
			gamma: None,
			format: TransformFormat::Webp { quality: 85 },
			split_tall_pages: false,
			container: ComicContainer::Cbz,
		}
	}

	/// The `phone` preset: WebP, no resize cap, tall pages split into panels.
	pub fn phone() -> Self {
		Self {
			max_width: None,
			max_height: None,
			grayscale: false,
			black_level: None,
			gamma: None,
			format: TransformFormat::Webp { quality: 80 },
			split_tall_pages: true,
			container: ComicContainer::Cbz,
		}
	}

	/// Every preset name accepted by [`TransformProfile::preset`].
	pub fn preset_names() -> &'static [&'static str] {
		&[
			"clara",
			"libra",
			"sage",
			"elipsa",
			"nia",
			"clara-colour",
			"libra-colour",
			"sage-colour",
			"koreader",
			"phone",
		]
	}

	/// Interpret a device's stored `transform_profile` JSON value.
	///
	/// Accepted shapes:
	///
	/// - `null` → `None` (the device has no profile)
	/// - `"libra"` → the named preset
	/// - `{"preset": "clara"}` → the named preset
	/// - any other JSON object → a full [`TransformProfile`] (fields default
	///   individually, so partial overrides are valid)
	pub fn from_device_profile(
		value: &serde_json::Value,
	) -> Option<TransformResult<Self>> {
		match value {
			serde_json::Value::Null => None,
			serde_json::Value::String(name) => Some(
				Self::preset(name).ok_or_else(|| {
					TransformError::Other(format!("unknown transform preset {name:?}"))
				}),
			),
			serde_json::Value::Object(map) => {
				if let Some(serde_json::Value::String(name)) = map.get("preset") {
					return Some(
						Self::preset(name).ok_or_else(|| {
							TransformError::Other(format!(
								"unknown transform preset {name:?}"
							))
						}),
					);
				}
				Some(serde_json::from_value(value.clone()).map_err(|error| {
					TransformError::Other(format!(
						"invalid transform profile: {error}"
					))
				}))
			},
			_ => Some(Err(TransformError::Other(
				"transform profile must be a string, object, or null".to_string(),
			))),
		}
	}

	/// A stable short hash of the profile, used in cache file names.
	///
	/// Any byte-affecting change to the profile (including the container)
	/// yields a different digest, so changing a profile never serves stale
	/// cache entries.
	pub fn digest(&self) -> String {
		use ring::digest::{digest, SHA256};

		let encoded = serde_json::to_vec(self)
			.expect("TransformProfile serializes to JSON");
		let hex = hex_lower(digest(&SHA256, &encoded).as_ref());
		hex[..16].to_string()
	}
}

fn hex_lower(bytes: &[u8]) -> String {
	data_encoding::HEXLOWER.encode(bytes)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn profile_with_width(width: u32) -> TransformProfile {
		TransformProfile {
			max_width: Some(width),
			..Default::default()
		}
	}

	#[test]
	fn kobo_presets_have_expected_dimensions() {
		let expected = [
			("clara", 1072, 1448),
			("libra", 1264, 1680),
			("sage", 1440, 1920),
			("elipsa", 1404, 1872),
			("nia", 758, 1024),
		];
		for (name, width, height) in expected {
			let profile =
				TransformProfile::preset(name).expect("preset should resolve");
			assert_eq!(profile.max_width, Some(width), "width of {name}");
			assert_eq!(profile.max_height, Some(height), "height of {name}");
			assert!(
				profile.grayscale,
				"black-and-white preset {name} should be grayscale"
			);
			assert_eq!(profile.container, ComicContainer::KepubFixedLayout);
		}
	}

	#[test]
	fn colour_presets_keep_rgb() {
		for name in ["clara-colour", "libra-colour", "sage-colour"] {
			let profile =
				TransformProfile::preset(name).expect("colour preset resolves");
			assert!(!profile.grayscale, "{name} must keep colour");
		}
	}

	#[test]
	fn koreader_and_phone_presets() {
		let koreader = TransformProfile::preset("koreader").unwrap();
		assert_eq!(koreader.format, TransformFormat::Webp { quality: 85 });
		assert_eq!(koreader.container, ComicContainer::Cbz);
		assert_eq!(koreader.max_width, Some(1920));

		let phone = TransformProfile::preset("phone").unwrap();
		assert_eq!(phone.max_width, None);
		assert_eq!(phone.max_height, None);
		assert!(phone.split_tall_pages);
		assert_eq!(phone.format, TransformProfile::phone().format);
	}

	#[test]
	fn unknown_preset_is_none() {
		assert!(TransformProfile::preset("kobo").is_none());
		assert!(TransformProfile::preset("").is_none());
		assert!(TransformProfile::preset("LIBRA").is_none());
	}

	#[test]
	fn preset_names_are_complete() {
		assert_eq!(TransformProfile::preset_names().len(), 10);
		for name in TransformProfile::preset_names() {
			assert!(
				TransformProfile::preset(name).is_some(),
				"{name} listed but does not resolve"
			);
		}
	}

	#[test]
	fn device_profile_json_shapes() {
		// null: no profile at all
		assert!(TransformProfile::from_device_profile(&serde_json::json!(null))
			.is_none());

		// string: preset name
		let resolved = TransformProfile::from_device_profile(&serde_json::json!("libra"))
			.expect("string shape yields a result")
			.expect("libra is a preset");
		assert_eq!(resolved, TransformProfile::preset("libra").unwrap());

		// object with a preset key
		let resolved = TransformProfile::from_device_profile(&serde_json::json!({
			"preset": "clara"
		}))
		.expect("object shape yields a result")
		.expect("clara is a preset");
		assert_eq!(resolved.max_width, Some(1072));

		// full object: partial overrides fill with defaults
		let resolved = TransformProfile::from_device_profile(&serde_json::json!({
			"grayscale": false,
			"format": { "type": "webp", "quality": 90 }
		}))
		.expect("object shape yields a result")
		.expect("valid object");
		assert!(!resolved.grayscale);
		assert_eq!(resolved.format, TransformFormat::Webp { quality: 90 });
		assert_eq!(resolved.container, ComicContainer::KepubFixedLayout);

		// invalid values surface as errors, never panic
		assert!(TransformProfile::from_device_profile(&serde_json::json!(42))
			.is_some_and(|result| result.is_err()));
		assert!(TransformProfile::from_device_profile(&serde_json::json!("nope"))
			.is_some_and(|result| result.is_err()));
		assert!(
			TransformProfile::from_device_profile(&serde_json::json!({ "preset": "x" }))
				.is_some_and(|result| result.is_err())
		);
		assert!(
			TransformProfile::from_device_profile(&serde_json::json!({ "max_width": "wide" }))
				.is_some_and(|result| result.is_err())
		);
	}

	#[test]
	fn serde_round_trip() {
		let profile = TransformProfile::preset("libra").unwrap();
		let json = serde_json::to_string(&profile).unwrap();
		let round = serde_json::from_str::<TransformProfile>(&json).unwrap();
		assert_eq!(profile, round);
	}

	#[test]
	fn digest_is_stable_and_input_sensitive() {
		let base = TransformProfile::preset("libra").unwrap();
		assert_eq!(base.digest(), base.digest());
		assert_ne!(base.digest(), TransformProfile::preset("clara").unwrap().digest());

		let mut different_container = base.clone();
		different_container.container = ComicContainer::Cbz;
		assert_ne!(base.digest(), different_container.digest());

		let mut different_level = profile_with_width(1264);
		different_level.black_level = Some(16);
		assert_ne!(profile_with_width(1264).digest(), different_level.digest());
	}
}
