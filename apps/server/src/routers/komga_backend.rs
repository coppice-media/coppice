use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
	body::Body,
	http::{HeaderMap, Response},
	response::IntoResponse,
};
use models::{
	entity::{media, series, user::AuthUser},
	shared::image_processor_options::SupportedImageFormat,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use stump_auth::AuthContext;
use stump_core::{job::stump_job::StumpJob, CoreEvent};
use stump_media::{
	image::{generate_image_metadata_from_bytes, GenericImageProcessor, ImageProcessor},
	media::{get_content_types_for_pages, get_page_async, get_page_count_async},
	ContentType, EpubProcessor,
};
use tokio::sync::broadcast::{self, Receiver, Sender};

use crate::{
	config::state::AppState,
	errors::APIError,
	routers::api::v2::{media as api_media, series as api_series},
	utils::{http::ImageResponse, serve_media},
};
use stump_komga::routes::{KomgaBackend, KomgaCoreEvent, KomgaImage};

/// Server-side implementation of the Komga backend contract. Every Stump-specific
/// operation (database, media processing, jobs, core events) stays in
/// `stump_core`/`stump_server`; the extracted router only sees this trait.
pub(crate) struct KomgaBackendAdapter {
	ctx: AppState,
	core_events: Arc<Sender<KomgaCoreEvent>>,
}

impl KomgaBackendAdapter {
	pub(crate) fn new(ctx: AppState) -> Self {
		let sender = Arc::new(broadcast::channel(256).0);
		let mut receiver = ctx.get_client_receiver();
		let forwarder = sender.clone();
		tokio::spawn(async move {
			loop {
				match receiver.recv().await {
					Ok(CoreEvent::CreatedMedia(media)) => {
						let _ = forwarder.send(KomgaCoreEvent::CreatedMedia {
							id: media.id,
							series_id: media.series_id,
							library_id: media.library_id,
						});
					},
					Ok(_) => {},
					Err(broadcast::error::RecvError::Lagged(skipped)) => {
						tracing::debug!(skipped, "Komga core event adapter lagged")
					},
					Err(broadcast::error::RecvError::Closed) => break,
				}
			}
		});
		Self {
			ctx,
			core_events: sender,
		}
	}
}

fn map_core_error(error: impl std::fmt::Display) -> stump_komga::errors::APIError {
	stump_komga::errors::APIError::InternalServerError(error.to_string())
}

fn map_server_error(error: APIError) -> stump_komga::errors::APIError {
	match error.status_code() {
		axum::http::StatusCode::BAD_REQUEST => {
			stump_komga::errors::APIError::BadRequest(error.to_string())
		},
		axum::http::StatusCode::NOT_FOUND => {
			stump_komga::errors::APIError::NotFound(error.to_string())
		},
		axum::http::StatusCode::UNAUTHORIZED => {
			stump_komga::errors::APIError::Unauthorized
		},
		axum::http::StatusCode::FORBIDDEN => {
			stump_komga::errors::APIError::Forbidden(error.to_string())
		},
		_ => stump_komga::errors::APIError::InternalServerError(error.to_string()),
	}
}

async fn image_response(
	image: ImageResponse,
) -> stump_komga::errors::APIResult<KomgaImage> {
	let content_type = image.content_type.to_string();
	let metadata = generate_image_metadata_from_bytes(image.data.clone())
		.await
		.map_err(map_core_error)?;
	let (width, height) = metadata
		.dimensions
		.map(|dimensions| {
			(
				Some(dimensions.width as i32),
				Some(dimensions.height as i32),
			)
		})
		.unwrap_or((None, None));
	Ok(KomgaImage {
		content_type,
		data: image.data,
		width,
		height,
	})
}

#[async_trait]
impl KomgaBackend for KomgaBackendAdapter {
	fn conn(&self) -> &sea_orm::DatabaseConnection {
		self.ctx.conn.as_ref()
	}

	fn conn_arc(&self) -> Arc<sea_orm::DatabaseConnection> {
		self.ctx.conn.clone()
	}

	fn core_events(&self) -> Receiver<KomgaCoreEvent> {
		self.core_events.subscribe()
	}

	async fn enqueue_library_scan(
		&self,
		library_id: String,
		path: String,
		deep: bool,
	) -> stump_komga::errors::APIResult<()> {
		let options = deep.then_some(stump_scanner::ScanOptions {
			config: stump_scanner::ScanConfig::ForceRebuild {
				force_rebuild: true,
			},
		});
		self.ctx
			.enqueue(StumpJob::library_scan(library_id, path, options))
			.await
			.map_err(map_core_error)
	}

	async fn enqueue_library_analysis(
		&self,
		library_id: String,
	) -> stump_komga::errors::APIResult<()> {
		self.ctx
            .enqueue(StumpJob::analyze_media(
                stump_core::filesystem::media::analysis::AnalysisJobConfig {
                    force_reanalysis: true,
                    scope: stump_core::filesystem::media::analysis::MediaAnalysisJobScope::Library(
                        library_id,
                    ),
                },
            ))
            .await
            .map_err(map_core_error)
	}

	async fn enqueue_book_analysis(
		&self,
		book_id: String,
	) -> stump_komga::errors::APIResult<()> {
		self.ctx
            .enqueue(StumpJob::analyze_media(
                stump_core::filesystem::media::analysis::AnalysisJobConfig {
                    force_reanalysis: true,
                    scope: stump_core::filesystem::media::analysis::MediaAnalysisJobScope::Book(
                        book_id,
                    ),
                },
            ))
            .await
            .map_err(map_core_error)
	}

	async fn book_pages(
		&self,
		book: &media::Model,
	) -> stump_komga::errors::APIResult<Vec<stump_komga::KomgaBookPage>> {
		// Komga lists image pages only; an EPUB that is not Divina-compatible
		// (which this adapter never claims) has an empty page list while its
		// `pagesCount` stays the position-based count. Stump's EPUB page
		// numbers are synthetic, so walking them as chapters is meaningless.
		if book.extension.eq_ignore_ascii_case("epub") {
			return Ok(Vec::new());
		}
		let count = get_page_count_async(&book.path, &self.ctx.config.media)
			.await
			.map_err(map_core_error)?;
		let count = usize::try_from(count).map_err(map_core_error)?;
		if count == 0 {
			return Ok(Vec::new());
		}
		let count = count.min(4096);
		let numbers = (1..=count).map(|number| number as i32).collect::<Vec<_>>();
		let content_types = get_content_types_for_pages(&book.path, numbers.clone())
			.map_err(map_core_error)?;
		Ok(numbers
			.into_iter()
			.map(|number| {
				let media_type = content_types
					.get(&number)
					.copied()
					.unwrap_or(ContentType::UNKNOWN)
					.to_string();
				stump_komga::KomgaBookPage {
					number,
					file_name: format!("page-{number}"),
					media_type,
					width: None,
					height: None,
					size_bytes: None,
					size: "0 B".to_owned(),
				}
			})
			.collect())
	}

	async fn book_page(
		&self,
		user: &AuthUser,
		book_id: String,
		page: u32,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(book_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Book not found".to_owned())
			})?;
		let page = i32::try_from(page).map_err(|_| {
			stump_komga::errors::APIError::BadRequest(
				"page index out of range".to_owned(),
			)
		})?;
		let (content_type, data) =
			get_page_async(&book.path, page, &self.ctx.config.media)
				.await
				.map_err(|error| map_server_error(APIError::from(error)))?;
		Ok(KomgaImage::new(content_type.to_string(), data))
	}

	async fn book_page_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
		page: u32,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let image = self.book_page(user, book_id, page).await?;
		let data = GenericImageProcessor::generate(
			&image.data,
			models::shared::image_processor_options::ImageProcessorOptions {
				format: SupportedImageFormat::Jpeg,
				..Default::default()
			},
		)
		.map_err(map_core_error)?;
		let metadata = generate_image_metadata_from_bytes(data.clone())
			.await
			.map_err(map_core_error)?;
		let (width, height) = metadata
			.dimensions
			.map(|dimensions| {
				(
					Some(dimensions.width as i32),
					Some(dimensions.height as i32),
				)
			})
			.unwrap_or((None, None));
		Ok(KomgaImage {
			content_type: "image/jpeg".to_owned(),
			data,
			width,
			height,
		})
	}

	async fn book_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let image =
			api_media::get_media_thumbnail_by_id(self.ctx.as_ref(), user, book_id)
				.await
				.map_err(map_server_error)?;
		image_response(image).await
	}

	async fn series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: &str,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let series = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id.to_owned()))
			.into_model::<series::SeriesThumbSelect>()
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Series not found".to_owned())
			})?;
		let first_book = media::Entity::find_for_user(user)
			.filter(media::Column::SeriesId.eq(series.id.clone()))
			.order_by_asc(media::Column::Name)
			.into_model::<media::MediaThumbSelect>()
			.one(self.conn())
			.await?;
		let config = models::entity::library_config::Entity::find()
			.filter(
				models::entity::library_config::Column::LibraryId
					.eq(series.library_id.clone()),
			)
			.one(self.conn())
			.await?;
		let format =
			config.and_then(|config| config.thumbnail_config.map(|config| config.format));
		let (content_type, data) = api_series::get_series_thumbnail(
			&series,
			first_book,
			format,
			self.ctx.config.as_ref(),
		)
		.await
		.map_err(map_server_error)?;
		image_response(ImageResponse::new(content_type, data)).await
	}

	async fn serve_book_file(
		&self,
		auth: AuthContext,
		headers: HeaderMap,
		book_id: String,
	) -> stump_komga::errors::APIResult<Response<Body>> {
		serve_media::serve_media_file(auth, headers, self.conn(), book_id)
			.await
			.map(IntoResponse::into_response)
			.map_err(map_server_error)
	}

	async fn readium_manifest(
		&self,
		path: String,
		base_url: String,
	) -> stump_komga::errors::APIResult<serde_json::Value> {
		let manifest = stump_media::ReadiumManifestGenerator::new(path, base_url)
			.generate_manifest()
			.map_err(map_core_error)?;
		serde_json::to_value(manifest).map_err(map_core_error)
	}

	async fn readium_positions(
		&self,
		path: String,
		base_url: String,
	) -> stump_komga::errors::APIResult<serde_json::Value> {
		let positions = stump_media::ReadiumManifestGenerator::new(path, base_url)
			.generate_positions()
			.map_err(map_core_error)?;
		serde_json::to_value(positions).map_err(map_core_error)
	}

	async fn readium_resource(
		&self,
		path: String,
		resource_path: PathBuf,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let (content_type, data) =
			EpubProcessor::get_resource_by_path(&path, "", resource_path)
				.map_err(|error| map_server_error(APIError::from(error)))?;
		Ok(KomgaImage::new(content_type.to_string(), data))
	}
}
