//! Deterministic preparation and EPUB 3 Media Overlay rendering for read-aloud.
//!
//! The preparation pass is the server-owned contract shared with an alignment
//! worker: every linear XHTML spine item is segmented into stable block targets,
//! and the canonical text digest includes those targets and normalized text.
//! A renderer accepts only targets produced by that pass. It never trusts an
//! element id returned by an external aligner merely because it looks plausible.

use std::{
	collections::BTreeMap,
	fs::File,
	io::{Read, Write},
	path::{Path, PathBuf},
};

use quick_xml::{
	events::{BytesEnd, BytesStart, Event},
	Reader, Writer,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};
/// segmentation or the derivative envelope changes.
pub const PREPARATION_VERSION: &str = "coppice-read-aloud-prep-v1";
/// Relative directory used for generated SMIL resources.
pub const SMIL_DIRECTORY: &str = "coppice-smil";
/// Relative directory used for the retained single-file audiobook.
pub const AUDIO_DIRECTORY: &str = "coppice-audio";

#[derive(Debug, Error)]
pub enum ReadAloudError {
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
	#[error("EPUB ZIP error: {0}")]
	Zip(#[from] zip::result::ZipError),
	#[error("EPUB XML error: {0}")]
	Xml(String),
	#[error("invalid EPUB: {0}")]
	Invalid(String),
	#[error("read-aloud target is not in the prepared XHTML: spine {spine_index}, ordinal {ordinal}, id {element_id}")]
	UnknownTarget {
		spine_index: i32,
		ordinal: i32,
		element_id: String,
	},
	#[error("read-aloud cache path collides with an existing EPUB entry: {0}")]
	EntryCollision(String),
}

/// One server-owned XHTML segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTarget {
	pub spine_index: i32,
	pub ordinal: i32,
	pub element_id: String,
	pub text: String,
}

/// A linear XHTML spine item after deterministic target injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSpine {
	pub spine_index: i32,
	pub package_path: String,
	pub media_type: String,
	pub bytes: Vec<u8>,
	pub targets: Vec<PreparedTarget>,
}

/// Immutable preparation output used by both validation and rendering.
#[derive(Debug, Clone)]
pub struct PreparedEpub {
	pub opf_path: String,
	pub entries: BTreeMap<String, Vec<u8>>,
	pub spines: Vec<PreparedSpine>,
	pub canonical_text_digest: String,
}

/// A map cue in renderer-neutral form. Keeping this small avoids making the
/// media crate depend on the worker transport crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderCue {
	pub spine_index: i32,
	pub ordinal: i32,
	pub element_id: String,
	pub track_index: i32,
	pub begin_ms: i64,
	pub end_ms: i64,
}

/// Result metadata for a deterministic derivative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedReadAloud {
	pub path: PathBuf,
	pub sha256: String,
	pub bytes: u64,
}

