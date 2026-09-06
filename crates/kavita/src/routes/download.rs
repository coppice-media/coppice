//! `DownloadController`: the size clients quote before downloading a chapter,
//! and the file downloads themselves.
//!
//! Turnleaf downloads EPUBs with `GET /api/Download/chapter?chapterId=`
//! (`src/lib/kavita/client.ts`, pin `b54f1f71`) and Inkita uses all three
//! routes (`core/downloadv2/strategies/DownloadApiStrategyV2.kt:198-202`).
//! Every route here sits behind Kavita's `RequireDownloadRole` policy, which
//! Stump satisfies with [`UserPermission::DownloadFile`] — the permission
//! `Account/roles` already reports as the `Download` role.
//!
//! Reference responses (`kavita-ref` 0.9.1.4, 2026-09-06):
//!
//! ```text
//! GET /api/Download/chapter?chapterId=4 -> 200 application/epub+zip
//!   Content-Disposition: attachment; filename=alice.epub; filename*=UTF-8''alice.epub
//!   Content-Length: 189886, Accept-Ranges: bytes; Range: bytes=0-99 -> 206
//! GET /api/Download/volume?volumeId=1  -> 200 application/x-cbz, the file itself
//! GET /api/Download/series?seriesId=1  -> 200 application/x-cbz, the single file
//! GET /api/Download/series?seriesId=8  -> 200 application/octet-stream
//!   Content-Disposition: attachment; filename=multi.zip; filename*=UTF-8''multi.zip
//!   (a zip of `multi v01.cbz` + `multi v02.cbz`, one entry per file)
//! ```

use std::{
	collections::HashSet,
	io::{Cursor, Write},
	sync::Arc,
};

use axum::{
	body::Body,
	extract::Query,
	http::{header, HeaderMap, HeaderValue, Response},
	routing::get,
	Extension, Json, Router,
};
use models::{
	entity::{media, user::AuthUser},
	shared::enums::UserPermission,
};
use serde::Deserialize;
use stump_auth::AuthContext;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::errors::{APIError, APIResult};

use super::{
	is_provider_media,
	query::{find_media, find_series_input},
	route_ci, KavitaBackend,
};

const COMIC_ZIP: &str = "application/x-cbz";
const OCTET_STREAM: &str = "application/octet-stream";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VolumeQuery {
	#[serde(default)]
	volume_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesQuery {
	#[serde(default)]
	series_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = route_ci(
		Router::<S>::new(),
		"/api/Download/chapter-size",
		get(chapter_size),
	);
	let router = route_ci(router, "/api/Download/chapter", get(download_chapter));
	let router = route_ci(router, "/api/Download/volume", get(download_volume));
	route_ci(router, "/api/Download/series", get(download_series))
}

/// `DownloadController` is annotated with Kavita's `RequireDownloadRole`
/// policy, so every route in it — `chapter-size` included — answers `403` for
/// a user without the `Download` role (`kavita-ref`: empty-bodied 403 for a
/// `Pleb`/`Login` user). Stump's equivalent is `DownloadFile`.
fn enforce_download(auth: &AuthContext) -> APIResult<AuthUser> {
	auth.user_and_enforce_permissions(&[UserPermission::DownloadFile])
		.map_err(|_| {
			APIError::Forbidden("You do not have permission to download files".to_owned())
		})
}

/// Kavita labels a download by file extension rather than by sniffing, and
/// falls back to `application/octet-stream` — which is what `kavita-ref`
/// answers for the zip it builds for a multi-file series, despite the `.zip`
/// name.
fn download_content_type(extension: &str) -> &'static str {
	match extension
		.trim_start_matches('.')
		.to_ascii_lowercase()
		.as_str()
	{
		"cbz" => COMIC_ZIP,
		"cbr" => "application/x-cbr",
		"cb7" => "application/x-cb7",
		"cbt" => "application/x-cbt",
		"epub" => "application/epub+zip",
		"pdf" => "application/pdf",
		_ => OCTET_STREAM,
	}
}

