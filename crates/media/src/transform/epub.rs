//! Deterministic EPUB preparation for CrossPoint Reader displays.
//!
//! This is an independent Rust implementation of the behaviour observed in
//! the pinned MIT CrossPoint web/calibre clients.  It never mutates the source
//! EPUB.  The default options are disabled at the call site, so an ordinary
//! delivery sends source bytes unchanged; when enabled, raster images are
//! fitted to an X3/X4 display, optionally cropped/grayscaled, and written as
//! JPEG while the OCF package and text remain intact.

use std::{
	collections::{BTreeMap, HashSet},
	io::{Cursor, Read, Write},
	path::Path,
};

use image::{
	codecs::jpeg::JpegEncoder,
	imageops::{self, FilterType},
	DynamicImage, GenericImageView, ImageBuffer, RgbaImage,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// Profile selected by a verified CrossPoint model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum EpubDeviceTarget {
	Auto,
	X3,
	X4,
}

impl Default for EpubDeviceTarget {
	fn default() -> Self {
		Self::Auto
	}
}

impl EpubDeviceTarget {
	/// Resolve `Auto` against `/api/status`'s model; unknown/missing models
	/// intentionally use the plugin's conservative X4 default.
	pub fn resolve(self, detected_model: Option<&str>) -> EpubDeviceProfile {
		let target = match self {
			Self::X3 => Self::X3,
			Self::X4 => Self::X4,
			Self::Auto => match detected_model
				.unwrap_or_default()
				.trim()
				.to_ascii_uppercase()
				.as_str()
			{
				"X3" => Self::X3,
				_ => Self::X4,
			},
		};
		match target {
			Self::X3 => EpubDeviceProfile {
				target,
				width: 528,
				height: 792,
			},
			Self::X4 | Self::Auto => EpubDeviceProfile {
				target: Self::X4,
				width: 480,
				height: 800,
			},
		}
	}
}

/// Concrete dimensions used for one optimization run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpubDeviceProfile {
	pub target: EpubDeviceTarget,
	pub width: u32,
	pub height: u32,
}

/// CrossPoint optimizer controls.  `enabled` defaults to false so callers
/// must opt into byte-changing delivery explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EpubOptimizerOptions {
	pub enabled: bool,
	pub device_target: EpubDeviceTarget,
	pub jpeg_quality: u8,
	pub grayscale: bool,
	pub auto_crop: bool,
	pub split_text: bool,
	pub remove_fonts: bool,
}

impl Default for EpubOptimizerOptions {
	fn default() -> Self {
		Self {
			enabled: false,
			device_target: EpubDeviceTarget::Auto,
			jpeg_quality: 85,
			grayscale: true,
			auto_crop: false,
			split_text: true,
			remove_fonts: true,
		}
	}
}

impl EpubOptimizerOptions {
	pub fn validated(mut self) -> Result<Self, EpubOptimizationError> {
		if self.jpeg_quality == 0 {
			return Err(EpubOptimizationError::InvalidOptions(
				"JPEG quality must be in 1..=100".to_string(),
			));
		}
		self.jpeg_quality = self.jpeg_quality.min(100);
		Ok(self)
	}

	/// Stable identity for the byte-affecting options.
	pub fn digest(&self, detected_model: Option<&str>) -> String {
		let profile = self.device_target.resolve(detected_model);
		let mut hasher = Sha256::new();
		hasher.update(b"stump-crosspoint-epub-optimizer-v1\0");
		hasher.update(serde_json::to_vec(self).expect("optimizer options serialize"));
		hasher.update(serde_json::to_vec(&profile).expect("device profile serialize"));
		hex_digest(&hasher.finalize())[..16].to_string()
	}
}

/// Summary of an optimization run and the exact profile identity used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpubOptimizationSummary {
	pub source_bytes: u64,
	pub output_bytes: u64,
	pub images: u32,
	pub cropped_images: u32,
	pub image_errors: u32,
	pub text_fixes: u32,
	pub removed_fonts: u32,
	pub profile: EpubDeviceProfile,
	pub profile_digest: String,
	pub optimized: bool,
}

