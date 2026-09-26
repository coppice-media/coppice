use axum::{
	body::Body,
	extract::{Path, State},
	http::{header, HeaderMap, HeaderValue, Response},
	middleware,
	response::IntoResponse,
	routing::{get, post},
	Extension, Router,
};
use models::{
	entity::{library, library_config, media, series, user::AuthUser},
	shared::{enums::UserPermission, image_processor_options::ImageProcessorOptions},
};
use sea_orm::{prelude::*, sea_query::Query, QuerySelect};
use stump_auth::AuthContext;
use stump_core::{config::StumpConfig, Ctx};
use stump_kindle::{BokoConverter, FormatPolicy, KindleError};
use stump_media::{
	get_saved_thumbnail, get_thumbnail,
	image::{generate_thumbnail_on_demand, on_demand_thumbnail_options},
	media::get_page_async,
	ContentType, FileError,
};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::auth::auth_middleware,
	routers::api::v2::audio::{get_audio_manifest, get_audio_track},
	utils::{http::ImageResponse, serve_media},
};

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	Router::new()
		.nest(
			"/media/{id}",
			Router::new()
				.route("/thumbnail", get(get_media_thumbnail_handler))
				.route("/page/{page}", get(get_media_page))
				.route("/audio/manifest", get(get_audio_manifest))
				.route("/audio/track/{index}", get(get_audio_track))
				.route("/file", get(get_media_file))
				.route("/kindle-file", post(get_media_kindle_file)),
		)
		.layer(middleware::from_fn_with_state(app_state, auth_middleware))
}

/// Download the file associated with the media.
pub(crate) async fn get_media_file(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	serve_media::serve_media_file_with_ctx(req, headers, &ctx, id).await
}

/// The **USB** send-to-Kindle path: download the book as a file a Kindle
/// mounted as a disk can open.
///
/// Amazon's e-mail service converts an EPUB on its side, but a Kindle plugged
/// into a computer has no gateway behind it: it opens AZW3/MOBI, PDF and text,
/// and nothing else. The route therefore runs the same lane as
/// `sendToKindle` with [`FormatPolicy::RequireKindle`] — an EPUB is converted
/// with the operator's `boko` or the download is refused, rather than saved
/// onto a device that cannot read it.
///
/// `POST` because it is not a plain read: it may run a converter and write a
/// temporary file. That temporary lives under the server's cache directory
/// and is removed when the response is built; the library file is never
/// touched.
///
/// The response is the file itself, with `Content-Disposition: attachment`
/// carrying `<book>.<format>`, so saving it into a mounted Kindle's
/// `documents/` folder is the whole sideload. `X-Stump-Kindle-Converted` says
/// whether the bytes came out of a conversion.
pub(crate) async fn get_media_kindle_file(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<impl IntoResponse> {
	let user = req
		.user_and_enforce_permissions(&[UserPermission::DownloadFile])
		.map_err(|_| {
			tracing::error!("User does not have permission to download file");
			APIError::forbidden_discreet()
		})?;

	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(id))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	let work_dir = tempfile::Builder::new()
		.prefix("kindle-usb-")
		.tempdir_in(ctx.config.get_cache_dir())
		.map_err(|error| {
			APIError::InternalServerError(format!(
				"Failed to create a working directory: {error}"
			))
		})?;

	let file = stump_kindle::prepare_file(
		std::path::Path::new(&book.path),
		FormatPolicy::RequireKindle,
		&BokoConverter,
		work_dir.path(),
	)
	.await
	.map_err(kindle_error)?;

	if let Some(note) = &file.note {
		tracing::debug!(%note, book = %book.id, "Serving a Kindle file unconverted");
	}

	// A quote or a newline in a book's file name would otherwise end the
	// header value early; every other character is legal in a quoted string.
	let filename = file
		.filename
		.replace(['"', '\\'], "_")
		.replace(['\r', '\n'], " ");
	let mut response = Response::new(Body::from(file.content));
	let headers = response.headers_mut();
	headers.insert(
		header::CONTENT_TYPE,
		HeaderValue::from_str(&file.content_type)
			.unwrap_or(HeaderValue::from_static("application/octet-stream")),
	);
	headers.insert(
		header::CONTENT_DISPOSITION,
		HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
			.unwrap_or(HeaderValue::from_static("attachment")),
	);
	headers.insert(
		"x-stump-kindle-converted",
		HeaderValue::from_static(if file.converted { "true" } else { "false" }),
	);
	Ok(response)
}