/// A single path component safe to put in a header and in a zip entry: no
/// separators, no quoting characters, no control bytes.
fn sanitize_file_name(name: &str) -> String {
	let sanitized = name.replace(
		|c: char| matches!(c, '/' | '\\' | '"' | ':') || c.is_control(),
		"_",
	);
	if sanitized.is_empty() {
		"download".to_owned()
	} else {
		sanitized
	}
}

/// The name a client sees for a media item's download: the file's own name on
/// disk, as `kavita-ref` serves it (`filename=science_comics_001.cbz`). A
/// provider-backed row has no file, so its generated CBZ is named after the
/// chapter, exactly like `ProviderHost::build_archive` names it.
fn download_file_name(media: &media::Model) -> String {
	if is_provider_media(media) {
		return format!("{}.cbz", sanitize_file_name(&media.name));
	}
	media
		.path
		.rsplit(['/', '\\'])
		.next()
		.filter(|name| !name.is_empty())
		.map(sanitize_file_name)
		.unwrap_or_else(|| sanitize_file_name(&media.name))
}

/// `attachment` with both the plain and the RFC 5987 file name, the pair
/// `kavita-ref` emits: `attachment; filename="multi v01.cbz";
/// filename*=UTF-8''multi%20v01.cbz`. The plain name is always quoted, which
/// ASP.NET only does for names that are not a bare token.
fn content_disposition(file_name: &str) -> String {
	format!(
		"attachment; filename=\"{file_name}\"; filename*=UTF-8''{}",
		urlencoding::encode(file_name)
	)
}

/// Relabel a response as a Kavita download, keeping everything else the file
/// service produced (status, `Content-Length`, `Accept-Ranges`,
/// `Content-Range`).
fn set_download_headers(
	response: &mut Response<Body>,
	file_name: &str,
	content_type: &str,
) {
	let headers = response.headers_mut();
	headers.insert(
		header::CONTENT_TYPE,
		HeaderValue::from_str(content_type)
			.unwrap_or_else(|_| HeaderValue::from_static(OCTET_STREAM)),
	);
	headers.insert(
		header::CONTENT_DISPOSITION,
		HeaderValue::from_str(&content_disposition(file_name))
			.unwrap_or_else(|_| HeaderValue::from_static("attachment")),
	);
}

/// A fully buffered download (a generated CBZ or the series zip): no file to
/// range over, so the body is served whole with its length.
fn bytes_response(data: Vec<u8>, file_name: &str, content_type: &str) -> Response<Body> {
	let length = data.len();
	let mut response = Response::new(Body::from(data));
	response.headers_mut().insert(
		header::CONTENT_LENGTH,
		HeaderValue::from_str(&length.to_string())
			.unwrap_or_else(|_| HeaderValue::from_static("0")),
	);
	set_download_headers(&mut response, file_name, content_type);
	response
}

/// Make `name` unique within one zip: a series can hold two files with the
/// same name in different folders, and a duplicate entry would make the
/// archive undecodable for half the clients that read it.
fn unique_entry_name(taken: &mut HashSet<String>, name: String) -> String {
	if taken.insert(name.clone()) {
		return name;
	}
	let (stem, extension) = match name.rsplit_once('.') {
		Some((stem, extension)) => (stem, format!(".{extension}")),
		None => (name.as_str(), String::new()),
	};
	for suffix in 2.. {
		let candidate = format!("{stem}-{suffix}{extension}");
		if taken.insert(candidate.clone()) {
			return candidate;
		}
	}
	unreachable!("the suffix range is unbounded")
}

/// One zip holding every member's download payload, in series order. Entries
/// are stored rather than deflated (which is what `kavita-ref` does): every
/// payload Stump serves here — CBZ, EPUB, PDF — is already compressed, so
/// deflating it again costs CPU for no bytes.
fn zip_entries(entries: Vec<(String, Vec<u8>)>) -> APIResult<Vec<u8>> {
	let mut cursor = Cursor::new(Vec::new());
	{
		let mut writer = ZipWriter::new(&mut cursor);
		let options =
			SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
		for (name, data) in entries {
			writer
				.start_file(name, options)
				.map_err(|error| APIError::InternalServerError(error.to_string()))?;
			writer
				.write_all(&data)
				.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		}
		writer
			.finish()
			.map_err(|error| APIError::InternalServerError(error.to_string()))?;
	}
	Ok(cursor.into_inner())
}

