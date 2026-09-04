use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use crate::{
	KomgaAuthor, KomgaBook, KomgaBookId, KomgaBookMetadata, KomgaLibrary, KomgaLibraryId,
	KomgaMediaStatus, KomgaReadingDirection, KomgaSeries, KomgaSeriesBookMetadata,
	KomgaSeriesId, KomgaSeriesMetadata, KomgaSeriesStatus, KomgaWebLink, Media,
	MediaProfile, ReadProgress, ScanInterval, SeriesCover,
};
use chrono::{DateTime, NaiveDate, Utc};
use models::{
	entity::{
		library, library_config, media, media_metadata, media_tag, reading_session,
		series, series_tag, tag, user::AuthUser,
	},
	shared::enums::FileStatus,
};
use sea_orm::{
	prelude::DateTimeWithTimeZone, ColumnTrait, DatabaseConnection, EntityTrait,
	QueryFilter, QueryOrder,
};

use crate::errors::APIResult;

/// Convert a user-visible Stump library into the Komga library shape.
///
/// Komga exposes more scanner switches than Stump persists.  The fields with no Stump
/// equivalent intentionally use Komga's neutral defaults; the comments below make those
/// losses explicit rather than pretending that Stump supports the option.
pub(crate) fn map_library(
	library: library::Model,
	config: Option<library_config::Model>,
	_user: &AuthUser,
) -> KomgaLibrary {
	let process_metadata = config
		.as_ref()
		.is_some_and(|config| config.process_metadata);
	let hash_files = config
		.as_ref()
		.is_some_and(|config| config.generate_file_hashes);
	let hash_koreader = config
		.as_ref()
		.is_some_and(|config| config.generate_koreader_hashes);
	let scan_directory_exclusions = config
		.as_ref()
		.and_then(|config| config.ignore_rules.as_ref())
		.map(|rules| rules.as_vec())
		.unwrap_or_default();

	KomgaLibrary {
		id: KomgaLibraryId::new(library.id),
		name: library.name,
		// `root` is a required Komga library property and is the persisted Stump root.  It is
		// metadata, not a book/series URL; filesystem browsing remains owner-only in its route.
		root: library.path,
		import_comic_info_book: process_metadata,
		import_comic_info_series: process_metadata,
		// Stump has no collection-level ComicInfo import toggle.
		import_comic_info_collection: false,
		import_comic_info_read_list: false,
		import_comic_info_series_append_volume: false,
		import_epub_book: process_metadata,
		import_epub_series: process_metadata,
		import_mylar_series: false,
		import_local_artwork: false,
		import_barcode_isbn: false,
		scan_force_modified_time: false,
		// Stump stores a watcher boolean, not a periodic Komga interval.  Do not invent a
		// frequency from that boolean.
		scan_interval: ScanInterval::Disabled,
		scan_on_startup: false,
		// Stump's scanner recognizes all three supported book families directly.
		scan_cbx: true,
		scan_pdf: true,
		scan_epub: true,
		scan_directory_exclusions,
		repair_extensions: false,
		// Stump can convert RAR archives to ZIP, but does not implement Komga's CBZ
		// conversion contract.
		convert_to_cbz: false,
		empty_trash_after_scan: false,
		// Stump does not persist a cover-selection policy.
		series_cover: SeriesCover::First,
		hash_files,
		hash_pages: false,
		hash_koreader,
		analyze_dimensions: false,
		oneshots_directory: None,
		unavailable: !matches!(library.status, FileStatus::Ready),
	}
}

/// Convert a batch of user-visible media rows.  Related series, sessions, and tags are loaded
/// once per batch, so this function does not perform a query for each book.
pub(crate) async fn map_books(
	conn: &DatabaseConnection,
	user: &AuthUser,
	books: Vec<media::ModelWithMetadata>,
) -> APIResult<Vec<KomgaBook>> {
	if books.is_empty() {
		return Ok(Vec::new());
	}

	let series_ids =
		unique_ids(books.iter().filter_map(|book| book.media.series_id.clone()));
	let series_rows = if series_ids.is_empty() {
		Vec::new()
	} else {
		series::ModelWithMetadata::find_for_user(user)
			.filter(series::Column::Id.is_in(series_ids))
			.filter(series::Column::DeletedAt.is_null())
			.into_model::<series::ModelWithMetadata>()
			.all(conn)
			.await?
	};
	let series_by_id: HashMap<String, series::ModelWithMetadata> = series_rows
		.into_iter()
		.map(|row| (row.series.id.clone(), row))
		.collect();

	let media_ids = unique_ids(books.iter().map(|book| book.media.id.clone()));
	let sessions = load_latest_sessions(conn, user, &media_ids).await?;
	let tags = load_media_tags(conn, &media_ids).await?;

	Ok(books
		.into_iter()
		.map(|book| {
			let media_id = book.media.id.clone();
			let related_series = book
				.media
				.series_id
				.as_ref()
				.and_then(|id| series_by_id.get(id));
			map_book(
				book,
				related_series,
				sessions.get(&media_id),
				tags.get(&media_id).map(Vec::as_slice).unwrap_or(&[]),
			)
		})
		.collect())
}