/// Prepare one EPUB and inject deterministic target ids into its linear spine.
pub fn prepare_epub(path: impl AsRef<Path>) -> Result<PreparedEpub, ReadAloudError> {
	let file = File::open(path)?;
	let mut archive = ZipArchive::new(file)?;
	let mut entries = read_entries(&mut archive)?;
	let container_path =
		find_entry(&entries, "META-INF/container.xml").ok_or_else(|| {
			ReadAloudError::Invalid("META-INF/container.xml is missing".into())
		})?;
	let opf_path = parse_rootfile(&entries[container_path])?;
	let opf_bytes = entries
		.get(&opf_path)
		.ok_or_else(|| {
			ReadAloudError::Invalid(format!("OPF entry is missing: {opf_path}"))
		})?
		.clone();
	let package = parse_package(&opf_bytes)?;
	if package.spine.is_empty() {
		return Err(ReadAloudError::Invalid("EPUB spine is empty".into()));
	}

	let mut spines = Vec::with_capacity(package.spine.len());
	let mut canonical = String::new();
	for (spine_index, idref) in package.spine.iter().enumerate() {
		let item = package
			.manifest
			.iter()
			.find(|item| item.id == *idref)
			.ok_or_else(|| {
				ReadAloudError::Invalid(format!("spine idref is missing: {idref}"))
			})?;
		let package_path = resolve_package_path(&opf_path, &item.href);
		let original = entries
			.get(&package_path)
			.ok_or_else(|| {
				ReadAloudError::Invalid(format!(
					"spine resource is missing: {package_path}"
				))
			})?
			.clone();
		let (bytes, targets) = segment_xhtml(&original, spine_index as i32)?;
		for target in &targets {
			canonical.push_str(&target.spine_index.to_string());
			canonical.push('\t');
			canonical.push_str(&target.ordinal.to_string());
			canonical.push('\t');
			canonical.push_str(&target.element_id);
			canonical.push('\t');
			canonical.push_str(&target.text);
			canonical.push('\n');
		}
		entries.insert(package_path.clone(), bytes.clone());
		spines.push(PreparedSpine {
			spine_index: spine_index as i32,
			package_path,
			media_type: item.media_type.clone(),
			bytes,
			targets,
		});
	}

	let canonical_text_digest =
		sha256_hex(format!("{PREPARATION_VERSION}\n{opf_path}\n{canonical}").as_bytes());
	Ok(PreparedEpub {
		opf_path,
		entries,
		spines,
		canonical_text_digest,
	})
}

/// Return whether a cue points at exactly one prepared target.
pub fn contains_target(prepared: &PreparedEpub, cue: &RenderCue) -> bool {
	prepared.spines.iter().any(|spine| {
		spine.spine_index == cue.spine_index
			&& spine.targets.iter().any(|target| {
				target.ordinal == cue.ordinal && target.element_id == cue.element_id
			})
	})
}

/// Serialize the prepared source EPUB with deterministic ZIP metadata. The
/// Storyteller worker uploads this exact representation, so its returned
/// element ids are from the same server-owned target set later validated here.
pub fn prepared_epub_bytes(prepared: &PreparedEpub) -> Result<Vec<u8>, ReadAloudError> {
	let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
	{
		let mut zip = ZipWriter::new(&mut cursor);
		let options: SimpleFileOptions = SimpleFileOptions::default()
			.compression_method(CompressionMethod::Stored)
			.last_modified_time(DateTime::default())
			.unix_permissions(0o644);
		if !prepared.entries.contains_key("mimetype") {
			return Err(ReadAloudError::Invalid("EPUB has no mimetype entry".into()));
		}
		zip.start_file("mimetype", options)?;
		zip.write_all(b"application/epub+zip")?;
		for (name, bytes) in &prepared.entries {
			if name == "mimetype" {
				continue;
			}
			zip.start_file(name.as_str(), options)?;
			zip.write_all(bytes)?;
		}
		zip.finish()?;
	}
	Ok(cursor.into_inner())
}

/// Validate every map cue against the server-owned prepared target set.
pub fn validate_targets(
	prepared: &PreparedEpub,
	cues: &[RenderCue],
) -> Result<(), ReadAloudError> {
	for cue in cues {
		if !contains_target(prepared, cue) {
			return Err(ReadAloudError::UnknownTarget {
				spine_index: cue.spine_index,
				ordinal: cue.ordinal,
				element_id: cue.element_id.clone(),
			});
		}
	}
	Ok(())
}