/// `DownloadController.GetChapterSize`: the chapter's size in bytes. Kavita
/// sums the sizes of the chapter's files and answers `200` with `0` for a
/// chapter it cannot resolve (`kavita-ref` `chapterId=9999` → `0`), so an
/// unknown or invisible chapter is not an error here either.
pub(crate) async fn chapter_size_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
) -> APIResult<i64> {
	let Some((input, index)) = find_media(ctx, user, chapter_id).await? else {
		return Ok(0);
	};
	Ok(input.media[index].media.size)
}

async fn chapter_size(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterQuery>,
) -> APIResult<Json<i64>> {
	let user = enforce_download(&auth)?;
	Ok(Json(
		chapter_size_for(ctx.as_ref(), &user, query.chapter_id.unwrap_or_default())
			.await?,
	))
}

/// Serve one media item as a download: a stored file goes through the same
/// file service every other Stump download uses (so `Range`,
/// `Content-Length` and conditional requests keep working), and a
/// provider-backed row is packed into a CBZ from its pages.
async fn serve_media_download(
	ctx: &dyn KavitaBackend,
	auth: AuthContext,
	headers: HeaderMap,
	media: &media::Model,
) -> APIResult<Response<Body>> {
	let file_name = download_file_name(media);
	if is_provider_media(media) {
		let data = ctx.media_bytes(&auth.user(), &media.id).await?;
		return Ok(bytes_response(data, &file_name, COMIC_ZIP));
	}
	let mut response = ctx.serve_media_file(auth, headers, &media.id).await?;
	set_download_headers(
		&mut response,
		&file_name,
		download_content_type(&media.extension),
	);
	Ok(response)
}

/// `DownloadController.DownloadChapter`/`DownloadVolume`. A Stump media item
/// is both the Kavita volume and its single chapter (`volumeId == chapterId`,
/// see [`crate::ids`]), so the two routes are the same download.
async fn download_media(
	ctx: &dyn KavitaBackend,
	auth: AuthContext,
	headers: HeaderMap,
	id: i32,
) -> APIResult<Response<Body>> {
	let user = enforce_download(&auth)?;
	let (input, index) = find_media(ctx, &user, id)
		.await?
		.ok_or_else(|| APIError::NotFound("Chapter does not exist".to_owned()))?;
	let media = input.media[index].media.clone();
	serve_media_download(ctx, auth, headers, &media).await
}

async fn download_chapter(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	download_media(
		ctx.as_ref(),
		auth,
		headers,
		query.chapter_id.unwrap_or_default(),
	)
	.await
}

async fn download_volume(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<VolumeQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	download_media(
		ctx.as_ref(),
		auth,
		headers,
		query.volume_id.unwrap_or_default(),
	)
	.await
}