/// Convert a batch of user-visible series rows.  Books, sessions, and both tag junctions are
/// fetched in batches; series-level counts are then computed in memory from those rows.
pub(crate) async fn map_series(
	conn: &DatabaseConnection,
	user: &AuthUser,
	series_rows: Vec<series::ModelWithMetadata>,
) -> APIResult<Vec<KomgaSeries>> {
	if series_rows.is_empty() {
		return Ok(Vec::new());
	}

	let series_ids = unique_ids(series_rows.iter().map(|row| row.series.id.clone()));
	let books = if series_ids.is_empty() {
		Vec::new()
	} else {
		media::ModelWithMetadata::find_for_user(user)
			.filter(media::Column::SeriesId.is_in(series_ids.clone()))
			.filter(media::Column::DeletedAt.is_null())
			.into_model::<media::ModelWithMetadata>()
			.all(conn)
			.await?
	};
	let media_ids = unique_ids(books.iter().map(|book| book.media.id.clone()));
	let sessions = load_latest_sessions(conn, user, &media_ids).await?;
	let media_tags = load_media_tags(conn, &media_ids).await?;
	let series_tags = load_series_tags(conn, &series_ids).await?;

	let mut books_by_series: HashMap<String, Vec<media::ModelWithMetadata>> =
		HashMap::new();
	for book in books {
		if let Some(series_id) = book.media.series_id.clone() {
			books_by_series.entry(series_id).or_default().push(book);
		}
	}

	Ok(series_rows
		.into_iter()
		.map(|row| {
			let id = row.series.id.clone();
			let books = books_by_series.remove(&id).unwrap_or_default();
			map_series_row(
				row,
				&books,
				&sessions,
				series_tags.get(&id).map(Vec::as_slice).unwrap_or(&[]),
				&media_tags,
			)
		})
		.collect())
}

fn map_book(
	book: media::ModelWithMetadata,
	related_series: Option<&series::ModelWithMetadata>,
	session: Option<&reading_session::ModelWithDevice>,
	tags: &[String],
) -> KomgaBook {
	let media = book.media;
	let metadata = book.metadata.as_ref();
	let series_id = media.series_id.clone().unwrap_or_default();
	let series_title = related_series
		.and_then(|row| row.metadata.as_ref())
		.and_then(|metadata| non_empty(metadata.title.clone()))
		.or_else(|| non_empty(metadata.and_then(|metadata| metadata.series.clone())))
		.unwrap_or_default();
	let library_id = related_series
		.and_then(|row| row.series.library_id.clone())
		.unwrap_or_default();
	let title = non_empty(metadata.and_then(|metadata| metadata.title.clone()))
		.unwrap_or_else(|| media.name.clone());
	let created = to_utc(&media.created_at);
	let last_modified = updated_or_created(media.updated_at.as_ref(), created);
	let file_last_modified = media
		.modified_at
		.as_ref()
		.map(to_utc)
		.unwrap_or(last_modified);
	let number_text = metadata
		.and_then(|metadata| metadata.number.as_ref())
		.map(ToString::to_string)
		.unwrap_or_default();
	let number_sort = number_sort_from_text(&number_text);
	let pages_count = media_pages(&media, metadata);

	KomgaBook {
		id: KomgaBookId::new(media.id.clone()),
		series_id: KomgaSeriesId::new(series_id),
		series_title,
		library_id: KomgaLibraryId::new(library_id),
		name: title.clone(),
		url: virtual_book_url(
			related_series.map(|row| row.series.path.as_str()),
			&media.path,
		),
		number: number_sort as i32,
		created,
		last_modified,
		file_last_modified,
		size_bytes: media.size,
		size: format_file_size(media.size),
		media: Media {
			status: map_media_status(media.status),
			media_type: media_type_for_extension(&media.extension),
			pages_count,
			// Stump persists no Komga media-analysis comment.
			comment: String::new(),
			// Stump has no persisted EPUB conversion/KEPUB analysis flags.
			epub_divina_compatible: false,
			epub_is_kepub: false,
			media_profile: media_profile_for_extension(&media.extension),
		},
		metadata: map_book_metadata(
			metadata,
			tags,
			&title,
			created,
			last_modified,
			number_text,
			number_sort,
		),
		read_progress: session.map(map_read_progress),
		deleted: media.deleted_at.is_some(),
		file_hash: media.hash.unwrap_or_default(),
		// Stump has no book-level one-shot flag.  A series-level ComicInfo booktype is
		// intentionally not copied here because it describes the containing series.
		oneshot: false,
	}
}