/// Render and atomically publish an EPUB 3 Media Overlay derivative.
///
/// The source audiobook is copied byte-for-byte into the derivative when it is
/// a single-file input. All ZIP timestamps, ordering, and compression choices
/// are fixed so repeated renders produce identical bytes.
pub fn render_to_path(
	prepared: &PreparedEpub,
	audio_path: impl AsRef<Path>,
	cues: &[RenderCue],
	destination: impl AsRef<Path>,
) -> Result<RenderedReadAloud, ReadAloudError> {
	if prepared.spines.is_empty() {
		return Err(ReadAloudError::Invalid("prepared EPUB has no spine".into()));
	}
	let audio_path = audio_path.as_ref();
	let audio_bytes = std::fs::read(audio_path)?;
	let opf_parent = Path::new(&prepared.opf_path)
		.parent()
		.map(Path::to_path_buf)
		.unwrap_or_default();
	let audio_entry =
		join_package_path(&opf_parent, &format!("{AUDIO_DIRECTORY}/book.m4b"));
	if prepared.entries.contains_key(&audio_entry) {
		return Err(ReadAloudError::EntryCollision(audio_entry));
	}

	let mut all_cues = BTreeMap::<i32, Vec<&RenderCue>>::new();
	for cue in cues {
		if cue.track_index != 0 || !contains_target(prepared, cue) {
			if !contains_target(prepared, cue) {
				return Err(ReadAloudError::UnknownTarget {
					spine_index: cue.spine_index,
					ordinal: cue.ordinal,
					element_id: cue.element_id.clone(),
				});
			}
			return Err(ReadAloudError::Invalid(
				"only track 0 is supported by the single-file read-aloud renderer".into(),
			));
		}
		all_cues.entry(cue.spine_index).or_default().push(cue);
	}

	let mut entries = prepared.entries.clone();
	let mut smil_items = Vec::new();
	for spine in &prepared.spines {
		let smil_entry = join_package_path(
			&opf_parent,
			&format!("{SMIL_DIRECTORY}/{:04}.smil", spine.spine_index),
		);
		if entries.contains_key(&smil_entry) {
			return Err(ReadAloudError::EntryCollision(smil_entry));
		}
		let smil = smil_for_spine(
			prepared,
			spine,
			all_cues
				.get(&spine.spine_index)
				.cloned()
				.unwrap_or_default(),
			&audio_entry,
			&smil_entry,
		)?;
		entries.insert(smil_entry.clone(), smil);
		smil_items.push((spine, smil_entry));
	}
	entries.insert(audio_entry.clone(), audio_bytes);
	let opf = rewrite_opf(prepared, &entries, &smil_items, &audio_entry)?;
	entries.insert(prepared.opf_path.clone(), opf);

	let destination = destination.as_ref();
	let parent = destination
		.parent()
		.ok_or_else(|| ReadAloudError::Invalid("derivative path has no parent".into()))?;
	std::fs::create_dir_all(parent)?;
	let staging = parent.join(format!(
		".{}.part",
		destination
			.file_name()
			.and_then(|n| n.to_str())
			.unwrap_or("read-aloud")
	));
	write_deterministic_zip(&entries, &staging)?;
	std::fs::rename(&staging, destination)?;
	let bytes = std::fs::read(destination)?;
	let sha256 = sha256_hex(&bytes);
	Ok(RenderedReadAloud {
		path: destination.to_path_buf(),
		sha256,
		bytes: bytes.len() as u64,
	})
}

/// A stable cache identity for a map and its immutable inputs.
pub fn cache_key(
	text_digest: &str,
	audio_manifest_digest: &str,
	map_bytes: &[u8],
) -> String {
	let mut hasher = Sha256::new();
	hasher.update(PREPARATION_VERSION.as_bytes());
	hasher.update([0]);
	hasher.update(text_digest.as_bytes());
	hasher.update([0]);
	hasher.update(audio_manifest_digest.as_bytes());
	hasher.update([0]);
	hasher.update(map_bytes);
	hex_encode(&hasher.finalize())
}

fn read_entries<R: Read + std::io::Seek>(
	archive: &mut ZipArchive<R>,
) -> Result<BTreeMap<String, Vec<u8>>, ReadAloudError> {
	let mut entries = BTreeMap::new();
	for index in 0..archive.len() {
		let mut entry = archive.by_index(index)?;
		if entry.is_dir() {
			continue;
		}
		let name = normalize_path(entry.name());
		if name.is_empty() || name.starts_with("../") || name.contains("/../") {
			return Err(ReadAloudError::Invalid(format!(
				"unsafe EPUB entry: {}",
				entry.name()
			)));
		}
		let mut bytes = Vec::new();
		entry.read_to_end(&mut bytes)?;
		entries.insert(name, bytes);
	}
	Ok(entries)
}

