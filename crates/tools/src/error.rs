//! The single error type every tool returns. See `crates/tools/README.md`.

/// Every failure surfaced by a [`crate::Tool`].
///
/// Tools report *recoverable* per-file problems as [`crate::Warning`]s (plan
/// stage) or [`crate::Report::skipped`] entries (apply stage); a `ToolError` is
/// reserved for failures that abort the whole plan or apply run.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
	/// No tool in [`crate::registry`] has the requested id.
	#[error("unknown tool: {0}")]
	UnknownTool(String),
	/// The caller passed paths that cannot be used (missing, wrong kind, empty).
	#[error("invalid input: {0}")]
	Invalid(String),
	/// The `options` JSON blob is malformed for this tool.
	#[error("invalid options: {0}")]
	Options(String),
	/// A [`crate::Plan`] built by one tool was handed to another.
	#[error("plan was built by tool {plan:?}, cannot be applied by {tool:?}")]
	PlanMismatch { tool: String, plan: String },
	/// A required external binary is absent, not executable, or too old.
	/// `hint` is operator-facing install/config advice.
	#[error("{tool} is unavailable: {reason} ({hint})")]
	ExternalToolMissing {
		tool: String,
		reason: String,
		hint: String,
	},
	#[error("{0}")]
	Io(#[from] std::io::Error),
	#[error("{0}")]
	Json(#[from] serde_json::Error),
	#[error("{0}")]
	Media(#[from] stump_media::error::FileError),
	#[error("{0}")]
	Transform(#[from] stump_media::transform::TransformError),
	#[error("{0}")]
	Archive(#[from] zip::result::ZipError),
	/// An MP4/M4B container `audio-chapters` could not read or rewrite.
	#[error("{0}")]
	Mp4(#[from] mp4ameta::Error),
	/// An MP4 sample table `audio-assemble` could not demux or mux. Distinct
	/// from [`Self::Mp4`]: that is the tagger failing on a container's
	/// metadata, this is the muxer failing on its media.
	#[error("{0}")]
	Mux(#[from] mp4::Error),
	/// An ID3v2 tag `audio-chapters` could not read or rewrite.
	#[error("{0}")]
	Id3(#[from] id3::Error),
	#[error("{0}")]
	Xml(#[from] quick_xml::Error),
}

pub type ToolResult<T> = Result<T, ToolError>;
