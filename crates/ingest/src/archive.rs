//! Exploding a container drop into the publications it holds.
//!
//! A `.cbz` is one book in a zip. A `.rar` holding 35 numbered MP3s and a
//! `cover.jpg` is one book too — but a `.rar` holding an EPUB, a MOBI and a
//! cover is *two* books, and the download a librarian actually drops is
//! usually the second kind. Treating every container as one book means the
//! folder audiobook arrives as a "comic archive" with 35 unreadable pages and
//! the EPUB inside `Ebooks.rar` never arrives at all.
//!
//! So a dropped `.zip`/`.rar`/`.7z` is asked one question before anything is
//! extracted: **does it contain publications?** The member list answers it for
//! free, from the zip central directory or `unrar l`, without writing a byte:
//!
//! * **No publications** — every member is an image, a document resource, or
//!   junk. That is a comic archive or an EPUB-shaped zip, and it stays exactly
//!   what it was before this module existed: one drop item of one book.
//! * **One or more publications** — the container is a *delivery*, not a book.
//!   It is extracted and [`classify`] turns its tree into one drop item per
//!   publication, sharing a drop group so the editor can show them together.
//!
//! # How members become items
//!
//! [`classify`] walks the extracted tree once and applies three rules, in this
//! order:
//!
//! 1. **A directory of audio is one book.** The same collapse the scanner
//!    performs through [`PathUtils::dir_is_audio_book`]: a flat directory whose
//!    files are audio plus sidecars is one [`IngestMediaKind::Audio`] item
//!    whose target is the directory. `cover.jpg`, `.nfo`, `.cue` and `.txt`
//!    beside it are that item's sidecars, never items of their own.
//! 2. **Every other publication is its own item.** An EPUB, MOBI, AZW3, CBZ,
//!    CBR or PDF is one book; three of them in one archive are three items.
//! 3. **Everything else is a sidecar or a note.** A cover image attaches to
//!    the nearest publication; anything left over is reported in
//!    [`Explosion::notes`] so a dropped file is accounted for rather than
//!    silently deleted.
//!
//! # Extraction
//!
//! Zip is extracted in-process. RAR uses the `unrar` bindings when the crate
//! is built with the `rar` feature — the same library the RAR page processor
//! uses, so a rar ingest cannot succeed where a rar *read* would fail — and
//! falls back to an operator-installed `unrar`, `7z`, or `bsdtar` otherwise.
//! 7-Zip is always external: there is no in-tree 7z reader, and inventing one
//! for an ingest edge case would be a codec, not a feature.
//!
//! A failed extraction is one failed drop item carrying the extractor's stderr
//! tail. It is never a partially exploded group: the destination is removed and
//! the archive is left in place, so fixing the installation and rescanning is
//! the whole recovery.

use std::{
	ffi::OsString,
	path::{Component, Path, PathBuf},
	time::Duration,
};

use globset::GlobSet;
use stump_media::PathUtils;
use stump_tools::external::{ExternalTool, MinVersion};

use crate::{contract::IngestMediaKind, error::IngestError, IngestResult};

/// Wall clock one extraction gets. A 400 MB RAR of MP3s unpacks in seconds on
/// any disk; an hour means the child is waiting on something that will never
/// arrive (a password prompt, a dead network mount).
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(3600);

/// Stems that make an image the publication's cover rather than a page.
const COVER_STEMS: [&str; 4] = ["cover", "folder", "front", "cover_art"];

/// Extensions that are a publication on their own.
const BOOK_EXTENSIONS: [&str; 8] =
	["epub", "mobi", "azw", "azw3", "prc", "cbz", "cbr", "pdf"];

/// Extensions that describe a publication without being one: notes, chapter
/// sheets, and metadata dumps a downloader ships beside the book.
const SIDECAR_EXTENSIONS: [&str; 9] = [
	"nfo", "txt", "cue", "opf", "json", "url", "md", "log", "sfv",
];

/// Image extensions. An image named [`COVER_STEMS`] is a cover sidecar;
/// any other image is a page, and a container of nothing but pages is a comic.
const IMAGE_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "gif", "avif"];

