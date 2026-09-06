//! Stump model → Kavita DTO mapping.
//!
//! Model: one Stump series is one Kavita series; every Stump media item is a
//! Kavita volume holding exactly one chapter, the shape Kavita itself builds
//! for a single-file volume (`chapter.number == "-100000"`,
//! `Parser.DefaultChapterNumber`). Volume and chapter share the media's
//! Kavita id. Numbers come from the media metadata volume, then an integral
//! metadata number, then the media's ordinal in the series.
//!
//! Book and LightNovel libraries are different: Kavita makes every file its
//! own series, so a Stump media item in such a library is a Kavita series
//! ([`SeriesKind::Book`]) whose single volume is the loose-leaf volume
//! (`-100000`, `Parser.LooseLeafVolumeNumber`) holding one special chapter, and
//! `series-detail` lists that chapter under `specials`. Field values follow
//! `kavita-ref` 0.9.1.4 (`komga-compat/kavita/capture/book-library.json`).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, Utc};
use models::{
	entity::{
		bookmark, collection, library, library_config, media, media_annotation,
		media_metadata, reading_list, reading_session, series, series_metadata,
	},
	shared::{enums::LibraryType as StumpLibraryType, readium::ReadiumLocator},
};

use crate::{
	dto::{
		AgeRating, AnnotationDto, AppUserCollectionDto, BookInfoDto, BookmarkDto,
		BookmarkSearchResultDto, ChapterDto, ChapterInfoDto, FileDimensionDto,
		FileTypeGroup, GenreTagDto, KavitaDateTime, KavitaFloat, LibraryDto, LibraryType,
		MangaFileDto, MangaFormat, MetadataLocksDto, PeopleDto, PersonDto, PersonRole,
		PublicationStatus, ReadingListDto, ReadingListItemChapterDto, ReadingListItemDto,
		ReadingListItemVolumeDto, ReadingListProvider, SearchResultDto, SeriesDetailDto,
		SeriesDto, SeriesMetadataDto, TagDto, VolumeDto,
	},
	progress::{last_progress_at, pages_read},
};

/// `Parser.DefaultChapterNumber`: the chapter number of a single-file volume.
pub const DEFAULT_CHAPTER_NUMBER: f32 = -100_000.0;
pub const DEFAULT_CHAPTER: &str = "-100000";
/// `Parser.LooseLeafVolumeNumber`: the volume number Kavita gives a file that
/// carries no volume, which is every book of a Book/LightNovel library.
pub const LOOSE_LEAF_VOLUME: i32 = -100_000;

/// Deterministic, restart-stable ids for names Kavita models as entities
/// (genres, people) while Stump stores them as text. FNV-1a folded into the
/// positive `int` range so clients can round-trip them through filters.
pub fn name_id(name: &str) -> i32 {
	let mut hash: u32 = 0x811c_9dc5;
	for byte in name.trim().to_lowercase().bytes() {
		hash ^= u32::from(byte);
		hash = hash.wrapping_mul(0x0100_0193);
	}
	let id = (hash & 0x7fff_ffff) as i32;
	if id == 0 {
		1
	} else {
		id
	}
}

/// Split Stump's comma-separated metadata lists, de-duplicated in order.
pub fn split_csv(raw: Option<&str>) -> Vec<String> {
	let mut values = Vec::new();
	let mut seen = BTreeSet::new();
	if let Some(raw) = raw {
		for value in raw
			.split(',')
			.map(str::trim)
			.filter(|value| !value.is_empty())
		{
			if seen.insert(value.to_lowercase()) {
				values.push(value.to_owned());
			}
		}
	}
	values
}

/// Kavita's `SeriesMetadataService` aggregation, which unions the values of
/// every row that contributes to a series. Stump keeps these as
/// comma-separated text, so the union is by trimmed value, ordered by first
/// appearance and de-duplicated case-insensitively.
#[derive(Default)]
struct NameUnion {
	seen: BTreeSet<String>,
	names: Vec<String>,
}

impl NameUnion {
	fn push(&mut self, name: &str) {
		let name = name.trim();
		if !name.is_empty() && self.seen.insert(name.to_lowercase()) {
			self.names.push(name.to_owned());
		}
	}

	/// Add one comma-separated Stump metadata column.
	fn push_csv(&mut self, raw: Option<&str>) {
		for value in raw.unwrap_or_default().split(',') {
			self.push(value);
		}
	}

	fn into_people(self, role: PersonRole) -> Vec<PersonDto> {
		self.names
			.into_iter()
			.map(|name| PersonDto::new(name_id(&name), name, vec![role]))
			.collect()
	}

	fn into_genres(self) -> Vec<GenreTagDto> {
		self.names
			.into_iter()
			.map(|title| GenreTagDto {
				id: name_id(&title),
				title,
			})
			.collect()
	}
}

/// A `media_metadata` text column, as the aggregation reads it.
type MediaColumn = fn(&media_metadata::Model) -> Option<&String>;

/// Union one series-metadata column with the same column of every media row
/// backing the series.
fn union_column(
	series_value: Option<&str>,
	media: &[MediaInput],
	column: Option<MediaColumn>,
) -> NameUnion {
	let mut union = NameUnion::default();
	union.push_csv(series_value);
	if let Some(column) = column {
		for item in media {
			union.push_csv(item.metadata.as_ref().and_then(column).map(String::as_str));
		}
	}
	union
}

/// The people Kavita lists on a series or chapter. The role/column pairs are
/// the in-memory twin of [`crate::routes::metadata::people_sources`], which
/// walks the same columns in SQL for `GET /api/Metadata/people`: Stump has no
/// column for Kavita's `translators` or `locations`, and carries an imprint
/// only on a series.
fn aggregate_people(
	series: Option<&series_metadata::Model>,
	media: &[MediaInput],
) -> PeopleDto {
	let role = |series_value: Option<&str>, column: Option<MediaColumn>, role| {
		union_column(series_value, media, column).into_people(role)
	};
	PeopleDto {
		writers: role(
			series.and_then(|m| m.writers.as_deref()),
			Some(|m| m.writers.as_ref()),
			PersonRole::Writer,
		),
		cover_artists: role(
			None,
			Some(|m| m.cover_artists.as_ref()),
			PersonRole::CoverArtist,
		),
		publishers: role(
			series.and_then(|m| m.publisher.as_deref()),
			Some(|m| m.publisher.as_ref()),
			PersonRole::Publisher,
		),
		characters: role(
			series.and_then(|m| m.characters.as_deref()),
			Some(|m| m.characters.as_ref()),
			PersonRole::Character,
		),
		pencillers: role(None, Some(|m| m.pencillers.as_ref()), PersonRole::Penciller),
		inkers: role(None, Some(|m| m.inkers.as_ref()), PersonRole::Inker),
		imprints: role(
			series.and_then(|m| m.imprint.as_deref()),
			None,
			PersonRole::Imprint,
		),
		colorists: role(None, Some(|m| m.colorists.as_ref()), PersonRole::Colorist),
		letterers: role(None, Some(|m| m.letterers.as_ref()), PersonRole::Letterer),
		editors: role(None, Some(|m| m.editors.as_ref()), PersonRole::Editor),
		translators: Vec::new(),
		teams: role(None, Some(|m| m.teams.as_ref()), PersonRole::Team),
		locations: Vec::new(),
	}
}

fn aggregate_genres(
	series: Option<&series_metadata::Model>,
	media: &[MediaInput],
) -> Vec<GenreTagDto> {
	union_column(
		series.and_then(|m| m.genres.as_deref()),
		media,
		Some(|m| m.genres.as_ref()),
	)
	.into_genres()
}

/// The series tags followed by every media tag, de-duplicated by title. Tag
/// ids stay the Stump `tags.id` the filter and `GET /api/Metadata/tags`
/// round-trip, so the first row carrying a title wins.
fn aggregate_tags(series_tags: &[TagDto], media: &[MediaInput]) -> Vec<TagDto> {
	let mut seen = BTreeSet::new();
	let mut tags = Vec::new();
	let rows = series_tags
		.iter()
		.chain(media.iter().flat_map(|item| item.tags.iter()));
	for tag in rows {
		let title = tag.title.trim();
		if !title.is_empty() && seen.insert(title.to_lowercase()) {
			tags.push(TagDto {
				id: tag.id,
				title: title.to_owned(),
			});
		}
	}
	tags
}

/// Stump's `locked_fields` (a JSON array of `MetadataField` names) as the
/// `*Locked` flags Kavita flattens into `SeriesMetadataDto` and `ChapterDto`.
/// Kavita also locks `translator` and `location`, which Stump does not model,
/// so those stay false; `language_lock` is the dedicated series column.
fn metadata_locks(
	locked_fields: Option<&serde_json::Value>,
	language_lock: bool,
) -> MetadataLocksDto {
	let fields = locked_field_names(locked_fields);
	let locked = |field: &str| fields.contains(field);
	MetadataLocksDto {
		language_locked: language_lock || locked("LANGUAGE"),
		summary_locked: locked("SUMMARY"),
		age_rating_locked: locked("AGE_RATING"),
		publication_status_locked: locked("STATUS"),
		genres_locked: locked("GENRES"),
		tags_locked: locked("TAGS"),
		writer_locked: locked("WRITERS"),
		character_locked: locked("CHARACTERS"),
		colorist_locked: locked("COLORISTS"),
		editor_locked: locked("EDITORS"),
		inker_locked: locked("INKERS"),
		imprint_locked: locked("IMPRINT"),
		letterer_locked: locked("LETTERERS"),
		penciller_locked: locked("PENCILLERS"),
		publisher_locked: locked("PUBLISHER"),
		translator_locked: false,
		team_locked: locked("TEAMS"),
		location_locked: false,
		cover_artist_locked: locked("COVER_ARTISTS"),
	}
}

