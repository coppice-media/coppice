use std::collections::BTreeMap;

use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_media::media::analyze_page;

use crate::contract::{
	BookSnapshot, IngestMediaKind, QualityCheck, QualityCheckError, QualityStatus,
};

use super::{
	archive_images, dimensions_from_analysis, disabled_outcome, enabled_setting,
	epub_spine_image_resource, epub_spine_len, image_dimensions, outcome,
	QUALITY_VERSION,
};

/// Verifies image dimensions consistency.
#[derive(Default)]
pub struct ImageDimensionsConsistentCheck;

impl ImageDimensionsConsistentCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for ImageDimensionsConsistentCheck {
	fn id(&self) -> &'static str {
		"image_dimensions_consistent"
	}

	fn name(&self) -> &'static str {
		"Image dimensions are consistent"
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
	) -> Result<crate::contract::QualityCheckOutcome, QualityCheckError> {
		if !enabled_setting(settings) {
			return Ok(disabled_outcome(self.id(), self.name()));
		}

		let mut dimensions = Vec::new();
		let mut image_count = 0usize;
		match book.media_kind {
			IngestMediaKind::ComicArchive => match archive_images(&book.staged_path) {
				Ok(images) => {
					image_count = images.len();
					for (_, bytes) in images {
						dimensions.push(image_dimensions(&bytes));
					}
				},
				Err(_) => {
					if let Some((count, analyzed)) = dimensions_from_analysis(book) {
						image_count = count;
						dimensions = analyzed;
					}
				},
			},
			IngestMediaKind::Epub => {
				let spine_len = epub_spine_len(&book.staged_path).unwrap_or(0);
				for index in 0..spine_len {
					if let Ok(Some((_, bytes))) =
						epub_spine_image_resource(&book.staged_path, index)
					{
						image_count += 1;
						dimensions.push(image_dimensions(&bytes));
					}
				}
			},
			IngestMediaKind::ComicRarArchive | IngestMediaKind::Pdf => {
				if let Some((count, analyzed)) = dimensions_from_analysis(book) {
					image_count = count;
					dimensions = analyzed;
				} else {
					image_count = book.pages.iter().filter(|page| page.is_image).count();
					let config = crate::config::IngestSettings::debug();
					for index in 0..image_count {
						let analyzed = analyze_page(
							book.staged_path.to_string_lossy().as_ref(),
							(index + 1) as i32,
							&config.media,
						);
						dimensions
							.push(analyzed.ok().map(|page| (page.width, page.height)));
					}
				}
			},
			// Neither has image pages to measure.
			IngestMediaKind::Audio | IngestMediaKind::Unknown => {},
		}

		if image_count < 2 {
			if let Some((analyzed_count, analyzed_dimensions)) =
				dimensions_from_analysis(book)
			{
				if analyzed_count >= 2 {
					image_count = analyzed_count;
					dimensions = analyzed_dimensions;
				}
			}
		}
		if image_count < 2 {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({"applicable": false, "image_count": image_count}),
			));
		}

		let mut frequencies = BTreeMap::<(u32, u32), usize>::new();
		for dimension in dimensions.iter().flatten() {
			*frequencies.entry(*dimension).or_default() += 1;
		}
		let modal = frequencies
			.into_iter()
			.max_by(
				|(left_dimension, left_count), (right_dimension, right_count)| {
					left_count
						.cmp(right_count)
						.then_with(|| right_dimension.cmp(left_dimension))
				},
			)
			.map(|(dimension, _)| dimension);
		let matching_pages = modal
			.map(|modal| {
				dimensions
					.iter()
					.filter(|dimension| **dimension == Some(modal))
					.count()
			})
			.unwrap_or(0);
		let ratio = matching_pages as f64 / image_count as f64;
		let (status, normalized_score) = if ratio == 1.0 {
			(QualityStatus::Pass, 1.0)
		} else if ratio >= 0.9 {
			(QualityStatus::Warn, ratio)
		} else {
			(QualityStatus::Fail, ratio)
		};
		let evidence = serde_json::json!({
			"image_count": image_count,
			"decoded_count": dimensions.iter().flatten().count(),
			"matching_pages": matching_pages,
			"ratio": ratio,
			"modal_dimensions": modal.map(|(width, height)| serde_json::json!({"width": width, "height": height})),
		});
		Ok(outcome(
			self.id(),
			self.name(),
			status,
			normalized_score,
			evidence,
		))
	}
}
