//! EPUB → AZW3 conversion through the `boko-convert` tool.
//!
//! The conversion is never a hard failure of the e-mail lane: an operator
//! without `boko` still delivers a readable book, because Amazon converts an
//! EPUB itself. Every reason a conversion did not happen therefore comes back
//! as [`Conversion::Skipped`] carrying the sentence to show the operator, and
//! only the USB lane turns that into an error (see
//! [`FormatPolicy`](crate::FormatPolicy)).

use std::{
	future::Future,
	path::{Path, PathBuf},
};

use stump_tools::{
	boko::{BokoConvert, KindleExport},
	NoopProgress, Tool,
};

/// What the conversion step produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conversion {
	/// The AZW3 boko wrote, inside the caller's working directory.
	Converted(PathBuf),
	/// Nothing was converted; the reason is reported on the delivery and the
	/// book is used as it is.
	Skipped(String),
}

/// EPUB to AZW3 conversion. Implemented by [`BokoConverter`] in production and
/// by the tests' fakes, which is what lets the lane be tested on a machine
/// that has no `boko` installed (the common case).
///
/// Desugared rather than written as `async fn` so the returned future is
/// explicitly `Send`: the lane runs inside an Axum handler
/// (`POST /api/v2/media/{id}/kindle-file`), whose future must be.
pub trait KindleConverter {
	fn to_azw3(
		&self,
		source: &Path,
		out_dir: &Path,
	) -> impl Future<Output = Conversion> + Send;
}

/// The real converter: `stump_tools`' `boko-convert`, pinned to the
/// EPUB → AZW3 pair by [`KindleExport`].
pub struct BokoConverter;

impl KindleConverter for BokoConverter {
	async fn to_azw3(&self, source: &Path, out_dir: &Path) -> Conversion {
		let source = source.to_path_buf();
		let out_dir = out_dir.to_path_buf();
		// `Tool::plan` and `Tool::apply` locate and run a child process, so
		// the whole conversion is one blocking unit off the async runtime.
		match tokio::task::spawn_blocking(move || convert_with_boko(&source, &out_dir))
			.await
		{
			Ok(conversion) => conversion,
			Err(error) => Conversion::Skipped(format!(
				"the conversion task did not finish: {error}"
			)),
		}
	}
}

fn convert_with_boko(source: &Path, out_dir: &Path) -> Conversion {
	let input = match KindleExport::input(
		vec![source.to_path_buf()],
		Some(out_dir.to_path_buf()),
	) {
		Ok(input) => input,
		Err(error) => return Conversion::Skipped(error.to_string()),
	};

	let tool = BokoConvert;
	// The expected failure is `ExternalToolMissing`: most servers have no
	// boko, which is exactly what the EPUB fallback exists for.
	let plan = match tool.plan(&input) {
		Ok(plan) => plan,
		Err(error) => return Conversion::Skipped(error.to_string()),
	};
	let Some(target) = plan
		.actions
		.first()
		.and_then(|action| action.target.clone())
	else {
		return Conversion::Skipped(format!(
			"boko planned no conversion: {}",
			describe(plan.warnings.iter().map(|warning| warning.message.clone()))
		));
	};

	match tool.apply(&plan, &mut NoopProgress) {
		Ok(report) if report.applied.len() == 1 && target.is_file() => {
			Conversion::Converted(target)
		},
		Ok(report) => Conversion::Skipped(format!(
			"boko converted nothing: {}",
			describe(report.skipped.into_iter().map(|(_, reason)| reason))
		)),
		Err(error) => Conversion::Skipped(error.to_string()),
	}
}

fn describe(reasons: impl Iterator<Item = String>) -> String {
	let joined = reasons.collect::<Vec<_>>().join("; ");
	if joined.is_empty() {
		"no reason given".to_string()
	} else {
		joined
	}
}