#[derive(Debug, Error)]
pub enum EpubOptimizationError {
	#[error("invalid EPUB optimizer options: {0}")]
	InvalidOptions(String),
	#[error("EPUB archive error: {0}")]
	Archive(String),
	#[error("EPUB has no valid mimetype entry")]
	MissingMimetype,
	#[error("EPUB source I/O failed: {0}")]
	Io(#[from] std::io::Error),
	#[error("EPUB image processing failed: {0}")]
	Image(String),
	#[error("EPUB XML rewrite failed: {0}")]
	Xml(String),
}

impl From<zip::result::ZipError> for EpubOptimizationError {
	fn from(error: zip::result::ZipError) -> Self {
		Self::Archive(error.to_string())
	}
}

/// Stateless optimizer.  `options.enabled == false` is an exact byte copy.
#[derive(Debug, Clone)]
pub struct EpubOptimizer {
	options: EpubOptimizerOptions,
}

impl EpubOptimizer {
	pub fn new(options: EpubOptimizerOptions) -> Result<Self, EpubOptimizationError> {
		Ok(Self {
			options: options.validated()?,
		})
	}

	pub fn options(&self) -> &EpubOptimizerOptions {
		&self.options
	}

	pub fn profile(&self, detected_model: Option<&str>) -> EpubDeviceProfile {
		self.options.device_target.resolve(detected_model)
	}

	pub fn profile_digest(&self, detected_model: Option<&str>) -> String {
		self.options.digest(detected_model)
	}

	/// Optimize a source path into a destination path without modifying the
	/// source.  Reading all source bytes before creating the destination also
	/// makes an accidental same-path invocation safe.
	pub fn optimize_path(
		&self,
		source: &Path,
		destination: &Path,
		detected_model: Option<&str>,
	) -> Result<EpubOptimizationSummary, EpubOptimizationError> {
		let bytes = std::fs::read(source)?;
		let (output, mut summary) = self.optimize_bytes(&bytes, detected_model)?;
		if source == destination {
			return Err(EpubOptimizationError::Io(std::io::Error::new(
				std::io::ErrorKind::InvalidInput,
				"optimizer destination must differ from source",
			)));
		}
		std::fs::write(destination, &output)?;
		summary.output_bytes = output.len() as u64;
		Ok(summary)
	}

