use std::env;

use serde::{Deserialize, Serialize};
use stump_config_gen::StumpConfigGenerator;

use super::{defaults::*, env_keys::*};

/// PDFium location and PDF page rendering settings. Flattened into [`super::StumpConfig`].
///
/// These are the raw inputs from which the `media` snapshot ([`stump_media::MediaConfig`])
/// is computed by [`super::StumpConfig::finalize`]; processors read the snapshot.
#[derive(StumpConfigGenerator, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
#[cfg_attr(feature = "graphql", graphql(name = "StumpPdfConfig"))]
pub struct PdfConfig {
	/// Path to the PDFium binary for PDF support.
	#[default_value(None)]
	#[env_key(PDFIUM_KEY)]
	pub pdfium_path: Option<String>,

	/// The DPI (dots per inch) to use when rendering PDF pages as images.
	#[default_value(DEFAULT_PDF_RENDER_DPI)]
	#[env_key(PDF_RENDER_DPI_KEY)]
	pub pdf_render_dpi: u32,

	/// The maximum width or height dimension for rendered PDF pages.
	#[default_value(DEFAULT_PDF_MAX_DIMENSION)]
	#[env_key(PDF_MAX_DIMENSION_KEY)]
	pub pdf_max_dimension: u32,

	/// The image format to use for rendered PDF pages (webp, png, jpeg).
	#[default_value(DEFAULT_PDF_RENDER_FORMAT.to_string())]
	#[env_key(PDF_RENDER_FORMAT_KEY)]
	pub pdf_render_format: String,

	/// Whether to enable disk caching for rendered PDF pages.
	#[default_value(DEFAULT_PDF_CACHE_PAGES)]
	#[env_key(PDF_CACHE_PAGES_KEY)]
	pub pdf_cache_pages: bool,

	/// Number of pages to pre-render before and after the current page.
	#[default_value(DEFAULT_PDF_PRERENDER_RANGE)]
	#[env_key(PDF_PRERENDER_RANGE_KEY)]
	pub pdf_prerender_range: u32,

	/// Whether to enable high-quality rendering with smoothing (slower but better quality).
	#[default_value(DEFAULT_PDF_HIGH_QUALITY)]
	#[env_key(PDF_HIGH_QUALITY_KEY)]
	pub pdf_high_quality: bool,
}