/// The container formats a drop may explode. Deliberately not `cbz`/`cbr`:
/// those extensions are a *claim* that the container is one comic, and this
/// module never overrules it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
	Zip,
	Rar,
	SevenZip,
}

impl ArchiveFormat {
	/// The format a dropped filename claims, or `None` for anything that is
	/// not a general-purpose container.
	pub fn for_filename(filename: &str) -> Option<Self> {
		match Path::new(filename)
			.extension()
			.and_then(|extension| extension.to_str())
			.unwrap_or_default()
			.to_ascii_lowercase()
			.as_str()
		{
			"zip" => Some(Self::Zip),
			"rar" => Some(Self::Rar),
			"7z" => Some(Self::SevenZip),
			_ => None,
		}
	}

	pub fn as_str(self) -> &'static str {
		match self {
			Self::Zip => "zip",
			Self::Rar => "rar",
			Self::SevenZip => "7z",
		}
	}
}

/// One entry of a container's member list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveMember {
	/// Slash-separated path inside the container, as the container spells it.
	pub path: String,
	pub byte_size: u64,
	pub is_dir: bool,
}

/// One publication found inside an exploded container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplodedItem {
	pub kind: IngestMediaKind,
	/// Absolute path in the extraction directory. A file for every kind
	/// except [`IngestMediaKind::Audio`], whose target may be a directory of
	/// parts.
	pub path: PathBuf,
	/// The name the drop item is known by: the file name, or the folder name
	/// for a folder audiobook.
	pub filename: String,
	/// Directory of the item inside the container, `/`-separated and empty at
	/// the root, so a nested delivery keeps its shape on commit.
	pub relative_path: Option<String>,
	/// Files that describe this item without being it.
	pub sidecars: Vec<PathBuf>,
}

/// What one container turned into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Explosion {
	pub items: Vec<ExplodedItem>,
	/// Members that became neither an item nor a sidecar, each with the
	/// reason. A dropped file is always accounted for.
	pub notes: Vec<String>,
}

/// Whether a container is a *delivery* of publications rather than one book.
///
/// Answered from the member list alone, so a 400 MB comic is never extracted
/// to discover it was a comic.
pub fn holds_publications(members: &[ArchiveMember]) -> bool {
	members.iter().any(|member| {
		!member.is_dir && matches!(classify_member(&member.path), Member::Publication(_))
	})
}

/// List a container's members without extracting it.
pub fn list_members(
	format: ArchiveFormat,
	path: &Path,
) -> IngestResult<Vec<ArchiveMember>> {
	match format {
		ArchiveFormat::Zip => list_zip(path),
		ArchiveFormat::Rar => list_rar(path),
		ArchiveFormat::SevenZip => list_external(&SEVEN_ZIP, "7z", path),
	}
}

/// Extract every member of a container into `destination`.
///
/// `destination` is created and owned by the caller. On any failure the
/// partially written tree is removed, so a caller never sees half a delivery.
pub fn extract(
	format: ArchiveFormat,
	path: &Path,
	destination: &Path,
) -> IngestResult<()> {
	std::fs::create_dir_all(destination)?;
	let result = match format {
		ArchiveFormat::Zip => extract_zip(path, destination),
		ArchiveFormat::Rar => extract_rar(path, destination),
		ArchiveFormat::SevenZip => extract_external(
			&SEVEN_ZIP,
			"7z",
			seven_zip_argv(path, destination),
			path,
			destination,
		),
	};
	if result.is_err() {
		let _ = std::fs::remove_dir_all(destination);
	}
	result
}

/// Turn an extracted tree into one item per publication.
///
/// Directories are visited depth-first so a folder audiobook is recognised
/// before its individual MP3s are, which is what makes 35 parts one book
/// instead of 35 books.
pub fn classify(root: &Path) -> IngestResult<Explosion> {
	let mut explosion = Explosion::default();
	visit(root, root, &mut explosion)?;
	// Deterministic order: the editor's sibling strip and every test read the
	// same list whatever order the filesystem enumerated.
	explosion
		.items
		.sort_by(|left, right| left.filename.cmp(&right.filename));
	explosion.notes.sort();
	attach_orphan_covers(root, &mut explosion)?;
	Ok(explosion)
}