/// The `MetadataField` names a `locked_fields` array holds; a malformed or
/// absent value locks nothing.
fn locked_field_names(locked_fields: Option<&serde_json::Value>) -> BTreeSet<&str> {
	locked_fields
		.and_then(|value| value.as_array())
		.map(|values| {
			values
				.iter()
				.filter_map(serde_json::Value::as_str)
				.collect()
		})
		.unwrap_or_default()
}

pub fn library_type(config: Option<&library_config::Model>) -> LibraryType {
	match config.map(|config| config.library_type) {
		Some(StumpLibraryType::Comic) => LibraryType::Comic,
		Some(StumpLibraryType::Book) | Some(StumpLibraryType::WebNovel) => {
			LibraryType::Book
		},
		Some(StumpLibraryType::LightNovel) => LibraryType::LightNovel,
		Some(StumpLibraryType::Manga)
		| Some(StumpLibraryType::Manhwa)
		| Some(StumpLibraryType::Webtoon)
		| Some(StumpLibraryType::Mixed)
		| None => LibraryType::Manga,
	}
}

/// The Stump library types whose media Kavita presents as their own series
/// (Kavita `Book` and `LightNovel` libraries): the `book_series` libraries.
pub const BOOK_LIBRARY_TYPES: [StumpLibraryType; 3] = [
	StumpLibraryType::Book,
	StumpLibraryType::WebNovel,
	StumpLibraryType::LightNovel,
];

pub fn map_library(
	id: i32,
	library: &library::Model,
	config: Option<&library_config::Model>,
) -> LibraryDto {
	LibraryDto {
		id,
		name: library.name.clone(),
		r#type: library_type(config),
		last_scanned: library.last_scanned_at.into(),
		cover_image: library
			.thumbnail_path
			.as_ref()
			.map(|_| format!("l{id}.png")),
		folder_watching: config.is_some_and(|config| config.watch),
		include_in_dashboard: true,
		include_in_recommended: true,
		manage_collections: true,
		manage_reading_lists: true,
		include_in_search: true,
		allow_scrobbling: false,
		folders: vec![library.path.clone()],
		collapse_series_relationships: false,
		library_file_types: vec![
			FileTypeGroup::Archive,
			FileTypeGroup::Epub,
			FileTypeGroup::Pdf,
			FileTypeGroup::Images,
		],
		exclude_patterns: Vec::new(),
		allow_metadata_matching: false,
		enable_metadata: config.is_some_and(|config| config.process_metadata),
		remove_prefix_for_sort_name: false,
		inherit_web_links_from_first_chapter: false,
		default_language: None,
		metadata_provider: None,
	}
}

/// A media item together with everything the volume/chapter mapping needs.
#[derive(Debug, Clone)]
pub struct MediaInput {
	pub id: i32,
	pub media: media::Model,
	pub metadata: Option<media_metadata::Model>,
	pub session: Option<reading_session::Model>,
	/// The Stump tags attached to the media item, in name order; Kavita has
	/// no per-chapter tag list of its own.
	pub tags: Vec<TagDto>,
	/// Position of the media in the series (1-based, name order).
	pub ordinal: i32,
	/// Whether the media is a book of a Book/LightNovel library, i.e. the
	/// single file of its own Kavita series: its volume is the loose-leaf
	/// volume and its chapter a special.
	pub book: bool,
}

impl MediaInput {
	pub fn pages(&self) -> i32 {
		if self.media.pages >= 0 {
			self.media.pages
		} else {
			self.metadata
				.as_ref()
				.and_then(|metadata| metadata.page_count)
				.unwrap_or_default()
				.max(0)
		}
	}

	pub fn pages_read(&self) -> i32 {
		pages_read(self.session.as_ref(), self.pages())
	}

	pub fn format(&self) -> MangaFormat {
		MangaFormat::from_extension(&self.media.extension)
	}

	/// The Kavita volume number for this media.
	pub fn number(&self) -> i32 {
		if self.book {
			return LOOSE_LEAF_VOLUME;
		}
		let metadata = self.metadata.as_ref();
		if let Some(volume) = metadata
			.and_then(|metadata| metadata.volume)
			.filter(|v| *v > 0)
		{
			return volume;
		}
		if let Some(number) = metadata
			.and_then(|metadata| metadata.number)
			.and_then(|number| number.to_string().parse::<f64>().ok())
			.filter(|number| number.is_finite() && *number > 0.0 && number.fract() == 0.0)
			.filter(|number| *number < f64::from(i32::MAX))
		{
			return number as i32;
		}
		self.ordinal
	}

	/// The metadata title, when it carries one.
	fn title(&self) -> Option<String> {
		self.metadata
			.as_ref()
			.and_then(|metadata| metadata.title.clone())
			.filter(|title| !title.trim().is_empty())
	}

	/// The name Kavita gives the chapter and, for a book, the series: the
	/// metadata title, else the file name without its extension.
	pub fn display_name(&self) -> String {
		self.title().unwrap_or_else(|| self.media.name.clone())
	}

	/// The folder holding the file; Kavita's `folderPath` for a book.
	fn folder(&self) -> Option<String> {
		std::path::Path::new(&self.media.path)
			.parent()
			.map(|parent| parent.to_string_lossy().into_owned())
			.filter(|parent| !parent.is_empty())
	}

	fn created(&self) -> KavitaDateTime {
		self.media.created_at.into()
	}

	fn modified(&self) -> KavitaDateTime {
		self.media.updated_at.or(Some(self.media.created_at)).into()
	}

	fn release_date(&self) -> KavitaDateTime {
		let metadata = self.metadata.as_ref();
		let year = metadata.and_then(|metadata| metadata.year);
		let month = metadata.and_then(|metadata| metadata.month).unwrap_or(1);
		let day = metadata.and_then(|metadata| metadata.day).unwrap_or(1);
		let date = year.and_then(|year| {
			NaiveDate::from_ymd_opt(year, month.max(1) as u32, day.max(1) as u32)
		});
		KavitaDateTime(date.map(|date| {
			DateTime::<Utc>::from_naive_utc_and_offset(
				date.and_hms_opt(0, 0, 0).expect("midnight is valid"),
				Utc,
			)
		}))
	}
}

/// The Kavita cover file name; clients only test that it is non-blank.
pub fn cover_name(volume_id: i32, chapter_id: i32) -> String {
	format!("v{volume_id}_c{chapter_id}.png")
}

pub fn map_file(input: &MediaInput) -> MangaFileDto {
	MangaFileDto {
		id: input.id,
		file_path: input.media.path.clone(),
		pages: input.pages(),
		bytes: input.media.size,
		format: input.format(),
		created: input.created(),
		extension: format!(".{}", input.media.extension.to_ascii_lowercase()),
		koreader_hash: input.media.koreader_hash.clone(),
	}
}

/// Book chapters (`input.book`) take the shape `kavita-ref` gives a file
/// without a volume: `range` is the title, `isSpecial` and `totalCount: 1`
/// (`komga-compat/kavita/capture/book-library.json`; the same instance
/// reports `totalCount: 0` for an archive without a ComicInfo `Count`, which
/// is every Stump file, since Stump stores that number once per series in
/// `series_metadata.total_issues` and never per file).
pub fn map_chapter(input: &MediaInput) -> ChapterDto {
	let locked = input
		.metadata
		.as_ref()
		.and_then(|metadata| metadata.locked_fields.as_ref());
	let lock_names = locked_field_names(locked);
	let metadata = input.metadata.as_ref();
	let pages = input.pages();
	let pages_read = input.pages_read();
	let last_progress: KavitaDateTime = last_progress_at(input.session.as_ref()).into();
	let title = input.title();
	let number = input.number();
	ChapterDto {
		id: input.id,
		range: if input.book {
			input.display_name()
		} else {
			input.media.name.clone()
		},
		number: DEFAULT_CHAPTER.to_owned(),
		min_number: KavitaFloat(DEFAULT_CHAPTER_NUMBER),
		max_number: KavitaFloat(DEFAULT_CHAPTER_NUMBER),
		sort_order: KavitaFloat(number as f32),
		pages,
		is_special: input.book,
		title: input.display_name(),
		files: vec![map_file(input)],
		pages_read,
		total_reads: input
			.session
			.as_ref()
			.map(|session| session.readthrough_number.max(0))
			.unwrap_or(0),
		last_reading_progress_utc: last_progress,
		last_reading_progress: last_progress,
		cover_image_locked: false,
		volume_id: input.id,
		created_utc: input.created(),
		last_modified_utc: input.modified(),
		created: input.created(),
		release_date: input.release_date(),
		title_name: title.unwrap_or_default(),
		summary: metadata.and_then(|metadata| metadata.summary.clone()),
		age_rating: AgeRating::from_min_age(
			metadata.and_then(|metadata| metadata.age_rating),
		),
		word_count: 0,
		volume_title: String::new(),
		min_hours_to_read: 0,
		max_hours_to_read: 0,
		avg_hours_to_read: KavitaFloat(0.0),
		web_links: metadata
			.and_then(|metadata| metadata.links.clone())
			.unwrap_or_default(),
		isbn: metadata
			.and_then(|metadata| metadata.identifier_isbn.clone())
			.unwrap_or_default(),
		people: aggregate_people(None, std::slice::from_ref(input)),
		genres: aggregate_genres(None, std::slice::from_ref(input)),
		tags: aggregate_tags(&[], std::slice::from_ref(input)),
		publication_status: PublicationStatus::OnGoing,
		language: metadata.and_then(|metadata| metadata.language.clone()),
		count: 0,
		total_count: i32::from(input.book),
		locks: metadata_locks(locked, false),
		release_date_locked: lock_names.contains("RELEASE_DATE"),
		title_name_locked: lock_names.contains("TITLE"),
		sort_order_locked: lock_names.contains("NUMBER"),
		cover_image: cover_name(input.id, input.id),
		primary_color: None,
		secondary_color: None,
		format: input.format(),
		ani_list_id: 0,
		mal_id: 0,
		hardcover_id: 0,
		metron_id: 0,
		comic_vine_id: None,
		manga_baka_id: 0,
		cbr_id: 0,
	}
}