fn map_book_metadata(
	metadata: Option<&media_metadata::Model>,
	tags: &[String],
	fallback_title: &str,
	created: DateTime<Utc>,
	last_modified: DateTime<Utc>,
	number_text: String,
	number_sort: f32,
) -> KomgaBookMetadata {
	let locked_fields = metadata.and_then(|metadata| metadata.locked_fields.as_ref());
	KomgaBookMetadata {
		title: non_empty(metadata.and_then(|metadata| metadata.title.clone()))
			.unwrap_or_else(|| fallback_title.to_owned()),
		summary: metadata
			.and_then(|metadata| metadata.summary.clone())
			.unwrap_or_default(),
		number: number_text,
		number_sort,
		release_date: metadata.and_then(|metadata| {
			date_from_parts(metadata.year, metadata.month, metadata.day)
		}),
		authors: metadata
			.map(authors_from_media_metadata)
			.unwrap_or_default(),
		tags: tags.to_vec(),
		isbn: metadata
			.and_then(|metadata| metadata.identifier_isbn.clone())
			.unwrap_or_default(),
		links: parse_links(metadata.and_then(|metadata| metadata.links.as_deref())),
		title_lock: is_locked(locked_fields, &["TITLE"]),
		summary_lock: is_locked(locked_fields, &["SUMMARY"]),
		number_lock: is_locked(locked_fields, &["NUMBER"]),
		// numberSort is derived from Stump's NUMBER field and has no separate lock.
		number_sort_lock: is_locked(locked_fields, &["NUMBER"]),
		release_date_lock: is_locked(locked_fields, &["RELEASE_DATE", "YEAR"]),
		authors_lock: is_locked(
			locked_fields,
			&[
				"WRITERS",
				"PENCILLERS",
				"INKERS",
				"COLORISTS",
				"LETTERERS",
				"COVER_ARTISTS",
				"EDITORS",
			],
		),
		tags_lock: is_locked(locked_fields, &["TAGS"]),
		isbn_lock: is_locked(locked_fields, &["ISBN"]),
		links_lock: is_locked(locked_fields, &["LINKS"]),
		// Metadata rows do not carry timestamps; the owning media timestamps are the
		// closest persisted change markers and keep Komelia's DTO complete.
		created,
		last_modified,
	}
}