/// `DownloadController.DownloadSeries`: the file itself when the series holds
/// exactly one (which is every series of a Book/LightNovel library, resolved
/// through the same [`find_series_input`] path as any other `seriesId`), and
/// one `<series>.zip` when it holds more.
async fn download_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesQuery>,
	headers: HeaderMap,
) -> APIResult<Response<Body>> {
	let ctx = ctx.as_ref();
	let user = enforce_download(&auth)?;
	let input = find_series_input(ctx, &user, query.series_id.unwrap_or_default())
		.await?
		.ok_or_else(|| APIError::NotFound("Invalid Series".to_owned()))?;
	let Some((first, rest)) = input.media.split_first() else {
		return Err(APIError::NotFound("Invalid Series".to_owned()));
	};
	if rest.is_empty() {
		let media = first.media.clone();
		return serve_media_download(ctx, auth, headers, &media).await;
	}

	let mut taken = HashSet::with_capacity(input.media.len());
	let mut entries = Vec::with_capacity(input.media.len());
	for member in &input.media {
		let name = unique_entry_name(&mut taken, download_file_name(&member.media));
		entries.push((name, ctx.media_bytes(&user, &member.media.id).await?));
	}
	Ok(bytes_response(
		zip_entries(entries)?,
		&format!("{}.zip", sanitize_file_name(&input.series.name)),
		OCTET_STREAM,
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ids::{IdKind, KavitaIds};
	use crate::routes::series::list_volumes;
	use crate::test_support::{
		auth_user, db, library_of_type, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;
	use sea_orm::{ActiveModelTrait, ActiveValue::Set};

	/// The authenticated Kavita router, with the whole response kept: the
	/// download contract is headers and bytes, not JSON.
	async fn download(
		backend: Arc<TestBackend>,
		user: &AuthUser,
		uri: &str,
		range: Option<&str>,
	) -> (axum::http::StatusCode, HeaderMap, Vec<u8>) {
		use tower::ServiceExt;
		let router = crate::routes::router::<()>(backend).layer(Extension(AuthContext {
			user: user.clone(),
			api_key: None,
			device_id: None,
		}));
		let mut builder = axum::http::Request::builder().method("GET").uri(uri);
		if let Some(range) = range {
			builder = builder.header(header::RANGE, range);
		}
		let response = router
			.oneshot(builder.body(Body::empty()).expect("request"))
			.await
			.expect("router response");
		let status = response.status();
		let headers = response.headers().clone();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
			.await
			.expect("response body")
			.to_vec();
		(status, headers, bytes)
	}

	fn header(headers: &HeaderMap, name: header::HeaderName) -> String {
		headers
			.get(name)
			.and_then(|value| value.to_str().ok())
			.unwrap_or_default()
			.to_owned()
	}

	/// A plain user (not a server owner, who bypasses permission checks) who
	/// holds the `Download` role.
	fn downloader(user: &models::entity::user::Model) -> AuthUser {
		let mut user = AuthUser {
			is_server_owner: false,
			..auth_user(user)
		};
		user.permissions.push(UserPermission::DownloadFile);
		user
	}

	/// `chapter-size` reports the media's byte size, and `0` — not an error —
	/// for a chapter id that resolves to nothing.
	#[tokio::test]
	async fn chapter_size_is_the_media_file_size_in_bytes() {
		let conn = db().await;
		let user_row = fake_data::User::new("sizer").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (series, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		models::entity::media::ActiveModel {
			size: Set(3_522_110),
			..files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		let series_id = KavitaIds::resolve(&backend.conn, IdKind::Series, &series.id)
			.await
			.unwrap();
		let volumes = list_volumes(&backend, &user, series_id).await.unwrap();
		let chapter_id = volumes[0].chapters[0].id;

		assert_eq!(
			chapter_size_for(&backend, &user, chapter_id).await.unwrap(),
			3_522_110
		);
		assert_eq!(
			chapter_size_for(&backend, &user, 9999).await.unwrap(),
			0,
			"an unresolvable chapter is 0 bytes, not a 404"
		);
	}

	/// A chapter download is the file itself, labelled the way `kavita-ref`
	/// labels it, and the volume route serves the same bytes because a Stump
	/// media item is both the volume and its chapter.
	#[tokio::test]
	async fn chapter_and_volume_download_stream_the_file_with_kavita_headers() {
		let conn = db().await;
		let user_row = fake_data::User::new("reader").insert(&conn).await;
		let user = downloader(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		let (_, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Wonderland",
			&[("alice", "epub", 15)],
		)
		.await;
		let contents = b"epub-bytes-for-alice".repeat(16);
		let path = backend.store_file(&files[0].id, "alice.epub", &contents);
		models::entity::media::ActiveModel {
			path: Set(path),
			..files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		let chapter_id = KavitaIds::resolve(&backend.conn, IdKind::Media, &files[0].id)
			.await
			.unwrap();
		let backend = Arc::new(backend);

		for uri in [
			format!("/api/Download/chapter?chapterId={chapter_id}"),
			format!("/api/Download/volume?volumeId={chapter_id}"),
			format!("/api/download/chapter?chapterId={chapter_id}"),
		] {
			let (status, headers, bytes) =
				download(backend.clone(), &user, &uri, None).await;
			assert_eq!(status, 200, "{uri}");
			assert_eq!(
				header(&headers, header::CONTENT_TYPE),
				"application/epub+zip",
				"{uri}"
			);
			assert_eq!(
				header(&headers, header::CONTENT_DISPOSITION),
				"attachment; filename=\"alice.epub\"; filename*=UTF-8''alice.epub",
				"{uri}"
			);
			assert_eq!(
				header(&headers, header::CONTENT_LENGTH),
				contents.len().to_string(),
				"{uri}"
			);
			assert_eq!(bytes, contents, "{uri}");
		}
	}

	/// The download goes through the same file service as every other Stump
	/// download, so a client resuming a transfer gets a `206` with the range
	/// it asked for — and still gets the Kavita labels.
	#[tokio::test]
	async fn chapter_download_answers_range_requests_with_partial_content() {
		let conn = db().await;
		let user_row = fake_data::User::new("resumer").insert(&conn).await;
		let user = downloader(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (_, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let contents: Vec<u8> = (0..=255u8).collect();
		let path = backend.store_file(&files[0].id, "science_comics_001.cbz", &contents);
		models::entity::media::ActiveModel {
			path: Set(path),
			..files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		let chapter_id = KavitaIds::resolve(&backend.conn, IdKind::Media, &files[0].id)
			.await
			.unwrap();

		let (status, headers, bytes) = download(
			Arc::new(backend),
			&user,
			&format!("/api/Download/chapter?chapterId={chapter_id}"),
			Some("bytes=0-9"),
		)
		.await;

		assert_eq!(status, 206);
		assert_eq!(header(&headers, header::CONTENT_RANGE), "bytes 0-9/256");
		assert_eq!(header(&headers, header::CONTENT_TYPE), COMIC_ZIP);
		assert_eq!(
			header(&headers, header::CONTENT_DISPOSITION),
			"attachment; filename=\"science_comics_001.cbz\"; \
			 filename*=UTF-8''science_comics_001.cbz"
		);
		assert_eq!(bytes, contents[..10]);
	}

	/// A series with more than one file is one zip holding every file, named
	/// after the series. A single-file series — every series of a Book
	/// library, whose `seriesId` is a book-series id — is the file itself.
	#[tokio::test]
	async fn series_download_zips_every_file_and_passes_a_lone_file_through() {
		let conn = db().await;
		let user_row = fake_data::User::new("collector").insert(&conn).await;
		let user = downloader(&user_row);
		let backend = TestBackend::new(conn);
		let comics = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (series, files) = series_with_files(
			&backend.conn,
			&comics.id,
			"Multi",
			&[("multi v01", "cbz", 36), ("multi v02", "cbz", 36)],
		)
		.await;
		let mut contents = Vec::new();
		for (index, file) in files.iter().enumerate() {
			let data = format!("cbz-{index}").repeat(32).into_bytes();
			let path = backend.store_file(
				&file.id,
				&format!("multi v0{}.cbz", index + 1),
				&data,
			);
			models::entity::media::ActiveModel {
				path: Set(path),
				..file.clone().into()
			}
			.update(&backend.conn)
			.await
			.unwrap();
			contents.push(data);
		}
		let series_id = KavitaIds::resolve(&backend.conn, IdKind::Series, &series.id)
			.await
			.unwrap();

		// A Book library: the media item is its own Kavita series, allocated
		// under the book-series kind.
		let books = library_of_type(&backend.conn, StumpLibraryType::Book).await;
		let (_, book_files) = series_with_files(
			&backend.conn,
			&books.id,
			"Wonderland",
			&[("alice", "epub", 15)],
		)
		.await;
		let book_bytes = b"alice-epub".repeat(8);
		let path = backend.store_file(&book_files[0].id, "alice.epub", &book_bytes);
		models::entity::media::ActiveModel {
			path: Set(path),
			..book_files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		let book_series_id =
			KavitaIds::resolve(&backend.conn, IdKind::BookSeries, &book_files[0].id)
				.await
				.unwrap();
		let backend = Arc::new(backend);

		let (status, headers, bytes) = download(
			backend.clone(),
			&user,
			&format!("/api/Download/series?seriesId={series_id}"),
			None,
		)
		.await;
		assert_eq!(status, 200);
		assert_eq!(header(&headers, header::CONTENT_TYPE), OCTET_STREAM);
		assert_eq!(
			header(&headers, header::CONTENT_DISPOSITION),
			"attachment; filename=\"Multi.zip\"; filename*=UTF-8''Multi.zip"
		);
		assert_eq!(
			header(&headers, header::CONTENT_LENGTH),
			bytes.len().to_string()
		);
		let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
		assert_eq!(
			archive.file_names().collect::<HashSet<_>>(),
			HashSet::from(["multi v01.cbz", "multi v02.cbz"]),
			"every file of the series is in the zip, under its own name"
		);
		for (index, name) in ["multi v01.cbz", "multi v02.cbz"].iter().enumerate() {
			let mut entry = archive.by_name(name).expect("entry");
			let mut data = Vec::new();
			std::io::Read::read_to_end(&mut entry, &mut data).unwrap();
			assert_eq!(data, contents[index], "{name} is byte-identical");
		}

		let (status, headers, bytes) = download(
			backend,
			&user,
			&format!("/api/Download/series?seriesId={book_series_id}"),
			None,
		)
		.await;
		assert_eq!(status, 200);
		assert_eq!(
			header(&headers, header::CONTENT_TYPE),
			"application/epub+zip"
		);
		assert_eq!(
			header(&headers, header::CONTENT_DISPOSITION),
			"attachment; filename=\"alice.epub\"; filename*=UTF-8''alice.epub"
		);
		assert_eq!(bytes, book_bytes, "a one-file series is the file itself");
	}

	/// A provider-backed chapter has no file: its pages are packed into a CBZ
	/// named after the chapter, served whole.
	#[tokio::test]
	async fn provider_backed_chapter_downloads_a_generated_cbz() {
		let conn = db().await;
		let user_row = fake_data::User::new("remote").insert(&conn).await;
		let user = downloader(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Manga).await;
		let (_, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Alpha Adventures",
			&[("Vol. 1 Ch. 1", "cbz", 3)],
		)
		.await;
		models::entity::media::ActiveModel {
			path: Set("provider://mock-en/alpha/alpha-ch1".to_owned()),
			source_provider: Set(Some("mock-en".to_owned())),
			remote_chapter_id: Set(Some("alpha-ch1".to_owned())),
			..files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		let chapter_id = KavitaIds::resolve(&backend.conn, IdKind::Media, &files[0].id)
			.await
			.unwrap();

		let (status, headers, bytes) = download(
			Arc::new(backend),
			&user,
			&format!("/api/Download/chapter?chapterId={chapter_id}"),
			None,
		)
		.await;

		assert_eq!(status, 200);
		assert_eq!(header(&headers, header::CONTENT_TYPE), COMIC_ZIP);
		assert_eq!(
			header(&headers, header::CONTENT_DISPOSITION),
			"attachment; filename=\"Vol. 1 Ch. 1.cbz\"; \
			 filename*=UTF-8''Vol.%201%20Ch.%201.cbz"
		);
		assert!(
			!bytes.is_empty(),
			"the generated archive is served, not the missing path"
		);
	}

	/// `DownloadController` sits behind the `Download` role: without it every
	/// route in it — the size probe included — is a 403, as on `kavita-ref`.
	#[tokio::test]
	async fn download_routes_require_the_download_permission() {
		let conn = db().await;
		let user_row = fake_data::User::new("pleb").insert(&conn).await;
		// `fake_data::User` inserts a server owner, who bypasses every
		// permission check; the `Download` role only matters for everyone else.
		let user = AuthUser {
			is_server_owner: false,
			..auth_user(&user_row)
		};
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (series, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let chapter_id = KavitaIds::resolve(&backend.conn, IdKind::Media, &files[0].id)
			.await
			.unwrap();
		let series_id = KavitaIds::resolve(&backend.conn, IdKind::Series, &series.id)
			.await
			.unwrap();
		let backend = Arc::new(backend);

		for uri in [
			format!("/api/Download/chapter?chapterId={chapter_id}"),
			format!("/api/Download/volume?volumeId={chapter_id}"),
			format!("/api/Download/series?seriesId={series_id}"),
			format!("/api/Download/chapter-size?chapterId={chapter_id}"),
		] {
			let (status, ..) = download(backend.clone(), &user, &uri, None).await;
			assert_eq!(status, 403, "{uri}");
		}
	}

	/// An id that names nothing the user can see is a 404, the same answer
	/// every other Kavita route in this crate gives.
	#[tokio::test]
	async fn download_of_an_unknown_id_is_not_found() {
		let conn = db().await;
		let user_row = fake_data::User::new("lost").insert(&conn).await;
		let user = downloader(&user_row);
		let backend = Arc::new(TestBackend::new(conn));

		for uri in [
			"/api/Download/chapter?chapterId=9999",
			"/api/Download/volume?volumeId=9999",
			"/api/Download/series?seriesId=9999",
			"/api/Download/chapter",
		] {
			let (status, ..) = download(backend.clone(), &user, uri, None).await;
			assert_eq!(status, 404, "{uri}");
		}
	}

	/// The label pairs `kavita-ref` emits, including the RFC 5987 form for a
	/// name that is not a bare token.
	#[test]
	fn downloads_are_labelled_by_extension_and_carry_both_file_names() {
		assert_eq!(download_content_type("cbz"), "application/x-cbz");
		assert_eq!(download_content_type(".CBZ"), "application/x-cbz");
		assert_eq!(download_content_type("epub"), "application/epub+zip");
		assert_eq!(download_content_type("pdf"), "application/pdf");
		assert_eq!(download_content_type("zip"), OCTET_STREAM);
		assert_eq!(download_content_type("mobi"), OCTET_STREAM);
		assert_eq!(
			content_disposition("multi v01.cbz"),
			"attachment; filename=\"multi v01.cbz\"; filename*=UTF-8''multi%20v01.cbz"
		);
	}

	/// Two files with the same name in different folders are one series; the
	/// zip must still be decodable.
	#[test]
	fn zip_entry_names_are_unique_within_the_archive() {
		let mut taken = HashSet::new();
		assert_eq!(
			unique_entry_name(&mut taken, "v01.cbz".to_owned()),
			"v01.cbz"
		);
		assert_eq!(
			unique_entry_name(&mut taken, "v01.cbz".to_owned()),
			"v01-2.cbz"
		);
		assert_eq!(
			unique_entry_name(&mut taken, "v01.cbz".to_owned()),
			"v01-3.cbz"
		);
		assert_eq!(unique_entry_name(&mut taken, "readme".to_owned()), "readme");
		assert_eq!(
			unique_entry_name(&mut taken, "readme".to_owned()),
			"readme-2"
		);
	}

	/// A file name never carries a path separator or a quote into a header or
	/// a zip entry.
	#[tokio::test]
	async fn file_names_come_from_the_file_and_stay_single_path_components() {
		let conn = db().await;
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (_, files) = series_with_files(
			&conn,
			&library.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		let mut media = files[0].clone();
		media.path = "/library/Comics/Science Comics/science_comics_001.cbz".to_owned();
		media.name = "science_comics_001".to_owned();
		assert_eq!(download_file_name(&media), "science_comics_001.cbz");

		media.path = "C:\\library\\a\"b.cbz".to_owned();
		assert_eq!(download_file_name(&media), "a_b.cbz");

		media.path = "provider://mock-en/alpha/alpha-ch1".to_owned();
		media.name = "Vol. 1 Ch. 1: Down/Up".to_owned();
		assert_eq!(download_file_name(&media), "Vol. 1 Ch. 1_ Down_Up.cbz");
	}
}