/// What one member's name says it is.
enum Member {
	/// A book in its own right.
	Publication(IngestMediaKind),
	/// Describes a publication: cover art, notes, a chapter sheet.
	Sidecar,
	/// An image that is a page, not a cover.
	Page,
	/// Anything else.
	Other,
}

fn classify_member(path: &str) -> Member {
	let path = Path::new(path);
	let extension = path
		.extension()
		.and_then(|extension| extension.to_str())
		.unwrap_or_default()
		.to_ascii_lowercase();
	let stem = path
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or_default()
		.to_ascii_lowercase();

	if BOOK_EXTENSIONS.contains(&extension.as_str()) {
		return Member::Publication(kind_for_extension(&extension));
	}
	if models::entity::media::AUDIO_EXTENSIONS.contains(&extension.as_str()) {
		return Member::Publication(IngestMediaKind::Audio);
	}
	if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
		return if COVER_STEMS.contains(&stem.as_str()) {
			Member::Sidecar
		} else {
			Member::Page
		};
	}
	if SIDECAR_EXTENSIONS.contains(&extension.as_str()) {
		return Member::Sidecar;
	}
	Member::Other
}

fn kind_for_extension(extension: &str) -> IngestMediaKind {
	match extension {
		"epub" => IngestMediaKind::Epub,
		"pdf" => IngestMediaKind::Pdf,
		"cbz" => IngestMediaKind::ComicArchive,
		"cbr" => IngestMediaKind::ComicRarArchive,
		// MOBI/AZW3/PRC have no ingest kind of their own: the processor reads
		// them, the page/quality lane does not, and `Unknown` is what the
		// existing filename mapping already produces for them.
		_ => IngestMediaKind::Unknown,
	}
}

/// Depth-first walk. Returns whether `directory` was consumed as a single
/// publication (a folder audiobook), in which case its members are not
/// visited individually.
fn visit(root: &Path, directory: &Path, explosion: &mut Explosion) -> IngestResult<bool> {
	if directory != root && directory.dir_is_audio_book(&GlobSet::empty()) {
		let (sidecars, _) = partition_directory(directory)?;
		explosion.items.push(ExplodedItem {
			kind: IngestMediaKind::Audio,
			path: directory.to_path_buf(),
			filename: file_name(directory),
			// The *containing* directory, never the book's own: an item
			// commits at `<library>/<relative path>/<filename>`, so naming
			// the audiobook folder here would nest it inside itself.
			relative_path: directory
				.parent()
				.and_then(|parent| relative_directory(root, parent)),
			sidecars,
		});
		return Ok(true);
	}

	let mut entries = Vec::new();
	for entry in std::fs::read_dir(directory)? {
		entries.push(entry?.path());
	}
	entries.sort();

	let mut directory_items = 0_usize;
	let mut loose_sidecars = Vec::new();
	let mut pages = 0_usize;
	for path in entries {
		let name = file_name(&path);
		if name.starts_with('.') || name == "__MACOSX" {
			continue;
		}
		if path.is_dir() {
			let before = explosion.items.len();
			visit(root, &path, explosion)?;
			directory_items += explosion.items.len() - before;
			continue;
		}
		if !path.is_file() {
			continue;
		}
		match classify_member(&name) {
			Member::Publication(kind) => {
				explosion.items.push(ExplodedItem {
					kind,
					path: path.clone(),
					filename: name,
					relative_path: relative_directory(root, directory),
					sidecars: Vec::new(),
				});
				directory_items += 1;
			},
			Member::Sidecar => loose_sidecars.push(path),
			Member::Page => pages += 1,
			Member::Other => explosion.notes.push(format!(
				"{}: not a publication, sidecar, or page",
				relative_display(root, &path)
			)),
		}
	}

	// A directory that produced exactly one item owns the sidecars beside it;
	// with several items nothing can say which book a stray `.nfo` describes,
	// so it is reported instead of guessed at.
	if directory_items == 1 && !loose_sidecars.is_empty() {
		if let Some(item) = explosion.items.last_mut() {
			item.sidecars.extend(loose_sidecars.drain(..));
		}
	}
	for sidecar in loose_sidecars {
		explosion.notes.push(format!(
			"{}: sidecar with no single publication to attach to",
			relative_display(root, &sidecar)
		));
	}
	if pages > 0 {
		explosion.notes.push(format!(
			"{}: {pages} loose image(s) ignored; a container of pages is a comic, not a delivery",
			relative_display(root, directory)
		));
	}

	Ok(false)
}