pub fn map_volume(series_id: i32, input: &MediaInput) -> VolumeDto {
	let number = input.number();
	VolumeDto {
		id: input.id,
		min_number: KavitaFloat(number as f32),
		max_number: KavitaFloat(number as f32),
		name: number.to_string(),
		number,
		pages: input.pages(),
		pages_read: input.pages_read(),
		last_modified_utc: input.modified(),
		created_utc: input.created(),
		created: input.created(),
		last_modified: input.modified(),
		series_id,
		chapters: vec![map_chapter(input)],
		min_hours_to_read: 0,
		max_hours_to_read: 0,
		avg_hours_to_read: KavitaFloat(0.0),
		word_count: 0,
		cover_image: cover_name(input.id, input.id),
		primary_color: None,
		secondary_color: None,
		ani_list_id: 0,
		mal_id: 0,
		hardcover_id: 0,
		metron_id: 0,
		comic_vine_id: None,
		manga_baka_id: 0,
		cbr_id: 0,
	}
}

/// Order media the way Kavita orders volumes: by number, then name.
pub fn sort_media(media: &mut [MediaInput]) {
	media.sort_by(|left, right| {
		left.number()
			.cmp(&right.number())
			.then_with(|| left.media.name.cmp(&right.media.name))
	});
}

/// What backs a Kavita series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesKind {
	/// A Stump series whose media are the volumes (Manga/Comic libraries).
	Grouped,
	/// One media item of a Book/LightNovel library; `media` holds exactly that
	/// item and `series` is the Stump series it is filed under.
	Book,
}

/// A series together with the media that back its volumes.
#[derive(Debug, Clone)]
pub struct SeriesInput {
	pub id: i32,
	pub series: series::Model,
	pub metadata: Option<series_metadata::Model>,
	/// The Stump tags attached to the series row, in name order. A book
	/// series is one file, not a Stump series, so it carries none of its own
	/// and aggregates its media's tags instead.
	pub tags: Vec<TagDto>,
	pub library_id: i32,
	pub library_name: String,
	pub media: Vec<MediaInput>,
	pub kind: SeriesKind,
}

impl SeriesInput {
	/// The media item this series is, when it is a book.
	pub fn book(&self) -> Option<&MediaInput> {
		match self.kind {
			SeriesKind::Book => self.media.first(),
			SeriesKind::Grouped => None,
		}
	}

	pub fn name(&self) -> String {
		if let Some(book) = self.book() {
			return book.display_name();
		}
		self.metadata
			.as_ref()
			.and_then(|metadata| metadata.title.clone())
			.filter(|title| !title.trim().is_empty())
			.unwrap_or_else(|| self.series.name.clone())
	}

	pub fn sort_name(&self) -> String {
		let title_sort = match self.book() {
			Some(book) => book
				.metadata
				.as_ref()
				.and_then(|metadata| metadata.title_sort.clone()),
			None => self
				.metadata
				.as_ref()
				.and_then(|metadata| metadata.title_sort.clone()),
		};
		title_sort
			.filter(|title| !title.trim().is_empty())
			.unwrap_or_else(|| self.name())
	}

	/// Kavita's `folderPath`: the series folder, or for a book the folder
	/// holding the file.
	fn folder_path(&self) -> String {
		self.book()
			.and_then(MediaInput::folder)
			.unwrap_or_else(|| self.series.path.clone())
	}

	pub fn pages(&self) -> i32 {
		self.media.iter().map(MediaInput::pages).sum()
	}

	pub fn pages_read(&self) -> i32 {
		self.media.iter().map(MediaInput::pages_read).sum()
	}

	pub fn format(&self) -> MangaFormat {
		self.media
			.first()
			.map(MediaInput::format)
			.unwrap_or(MangaFormat::Unknown)
	}

	fn latest_read(&self) -> KavitaDateTime {
		self.media
			.iter()
			.filter_map(|media| last_progress_at(media.session.as_ref()))
			.max()
			.into()
	}

	fn last_chapter_added(&self) -> KavitaDateTime {
		self.media
			.iter()
			.map(|media| media.media.created_at.with_timezone(&Utc))
			.max()
			.or(Some(self.series.created_at.with_timezone(&Utc)))
			.into()
	}

	/// The most recent progress instant across the series' media: the
	/// `LatestReadDate` primary sort of Kavita's on-deck query.
	pub(crate) fn latest_read_at(&self) -> Option<chrono::DateTime<Utc>> {
		self.media
			.iter()
			.filter_map(|media| last_progress_at(media.session.as_ref()))
			.max()
	}

	/// The newest media creation instant, falling back to the series' own:
	/// the `LastChapterAdded` tiebreak of Kavita's on-deck query.
	pub(crate) fn last_chapter_added_at(&self) -> chrono::DateTime<Utc> {
		self.media
			.iter()
			.map(|media| media.media.created_at.with_timezone(&Utc))
			.max()
			.unwrap_or(self.series.created_at.with_timezone(&Utc))
	}

	fn first_media_id(&self) -> i32 {
		self.media.first().map(|media| media.id).unwrap_or(self.id)
	}
}

pub fn map_series(input: &SeriesInput) -> SeriesDto {
	let name = input.name();
	let cover_id = input.first_media_id();
	let book = input.book();
	// A book series is created and scanned with its file; a grouped series
	// carries its own timestamps.
	let created = book
		.map(|book| book.media.created_at)
		.unwrap_or(input.series.created_at);
	let last_scanned = match book {
		Some(book) => book.media.updated_at.unwrap_or(book.media.created_at),
		None => input.series.updated_at.unwrap_or(input.series.created_at),
	};
	let folder_path = input.folder_path();
	SeriesDto {
		id: input.id,
		name: name.clone(),
		original_name: match book {
			Some(_) => name,
			None => input.series.name.clone(),
		},
		localized_name: String::new(),
		sort_name: input.sort_name(),
		pages: input.pages(),
		cover_image_locked: match book {
			Some(book) => book.media.thumbnail_path.is_some(),
			None => input.series.thumbnail_path.is_some(),
		},
		last_chapter_added: input.last_chapter_added(),
		last_chapter_added_utc: input.last_chapter_added(),
		user_rating: KavitaFloat(0.0),
		has_user_rated: false,
		total_reads: 0,
		pages_read: input.pages_read(),
		latest_read_date: input.latest_read(),
		format: input.format(),
		created: created.into(),
		sort_name_locked: book.is_none()
			&& input
				.metadata
				.as_ref()
				.is_some_and(|metadata| metadata.title_sort_lock),
		localized_name_locked: false,
		name_locked: false,
		word_count: 0,
		library_id: input.library_id,
		library_name: input.library_name.clone(),
		min_hours_to_read: 0,
		max_hours_to_read: 0,
		avg_hours_to_read: KavitaFloat(0.0),
		folder_path: folder_path.clone(),
		lowest_folder_path: folder_path,
		last_folder_scanned: Some(last_scanned).into(),
		dont_match: false,
		is_blacklisted: false,
		is_stand_alone: false,
		metadata_provider_override: None,
		cover_image: cover_name(cover_id, cover_id),
		primary_color: None,
		secondary_color: None,
		ani_list_id: 0,
		mal_id: 0,
		hardcover_id: 0,
		metron_id: 0,
		comic_vine_id: None,
		manga_baka_id: 0,
		manga_baka_edition_id: None,
		cbr_id: 0,
	}
}