fn find_entry<'a>(
	entries: &'a BTreeMap<String, Vec<u8>>,
	wanted: &str,
) -> Option<&'a String> {
	let wanted = normalize_path(wanted);
	entries
		.keys()
		.find(|name| name.eq_ignore_ascii_case(&wanted))
}

fn parse_rootfile(bytes: &[u8]) -> Result<String, ReadAloudError> {
	let mut reader = Reader::from_reader(bytes);
	reader.config_mut().trim_text(true);
	let mut buffer = Vec::new();
	loop {
		match reader.read_event_into(&mut buffer) {
			Ok(Event::Start(event)) | Ok(Event::Empty(event))
				if local_name(event.name().as_ref()) == "rootfile" =>
			{
				for attr in event.attributes().with_checks(false) {
					let attr = attr.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
					if local_name(attr.key.as_ref()) == "full-path" {
						return Ok(normalize_path(&String::from_utf8_lossy(&attr.value)));
					}
				}
			},
			Ok(Event::Eof) => break,
			Err(error) => return Err(ReadAloudError::Xml(error.to_string())),
			_ => {},
		}
		buffer.clear();
	}
	Err(ReadAloudError::Invalid(
		"container has no rootfile full-path".into(),
	))
}

#[derive(Debug, Clone)]
struct ManifestItem {
	id: String,
	href: String,
	media_type: String,
}

#[derive(Debug, Clone)]
struct Package {
	manifest: Vec<ManifestItem>,
	spine: Vec<String>,
}

fn parse_package(bytes: &[u8]) -> Result<Package, ReadAloudError> {
	let mut reader = Reader::from_reader(bytes);
	reader.config_mut().trim_text(true);
	let mut buffer = Vec::new();
	let mut manifest = Vec::new();
	let mut spine = Vec::new();
	loop {
		match reader.read_event_into(&mut buffer) {
			Ok(Event::Start(event)) | Ok(Event::Empty(event)) => {
				match local_name(event.name().as_ref()) {
					"item" => {
						let mut id = None;
						let mut href = None;
						let mut media_type = None;
						for attr in event.attributes().with_checks(false) {
							let attr =
								attr.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
							match local_name(attr.key.as_ref()) {
								"id" => {
									id = Some(
										String::from_utf8_lossy(&attr.value).into_owned(),
									)
								},
								"href" => {
									href = Some(
										String::from_utf8_lossy(&attr.value).into_owned(),
									)
								},
								"media-type" => {
									media_type = Some(
										String::from_utf8_lossy(&attr.value).into_owned(),
									)
								},
								_ => {},
							}
						}
						if let (Some(id), Some(href), Some(media_type)) =
							(id, href, media_type)
						{
							manifest.push(ManifestItem {
								id,
								href,
								media_type,
							});
						}
					},
					"itemref" => {
						for attr in event.attributes().with_checks(false) {
							let attr =
								attr.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
							if local_name(attr.key.as_ref()) == "idref" {
								spine.push(
									String::from_utf8_lossy(&attr.value).into_owned(),
								);
								break;
							}
						}
					},
					_ => {},
				}
			},
			Ok(Event::Eof) => break,
			Err(error) => return Err(ReadAloudError::Xml(error.to_string())),
			_ => {},
		}
		buffer.clear();
	}
	Ok(Package { manifest, spine })
}