/// A folder audiobook's sidecars and its audio parts.
fn partition_directory(directory: &Path) -> IngestResult<(Vec<PathBuf>, Vec<PathBuf>)> {
	let mut sidecars = Vec::new();
	let mut audio = Vec::new();
	let mut entries = Vec::new();
	for entry in std::fs::read_dir(directory)? {
		entries.push(entry?.path());
	}
	entries.sort();
	for path in entries {
		if !path.is_file() {
			continue;
		}
		let name = file_name(&path);
		if name.starts_with('.') {
			continue;
		}
		match classify_member(&name) {
			Member::Publication(IngestMediaKind::Audio) => audio.push(path),
			// `dir_is_audio_book` already refused a directory holding another
			// publication, so anything left here describes the book.
			_ => sidecars.push(path),
		}
	}
	Ok((sidecars, audio))
}

/// A cover at the container root belongs to the delivery's only publication.
///
/// The common `Ebooks.rar` shape is `cover.jpg` + `book.epub` + `book.mobi`:
/// the cover is attached when exactly one item can own it, and reported
/// otherwise.
fn attach_orphan_covers(root: &Path, explosion: &mut Explosion) -> IngestResult<()> {
	if explosion.items.len() != 1 {
		return Ok(());
	}
	let claimed = explosion
		.notes
		.iter()
		.filter(|note| note.contains("no single publication"))
		.count();
	if claimed == 0 {
		return Ok(());
	}
	let (sidecars, _) = partition_root_sidecars(root)?;
	explosion
		.notes
		.retain(|note| !note.contains("no single publication"));
	if let Some(item) = explosion.items.first_mut() {
		for sidecar in sidecars {
			if !item.sidecars.contains(&sidecar) {
				item.sidecars.push(sidecar);
			}
		}
		item.sidecars.sort();
	}
	Ok(())
}

fn partition_root_sidecars(root: &Path) -> IngestResult<(Vec<PathBuf>, Vec<PathBuf>)> {
	let mut sidecars = Vec::new();
	let mut rest = Vec::new();
	for entry in std::fs::read_dir(root)? {
		let path = entry?.path();
		if !path.is_file() {
			continue;
		}
		let name = file_name(&path);
		if name.starts_with('.') {
			continue;
		}
		match classify_member(&name) {
			Member::Sidecar => sidecars.push(path),
			_ => rest.push(path),
		}
	}
	sidecars.sort();
	Ok((sidecars, rest))
}

fn file_name(path: &Path) -> String {
	path.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_default()
}

fn relative_directory(root: &Path, directory: &Path) -> Option<String> {
	let relative = directory.strip_prefix(root).ok()?;
	let value = relative.to_string_lossy().replace('\\', "/");
	(!value.is_empty()).then_some(value)
}

fn relative_display(root: &Path, path: &Path) -> String {
	path.strip_prefix(root)
		.unwrap_or(path)
		.to_string_lossy()
		.replace('\\', "/")
}

// ---------------------------------------------------------------------------
// zip
// ---------------------------------------------------------------------------

fn list_zip(path: &Path) -> IngestResult<Vec<ArchiveMember>> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file)
		.map_err(|error| IngestError::BadRequest(format!("unreadable zip: {error}")))?;
	let mut members = Vec::with_capacity(archive.len());
	for index in 0..archive.len() {
		let entry = archive.by_index(index).map_err(|error| {
			IngestError::BadRequest(format!("unreadable zip: {error}"))
		})?;
		members.push(ArchiveMember {
			path: entry.name().replace('\\', "/"),
			byte_size: entry.size(),
			is_dir: entry.is_dir(),
		});
	}
	Ok(members)
}

