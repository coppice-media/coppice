use std::path::{Path, PathBuf};

use axum::{
	extract::{Path as AxumPath, State},
	http::{header, HeaderMap, HeaderValue},
	response::{IntoResponse, Response},
	Json,
};
use models::{
	domain::audio::AudioChapterSource,
	entity::{
		book_club_book,
		media::{self},
		media_audio, media_audio_chapter, media_audio_track, media_metadata,
	},
	shared::enums::FileStatus,
};
use sea_orm::{prelude::*, QueryOrder};
use stump_core::filesystem::media::visible_pages::{
	physical_page, visible_pages as visible_book_pages,
};
use stump_media::{
	media::format::epub::EpubProcessor, media::get_page_async, ContentType,
	ReadiumManifestGenerator,
};
use tokio::fs;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::HostExtractor,
	utils::{
		http::{BufferResponse, ImageResponse},
		serve_media::serve_reader_file,
	},
};

use super::{reader::GuestAccess, types::*};

const EPUB_RESOURCE_CSP: &str = "default-src 'none'; script-src 'none'; object-src 'none'; connect-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; media-src 'self'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'; sandbox allow-same-origin";

/// The title Home shows for a library book (GraphQL `resolvedName`): the
/// metadata title when present, otherwise the file-derived name.
pub(super) async fn media_display_title(
	conn: &DatabaseConnection,
	media: &media::Model,
) -> APIResult<String> {
	let metadata_title = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.eq(&media.id))
		.one(conn)
		.await?
		.and_then(|metadata| metadata.title)
		.filter(|title| !title.trim().is_empty());
	Ok(metadata_title.unwrap_or_else(|| media.name.clone()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ValidationMode {
	Request,
	Snapshot,
	Publish,
}

pub(super) async fn media_for_club_book(
	ctx: &AppState,
	viewer: &models::entity::user::AuthUser,
	book: &book_club_book::Model,
	session_id: &str,
	mode: ValidationMode,
) -> APIResult<(media::Model, ReaderBook)> {
	let media_id = book
		.book_entity_id
		.as_deref()
		.ok_or_else(|| APIError::NotFound("Stored reader book not found".to_string()))?;
	let media = media::Entity::find_for_user(viewer)
		.filter(media::Column::Id.eq(media_id))
		.filter(media::Column::DeletedAt.is_null())
		.into_model::<media::Model>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Stored reader book not found".to_string()))?;
	if media.status != FileStatus::Ready {
		return Err(APIError::NotFound("Reader book is unavailable".to_string()));
	}

	let content_type = ContentType::from_extension(&media.extension);
	let kind = if content_type.is_epub() {
		ReaderKind::Epub
	} else if content_type.is_audio() {
		ReaderKind::Audio
	} else if content_type == ContentType::PDF
		|| content_type == ContentType::COMIC_ZIP
		|| content_type == ContentType::COMIC_RAR
		|| content_type.is_image()
	{
		ReaderKind::Paged
	} else {
		return Err(APIError::NotFound(
			"Reader format is unavailable".to_string(),
		));
	};
	let metadata = fs::metadata(&media.path)
		.await
		.map_err(|_| APIError::NotFound("Reader book is unavailable".to_string()))?;
	if !metadata.is_file() && !(matches!(kind, ReaderKind::Audio) && metadata.is_dir()) {
		return Err(APIError::NotFound("Reader book is unavailable".to_string()));
	}

	if mode == ValidationMode::Publish {
		match kind {
			ReaderKind::Epub => {
				let path = media.path.clone();
				tokio::task::spawn_blocking(move || {
					ReadiumManifestGenerator::new(path, "https://reader.invalid")
						.generate_manifest()
				})
				.await
				.map_err(|_| {
					APIError::NotFound("Reader book is unavailable".to_string())
				})?
				.map_err(|_| {
					APIError::NotFound("Reader book is unavailable".to_string())
				})?;
			},
			ReaderKind::Paged => {
				let visible = visible_book_pages(
					ctx.conn.as_ref(),
					&ctx.visible_pages_cache(),
					&media.id,
					media.pages,
				)
				.await?;
				let physical = visible.first().copied().ok_or_else(|| {
					APIError::NotFound("Reader pages are unavailable".to_string())
				})?;
				get_page_async(&media.path, physical, &ctx.config.media)
					.await
					.map_err(|_| {
						APIError::NotFound("Reader pages are unavailable".to_string())
					})?;
			},
			ReaderKind::Audio => {},
		}
	}
	if matches!(kind, ReaderKind::Audio) {
		media_audio::Entity::find_by_id(&media.id)
			.one(ctx.conn.as_ref())
			.await?
			.ok_or_else(|| {
				APIError::NotFound("Reader audio is unavailable".to_string())
			})?;
		if mode != ValidationMode::Request {
			let tracks = media_audio_track::Entity::find()
				.filter(media_audio_track::Column::MediaId.eq(&media.id))
				.order_by_asc(media_audio_track::Column::Index)
				.all(ctx.conn.as_ref())
				.await?;
			if tracks.is_empty() {
				return Err(APIError::NotFound(
					"Reader audio is unavailable".to_string(),
				));
			}
			for track in tracks {
				let track_metadata = fs::metadata(&track.path).await.map_err(|_| {
					APIError::NotFound("Reader audio is unavailable".to_string())
				})?;
				if !track_metadata.is_file()
					|| track_metadata.len() != track.byte_size.max(0) as u64
				{
					return Err(APIError::NotFound(
						"Reader audio is unavailable".to_string(),
					));
				}
			}
		}
	}

	let (page_count, visible_pages) = if matches!(kind, ReaderKind::Paged) {
		let pages = visible_book_pages(
			ctx.conn.as_ref(),
			&ctx.visible_pages_cache(),
			&media.id,
			media.pages,
		)
		.await?;
		(pages.len() as i32, pages.iter().copied().collect())
	} else {
		(0, Vec::new())
	};
	let audio = if matches!(kind, ReaderKind::Audio) {
		Some(audio_metadata(ctx, session_id, &media.id, &book.id).await?)
	} else {
		None
	};
	let title = match book.title.clone() {
		Some(title) => title,
		None => media_display_title(ctx.conn.as_ref(), &media).await?,
	};
	Ok((
		media.clone(),
		ReaderBook {
			id: book.id.clone(),
			title,
			extension: media.extension.to_lowercase(),
			reader_kind: kind,
			page_count,
			visible_pages,
			audio,
		},
	))
}

async fn audio_metadata(
	ctx: &AppState,
	session_id: &str,
	media_id: &str,
	club_book_id: &str,
) -> APIResult<ReaderAudio> {
	let audio = media_audio::Entity::find_by_id(media_id)
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Reader audio is unavailable".to_string()))?;
	let tracks = media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(media_id))
		.order_by_asc(media_audio_track::Column::Index)
		.all(ctx.conn.as_ref())
		.await?;
	let chapters = media_audio_chapter::Entity::find()
		.filter(media_audio_chapter::Column::MediaId.eq(media_id))
		.order_by_asc(media_audio_chapter::Column::Index)
		.all(ctx.conn.as_ref())
		.await?;
	Ok(ReaderAudio {
		duration_ms: audio.duration_ms,
		codec: audio.codec,
		chapter_source: graphql_chapter_source(&audio.chapter_source).to_string(),
		tracks: tracks
			.into_iter()
			.map(|track| ReaderAudioTrack {
				index: track.index,
				mime: track.mime,
				duration_ms: track.duration_ms,
				start_offset_ms: track.start_offset_ms,
				byte_size: track.byte_size,
				url: format!(
					"/api/v2/club-reader/sessions/{session_id}/books/{club_book_id}/audio/{}",
					track.index
				),
			})
			.collect(),
		chapters: chapters
			.into_iter()
			.map(|chapter| ReaderAudioChapter {
				index: chapter.index,
				title: chapter.title,
				start_ms: chapter.start_ms,
				end_ms: chapter.end_ms,
			})
			.collect(),
	})
}

/// The GraphQL enum spelling of [`AudioChapterSource`], which the guest reader
/// shares with the authenticated reader (`chapterSource === 'PER_TRACK'`).
/// Matches `crates/graphql/schema.graphql` (async-graphql's default renaming).
fn graphql_chapter_source(source: &AudioChapterSource) -> &'static str {
	match source {
		AudioChapterSource::Mp4Chpl => "MP_4_CHPL",
		AudioChapterSource::Mp4ChapterTrack => "MP_4_CHAPTER_TRACK",
		AudioChapterSource::Id3Chap => "ID_3_CHAP",
		AudioChapterSource::VorbisComment => "VORBIS_COMMENT",
		AudioChapterSource::PerTrack => "PER_TRACK",
		AudioChapterSource::None => "NONE",
	}
}

