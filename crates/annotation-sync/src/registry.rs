//! The sink catalog: which sinks exist and how to construct one per user.
//!
//! `markdown` is always available; `git` requires the `git` feature (on by
//! default; the headless server links it). Encryption of secret sink settings
//! happens in the host — values arriving here are already decrypted.

use std::path::Path;
use std::sync::Arc;

use stump_api_types::settings::SettingValues;

use crate::error::AnnotationSyncError;
#[cfg(feature = "git")]
use crate::git::{GitSink, GIT_SINK_ID};
use crate::markdown::{MarkdownSink, MARKDOWN_SINK_ID};
use crate::sink::{Sink, SinkDescriptor};

/// Static catalog of every compiled-in sink, for GraphQL and editors.
pub fn catalog() -> Vec<SinkDescriptor> {
	let values = SettingValues::default();
	let mut sinks = Vec::with_capacity(2);
	sinks.push(MarkdownSink::new(Path::new(""), &values).descriptor());
	#[cfg(feature = "git")]
	sinks.push(GitSink::new(Path::new(""), &values).descriptor());
	sinks
}

/// Constructs the sink with `id` for one user from its configured values.
pub fn sink(
	id: &str,
	root: &Path,
	values: &SettingValues,
) -> Result<Arc<dyn Sink>, AnnotationSyncError> {
	match id {
		MARKDOWN_SINK_ID => Ok(Arc::new(MarkdownSink::new(root, values))),
		#[cfg(feature = "git")]
		GIT_SINK_ID => Ok(Arc::new(GitSink::new(root, values))),
		_ => Err(AnnotationSyncError::sink(format!(
			"unknown annotation sink: {id}"
		))),
	}
}
