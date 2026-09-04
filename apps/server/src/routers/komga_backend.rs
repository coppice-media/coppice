use std::{
	path::{Path, PathBuf},
	sync::Arc,
};

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
	get_saved_thumbnail,
	image::{
		generate_image_metadata_from_bytes, replace_thumbnail, GenericImageProcessor,
		ImageProcessor,
	},
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
use stump_komga::{
	routes::{KomgaBackend, KomgaCoreEvent, KomgaImage},
	KomgaBookId, KomgaSeriesId, KomgaThumbnailId,
};

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
					Ok(CoreEvent::MediaDeleted(media)) => {
						let _ = forwarder.send(KomgaCoreEvent::MediaDeleted {
							id: media.id,
							series_id: media.series_id,
							library_id: media.library_id,
						});
					},
					Ok(CoreEvent::SeriesDeleted(series)) => {
						let _ = forwarder.send(KomgaCoreEvent::SeriesDeleted {
							id: series.id,
							library_id: series.library_id,
						});
					},
					Ok(CoreEvent::JobQueueStatus(status)) => {
						let _ = forwarder.send(KomgaCoreEvent::JobQueueStatus {
							count: status.count,
							count_by_type: status.count_by_type,
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

const BOOK_THUMBNAIL_PREFIX: &str = "stump-book-";
const SERIES_THUMBNAIL_PREFIX: &str = "stump-series-";
const BOOK_UPLOAD_THUMBNAIL_PREFIX: &str = "stump-upload-book-";
const SERIES_UPLOAD_THUMBNAIL_PREFIX: &str = "stump-upload-series-";

fn thumbnail_id(prefix: &str, id: &str) -> KomgaThumbnailId {
	KomgaThumbnailId::new(format!("{prefix}{id}"))
}

fn book_thumbnail_id(id: &str) -> KomgaThumbnailId {
	thumbnail_id(BOOK_THUMBNAIL_PREFIX, id)
}

fn book_upload_thumbnail_id(id: &str) -> KomgaThumbnailId {
	thumbnail_id(BOOK_UPLOAD_THUMBNAIL_PREFIX, id)
}
fn series_thumbnail_id(id: &str) -> KomgaThumbnailId {
	thumbnail_id(SERIES_THUMBNAIL_PREFIX, id)
}

fn series_upload_thumbnail_id(id: &str) -> KomgaThumbnailId {
	thumbnail_id(SERIES_UPLOAD_THUMBNAIL_PREFIX, id)
}

fn thumbnail_details(
	image: &KomgaImage,
) -> stump_komga::errors::APIResult<(String, i64, i32, i32)> {
	if !image.content_type.starts_with("image/") {
		return Err(stump_komga::errors::APIError::BadRequest(
			"The requested asset is not an image".to_string(),
		));
	}
	let (Some(width), Some(height)) = (image.width, image.height) else {
		return Err(stump_komga::errors::APIError::BadRequest(
			"Thumbnail dimensions are unavailable".to_string(),
		));
	};
	let file_size = i64::try_from(image.data.len()).map_err(map_core_error)?;
	Ok((image.content_type.clone(), file_size, width, height))
}

async fn saved_thumbnail(path: &str) -> stump_komga::errors::APIResult<KomgaImage> {
	let (content_type, data) = get_saved_thumbnail(Path::new(path))
		.await
		.map_err(map_core_error)?;
	image_response(ImageResponse::new(content_type, data)).await
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
	async fn book_thumbnails(
		&self,
		user: &AuthUser,
		book_id: String,
	) -> stump_komga::errors::APIResult<Vec<stump_komga::KomgaBookThumbnail>> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(book_id.clone()))
			.filter(media::Column::DeletedAt.is_null())
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Book not found".to_owned())
			})?;

		let uploaded = match book.thumbnail_path.as_deref() {
			Some(path) => match saved_thumbnail(path).await {
				Ok(image) => Some(image),
				Err(error) => {
					tracing::warn!(?error, path, "Failed to read saved book thumbnail");
					None
				},
			},
			None => None,
		};
		let image = match uploaded.as_ref() {
			Some(image) => image.clone(),
			None => self.book_thumbnail(user, book.id.clone()).await?,
		};
		let (media_type, file_size, width, height) = thumbnail_details(&image)?;
		let mut thumbnails = Vec::with_capacity(if uploaded.is_some() { 2 } else { 1 });
		if uploaded.is_some() {
			thumbnails.push(stump_komga::KomgaBookThumbnail {
				id: book_upload_thumbnail_id(&book.id),
				book_id: KomgaBookId::new(book.id.clone()),
				r#type: "USER_UPLOADED".to_owned(),
				selected: true,
				media_type: media_type.clone(),
				file_size,
				width,
				height,
			});
			thumbnails.push(stump_komga::KomgaBookThumbnail {
				id: book_thumbnail_id(&book.id),
				book_id: KomgaBookId::new(book.id),
				r#type: "GENERATED".to_owned(),
				selected: false,
				media_type,
				file_size,
				width,
				height,
			});
		} else {
			thumbnails.push(stump_komga::KomgaBookThumbnail {
				id: book_thumbnail_id(&book.id),
				book_id: KomgaBookId::new(book.id),
				r#type: "GENERATED".to_owned(),
				selected: true,
				media_type,
				file_size,
				width,
				height,
			});
		}
		Ok(thumbnails)
	}

	async fn book_thumbnail_by_id(
		&self,
		user: &AuthUser,
		book_id: String,
		thumbnail_id: String,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(book_id.clone()))
			.filter(media::Column::DeletedAt.is_null())
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Book not found".to_owned())
			})?;
		if thumbnail_id == book_upload_thumbnail_id(&book.id).to_string() {
			let path = book.thumbnail_path.as_deref().ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Thumbnail not found".to_owned())
			})?;
			return saved_thumbnail(path).await.map_err(|_| {
				stump_komga::errors::APIError::NotFound("Thumbnail not found".to_owned())
			});
		}
		if thumbnail_id == book_thumbnail_id(&book.id).to_string() {
			return self.book_thumbnail(user, book.id).await;
		}
		Err(stump_komga::errors::APIError::NotFound(
			"Thumbnail not found".to_owned(),
		))
	}

	async fn upload_book_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
		bytes: Vec<u8>,
		selected: bool,
	) -> stump_komga::errors::APIResult<stump_komga::KomgaBookThumbnail> {
		if !selected {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Stump only supports selected thumbnail uploads".to_owned(),
			));
		}
		if bytes.is_empty() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file is empty".to_owned(),
			));
		}
		if bytes.len() > self.ctx.config.max_file_upload_size {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file exceeds the configured upload limit".to_owned(),
			));
		}
		let content_type = ContentType::from_bytes(&bytes);
		if !content_type.is_image() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file must be an image".to_owned(),
			));
		}
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(book_id))
			.filter(media::Column::DeletedAt.is_null())
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Book not found".to_owned())
			})?;
		let metadata = generate_image_metadata_from_bytes(bytes.clone())
			.await
			.map_err(map_core_error)?;
		let path = replace_thumbnail(
			&book.id,
			content_type.extension(),
			&bytes,
			&self.ctx.config.media,
		)
		.await
		.map_err(map_core_error)?;
		media::Entity::update_many()
			.col_expr(
				media::Column::ThumbnailPath,
				sea_orm::sea_query::Expr::value(Some(path.to_string_lossy().to_string())),
			)
			.col_expr(
				media::Column::ThumbnailMeta,
				sea_orm::sea_query::Expr::value(Some(metadata)),
			)
			.filter(media::Column::Id.eq(book.id.clone()))
			.exec(self.conn())
			.await?;
		let image = saved_thumbnail(&path.to_string_lossy()).await?;
		let (media_type, file_size, width, height) = thumbnail_details(&image)?;
		Ok(stump_komga::KomgaBookThumbnail {
			id: book_upload_thumbnail_id(&book.id),
			book_id: KomgaBookId::new(book.id),
			r#type: "USER_UPLOADED".to_owned(),
			selected: true,
			media_type,
			file_size,
			width,
			height,
		})
	}

	async fn delete_book_thumbnail(
		&self,
		user: &AuthUser,
		book_id: String,
		thumbnail_id: String,
	) -> stump_komga::errors::APIResult<()> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(book_id))
			.filter(media::Column::DeletedAt.is_null())
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Book not found".to_owned())
			})?;
		if thumbnail_id == book_thumbnail_id(&book.id).to_string() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Generated thumbnails cannot be deleted".to_owned(),
			));
		}
		if thumbnail_id != book_upload_thumbnail_id(&book.id).to_string()
			|| book.thumbnail_path.is_none()
		{
			return Err(stump_komga::errors::APIError::NotFound(
				"Thumbnail not found".to_owned(),
			));
		}
		let ids = [book.id.clone()];
		stump_media::image::remove_thumbnails(
			&ids,
			&self.ctx.config.get_thumbnails_dir(),
		)
		.await
		.map_err(map_core_error)?;
		media::Entity::update_many()
			.col_expr(
				media::Column::ThumbnailPath,
				sea_orm::sea_query::Expr::value(None::<String>),
			)
			.col_expr(
				media::Column::ThumbnailMeta,
				sea_orm::sea_query::Expr::value(
					None::<models::shared::image::ImageMetadata>,
				),
			)
			.filter(media::Column::Id.eq(book.id))
			.exec(self.conn())
			.await?;
		Ok(())
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

	async fn series_thumbnails(
		&self,
		user: &AuthUser,
		series_id: String,
	) -> stump_komga::errors::APIResult<Vec<stump_komga::KomgaSeriesThumbnail>> {
		let series = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Series not found".to_owned())
			})?;
		let Some(path) = series.thumbnail_path.as_deref() else {
			return Ok(Vec::new());
		};
		let image = match saved_thumbnail(path).await {
			Ok(image) => image,
			Err(error) => {
				tracing::warn!(?error, path, "Failed to read saved series thumbnail");
				return Ok(Vec::new());
			},
		};
		let (media_type, file_size, width, height) = thumbnail_details(&image)?;
		Ok(vec![stump_komga::KomgaSeriesThumbnail {
			id: series_upload_thumbnail_id(&series.id),
			series_id: KomgaSeriesId::new(series.id),
			r#type: stump_komga::KomgaSeriesThumbnailType::UserUploaded,
			selected: true,
			media_type,
			file_size,
			width,
			height,
		}])
	}

	async fn series_thumbnail_by_id(
		&self,
		user: &AuthUser,
		series_id: String,
		thumbnail_id: String,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let series = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Series not found".to_owned())
			})?;
		if thumbnail_id != series_upload_thumbnail_id(&series.id).to_string() {
			return Err(stump_komga::errors::APIError::NotFound(
				"Thumbnail not found".to_owned(),
			));
		}
		let path = series.thumbnail_path.as_deref().ok_or_else(|| {
			stump_komga::errors::APIError::NotFound("Thumbnail not found".to_owned())
		})?;
		saved_thumbnail(path).await.map_err(|_| {
			stump_komga::errors::APIError::NotFound("Thumbnail not found".to_owned())
		})
	}

	async fn upload_series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: String,
		bytes: Vec<u8>,
		selected: bool,
	) -> stump_komga::errors::APIResult<stump_komga::KomgaSeriesThumbnail> {
		if !selected {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Stump only supports selected thumbnail uploads".to_owned(),
			));
		}
		if bytes.is_empty() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file is empty".to_owned(),
			));
		}
		if bytes.len() > self.ctx.config.max_file_upload_size {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file exceeds the configured upload limit".to_owned(),
			));
		}
		let content_type = ContentType::from_bytes(&bytes);
		if !content_type.is_image() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file must be an image".to_owned(),
			));
		}
		let series = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Series not found".to_owned())
			})?;
		let metadata = generate_image_metadata_from_bytes(bytes.clone())
			.await
			.map_err(map_core_error)?;
		let path = replace_thumbnail(
			&series.id,
			content_type.extension(),
			&bytes,
			&self.ctx.config.media,
		)
		.await
		.map_err(map_core_error)?;
		series::Entity::update_many()
			.col_expr(
				series::Column::ThumbnailPath,
				sea_orm::sea_query::Expr::value(Some(path.to_string_lossy().to_string())),
			)
			.col_expr(
				series::Column::ThumbnailMeta,
				sea_orm::sea_query::Expr::value(Some(metadata)),
			)
			.filter(series::Column::Id.eq(series.id.clone()))
			.exec(self.conn())
			.await?;
		let image = saved_thumbnail(&path.to_string_lossy()).await?;
		let (media_type, file_size, width, height) = thumbnail_details(&image)?;
		Ok(stump_komga::KomgaSeriesThumbnail {
			id: series_upload_thumbnail_id(&series.id),
			series_id: KomgaSeriesId::new(series.id),
			r#type: stump_komga::KomgaSeriesThumbnailType::UserUploaded,
			selected: true,
			media_type,
			file_size,
			width,
			height,
		})
	}

	async fn delete_series_thumbnail(
		&self,
		user: &AuthUser,
		series_id: String,
		thumbnail_id: String,
	) -> stump_komga::errors::APIResult<()> {
		let series = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				stump_komga::errors::APIError::NotFound("Series not found".to_owned())
			})?;
		if thumbnail_id == series_thumbnail_id(&series.id).to_string() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Generated thumbnails cannot be deleted".to_owned(),
			));
		}
		if thumbnail_id != series_upload_thumbnail_id(&series.id).to_string()
			|| series.thumbnail_path.is_none()
		{
			return Err(stump_komga::errors::APIError::NotFound(
				"Thumbnail not found".to_owned(),
			));
		}
		let ids = [series.id.clone()];
		stump_media::image::remove_thumbnails(
			&ids,
			&self.ctx.config.get_thumbnails_dir(),
		)
		.await
		.map_err(map_core_error)?;
		series::Entity::update_many()
			.col_expr(
				series::Column::ThumbnailPath,
				sea_orm::sea_query::Expr::value(None::<String>),
			)
			.col_expr(
				series::Column::ThumbnailMeta,
				sea_orm::sea_query::Expr::value(
					None::<models::shared::image::ImageMetadata>,
				),
			)
			.filter(series::Column::Id.eq(series.id))
			.exec(self.conn())
			.await?;
		Ok(())
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