/// The club's true current book: the first uncompleted queue row. A guest may
/// only read or write the published book while it is still this head; a
/// reorder that moves another book ahead pauses the session.
pub(super) async fn queue_head<C: ConnectionTrait>(
	conn: &C,
	club_id: &str,
) -> Result<Option<book_club_book::Model>, DbErr> {
	book_club_book::Entity::find_current_for_book_club_id(club_id)
		.order_by_asc(book_club_book::Column::Id)
		.one(conn)
		.await
}

/// The published book while it is still the queue head: `None` when nothing is
/// published or the session is paused.
pub(super) async fn live_book_id<C: ConnectionTrait>(
	conn: &C,
	club_id: &str,
	published_book_id: Option<&str>,
) -> Result<Option<String>, DbErr> {
	let Some(book_id) = published_book_id else {
		return Ok(None);
	};
	Ok(queue_head(conn, club_id)
		.await?
		.map(|head| head.id)
		.filter(|head| head == book_id))
}

pub(super) async fn snapshot_book(
	ctx: &AppState,
	access: &GuestAccess,
) -> APIResult<Option<ReaderBook>> {
	let Some(book_id) = access.session.published_book_id.as_deref() else {
		return Ok(None);
	};
	let Some(current) =
		queue_head(ctx.conn.as_ref(), &access.session.book_club_id).await?
	else {
		return Ok(None);
	};
	if current.id != book_id {
		return Ok(None);
	}
	let (_, book) = media_for_club_book(
		ctx,
		&access.publisher,
		&current,
		&access.session.id,
		ValidationMode::Snapshot,
	)
	.await?;
	Ok(Some(book))
}

