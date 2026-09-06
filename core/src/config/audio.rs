use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// Audiobook converter defaults. Flattened into [`super::StumpConfig`].
///
/// These are the *server* half of the three audio settings layers: the library
/// half is the `audio` section of `library_configs.metadata_policy`
/// (`stump_ingest::policy::AudioPolicy`), which decides *whether* to assemble,
/// and the device half is the `audio` section of a device's
/// `transform_profile` (`stump_media::transform::AudioProfile`), which decides
/// how a device's *requests* are answered. This group decides what an assemble
/// produces, so that a book has one canonical form however it got there.
///
/// `audio_ffmpeg` is resolved at startup rather than at first use, like
/// `ingest_preprocess_command`: a path typo must stop the server, not fail one
/// conversion per audiobook for the rest of its uptime.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpAudioConfig"))]
pub struct AudioConfig {
	/// Container an assembled audiobook is written as. `m4b` is the only
	/// accepted value: it is the one container that carries a `chpl` atom, a
	/// chapter track, iTunes tags and a cover in a single file, which is what
	/// every audiobook player expects to find.
	///
	/// Opus is deliberately *not* accepted here. It is a first-class input and
	/// an opt-in per-device delivery preset
	/// (`stump_media::transform::AudioOutput::Opus`), never a stored form, so
	/// that turning it on for one phone cannot change what the library holds.
	#[default_value(DEFAULT_AUDIO_CANONICAL.to_string())]
	#[env_key(AUDIO_CANONICAL_KEY)]
	#[validator(validate_audio_canonical)]
	pub audio_canonical: String,

	/// AAC bitrate used when an assemble has to re-encode, as an `ffmpeg`
	/// `-b:a` argument. Only lossy inputs that are not already AAC are
	/// re-encoded; an AAC input is stream-copied, so this value does not
	/// degrade a book that was already in the canonical codec.
	#[default_value(DEFAULT_AUDIO_AAC_BITRATE.to_string())]
	#[env_key(AUDIO_AAC_BITRATE_KEY)]
	#[validator(validate_audio_bitrate)]
	pub audio_aac_bitrate: String,

	/// Absolute path to the `ffmpeg` binary. Empty searches `PATH`, which is
	/// the normal case; a path is for an operator whose `ffmpeg` is not on the
	/// server's `PATH` (a container sidecar mount, a Nix store path).
	///
	/// `ffmpeg` is the one external tool the audio lane needs, and only for
	/// encoding: probing, tagging and chapter writing are pure Rust. It is
	/// never a build dependency.
	#[default_value(DEFAULT_AUDIO_FFMPEG.to_string())]
	#[env_key(AUDIO_FFMPEG_KEY)]
	#[validator(validate_audio_ffmpeg)]
	pub audio_ffmpeg: String,
}

impl AudioConfig {
	/// Refuse a configuration the server could not honour.
	///
	/// The generator's `#[validator]` hooks reject a bad *value* and fall back
	/// to the default, which is right for a cosmetic key and wrong here: an
	/// operator who set `STUMP_AUDIO_FFMPEG` to a typo would get a server
	/// that silently searched `PATH` instead, and one who set
	/// `STUMP_AUDIO_CANONICAL=opus` would get M4B files without being told.
	/// So the group is re-checked at startup and a bad value stops the boot,
	/// exactly as an unresolvable `ingest_preprocess_command` does.
	pub fn validate(&self) -> Result<(), String> {
		if !AUDIO_CANONICAL_VALUES.contains(&self.audio_canonical.as_str()) {
			return Err(format!(
				"{AUDIO_CANONICAL_KEY} ({}) is not a canonical audio container; \
				 expected one of: {}. Opus delivery is a per-device transform \
				 preset, not a canonical container.",
				self.audio_canonical,
				AUDIO_CANONICAL_VALUES.join(", ")
			));
		}
		match stump_media::transform::parse_bitrate(&self.audio_aac_bitrate) {
			Some(bits) if (MIN_AAC_BITRATE..=MAX_AAC_BITRATE).contains(&bits) => {},
			Some(bits) => {
				return Err(format!(
					"{AUDIO_AAC_BITRATE_KEY} ({bits} bits per second) is outside \
					 the supported {MIN_AAC_BITRATE}..={MAX_AAC_BITRATE}"
				))
			},
			None => {
				return Err(format!(
					"{AUDIO_AAC_BITRATE_KEY} ({}) is not a bitrate; expected a \
					 count of bits per second, optionally suffixed with k or M \
					 (e.g. 64k)",
					self.audio_aac_bitrate
				))
			},
		}
		let ffmpeg = self.audio_ffmpeg.trim();
		if !ffmpeg.is_empty() && !is_executable(std::path::Path::new(ffmpeg)) {
			return Err(format!(
				"{AUDIO_FFMPEG_KEY} ({ffmpeg}) is not an executable file; unset \
				 it to search PATH"
			));
		}
		Ok(())
	}
}

/// Containers [`AudioConfig::audio_canonical`] accepts.
pub const AUDIO_CANONICAL_VALUES: &[&str] = &["m4b"];

fn validate_audio_canonical(name: &String) -> bool {
	if AUDIO_CANONICAL_VALUES.contains(&name.as_str()) {
		return true;
	}

	eprintln!(
		"Invalid canonical audio container {name}; expected one of: {}. Opus \
		 delivery is a per-device transform preset, not a canonical container.",
		AUDIO_CANONICAL_VALUES.join(", ")
	);
	false
}