fn map_series_row(
	row: series::ModelWithMetadata,
	books: &[media::ModelWithMetadata],
	sessions: &HashMap<String, reading_session::ModelWithDevice>,
	series_tags: &[String],
	media_tags: &HashMap<String, Vec<String>>,
) -> KomgaSeries {
	let series = row.series;
	let metadata = row.metadata.as_ref();
	let title = non_empty(metadata.and_then(|metadata| metadata.title.clone()))
		.unwrap_or_else(|| series.name.clone());
	let title_sort = series_title_sort(
		metadata.and_then(|metadata| metadata.title_sort.clone()),
		&title,
	);
	let summary = metadata
		.and_then(|metadata| metadata.summary.clone())
		.or_else(|| series.description.clone())
		.unwrap_or_default();
	let created = to_utc(&series.created_at);
	let last_modified = updated_or_created(series.updated_at.as_ref(), created);
	let locked_fields = metadata.and_then(|metadata| metadata.locked_fields.as_ref());
	let (books_read_count, books_in_progress_count) =
		books
			.iter()
			.fold((0_i32, 0_i32), |(read, in_progress), book| {
				match sessions
					.get(&book.media.id)
					.map(|session| session.model.status)
				{
					Some(models::shared::enums::ReadingStatus::Finished) => {
						(read + 1, in_progress)
					},
					Some(models::shared::enums::ReadingStatus::Reading) => {
						(read, in_progress + 1)
					},
					// Abandoned and not-started books are unread from Komga's three-state
					// perspective.
					_ => (read, in_progress),
				}
			});
	let books_count = len_i32(books.len());
	let books_unread_count = books_count
		.saturating_sub(books_read_count)
		.saturating_sub(books_in_progress_count);
	let books_metadata =
		map_series_books_metadata(books, media_tags, created, last_modified);
	let series_id = series.id.clone();
	let library_id = series.library_id.clone().unwrap_or_default();

	KomgaSeries {
		id: KomgaSeriesId::new(series_id.clone()),
		library_id: KomgaLibraryId::new(library_id),
		name: title.clone(),
		url: virtual_series_url(&series.path),
		books_count,
		books_read_count,
		books_unread_count,
		books_in_progress_count,
		metadata: KomgaSeriesMetadata {
			status: map_series_status(
				metadata.and_then(|metadata| metadata.status.as_deref()),
			),
			status_lock: is_locked(locked_fields, &["STATUS"]),
			title: title.clone(),
			alternate_titles: parse_alternate_titles(
				metadata.and_then(|metadata| metadata.alternate_titles.as_deref()),
			),
			alternate_titles_lock: metadata
				.is_some_and(|metadata| metadata.alternate_titles_lock),
			title_lock: is_locked(locked_fields, &["TITLE"]),
			title_sort,
			title_sort_lock: metadata.is_some_and(|metadata| metadata.title_sort_lock),
			summary: summary.clone(),
			summary_lock: is_locked(locked_fields, &["SUMMARY"]),
			reading_direction: map_reading_direction(
				metadata.and_then(|metadata| metadata.reading_direction.as_deref()),
			),
			reading_direction_lock: metadata
				.is_some_and(|metadata| metadata.reading_direction_lock),
			publisher: metadata
				.and_then(|metadata| metadata.publisher.clone())
				.unwrap_or_default(),
			publisher_lock: is_locked(locked_fields, &["PUBLISHER"]),
			age_rating: metadata.and_then(|metadata| metadata.age_rating),
			age_rating_lock: is_locked(locked_fields, &["AGE_RATING"]),
			language: metadata.and_then(|metadata| metadata.language.clone()),
			language_lock: metadata.is_some_and(|metadata| metadata.language_lock),
			genres: split_csv(metadata.and_then(|metadata| metadata.genres.as_deref())),
			genres_lock: is_locked(locked_fields, &["GENRES"]),
			tags: series_tags.to_vec(),
			tags_lock: is_locked(locked_fields, &["TAGS"]),
			total_book_count: metadata.and_then(|metadata| metadata.total_issues),
			total_book_count_lock: is_locked(locked_fields, &["VOLUME_COUNT"]),
			sharing_labels: Vec::new(),
			sharing_labels_lock: false,
			links: parse_links(metadata.and_then(|metadata| metadata.links.as_deref())),
			links_lock: is_locked(locked_fields, &["LINKS"]),
		},
		deleted: series.deleted_at.is_some(),
		oneshot: metadata
			.is_some_and(|metadata| is_one_shot(metadata.booktype.as_deref())),
		books_metadata,
		created,
		last_modified,
		// Series rows have no directory mtime.  The persisted row update time is the
		// closest equivalent and avoids exposing the filesystem path as a URL.
		file_last_modified: last_modified,
	}
}

fn map_series_books_metadata(
	books: &[media::ModelWithMetadata],
	media_tags: &HashMap<String, Vec<String>>,
	series_created: DateTime<Utc>,
	series_last_modified: DateTime<Utc>,
) -> KomgaSeriesBookMetadata {
	let mut authors = BTreeSet::<(String, String)>::new();
	let mut tags = BTreeSet::<String>::new();
	let mut release_date = None;
	let mut summary = String::new();
	let mut summary_number = String::new();
	let mut created = None;
	let mut last_modified = None;

	let mut ordered_books: Vec<&media::ModelWithMetadata> = books.iter().collect();
	ordered_books.sort_by(|left, right| {
		let left_number = book_number(left).unwrap_or(f32::INFINITY);
		let right_number = book_number(right).unwrap_or(f32::INFINITY);
		left_number
			.partial_cmp(&right_number)
			.unwrap_or(Ordering::Equal)
			.then_with(|| left.media.name.cmp(&right.media.name))
			.then_with(|| left.media.id.cmp(&right.media.id))
	});

	for book in ordered_books {
		if let Some(metadata) = book.metadata.as_ref() {
			for author in authors_from_media_metadata(metadata) {
				authors.insert((author.name, author.role));
			}
			if summary.is_empty() {
				summary = metadata.summary.clone().unwrap_or_default();
			}
			if summary_number.is_empty() {
				summary_number = metadata
					.number
					.as_ref()
					.map(ToString::to_string)
					.unwrap_or_default();
			}
			if release_date.is_none() {
				release_date =
					date_from_parts(metadata.year, metadata.month, metadata.day);
			}
		}
		if let Some(book_tags) = media_tags.get(&book.media.id) {
			tags.extend(book_tags.iter().cloned());
		}
		let book_created = to_utc(&book.media.created_at);
		created = Some(created.map_or(book_created, |current: DateTime<Utc>| {
			current.min(book_created)
		}));
		let book_last_modified =
			updated_or_created(book.media.updated_at.as_ref(), book_created);
		last_modified = Some(
			last_modified.map_or(book_last_modified, |current: DateTime<Utc>| {
				current.max(book_last_modified)
			}),
		);
	}

	KomgaSeriesBookMetadata {
		authors: authors
			.into_iter()
			.map(|(name, role)| KomgaAuthor { name, role })
			.collect(),
		tags: tags.into_iter().collect(),
		release_date,
		summary,
		summary_number,
		created: created.unwrap_or(series_created),
		last_modified: last_modified.unwrap_or(series_last_modified),
	}
}