/// The published book is no longer the club's queue head (reordered or
/// completed elsewhere): the session is paused until a manager republishes.
pub(super) fn paused_error() -> APIError {
	APIError::Conflict("Reader session is paused".to_string())
}

pub(super) async fn active_book(
	ctx: &AppState,
	access: &GuestAccess,
	book_id: &str,
) -> APIResult<(book_club_book::Model, media::Model, ReaderBook)> {
	if access.session.published_book_id.as_deref() != Some(book_id) {
		return Err(APIError::NotFound("Reader book not found".to_string()));
	}
	let current = queue_head(ctx.conn.as_ref(), &access.session.book_club_id)
		.await?
		.filter(|head| head.id == book_id)
		.ok_or_else(paused_error)?;
	let (media, reader_book) = media_for_club_book(
		ctx,
		&access.publisher,
		&current,
		&access.session.id,
		ValidationMode::Request,
	)
	.await?;
	Ok((current, media, reader_book))
}

pub(super) async fn manifest(
	AxumPath((session_id, book_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	headers: HeaderMap,
) -> APIResult<Response> {
	let access = super::reader::load_access(&ctx, &session_id, &headers).await?;
	let (_, media, _) = active_book(&ctx, &access, &book_id).await?;
	if !ContentType::from_extension(&media.extension).is_epub() {
		return Err(APIError::NotFound("Reader book not found".to_string()));
	}
	let base_url = host.url_for_path(&format!(
		"api/v2/club-reader/sessions/{session_id}/books/{book_id}"
	));
	let path = media.path;
	let manifest = tokio::task::spawn_blocking(move || {
		ReadiumManifestGenerator::new(path, base_url).generate_manifest()
	})
	.await
	.map_err(|_| APIError::NotFound("Reader book is unavailable".to_string()))?
	.map_err(|_| APIError::NotFound("Reader book is unavailable".to_string()))?;
	Ok((
		[(header::CONTENT_TYPE, "application/webpub+json")],
		Json(manifest),
	)
		.into_response())
}

pub(super) async fn positions(
	AxumPath((session_id, book_id)): AxumPath<(String, String)>,
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	headers: HeaderMap,
) -> APIResult<Response> {
	let access = super::reader::load_access(&ctx, &session_id, &headers).await?;
	let (_, media, _) = active_book(&ctx, &access, &book_id).await?;
	if !ContentType::from_extension(&media.extension).is_epub() {
		return Err(APIError::NotFound("Reader book not found".to_string()));
	}
	let base_url = host.url_for_path(&format!(
		"api/v2/club-reader/sessions/{session_id}/books/{book_id}"
	));
	let path = media.path;
	let positions = tokio::task::spawn_blocking(move || {
		ReadiumManifestGenerator::new(path, base_url).generate_positions()
	})
	.await
	.map_err(|_| APIError::NotFound("Reader book is unavailable".to_string()))?
	.map_err(|_| APIError::NotFound("Reader book is unavailable".to_string()))?;
	Ok((
		[(
			header::CONTENT_TYPE,
			"application/vnd.readium.position-list+json",
		)],
		Json(positions),
	)
		.into_response())
}

pub(super) async fn resource(
	AxumPath((session_id, book_id, path)): AxumPath<(String, String, String)>,
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<Response> {
	let access = super::reader::load_access(&ctx, &session_id, &headers).await?;
	let (_, media, _) = active_book(&ctx, &access, &book_id).await?;
	if !ContentType::from_extension(&media.extension).is_epub() {
		return Err(APIError::NotFound("Reader resource not found".to_string()));
	}
	let package_path = validate_package_path(&path)?;
	let archive_path = media.path;
	let (content_type, data) = tokio::task::spawn_blocking(move || {
		let root = package_path
			.parent()
			.map(|parent| parent.to_string_lossy().into_owned())
			.unwrap_or_default();
		let resource = package_path
			.file_name()
			.map(PathBuf::from)
			.unwrap_or(package_path);
		EpubProcessor::get_resource_by_path(&archive_path, &root, resource)
	})
	.await
	.map_err(|_| APIError::NotFound("Reader resource not found".to_string()))?
	.map_err(|_| APIError::NotFound("Reader resource not found".to_string()))?;
	let mut response = BufferResponse::from((content_type, data)).into_response();
	response.headers_mut().insert(
		header::CONTENT_SECURITY_POLICY,
		HeaderValue::from_static(EPUB_RESOURCE_CSP),
	);
	Ok(response)
}

fn validate_package_path(path: &str) -> APIResult<PathBuf> {
	if path.is_empty() || path.len() > 2_048 || path.contains('\\') {
		return Err(APIError::NotFound("Reader resource not found".to_string()));
	}
	let package_path = Path::new(path);
	if package_path
		.components()
		.any(|component| !matches!(component, std::path::Component::Normal(_)))
	{
		return Err(APIError::NotFound("Reader resource not found".to_string()));
	}
	Ok(package_path.to_path_buf())
}

pub(super) async fn page(
	AxumPath((session_id, book_id, page)): AxumPath<(String, String, i32)>,
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<ImageResponse> {
	if page <= 0 {
		return Err(APIError::NotFound("Reader page not found".to_string()));
	}
	let access = super::reader::load_access(&ctx, &session_id, &headers).await?;
	let (_, media, book) = active_book(&ctx, &access, &book_id).await?;
	if !matches!(book.reader_kind, ReaderKind::Paged) {
		return Err(APIError::NotFound("Reader page not found".to_string()));
	}
	let cache = ctx.visible_pages_cache();
	let visible =
		visible_book_pages(ctx.conn.as_ref(), &cache, &media.id, media.pages).await?;
	let physical = physical_page(&visible, page)
		.ok_or_else(|| APIError::NotFound("Reader page not found".to_string()))?;
	let content = get_page_async(&media.path, physical, &ctx.config.media)
		.await
		.map_err(|_| APIError::NotFound("Reader page not found".to_string()))?;
	Ok(ImageResponse::from(content))
}

pub(super) async fn audio_track(
	AxumPath((session_id, book_id, track_index)): AxumPath<(String, String, i32)>,
	State(ctx): State<AppState>,
	headers: HeaderMap,
) -> APIResult<Response> {
	if track_index < 0 {
		return Err(APIError::NotFound(
			"Reader audio track not found".to_string(),
		));
	}
	let access = super::reader::load_access(&ctx, &session_id, &headers).await?;
	let (_, media, book) = active_book(&ctx, &access, &book_id).await?;
	if !matches!(book.reader_kind, ReaderKind::Audio) {
		return Err(APIError::NotFound(
			"Reader audio track not found".to_string(),
		));
	}
	let track = media_audio_track::Entity::find()
		.filter(media_audio_track::Column::MediaId.eq(&media.id))
		.filter(media_audio_track::Column::Index.eq(track_index))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or_else(|| APIError::NotFound("Reader audio track not found".to_string()))?;
	let metadata = fs::metadata(&track.path)
		.await
		.map_err(|_| APIError::NotFound("Reader audio track not found".to_string()))?;
	if !metadata.is_file() || metadata.len() != track.byte_size.max(0) as u64 {
		return Err(APIError::NotFound(
			"Reader audio track not found".to_string(),
		));
	}
	serve_reader_file(&track.path, headers, &track.mime).await
}