/// `SeriesMetadataService.UpdateSeriesMetadata`: Kavita recomputes a series'
/// metadata from every chapter under it, so each list here is the union of
/// the Stump series row and the metadata of every media item backing it
/// (first appearance wins, de-duplicated case-insensitively) and each scalar
/// prefers the series row before falling back to the media.
///
/// Book series (`input.book()`) are one file that Kavita presents as its own
/// series, so they take the file's metadata alone, with
/// `maxCount == totalCount == 1` and therefore `Completed`
/// (`komga-compat/kavita/capture/book-library.json`).
pub fn map_series_metadata(input: &SeriesInput) -> SeriesMetadataDto {
	if let Some(book) = input.book() {
		let file = std::slice::from_ref(book);
		let metadata = book.metadata.as_ref();
		let locked = metadata.and_then(|metadata| metadata.locked_fields.as_ref());
		return SeriesMetadataDto {
			id: input.id,
			summary: metadata
				.and_then(|metadata| metadata.summary.clone())
				.unwrap_or_default(),
			genres: aggregate_genres(None, file),
			tags: aggregate_tags(&[], file),
			people: aggregate_people(None, file),
			age_rating: AgeRating::from_min_age(
				metadata.and_then(|metadata| metadata.age_rating),
			),
			release_year: metadata.and_then(|metadata| metadata.year).unwrap_or(0),
			language: metadata
				.and_then(|metadata| metadata.language.clone())
				.unwrap_or_default(),
			max_count: 1,
			total_count: 1,
			publication_status: PublicationStatus::Completed,
			web_links: metadata
				.and_then(|metadata| metadata.links.clone())
				.unwrap_or_default(),
			locks: metadata_locks(locked, false),
			release_year_locked: locked_field_names(locked).contains("YEAR"),
			series_id: input.id,
		};
	}
	let metadata = input.metadata.as_ref();
	let media_metadata = || {
		input
			.media
			.iter()
			.filter_map(|media| media.metadata.as_ref())
	};
	let locked = metadata.and_then(|metadata| metadata.locked_fields.as_ref());
	let total_issues = metadata
		.and_then(|metadata| metadata.total_issues)
		.unwrap_or(0);
	SeriesMetadataDto {
		id: input.id,
		summary: metadata
			.and_then(|metadata| metadata.summary.clone())
			.or_else(|| input.series.description.clone())
			.unwrap_or_default(),
		genres: aggregate_genres(metadata, &input.media),
		tags: aggregate_tags(&input.tags, &input.media),
		people: aggregate_people(metadata, &input.media),
		// Kavita keeps the strictest rating of any chapter; Stump's ratings
		// are minimum ages, so that is the largest one.
		age_rating: AgeRating::from_min_age(
			metadata
				.and_then(|metadata| metadata.age_rating)
				.or_else(|| media_metadata().filter_map(|m| m.age_rating).max()),
		),
		// The series started with its earliest-dated file.
		release_year: metadata
			.and_then(|metadata| metadata.year)
			.or_else(|| {
				media_metadata()
					.filter_map(|m| m.year)
					.filter(|year| *year > 0)
					.min()
			})
			.unwrap_or(0),
		language: metadata
			.and_then(|metadata| metadata.language.clone())
			.or_else(|| {
				media_metadata()
					.filter_map(|m| m.language.clone())
					.find(|language| !language.trim().is_empty())
			})
			.unwrap_or_default(),
		max_count: total_issues,
		total_count: total_issues,
		publication_status: PublicationStatus::from_status_text(
			metadata.and_then(|metadata| metadata.status.as_deref()),
		),
		web_links: metadata
			.and_then(|metadata| metadata.links.clone())
			.unwrap_or_default(),
		locks: metadata_locks(
			locked,
			metadata.is_some_and(|metadata| metadata.language_lock),
		),
		release_year_locked: locked_field_names(locked).contains("YEAR"),
		series_id: input.id,
	}
}

/// Whether a volume or chapter still counts as unread.
fn is_unread(pages_read: i32, pages: i32) -> bool {
	pages_read < pages || pages == 0
}

/// `SeriesService.GetSeriesDetail`: grouped series list their volumes; a book
/// is one special chapter, listed under `specials` with no volumes.
pub fn map_series_detail(
	input: &SeriesInput,
	library_type: LibraryType,
) -> SeriesDetailDto {
	if let Some(book) = input.book() {
		let chapter = map_chapter(book);
		let unread_count = i32::from(is_unread(chapter.pages_read, chapter.pages));
		return SeriesDetailDto {
			specials: vec![chapter],
			chapters: Vec::new(),
			volumes: Vec::new(),
			storyline_chapters: Vec::new(),
			library_type,
			unread_count,
			total_count: 1,
		};
	}
	let volumes = input
		.media
		.iter()
		.map(|media| map_volume(input.id, media))
		.collect::<Vec<_>>();
	let total_count = i32::try_from(volumes.len()).unwrap_or(i32::MAX);
	let unread_count = volumes
		.iter()
		.filter(|volume| is_unread(volume.pages_read, volume.pages))
		.count();
	SeriesDetailDto {
		specials: Vec::new(),
		chapters: Vec::new(),
		volumes,
		storyline_chapters: Vec::new(),
		library_type,
		unread_count: i32::try_from(unread_count).unwrap_or(i32::MAX),
		total_count,
	}
}

/// `Parser.SpecialVolume`: the volume number Kavita gives a file it parsed as
/// a special of a numbered series.
pub const SPECIAL_VOLUME: &str = "100000";

/// `Parser.CleanSpecialTitle`: underscores become spaces and `SP<digits>`
/// tokens are dropped; a title that cleans to nothing keeps its original.
pub fn clean_special_title(name: &str) -> String {
	let spaced = name.replace('_', " ");
	let mut cleaned = String::with_capacity(spaced.len());
	let mut chars = spaced.char_indices();
	while let Some((index, ch)) = chars.next() {
		let tail = &spaced[index..];
		// `SP\d+`, case-insensitively: skip the token and its digit run.
		if ch.eq_ignore_ascii_case(&'s')
			&& tail.len() >= 3
			&& tail[1..2].eq_ignore_ascii_case("p")
			&& tail[2..].starts_with(|c: char| c.is_ascii_digit())
		{
			let digits = tail[2..]
				.find(|c: char| !c.is_ascii_digit())
				.unwrap_or(tail.len() - 2);
			// The `p` plus every digit; all ASCII, so one `char` each.
			for _ in 0..=digits {
				chars.next();
			}
			continue;
		}
		cleaned.push(ch);
	}
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		name.to_owned()
	} else {
		trimmed.to_owned()
	}
}

/// The file name `GET /api/Reader/image` serves a page under, reused as
/// `FileDimensionDto.fileName`: Stump has no per-page archive entry name in
/// the database, so the page is named exactly as the page route names it.
pub fn page_file_name(media_name: &str, page: i32) -> String {
	format!("{}-{page}.img", media_name.replace(['/', '\\', '"'], "_"))
}

/// `ReaderService.GetPairs`: page 0 stands alone, then pages pair up
/// left-to-right, and a wide page (or the page after one) breaks the pairing.
pub fn double_pairs(dimensions: &[FileDimensionDto]) -> BTreeMap<String, i32> {
	let mut pairs = BTreeMap::new();
	let Some(first) = dimensions.first() else {
		return pairs;
	};
	pairs.insert(first.page_number.to_string(), first.page_number);
	let mut pair_start = true;
	let mut previous = first;
	for dimension in dimensions.iter().skip(1) {
		let page = dimension.page_number;
		if dimension.is_wide || previous.is_wide || previous.page_number == 0 {
			pairs.insert(page.to_string(), page);
			pair_start = true;
		} else {
			pairs.insert(page.to_string(), if pair_start { page - 1 } else { page });
			pair_start = !pair_start;
		}
		previous = dimension;
	}
	pairs
}

/// `ReaderController.GetChapterInfo`. `libraryType` is `Manga` for every
/// chapter: Kavita never assigns the field, so its `ChapterInfoDto` always
/// carries `default(LibraryType)` — `kavita-ref` reports `0` for a chapter of
/// a Comic library and of a Book library alike.
pub fn map_chapter_info(
	input: &SeriesInput,
	media: &MediaInput,
	page_dimensions: Option<Vec<FileDimensionDto>>,
) -> ChapterInfoDto {
	let chapter = map_chapter(media);
	let volume_number = if media.book {
		DEFAULT_CHAPTER.to_owned()
	} else {
		media.number().to_string()
	};
	let file_name = std::path::Path::new(&media.media.path)
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_else(|| media.media.name.clone());
	let series_name = input.name();
	let title = if chapter.title_name.is_empty() {
		series_name.clone()
	} else {
		format!("{series_name} - {}", chapter.title_name)
	};
	// `GetChapterInfo`'s subtitle rules, with `LibraryType.Manga`'s
	// "Chapter " label: a special is named by its file, a loose-leaf volume by
	// its chapter and a numbered volume by its volume.
	let subtitle = if chapter.is_special {
		std::path::Path::new(&file_name)
			.file_stem()
			.map(|stem| stem.to_string_lossy().into_owned())
			.unwrap_or_else(|| file_name.clone())
	} else if volume_number == DEFAULT_CHAPTER {
		format!("Chapter {}", chapter.number)
	} else if chapter.number == DEFAULT_CHAPTER {
		format!("Volume {volume_number}")
	} else {
		format!("Volume {volume_number} Chapter {}", chapter.number)
	};
	let double_pairs = page_dimensions
		.as_deref()
		.map(|dimensions| double_pairs(dimensions));
	ChapterInfoDto {
		chapter_number: chapter.number,
		volume_number,
		volume_id: media.id,
		series_name,
		series_format: input.format(),
		series_id: input.id,
		library_id: input.library_id,
		library_type: LibraryType::Manga,
		chapter_title: chapter.title_name,
		pages: chapter.pages,
		file_name,
		is_special: chapter.is_special,
		subtitle,
		title,
		series_total_pages: input.pages(),
		series_total_pages_read: input.pages_read(),
		page_dimensions,
		double_pairs,
	}
}

/// `BookController.GetBookInfo`: the same identity block as `chapter-info`
/// without the page dimensions. `chapterTitle` is `null` when the file has no
/// metadata title, exactly as `kavita-ref` reports for a bare EPUB.
pub fn map_book_info(input: &SeriesInput, media: &MediaInput) -> BookInfoDto {
	let chapter = map_chapter(media);
	BookInfoDto {
		book_title: chapter.title_name.clone(),
		series_id: input.id,
		volume_id: media.id,
		series_format: input.format(),
		series_name: input.name(),
		chapter_number: chapter.number,
		volume_number: if media.book {
			DEFAULT_CHAPTER.to_owned()
		} else {
			media.number().to_string()
		},
		library_id: input.library_id,
		pages: chapter.pages,
		is_special: chapter.is_special,
		chapter_title: Some(chapter.title_name).filter(|title| !title.is_empty()),
	}
}

