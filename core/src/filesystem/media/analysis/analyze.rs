use crate::{
	filesystem::media::analysis::job::{AnalyzeMediaJob, AnalyzeMediaOutput},
	job::JobServices,
};
use stump_jobs::{JobContext, JobError, JobExecuteLog, JobProgress, JobTaskOutput};
use stump_media::media::{analyze_page, get_page, AnalyzedPage};

use std::{
	collections::HashMap,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	},
};

use futures::{stream, StreamExt};
use models::{
	entity::{media_analysis, page_hash},
	shared::analysis::{MediaAnalysisData, PageDimension},
};
use sea_orm::{
	sea_query::OnConflict, ActiveValue::Set, ColumnTrait, EntityTrait, FromQueryResult,
	QueryFilter,
};

#[derive(Debug, Clone, FromQueryResult)]
pub struct MediaForProcessing {
	pub id: String,
	pub path: String,
	pub pages: i32,
	pub page_count: Option<i32>,
}

struct BookPageAnalysisOutput {
	dimensions: PageDimension,
	content_type: String,
	/// Freshly computed perceptual hash; `None` when the page already had one,
	/// is not an image, or could not be decoded.
	dhash: Option<i64>,
}

struct ExistingPageAnalysis {
	dimensions: Option<PageDimension>,
	content_type: Option<String>,
	dhash: Option<i64>,
}

impl ExistingPageAnalysis {
	fn new(
		existing_analysis: &Option<MediaAnalysisData>,
		existing_hashes: &HashMap<i32, i64>,
		page: i32,
	) -> Self {
		let mut this = ExistingPageAnalysis {
			dimensions: None,
			content_type: None,
			dhash: existing_hashes.get(&page).copied(),
		};

		let index = (page - 1) as usize;
		if let Some(analysis) = existing_analysis {
			if let Some(dimensions) = analysis.dimensions.get(index) {
				this.dimensions = Some(dimensions.clone());
			}
			if let Some(content_type) = analysis.content_types.get(index) {
				this.content_type = Some(content_type.clone());
			}
		}
		this
	}

	fn has_all(&self) -> bool {
		self.dimensions.is_some()
			&& self.content_type.is_some()
			&& (self.dhash.is_some()
				|| !self
					.content_type
					.as_deref()
					.is_some_and(|content_type| content_type.starts_with("image/")))
	}
}

/// Decode the page once and hash it; failures are logged, never fatal, so a
/// corrupt page still keeps its dimension record.
fn hash_page(path: &str, page: i32, config: &stump_media::MediaConfig) -> Option<i64> {
	match get_page(path, page, config) {
		Ok((content_type, bytes)) if content_type.is_image() => {
			match stump_media::page_dhash(&bytes) {
				Ok(hash) => Some(hash as i64),
				Err(error) => {
					tracing::warn!(?error, path, page, "Failed to decode page for hashing");
					None
				},
			}
		},
		Ok(_) => None,
		Err(error) => {
			tracing::warn!(?error, path, page, "Failed to read page for hashing");
			None
		},
	}
}

async fn analyze_book_page(
	path: String,
	page: i32,
	existing_analysis: ExistingPageAnalysis,
	ctx: &JobContext<JobServices>,
	force_reanalysis: bool,
) -> Result<BookPageAnalysisOutput, JobError> {
	// If we aren't force reanalyzing and we have all of the things we need, return early
	if !force_reanalysis && existing_analysis.has_all() {
		return Ok(BookPageAnalysisOutput {
			dimensions: existing_analysis.dimensions.unwrap(),
			content_type: existing_analysis.content_type.unwrap(),
			dhash: None,
		});
	}

	let config_owned = ctx.config().media.clone();
	let path_owned = path.clone();
	let needs_hash = force_reanalysis || existing_analysis.dhash.is_none();
	let (analyzed, dhash) = tokio::task::spawn_blocking(move || {
		let analyzed = analyze_page(&path_owned, page, &config_owned)?;
		let dhash = (needs_hash && analyzed.content_type.is_image())
			.then(|| hash_page(&path_owned, page, &config_owned))
			.flatten();
		Ok::<_, stump_media::FileError>((analyzed, dhash))
	})
	.await
	.map_err(|e| JobError::Unknown(e.to_string()))?
	.map_err(|e| JobError::TaskFailed(e.to_string()))?;

	let AnalyzedPage {
		content_type,
		height,
		width,
	} = analyzed;
	let dimensions = PageDimension { height, width };

	Ok(BookPageAnalysisOutput {
		dimensions,
		content_type: content_type.mime_type(),
		dhash,
	})
}