fn segment_xhtml(
	bytes: &[u8],
	spine_index: i32,
) -> Result<(Vec<u8>, Vec<PreparedTarget>), ReadAloudError> {
	let mut reader = Reader::from_reader(bytes);
	reader.config_mut().trim_text(false);
	let mut writer = Writer::new(Vec::new());
	let mut buffer = Vec::new();
	let mut active = Vec::<(i32, String, String)>::new();
	let mut targets = Vec::new();
	let mut next_ordinal = 0_i32;
	loop {
		match reader.read_event_into(&mut buffer) {
			Ok(Event::Start(event)) => {
				let candidate = is_segment_element(event.name().as_ref());
				let target = if candidate {
					let ordinal = next_ordinal;
					next_ordinal += 1;
					let id = format!("coppice-{spine_index}-{ordinal}");
					let element_name =
						String::from_utf8_lossy(event.name().as_ref()).into_owned();
					let mut start = BytesStart::new(element_name);
					for attr in event.attributes().with_checks(false) {
						let attr =
							attr.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
						if local_name(attr.key.as_ref()) != "id" {
							start
								.push_attribute((attr.key.as_ref(), attr.value.as_ref()));
						}
					}
					start.push_attribute(("id", id.as_str()));
					writer
						.write_event(Event::Start(start.to_owned()))
						.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
					active.push((ordinal, id.clone(), String::new()));
					Some((ordinal, id))
				} else {
					writer
						.write_event(Event::Start(event.to_owned()))
						.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
					None
				};
				let _ = target;
			},
			Ok(Event::Empty(event)) => {
				if is_segment_element(event.name().as_ref()) {
					let ordinal = next_ordinal;
					next_ordinal += 1;
					let id = format!("coppice-{spine_index}-{ordinal}");
					let element_name =
						String::from_utf8_lossy(event.name().as_ref()).into_owned();
					let mut start = BytesStart::new(element_name);
					for attr in event.attributes().with_checks(false) {
						let attr =
							attr.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
						if local_name(attr.key.as_ref()) != "id" {
							start
								.push_attribute((attr.key.as_ref(), attr.value.as_ref()));
						}
					}
					start.push_attribute(("id", id.as_str()));
					writer
						.write_event(Event::Empty(start.to_owned()))
						.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
					targets.push(PreparedTarget {
						spine_index,
						ordinal,
						element_id: id,
						text: String::new(),
					});
				} else {
					writer
						.write_event(Event::Empty(event.to_owned()))
						.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
				}
			},
			Ok(Event::Text(event)) => {
				if let Some((_, _, content)) = active.last_mut() {
					content.push_str(&String::from_utf8_lossy(event.as_ref()));
				}
				writer
					.write_event(Event::Text(event.to_owned()))
					.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
			},
			Ok(Event::CData(event)) => {
				if let Some((_, _, content)) = active.last_mut() {
					content.push_str(&String::from_utf8_lossy(event.as_ref()));
				}
				writer
					.write_event(Event::CData(event.to_owned()))
					.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
			},
			Ok(Event::End(event)) => {
				let element_name =
					String::from_utf8_lossy(event.name().as_ref()).into_owned();
				writer
					.write_event(Event::End(BytesEnd::new(element_name)))
					.map_err(|e| ReadAloudError::Xml(e.to_string()))?;
				if is_segment_element(event.name().as_ref()) {
					if let Some((ordinal, id, content)) = active.pop() {
						targets.push(PreparedTarget {
							spine_index,
							ordinal,
							element_id: id,
							text: normalize_text(&content),
						});
					}
				}
			},
			Ok(Event::Eof) => break,
			Ok(event) => writer
				.write_event(event.to_owned())
				.map_err(|e| ReadAloudError::Xml(e.to_string()))?,
			Err(error) => return Err(ReadAloudError::Xml(error.to_string())),
		}
		buffer.clear();
	}
	targets.sort_by_key(|target| target.ordinal);
	Ok((writer.into_inner(), targets))
}