fn extract_zip(path: &Path, destination: &Path) -> IngestResult<()> {
	let file = std::fs::File::open(path)?;
	let mut archive = zip::ZipArchive::new(file)
		.map_err(|error| IngestError::BadRequest(format!("unreadable zip: {error}")))?;
	let mut written = 0_usize;
	for index in 0..archive.len() {
		let mut entry = archive.by_index(index).map_err(|error| {
			IngestError::BadRequest(format!("unreadable zip: {error}"))
		})?;
		let Some(relative) = safe_member_path(entry.name()) else {
			continue;
		};
		let target = destination.join(&relative);
		if entry.is_dir() {
			std::fs::create_dir_all(&target)?;
			continue;
		}
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let mut output = std::fs::File::create(&target)?;
		std::io::copy(&mut entry, &mut output)?;
		written += 1;
	}
	if written == 0 {
		return Err(IngestError::BadRequest(
			"zip contained no files".to_string(),
		));
	}
	Ok(())
}

/// A member path that cannot escape the destination.
///
/// Absolute paths, drive prefixes and `..` are dropped rather than sanitised:
/// a container that names `../../etc/passwd` is hostile, and the honest
/// response to a hostile member is not to write it anywhere.
fn safe_member_path(name: &str) -> Option<PathBuf> {
	let name = name.replace('\\', "/");
	if name.is_empty() || name.starts_with('/') {
		return None;
	}
	let mut relative = PathBuf::new();
	for component in Path::new(&name).components() {
		match component {
			Component::Normal(part) => relative.push(part),
			Component::CurDir => {},
			Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
				return None
			},
		}
	}
	(!relative.as_os_str().is_empty()).then_some(relative)
}

// ---------------------------------------------------------------------------
// rar
// ---------------------------------------------------------------------------

#[cfg(feature = "rar")]
fn list_rar(path: &Path) -> IngestResult<Vec<ArchiveMember>> {
	match stump_media::RarProcessor::list_entries(path) {
		Ok(entries) => Ok(entries
			.into_iter()
			.map(|entry| ArchiveMember {
				path: entry.path.replace('\\', "/"),
				byte_size: entry.byte_size,
				is_dir: entry.is_dir,
			})
			.collect()),
		// The bindings refuse a header they cannot read at all (a RAR5
		// feature the vendored unrar predates, a passworded archive). The
		// operator's own `unrar` is newer more often than not, so it gets the
		// second attempt rather than the drop failing here.
		Err(error) => {
			tracing::debug!(?error, ?path, "unrar bindings could not list; trying PATH");
			list_external(&UNRAR, "unrar", path)
		},
	}
}

#[cfg(not(feature = "rar"))]
fn list_rar(path: &Path) -> IngestResult<Vec<ArchiveMember>> {
	list_external(&UNRAR, "unrar", path)
}

#[cfg(feature = "rar")]
fn extract_rar(path: &Path, destination: &Path) -> IngestResult<()> {
	match stump_media::RarProcessor::extract_all(path, destination) {
		Ok(0) => Err(IngestError::BadRequest(
			"rar contained no files".to_string(),
		)),
		Ok(_) => Ok(()),
		Err(error) => {
			tracing::debug!(
				?error,
				?path,
				"unrar bindings could not extract; trying PATH"
			);
			extract_external(
				&UNRAR,
				"unrar",
				unrar_argv(path, destination),
				path,
				destination,
			)
		},
	}
}

#[cfg(not(feature = "rar"))]
fn extract_rar(path: &Path, destination: &Path) -> IngestResult<()> {
	extract_external(
		&UNRAR,
		"unrar",
		unrar_argv(path, destination),
		path,
		destination,
	)
}

// ---------------------------------------------------------------------------
// operator-installed extractors
// ---------------------------------------------------------------------------

/// `unrar`, for a build without the bindings or an archive they refuse.
static UNRAR: ExternalTool = ExternalTool {
	name: "unrar",
	probe: "unrar",
	// `unrar` with no arguments prints its banner and exits non-zero, and
	// `-V` is not an option it knows; the help screen carries the version.
	version_arg: "-?",
	anchor: "unrar ",
	min: MinVersion::MajorMinor(5, 0),
	extra_dirs: no_extra_dirs,
	searched: "PATH",
	hint: "install `unrar` (or build Stump with the `rar` feature), or extract the archive yourself and drop its contents",
};