/// AAC below this is unlistenable for speech, and libfdk/aac refuse much
/// lower per channel anyway.
const MIN_AAC_BITRATE: u32 = 8_000;

/// An audiobook has no use for more than this; a value above it is a typo
/// that would multiply every assembled file's size.
const MAX_AAC_BITRATE: u32 = 320_000;

fn validate_audio_bitrate(bitrate: &String) -> bool {
	let Some(bits) = stump_media::transform::parse_bitrate(bitrate) else {
		eprintln!(
			"Invalid AAC bitrate {bitrate}; expected a count of bits per second, \
			 optionally suffixed with k or M (e.g. 64k)"
		);
		return false;
	};
	if !(MIN_AAC_BITRATE..=MAX_AAC_BITRATE).contains(&bits) {
		eprintln!(
			"AAC bitrate {bitrate} is outside the supported \
			 {MIN_AAC_BITRATE}..={MAX_AAC_BITRATE} bits per second"
		);
		return false;
	}
	true
}

fn validate_audio_ffmpeg(path: &String) -> bool {
	let path = path.trim();
	if path.is_empty() {
		return true;
	}
	if is_executable(std::path::Path::new(path)) {
		return true;
	}

	eprintln!(
		"{AUDIO_FFMPEG_KEY} ({path}) is not an executable file; unset it to search PATH"
	);
	false
}

/// Whether `path` is a file this process could execute.
///
/// `stump_tools::external::is_executable` answers the same question, but
/// `stump_tools` exists to drive conversion binaries and carries `epub`,
/// `zip`, `id3` and `mp4ameta` with it; core is compiled into every build
/// including `minimal`, and one permission bit is not worth those crates.
#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
	use std::os::unix::fs::PermissionsExt;

	std::fs::metadata(path)
		.is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
	std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_audio_defaults() {
		let config = AudioConfig::new();
		assert_eq!(config.audio_canonical, "m4b");
		assert_eq!(config.audio_aac_bitrate, "64k");
		assert_eq!(config.audio_ffmpeg, "");
		assert_eq!(AUDIO_CANONICAL_KEY, "STUMP_AUDIO_CANONICAL");
		assert_eq!(AUDIO_AAC_BITRATE_KEY, "STUMP_AUDIO_AAC_BITRATE");
		assert_eq!(AUDIO_FFMPEG_KEY, "STUMP_AUDIO_FFMPEG");
	}

	#[test]
	fn test_audio_canonical_validation() {
		assert!(validate_audio_canonical(&"m4b".to_string()));
		// Opus is delivery-only, so it must be refused here even though the
		// device preset accepts it.
		assert!(!validate_audio_canonical(&"opus".to_string()));
		assert!(!validate_audio_canonical(&"M4B".to_string()));
		assert!(!validate_audio_canonical(&String::new()));
	}

	#[test]
	fn test_audio_bitrate_validation() {
		assert!(validate_audio_bitrate(&"64k".to_string()));
		assert!(validate_audio_bitrate(&"128000".to_string()));
		assert!(validate_audio_bitrate(&"8k".to_string()));
		assert!(validate_audio_bitrate(&"320k".to_string()));
		assert!(!validate_audio_bitrate(&"7k".to_string()));
		assert!(!validate_audio_bitrate(&"1M".to_string()));
		assert!(!validate_audio_bitrate(&"loud".to_string()));
		assert!(!validate_audio_bitrate(&String::new()));
	}

	#[test]
	fn test_audio_ffmpeg_validation() {
		// Empty is the documented "search PATH" value, not a mistake.
		assert!(validate_audio_ffmpeg(&String::new()));
		assert!(validate_audio_ffmpeg(&"   ".to_string()));
		assert!(!validate_audio_ffmpeg(&"/nope/ffmpeg".to_string()));
		// A directory holds a name, not an installation.
		assert!(!validate_audio_ffmpeg(&"/usr/bin".to_string()));
		assert!(validate_audio_ffmpeg(&"/bin/sh".to_string()));
	}

	/// The startup gate, not the generator hook: an operator who mistypes a
	/// key must get a stopped server rather than one that quietly used the
	/// default and told them in a line they scrolled past.
	#[test]
	fn test_audio_startup_validation_is_fatal_per_key() {
		assert!(AudioConfig::new().validate().is_ok());

		let with = |mutate: fn(&mut AudioConfig)| {
			let mut config = AudioConfig::new();
			mutate(&mut config);
			config.validate()
		};

		let error = with(|config| config.audio_canonical = "opus".to_string())
			.expect_err("opus is delivery-only");
		assert!(error.contains(AUDIO_CANONICAL_KEY), "{error}");
		assert!(error.contains("per-device transform"), "{error}");

		assert!(with(|config| config.audio_aac_bitrate = "999k".to_string()).is_err());
		assert!(with(|config| config.audio_aac_bitrate = "loud".to_string()).is_err());
		assert!(with(|config| config.audio_aac_bitrate = "8k".to_string()).is_ok());

		assert!(with(|config| config.audio_ffmpeg = "/nope/ffmpeg".to_string()).is_err());
		// Empty is the documented "search PATH" value on a host with no
		// ffmpeg at all, so it must never stop a boot.
		assert!(with(|config| config.audio_ffmpeg = "  ".to_string()).is_ok());
	}
}