fn smil_for_spine(
	prepared: &PreparedEpub,
	spine: &PreparedSpine,
	cues: Vec<&RenderCue>,
	audio_entry: &str,
	smil_entry: &str,
) -> Result<Vec<u8>, ReadAloudError> {
	let smil_dir = Path::new(smil_entry)
		.parent()
		.unwrap_or_else(|| Path::new(""));
	let audio_href = relative_package_href(smil_dir, audio_entry);
	let text_href = |target: &RenderCue| {
		let path = prepared
			.spines
			.iter()
			.find(|candidate| candidate.spine_index == target.spine_index)
			.map(|item| item.package_path.as_str())
			.unwrap_or("");
		format!(
			"{}#{}",
			relative_package_href(smil_dir, path),
			target.element_id
		)
	};
	let mut output = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<smil xmlns=\"http://www.w3.org/ns/SMIL\" version=\"3.0\"><body><seq id=\"coppice-seq\">");
	for (index, cue) in cues.into_iter().enumerate() {
		output.push_str(&format!("<par id=\"coppice-par-{}-{}\"><text src=\"{}\"/><audio src=\"{}\" clipBegin=\"{}ms\" clipEnd=\"{}ms\"/></par>", spine.spine_index, index, xml_escape(&text_href(cue)), xml_escape(&audio_href), cue.begin_ms, cue.end_ms));
	}
	output.push_str("</seq></body></smil>\n");
	Ok(output.into_bytes())
}

fn rewrite_opf(
	prepared: &PreparedEpub,
	entries: &BTreeMap<String, Vec<u8>>,
	smil_items: &[(&PreparedSpine, String)],
	audio_entry: &str,
) -> Result<Vec<u8>, ReadAloudError> {
	let opf_parent = Path::new(&prepared.opf_path)
		.parent()
		.unwrap_or_else(|| Path::new(""));
	let mut opf = String::from_utf8(
		prepared
			.entries
			.get(&prepared.opf_path)
			.cloned()
			.ok_or_else(|| ReadAloudError::Invalid("OPF entry disappeared".into()))?,
	)
	.map_err(|_| ReadAloudError::Invalid("OPF is not UTF-8".into()))?;
	for (spine, smil_entry) in smil_items {
		let Some(item_id) = prepared_item_id(prepared, &spine.package_path) else {
			continue;
		};
		let smil_id = format!("coppice-smil-{}", spine.spine_index);
		add_media_overlay(&mut opf, &item_id, &smil_id)?;
		let href = relative_package_href(opf_parent, smil_entry);
		let item = format!("<item id=\"{smil_id}\" href=\"{}\" media-type=\"application/smil+xml\" properties=\"media-overlay\"/>", xml_escape(&href));
		insert_before_manifest_end(&mut opf, &item)?;
	}
	let audio_href = relative_package_href(opf_parent, audio_entry);
	insert_before_manifest_end(
		&mut opf,
		&format!(
			"<item id=\"coppice-audio\" href=\"{}\" media-type=\"audio/mp4\"/>",
			xml_escape(&audio_href)
		),
	)?;
	let _ = entries;
	Ok(opf.into_bytes())
}

fn prepared_item_id(prepared: &PreparedEpub, package_path: &str) -> Option<String> {
	let opf_bytes = prepared.entries.get(&prepared.opf_path)?;
	let package = parse_package(opf_bytes).ok()?;
	package
		.manifest
		.into_iter()
		.find(|item| resolve_package_path(&prepared.opf_path, &item.href) == package_path)
		.map(|item| item.id)
}

fn add_media_overlay(
	opf: &mut String,
	item_id: &str,
	smil_id: &str,
) -> Result<(), ReadAloudError> {
	let needle = format!("id=\"{item_id}\"");
	let Some(id_start) = opf.find(&needle) else {
		return Err(ReadAloudError::Invalid(format!(
			"OPF has no manifest item {item_id}"
		)));
	};
	let Some(start) = opf[..id_start].rfind("<item") else {
		return Err(ReadAloudError::Invalid("malformed OPF item".into()));
	};
	let Some(end_rel) = opf[id_start..].find('>') else {
		return Err(ReadAloudError::Invalid("malformed OPF item".into()));
	};
	let end = id_start + end_rel;
	if opf[start..end].contains("media-overlay=") {
		return Ok(());
	}
	opf.insert_str(end, &format!(" media-overlay=\"{smil_id}\""));
	Ok(())
}