/// `EntityNamingService.FormatReadingListItemTitle`: EPUBs are named by their
/// chapter (a book's title) under the volume label, a default-numbered
/// chapter of a numbered volume by that volume, a special by its title or
/// cleaned chapter and everything else by the library's chapter label.
fn reading_list_item_title(
	library_type: LibraryType,
	format: MangaFormat,
	chapter_number: &str,
	volume_number: &str,
	chapter_title_name: &str,
	is_special: bool,
) -> String {
	if format == MangaFormat::Epub {
		let cleaned = clean_special_title(chapter_number);
		if cleaned == DEFAULT_CHAPTER {
			if !chapter_title_name.is_empty() {
				return chapter_title_name.to_owned();
			}
			return format!("Volume {}", clean_special_title(volume_number));
		}
		if volume_number == SPECIAL_VOLUME {
			return cleaned;
		}
		return format!("Volume {cleaned}");
	}
	if chapter_number == DEFAULT_CHAPTER && volume_number != DEFAULT_CHAPTER {
		return format!("Volume {volume_number}");
	}
	let display = if chapter_number
		.chars()
		.all(|c| c.is_ascii_digit() || c == '.')
		&& !chapter_number.is_empty()
	{
		chapter_number.to_owned()
	} else {
		clean_special_title(chapter_number)
	};
	if chapter_number == DEFAULT_CHAPTER && !chapter_title_name.is_empty() {
		return chapter_title_name.to_owned();
	}
	if is_special {
		return if chapter_title_name.is_empty() {
			display
		} else {
			chapter_title_name.to_owned()
		};
	}
	match library_type {
		LibraryType::Comic | LibraryType::ComicVine => format!("Issue #{display}"),
		LibraryType::Book | LibraryType::LightNovel => format!("Book {display}"),
		_ => format!("Chapter {display}"),
	}
}

/// `ReadingListService.GetReadingListItems` item: the media as a reading-list
/// entry, carrying the series, volume and chapter identity blocks Kavita
/// projects alongside it.
pub fn map_reading_list_item(
	reading_list_id: i32,
	item_id: i32,
	order: i32,
	input: &SeriesInput,
	media: &MediaInput,
	library_type: LibraryType,
) -> ReadingListItemDto {
	let chapter = map_chapter(media);
	let volume_name = if media.book {
		DEFAULT_CHAPTER.to_owned()
	} else {
		media.number().to_string()
	};
	let writer = chapter.people.writers.first();
	let penciller = chapter.people.pencillers.first();
	let title = reading_list_item_title(
		library_type,
		input.format(),
		&chapter.range,
		&volume_name,
		&chapter.title_name,
		chapter.is_special,
	);
	ReadingListItemDto {
		id: item_id,
		order,
		chapter_id: media.id,
		series_id: input.id,
		series_name: input.name(),
		series_sort_name: input.sort_name(),
		series_format: input.format(),
		pages_read: chapter.pages_read,
		pages_total: chapter.pages,
		chapter_number: chapter.range.clone(),
		volume_number: volume_name.clone(),
		chapter_title_name: chapter.title_name.clone(),
		volume_id: media.id,
		library_id: input.library_id,
		title,
		library_type,
		library_name: input.library_name.clone(),
		release_date: chapter.release_date,
		reading_list_id,
		last_reading_progress_utc: chapter.last_reading_progress_utc,
		file_size: media.media.size,
		summary: chapter.summary.clone().unwrap_or_default(),
		is_special: chapter.is_special,
		chapter: ReadingListItemChapterDto {
			id: media.id,
			range: chapter.range,
			title_name: chapter.title_name,
			min_number: chapter.min_number,
			max_number: chapter.max_number,
			sort_order: chapter.sort_order,
			pages: chapter.pages,
			is_special: chapter.is_special,
			release_date: chapter.release_date,
			summary: chapter.summary.unwrap_or_default(),
			writer_name: writer.map(|person| person.name.clone()),
			writer_id: writer.map(|person| person.id),
			penciller_name: penciller.map(|person| person.name.clone()),
			penciller_id: penciller.map(|person| person.id),
		},
		volume: ReadingListItemVolumeDto {
			id: media.id,
			name: volume_name,
			min_number: KavitaFloat(media.number() as f32),
			max_number: KavitaFloat(media.number() as f32),
			series_id: input.id,
		},
	}
}

/// `ReadingListDto`: a Stump reading list as Kavita reports one. Stump has no
/// promotion, CBL provenance or age rating on a list, so those keep the
/// values `kavita-ref` shows for a freshly created list; `summary` is the
/// list's description and `startingYear`/`endingYear` stay `0` because Stump
/// does not compute a list's date range.
pub fn map_reading_list(
	id: i32,
	list: &reading_list::Model,
	item_count: i32,
	owner_user_name: String,
) -> ReadingListDto {
	ReadingListDto {
		id,
		title: list.name.clone(),
		summary: list.description.clone().unwrap_or_default(),
		promoted: false,
		cover_image_locked: false,
		cover_image: None,
		primary_color: None,
		secondary_color: None,
		item_count,
		starting_year: 0,
		starting_month: 0,
		ending_year: 0,
		ending_month: 0,
		age_rating: AgeRating::Unknown,
		owner_user_name,
		source_path: None,
		download_url: None,
		sha_hash: None,
		provider: ReadingListProvider::None,
		last_sync_check_utc: None,
		last_synced_utc: None,
		total_items_at_import: 0,
		tags: Vec::new(),
		can_sync: false,
	}
}

/// `SearchResultDto`: the trimmed series projection Kavita's search returns.
/// `volumeCount`/`chapterCount` are the series' media count (one volume with
/// one chapter per file), so a book series reports `1`/`1`.
pub fn map_search_result(input: &SeriesInput) -> SearchResultDto {
	let name = input.name();
	let count = i32::try_from(input.media.len()).unwrap_or(i32::MAX);
	let release_year = match input.book() {
		Some(book) => book.metadata.as_ref().and_then(|metadata| metadata.year),
		None => input.metadata.as_ref().and_then(|metadata| metadata.year),
	};
	SearchResultDto {
		series_id: input.id,
		original_name: match input.book() {
			Some(_) => name.clone(),
			None => input.series.name.clone(),
		},
		name,
		sort_name: input.sort_name(),
		localized_name: String::new(),
		format: input.format(),
		library_name: input.library_name.clone(),
		library_id: input.library_id,
		release_year: release_year.unwrap_or(0),
		volume_count: count,
		chapter_count: count,
	}
}

/// `BookmarkSearchResultDto`: the series a bookmark points into.
pub fn map_bookmark_search_result(
	input: &SeriesInput,
	media: &MediaInput,
) -> BookmarkSearchResultDto {
	BookmarkSearchResultDto {
		library_id: input.library_id,
		volume_id: media.id,
		series_id: input.id,
		chapter_id: media.id,
		series_name: input.name(),
		localized_series_name: String::new(),
	}
}

/// `BookmarkDto`: a Stump bookmark as Kavita reports one. Stump stores the
/// bookmarked page against the media item, which is the Kavita volume and its
/// single chapter at once, so `volumeId == chapterId`. `imageOffset` is `0`
/// (Kavita's split-image offset, which Stump does not model) and `xPath`
/// carries the Readium fragment of a locator-addressed bookmark.
pub fn map_bookmark(
	id: i32,
	bookmark: &bookmark::Model,
	input: &SeriesInput,
	media: &MediaInput,
	include_series: bool,
) -> BookmarkDto {
	BookmarkDto {
		id,
		page: bookmark.page.unwrap_or(0),
		volume_id: media.id,
		series_id: input.id,
		chapter_id: media.id,
		image_offset: 0,
		x_path: bookmark.locator.as_ref().and_then(locator_fragment),
		series: include_series.then(|| map_series(input)),
		chapter_title: None,
	}
}

/// The DOM anchor of a Readium locator: its first fragment, else its CSS
/// selector, else its partial CFI — the three ways Stump records a position
/// inside a document, and what Kavita's `xPath` names.
fn locator_fragment(locator: &ReadiumLocator) -> Option<String> {
	let locations = locator.locations.as_ref()?;
	locations
		.fragments
		.as_ref()
		.and_then(|fragments| fragments.first().cloned())
		.or_else(|| locations.css_selector.clone())
		.or_else(|| locations.partial_cfi.clone())
}

/// `AppUserCollectionDto`: a Stump collection as Kavita reports one.
pub fn map_collection(
	id: i32,
	collection: &collection::Model,
	item_count: i32,
	owner: String,
) -> AppUserCollectionDto {
	AppUserCollectionDto {
		id,
		title: collection.name.clone(),
		summary: collection.description.clone().unwrap_or_default(),
		promoted: false,
		age_rating: AgeRating::Unknown,
		cover_image: None,
		primary_color: None,
		secondary_color: None,
		cover_image_locked: false,
		item_count,
		owner,
		last_sync_utc: KavitaDateTime::default(),
		source: 0,
		source_url: None,
		total_source_count: 0,
		missing_series_from_source: None,
	}
}

