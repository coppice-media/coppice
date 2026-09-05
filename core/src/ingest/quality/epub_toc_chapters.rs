use std::{
	collections::BTreeSet,
	io::Cursor,
	path::{Component, Path, PathBuf},
};

use quick_xml::{events::Event, Reader};
use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_media::EpubProcessor;

use crate::ingest::contract::{
	BookSnapshot, IngestMediaKind, QualityCheck, QualityCheckError, QualityStatus,
};

use super::{disabled_outcome, enabled_setting, outcome, QUALITY_VERSION};

/// Verifies EPUB table of contents chapters.
#[derive(Default)]
pub struct EpubTocChaptersCheck;

impl EpubTocChaptersCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for EpubTocChaptersCheck {
	fn id(&self) -> &'static str {
		"epub_toc_chapters"
	}

	fn name(&self) -> &'static str {
		"EPUB table of contents chapters"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		15
	}

	fn settings(&self) -> &[SettingDefinition] {
		super::enabled_settings()
	}

	async fn run(
		&self,
		book: &BookSnapshot,
		settings: &SettingValues,
	) -> Result<crate::ingest::contract::QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}
		if !matches!(book.media_kind, IngestMediaKind::Epub) {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({"applicable": false}),
			));
		}

		let path = &book.staged_path;
		let document = match EpubProcessor::open(path.to_string_lossy().as_ref()) {
			Ok(document) => document,
			Err(_) => {
				return Ok(outcome(
					self.id(),
					self.name(),
					QualityStatus::Fail,
					0.0,
					serde_json::json!({"package_valid": false, "spine_count": 0, "toc_entries": 0}),
				));
			},
		};
		let spine_count = document.spine.len();
		if spine_count == 0 {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				serde_json::json!({"package_valid": true, "spine_count": 0, "toc_entries": 0}),
			));
		}

		let spine_paths = document
			.spine
			.iter()
			.filter_map(|item| document.resources.get(&item.idref))
			.map(|resource| normalize_path(&resource.path))
			.collect::<BTreeSet<_>>();
		let mut toc_entries = flatten_toc(&document.toc)
			.into_iter()
			.filter(|path| resolves_spine(path, &spine_paths))
			.count();
		let mut nav_entries = 0usize;
		let mut nav_parse_failed = false;
		if let Some(nav_id) = document.get_nav_id() {
			if let Some(nav_resource) = document.resources.get(&nav_id) {
				nav_entries = match EpubProcessor::get_resource_by_id(
					path.to_string_lossy().as_ref(),
					&nav_id,
				) {
					Ok((_, bytes)) => {
						match nav_spine_entries(&bytes, &nav_resource.path, &spine_paths)
						{
							Ok(count) => count,
							Err(_) => {
								nav_parse_failed = true;
								0
							},
						}
					},
					Err(_) => {
						nav_parse_failed = true;
						0
					},
				};
			}
		}
		toc_entries += nav_entries;
		let evidence = serde_json::json!({
			"package_valid": true,
			"spine_count": spine_count,
			"toc_entries": toc_entries,
			"nav_entries": nav_entries,
			"ncx_entries": document.toc.len(),
		});
		if nav_parse_failed {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				evidence,
			));
		}
		if toc_entries > 0 {
			Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Pass,
				1.0,
				evidence,
			))
		} else {
			Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Warn,
				0.5,
				evidence,
			))
		}
	}
}

fn flatten_toc(points: &[epub::doc::NavPoint]) -> Vec<PathBuf> {
	fn visit(point: &epub::doc::NavPoint, paths: &mut Vec<PathBuf>) {
		paths.push(strip_fragment(&point.content));
		for child in &point.children {
			visit(child, paths);
		}
	}
	let mut paths = Vec::new();
	for point in points {
		visit(point, &mut paths);
	}
	paths
}

fn nav_spine_entries(
	bytes: &[u8],
	nav_path: &Path,
	spine_paths: &BTreeSet<String>,
) -> Result<usize, quick_xml::Error> {
	let base = nav_path.parent().unwrap_or_else(|| Path::new(""));
	let mut reader = Reader::from_reader(Cursor::new(bytes));
	let mut buffer = Vec::new();
	let mut resolved = 0;
	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(event) | Event::Empty(event)
				if event.name().as_ref().eq_ignore_ascii_case(b"a") =>
			{
				for attribute in event.attributes() {
					let attribute = attribute?;
					if attribute.key.as_ref().eq_ignore_ascii_case(b"href") {
						let value = attribute.unescape_value()?.into_owned();
						let target = base.join(strip_fragment(Path::new(&value)));
						if resolves_spine(&target, spine_paths) {
							resolved += 1;
						}
					}
				}
			},
			Event::Eof => break,
			_ => {},
		}
		buffer.clear();
	}
	Ok(resolved)
}

fn strip_fragment(path: &Path) -> PathBuf {
	let value = path.to_string_lossy();
	PathBuf::from(value.split(['#', '?']).next().unwrap_or_default())
}

fn normalize_path(path: &Path) -> String {
	let mut normalized = PathBuf::new();
	for component in path.components() {
		match component {
			Component::Normal(part) => normalized.push(part),
			Component::CurDir => {},
			Component::ParentDir => {
				normalized.pop();
			},
			_ => {},
		}
	}
	normalized.to_string_lossy().replace('\\', "/")
}

fn resolves_spine(path: &Path, spine_paths: &BTreeSet<String>) -> bool {
	let normalized = normalize_path(path);
	if spine_paths.contains(&normalized) {
		return true;
	}
	spine_paths.iter().any(|spine| {
		spine.ends_with(&format!("/{normalized}"))
			|| normalized.ends_with(&format!("/{spine}"))
	})
}