fn insert_before_manifest_end(
	opf: &mut String,
	item: &str,
) -> Result<(), ReadAloudError> {
	let lower = opf.to_ascii_lowercase();
	let Some(end) = lower
		.find("</manifest>")
		.or_else(|| lower.find("</opf:manifest>"))
	else {
		return Err(ReadAloudError::Invalid(
			"OPF has no manifest close tag".into(),
		));
	};
	opf.insert_str(end, item);
	Ok(())
}

fn write_deterministic_zip(
	entries: &BTreeMap<String, Vec<u8>>,
	destination: &Path,
) -> Result<(), ReadAloudError> {
	let file = File::create(destination)?;
	let mut zip = ZipWriter::new(file);
	let options: SimpleFileOptions = SimpleFileOptions::default()
		.compression_method(CompressionMethod::Stored)
		.last_modified_time(DateTime::default())
		.unix_permissions(0o644);
	if let Some(bytes) = entries.get("mimetype") {
		zip.start_file("mimetype", options)?;
		zip.write_all(bytes)?;
	} else {
		return Err(ReadAloudError::Invalid("EPUB has no mimetype entry".into()));
	}
	for (name, bytes) in entries {
		if name == "mimetype" {
			continue;
		}
		zip.start_file(name.as_str(), options)?;
		zip.write_all(bytes)?;
	}
	zip.finish()?;
	Ok(())
}

fn resolve_package_path(opf_path: &str, href: &str) -> String {
	let decoded = urlencoding::decode(href)
		.map(|value| value.into_owned())
		.unwrap_or_else(|_| href.to_owned());
	let parent = Path::new(opf_path)
		.parent()
		.unwrap_or_else(|| Path::new(""));
	join_package_path(parent, decoded.split('#').next().unwrap_or(&decoded))
}

fn join_package_path(parent: &Path, child: &str) -> String {
	let mut parts = parent
		.iter()
		.map(|part| part.to_string_lossy().to_string())
		.collect::<Vec<_>>();
	for part in child.split('/') {
		match part {
			"" | "." => {},
			".." => {
				let _ = parts.pop();
			},
			value => parts.push(value.to_owned()),
		}
	}
	parts.join("/")
}

fn relative_package_href(from_dir: &Path, to_path: &str) -> String {
	let from = from_dir
		.iter()
		.map(|part| part.to_string_lossy().to_string())
		.collect::<Vec<_>>();
	let to = Path::new(to_path)
		.iter()
		.map(|part| part.to_string_lossy().to_string())
		.collect::<Vec<_>>();
	let mut shared = 0;
	while shared < from.len() && shared < to.len() && from[shared] == to[shared] {
		shared += 1;
	}
	let mut result = vec!["..".to_owned(); from.len().saturating_sub(shared)];
	result.extend(to.into_iter().skip(shared));
	if result.is_empty() {
		".".to_owned()
	} else {
		result.join("/")
	}
}

fn normalize_path(path: &str) -> String {
	path.trim_start_matches('/')
		.split('/')
		.filter(|part| !part.is_empty() && *part != ".")
		.fold(Vec::new(), |mut parts, part| {
			if part == ".." {
				let _ = parts.pop();
			} else {
				parts.push(part);
			}
			parts
		})
		.join("/")
}

fn local_name(name: &[u8]) -> &str {
	std::str::from_utf8(name)
		.unwrap_or_default()
		.rsplit(':')
		.next()
		.unwrap_or_default()
}