/// `AnnotationDto`: a Stump media annotation as Kavita reports one.
///
/// Stump keeps the highlighted range in the Readium locator (`text.highlight`
/// with its surrounding context) and the note in `annotation_text`; Kavita
/// splits those into `selectedText` and `comment`. Annotations are private to
/// their owner in Stump, so `likes` is empty, `containsSpoiler` is false and
/// `highlightCount` counts the one highlight this row carries.
pub fn map_annotation(
	id: i32,
	annotation: &media_annotation::Model,
	input: &SeriesInput,
	media: &MediaInput,
	owner_user_id: i32,
	owner_username: String,
) -> AnnotationDto {
	let text = annotation.locator.text.as_ref();
	let highlight = text.and_then(|text| text.highlight.clone());
	let context = text.map(|text| {
		format!(
			"{}{}{}",
			text.before.clone().unwrap_or_default(),
			highlight.clone().unwrap_or_default(),
			text.after.clone().unwrap_or_default()
		)
	});
	let comment = annotation
		.annotation_text
		.clone()
		.filter(|note| !note.trim().is_empty());
	AnnotationDto {
		id,
		x_path: locator_fragment(&annotation.locator).unwrap_or_default(),
		ending_x_path: None,
		selected_text: highlight.clone(),
		comment: comment.clone(),
		comment_html: comment.clone(),
		comment_plain_text: comment,
		chapter_title: Some(annotation.locator.chapter_title.clone())
			.filter(|title| !title.is_empty()),
		context: context.filter(|context| !context.is_empty()),
		highlight_count: i32::from(highlight.is_some()),
		contains_spoiler: false,
		page_number: annotation
			.locator
			.locations
			.as_ref()
			.and_then(|locations| locations.position)
			.unwrap_or(0),
		selected_slot_index: 0,
		likes: Vec::new(),
		series_name: input.name(),
		library_name: input.library_name.clone(),
		chapter_id: media.id,
		volume_id: media.id,
		series_id: input.id,
		library_id: input.library_id,
		owner_user_id,
		owner_username,
		age_rating: AgeRating::Unknown,
		created_utc: KavitaDateTime::new(annotation.created_at.with_timezone(&Utc)),
		last_modified_utc: KavitaDateTime::new(annotation.updated_at.with_timezone(&Utc)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::TimeZone;
	use models::shared::enums::FileStatus;
	use rust_decimal::Decimal;

	fn media(name: &str, extension: &str, pages: i32) -> media::Model {
		media::Model {
			id: format!("media-{name}"),
			name: name.to_owned(),
			size: 1234,
			extension: extension.to_owned(),
			pages,
			updated_at: None,
			created_at: Utc.with_ymd_and_hms(2026, 9, 5, 0, 30, 44).unwrap().into(),
			modified_at: None,
			hash: None,
			koreader_hash: Some("ABC".to_owned()),
			path: format!("/library/Series/{name}.{extension}"),
			status: FileStatus::Ready,
			thumbnail_meta: None,
			thumbnail_path: None,
			series_id: Some("series-1".to_owned()),
			deleted_at: None,
			source_provider: None,
			remote_id: None,
			remote_chapter_id: None,
		}
	}

	fn input(name: &str, extension: &str, pages: i32, ordinal: i32) -> MediaInput {
		MediaInput {
			id: 10 + ordinal,
			media: media(name, extension, pages),
			metadata: None,
			tags: Vec::new(),
			session: None,
			ordinal,
			book: false,
		}
	}

	fn series_model(name: &str) -> series::Model {
		series::Model {
			id: "series-1".to_owned(),
			name: name.to_owned(),
			description: None,
			created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap().into(),
			updated_at: None,
			deleted_at: None,
			path: "/library/Collection".to_owned(),
			status: FileStatus::Ready,
			thumbnail_meta: None,
			thumbnail_path: None,
			library_id: Some("lib".to_owned()),
			source_provider: None,
			remote_id: None,
		}
	}

	/// The shape `kavita-ref` 0.9.1.4 gives a Book-library EPUB
	/// (`komga-compat/kavita/capture/book-library.json`, `book_volumes_4`,
	/// `book_series_detail_4`, `book_metadata_4`).
	#[test]
	fn book_series_is_one_loose_leaf_volume_with_a_special_chapter() {
		let mut book = input("alice", "epub", 15, 1);
		book.book = true;
		book.metadata = Some(media_metadata::Model {
			title: Some("Alice's Adventures in Wonderland".to_owned()),
			writers: Some("Lewis Carroll".to_owned()),
			genres: Some("Fantasy fiction, Children's stories".to_owned()),
			language: Some("en".to_owned()),
			year: Some(2008),
			month: Some(6),
			day: Some(27),
			..default_media_metadata()
		});
		let series = SeriesInput {
			id: 40,
			series: series_model("Collection"),
			metadata: None,
			tags: Vec::new(),
			library_id: 2,
			library_name: "Book Library".to_owned(),
			media: vec![book],
			kind: SeriesKind::Book,
		};

		let dto = map_series(&series);
		assert_eq!(dto.name, "Alice's Adventures in Wonderland");
		assert_eq!(dto.original_name, "Alice's Adventures in Wonderland");
		assert_eq!(dto.sort_name, "Alice's Adventures in Wonderland");
		assert_eq!(dto.pages, 15);
		assert_eq!(dto.format, MangaFormat::Epub);
		assert_eq!(dto.folder_path, "/library/Series");
		assert_eq!(dto.lowest_folder_path, "/library/Series");
		assert_eq!(dto.cover_image, "v11_c11.png");
		assert_eq!(
			serde_json::to_value(&dto).unwrap()["created"],
			serde_json::json!("2026-09-05T00:30:44.0000000"),
			"a book is created with its file, not with the folder series"
		);

		let volume = map_volume(series.id, &series.media[0]);
		assert_eq!(volume.series_id, 40);
		assert_eq!(volume.number, -100000);
		assert_eq!(volume.min_number, KavitaFloat(-100000.0));
		assert_eq!(volume.max_number, KavitaFloat(-100000.0));
		assert_eq!(volume.name, "-100000");
		assert_eq!(volume.chapters.len(), 1);
		let chapter = &volume.chapters[0];
		assert!(chapter.is_special);
		assert_eq!(chapter.range, "Alice's Adventures in Wonderland");
		assert_eq!(chapter.title, "Alice's Adventures in Wonderland");
		assert_eq!(chapter.title_name, "Alice's Adventures in Wonderland");
		assert_eq!(chapter.number, "-100000");
		assert_eq!(chapter.sort_order, KavitaFloat(-100000.0));
		assert_eq!((chapter.count, chapter.total_count), (0, 1));
		assert_eq!(chapter.volume_title, "");
		assert_eq!(chapter.word_count, 0);
		assert_eq!(chapter.publication_status, PublicationStatus::OnGoing);
		assert_eq!(chapter.files[0].file_path, "/library/Series/alice.epub");
		assert_eq!(
			serde_json::to_value(chapter).unwrap()["releaseDate"],
			serde_json::json!("2008-06-27T00:00:00.0000000")
		);

		let detail = map_series_detail(&series, LibraryType::Book);
		assert_eq!(detail.specials.len(), 1);
		assert!(detail.volumes.is_empty());
		assert!(detail.chapters.is_empty());
		assert!(detail.storyline_chapters.is_empty());
		assert_eq!(detail.library_type, LibraryType::Book);
		assert_eq!((detail.unread_count, detail.total_count), (1, 1));

		let metadata = map_series_metadata(&series);
		assert_eq!(metadata.series_id, 40);
		assert_eq!(metadata.publication_status, PublicationStatus::Completed);
		assert_eq!((metadata.max_count, metadata.total_count), (1, 1));
		assert_eq!(metadata.release_year, 2008);
		assert_eq!(metadata.language, "en");
		assert_eq!(metadata.people.writers[0].name, "Lewis Carroll");
		assert_eq!(metadata.genres.len(), 2);
		assert!(metadata.tags.is_empty());
	}

	#[test]
	fn book_without_metadata_is_named_after_its_file() {
		let mut book = input("rust_book", "pdf", 671, 1);
		book.book = true;
		let series = SeriesInput {
			id: 41,
			series: series_model("Collection"),
			metadata: None,
			tags: Vec::new(),
			library_id: 2,
			library_name: "Book Library".to_owned(),
			media: vec![book],
			kind: SeriesKind::Book,
		};
		assert_eq!(map_series(&series).name, "rust_book");
		let chapter = map_chapter(&series.media[0]);
		// kavita-ref `comic_pdf_volumes_2`: `range`/`title` fall back to the
		// file name and `titleName` stays empty.
		assert_eq!(chapter.range, "rust_book");
		assert_eq!(chapter.title, "rust_book");
		assert_eq!(chapter.title_name, "");
		assert!(chapter.is_special);
	}

	#[test]
	fn single_file_volume_shape_matches_kavita() {
		let volume = map_volume(3, &input("Book v02", "cbz", 36, 1));
		assert_eq!(volume.number, 1);
		assert_eq!(volume.min_number, KavitaFloat(1.0));
		assert_eq!(volume.name, "1");
		assert_eq!(volume.series_id, 3);
		assert_eq!(volume.chapters.len(), 1);
		let chapter = &volume.chapters[0];
		assert_eq!(chapter.number, "-100000");
		assert_eq!(chapter.min_number, KavitaFloat(-100000.0));
		assert_eq!(chapter.volume_id, volume.id);
		assert_eq!(chapter.id, volume.id);
		assert_eq!(chapter.files[0].extension, ".cbz");
		assert_eq!(chapter.files[0].format, MangaFormat::Archive);
		assert!(!chapter.cover_image.is_empty());
		assert!(!volume.cover_image.is_empty());
		let json = serde_json::to_value(&volume).unwrap();
		assert_eq!(json["minNumber"], serde_json::json!(1));
		assert_eq!(json["chapters"][0]["minNumber"], serde_json::json!(-100000));
		assert_eq!(json["chapters"][0]["writers"], serde_json::json!([]));
		assert_eq!(
			json["chapters"][0]["languageLocked"],
			serde_json::json!(false)
		);
		assert_eq!(
			json["created"],
			serde_json::json!("2026-09-05T00:30:44.0000000")
		);
	}

	#[test]
	fn volume_number_prefers_metadata_then_ordinal() {
		let mut with_volume = input("a", "cbz", 10, 7);
		with_volume.metadata = Some(media_metadata::Model {
			volume: Some(4),
			number: Some(Decimal::new(12, 0)),
			..default_media_metadata()
		});
		assert_eq!(with_volume.number(), 4);
		let mut with_number = input("b", "cbz", 10, 7);
		with_number.metadata = Some(media_metadata::Model {
			number: Some(Decimal::new(12, 0)),
			..default_media_metadata()
		});
		assert_eq!(with_number.number(), 12);
		let mut fractional = input("c", "cbz", 10, 7);
		fractional.metadata = Some(media_metadata::Model {
			number: Some(Decimal::new(125, 1)),
			..default_media_metadata()
		});
		assert_eq!(fractional.number(), 7);
		assert_eq!(input("d", "cbz", 10, 7).number(), 7);
	}

	#[test]
	fn series_aggregates_pages_and_format() {
		let series = SeriesInput {
			id: 1,
			series: series::Model {
				path: "/library/Alpha".to_owned(),
				..series_model("Alpha")
			},
			metadata: None,
			tags: Vec::new(),
			library_id: 5,
			library_name: "Lib".to_owned(),
			media: vec![input("v1", "epub", 15, 1), input("v2", "epub", 20, 2)],
			kind: SeriesKind::Grouped,
		};
		let dto = map_series(&series);
		assert_eq!(dto.pages, 35);
		assert_eq!(dto.pages_read, 0);
		assert_eq!(dto.format, MangaFormat::Epub);
		assert_eq!(dto.library_id, 5);
		assert_eq!(dto.name, "Alpha");
		assert_eq!(dto.sort_name, "Alpha");
		assert_eq!(dto.cover_image, "v11_c11.png");
		let json = serde_json::to_value(&dto).unwrap();
		assert_eq!(
			json["latestReadDate"],
			serde_json::json!("0001-01-01T00:00:00")
		);
		assert_eq!(json["userRating"], serde_json::json!(0));
		let detail = map_series_detail(&series, LibraryType::Book);
		assert_eq!(detail.total_count, 2);
		assert_eq!(detail.unread_count, 2);
		assert!(detail.specials.is_empty() && detail.chapters.is_empty());
		let metadata = map_series_metadata(&series);
		assert_eq!(metadata.publication_status, PublicationStatus::OnGoing);
		assert_eq!(metadata.series_id, 1);
	}

	#[test]
	fn name_ids_are_stable_and_positive() {
		assert_eq!(name_id("Fantasy"), name_id(" fantasy "));
		assert!(name_id("Fantasy") > 0);
		assert_ne!(name_id("Fantasy"), name_id("Horror"));
		assert_eq!(
			split_csv(Some("Fantasy, Horror, fantasy,")).len(),
			2,
			"csv splitting de-duplicates case-insensitively"
		);
	}

	#[test]
	fn library_types_map_onto_kavita() {
		assert_eq!(library_type(None), LibraryType::Manga);
		let config = library_config::Model {
			library_type: StumpLibraryType::Book,
			..default_library_config()
		};
		assert_eq!(library_type(Some(&config)), LibraryType::Book);
		let config = library_config::Model {
			library_type: StumpLibraryType::Comic,
			..config
		};
		assert_eq!(library_type(Some(&config)), LibraryType::Comic);
	}

	fn grouped_series(media: Vec<MediaInput>) -> SeriesInput {
		SeriesInput {
			id: 1,
			series: series_model("Berserk"),
			metadata: None,
			tags: Vec::new(),
			library_id: 5,
			library_name: "Comics".to_owned(),
			media,
			kind: SeriesKind::Grouped,
		}
	}

	fn names(people: &[PersonDto]) -> Vec<&str> {
		people.iter().map(|person| person.name.as_str()).collect()
	}

	fn titles<'a, T: 'a>(rows: &'a [T], title: fn(&'a T) -> &'a str) -> Vec<&'a str> {
		rows.iter().map(title).collect()
	}

	/// `SeriesMetadataService.UpdateSeriesMetadata` recomputes a series from
	/// every chapter under it: each list is the union of the Stump series row
	/// and every file, first appearance first and de-duplicated
	/// case-insensitively.
	#[test]
	fn series_metadata_unions_the_series_row_and_every_file() {
		let mut first = input("v1", "cbz", 10, 1);
		first.metadata = Some(media_metadata::Model {
			genres: Some("Horror, Action".to_owned()),
			writers: Some("Kentaro Miura, Studio Gaga".to_owned()),
			pencillers: Some("Kentaro Miura".to_owned()),
			inkers: Some("Studio Gaga".to_owned()),
			colorists: Some("Studio Gaga".to_owned()),
			letterers: Some("Duncan Fredoo".to_owned()),
			editors: Some("Dark Horse".to_owned()),
			cover_artists: Some("Kentaro Miura".to_owned()),
			characters: Some("Guts, Griffith".to_owned()),
			teams: Some("Band of the Hawk".to_owned()),
			publisher: Some("Hakusensha".to_owned()),
			age_rating: Some(16),
			year: Some(1990),
			language: Some("ja".to_owned()),
			..default_media_metadata()
		});
		first.tags = vec![TagDto {
			id: 7,
			title: "Seinen".to_owned(),
		}];
		let mut second = input("v2", "cbz", 10, 2);
		second.metadata = Some(media_metadata::Model {
			// `action` repeats the first file's genre in another case.
			genres: Some("action, Dark Fantasy".to_owned()),
			writers: Some("kentaro miura, Ghost Writer".to_owned()),
			age_rating: Some(18),
			year: Some(1989),
			language: Some("en".to_owned()),
			..default_media_metadata()
		});
		second.tags = vec![
			TagDto {
				id: 7,
				title: "seinen".to_owned(),
			},
			TagDto {
				id: 9,
				title: "Long Running".to_owned(),
			},
		];
		let mut series = grouped_series(vec![first, second]);
		series.metadata = Some(series_metadata::Model {
			genres: Some("Fantasy".to_owned()),
			writers: Some("Kentaro Miura".to_owned()),
			characters: Some("Casca".to_owned()),
			imprint: Some("Jets Comics".to_owned()),
			total_issues: Some(41),
			status: Some("Ended".to_owned()),
			links: Some("https://example.test/berserk".to_owned()),
			..default_series_metadata()
		});
		series.tags = vec![TagDto {
			id: 3,
			title: "Classic".to_owned(),
		}];

		let dto = map_series_metadata(&series);
		assert_eq!(
			titles(&dto.genres, |genre| genre.title.as_str()),
			["Fantasy", "Horror", "Action", "Dark Fantasy"],
			"series row first, then each file in order, one entry per value"
		);
		assert_eq!(
			names(&dto.people.writers),
			["Kentaro Miura", "Studio Gaga", "Ghost Writer"]
		);
		assert_eq!(
			titles(&dto.tags, |tag| tag.title.as_str()),
			["Classic", "Seinen", "Long Running"]
		);
		assert_eq!(
			dto.tags.iter().map(|tag| tag.id).collect::<Vec<_>>(),
			[3, 7, 9],
			"tag ids stay the Stump row ids the filter round-trips"
		);
		assert_eq!(names(&dto.people.characters), ["Casca", "Guts", "Griffith"]);
		assert_eq!(names(&dto.people.imprints), ["Jets Comics"]);
		assert_eq!(names(&dto.people.publishers), ["Hakusensha"]);
		assert_eq!(names(&dto.people.pencillers), ["Kentaro Miura"]);
		assert_eq!(names(&dto.people.inkers), ["Studio Gaga"]);
		assert_eq!(names(&dto.people.colorists), ["Studio Gaga"]);
		assert_eq!(names(&dto.people.letterers), ["Duncan Fredoo"]);
		assert_eq!(names(&dto.people.editors), ["Dark Horse"]);
		assert_eq!(names(&dto.people.cover_artists), ["Kentaro Miura"]);
		assert_eq!(names(&dto.people.teams), ["Band of the Hawk"]);
		assert!(
			dto.people.translators.is_empty() && dto.people.locations.is_empty(),
			"Stump records neither role"
		);
		assert_eq!(dto.people.writers[0].roles, vec![PersonRole::Writer]);
		// The series row carries none of these, so the files decide.
		assert_eq!(
			dto.age_rating,
			AgeRating::R18Plus,
			"the strictest file wins"
		);
		assert_eq!(
			dto.release_year, 1989,
			"the earliest file starts the series"
		);
		assert_eq!(dto.language, "ja", "the first file that names one");
		assert_eq!((dto.max_count, dto.total_count), (41, 41));
		assert_eq!(dto.publication_status, PublicationStatus::Ended);
		assert_eq!(dto.web_links, "https://example.test/berserk");
	}

	/// A book series is one file presented as its own series, so its tags are
	/// that file's tags.
	#[test]
	fn book_series_metadata_carries_its_files_tags() {
		let mut book = input("alice", "epub", 15, 1);
		book.book = true;
		book.tags = vec![
			TagDto {
				id: 4,
				title: "Public Domain".to_owned(),
			},
			TagDto {
				id: 4,
				title: "public domain".to_owned(),
			},
		];
		book.metadata = Some(media_metadata::Model {
			genres: Some("Fantasy fiction".to_owned()),
			..default_media_metadata()
		});
		let mut series = grouped_series(vec![book]);
		series.kind = SeriesKind::Book;
		// The Stump series row is only the folder the file sits in.
		series.tags = vec![TagDto {
			id: 8,
			title: "Folder Tag".to_owned(),
		}];

		let dto = map_series_metadata(&series);
		assert_eq!(
			titles(&dto.tags, |tag| tag.title.as_str()),
			["Public Domain"]
		);
		let chapter = map_chapter(&series.media[0]);
		assert_eq!(
			titles(&chapter.tags, |tag| tag.title.as_str()),
			["Public Domain"]
		);
	}

	/// Stump's `locked_fields` are `MetadataField` names; Kavita flattens one
	/// `*Locked` flag per field into the metadata DTOs.
	#[test]
	fn locked_fields_map_onto_kavita_locks() {
		let locked = serde_json::json!([
			"SUMMARY",
			"GENRES",
			"TAGS",
			"AGE_RATING",
			"STATUS",
			"YEAR",
			"WRITERS",
			"CHARACTERS",
			"COLORISTS",
			"EDITORS",
			"INKERS",
			"IMPRINT",
			"LETTERERS",
			"PENCILLERS",
			"PUBLISHER",
			"TEAMS",
			"COVER_ARTISTS",
		]);
		let mut series = grouped_series(vec![input("v1", "cbz", 10, 1)]);
		series.metadata = Some(series_metadata::Model {
			locked_fields: Some(locked.clone()),
			language_lock: true,
			..default_series_metadata()
		});

		let dto = map_series_metadata(&series);
		assert_eq!(
			dto.locks,
			MetadataLocksDto {
				language_locked: true,
				summary_locked: true,
				age_rating_locked: true,
				publication_status_locked: true,
				genres_locked: true,
				tags_locked: true,
				writer_locked: true,
				character_locked: true,
				colorist_locked: true,
				editor_locked: true,
				inker_locked: true,
				imprint_locked: true,
				letterer_locked: true,
				penciller_locked: true,
				publisher_locked: true,
				// Stump has no translator or location metadata to lock.
				translator_locked: false,
				team_locked: true,
				location_locked: false,
				cover_artist_locked: true,
			}
		);
		assert!(dto.release_year_locked);

		// The series' `language_lock` column is the only lock that does not
		// come from `locked_fields`.
		let mut unlocked = grouped_series(vec![input("v1", "cbz", 10, 1)]);
		unlocked.metadata = Some(series_metadata::Model {
			locked_fields: Some(serde_json::json!("not an array")),
			..default_series_metadata()
		});
		let dto = map_series_metadata(&unlocked);
		assert_eq!(dto.locks, MetadataLocksDto::default());
		assert!(!dto.release_year_locked);

		// A chapter locks the same fields from its own file's row, plus the
		// three flags Kavita keeps only on a chapter.
		let mut media = input("v1", "cbz", 10, 1);
		media.metadata = Some(media_metadata::Model {
			locked_fields: Some(serde_json::json!([
				"GENRES",
				"TITLE",
				"RELEASE_DATE",
				"NUMBER"
			])),
			..default_media_metadata()
		});
		let chapter = map_chapter(&media);
		assert!(chapter.locks.genres_locked);
		assert!(!chapter.locks.tags_locked);
		assert!(chapter.title_name_locked);
		assert!(chapter.release_date_locked);
		assert!(chapter.sort_order_locked);
	}

	/// The serialized key sets `kavita-ref` 0.9.1.4 answers
	/// `GET /api/Series/chapter?chapterId=` and `GET /api/Series/metadata?seriesId=`
	/// with: clients deserialize these into required fields, so a missing key
	/// is a client failure and an extra key is not Kavita.
	#[test]
	fn metadata_dto_field_sets_match_kavita_ref() {
		const CHAPTER_KEYS: [&str; 82] = [
			"ageRating",
			"ageRatingLocked",
			"aniListId",
			"avgHoursToRead",
			"cbrId",
			"characterLocked",
			"characters",
			"coloristLocked",
			"colorists",
			"comicVineId",
			"count",
			"coverArtistLocked",
			"coverArtists",
			"coverImage",
			"coverImageLocked",
			"created",
			"createdUtc",
			"editorLocked",
			"editors",
			"files",
			"format",
			"genres",
			"genresLocked",
			"hardcoverId",
			"id",
			"imprintLocked",
			"imprints",
			"inkerLocked",
			"inkers",
			"isSpecial",
			"isbn",
			"language",
			"languageLocked",
			"lastModifiedUtc",
			"lastReadingProgress",
			"lastReadingProgressUtc",
			"lettererLocked",
			"letterers",
			"locationLocked",
			"locations",
			"malId",
			"mangaBakaId",
			"maxHoursToRead",
			"maxNumber",
			"metronId",
			"minHoursToRead",
			"minNumber",
			"number",
			"pages",
			"pagesRead",
			"pencillerLocked",
			"pencillers",
			"primaryColor",
			"publicationStatus",
			"publicationStatusLocked",
			"publisherLocked",
			"publishers",
			"range",
			"releaseDate",
			"releaseDateLocked",
			"secondaryColor",
			"sortOrder",
			"sortOrderLocked",
			"summary",
			"summaryLocked",
			"tags",
			"tagsLocked",
			"teamLocked",
			"teams",
			"title",
			"titleName",
			"titleNameLocked",
			"totalCount",
			"totalReads",
			"translatorLocked",
			"translators",
			"volumeId",
			"volumeTitle",
			"webLinks",
			"wordCount",
			"writerLocked",
			"writers",
		];
		const SERIES_METADATA_KEYS: [&str; 45] = [
			"ageRating",
			"ageRatingLocked",
			"characterLocked",
			"characters",
			"coloristLocked",
			"colorists",
			"coverArtistLocked",
			"coverArtists",
			"editorLocked",
			"editors",
			"genres",
			"genresLocked",
			"id",
			"imprintLocked",
			"imprints",
			"inkerLocked",
			"inkers",
			"language",
			"languageLocked",
			"lettererLocked",
			"letterers",
			"locationLocked",
			"locations",
			"maxCount",
			"pencillerLocked",
			"pencillers",
			"publicationStatus",
			"publicationStatusLocked",
			"publisherLocked",
			"publishers",
			"releaseYear",
			"releaseYearLocked",
			"seriesId",
			"summary",
			"summaryLocked",
			"tags",
			"tagsLocked",
			"teamLocked",
			"teams",
			"totalCount",
			"translatorLocked",
			"translators",
			"webLinks",
			"writerLocked",
			"writers",
		];

		let keys = |value: serde_json::Value| {
			value
				.as_object()
				.expect("DTO serializes as an object")
				.keys()
				.cloned()
				.collect::<Vec<_>>()
		};
		let series = grouped_series(vec![input("v1", "cbz", 10, 1)]);
		assert_eq!(
			keys(serde_json::to_value(map_chapter(&series.media[0])).unwrap()),
			CHAPTER_KEYS
		);
		assert_eq!(
			keys(serde_json::to_value(map_series_metadata(&series)).unwrap()),
			SERIES_METADATA_KEYS
		);
	}

	fn default_series_metadata() -> series_metadata::Model {
		series_metadata::Model {
			series_id: "series-1".to_owned(),
			age_rating: None,
			characters: None,
			booktype: None,
			collects: None,
			comicid: None,
			comic_image: None,
			description_formatted: None,
			genres: None,
			imprint: None,
			links: None,
			meta_type: None,
			publication_run: None,
			publisher: None,
			status: None,
			summary: None,
			title: None,
			title_sort: None,
			reading_direction: None,
			language: None,
			alternate_titles: None,
			title_sort_lock: false,
			reading_direction_lock: false,
			language_lock: false,
			alternate_titles_lock: false,
			total_issues: None,
			volume: None,
			writers: None,
			year: None,
			metadata_source: None,
			metadata_external_id: None,
			locked_fields: None,
		}
	}

	fn default_media_metadata() -> media_metadata::Model {
		media_metadata::Model {
			id: 1,
			media_id: Some("media".to_owned()),
			age_rating: None,
			characters: None,
			colorists: None,
			cover_artists: None,
			format: None,
			day: None,
			editors: None,
			genres: None,
			identifier_amazon: None,
			identifier_calibre: None,
			identifier_google: None,
			identifier_isbn: None,
			identifier_mobi_asin: None,
			identifier_uuid: None,
			inkers: None,
			language: None,
			letterers: None,
			links: None,
			month: None,
			notes: None,
			number: None,
			page_count: None,
			pencillers: None,
			publisher: None,
			series: None,
			series_group: None,
			story_arc: None,
			story_arc_number: None,
			summary: None,
			teams: None,
			title: None,
			title_sort: None,
			volume: None,
			writers: None,
			year: None,
			metadata_source: None,
			metadata_external_id: None,
			locked_fields: None,
		}
	}

	fn default_library_config() -> library_config::Model {
		library_config::Model {
			id: 1,
			convert_rar_to_zip: false,
			hard_delete_conversions: false,
			default_reading_dir: Default::default(),
			default_reading_mode: Default::default(),
			default_reading_image_scale_fit: Default::default(),
			generate_file_hashes: false,
			generate_koreader_hashes: false,
			process_metadata: true,
			watch: false,
			library_pattern: Default::default(),
			default_library_view_mode: Default::default(),
			hide_series_view: false,
			library_type: Default::default(),
			skip_book_overview: false,
			thumbnail_config: None,
			process_thumbnail_colors_even_without_config: false,
			ignore_rules: None,
			library_id: Some("lib".to_owned()),
			metadata_policy: None,
		}
	}
}
