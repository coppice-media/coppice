//! Stump model → Kavita DTO mapping.
//!
//! Model: one Stump series is one Kavita series; every Stump media item is a
//! Kavita volume holding exactly one chapter, the shape Kavita itself builds
//! for a single-file volume (`chapter.number == "-100000"`,
//! `Parser.DefaultChapterNumber`). Volume and chapter share the media's
//! Kavita id. Numbers come from the media metadata volume, then an integral
//! metadata number, then the media's ordinal in the series.

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use models::{
	entity::{
		library, library_config, media, media_metadata, reading_session, series,
		series_metadata,
	},
	shared::enums::LibraryType as StumpLibraryType,
};

use crate::{
	dto::{
		AgeRating, ChapterDto, FileTypeGroup, GenreTagDto, KavitaDateTime, KavitaFloat,
		LibraryDto, LibraryType, MangaFileDto, MangaFormat, MetadataLocksDto, PeopleDto,
		PersonDto, PersonRole, PublicationStatus, SeriesDetailDto, SeriesDto,
		SeriesMetadataDto, TagDto, VolumeDto,
	},
	progress::{last_progress_at, pages_read},
};

/// `Parser.DefaultChapterNumber`: the chapter number of a single-file volume.
pub const DEFAULT_CHAPTER_NUMBER: f32 = -100_000.0;
pub const DEFAULT_CHAPTER: &str = "-100000";

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

pub fn people(raw: Option<&str>, role: PersonRole) -> Vec<PersonDto> {
	split_csv(raw)
		.into_iter()
		.map(|name| PersonDto::new(name_id(&name), name, vec![role]))
		.collect()
}