/// Maps the lane's refusals onto HTTP: anything the operator can act on (an
/// unsupported format, an EPUB with no converter installed, a file too small
/// to be a book) is a 400 carrying the lane's own sentence, because it is the
/// request that cannot be satisfied and not the server that failed.
fn kindle_error(error: KindleError) -> APIError {
	match error {
		KindleError::BookNotFound => APIError::NotFound(error.to_string()),
		KindleError::UnsupportedFormat { .. }
		| KindleError::ConversionRequired(_)
		| KindleError::TooLarge(_)
		| KindleError::Attachment(_) => APIError::BadRequest(error.to_string()),
		KindleError::Database(error) => APIError::DbError(error),
		error => {
			tracing::error!(?error, "Failed to build the Kindle file");
			APIError::InternalServerError(error.to_string())
		},
	}
}

/// The thumbnail for a book: its stored one, else the file the thumbnail job
/// or an earlier request left under `thumbnails/`, else one generated now
/// from its first page and kept for the next request. `thumbnail_config` is
/// the owning library's; a library without one gets
/// [`on_demand_thumbnail_options`] so no request ever serves the raw page
/// (1–2 MB at print resolution) when a bounded thumbnail can be encoded.
pub(crate) async fn get_media_thumbnail(
	book: &media::MediaThumbSelect,
	thumbnail_config: Option<ImageProcessorOptions>,
	config: &StumpConfig,
) -> APIResult<(ContentType, Vec<u8>)> {
	// Note: This doesn't hard-fail because if the saved thumbnail is missing or corrupt, we want
	// to just pull something else instead of erroring out entirely.
	if let Some(path) = &book.thumbnail_path {
		match get_saved_thumbnail(std::path::Path::new(path)).await {
			Ok(result) => return Ok(result),
			Err(_) => {
				tracing::warn!(path = ?path, "Failed to get saved thumbnail");
			},
		}
	}

	let image_format = thumbnail_config.as_ref().map(|options| options.format);
	if let Some(found) =
		get_thumbnail(config.get_thumbnails_dir(), &book.id, image_format).await?
	{
		return Ok(found);
	}

	let options = thumbnail_config.unwrap_or_else(on_demand_thumbnail_options);
	Ok(
		generate_thumbnail_on_demand(&book.id, &book.path, options, &config.media)
			.await?,
	)
}

pub(crate) async fn get_media_thumbnail_by_id(
	ctx: &Ctx,
	user: &AuthUser,
	book_id: String,
) -> APIResult<ImageResponse> {
	let book = media::Entity::find_for_user(user)
		.columns(media::MediaThumbSelect::columns())
		.filter(media::Column::Id.eq(book_id))
		.into_model::<media::MediaThumbSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;
	thumbnail_for_row(ctx, book).await
}

/// The same thumbnail, for a book resolved without a user.
///
/// The only caller is the Audiobookshelf profile's cover route, which is
/// unauthenticated by that protocol's design from 2.17.0 on and has already
/// narrowed the row to an audible, undeleted book.
#[cfg(feature = "abs")]
pub(crate) async fn get_media_thumbnail_unscoped(
	ctx: &Ctx,
	book_id: &str,
) -> APIResult<ImageResponse> {
	let book = media::Entity::find()
		.columns(media::MediaThumbSelect::columns())
		.filter(media::Column::Id.eq(book_id))
		.filter(media::Column::DeletedAt.is_null())
		.into_model::<media::MediaThumbSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;
	thumbnail_for_row(ctx, book).await
}

async fn thumbnail_for_row(
	ctx: &Ctx,
	book: media::MediaThumbSelect,
) -> APIResult<ImageResponse> {
	// Note: This doesn't hard-fail because if the saved thumbnail is missing or corrupt, we want
	// to just pull something else instead of erroring out entirely.
	if let Some(path) = &book.thumbnail_path {
		match get_saved_thumbnail(std::path::Path::new(path)).await {
			Ok(result) => return Ok(result.into()),
			Err(_) => {
				tracing::warn!(path = ?path, "Failed to get saved thumbnail");
			},
		}
	}

	let library_config = library_config::Entity::find()
		.filter(
			library_config::Column::LibraryId.in_subquery(
				Query::select()
					.column(library::Column::Id)
					.from(library::Entity)
					.and_where(
						library::Column::Id.in_subquery(
							Query::select()
								.column(series::Column::LibraryId)
								.from(series::Entity)
								.and_where(series::Column::Id.eq(book.series_id.clone()))
								.to_owned(),
						),
					)
					.to_owned(),
			),
		)
		.one(ctx.conn.as_ref())
		.await?;
	let thumbnail_config = library_config.and_then(|o| o.thumbnail_config);

	get_media_thumbnail(&book, thumbnail_config, ctx.config.as_ref())
		.await
		.map(ImageResponse::from)
}

