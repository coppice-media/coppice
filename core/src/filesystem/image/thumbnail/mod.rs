mod generate;
mod generation_job;
mod placeholder_job;

pub use generate::{
	bump_media_thumbnail_fallbacks, bump_series_thumbnail_fallbacks,
	generate_book_thumbnail, GenerateThumbnailOptions, ThumbnailGenerateError,
};
pub use generation_job::{
	ThumbnailGenerationJob, ThumbnailGenerationJobParams, ThumbnailGenerationJobScope,
	ThumbnailGenerationOutput,
};
pub use placeholder_job::{
	PlaceholderGenerationJob, PlaceholderGenerationJobConfig,
	PlaceholderGenerationJobScope, PlaceholderGenerationOutput,
};