/// `7z`, the only reader for a 7-Zip container.
static SEVEN_ZIP: ExternalTool = ExternalTool {
	name: "7z",
	probe: "7z",
	version_arg: "--help",
	anchor: "7-zip",
	min: MinVersion::MajorMinor(16, 0),
	extra_dirs: no_extra_dirs,
	searched: "PATH",
	hint: "install `p7zip` (or `7zip`), or extract the archive yourself and drop its contents",
};

/// `bsdtar`, which reads rar and 7z through libarchive. The last resort:
/// `--version` is the only banner it prints, and libarchive cannot read
/// RAR5 in every build, so it is tried after the format's own tool.
static BSDTAR: ExternalTool = ExternalTool {
	name: "bsdtar",
	probe: "bsdtar",
	version_arg: "--version",
	anchor: "bsdtar ",
	min: MinVersion::MajorMinor(3, 0),
	extra_dirs: no_extra_dirs,
	searched: "PATH",
	hint: "install `libarchive` (bsdtar)",
};

fn no_extra_dirs() -> Vec<PathBuf> {
	Vec::new()
}

fn unrar_argv(path: &Path, destination: &Path) -> Vec<OsString> {
	// `x` keeps member directories; `-y` answers every prompt; `-p-` refuses
	// a passworded archive instead of blocking on stdin forever.
	let mut destination = destination.as_os_str().to_owned();
	destination.push("/");
	vec![
		OsString::from("x"),
		OsString::from("-y"),
		OsString::from("-p-"),
		OsString::from("--"),
		path.as_os_str().to_owned(),
		destination,
	]
}

fn seven_zip_argv(path: &Path, destination: &Path) -> Vec<OsString> {
	let mut output = OsString::from("-o");
	output.push(destination.as_os_str());
	vec![
		OsString::from("x"),
		OsString::from("-y"),
		OsString::from("-p"),
		output,
		OsString::from("--"),
		path.as_os_str().to_owned(),
	]
}

fn bsdtar_argv(path: &Path, destination: &Path) -> Vec<OsString> {
	vec![
		OsString::from("-x"),
		OsString::from("-f"),
		path.as_os_str().to_owned(),
		OsString::from("-C"),
		destination.as_os_str().to_owned(),
	]
}

/// The member list an installed extractor prints.
///
/// `unrar l` and `7z l` print *different* fixed-width tables — `Attributes
/// Size Date Time Name` against `Date Time Attr Size Compressed Name` — so
/// the columns are read from the header row rather than counted. The `Name`
/// header's byte offset is where every member name starts, which is also the
/// only way a name containing spaces survives; everything left of it is the
/// row's own metadata, so an attribute flag can never be confused with a
/// filename.
///
/// The child is run here rather than through [`ExternalTool::run`] for one
/// reason: that helper keeps only the *tail* of stdout, which is exactly
/// right for a failure message and exactly wrong for a listing. A 35-part
/// audiobook's table is over four kilobytes, so the tail drops the header
/// row, the offset is never found, and the archive parses as holding nothing
/// — a folder audiobook silently admitted as a one-file comic. `locate` is
/// still used, so the binary is found and version-checked the same way.
fn list_external(
	tool: &ExternalTool,
	binary: &str,
	path: &Path,
) -> IngestResult<Vec<ArchiveMember>> {
	use std::process::{Command, Stdio};

	let install = tool.locate(None).map_err(tool_error)?;
	let program = tool.binary(&install.dir, binary);
	// `wait_with_output` drains both pipes as the child writes, so a listing
	// larger than a pipe buffer cannot deadlock and none of it is dropped.
	let output = Command::new(&program)
		.args([
			OsString::from("l"),
			OsString::from("--"),
			path.as_os_str().to_owned(),
		])
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.output()
		.map_err(|error| {
			if error.kind() == std::io::ErrorKind::NotFound {
				tool_error(
					tool.missing(format!("{} could not be executed", program.display())),
				)
			} else {
				IngestError::IoError(error)
			}
		})?;
	let listing = String::from_utf8_lossy(&output.stdout).into_owned();
	if !output.status.success() {
		let errors = String::from_utf8_lossy(&output.stderr).into_owned();
		return Err(IngestError::BadRequest(format!(
			"`{binary} l` failed (exit {}): {}",
			output.status.code().unwrap_or(-1),
			tail(&errors, &listing)
		)));
	}
	let members = parse_listing(&listing);
	if members.is_empty() {
		// An archive with no members at all is not something to admit as a
		// one-file book: either the container is empty or the listing was not
		// understood, and both deserve a failed drop that says so.
		return Err(IngestError::BadRequest(format!(
			"`{binary} l` listed no members: {}",
			tail("", &listing)
		)));
	}
	Ok(members)
}