pub(crate) async fn get_media_thumbnail_handler(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<ImageResponse> {
	get_media_thumbnail_by_id(&ctx, &req.user(), id).await
}

pub(crate) async fn get_media_page(
	Path((id, page)): Path<(String, u32)>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<ImageResponse> {
	let book = media::Entity::find_for_user(&req.user())
		.filter(media::Column::Id.eq(id.clone()))
		.into_model::<media::Model>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	// Duplicate-page skipping renumbers visible pages onto the physical file;
	// a visible page past the end of the renumbered list does not exist.
	let cache = ctx.visible_pages_cache();
	let visible = stump_core::filesystem::media::visible_pages::visible_pages(
		ctx.conn.as_ref(),
		&cache,
		&book.id,
		book.pages,
	)
	.await?;
	let physical = stump_core::filesystem::media::visible_pages::physical_page(
		&visible,
		page as i32,
	)
	.ok_or(APIError::NotFound("Page not found".to_string()))?;

	let content = match get_page_async(&book.path, physical, &ctx.config.media).await {
		Ok(result) => result,
		Err(FileError::NoImageError) => {
			return Err(APIError::NotFound("Page not found".to_string()))
		},
		// Everything else is classified centrally, so a provider chapter the
		// source no longer serves reads as 404 here exactly as it does on the
		// Komga, OPDS, and Kavita lanes.
		Err(error) => return Err(APIError::from(error)),
	};

	Ok(ImageResponse::from(content))
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use models::shared::image_processor_options::{
		ExactDimensionResize, ImageResizeMethod, SupportedImageFormat,
	};
	use stump_media::image::{GenericImageProcessor, ImageProcessor};
	use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

	use super::*;

	/// A one-page CBZ whose page is the media crate's fixture photo blown up
	/// to print resolution, in a config directory of its own.
	fn book_in_fresh_config(
		root: &std::path::Path,
	) -> (StumpConfig, media::MediaThumbSelect, usize) {
		let mut config = StumpConfig::new(root.to_string_lossy().to_string());
		config.finalize();
		std::fs::create_dir_all(config.get_thumbnails_dir()).expect("thumbnails dir");

		let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("../../crates/media/integration-tests/data/example.png");
		let page = GenericImageProcessor::generate_from_path(
			&fixture.to_string_lossy(),
			ImageProcessorOptions {
				resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
					width: 1500,
					height: 2100,
				})),
				format: SupportedImageFormat::Png,
				quality: None,
				page: None,
			},
		)
		.expect("upscaled fixture");

		let book_path = root.join("book.cbz");
		let mut zip =
			ZipWriter::new(std::fs::File::create(&book_path).expect("create cbz"));
		zip.start_file(
			"001.png",
			SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
		)
		.expect("start entry");
		zip.write_all(&page).expect("write entry");
		zip.finish().expect("finish cbz");

		let book = media::MediaThumbSelect {
			id: "book-1".to_owned(),
			path: book_path.to_string_lossy().to_string(),
			series_id: "series-1".to_owned(),
			thumbnail_path: None,
			thumbnail_meta: None,
		};
		(config, book, page.len())
	}

	#[tokio::test]
	async fn unconfigured_library_gets_a_kept_bounded_thumbnail_not_the_page() {
		let tempdir = tempfile::tempdir().expect("tempdir");
		let (config, book, page_len) = book_in_fresh_config(tempdir.path());

		let (content_type, first) = get_media_thumbnail(&book, None, &config)
			.await
			.expect("first request");
		assert_eq!(content_type, ContentType::WEBP);
		assert!(first.len() * 8 < page_len);

		let saved = config.get_thumbnails_dir().join("book-1.webp");
		assert_eq!(std::fs::read(&saved).expect("kept thumbnail"), first);

		// With the book gone, only the kept file can answer: the second
		// request reads it instead of extracting the page again.
		std::fs::remove_file(&book.path).expect("remove book");
		let (content_type, second) = get_media_thumbnail(&book, None, &config)
			.await
			.expect("second request");
		assert_eq!(content_type, ContentType::WEBP);
		assert_eq!(second, first);
	}

	#[tokio::test]
	async fn configured_library_keeps_its_own_format() {
		let tempdir = tempfile::tempdir().expect("tempdir");
		let (config, book, _) = book_in_fresh_config(tempdir.path());
		let thumbnail_config = ImageProcessorOptions {
			resize_method: Some(ImageResizeMethod::Exact(ExactDimensionResize {
				width: 100,
				height: 150,
			})),
			format: SupportedImageFormat::Jpeg,
			quality: None,
			page: None,
		};

		let (content_type, _) =
			get_media_thumbnail(&book, Some(thumbnail_config), &config)
				.await
				.expect("first request");
		assert_eq!(content_type, ContentType::JPEG);
		assert!(config.get_thumbnails_dir().join("book-1.jpeg").is_file());
		assert!(!config.get_thumbnails_dir().join("book-1.webp").exists());
	}
}