	/// Optimize bytes in memory.  Queue callers can stage the returned bytes
	/// in a delivery cache; source media are never rewritten.
	pub fn optimize_bytes(
		&self,
		source: &[u8],
		detected_model: Option<&str>,
	) -> Result<(Vec<u8>, EpubOptimizationSummary), EpubOptimizationError> {
		let profile = self.profile(detected_model);
		let profile_digest = self.profile_digest(detected_model);
		if !self.options.enabled {
			return Ok((
				source.to_vec(),
				EpubOptimizationSummary {
					source_bytes: source.len() as u64,
					output_bytes: source.len() as u64,
					images: 0,
					cropped_images: 0,
					image_errors: 0,
					text_fixes: 0,
					removed_fonts: 0,
					profile,
					profile_digest,
					optimized: false,
				},
			));
		}
		optimize_archive(source, &self.options, profile, profile_digest)
	}
}

/// Convenience wrapper for callers that do not need to retain an optimizer.
pub fn optimize_epub(
	source: &Path,
	destination: &Path,
	options: EpubOptimizerOptions,
	detected_model: Option<&str>,
) -> Result<EpubOptimizationSummary, EpubOptimizationError> {
	EpubOptimizer::new(options)?.optimize_path(source, destination, detected_model)
}

fn optimize_archive(
	source: &[u8],
	options: &EpubOptimizerOptions,
	profile: EpubDeviceProfile,
	profile_digest: String,
) -> Result<(Vec<u8>, EpubOptimizationSummary), EpubOptimizationError> {
	let mut archive = ZipArchive::new(Cursor::new(source))?;
	let mut entries = Vec::with_capacity(archive.len());
	let mut mimetype_count = 0_u32;
	let mut image_outputs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
	let mut successful_renames = BTreeMap::new();
	let mut image_count = 0_u32;
	let mut cropped_count = 0_u32;
	let mut image_errors = 0_u32;

	for index in 0..archive.len() {
		let mut entry = archive.by_index(index)?;
		let name = entry.name().replace('\\', "/");
		if name.is_empty() || name.ends_with('/') {
			continue;
		}
		if name == "mimetype" {
			mimetype_count += 1;
		}
		let mut bytes = Vec::with_capacity(entry.size() as usize);
		entry.read_to_end(&mut bytes)?;
		if is_raster_path(&name) {
			match process_image(&bytes, &name, options, profile) {
				Ok((jpeg, cropped)) => {
					image_count += 1;
					cropped_count += u32::from(cropped);
					let output_name = renamed_path(&name);
					successful_renames.insert(name.clone(), output_name.clone());
					image_outputs.insert(output_name, jpeg);
					// Retain the source slot so the transformed image is
					// emitted in deterministic archive order below.
					entries.push((name, bytes));
				},
				Err(_) => {
					image_errors += 1;
					entries.push((name, bytes));
				},
			}
		} else {
			entries.push((name, bytes));
		}
	}
	if mimetype_count != 1 {
		return Err(EpubOptimizationError::MissingMimetype);
	}
	let mimetype = entries
		.iter()
		.find(|(name, _)| name == "mimetype")
		.map(|(_, bytes)| bytes.as_slice())
		.ok_or(EpubOptimizationError::MissingMimetype)?;
	if mimetype != b"application/epub+zip" {
		return Err(EpubOptimizationError::MissingMimetype);
	}

	// Add successfully transformed images back at their original ordering
	// slots.  A failed decode retains the source filename and bytes.
	let mut rebuilt = Vec::with_capacity(entries.len());
	let mut emitted_images = HashSet::new();
	let mut text_fixes = 0_u32;
	let mut removed_fonts = 0_u32;
	let identifier = entries
		.iter()
		.find(|(name, _)| name.to_ascii_lowercase().ends_with(".opf"))
		.and_then(|(_, bytes)| decode_text(bytes).ok())
		.and_then(|opf| extract_identifier(&opf));

	for (name, mut bytes) in entries {
		if name == "mimetype" {
			rebuilt.push((name, bytes));
			continue;
		}
		if let Some(output_name) = successful_renames.get(&name) {
			// Do not emit the old source image; output order follows the source
			// archive and each image is stored uncompressed.
			if let Some(image) = image_outputs.get(output_name) {
				rebuilt.push((output_name.clone(), image.clone()));
				emitted_images.insert(output_name.clone());
			}
			continue;
		}
		let lower = name.to_ascii_lowercase();
		if lower.ends_with(".opf") {
			let text = decode_text(&bytes)?;
			let rewritten = rewrite_opf(&text, &successful_renames, options.remove_fonts);
			text_fixes += u32::from(rewritten != text);
			bytes = rewritten.into_bytes();
		} else if is_xhtml_path(&lower) {
			let text = decode_text(&bytes)?;
			let (rewritten, fixes) = rewrite_xhtml(&text, &successful_renames);
			text_fixes += fixes;
			bytes = rewritten.into_bytes();
		} else if lower.ends_with(".ncx") {
			let text = decode_text(&bytes)?;
			let rewritten = rewrite_ncx(
				&rewrite_references(&text, &successful_renames),
				identifier.as_deref(),
			);
			text_fixes += u32::from(rewritten != text);
			bytes = rewritten.into_bytes();
		} else if lower.ends_with(".css") {
			let text = decode_text(&bytes)?;
			let mut rewritten = rewrite_references(&text, &successful_renames);
			if options.remove_fonts {
				let (without_fonts, removed) = remove_font_css(&rewritten);
				rewritten = without_fonts;
				removed_fonts += removed;
			}
			text_fixes += u32::from(rewritten != text);
			bytes = rewritten.into_bytes();
		} else if options.remove_fonts && is_font_path(&lower) {
			removed_fonts += 1;
			continue;
		}
		rebuilt.push((name, bytes));
	}
	// No image should disappear simply because a duplicate-name ZIP entry was
	// unusual; emitted outputs are already present at source positions.
	for (name, bytes) in image_outputs {
		if !emitted_images.contains(&name) {
			rebuilt.push((name, bytes));
		}
	}

	if options.split_text {
		let (split, fixes) = split_large_paragraphs(rebuilt);
		rebuilt = split;
		text_fixes += fixes;
	}

	let mut output = Cursor::new(Vec::new());
	{
		let mut writer = ZipWriter::new(&mut output);
		let stored = zip_options(CompressionMethod::Stored);
		let deflated = zip_options(CompressionMethod::Deflated);
		let mimetype = rebuilt
			.iter()
			.find(|(name, _)| name == "mimetype")
			.map(|(_, bytes)| bytes.as_slice())
			.ok_or(EpubOptimizationError::MissingMimetype)?;
		writer.start_file("mimetype", stored)?;
		writer.write_all(mimetype)?;
		for (name, bytes) in rebuilt {
			if name == "mimetype" {
				continue;
			}
			let options = if is_output_image(&name) {
				stored
			} else {
				deflated
			};
			writer.start_file(name, options)?;
			writer.write_all(&bytes)?;
		}
		writer.finish()?;
	}
	let bytes = output.into_inner();
	Ok((
		bytes.clone(),
		EpubOptimizationSummary {
			source_bytes: source.len() as u64,
			output_bytes: bytes.len() as u64,
			images: image_count,
			cropped_images: cropped_count,
			image_errors,
			text_fixes,
			removed_fonts,
			profile,
			profile_digest,
			optimized: true,
		},
	))
}

fn zip_options(method: CompressionMethod) -> SimpleFileOptions {
	SimpleFileOptions::default()
		.compression_method(method)
		.last_modified_time(DateTime::default())
		.unix_permissions(0o644)
}

fn is_raster_path(path: &str) -> bool {
	[".png", ".gif", ".webp", ".bmp", ".jpeg", ".jpg"]
		.iter()
		.any(|suffix| path.to_ascii_lowercase().ends_with(suffix))
}

fn is_output_image(path: &str) -> bool {
	path.to_ascii_lowercase().ends_with(".jpg")
}

fn is_xhtml_path(path: &str) -> bool {
	[".xhtml", ".html", ".htm"]
		.iter()
		.any(|suffix| path.ends_with(suffix))
}

fn is_font_path(path: &str) -> bool {
	[".otf", ".ttf", ".woff", ".woff2"]
		.iter()
		.any(|suffix| path.ends_with(suffix))
}

fn renamed_path(path: &str) -> String {
	for suffix in [".png", ".gif", ".webp", ".bmp", ".jpeg"] {
		if path.to_ascii_lowercase().ends_with(suffix) {
			return format!("{}.jpg", &path[..path.len() - suffix.len()]);
		}
	}
	path.to_string()
}

fn decode_text(bytes: &[u8]) -> Result<String, EpubOptimizationError> {
	let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
	std::str::from_utf8(bytes)
		.map(str::to_owned)
		.map_err(|error| EpubOptimizationError::Xml(error.to_string()))
}

fn process_image(
	bytes: &[u8],
	path: &str,
	options: &EpubOptimizerOptions,
	profile: EpubDeviceProfile,
) -> Result<(Vec<u8>, bool), EpubOptimizationError> {
	let image = image::load_from_memory(bytes)
		.map_err(|error| EpubOptimizationError::Image(error.to_string()))?;
	let (width, height) = image.dimensions();
	let mut cropped = false;
	let mut source = flatten_white(&image);
	if options.auto_crop && width >= 240 && height >= 240 && !is_cover_name(path) {
		if let Some(box_) = crop_box(&source) {
			source =
				imageops::crop_imm(&source, box_.0, box_.1, box_.2, box_.3).to_image();
			cropped = true;
		}
	}
	let (source_width, source_height) = source.dimensions();
	let (final_width, final_height) =
		fit_dimensions(source_width, source_height, profile.width, profile.height);
	let scaled = if (final_width, final_height) == (source_width, source_height) {
		source
	} else {
		imageops::resize(&source, final_width, final_height, FilterType::Lanczos3)
	};
	let mut output = Vec::new();
	if options.grayscale {
		let gray = DynamicImage::ImageRgba8(scaled).into_luma8();
		JpegEncoder::new_with_quality(&mut output, options.jpeg_quality)
			.encode_image(&gray)
	} else {
		let rgb = DynamicImage::ImageRgba8(scaled).into_rgb8();
		JpegEncoder::new_with_quality(&mut output, options.jpeg_quality)
			.encode_image(&rgb)
	}
	.map_err(|error| EpubOptimizationError::Image(error.to_string()))?;
	Ok((output, cropped))
}

fn flatten_white(image: &DynamicImage) -> RgbaImage {
	let rgba = image.to_rgba8();
	let mut output = ImageBuffer::from_pixel(
		rgba.width(),
		rgba.height(),
		image::Rgba([255, 255, 255, 255]),
	);
	imageops::overlay(&mut output, &rgba, 0, 0);
	output
}

fn fit_dimensions(
	width: u32,
	height: u32,
	max_width: u32,
	max_height: u32,
) -> (u32, u32) {
	if width <= max_width && height <= max_height {
		return (width.max(1), height.max(1));
	}
	let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64);
	(
		((width as f64 * scale).round() as u32).max(1),
		((height as f64 * scale).round() as u32).max(1),
	)
}