pub async fn safely_analyze_book(
	book: MediaForProcessing,
	existing_analysis: Option<MediaAnalysisData>,
	ctx: &JobContext<JobServices>,
	force_reanalysis: bool,
) -> JobTaskOutput<AnalyzeMediaJob> {
	let mut output = AnalyzeMediaOutput::default();
	let mut logs = Vec::new();

	let page_count = book.page_count.unwrap_or(book.pages);

	let mut image_dimensions: Vec<PageDimension> =
		Vec::with_capacity(page_count as usize);
	let mut content_types: Vec<String> = Vec::with_capacity(page_count as usize);
	let mut new_hashes: Vec<(i32, i64)> = Vec::new();

	let existing_hashes: HashMap<i32, i64> = if force_reanalysis {
		HashMap::new()
	} else {
		match page_hash::Entity::find()
			.filter(page_hash::Column::MediaId.eq(book.id.as_str()))
			.all(ctx.conn())
			.await
		{
			Ok(rows) => rows.into_iter().map(|row| (row.page, row.dhash)).collect(),
			Err(error) => {
				tracing::warn!(?error, book_id = %book.id, "Failed to load page hashes");
				HashMap::new()
			},
		}
	};
	let existing_hashes = Arc::new(existing_hashes);

	// TODO: Make this configurable
	let concurrency = 10;
	let completed = Arc::new(AtomicUsize::new(0));

	ctx.report_progress(JobProgress::subtask_position(0, page_count));

	let mut page_stream = stream::iter(1..=page_count)
		.map(|page_num| {
			let book_path = book.path.clone();
			let existing = existing_analysis.clone();
			let existing_hashes = existing_hashes.clone();
			let completed = completed.clone();

			async move {
				let analysis = analyze_book_page(
					book_path,
					page_num,
					ExistingPageAnalysis::new(&existing, &existing_hashes, page_num),
					ctx,
					force_reanalysis,
				)
				.await;
				let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
				ctx.report_progress(JobProgress::subtask_position(
					count as i32,
					page_count,
				));

				(page_num, analysis)
			}
		})
		.buffered(concurrency);

	while let Some((page_num, analysis)) = page_stream.next().await {
		match analysis {
			Ok(result) => {
				image_dimensions.push(result.dimensions);
				content_types.push(result.content_type);
				if let Some(dhash) = result.dhash {
					new_hashes.push((page_num, dhash));
				}
				output.pages_analyzed += 1;
			},
			Err(err) => {
				// TODO: This is REALLY tricky. We rely on the vecs to fully encapsulate the pages, i.e. an elem
				// per page. If we skip a page due to an error, we will have misaligned data. It might be better to
				// either:
				// 1. Use an enum and push a "failed" variant to the vecs that resolves to None later
				// 2. Restructure to use a map of page number to analysis data
				// This requires rethinking the entire analysis storage structure, so for now we will just log the error
				tracing::error!(
					?err,
					book_id = %book.id,
					"Failed to analyze page {}/{}",
					page_num,
					page_count
				);
				logs.push(
					JobExecuteLog::error(format!(
						"Failed to analyze page {}/{}",
						page_num, page_count
					))
					.with_ctx(book.id.clone()),
				);
			},
		}
	}

	let constructed_analysis = MediaAnalysisData {
		dimensions: image_dimensions,
		content_types,
	};

	if !new_hashes.is_empty() {
		let created_at = chrono::Utc::now().into();
		let rows = new_hashes
			.into_iter()
			.map(|(page, dhash)| page_hash::ActiveModel {
				media_id: Set(book.id.clone()),
				page: Set(page),
				dhash: Set(dhash),
				created_at: Set(created_at),
			})
			.collect::<Vec<_>>();
		if let Err(error) = page_hash::Entity::insert_many(rows)
			.on_conflict(
				OnConflict::columns([page_hash::Column::MediaId, page_hash::Column::Page])
					.update_columns([page_hash::Column::Dhash, page_hash::Column::CreatedAt])
					.to_owned(),
			)
			.exec(ctx.conn())
			.await
		{
			tracing::error!(?error, book_id = %book.id, "Failed to write page hashes");
			logs.push(
				JobExecuteLog::error(format!("Failed to write page hashes: {error}"))
					.with_ctx(book.id.clone()),
			);
		} else {
			ctx.invalidate_visible_pages(&book.id);
		}
	}

	match &existing_analysis {
		Some(existing) if existing == &constructed_analysis => {
			// No changes, do nothing
			tracing::trace!(
                book_id = %book.id,
                "No changes detected in analysis, skipping database write");
			return JobTaskOutput {
				output,
				logs,
				subtasks: vec![],
			};
		},

		_ => {},
	}

	let model = media_analysis::ActiveModel {
		media_id: Set(book.id.clone()),
		data: Set(constructed_analysis),
		..Default::default()
	};

	if let Err(e) = media_analysis::Entity::insert(model)
		.on_conflict(
			OnConflict::columns([media_analysis::Column::MediaId])
				.update_columns([media_analysis::Column::Data])
				.to_owned(),
		)
		.exec(ctx.conn())
		.await
	{
		tracing::error!(
			?e,
			book_id = %book.id,
			"Failed to write analysis data to database"
		);
		logs.push(
			JobExecuteLog::error(format!(
				"Failed to write analysis data to database: {}",
				e
			))
			.with_ctx(book.id.clone()),
		);
	} else {
		tracing::trace!(book_id = %book.id, "Successfully wrote page analysis to database");
	}

	JobTaskOutput {
		output,
		logs,
		subtasks: vec![],
	}
}
