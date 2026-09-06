use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::contract::{
	BookSnapshot, IngestMediaKind, QualityCheck, QualityCheckError, QualityStatus,
};

use super::{
	archive_images, canonical_page, disabled_outcome, enabled_setting, epub_spine_len,
	first_decodable_page, image_digest, outcome, QUALITY_VERSION,
};

/// Verifies that the cover is not duplicated as page two.
#[derive(Default)]
pub struct CoverNotPageTwoCheck;

impl CoverNotPageTwoCheck {
	pub fn new() -> Self {
		Self
	}
}

#[async_trait::async_trait]
impl QualityCheck for CoverNotPageTwoCheck {
	fn id(&self) -> &'static str {
		"cover_not_page_two"
	}

	fn name(&self) -> &'static str {
		"Cover is not page two"
	}

	fn version(&self) -> &'static str {
		QUALITY_VERSION
	}

	fn weight(&self) -> u16 {
		10
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
		if matches!(book.media_kind, IngestMediaKind::Unknown) {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({"applicable": false, "resource_count": 0}),
			));
		}

		let resource_count = match book.media_kind {
			IngestMediaKind::ComicArchive => archive_images(&book.staged_path)
				.map(|images| images.len())
				.unwrap_or_else(|_| {
					book.pages.iter().filter(|page| page.is_image).count()
				}),
			IngestMediaKind::Epub => {
				epub_spine_len(&book.staged_path).unwrap_or(book.pages.len())
			},
			_ => book.pages.len(),
		};
		if resource_count < 2 {
			return Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::NotApplicable,
				0.0,
				serde_json::json!({"applicable": false, "resource_count": resource_count}),
			));
		}

		let cover = first_decodable_page(book).ok().flatten();
		let second = canonical_page(book, 1).ok().flatten();
		let cover_digest = cover
			.as_ref()
			.and_then(|(_, bytes)| image_digest(bytes).ok());
		let second_digest = second
			.as_ref()
			.and_then(|(_, bytes)| image_digest(bytes).ok());
		let evidence = serde_json::json!({
			"resource_count": resource_count,
			"cover_path": cover.as_ref().map(|(path, _)| path),
			"second_path": second.as_ref().map(|(path, _)| path),
			"cover_digest": cover_digest,
			"second_digest": second_digest,
		});

		match (cover_digest, second_digest) {
			(Some(cover_digest), Some(second_digest))
				if cover_digest != second_digest =>
			{
				Ok(outcome(
					self.id(),
					self.name(),
					QualityStatus::Pass,
					1.0,
					evidence,
				))
			},
			(Some(_), Some(_)) => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Fail,
				0.0,
				evidence,
			)),
			_ => Ok(outcome(
				self.id(),
				self.name(),
				QualityStatus::Warn,
				0.5,
				evidence,
			)),
		}
	}
}
