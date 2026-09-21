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
	entity::{library as library_entity, library_config, media, series, user::AuthUser},
	shared::ignore_rules::IgnoreRules,
};
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set};
use stump_auth::AuthContext;
use stump_core::{job::stump_job::StumpJob, reading_state::SourceProtocol, CoreEvent};
use stump_devices::{CredentialRef, Protocol};
use stump_media::{
	get_saved_thumbnail,
	image::{
		generate_image_metadata_from_bytes, replace_thumbnail, GenericImageProcessor,
		ImageProcessor,
	},
	media::{get_content_types_for_pages_async, get_page_async, get_page_count_async},
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
	KomgaBookId, KomgaLibraryCreateRequest, KomgaLibraryUpdateRequest, KomgaSeriesId,
	KomgaThumbnailId, PatchValue, ScanInterval,
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
					Ok(CoreEvent::LibraryCreated(library)) => {
						let _ = forwarder
							.send(KomgaCoreEvent::LibraryCreated { id: library.id });
					},
					Ok(CoreEvent::LibraryUpdated(library)) => {
						let _ = forwarder
							.send(KomgaCoreEvent::LibraryUpdated { id: library.id });
					},
					Ok(CoreEvent::LibraryDeleted(library)) => {
						let _ = forwarder
							.send(KomgaCoreEvent::LibraryDeleted { id: library.id });
					},
					Ok(CoreEvent::CollectionAdded(collection)) => {
						let _ = forwarder.send(KomgaCoreEvent::CollectionAdded {
							collection_id: collection.id,
							series_ids: collection.series_ids,
						});
					},
					Ok(CoreEvent::CollectionChanged(collection)) => {
						let _ = forwarder.send(KomgaCoreEvent::CollectionChanged {
							collection_id: collection.id,
							series_ids: collection.series_ids,
						});
					},
					Ok(CoreEvent::CollectionDeleted(collection)) => {
						let _ = forwarder.send(KomgaCoreEvent::CollectionDeleted {
							collection_id: collection.id,
							series_ids: collection.series_ids,
						});
					},
					Ok(CoreEvent::ReadListAdded(read_list)) => {
						let _ = forwarder.send(KomgaCoreEvent::ReadListAdded {
							read_list_id: read_list.id,
							book_ids: read_list.book_ids,
						});
					},
					Ok(CoreEvent::ReadListChanged(read_list)) => {
						let _ = forwarder.send(KomgaCoreEvent::ReadListChanged {
							read_list_id: read_list.id,
							book_ids: read_list.book_ids,
						});
					},
					Ok(CoreEvent::ReadListDeleted(read_list)) => {
						let _ = forwarder.send(KomgaCoreEvent::ReadListDeleted {
							read_list_id: read_list.id,
							book_ids: read_list.book_ids,
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
		// Reading heads moved by other protocols (Kobo, KOReader, OPDS, the
		// native API) surface as Komga read-progress events, so Komelia sees
		// them the same way it sees its own writes. Komga's own routes publish
		// directly on the route-local bus and are not announced here.
		let mut heads = ctx.reading_state_events();
		let forwarder = sender.clone();
		tokio::spawn(async move {
			loop {
				match heads.recv().await {
					Ok(changed) if changed.protocol != SourceProtocol::Komga => {
						let _ = forwarder.send(KomgaCoreEvent::ReadProgressChanged {
							book_id: changed.media_id,
							series_id: changed.series_id.unwrap_or_default(),
							user_id: changed.user_id,
							deleted: changed.cleared,
						});
					},
					Ok(_) => {},
					Err(broadcast::error::RecvError::Lagged(skipped)) => {
						tracing::debug!(skipped, "Komga reading-state adapter lagged")
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
	if let APIError::DatabaseBusy = error {
		return stump_komga::errors::APIError::DatabaseBusy;
	}
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

/// Maps core-service errors onto the Komga error surface with their precise
/// status codes (`404` for missing rows, `400` for validation, `403` for
/// visibility/permission failures, `503` for disabled background jobs and for
/// SQLite write-lock contention).
fn map_core_error_status(error: stump_core::CoreError) -> stump_komga::errors::APIError {
	match error {
		stump_core::CoreError::NotFound(message) => {
			stump_komga::errors::APIError::NotFound(message)
		},
		stump_core::CoreError::BadRequest(message) => {
			stump_komga::errors::APIError::BadRequest(message)
		},
		stump_core::CoreError::Forbidden(message) => {
			stump_komga::errors::APIError::Forbidden(message)
		},
		stump_core::CoreError::DBError(error) => {
			stump_komga::errors::APIError::from(error)
		},
		stump_core::CoreError::FeatureDisabled(feature) => {
			stump_komga::errors::APIError::ServiceUnavailable(format!(
				"{feature} are disabled"
			))
		},
		other => map_core_error(other),
	}
}

/// Maps Komga's `ScanIntervalDto` onto a 5-field cron schedule for Stump's
/// scheduled-job runner. `DISABLED` maps to `None` (no periodic scan).
fn scan_interval_cron(interval: ScanInterval) -> Option<&'static str> {
	match interval {
		ScanInterval::Disabled => None,
		ScanInterval::Hourly => Some("0 * * * *"),
		ScanInterval::Every6h => Some("0 */6 * * *"),
		ScanInterval::Every12h => Some("0 */12 * * *"),
		ScanInterval::Daily => Some("0 0 * * *"),
		ScanInterval::Weekly => Some("0 0 * * 0"),
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
		let content_types =
			get_content_types_for_pages_async(&book.path, numbers.clone())
				.await
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

	async fn visible_pages(
		&self,
		book: &media::Model,
	) -> stump_komga::errors::APIResult<Option<Vec<i32>>> {
		// Physical pages (1-based) still visible under duplicate-page
		// skipping; `None` when nothing is skipped.
		let cache = self.ctx.visible_pages_cache();
		let visible = stump_core::filesystem::media::visible_pages::visible_pages(
			self.ctx.conn.as_ref(),
			&cache,
			&book.id,
			book.pages,
		)
		.await
		.map_err(map_core_error)?;
		if visible.len() as i32 == book.pages.max(0) {
			Ok(None)
		} else {
			Ok(Some(visible.to_vec()))
		}
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
				format:
					models::shared::image_processor_options::SupportedImageFormat::Jpeg,
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
				"Coppice only supports selected thumbnail uploads".to_owned(),
			));
		}
		if bytes.is_empty() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file is empty".to_owned(),
			));
		}
		if bytes.len() > self.ctx.config.protocols.max_file_upload_size {
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
		let stored = series::Entity::find_for_user(user)
			.filter(series::Column::Id.eq(series_id.to_owned()))
			.into_model::<series::SeriesThumbSelect>()
			.one(self.conn())
			.await?;
		// Mode B: a live-only virtual series has no row; its cover comes
		// straight from the source (cached by the provider host). A
		// materialised one has a row but no thumbnail on disk and no local
		// file to render, so the source cover is its thumbnail too — unless
		// an operator uploaded one, which always wins.
		#[cfg(feature = "providers")]
		let series = match stored {
			Some(series) if series.thumbnail_path.is_some() => series,
			other => {
				let cover = super::provider_virtual::virtual_series_cover(
					&self.ctx, user, series_id,
				)
				.await;
				match (cover, other) {
					(Some(Ok((content_type, data))), _) => {
						return image_response(ImageResponse::new(content_type, data))
							.await;
					},
					// A provider cover that failed to load still has the
					// page-1 fallback below when the series is materialised.
					(Some(Err(error)), Some(series)) => {
						tracing::warn!(
							error,
							series_id,
							"Provider series cover failed; falling back to the first book"
						);
						series
					},
					(Some(Err(error)), None) => {
						return Err(map_core_error(error));
					},
					(None, Some(series)) => series,
					(None, None) => {
						return Err(stump_komga::errors::APIError::NotFound(
							"Series not found".to_owned(),
						));
					},
				}
			},
		};
		#[cfg(not(feature = "providers"))]
		let series = stored.ok_or_else(|| {
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
				"Coppice only supports selected thumbnail uploads".to_owned(),
			));
		}
		if bytes.is_empty() {
			return Err(stump_komga::errors::APIError::BadRequest(
				"Thumbnail file is empty".to_owned(),
			));
		}
		if bytes.len() > self.ctx.config.protocols.max_file_upload_size {
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
		let manifest = tokio::task::spawn_blocking(move || {
			stump_media::ReadiumManifestGenerator::new(path, base_url).generate_manifest()
		})
		.await
		.map_err(map_core_error)?
		.map_err(map_core_error)?;
		serde_json::to_value(manifest).map_err(map_core_error)
	}

	async fn readium_positions(
		&self,
		path: String,
		base_url: String,
	) -> stump_komga::errors::APIResult<serde_json::Value> {
		let positions = tokio::task::spawn_blocking(move || {
			stump_media::ReadiumManifestGenerator::new(path, base_url)
				.generate_positions()
		})
		.await
		.map_err(map_core_error)?
		.map_err(map_core_error)?;
		serde_json::to_value(positions).map_err(map_core_error)
	}

	async fn readium_resource(
		&self,
		path: String,
		resource_path: PathBuf,
	) -> stump_komga::errors::APIResult<KomgaImage> {
		let (content_type, data) = tokio::task::spawn_blocking(move || {
			EpubProcessor::get_resource_by_path(&path, "", resource_path)
		})
		.await
		.map_err(map_core_error)?
		.map_err(|error| map_server_error(APIError::from(error)))?;
		Ok(KomgaImage::new(content_type.to_string(), data))
	}

	async fn record_sync(&self, auth: &AuthContext, summary: serde_json::Value) {
		let Some(api_key) = auth.api_key.as_deref() else {
			return;
		};
		if let Err(error) = self
			.ctx
			.devices()
			.touch(
				CredentialRef::ApiKey(api_key),
				Protocol::Komga,
				Some(summary),
			)
			.await
		{
			tracing::warn!(?error, "Failed to record the Komga sync on its device");
		}
	}

	#[cfg(feature = "providers")]
	async fn virtual_library_source(&self, library_id: &str) -> Option<String> {
		super::provider_virtual::virtual_library_source(&self.ctx, library_id).await
	}

	/// Mode B live browse for one virtual library page, as a Komga page
	/// envelope. Live cards report zero books; the count is learned when the
	/// series is materialised. Rows the user's age restriction hides are
	/// dropped here, since there is no `series_metadata` row to filter on in
	/// SQL yet.
	#[cfg(feature = "providers")]
	async fn virtual_series_list(
		&self,
		user: &AuthUser,
		library_id: String,
		search: &stump_komga::KomgaSeriesSearch,
		sorts: &[String],
		page: i32,
		size: i32,
		unpaged: bool,
	) -> Option<stump_komga::errors::APIResult<stump_komga::Page<stump_komga::KomgaSeries>>>
	{
		let source_id =
			super::provider_virtual::virtual_library_source(&self.ctx, &library_id)
				.await?;
		let kind = super::provider_virtual::browse_kind(
			search.full_text_search.as_deref(),
			sorts,
		);
		let result = super::provider_virtual::browse_page(
			&self.ctx,
			&source_id,
			&library_id,
			&kind,
			page.max(0) as u32,
		)
		.await;
		let result = match result {
			Ok(result) => result,
			Err(error) => {
				return Some(Err(map_core_error(error)));
			},
		};
		let adult_source =
			super::provider_virtual::source_is_adult(&self.ctx, &source_id).await;
		let content: Vec<stump_komga::KomgaSeries> = result
			.items
			.iter()
			.map(|remote| {
				super::provider_virtual::map_remote_series(
					&source_id,
					&library_id,
					remote,
					adult_source,
				)
			})
			.filter(|series| {
				super::provider_virtual::age_restriction_allows(
					user,
					series.metadata.age_rating,
				)
			})
			.collect();
		// Live browse has no total count; one extra phantom element marks
		// `hasNext` so Komga-style clients keep paging.
		let total = if result.has_next {
			(page.max(0) as i32).saturating_mul(size.max(1)) + content.len() as i32 + 1
		} else {
			(page.max(0) as i32).saturating_mul(size.max(1)) + content.len() as i32
		};
		Some(Ok(stump_komga::Page::new(
			content, page, size, total, unpaged,
		)))
	}

	/// Creates a library from a Komga `LibraryCreationDto` and wires its
	/// scheduled scan interval.
	async fn create_library(
		&self,
		request: KomgaLibraryCreateRequest,
	) -> stump_komga::errors::APIResult<library_entity::Model> {
		let ignore_rules = if request.scan_directory_exclusions.is_empty() {
			None
		} else {
			Some(
				IgnoreRules::new(request.scan_directory_exclusions.clone()).map_err(
					|error| stump_komga::errors::APIError::BadRequest(error.to_string()),
				)?,
			)
		};
		let created = stump_library::library::create_library(
			&self.ctx,
			stump_library::library::NewLibrary {
				name: request.name,
				path: request.root,
				description: None,
				emoji: None,
				// `hashFiles`/`hashKoreader` have direct config counterparts;
				// the four ComicInfo/EPUB import switches collapse onto the single
				// `process_metadata` switch (on when any of them is on). Komga
				// drives periodic scans via `scanInterval` instead of filesystem
				// watching.
				config: library_config::ActiveModel {
					generate_file_hashes: Set(request.hash_files),
					generate_koreader_hashes: Set(request.hash_koreader),
					process_metadata: Set(request.import_comic_info_book
						|| request.import_comic_info_series
						|| request.import_epub_book
						|| request.import_epub_series),
					ignore_rules: Set(ignore_rules),
					watch: Set(false),
					..Default::default()
				},
				tags: Vec::new(),
				// Upstream always scans a freshly added library.
				scan_after_persist: true,
			},
		)
		.await
		.map_err(map_core_error_status)?;

		if let Some(cron) = scan_interval_cron(request.scan_interval) {
			stump_library::library::sync_library_scan_schedule(
				&self.ctx,
				&created.id,
				&created.name,
				Some(cron),
			)
			.await
			.map_err(map_core_error_status)?;
		}

		Ok(created)
	}

	async fn update_library(
		&self,
		user: &AuthUser,
		id: &str,
		request: KomgaLibraryUpdateRequest,
	) -> stump_komga::errors::APIResult<()> {
		let (existing_library, existing_config) =
			library_entity::Entity::find_for_user(user)
				.filter(library_entity::Column::Id.eq(id.to_owned()))
				.find_also_related(library_config::Entity)
				.one(self.ctx.conn.as_ref())
				.await
				.map_err(stump_komga::errors::APIError::DbError)?
				.ok_or_else(|| {
					stump_komga::errors::APIError::NotFound(
						"Library not found".to_owned(),
					)
				})?;
		let existing_config = existing_config.ok_or_else(|| {
			stump_komga::errors::APIError::InternalServerError(
				"Library is missing associated config!".to_owned(),
			)
		})?;

		let name = request
			.name
			.clone()
			.unwrap_or_else(|| existing_library.name.clone());
		let path = request
			.root
			.clone()
			.unwrap_or_else(|| existing_library.path.clone());

		// The patch merges onto the stored config row; omitted fields keep
		// their values, matching upstream's `LibraryUpdateDto` semantics.
		let mut config = existing_config.into_active_model();
		if let Some(hash_files) = request.hash_files {
			config.generate_file_hashes = Set(hash_files);
		}
		if let Some(hash_koreader) = request.hash_koreader {
			config.generate_koreader_hashes = Set(hash_koreader);
		}
		// The four import switches collapse onto `process_metadata`: when the
		// patch carries any of them, the switch becomes the OR of those present.
		let import_switches = [
			request.import_comic_info_book,
			request.import_comic_info_series,
			request.import_epub_book,
			request.import_epub_series,
		];
		if import_switches.iter().any(Option::is_some) {
			config.process_metadata = Set(import_switches.iter().flatten().any(|on| *on));
		}
		match &request.scan_directory_exclusions {
			PatchValue::Unset => {},
			PatchValue::Some(rules) => {
				config.ignore_rules = Set(if rules.is_empty() {
					None
				} else {
					Some(IgnoreRules::new(rules.clone()).map_err(|error| {
						stump_komga::errors::APIError::BadRequest(error.to_string())
					})?)
				});
			},
			PatchValue::None => config.ignore_rules = Set(None),
		}

		// Upstream rescans when scan-affecting fields change; the root is the
		// one of those Stump can express. Komga patches have no watcher
		// concept, so watcher state is left untouched.
		let scan_after_persist = path != existing_library.path;

		let updated = stump_library::library::update_library(
			&self.ctx,
			user,
			id,
			stump_library::library::UpdatedLibrary {
				name,
				path,
				description: existing_library.description.clone(),
				emoji: existing_library.emoji.clone(),
				config: Some(config),
				tags: None,
				scan_after_persist,
				watch: stump_library::library::WatchUpdate::Keep,
			},
		)
		.await
		.map_err(map_core_error_status)?;

		if let Some(interval) = request.scan_interval {
			stump_library::library::sync_library_scan_schedule(
				&self.ctx,
				&updated.id,
				&updated.name,
				scan_interval_cron(interval),
			)
			.await
			.map_err(map_core_error_status)?;
		}

		Ok(())
	}

	async fn delete_library(
		&self,
		user: &AuthUser,
		id: &str,
	) -> stump_komga::errors::APIResult<()> {
		stump_library::library::delete_library(&self.ctx, user, id)
			.await
			.map_err(map_core_error_status)?;
		Ok(())
	}

	async fn enqueue_series_analysis(
		&self,
		series_id: String,
	) -> stump_komga::errors::APIResult<()> {
		self.ctx
			.enqueue(StumpJob::analyze_media(
				stump_core::filesystem::media::analysis::AnalysisJobConfig {
					force_reanalysis: true,
					scope:
						stump_core::filesystem::media::analysis::MediaAnalysisJobScope::Series(
							series_id,
						),
				},
			))
			.await
			.map_err(map_core_error)
	}

	fn library_roots(&self) -> Vec<String> {
		self.ctx.config.server.library_roots.clone()
	}

	#[cfg(feature = "providers")]
	async fn virtual_series_by_id(
		&self,
		user: &AuthUser,
		series_id: &str,
	) -> Option<stump_komga::errors::APIResult<stump_komga::KomgaSeries>> {
		let series =
			super::provider_virtual::virtual_series_by_id(&self.ctx, user, series_id)
				.await?;
		Some(Ok(series))
	}

	/// Materialise a live-only virtual series (or refresh a stored one).
	#[cfg(feature = "providers")]
	async fn virtual_materialise_series(
		&self,
		user: &AuthUser,
		series_id: &str,
	) -> stump_komga::errors::APIResult<bool> {
		match super::provider_virtual::materialise_virtual_series(
			&self.ctx, user, series_id,
		)
		.await
		{
			None => Ok(false),
			Some(Ok(_row)) => Ok(true),
			Some(Err(error)) => Err(map_core_error(error)),
		}
	}

	async fn create_collection(
		&self,
		user: &AuthUser,
		name: String,
		ordered: bool,
		series_ids: Vec<String>,
	) -> stump_komga::errors::APIResult<models::entity::collection::Model> {
		stump_collections::create_collection(
			&self.ctx,
			user,
			stump_collections::CollectionCreate {
				name,
				ordered,
				series_ids,
			},
		)
		.await
		.map_err(map_core_error_status)
	}

	async fn update_collection(
		&self,
		user: &AuthUser,
		id: &str,
		name: Option<String>,
		ordered: Option<bool>,
		series_ids: Option<Vec<String>>,
	) -> stump_komga::errors::APIResult<()> {
		stump_collections::update_collection(
			&self.ctx,
			user,
			id,
			stump_collections::CollectionUpdate {
				name,
				ordered,
				series_ids,
			},
		)
		.await
		.map(|_| ())
		.map_err(map_core_error_status)
	}

	async fn delete_collection(
		&self,
		user: &AuthUser,
		id: &str,
	) -> stump_komga::errors::APIResult<()> {
		stump_collections::delete_collection(&self.ctx, user, id)
			.await
			.map_err(map_core_error_status)
	}

	async fn create_read_list(
		&self,
		user: &AuthUser,
		name: String,
		summary: Option<String>,
		ordered: bool,
		book_ids: Vec<String>,
	) -> stump_komga::errors::APIResult<models::entity::reading_list::Model> {
		stump_collections::create_read_list(
			&self.ctx,
			user,
			stump_collections::ReadListCreate {
				name,
				summary,
				ordered,
				book_ids,
			},
		)
		.await
		.map_err(map_core_error_status)
	}

	async fn update_read_list(
		&self,
		user: &AuthUser,
		id: &str,
		name: Option<String>,
		summary: Option<Option<String>>,
		ordered: Option<bool>,
		book_ids: Option<Vec<String>>,
	) -> stump_komga::errors::APIResult<()> {
		stump_collections::update_read_list(
			&self.ctx,
			user,
			id,
			stump_collections::ReadListUpdate {
				name,
				summary,
				ordered,
				book_ids,
			},
		)
		.await
		.map(|_| ())
		.map_err(map_core_error_status)
	}

	async fn delete_read_list(
		&self,
		user: &AuthUser,
		id: &str,
	) -> stump_komga::errors::APIResult<()> {
		stump_collections::delete_read_list(&self.ctx, user, id)
			.await
			.map_err(map_core_error_status)
	}
}