fn parse_listing(text: &str) -> Vec<ArchiveMember> {
	let mut members = Vec::new();
	let mut name_offset = None;
	let mut inside = false;
	for line in text.lines() {
		if !inside {
			// The header row names the columns; the rule row below it opens
			// the table.
			if let Some(at) = line.find("Name") {
				name_offset = Some(at);
				continue;
			}
			if name_offset.is_some() && is_rule(line) {
				inside = true;
			}
			continue;
		}
		if is_rule(line) {
			break;
		}
		let Some(offset) = name_offset else { break };
		if line.len() <= offset {
			continue;
		}
		let name = line[offset..].trim();
		if name.is_empty() {
			continue;
		}
		let prefix = &line[..offset];
		members.push(ArchiveMember {
			path: name.replace('\\', "/"),
			byte_size: row_size(prefix),
			is_dir: row_is_dir(prefix),
		});
	}
	members
}

fn is_rule(line: &str) -> bool {
	let trimmed = line.trim();
	!trimmed.is_empty() && trimmed.chars().all(|c| c == '-' || c == ' ')
}

/// The uncompressed size of a row.
///
/// The first numeric column is the size in both layouts: `unrar` prints only
/// one number before the name, and `7z` prints size before compressed size.
fn row_size(prefix: &str) -> u64 {
	prefix
		.split_whitespace()
		.find_map(|column| column.parse::<u64>().ok())
		.unwrap_or(0)
}

/// Whether a row's attribute flags mark a directory.
///
/// The flag column is the only all-alphabetic-or-dot column in the row
/// prefix (`...D...` for `unrar`, `D....` for `7z`); the timestamp columns
/// carry digits and the size columns are numeric, so neither can be mistaken
/// for it. The name is never examined, so a book called `D.epub` stays a file.
fn row_is_dir(prefix: &str) -> bool {
	prefix
		.split_whitespace()
		.filter(|column| {
			column.len() >= 4
				&& column.chars().all(|c| c == '.' || c.is_ascii_alphabetic())
		})
		.any(|column| column.contains('D') || column.contains('d'))
}

/// Run one extractor, falling back to `bsdtar` when the format's own tool is
/// not installed or refuses the container.
///
/// The primary tool's failure is what surfaces: `bsdtar` reporting "unrecognized
/// archive format" for a RAR5 its libarchive cannot read is noise beside
/// "unrar is not installed", which is the sentence that names the fix.
fn extract_external(
	tool: &ExternalTool,
	binary: &str,
	argv: Vec<OsString>,
	path: &Path,
	destination: &Path,
) -> IngestResult<()> {
	match run_extractor(tool, binary, &argv) {
		Ok(()) => Ok(()),
		Err(primary) => {
			match run_extractor(&BSDTAR, "bsdtar", &bsdtar_argv(path, destination)) {
				Ok(()) => Ok(()),
				Err(_) => Err(primary),
			}
		},
	}
}

fn run_extractor(
	tool: &ExternalTool,
	binary: &str,
	argv: &[OsString],
) -> IngestResult<()> {
	let install = tool.locate(None).map_err(tool_error)?;
	let output = tool
		.run(&tool.binary(&install.dir, binary), argv, EXTRACT_TIMEOUT)
		.map_err(tool_error)?;
	if output.succeeded() {
		return Ok(());
	}
	Err(IngestError::BadRequest(format!(
		"`{binary}` failed ({}): {}",
		output.describe_status(),
		tail(&output.stderr, &output.stdout)
	)))
}