async fn load_latest_sessions(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_ids: &[String],
) -> APIResult<HashMap<String, reading_session::ModelWithDevice>> {
	if media_ids.is_empty() {
		return Ok(HashMap::new());
	}
	let rows = reading_session::ModelWithDevice::find()
		.filter(reading_session::Column::UserId.eq(user.id.clone()))
		.filter(reading_session::Column::MediaId.is_in(media_ids.to_vec()))
		.order_by_desc(reading_session::Column::UpdatedAt)
		.order_by_desc(reading_session::Column::CreatedAt)
		.order_by_desc(reading_session::Column::Id)
		.into_model::<reading_session::ModelWithDevice>()
		.all(conn)
		.await?;

	let mut latest: HashMap<String, reading_session::ModelWithDevice> =
		HashMap::with_capacity(rows.len());
	for row in rows {
		let media_id = row.model.media_id.clone();
		let replace = latest.get(&media_id).is_none_or(|current| {
			session_order_key(&row.model) > session_order_key(&current.model)
		});
		if replace {
			latest.insert(media_id, row);
		}
	}
	Ok(latest)
}

async fn load_media_tags(
	conn: &DatabaseConnection,
	media_ids: &[String],
) -> APIResult<HashMap<String, Vec<String>>> {
	if media_ids.is_empty() {
		return Ok(HashMap::new());
	}
	let rows = media_tag::Entity::find()
		.filter(media_tag::Column::MediaId.is_in(media_ids.to_vec()))
		.find_also_related(tag::Entity)
		.all(conn)
		.await?;
	let mut result: HashMap<String, Vec<String>> = HashMap::new();
	for (link, tag) in rows {
		if let Some(tag) = tag {
			result.entry(link.media_id).or_default().push(tag.name);
		}
	}
	finish_tag_map(result)
}

async fn load_series_tags(
	conn: &DatabaseConnection,
	series_ids: &[String],
) -> APIResult<HashMap<String, Vec<String>>> {
	if series_ids.is_empty() {
		return Ok(HashMap::new());
	}
	let rows = series_tag::Entity::find()
		.filter(series_tag::Column::SeriesId.is_in(series_ids.to_vec()))
		.find_also_related(tag::Entity)
		.all(conn)
		.await?;
	let mut result: HashMap<String, Vec<String>> = HashMap::new();
	for (link, tag) in rows {
		if let Some(tag) = tag {
			result.entry(link.series_id).or_default().push(tag.name);
		}
	}
	finish_tag_map(result)
}

fn finish_tag_map(
	mut tags: HashMap<String, Vec<String>>,
) -> APIResult<HashMap<String, Vec<String>>> {
	for values in tags.values_mut() {
		values.sort();
		values.dedup();
	}
	Ok(tags)
}

fn map_read_progress(session: &reading_session::ModelWithDevice) -> ReadProgress {
	let model = &session.model;
	let device_id = model
		.device_ids
		.as_ref()
		.and_then(|ids| ids.0.first())
		.cloned()
		.unwrap_or_default();
	ReadProgress {
		page: model.end_page.or(model.start_page).unwrap_or_default(),
		completed: model.is_complete(),
		read_date: model
			.updated_at
			.as_ref()
			.map(to_utc)
			.unwrap_or_else(|| to_utc(&model.created_at)),
		device_id,
		// A device name is optional in Stump.  Empty is intentional when the linked
		// reading-device row does not exist; no synthetic device name is invented.
		device_name: session
			.device
			.as_ref()
			.map(|device| device.name.clone())
			.unwrap_or_default(),
		created: to_utc(&model.created_at),
		last_modified: model
			.updated_at
			.as_ref()
			.map(to_utc)
			.unwrap_or_else(|| to_utc(&model.created_at)),
	}
}

fn authors_from_media_metadata(metadata: &media_metadata::Model) -> Vec<KomgaAuthor> {
	let mut authors = Vec::new();
	let mut seen = BTreeSet::<(String, String)>::new();
	for (raw, role) in [
		(metadata.writers.as_deref(), "writer"),
		(metadata.pencillers.as_deref(), "penciller"),
		(metadata.inkers.as_deref(), "inker"),
		(metadata.colorists.as_deref(), "colorist"),
		(metadata.letterers.as_deref(), "letterer"),
		(metadata.cover_artists.as_deref(), "cover"),
		(metadata.editors.as_deref(), "editor"),
	] {
		for name in split_csv(raw) {
			if seen.insert((name.clone(), role.to_owned())) {
				authors.push(KomgaAuthor {
					name,
					role: role.to_owned(),
				});
			}
		}
	}
	authors
}

