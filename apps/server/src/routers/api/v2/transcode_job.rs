//! The server's own implementation of the `transcode` job kind.
//!
//! This is the *local fallback* the design names: when no worker advertises
//! `transcode`, the server does it itself with the operator-installed `ffmpeg`,
//! through the same [`stump_tools::ffmpeg`] adapter the worker binary uses. One
//! argv builder, two hosts, identical output — so a cache entry does not depend
//! on who filled it, and a deployment can move the work to a worker (or back)
//! without invalidating anything.
//!
//! When the server has no `ffmpeg` either, the runner fails and the delivery
//! route degrades to the stored bytes, exactly as it did before workers
//! existed. A VPS with no `ffmpeg` and a connected worker gets Opus; a VPS with
//! neither gets its MP3. Nothing in between is a broken response.

use std::path::PathBuf;
use std::sync::Arc;

use models::entity::media_audio_track;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::Value;
use stump_tools::ffmpeg::{self, Codec, Encode};
use stump_worker::{
	kind::{LocalJob, LocalRunner, TranscodeInput, TranscodeOutput, TranscodeResult},
	KindRegistry, TRANSCODE,
};

use crate::config::state::AppState;

/// The registry the host installs at boot: `transcode` runs here when nobody
/// else can, and nothing else has a local implementation yet.
pub(crate) fn registry(ctx: &AppState) -> KindRegistry {
	KindRegistry::new().with_local(
		TRANSCODE,
		Arc::new(TranscodeLocalRunner {
			conn: ctx.conn.clone(),
			bin_dir: ffmpeg_bin_dir(ctx),
		}) as Arc<dyn LocalRunner>,
	)
}

struct TranscodeLocalRunner {
	conn: Arc<DatabaseConnection>,
	/// The directory `ffmpeg` is looked for in, from `STUMP_AUDIO_FFMPEG`.
	bin_dir: Option<PathBuf>,
}

#[async_trait::async_trait]
impl LocalRunner for TranscodeLocalRunner {
	async fn run(&self, job: LocalJob) -> Result<Value, String> {
		let input: TranscodeInput = serde_json::from_value(job.input)
			.map_err(|error| format!("invalid transcode input: {error}"))?;

		let track = media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.eq(&input.media_id))
			.filter(media_audio_track::Column::Index.eq(input.track_index))
			.one(self.conn.as_ref())
			.await
			.map_err(|error| error.to_string())?
			.ok_or_else(|| "the track no longer exists".to_string())?;

		let bin_dir = self.bin_dir.clone();
		let output_path = job.output_path.clone();
		let target = input.output.clone();
		let duration_ms = if input.duration_ms > 0 {
			input.duration_ms
		} else {
			track.duration_ms
		};
		let source = PathBuf::from(track.path);

		let bytes = tokio::task::spawn_blocking(move || {
			encode(
				&source,
				&output_path,
				&target,
				duration_ms,
				bin_dir.as_deref(),
			)
		})
		.await
		.map_err(|error| format!("transcode task panicked: {error}"))??;

		Ok(serde_json::to_value(TranscodeResult {
			bytes,
			// The local path writes straight into the destination, so there is
			// nothing to verify a digest against; it is reported so the two
			// result shapes are one shape.
			sha256: String::new(),
			mime: input.output.mime().to_string(),
		})
		.unwrap_or(Value::Null))
	}
}

/// Encode into a sibling temp file, then rename onto `output_path`.
///
/// Staging in the *same* directory keeps the rename within one filesystem, so
/// it is atomic and a killed run never leaves a truncated file for the
/// awaiting caller to publish into the delivery cache.
fn encode(
	source: &std::path::Path,
	output_path: &std::path::Path,
	target: &TranscodeOutput,
	duration_ms: i64,
	bin_dir: Option<&std::path::Path>,
) -> Result<u64, String> {
	let parent = output_path
		.parent()
		.ok_or_else(|| "the output path has no directory".to_string())?;
	std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;

	let install = ffmpeg::locate(bin_dir).map_err(|error| error.to_string())?;

	let staging = tempfile::Builder::new()
		.prefix("worker-transcode-")
		.tempdir_in(parent)
		.map_err(|error| error.to_string())?;
	// `ffmpeg` reads the muxer off the output path, so the staged name carries
	// the real extension rather than a `.tmp` suffix.
	let staged = staging.path().join(format!("track.{}", target.extension()));

	let inputs = [source.to_path_buf()];
	let output = ffmpeg::encode(
		&install,
		&Encode {
			inputs: &inputs,
			output: &staged,
			codec: match target {
				TranscodeOutput::Opus { .. } => Codec::Opus,
				TranscodeOutput::Aac { .. } => Codec::Aac,
			},
			bitrate: Some(target.bitrate()),
			metadata: None,
			cover: None,
			// MP4-only; an Ogg stream has no `moov` atom to move.
			faststart: matches!(target, TranscodeOutput::Aac { .. }),
			timeout: ffmpeg::timeout_for(duration_ms),
		},
	)
	.map_err(|error| error.to_string())?;

	if !output.succeeded() {
		return Err(format!(
			"ffmpeg {}: {}",
			output.describe_status(),
			output.stderr.trim()
		));
	}

	let bytes = std::fs::metadata(&staged)
		.map_err(|error| error.to_string())?
		.len();
	std::fs::rename(&staged, output_path).map_err(|error| error.to_string())?;
	Ok(bytes)
}

/// The config holds the binary's own path (validated at startup) and
/// `ffmpeg::locate` searches a *directory*, so the parent is what it wants;
/// empty means "search `PATH`".
fn ffmpeg_bin_dir(ctx: &AppState) -> Option<PathBuf> {
	let configured = ctx.config.audio.audio_ffmpeg.trim();
	if configured.is_empty() {
		return None;
	}
	std::path::Path::new(configured)
		.parent()
		.map(std::path::Path::to_path_buf)
}