fn tool_error(error: stump_tools::ToolError) -> IngestError {
	IngestError::BadRequest(error.to_string())
}

/// The failure reason an operator reads: the extractor's stderr, or its
/// stdout when it reports failures there (7-Zip does).
fn tail(stderr: &str, stdout: &str) -> String {
	let source = if stderr.trim().is_empty() {
		stdout
	} else {
		stderr
	};
	let lines = source.lines().rev().take(6).collect::<Vec<_>>();
	lines
		.into_iter()
		.rev()
		.collect::<Vec<_>>()
		.join("; ")
		.trim()
		.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn format_is_claimed_by_extension_and_never_by_a_comic_extension() {
		assert_eq!(
			ArchiveFormat::for_filename("a.zip"),
			Some(ArchiveFormat::Zip)
		);
		assert_eq!(
			ArchiveFormat::for_filename("a.RAR"),
			Some(ArchiveFormat::Rar)
		);
		assert_eq!(
			ArchiveFormat::for_filename("a.7z"),
			Some(ArchiveFormat::SevenZip)
		);
		for comic in ["a.cbz", "a.cbr", "a.epub", "a.pdf", "a.mp3"] {
			assert_eq!(ArchiveFormat::for_filename(comic), None, "{comic}");
		}
	}

	#[test]
	fn a_container_of_pages_holds_no_publications() {
		let pages = (1..=4)
			.map(|index| ArchiveMember {
				path: format!("page-{index}.jpg"),
				byte_size: 10,
				is_dir: false,
			})
			.collect::<Vec<_>>();
		assert!(!holds_publications(&pages));
	}

	#[test]
	fn a_container_of_books_or_audio_holds_publications() {
		let books = vec![
			ArchiveMember {
				path: "cover.jpg".to_string(),
				byte_size: 1,
				is_dir: false,
			},
			ArchiveMember {
				path: "book.epub".to_string(),
				byte_size: 2,
				is_dir: false,
			},
		];
		assert!(holds_publications(&books));

		let audio = vec![ArchiveMember {
			path: "book/01.mp3".to_string(),
			byte_size: 2,
			is_dir: false,
		}];
		assert!(holds_publications(&audio));
	}

	#[test]
	fn a_member_path_that_escapes_is_refused_rather_than_sanitised() {
		assert_eq!(
			safe_member_path("a/b.epub"),
			Some(PathBuf::from("a/b.epub"))
		);
		assert_eq!(safe_member_path("./a.epub"), Some(PathBuf::from("a.epub")));
		assert_eq!(safe_member_path("../a.epub"), None);
		assert_eq!(safe_member_path("/etc/passwd"), None);
		assert_eq!(safe_member_path("a/../../b"), None);
		assert_eq!(safe_member_path(""), None);
	}

	/// Both `unrar l` and `7z l` are parsed by the same rule: the rows
	/// between the two `-----` rules, name from the fifth column on so a
	/// member with spaces in its name survives.
	#[test]
	fn external_listings_parse_names_with_spaces_and_mark_directories() {
		let unrar = "\nUNRAR 7.23 freeware\n\nArchive: x.rar\n\n Attributes      Size     Date   Time   Name\n----------- ---------- ---------- -----  ----\n    ..A....   15559021 2019-04-02 19:40  Three Body/01 - Chapter 1.mp3\n    ...D...          0 2019-04-02 19:40  Three Body\n----------- ---------- ---------- -----  ----\n            15559021                    2\n";
		let members = parse_listing(unrar);
		assert_eq!(
			members,
			vec![
				ArchiveMember {
					path: "Three Body/01 - Chapter 1.mp3".to_string(),
					byte_size: 15_559_021,
					is_dir: false,
				},
				ArchiveMember {
					path: "Three Body".to_string(),
					byte_size: 0,
					is_dir: true,
				},
			]
		);
		assert!(holds_publications(&members));
	}
}