fn parse_links(raw: Option<&str>) -> Vec<KomgaWebLink> {
	split_csv(raw)
		.into_iter()
		.map(|url| KomgaWebLink {
			// Stump stores only comma-separated URLs, so retaining the URL as the
			// label is the least lossy representation of the missing label field.
			label: url.clone(),
			url,
		})
		.collect()
}

fn split_csv(raw: Option<&str>) -> Vec<String> {
	let mut values = Vec::new();
	let mut seen = BTreeSet::new();
	if let Some(raw) = raw {
		for value in raw
			.split(',')
			.map(str::trim)
			.filter(|value| !value.is_empty())
		{
			let value = value.to_owned();
			if seen.insert(value.clone()) {
				values.push(value);
			}
		}
	}
	values
}

fn date_from_parts(
	year: Option<i32>,
	month: Option<i32>,
	day: Option<i32>,
) -> Option<NaiveDate> {
	let year = year?;
	let month = month?;
	NaiveDate::from_ymd_opt(year, month as u32, day.unwrap_or(1) as u32)
}

fn media_type_for_extension(extension: &str) -> Option<String> {
	let extension = extension.to_ascii_lowercase();
	match extension.as_str() {
		"xhtml" => Some("application/xhtml+xml".to_string()),
		"xml" | "opf" | "ncx" => Some("application/xml".to_string()),
		"html" => Some("text/html".to_string()),
		"pdf" => Some("application/pdf".to_string()),
		"epub" => Some("application/epub+zip".to_string()),
		"zip" => Some("application/zip".to_string()),
		"cbz" => Some("application/vnd.comicbook+zip".to_string()),
		"rar" => Some("application/vnd.rar".to_string()),
		"cbr" => Some("application/vnd.comicbook-rar".to_string()),
		"avif" => Some("image/avif".to_string()),
		"heif" => Some("image/heif".to_string()),
		"png" => Some("image/png".to_string()),
		"jpg" | "jpeg" => Some("image/jpeg".to_string()),
		"jxl" => Some("image/jxl".to_string()),
		"webp" => Some("image/webp".to_string()),
		"gif" => Some("image/gif".to_string()),
		"txt" => Some("text/plain".to_string()),
		_ => None,
	}
}

/// Extensions that map to each Komga media profile; the inverse of
/// `media_profile_for_extension` and shared with catalog filtering.
pub(crate) fn extensions_for_media_profile(
	profile: MediaProfile,
) -> &'static [&'static str] {
	match profile {
		MediaProfile::Epub => &["epub"],
		MediaProfile::Pdf => &["pdf"],
		MediaProfile::Divina => &["cbz", "cbr", "zip", "rar"],
	}
}

fn media_profile_for_extension(extension: &str) -> Option<MediaProfile> {
	let extension = extension.trim().to_ascii_lowercase();
	[MediaProfile::Epub, MediaProfile::Pdf, MediaProfile::Divina]
		.into_iter()
		.find(|profile| {
			extensions_for_media_profile(*profile).contains(&extension.as_str())
		})
}

fn map_reading_direction(value: Option<&str>) -> Option<KomgaReadingDirection> {
	match value.map(str::trim) {
		Some("LEFT_TO_RIGHT") => Some(KomgaReadingDirection::LeftToRight),
		Some("RIGHT_TO_LEFT") => Some(KomgaReadingDirection::RightToLeft),
		Some("VERTICAL") => Some(KomgaReadingDirection::Vertical),
		Some("WEBTOON") => Some(KomgaReadingDirection::Webtoon),
		_ => None,
	}
}

fn parse_alternate_titles(value: Option<&str>) -> Vec<crate::KomgaAlternativeTitle> {
	value
		.and_then(|value| serde_json::from_str(value).ok())
		.unwrap_or_default()
}

fn map_media_status(status: FileStatus) -> KomgaMediaStatus {
	match status {
		FileStatus::Ready => KomgaMediaStatus::Ready,
		FileStatus::Unknown => KomgaMediaStatus::Unknown,
		FileStatus::Unsupported => KomgaMediaStatus::Unsupported,
		FileStatus::Error => KomgaMediaStatus::Error,
		// Komga has no MISSING state.  UNKNOWN is the non-error state used when Stump
		// cannot assert that a media file is currently available.
		FileStatus::Missing => KomgaMediaStatus::Unknown,
	}
}

/// Komga's `url` fields are filesystem paths; Komelia takes the last segment of a
/// book's `url` as its local download filename (so the extension must survive).
/// Stump never exposes real paths to users, so emit a virtual path made only of
/// the series directory name and the file name.
fn virtual_book_url(series_path: Option<&str>, media_path: &str) -> String {
	let file = last_path_component(media_path);
	match series_path
		.map(last_path_component)
		.filter(|dir| !dir.is_empty())
	{
		Some(dir) => format!("/{dir}/{file}"),
		None => format!("/{file}"),
	}
}