fn is_cover_name(path: &str) -> bool {
	let basename = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
	["cover", "thumbnail", "thumb", "icon"]
		.iter()
		.any(|prefix| basename.starts_with(prefix))
}

fn crop_box(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
	let width = image.width();
	let height = image.height();
	let samples = [
		(0, 0),
		(width.saturating_sub(12), 0),
		(0, height.saturating_sub(12)),
		(width.saturating_sub(12), height.saturating_sub(12)),
	];
	let mut background = [0_u32; 3];
	let mut count = 0_u32;
	for (x, y) in samples {
		let pixel = image.get_pixel(x, y).0;
		for channel in 0..3 {
			background[channel] += pixel[channel] as u32;
		}
		count += 1;
	}
	let background = [
		(background[0] / count) as i16,
		(background[1] / count) as i16,
		(background[2] / count) as i16,
	];
	let near_white = background.iter().all(|channel| *channel >= 245);
	let threshold = if near_white { 10 } else { 28 };
	let mut left = width;
	let mut top = height;
	let mut right = 0_u32;
	let mut bottom = 0_u32;
	for y in 0..height {
		for x in 0..width {
			let pixel = image.get_pixel(x, y).0;
			let differs = (0..3).any(|channel| {
				(pixel[channel] as i16 - background[channel]).unsigned_abs() as i16
					> threshold
			});
			if differs {
				left = left.min(x);
				top = top.min(y);
				right = right.max(x + 1);
				bottom = bottom.max(y + 1);
			}
		}
	}
	if right <= left || bottom <= top {
		return None;
	}
	let padding = 8;
	left = left.saturating_sub(padding);
	top = top.saturating_sub(padding);
	right = (right + padding).min(width);
	bottom = (bottom + padding).min(height);
	let area = width as u64 * height as u64;
	let crop_area = (right - left) as u64 * (bottom - top) as u64;
	if area == 0 || (area - crop_area) * 100 < area * 8 {
		return None;
	}
	Some((left, top, right - left, bottom - top))
}

