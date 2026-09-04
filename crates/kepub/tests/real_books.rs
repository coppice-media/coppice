use std::{
	collections::{BTreeMap, HashMap},
	env, fs,
	io::{Cursor, Read},
	path::{Path, PathBuf},
	process::Command,
	time::Instant,
};

use html5ever::{parse_document, tendril::TendrilSink, ParseOpts};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use quick_xml::{events::Event, Reader};
use stump_kepub::{transform_epub, TransformOptions};
use zip::ZipArchive;

/// Differential test against a local corpus.  With `KEPUB_REAL_INPUT` alone the
/// outputs are structurally validated; with `KEPUBIFY_BIN` also set (a kepubify
/// build from the pinned commit) every ZIP entry must be byte-identical to
/// kepubify's.  Books kepubify itself rejects are reported and skipped.
#[test]
#[ignore = "requires the local KEPUB_REAL_INPUT EPUB corpus"]
fn convert_real_books() {
	let Some(input_dir) = env::var_os("KEPUB_REAL_INPUT") else {
		eprintln!("KEPUB_REAL_INPUT is unset; real EPUB differential test skipped");
		return;
	};
	let input_dir = PathBuf::from(input_dir);
	let mut files: Vec<PathBuf> = fs::read_dir(&input_dir)
		.unwrap_or_else(|error| panic!("read {}: {error}", input_dir.display()))
		.map(|entry| entry.expect("read real EPUB directory entry").path())
		.filter(|path| {
			path.is_file()
				&& path
					.extension()
					.is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
		})
		.collect();
	files.sort();
	assert!(
		!files.is_empty(),
		"KEPUB_REAL_INPUT={} contains no *.epub files",
		input_dir.display()
	);
	let kepubify = env::var_os("KEPUBIFY_BIN").map(PathBuf::from);
	let scratch =
		env::temp_dir().join(format!("stump-kepub-oracle-{}", std::process::id()));
	let mut failures = Vec::new();
	for path in files {
		let name = path.display().to_string();
		let source = match fs::read(&path) {
			Ok(source) => source,
			Err(error) => {
				let message = format!("{name}: read failed: {error}");
				eprintln!("real_books ERROR {message}");
				failures.push(message);
				continue;
			},
		};
		let started = Instant::now();
		let output = match transform_epub(&source, &TransformOptions::default()) {
			Ok(output) => output,
			Err(error) => {
				let message = format!("{name}: transform failed: {error}");
				eprintln!("real_books ERROR {message}");
				failures.push(message);
				continue;
			},
		};
		let elapsed = started.elapsed();
		let validation =
			match &kepubify {
				Some(bin) => match kepubify_reference(bin, &path, &scratch) {
					Some(reference) => compare_entries(&output, &reference),
					None => {
						println!("real_books {name}: kepubify rejected this book; parity skipped");
						validate_output(&output)
					},
				},
				None => validate_output(&output),
			};
		println!(
			"real_books {}: {:.3}s input={} output={}",
			name,
			elapsed.as_secs_f64(),
			source.len(),
			output.len()
		);
		if let Err(error) = validation {
			let message = format!("{name}: {error}");
			eprintln!("real_books ERROR {message}");
			failures.push(message);
		}
	}

	assert!(
		failures.is_empty(),
		"real EPUB validation failures:\n{}",
		failures.join("\n")
	);
}

/// Run the pinned kepubify binary on `input`; `None` when it refuses the book.
fn kepubify_reference(bin: &Path, input: &Path, scratch: &Path) -> Option<Vec<u8>> {
	let _ = fs::remove_dir_all(scratch);
	fs::create_dir_all(scratch).expect("create kepubify scratch dir");
	let status = Command::new(bin)
		.arg("-o")
		.arg(scratch)
		.arg(input)
		.output()
		.expect("run kepubify");
	if !status.status.success() {
		return None;
	}
	let produced = fs::read_dir(scratch)
		.expect("read kepubify scratch dir")
		.map(|entry| entry.expect("scratch entry").path())
		.find(|path| path.extension().is_some_and(|ext| ext == "epub"))
		.expect("kepubify wrote an output file");
	Some(fs::read(produced).expect("read kepubify output"))
}