fn virtual_series_url(series_path: &str) -> String {
	format!("/{}", last_path_component(series_path))
}

fn last_path_component(path: &str) -> &str {
	path.trim_end_matches(['/', '\\'])
		.rsplit(['/', '\\'])
		.next()
		.unwrap_or_default()
}

fn map_series_status(status: Option<&str>) -> KomgaSeriesStatus {
	match status.map(normalize_token).as_deref() {
		Some("ENDED") | Some("COMPLETED") => KomgaSeriesStatus::Ended,
		Some("ABANDONED") | Some("CANCELLED") | Some("CANCELED") => {
			KomgaSeriesStatus::Abandoned
		},
		Some("HIATUS") | Some("ON_HOLD") | Some("ONHOLD") => KomgaSeriesStatus::Hiatus,
		// Stump metadata uses Continuing; Komga calls the same state Ongoing.  A
		// missing/unknown status is exposed as the neutral ongoing state.
		_ => KomgaSeriesStatus::Ongoing,
	}
}

fn normalize_token(value: &str) -> String {
	value
		.trim()
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() {
				character.to_ascii_uppercase()
			} else {
				'_'
			}
		})
		.collect()
}

fn is_one_shot(booktype: Option<&str>) -> bool {
	matches!(
		booktype.map(normalize_token).as_deref(),
		Some("ONESHOT") | Some("ONE_SHOT") | Some("ONE_SHOT_BOOK")
	)
}

fn media_pages(media: &media::Model, metadata: Option<&media_metadata::Model>) -> i32 {
	if media.pages >= 0 {
		media.pages
	} else {
		metadata
			.and_then(|metadata| metadata.page_count)
			.unwrap_or_default()
			.max(0)
	}
}

fn book_number(book: &media::ModelWithMetadata) -> Option<f32> {
	book.metadata
		.as_ref()
		.and_then(|metadata| metadata.number.as_ref())
		.and_then(|number| number.to_string().parse::<f32>().ok())
		.filter(|number| number.is_finite())
}

fn number_sort_from_text(value: &str) -> f32 {
	value
		.parse::<f32>()
		.ok()
		.filter(|number| number.is_finite())
		.unwrap_or_default()
}

fn format_file_size(bytes: i64) -> String {
	if bytes <= 0 {
		return "0 B".to_owned();
	}
	const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
	let mut value = bytes as f64;
	let mut unit = 0;
	while value >= 1024.0 && unit < UNITS.len() - 1 {
		value /= 1024.0;
		unit += 1;
	}
	if unit == 0 {
		format!("{} {}", bytes, UNITS[unit])
	} else if value >= 10.0 {
		format!("{value:.0} {}", UNITS[unit])
	} else {
		format!("{value:.2} {}", UNITS[unit])
	}
}

fn to_utc(value: &DateTimeWithTimeZone) -> DateTime<Utc> {
	value.with_timezone(&Utc)
}

fn updated_or_created(
	updated: Option<&DateTimeWithTimeZone>,
	created: DateTime<Utc>,
) -> DateTime<Utc> {
	updated.map(to_utc).unwrap_or(created)
}

fn session_order_key(
	session: &reading_session::Model,
) -> (DateTime<Utc>, DateTime<Utc>, i32) {
	(
		session
			.updated_at
			.as_ref()
			.map(to_utc)
			.unwrap_or_else(|| to_utc(&session.created_at)),
		to_utc(&session.created_at),
		session.id,
	)
}

fn unique_ids<I>(ids: I) -> Vec<String>
where
	I: IntoIterator<Item = String>,
{
	let mut values = Vec::new();
	let mut seen = BTreeSet::new();
	for id in ids {
		if !id.is_empty() && seen.insert(id.clone()) {
			values.push(id);
		}
	}
	values
}

fn len_i32(value: usize) -> i32 {
	value.min(i32::MAX as usize) as i32
}

fn non_empty(value: Option<String>) -> Option<String> {
	value.filter(|value| !value.trim().is_empty())
}

fn series_title_sort(title_sort: Option<String>, title: &str) -> String {
	title_sort.unwrap_or_else(|| title.to_owned())
}