fn rewrite_references(text: &str, renames: &BTreeMap<String, String>) -> String {
	let mut output = text.to_string();
	// Replace full package paths and basenames.  Restricting replacements to
	// known successful images means a failed decode never leaves a dangling
	// `.jpg` reference.
	for (old, new) in renames {
		if old != new {
			output = output.replace(old, new);
			if let (Some(old_base), Some(new_base)) =
				(old.rsplit('/').next(), new.rsplit('/').next())
			{
				output = output.replace(old_base, new_base);
			}
		}
	}
	output
}

fn rewrite_xhtml(text: &str, renames: &BTreeMap<String, String>) -> (String, u32) {
	let mut rewritten = rewrite_references(text, renames);
	let mut fixes = u32::from(rewritten != text);
	let image_re = Regex::new(r"(?is)<img\b[^>]*>").expect("valid image element regex");
	let dimension_re = Regex::new(r#"(?i)\s+(?:width|height)\s*=\s*["'][^"']*["']"#)
		.expect("valid image dimension regex");
	let style_re = Regex::new(r#"(?i)\s+style\s*=\s*["']([^"']*)["']"#)
		.expect("valid image style regex");
	rewritten = image_re
		.replace_all(&rewritten, |captures: &regex::Captures<'_>| {
			let original = captures
				.get(0)
				.map(|value| value.as_str())
				.unwrap_or_default();
			let mut image = dimension_re.replace_all(original, "").into_owned();
			if let Some(style) = style_re.captures(&image) {
				let existing =
					style.get(1).map(|value| value.as_str()).unwrap_or_default();
				let mut declarations = existing.trim().trim_end_matches(';').to_owned();
				if !declarations.is_empty() {
					declarations.push(';');
				}
				if !existing.to_ascii_lowercase().contains("max-width") {
					declarations.push_str("max-width:100%;");
				}
				if !existing.to_ascii_lowercase().contains("height") {
					declarations.push_str("height:auto;");
				}
				image = style_re
					.replace(&image, format!(r#" style="{declarations}""#))
					.into_owned();
			} else {
				let insertion = r#" style="max-width:100%;height:auto;""#;
				if let Some(offset) = image.rfind("/>") {
					image.insert_str(offset, insertion);
				} else if let Some(offset) = image.rfind('>') {
					image.insert_str(offset, insertion);
				}
			}
			if image != original {
				fixes = fixes.saturating_add(1);
			}
			image
		})
		.into_owned();
	(rewritten, fixes)
}

fn rewrite_opf(
	text: &str,
	renames: &BTreeMap<String, String>,
	remove_fonts: bool,
) -> String {
	let mut text = rewrite_references(text, renames);
	if remove_fonts {
		let font_item_re = Regex::new(
            r#"(?is)<(?:\w+:)?item\b[^>]*(?:href\s*=\s*["'][^"']*\.(?:otf|ttf|woff2?)["']|media-type\s*=\s*["']font/)[^>]*>"#,
        )
        .expect("valid font item regex");
		text = font_item_re.replace_all(&text, "").into_owned();
	}
	let item_re =
		Regex::new(r"(?is)<(?:\w+:)?item\b[^>]*>").expect("valid OPF item regex");
	text = item_re
		.replace_all(&text, |captures: &regex::Captures<'_>| {
			let mut item = captures
				.get(0)
				.map(|m| m.as_str())
				.unwrap_or_default()
				.to_string();
			let lower = item.to_ascii_lowercase();
			let image = lower.contains(".jpg") && lower.contains("image/");
			if image {
				let media_type = Regex::new(
					r#"(?i)\s+media-type\s*=\s*["']image/(?:png|gif|webp|bmp|jpeg|jpg)["']"#,
				)
				.expect("valid media type regex");
				item = media_type
					.replace(&item, r#" media-type="image/jpeg""#)
					.into_owned();
			}
			let properties = Regex::new(r#"(?i)\s+properties\s*=\s*["']([^"']*)["']"#)
				.expect("valid properties regex");
			properties
				.replace(&item, |props: &regex::Captures<'_>| {
					let value = props.get(1).map(|m| m.as_str()).unwrap_or_default();
					let filtered = value
						.split_whitespace()
						.filter(|token| *token != "svg")
						.collect::<Vec<_>>()
						.join(" ");
					if filtered.is_empty() {
						String::new()
					} else {
						format!(r#" properties="{filtered}""#)
					}
				})
				.into_owned()
		})
		.into_owned();
	ensure_cover_meta(&text)
}

fn ensure_cover_meta(text: &str) -> String {
	let item_re = Regex::new(r#"(?is)<(?:\w+:)?item\b[^>]*(?:properties\s*=\s*['\"][^'\"]*cover-image[^'\"]*['\"]|id\s*=\s*['\"][^'\"]*cover[^'\"]*['\"]|href\s*=\s*['\"][^'\"]*cover[^'\"]*['\"])[^>]*>"#).expect("valid cover item regex");
	let Some(item) = item_re.find(text) else {
		return text.to_string();
	};
	let id_re =
		Regex::new(r#"(?i)\bid\s*=\s*['\"]([^'\"]+)['\"]"#).expect("valid id regex");
	let Some(id) = id_re
		.captures(item.as_str())
		.and_then(|captures| captures.get(1))
	else {
		return text.to_string();
	};
	let cover_id = id.as_str();
	let meta_re = Regex::new(r#"(?is)<meta\b[^>]*\bname\s*=\s*['\"]cover['\"][^>]*>"#)
		.expect("valid cover meta regex");
	if let Some(meta) = meta_re.find(text) {
		let content_re = Regex::new(r#"(?i)\bcontent\s*=\s*["'][^"']*["']"#)
			.expect("valid content regex");
		let rewritten =
			content_re.replace(meta.as_str(), format!(r#" content="{cover_id}""#));
		return format!(
			"{}{}{}",
			&text[..meta.start()],
			rewritten,
			&text[meta.end()..]
		);
	}
	let metadata_re =
		Regex::new(r"(?is)</(?:\w+:)?metadata>").expect("valid metadata close regex");
	if let Some(metadata_end) = metadata_re.find(text) {
		let meta = format!(r#"<meta name="cover" content="{cover_id}"/>"#);
		return format!(
			"{}{}{}",
			&text[..metadata_end.start()],
			meta,
			&text[metadata_end.start()..]
		);
	}
	text.to_string()
}

fn extract_identifier(opf: &str) -> Option<String> {
	let package_re = Regex::new(
		r#"(?is)<(?:\w+:)?package\b[^>]*\bunique-identifier\s*=\s*['\"]([^'\"]+)['\"]"#,
	)
	.ok()?;
	let package_id = package_re
		.captures(opf)
		.and_then(|c| c.get(1))
		.map(|m| m.as_str().to_string());
	if let Some(package_id) = package_id {
		let identifier_re = Regex::new(&format!(r#"(?is)<(?:\w+:)?identifier\b[^>]*\bid\s*=\s*['\"]{}['\"][^>]*>(.*?)</(?:\w+:)?identifier>"#, regex::escape(&package_id))).ok()?;
		if let Some(value) = identifier_re.captures(opf).and_then(|c| c.get(1)) {
			return Some(value.as_str().trim().to_string());
		}
	}
	Regex::new(r"(?is)<(?:\w+:)?identifier\b[^>]*>(.*?)</(?:\w+:)?identifier>")
		.ok()?
		.captures(opf)
		.and_then(|c| c.get(1))
		.map(|m| m.as_str().trim().to_string())
}

fn rewrite_ncx(text: &str, identifier: Option<&str>) -> String {
	let Some(identifier) = identifier else {
		return text.to_string();
	};
	let meta_re =
		Regex::new(r#"(?is)(<meta\b[^>]*\bname\s*=\s*['\"]dtb:uid['\"][^>]*>)"#)
			.expect("valid NCX meta regex");
	meta_re
		.replace(text, |captures: &regex::Captures<'_>| {
			let original = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
			let content_re = Regex::new(r#"(?i)\bcontent\s*=\s*["'][^"']*["']"#)
				.expect("valid NCX content regex");
			content_re
				.replace(original, format!(r#"content="{identifier}""#))
				.into_owned()
		})
		.into_owned()
}

fn remove_font_css(text: &str) -> (String, u32) {
	let re =
		Regex::new(r"(?is)@font-face\s*\{[^}]*\}\s*").expect("valid font-face regex");
	let count = re.find_iter(text).count() as u32;
	(re.replace_all(text, "").into_owned(), count)
}

fn split_large_paragraphs(
	entries: Vec<(String, Vec<u8>)>,
) -> (Vec<(String, Vec<u8>)>, u32) {
	let paragraph_re =
		Regex::new(r"(?is)<p\b([^>]*)>(.*?)</p>").expect("valid paragraph regex");
	let mut fixes = 0_u32;
	let mut output = Vec::with_capacity(entries.len());
	for (name, bytes) in entries {
		if !is_xhtml_path(&name.to_ascii_lowercase()) {
			output.push((name, bytes));
			continue;
		}
		let Ok(text) = decode_text(&bytes) else {
			output.push((name, bytes));
			continue;
		};
		let rewritten = paragraph_re
			.replace_all(&text, |captures: &regex::Captures<'_>| {
				let attrs = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
				let inner = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
				if inner.len() <= 1600
					|| inner.contains("<table")
					|| inner.contains("<div")
				{
					return captures
						.get(0)
						.map(|m| m.as_str())
						.unwrap_or_default()
						.to_string();
				}
				let chunks = split_lossless(inner, 1200);
				if chunks.len() < 2 {
					return captures
						.get(0)
						.map(|m| m.as_str())
						.unwrap_or_default()
						.to_string();
				}
				fixes += 1;
				chunks
					.into_iter()
					.enumerate()
					.map(|(index, chunk)| {
						let suffix = if index == 0 {
							std::borrow::Cow::Borrowed(attrs)
						} else {
							std::borrow::Cow::Owned(strip_id(attrs))
						};
						format!("<p{suffix}>{chunk}</p>")
					})
					.collect::<String>()
			})
			.into_owned();
		output.push((name, rewritten.into_bytes()));
	}
	(output, fixes)
}

fn split_lossless(text: &str, target: usize) -> Vec<&str> {
	let mut chunks = Vec::new();
	let mut start = 0;
	while text.len() - start > target {
		let end = start + target;
		let relative = text[start..end]
			.rfind(". ")
			.map(|index| index + 2)
			.or_else(|| text[start..end].rfind(' ').map(|index| index + 1));
		let Some(relative) = relative else { break };
		let cut = start + relative;
		chunks.push(&text[start..cut]);
		start = cut;
	}
	chunks.push(&text[start..]);
	chunks
}

fn strip_id(attrs: &str) -> String {
	Regex::new(r#"(?i)\s+id\s*=\s*["'][^"']*["']"#)
		.expect("valid id attr regex")
		.replace(attrs, "")
		.into_owned()
}

fn hex_digest(bytes: &[u8]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use image::{Rgb, RgbImage};

	fn fixture(image: &[u8]) -> Vec<u8> {
		let mut output = Cursor::new(Vec::new());
		let mut writer = ZipWriter::new(&mut output);
		let stored = zip_options(CompressionMethod::Stored);
		let deflated = zip_options(CompressionMethod::Deflated);
		writer.start_file("mimetype", stored).unwrap();
		writer.write_all(b"application/epub+zip").unwrap();
		writer
			.start_file("META-INF/container.xml", deflated)
			.unwrap();
		writer.write_all(br#"<container><rootfiles><rootfile full-path="OEBPS/package.opf"/></rootfiles></container>"#).unwrap();
		writer.start_file("OEBPS/package.opf", deflated).unwrap();
		writer.write_all(br#"<package unique-identifier="uid"><metadata><dc:identifier id="uid">urn:test</dc:identifier></metadata><manifest><item id="cover-image" href="cover.png" media-type="image/png" properties="cover-image svg"/><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#).unwrap();
		writer.start_file("OEBPS/chapter.xhtml", deflated).unwrap();
		writer.write_all(br#"<html><head></head><body><p id="p">A very long paragraph that is deliberately repeated so the optimizer can preserve every visible character while splitting the paragraph into safe pieces. A very long paragraph that is deliberately repeated so the optimizer can preserve every visible character while splitting the paragraph into safe pieces.</p><img src="cover.png" width="100" height="100"/></body></html>"#).unwrap();
		writer.start_file("OEBPS/cover.png", stored).unwrap();
		writer.write_all(image).unwrap();
		writer.start_file("OEBPS/font.ttf", stored).unwrap();
		writer.write_all(b"font").unwrap();
		writer.finish().unwrap();
		drop(writer);
		output.into_inner()
	}

	#[test]
	fn defaults_are_off_and_off_is_byte_exact() {
		let source = fixture(b"not-an-image");
		let optimizer = EpubOptimizer::new(EpubOptimizerOptions::default()).unwrap();
		let (output, summary) = optimizer.optimize_bytes(&source, Some("X4")).unwrap();
		assert_eq!(source, output);
		assert!(!summary.optimized);
		assert_eq!(summary.profile.width, 480);
	}

	#[test]
	fn optimized_output_is_deterministic_and_mimetype_first() {
		let mut image = RgbImage::new(960, 1600);
		for pixel in image.pixels_mut() {
			*pixel = Rgb([20, 30, 40]);
		}
		let mut encoded = Vec::new();
		DynamicImage::ImageRgb8(image)
			.write_to(&mut Cursor::new(&mut encoded), image::ImageFormat::Png)
			.unwrap();
		let source = fixture(&encoded);
		let options = EpubOptimizerOptions {
			enabled: true,
			..Default::default()
		};
		let optimizer = EpubOptimizer::new(options).unwrap();
		let (one, first) = optimizer.optimize_bytes(&source, Some("X3")).unwrap();
		let (two, second) = optimizer.optimize_bytes(&source, Some("X3")).unwrap();
		assert_eq!(one, two);
		assert_eq!(first.profile_digest, second.profile_digest);
		let mut archive = ZipArchive::new(Cursor::new(one)).unwrap();
		assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
		assert!(archive.by_name("OEBPS/cover.jpg").is_ok());
		assert!(archive.by_name("OEBPS/font.ttf").is_err());
		let chapter = {
			let mut entry = archive.by_name("OEBPS/chapter.xhtml").unwrap();
			let mut bytes = Vec::new();
			entry.read_to_end(&mut bytes).unwrap();
			String::from_utf8(bytes).unwrap()
		};
		assert!(!chapter.contains("width=\"100\""));
		assert!(chapter.contains("max-width:100%"));
	}
}