fn zip_entries(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
	let mut archive = ZipArchive::new(Cursor::new(bytes))
		.map_err(|error| format!("output is not a valid ZIP: {error}"))?;
	let mut entries = BTreeMap::new();
	for index in 0..archive.len() {
		let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
		let mut data = Vec::with_capacity(entry.size() as usize);
		entry
			.read_to_end(&mut data)
			.map_err(|error| error.to_string())?;
		entries.insert(entry.name().to_owned(), data);
	}
	Ok(entries)
}

/// Entry-level parity: kepubify's own ZIP writer is not byte-stable, so the
/// contract is the same entry set with byte-identical contents.
fn compare_entries(output: &[u8], reference: &[u8]) -> Result<(), String> {
	let ours = zip_entries(output)?;
	let theirs = zip_entries(reference)?;
	let ours_only: Vec<_> = ours.keys().filter(|k| !theirs.contains_key(*k)).collect();
	let theirs_only: Vec<_> = theirs.keys().filter(|k| !ours.contains_key(*k)).collect();
	if !ours_only.is_empty() || !theirs_only.is_empty() {
		return Err(format!(
			"entry set differs: ours-only={ours_only:?} kepubify-only={theirs_only:?}"
		));
	}
	let differing: Vec<_> = ours
		.iter()
		.filter(|(name, data)| theirs[*name] != **data)
		.map(|(name, data)| {
			let reference = &theirs[name];
			let offset = data
				.iter()
				.zip(reference)
				.position(|(a, b)| a != b)
				.unwrap_or(data.len().min(reference.len()));
			format!("{name} (first difference at byte {offset})")
		})
		.collect();
	if differing.is_empty() {
		Ok(())
	} else {
		Err(format!(
			"{} entries differ from kepubify: {differing:?}",
			differing.len()
		))
	}
}

fn validate_output(output: &[u8]) -> Result<(), String> {
	let mut archive = ZipArchive::new(Cursor::new(output))
		.map_err(|error| format!("output is not a valid ZIP: {error}"))?;
	let package_path = read_package_path(&mut archive)?;
	let opf = read_zip_entry(&mut archive, &package_path)?;
	let spine_documents = read_spine_documents(&opf, &package_path)?;
	if spine_documents.is_empty() {
		return Err("package spine has no itemref documents".to_string());
	}
	for document in spine_documents {
		if document.ends_with("kepubify-titlepage-dummy.xhtml") {
			continue; // written verbatim by design; never has spans
		}
		let bytes = read_zip_entry(&mut archive, &document)?;
		let text = std::str::from_utf8(&bytes).map_err(|error| {
			format!("spine document {document:?} is not UTF-8: {error}")
		})?;
		let has_spans = text.contains("class=\"koboSpan\"");
		if !has_spans && has_span_candidate(&bytes)? {
			return Err(format!("spine document {document:?} contains no koboSpan"));
		}
		if !has_spans {
			println!(
				"real_books NOTE spine document {document:?} has no koboSpan (no non-skipped body text or image; expected parity)"
			);
		}
		if !text.contains("id=\"book-columns\"") {
			return Err(format!(
				"spine document {document:?} contains no book-columns wrapper"
			));
		}
		if !text.contains("id=\"book-inner\"") {
			return Err(format!(
				"spine document {document:?} contains no book-inner wrapper"
			));
		}
	}
	Ok(())
}

fn has_span_candidate(bytes: &[u8]) -> Result<bool, String> {
	let dom = parse_document(RcDom::default(), ParseOpts::default())
		.from_utf8()
		.read_from(&mut Cursor::new(bytes))
		.map_err(|error| format!("parse content document: {error}"))?;
	let Some(body) = find_element(&dom.document, "body") else {
		return Ok(false);
	};
	Ok(node_has_span_candidate(&body))
}

fn find_element(root: &Handle, wanted: &str) -> Option<Handle> {
	let mut stack = vec![root.clone()];
	while let Some(node) = stack.pop() {
		if let NodeData::Element { ref name, .. } = node.data {
			if name.local.as_ref().eq_ignore_ascii_case(wanted) {
				return Some(node);
			}
		}
		for child in node.children.borrow().iter().rev() {
			stack.push(child.clone());
		}
	}
	None
}