fn is_locked(fields: Option<&serde_json::Value>, names: &[&str]) -> bool {
	fields
		.and_then(serde_json::Value::as_array)
		.is_some_and(|fields| {
			fields.iter().any(|field| {
				field.as_str().map(normalize_token).is_some_and(|field| {
					names.iter().any(|name| normalize_token(name) == field)
				})
			})
		})
}
#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn komga_urls_keep_only_last_path_components() {
		let root = "/srv/private/library";
		let series = format!("{root}/Synthetic Series");
		let book = format!("{series}/00 synthetic prelude.cbz");

		let url = virtual_book_url(Some(&series), &book);
		assert_eq!(url, "/Synthetic Series/00 synthetic prelude.cbz");
		assert!(!url.contains(root));
		assert_eq!(virtual_series_url(&series), "/Synthetic Series");
		assert_eq!(virtual_book_url(None, &book), "/00 synthetic prelude.cbz");
		assert_eq!(
			virtual_book_url(Some("C:\\lib\\Series\\"), "C:\\lib\\Series\\b.epub"),
			"/Series/b.epub"
		);
	}

	fn metadata() -> media_metadata::Model {
		media_metadata::Model {
			id: 1,
			media_id: Some("book".to_owned()),
			..Default::default()
		}
	}

	#[test]
	fn extension_mapping_is_case_insensitive_and_preserves_mime_specificity() {
		assert_eq!(
			media_profile_for_extension("CBZ"),
			Some(MediaProfile::Divina)
		);
		assert_eq!(media_profile_for_extension("PDF"), Some(MediaProfile::Pdf));
		assert_eq!(
			media_profile_for_extension("ePuB"),
			Some(MediaProfile::Epub)
		);
		assert_eq!(media_profile_for_extension("txt"), None);
		assert_eq!(
			media_type_for_extension("CBZ"),
			Some("application/vnd.comicbook+zip".to_owned())
		);
		assert_eq!(media_type_for_extension("unknown"), None);
	}

	#[test]
	fn file_status_mapping_covers_every_stump_state() {
		assert_eq!(map_media_status(FileStatus::Ready), KomgaMediaStatus::Ready);
		assert_eq!(
			map_media_status(FileStatus::Unknown),
			KomgaMediaStatus::Unknown
		);
		assert_eq!(
			map_media_status(FileStatus::Unsupported),
			KomgaMediaStatus::Unsupported
		);
		assert_eq!(map_media_status(FileStatus::Error), KomgaMediaStatus::Error);
		assert_eq!(
			map_media_status(FileStatus::Missing),
			KomgaMediaStatus::Unknown
		);
	}

	#[test]
	fn date_mapping_requires_year_and_month_and_defaults_day() {
		assert_eq!(
			date_from_parts(Some(2024), Some(2), None),
			NaiveDate::from_ymd_opt(2024, 2, 1)
		);
		assert_eq!(date_from_parts(Some(2024), None, Some(2)), None);
		assert_eq!(date_from_parts(Some(2024), Some(2), Some(30)), None);
	}

	#[test]
	fn author_mapping_parses_all_persisted_credit_csv_fields() {
		let mut model = metadata();
		model.writers = Some("Writer A, Writer B, Writer A".to_owned());
		model.pencillers = Some("Artist".to_owned());
		model.editors = Some("Editor".to_owned());
		let authors = authors_from_media_metadata(&model);
		assert_eq!(
			authors,
			vec![
				KomgaAuthor {
					name: "Writer A".to_owned(),
					role: "writer".to_owned(),
				},
				KomgaAuthor {
					name: "Writer B".to_owned(),
					role: "writer".to_owned(),
				},
				KomgaAuthor {
					name: "Artist".to_owned(),
					role: "penciller".to_owned(),
				},
				KomgaAuthor {
					name: "Editor".to_owned(),
					role: "editor".to_owned(),
				},
			]
		);
	}

	#[test]
	fn lock_fields_match_persisted_metadata_field_names() {
		// `locked_fields` stores SCREAMING_SNAKE_CASE `MetadataField` values.
		let value = serde_json::json!(["TITLE", "ISBN"]);
		assert!(is_locked(Some(&value), &["TITLE"]));
		assert!(is_locked(Some(&value), &["ISBN"]));
		assert!(!is_locked(Some(&value), &["SUMMARY"]));
	}

	#[test]
	fn one_shot_status_and_size_helpers_are_deterministic() {
		assert!(is_one_shot(Some("OneShot")));
		assert!(is_one_shot(Some("one-shot")));
		assert!(!is_one_shot(Some("TPB")));
		assert_eq!(format_file_size(0), "0 B");
		assert_eq!(format_file_size(1024), "1.00 KB");
	}

	#[test]
	fn number_sort_rejects_non_finite_values() {
		assert_eq!(number_sort_from_text("2.5"), 2.5);
		assert_eq!(number_sort_from_text("NaN"), 0.0);
		assert_eq!(number_sort_from_text("not-a-number"), 0.0);
	}

	#[test]
	fn title_sort_falls_back_to_series_title_when_unset() {
		assert_eq!(series_title_sort(None, "Series title"), "Series title");

		assert_eq!(
			series_title_sort(Some("Sort title".to_owned()), "Series title"),
			"Sort title"
		);
	}
}