pub fn genres(raw: Option<&str>) -> Vec<GenreTagDto> {
	split_csv(raw)
		.into_iter()
		.map(|title| GenreTagDto {
			id: name_id(&title),
			title,
		})
		.collect()
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
	/// Position of the media in the series (1-based, name order).
	pub ordinal: i32,
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

pub fn map_chapter(input: &MediaInput) -> ChapterDto {
	let metadata = input.metadata.as_ref();
	let pages = input.pages();
	let pages_read = input.pages_read();
	let last_progress: KavitaDateTime = last_progress_at(input.session.as_ref()).into();
	let title = metadata
		.and_then(|metadata| metadata.title.clone())
		.filter(|title| !title.trim().is_empty());
	let number = input.number();
	ChapterDto {
		id: input.id,
		range: input.media.name.clone(),
		number: DEFAULT_CHAPTER.to_owned(),
		min_number: KavitaFloat(DEFAULT_CHAPTER_NUMBER),
		max_number: KavitaFloat(DEFAULT_CHAPTER_NUMBER),
		sort_order: KavitaFloat(number as f32),
		pages,
		is_special: false,
		title: title.clone().unwrap_or_else(|| input.media.name.clone()),
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
		people: PeopleDto {
			writers: people(
				metadata.and_then(|m| m.writers.as_deref()),
				PersonRole::Writer,
			),
			cover_artists: people(
				metadata.and_then(|m| m.cover_artists.as_deref()),
				PersonRole::CoverArtist,
			),
			publishers: people(
				metadata.and_then(|m| m.publisher.as_deref()),
				PersonRole::Publisher,
			),
			characters: people(
				metadata.and_then(|m| m.characters.as_deref()),
				PersonRole::Character,
			),
			pencillers: people(
				metadata.and_then(|m| m.pencillers.as_deref()),
				PersonRole::Penciller,
			),
			inkers: people(
				metadata.and_then(|m| m.inkers.as_deref()),
				PersonRole::Inker,
			),
			imprints: Vec::new(),
			colorists: people(
				metadata.and_then(|m| m.colorists.as_deref()),
				PersonRole::Colorist,
			),
			letterers: people(
				metadata.and_then(|m| m.letterers.as_deref()),
				PersonRole::Letterer,
			),
			editors: people(
				metadata.and_then(|m| m.editors.as_deref()),
				PersonRole::Editor,
			),
			translators: Vec::new(),
			teams: people(metadata.and_then(|m| m.teams.as_deref()), PersonRole::Team),
			locations: Vec::new(),
		},
		genres: genres(metadata.and_then(|metadata| metadata.genres.as_deref())),
		tags: Vec::new(),
		publication_status: PublicationStatus::OnGoing,
		language: metadata.and_then(|metadata| metadata.language.clone()),
		count: 0,
		total_count: 0,
		locks: MetadataLocksDto::default(),
		release_date_locked: false,
		title_name_locked: false,
		sort_order_locked: false,
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

/// A series together with the media that back its volumes.
#[derive(Debug, Clone)]
pub struct SeriesInput {
	pub id: i32,
	pub series: series::Model,
	pub metadata: Option<series_metadata::Model>,
	pub library_id: i32,
	pub library_name: String,
	pub media: Vec<MediaInput>,
}

impl SeriesInput {
	pub fn name(&self) -> String {
		self.metadata
			.as_ref()
			.and_then(|metadata| metadata.title.clone())
			.filter(|title| !title.trim().is_empty())
			.unwrap_or_else(|| self.series.name.clone())
	}

	pub fn sort_name(&self) -> String {
		self.metadata
			.as_ref()
			.and_then(|metadata| metadata.title_sort.clone())
			.filter(|title| !title.trim().is_empty())
			.unwrap_or_else(|| self.name())
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
	SeriesDto {
		id: input.id,
		name: name.clone(),
		original_name: input.series.name.clone(),
		localized_name: String::new(),
		sort_name: input.sort_name(),
		pages: input.pages(),
		cover_image_locked: input.series.thumbnail_path.is_some(),
		last_chapter_added: input.last_chapter_added(),
		last_chapter_added_utc: input.last_chapter_added(),
		user_rating: KavitaFloat(0.0),
		has_user_rated: false,
		total_reads: 0,
		pages_read: input.pages_read(),
		latest_read_date: input.latest_read(),
		format: input.format(),
		created: input.series.created_at.into(),
		sort_name_locked: input
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
		folder_path: input.series.path.clone(),
		lowest_folder_path: input.series.path.clone(),
		last_folder_scanned: input
			.series
			.updated_at
			.or(Some(input.series.created_at))
			.into(),
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

pub fn map_series_metadata(input: &SeriesInput, tags: Vec<TagDto>) -> SeriesMetadataDto {
	let metadata = input.metadata.as_ref();
	let first_media = input
		.media
		.first()
		.and_then(|media| media.metadata.as_ref());
	let language = metadata
		.and_then(|metadata| metadata.language.clone())
		.or_else(|| first_media.and_then(|metadata| metadata.language.clone()))
		.unwrap_or_default();
	SeriesMetadataDto {
		id: input.id,
		summary: metadata
			.and_then(|metadata| metadata.summary.clone())
			.or_else(|| input.series.description.clone())
			.unwrap_or_default(),
		genres: genres(metadata.and_then(|metadata| metadata.genres.as_deref())),
		tags,
		people: PeopleDto {
			writers: people(
				metadata.and_then(|m| m.writers.as_deref()),
				PersonRole::Writer,
			),
			cover_artists: Vec::new(),
			publishers: people(
				metadata.and_then(|m| m.publisher.as_deref()),
				PersonRole::Publisher,
			),
			characters: people(
				metadata.and_then(|m| m.characters.as_deref()),
				PersonRole::Character,
			),
			pencillers: Vec::new(),
			inkers: Vec::new(),
			imprints: people(
				metadata.and_then(|m| m.imprint.as_deref()),
				PersonRole::Imprint,
			),
			colorists: Vec::new(),
			letterers: Vec::new(),
			editors: Vec::new(),
			translators: Vec::new(),
			teams: Vec::new(),
			locations: Vec::new(),
		},
		age_rating: AgeRating::from_min_age(
			metadata.and_then(|metadata| metadata.age_rating),
		),
		release_year: metadata.and_then(|metadata| metadata.year).unwrap_or(0),
		language,
		max_count: metadata
			.and_then(|metadata| metadata.total_issues)
			.unwrap_or(0),
		total_count: metadata
			.and_then(|metadata| metadata.total_issues)
			.unwrap_or(0),
		publication_status: PublicationStatus::from_status_text(
			metadata.and_then(|metadata| metadata.status.as_deref()),
		),
		web_links: metadata
			.and_then(|metadata| metadata.links.clone())
			.unwrap_or_default(),
		locks: MetadataLocksDto {
			language_locked: metadata.is_some_and(|metadata| metadata.language_lock),
			..MetadataLocksDto::default()
		},
		release_year_locked: false,
		series_id: input.id,
	}
}

pub fn map_series_detail(
	input: &SeriesInput,
	library_type: LibraryType,
) -> SeriesDetailDto {
	let volumes = input
		.media
		.iter()
		.map(|media| map_volume(input.id, media))
		.collect::<Vec<_>>();
	let total_count = i32::try_from(volumes.len()).unwrap_or(i32::MAX);
	let unread_count = volumes
		.iter()
		.filter(|volume| volume.pages_read < volume.pages || volume.pages == 0)
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
			session: None,
			ordinal,
		}
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
				id: "series-1".to_owned(),
				name: "Alpha".to_owned(),
				description: None,
				created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap().into(),
				updated_at: None,
				deleted_at: None,
				path: "/library/Alpha".to_owned(),
				status: FileStatus::Ready,
				thumbnail_meta: None,
				thumbnail_path: None,
				library_id: Some("lib".to_owned()),
				source_provider: None,
				remote_id: None,
			},
			metadata: None,
			library_id: 5,
			library_name: "Lib".to_owned(),
			media: vec![input("v1", "epub", 15, 1), input("v2", "epub", 20, 2)],
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
		let metadata = map_series_metadata(&series, Vec::new());
		assert_eq!(metadata.publication_status, PublicationStatus::OnGoing);
		assert_eq!(metadata.series_id, 1);
	}

	#[test]
	fn name_ids_are_stable_and_positive() {
		assert_eq!(name_id("Fantasy"), name_id(" fantasy "));
		assert!(name_id("Fantasy") > 0);
		assert_ne!(name_id("Fantasy"), name_id("Horror"));
		assert_eq!(
			genres(Some("Fantasy, Horror, fantasy,")).len(),
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
		}
	}
}