fn node_has_span_candidate(node: &Handle) -> bool {
	if let NodeData::Text { ref contents } = node.data {
		return contents.borrow().chars().any(|ch| !ch.is_whitespace());
	}
	let NodeData::Element { ref name, .. } = node.data else {
		for child in node.children.borrow().iter() {
			if node_has_span_candidate(child) {
				return true;
			}
		}
		return false;
	};
	let name = name.local.as_ref();
	if matches!(
		name,
		"script" | "style" | "pre" | "audio" | "video" | "svg" | "math"
	) {
		return false;
	}
	if name == "img" {
		return true;
	}
	node.children.borrow().iter().any(node_has_span_candidate)
}

fn read_package_path(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<String, String> {
	let container = read_zip_entry(archive, "META-INF/container.xml")?;
	let mut reader = Reader::from_reader(container.as_slice());
	reader.config_mut().trim_text(false);
	let mut buffer = Vec::new();
	loop {
		match reader
			.read_event_into(&mut buffer)
			.map_err(|error| format!("parse container.xml: {error}"))?
		{
			Event::Start(element) | Event::Empty(element)
				if local_name(element.name().as_ref()) == b"rootfile" =>
			{
				if let Some(path) = attribute(&element, b"full-path")? {
					return Ok(path);
				}
			},
			Event::Eof => break,
			_ => {},
		}
		buffer.clear();
	}
	Err("container.xml has no rootfile full-path".to_string())
}

fn read_spine_documents(opf: &[u8], package_path: &str) -> Result<Vec<String>, String> {
	let mut reader = Reader::from_reader(opf);
	reader.config_mut().trim_text(false);
	let mut buffer = Vec::new();
	let mut manifest = HashMap::new();
	let mut spine_ids = Vec::new();
	loop {
		match reader
			.read_event_into(&mut buffer)
			.map_err(|error| format!("parse {package_path}: {error}"))?
		{
			Event::Start(element) | Event::Empty(element) => {
				match local_name(element.name().as_ref()) {
					b"item" => {
						let Some(id) = attribute(&element, b"id")? else {
							buffer.clear();
							continue;
						};
						let Some(href) = attribute(&element, b"href")? else {
							buffer.clear();
							continue;
						};
						manifest.insert(id, href);
					},
					b"itemref" => {
						if let Some(idref) = attribute(&element, b"idref")? {
							spine_ids.push(idref);
						}
					},
					_ => {},
				}
			},
			Event::Eof => break,
			_ => {},
		}
		buffer.clear();
	}

	let package_dir = package_path
		.rsplit_once('/')
		.map_or("", |(directory, _)| directory);
	spine_ids
		.into_iter()
		.map(|id| {
			let href = manifest
				.get(&id)
				.ok_or_else(|| format!("spine idref {id:?} has no manifest item"))?;
			Ok(join_archive_path(package_dir, href))
		})
		.collect()
}

fn read_zip_entry(
	archive: &mut ZipArchive<Cursor<&[u8]>>,
	name: &str,
) -> Result<Vec<u8>, String> {
	let mut file = archive
		.by_name(name)
		.map_err(|error| format!("missing ZIP entry {name:?}: {error}"))?;
	let mut bytes = Vec::with_capacity(file.size() as usize);
	file.read_to_end(&mut bytes)
		.map_err(|error| format!("read ZIP entry {name:?}: {error}"))?;
	Ok(bytes)
}

fn attribute(
	element: &quick_xml::events::BytesStart<'_>,
	wanted: &[u8],
) -> Result<Option<String>, String> {
	for attr in element.attributes().with_checks(false) {
		let attr = attr.map_err(|error| format!("parse XML attribute: {error}"))?;
		if local_name(attr.key.as_ref()) == wanted {
			return attr
				.unescape_value()
				.map(|value| Some(value.into_owned()))
				.map_err(|error| format!("decode XML attribute: {error}"));
		}
	}
	Ok(None)
}

fn local_name(name: &[u8]) -> &[u8] {
	name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn join_archive_path(directory: &str, href: &str) -> String {
	let href = href.split(['#', '?']).next().unwrap_or(href);
	if href.starts_with('/') {
		return href.trim_start_matches('/').to_string();
	}
	let mut components = Vec::new();
	for component in directory
		.split('/')
		.chain(href.split('/'))
		.filter(|component| !component.is_empty() && *component != ".")
	{
		if component == ".." {
			components.pop();
		} else {
			components.push(component);
		}
	}
	components.join("/")
}