fn is_segment_element(name: &[u8]) -> bool {
	matches!(
		local_name(name).to_ascii_lowercase().as_str(),
		"p" | "h1"
			| "h2" | "h3"
			| "h4" | "h5"
			| "h6" | "li"
			| "blockquote"
			| "section"
			| "div"
	)
}

fn normalize_text(value: &str) -> String {
	value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn xml_escape(value: &str) -> String {
	value
		.replace('&', "&amp;")
		.replace('"', "&quot;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
}

fn sha256_hex(bytes: &[u8]) -> String {
	let mut hasher = Sha256::new();
	hasher.update(bytes);
	hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Cursor;

	fn fixture(path: &Path) {
		let file = File::create(path).unwrap();
		let mut zip = ZipWriter::new(file);
		let options: SimpleFileOptions = SimpleFileOptions::default()
			.compression_method(CompressionMethod::Stored)
			.last_modified_time(DateTime::default());
		zip.start_file("mimetype", options).unwrap();
		zip.write_all(b"application/epub+zip").unwrap();
		zip.start_file("META-INF/container.xml", options).unwrap();
		zip.write_all(br#"<container><rootfiles><rootfile full-path="OEBPS/package.opf"/></rootfiles></container>"#).unwrap();
		zip.start_file("OEBPS/package.opf", options).unwrap();
		zip.write_all(br#"<package><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#).unwrap();
		zip.start_file("OEBPS/chapter.xhtml", options).unwrap();
		zip.write_all(br#"<html><body><p> Hello   world </p></body></html>"#)
			.unwrap();
		zip.finish().unwrap();
	}

	#[test]
	fn preparation_assigns_stable_targets_and_digest() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("book.epub");
		fixture(&path);
		let first = prepare_epub(&path).unwrap();
		let second = prepare_epub(&path).unwrap();
		assert_eq!(first.canonical_text_digest, second.canonical_text_digest);
		assert_eq!(first.spines[0].targets[0].element_id, "coppice-0-0");
		assert!(String::from_utf8(first.spines[0].bytes.clone())
			.unwrap()
			.contains("id=\"coppice-0-0\""));
	}

	#[test]
	fn deterministic_render_keeps_mimetype_first_and_audio_bytes() {
		let dir = tempfile::tempdir().unwrap();
		let epub = dir.path().join("book.epub");
		let audio = dir.path().join("book.m4b");
		fixture(&epub);
		std::fs::write(&audio, b"stable-audio").unwrap();
		let prepared = prepare_epub(&epub).unwrap();
		let cue = RenderCue {
			spine_index: 0,
			ordinal: 0,
			element_id: "coppice-0-0".into(),
			track_index: 0,
			begin_ms: 0,
			end_ms: 1000,
		};
		let one = dir.path().join("one.epub");
		let two = dir.path().join("two.epub");
		render_to_path(&prepared, &audio, &[cue.clone()], &one).unwrap();
		render_to_path(&prepared, &audio, &[cue], &two).unwrap();
		assert_eq!(std::fs::read(&one).unwrap(), std::fs::read(&two).unwrap());
		let mut archive = ZipArchive::new(File::open(one).unwrap()).unwrap();
		assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
		let mut audio_entry = archive.by_name("OEBPS/coppice-audio/book.m4b").unwrap();
		let mut bytes = Vec::new();
		audio_entry.read_to_end(&mut bytes).unwrap();
		assert_eq!(bytes, b"stable-audio");
	}

	#[test]
	fn parser_rejects_unprepared_target() {
		let prepared = PreparedEpub {
			opf_path: "package.opf".into(),
			entries: BTreeMap::new(),
			spines: Vec::new(),
			canonical_text_digest: String::new(),
		};
		let cue = RenderCue {
			spine_index: 0,
			ordinal: 0,
			element_id: "arbitrary".into(),
			track_index: 0,
			begin_ms: 0,
			end_ms: 1,
		};
		assert!(!contains_target(&prepared, &cue));
		let _ = Cursor::new(Vec::<u8>::new());
	}
}
