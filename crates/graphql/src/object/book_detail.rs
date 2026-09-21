//! Merged, user-scoped book detail objects.
//!
//! A detail page is anchored by one visible media id. Confirmed liseur links
//! resolve the work internally and keep ebook/audiobook rows distinct; an
//! unpaired media row is represented as a one-edition work with no public work
//! identity. Search, review, history, and read-aloud readiness all use the same
//! visibility and pairing rules.

use std::collections::BTreeMap;

use async_graphql::{Enum, Json, SimpleObject, ID};
use metadata_integrations::MetadataField;
use models::{
	domain::edition_pair::{self, PairEvidence, PairStatus},
	entity::{
		book_review, book_work_metadata, device, liseur_sync_media_link,
		liseur_sync_work, media, media_audio, media_audio_chapter, media_metadata,
		reading_head, reading_session, user::AuthUser,
	},
	shared::{
		enums::{DeviceKind, FileStatus},
		readium::ReadiumLocator,
	},
};
use num_traits::ToPrimitive;
use sea_orm::{prelude::*, QueryOrder};
use serde_json::Value;
use stump_library::{editions, sync_maps};
use stump_worker::AlignGranularity;

use crate::{
	data::CoreContext,
	object::{
		edition_pair::ChapterMapEntry,
		media::Media,
		media_metadata::MediaMetadata,
		sync_map::{ReadAloudArtifact, SyncMap},
	},
};

/// The explicit metadata destination selected by a book-detail operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum BookMetadataScope {
	Work,
	Ebook,
	Audiobook,
	Both,
}

/// Distinct lanes in a merged work view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum BookEditionKind {
	Ebook,
	Audiobook,
	Other,
}

/// Cache/readiness states for an accepted read-aloud artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum BookReadAloudStatus {
	NoAudiobookPaired,
	Unavailable,
	ChapterMapUnavailable,
	SyncMapUnavailable,
	CacheMissing,
	Ready,
	Failed,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookFile {
	pub path: String,
	pub size: i64,
	pub extension: String,
	pub hash: Option<String>,
	pub koreader_hash: Option<String>,
	pub status: FileStatus,
	pub modified_at: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookAudioChapter {
	pub id: ID,
	pub index: i32,
	pub title: Option<String>,
	pub start_ms: i64,
	pub end_ms: Option<i64>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookAudioFacts {
	pub duration_ms: i64,
	pub codec: String,
	pub sample_rate: Option<i32>,
	pub channels: Option<i32>,
	pub bitrate: Option<i32>,
	pub chapter_source: String,
	pub chapters: Vec<BookAudioChapter>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookEdition {
	pub media_id: ID,
	pub kind: BookEditionKind,
	pub title: String,
	pub media: Media,
	pub metadata: Option<MediaMetadata>,
	pub file: BookFile,
	pub audio: Option<BookAudioFacts>,
	pub pair_evidence: Option<PairEvidence>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookWorkMetadata {
	pub work_id: ID,
	pub title: Option<String>,
	pub author: Option<String>,
	pub metadata: Json<Value>,
	pub locked_fields: Vec<MetadataField>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookMetadataMismatch {
	pub field: MetadataField,
	pub ebook_value: Option<String>,
	pub audiobook_value: Option<String>,
	pub work_value: Option<String>,
	pub resolved: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookReadAloud {
	pub status: BookReadAloudStatus,
	pub reason: Option<String>,
	pub ebook_media_id: Option<ID>,
	pub audio_media_id: Option<ID>,
	pub chapter_map: Vec<ChapterMapEntry>,
	pub sync_map: Option<SyncMap>,
	pub artifact: Option<ReadAloudArtifact>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookDetail {
	pub media_id: ID,
	pub work_id: Option<ID>,
	pub title: String,
	pub authors: Vec<String>,
	pub work_metadata: Option<BookWorkMetadata>,
	pub editions: Vec<BookEdition>,
	pub mismatches: Vec<BookMetadataMismatch>,
	pub review: Option<super::book_review::BookReview>,
	pub read_aloud: BookReadAloud,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookReadingDevice {
	pub id: ID,
	pub name: Option<String>,
	pub kind: Option<DeviceKind>,
	pub revoked: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookReadingHead {
	pub locator: Option<ReadiumLocator>,
	pub progression: f64,
	pub page: Option<i32>,
	pub position_ms: Option<i64>,
	pub track_index: Option<i32>,
	pub completed: bool,
	pub updated_at: String,
	pub source_protocol: String,
	pub source_device_id: Option<ID>,
	pub source_device: Option<BookReadingDevice>,
	pub revision: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookReadingSession {
	pub id: ID,
	pub session_date: String,
	pub status: String,
	pub readthrough_number: i32,
	pub start_locator: Option<ReadiumLocator>,
	pub end_locator: Option<ReadiumLocator>,
	pub start_page: Option<i32>,
	pub end_page: Option<i32>,
	pub end_position_ms: Option<i64>,
	pub start_percentage: Option<f64>,
	pub end_percentage: Option<f64>,
	pub elapsed_seconds: Option<i64>,
	pub notes: Option<String>,
	pub source_protocol: String,
	pub source_device_ids: Vec<ID>,
	pub source_devices: Vec<BookReadingDevice>,
	pub liseur_session_id: Option<ID>,
	pub created_at: String,
	pub updated_at: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookReadingEdition {
	pub media_id: ID,
	pub kind: BookEditionKind,
	pub head: Option<BookReadingHead>,
	pub sessions: Vec<BookReadingSession>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookReadingLog {
	pub media_id: ID,
	pub work_id: Option<ID>,
	pub editions: Vec<BookReadingEdition>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct BookSearchResult {
	pub media_id: ID,
	pub work_id: Option<ID>,
	pub title: String,
	pub authors: Vec<String>,
	pub kind: BookEditionKind,
	pub score: f64,
}

impl BookEditionKind {
	pub(crate) fn from_model(model: &media::Model, audio: bool) -> Self {
		let extension = model.extension.to_ascii_lowercase();
		if audio
			|| media::AUDIO_EXTENSIONS
				.iter()
				.any(|value| value.eq_ignore_ascii_case(&extension))
		{
			Self::Audiobook
		} else if extension == "epub" {
			Self::Ebook
		} else {
			Self::Other
		}
	}
}

impl From<&media::Model> for BookFile {
	fn from(model: &media::Model) -> Self {
		Self {
			path: model.path.clone(),
			size: model.size,
			extension: model.extension.clone(),
			hash: model.hash.clone(),
			koreader_hash: model.koreader_hash.clone(),
			status: model.status,
			modified_at: model.modified_at.map(|value| value.to_rfc3339()),
		}
	}
}

impl From<book_review::Model> for super::book_review::BookReview {
	fn from(model: book_review::Model) -> Self {
		Self {
			id: ID::from(model.id),
			work_id: model.work_id.map(ID::from),
			media_id: model.media_id.map(ID::from),
			rating: model.rating,
			content: model.content,
			is_private: model.is_private,
			created_at: model.created_at.to_rfc3339(),
			updated_at: model.updated_at.to_rfc3339(),
		}
	}
}

impl From<&device::Model> for BookReadingDevice {
	fn from(model: &device::Model) -> Self {
		Self {
			id: ID::from(model.id.clone()),
			name: Some(model.name.clone()),
			kind: Some(model.kind),
			revoked: model.is_revoked(),
		}
	}
}

pub(crate) fn metadata_title(
	metadata: Option<&media_metadata::Model>,
	fallback: &str,
) -> String {
	metadata
		.and_then(|metadata| metadata.title.clone())
		.filter(|title| !title.trim().is_empty())
		.unwrap_or_else(|| fallback.to_owned())
}

pub(crate) fn metadata_authors(metadata: Option<&media_metadata::Model>) -> Vec<String> {
	metadata
		.and_then(|metadata| metadata.writers.as_deref())
		.map(|writers| {
			writers
				.split(',')
				.map(str::trim)
				.filter(|value| !value.is_empty())
				.map(str::to_owned)
				.collect()
		})
		.unwrap_or_default()
}

fn metadata_value(
	metadata: Option<&media_metadata::Model>,
	field: MetadataField,
) -> Option<String> {
	let metadata = metadata?;
	let value = match field {
		MetadataField::Title => metadata.title.clone(),
		MetadataField::Summary => metadata.summary.clone(),
		MetadataField::Genres => metadata.genres.clone(),
		MetadataField::Publisher => metadata.publisher.clone(),
		MetadataField::Year => metadata.year.map(|value| value.to_string()),
		MetadataField::PageCount => metadata.page_count.map(|value| value.to_string()),
		MetadataField::Isbn => metadata.identifier_isbn.clone(),
		MetadataField::Writers => metadata.writers.clone(),
		MetadataField::Narrators => metadata.narrators.clone(),
		MetadataField::Language => metadata.language.clone(),
		MetadataField::Series => metadata.series.clone(),
		MetadataField::SeriesGroup => metadata.series_group.clone(),
		MetadataField::TitleSort => metadata.title_sort.clone(),
		MetadataField::Notes => metadata.notes.clone(),
		MetadataField::Format => metadata.format.clone(),
		MetadataField::Number => metadata.number.map(|value| value.to_string()),
		MetadataField::VolumeCount => metadata.volume.map(|value| value.to_string()),
		MetadataField::AgeRating => metadata.age_rating.map(|value| value.to_string()),
		MetadataField::IdentifierAmazon => metadata.identifier_amazon.clone(),
		MetadataField::IdentifierCalibre => metadata.identifier_calibre.clone(),
		MetadataField::IdentifierGoogle => metadata.identifier_google.clone(),
		MetadataField::IdentifierMobiAsin => metadata.identifier_mobi_asin.clone(),
		MetadataField::IdentifierUuid => metadata.identifier_uuid.clone(),
		MetadataField::Links => metadata.links.clone(),
		MetadataField::Characters => metadata.characters.clone(),
		MetadataField::Colorists => metadata.colorists.clone(),
		MetadataField::CoverArtists => metadata.cover_artists.clone(),
		MetadataField::Editors => metadata.editors.clone(),
		MetadataField::Inkers => metadata.inkers.clone(),
		MetadataField::Letterers => metadata.letterers.clone(),
		MetadataField::Pencillers => metadata.pencillers.clone(),
		MetadataField::Teams => metadata.teams.clone(),
		_ => None,
	};
	value.filter(|value| !value.trim().is_empty())
}

fn work_value(
	work: Option<&liseur_sync_work::Model>,
	metadata: Option<&book_work_metadata::Model>,
	field: MetadataField,
) -> Option<String> {
	match field {
		MetadataField::Title => metadata
			.and_then(|metadata| metadata.title.clone())
			.or_else(|| work.map(|work| work.title.clone())),
		MetadataField::Writers => metadata
			.and_then(|metadata| metadata.author.clone())
			.or_else(|| work.map(|work| work.author.clone())),
		_ => metadata.and_then(|metadata| {
			metadata
				.metadata
				.as_ref()
				.and_then(|value| {
					value
						.get(metadata_field_name(field))
						.and_then(Value::as_str)
				})
				.map(str::to_owned)
		}),
	}
	.filter(|value| !value.trim().is_empty())
}

pub(crate) fn metadata_field_name(field: MetadataField) -> &'static str {
	match field {
		MetadataField::Title => "title",
		MetadataField::Summary => "summary",
		MetadataField::Genres => "genres",
		MetadataField::Tags => "tags",
		MetadataField::Artists => "artists",
		MetadataField::Publisher => "publisher",
		MetadataField::Year => "year",
		MetadataField::AgeRating => "age_rating",
		MetadataField::Cover => "cover",
		MetadataField::Status => "status",
		MetadataField::VolumeCount => "volume_count",
		MetadataField::PageCount => "page_count",
		MetadataField::Isbn => "isbn",
		MetadataField::ReleaseDate => "release_date",
		MetadataField::Colorists => "colorists",
		MetadataField::Letterers => "letterers",
		MetadataField::CoverArtists => "cover_artists",
		MetadataField::Writers => "writers",
		MetadataField::Format => "format",
		MetadataField::TitleSort => "title_sort",
		MetadataField::Number => "number",
		MetadataField::Series => "series",
		MetadataField::SeriesGroup => "series_group",
		MetadataField::Notes => "notes",
		MetadataField::Language => "language",
		MetadataField::Editors => "editors",
		MetadataField::Inkers => "inkers",
		MetadataField::Teams => "teams",
		MetadataField::Links => "links",
		MetadataField::Characters => "characters",
		MetadataField::StoryArc => "story_arc",
		MetadataField::StoryArcNumber => "story_arc_number",
		MetadataField::BookType => "book_type",
		MetadataField::Imprint => "imprint",
		MetadataField::PublicationRun => "publication_run",
		MetadataField::Pencillers => "pencillers",
		MetadataField::IdentifierAmazon => "identifier_amazon",
		MetadataField::IdentifierCalibre => "identifier_calibre",
		MetadataField::IdentifierGoogle => "identifier_google",
		MetadataField::IdentifierMobiAsin => "identifier_mobi_asin",
		MetadataField::IdentifierUuid => "identifier_uuid",
		MetadataField::ComicId => "comic_id",
		MetadataField::MetaType => "meta_type",
		MetadataField::ComicImage => "comic_image",
		MetadataField::DescriptionFormatted => "description_formatted",
		MetadataField::Narrators => "narrators",
		MetadataField::Subtitle => "subtitle",
	}
}

const MISMATCH_FIELDS: [MetadataField; 10] = [
	MetadataField::Title,
	MetadataField::Summary,
	MetadataField::Genres,
	MetadataField::Publisher,
	MetadataField::Year,
	MetadataField::Isbn,
	MetadataField::Writers,
	MetadataField::Narrators,
	MetadataField::Language,
	MetadataField::Series,
];

fn mismatch_rows(
	editions: &[BookEdition],
	work: Option<&liseur_sync_work::Model>,
	work_metadata: Option<&book_work_metadata::Model>,
) -> Vec<BookMetadataMismatch> {
	let ebook = editions
		.iter()
		.find(|edition| edition.kind == BookEditionKind::Ebook);
	let audio = editions
		.iter()
		.find(|edition| edition.kind == BookEditionKind::Audiobook);
	MISMATCH_FIELDS
		.iter()
		.filter_map(|field| {
			let ebook_value = ebook.and_then(|edition| {
				edition
					.metadata
					.as_ref()
					.and_then(|metadata| metadata_value(Some(&metadata.model), *field))
			});
			let audiobook_value = audio.and_then(|edition| {
				edition
					.metadata
					.as_ref()
					.and_then(|metadata| metadata_value(Some(&metadata.model), *field))
			});
			let work_value = work_value(work, work_metadata, *field);
			let has_difference = match (&ebook_value, &audiobook_value) {
				(Some(left), Some(right)) => left != right,
				(None, None) => false,
				_ => true,
			};
			has_difference.then_some(BookMetadataMismatch {
				field: *field,
				ebook_value,
				audiobook_value,
				work_value,
				resolved: false,
			})
		})
		.collect()
}

async fn build_edition(
	conn: &DatabaseConnection,
	model: media::ModelWithMetadata,
	link: Option<&liseur_sync_media_link::Model>,
) -> Result<BookEdition, async_graphql::Error> {
	let audio_model = media_audio::Entity::find()
		.filter(media_audio::Column::MediaId.eq(model.media.id.clone()))
		.one(conn)
		.await?;
	let kind = BookEditionKind::from_model(&model.media, audio_model.is_some());
	let audio = if let Some(audio_model) = audio_model {
		let chapters = media_audio_chapter::Entity::find()
			.filter(media_audio_chapter::Column::MediaId.eq(model.media.id.clone()))
			.order_by_asc(media_audio_chapter::Column::Index)
			.all(conn)
			.await?
			.into_iter()
			.map(|chapter| BookAudioChapter {
				id: ID::from(chapter.id),
				index: chapter.index,
				title: chapter.title,
				start_ms: chapter.start_ms,
				end_ms: chapter.end_ms,
			})
			.collect();
		Some(BookAudioFacts {
			duration_ms: audio_model.duration_ms,
			codec: audio_model.codec,
			sample_rate: audio_model.sample_rate,
			channels: audio_model.channels,
			bitrate: audio_model.bitrate,
			chapter_source: audio_model.chapter_source.to_string(),
			chapters,
		})
	} else {
		None
	};
	Ok(BookEdition {
		media_id: ID::from(model.media.id.clone()),
		kind,
		title: metadata_title(model.metadata.as_ref(), &model.media.name),
		media: model.clone().into(),
		metadata: model.metadata.map(Into::into),
		file: BookFile::from(&model.media),
		audio,
		pair_evidence: link
			.and_then(|link| link.pair_evidence.as_deref())
			.and_then(|value| value.parse().ok()),
	})
}

async fn build_read_aloud(
	core: &CoreContext,
	user: &AuthUser,
	editions: &[BookEdition],
) -> Result<BookReadAloud, async_graphql::Error> {
	let ebook = editions
		.iter()
		.find(|edition| edition.kind == BookEditionKind::Ebook);
	let audio = editions
		.iter()
		.find(|edition| edition.kind == BookEditionKind::Audiobook);
	let (Some(ebook), Some(audio)) = (ebook, audio) else {
		return Ok(BookReadAloud {
			status: BookReadAloudStatus::NoAudiobookPaired,
			reason: Some("No audiobook paired".to_owned()),
			ebook_media_id: ebook.map(|edition| edition.media_id.clone()),
			audio_media_id: audio.map(|edition| edition.media_id.clone()),
			chapter_map: Vec::new(),
			sync_map: None,
			artifact: None,
		});
	};
	let chapter_map_models = editions::chapter_map(
		core.conn.as_ref(),
		ebook.media_id.as_ref(),
		audio.media_id.as_ref(),
	)
	.await?;
	let chapter_map: Vec<_> = chapter_map_models.into_iter().map(Into::into).collect();
	if chapter_map.is_empty() {
		return Ok(BookReadAloud {
			status: BookReadAloudStatus::ChapterMapUnavailable,
			reason: Some("Chapter map unavailable".to_owned()),
			ebook_media_id: Some(ebook.media_id.clone()),
			audio_media_id: Some(audio.media_id.clone()),
			chapter_map,
			sync_map: None,
			artifact: None,
		});
	}
	let map = sync_maps::latest_sync_map_for_user(
		core.conn.as_ref(),
		&user.id,
		ebook.media_id.as_ref(),
		audio.media_id.as_ref(),
		AlignGranularity::Sentence,
	)
	.await
	.map_err(|error| async_graphql::Error::new(error.to_string()))?;
	let Some(map) = map else {
		return Ok(BookReadAloud {
			status: BookReadAloudStatus::SyncMapUnavailable,
			reason: Some("Sync map unavailable".to_owned()),
			ebook_media_id: Some(ebook.media_id.clone()),
			audio_media_id: Some(audio.media_id.clone()),
			chapter_map,
			sync_map: None,
			artifact: None,
		});
	};
	let cache_path =
		sync_maps::read_aloud_cache_path(core.config.get_transform_cache_dir(), &map)
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
	let sync_map = Some(SyncMap::from(map.clone()));
	if tokio::fs::metadata(&cache_path).await.is_err() {
		return Ok(BookReadAloud {
			status: BookReadAloudStatus::CacheMissing,
			reason: Some("Read-aloud cache missing".to_owned()),
			ebook_media_id: Some(ebook.media_id.clone()),
			audio_media_id: Some(audio.media_id.clone()),
			chapter_map,
			sync_map,
			artifact: None,
		});
	}
	let cache_key = cache_path
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or_default()
		.to_owned();
	Ok(BookReadAloud {
		status: BookReadAloudStatus::Ready,
		reason: None,
		ebook_media_id: Some(ebook.media_id.clone()),
		audio_media_id: Some(audio.media_id.clone()),
		chapter_map,
		sync_map,
		artifact: Some(ReadAloudArtifact {
			url: format!("/api/v2/media/{}/read-aloud.epub", ebook.media_id.as_ref()),
			mime_type: "application/epub+zip".to_owned(),
			cache_key,
		}),
	})
}

/// Resolve the current user's visible media id into the merged detail object.
pub async fn load_book_detail(
	core: &CoreContext,
	user: &AuthUser,
	media_id: &str,
) -> Result<Option<BookDetail>, async_graphql::Error> {
	let Some(_anchor) = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::Id.eq(media_id))
		.filter(media::Column::DeletedAt.is_null())
		.into_model::<media::ModelWithMetadata>()
		.one(core.conn.as_ref())
		.await?
	else {
		return Ok(None);
	};
	let anchor_link =
		edition_pair::link_for_media(core.conn.as_ref(), &user.id, media_id).await?;
	let confirmed = anchor_link.as_ref().is_some_and(|link| {
		PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
	});
	let mut ids = vec![media_id.to_owned()];
	let links = if confirmed {
		edition_pair::linked_media(
			core.conn.as_ref(),
			&user.id,
			media_id,
			Some(PairStatus::Confirmed),
		)
		.await?
	} else {
		Vec::new()
	};
	ids.extend(links.iter().map(|link| link.media_id.clone()));
	let models = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::Id.is_in(ids.clone()))
		.filter(media::Column::DeletedAt.is_null())
		.into_model::<media::ModelWithMetadata>()
		.all(core.conn.as_ref())
		.await?;
	let by_id: BTreeMap<_, _> = models
		.into_iter()
		.map(|model| (model.media.id.clone(), model))
		.collect();
	let mut editions = Vec::with_capacity(ids.len());
	for id in &ids {
		if let Some(model) = by_id.get(id) {
			let link = if id == media_id {
				anchor_link.as_ref()
			} else {
				links.iter().find(|link| link.media_id == *id)
			};
			editions.push(build_edition(core.conn.as_ref(), model.clone(), link).await?);
		}
	}
	if editions.is_empty() {
		return Ok(None);
	}
	let work = if confirmed {
		if let Some(link) = anchor_link.as_ref() {
			liseur_sync_work::Entity::find_by_id(link.work_id.clone())
				.filter(liseur_sync_work::Column::UserId.eq(user.id.clone()))
				.one(core.conn.as_ref())
				.await?
		} else {
			None
		}
	} else {
		None
	};
	let work_metadata = if let Some(work) = &work {
		book_work_metadata::Entity::find_by_id(work.id.clone())
			.filter(book_work_metadata::Column::UserId.eq(user.id.clone()))
			.one(core.conn.as_ref())
			.await?
	} else {
		None
	};
	let review = if let Some(work) = &work {
		book_review::Entity::find_for_work(&user.id, &work.id)
			.order_by_desc(book_review::Column::UpdatedAt)
			.one(core.conn.as_ref())
			.await?
	} else {
		book_review::Entity::find_for_media(&user.id, media_id)
			.order_by_desc(book_review::Column::UpdatedAt)
			.one(core.conn.as_ref())
			.await?
	};
	let title = work
		.as_ref()
		.and_then(|work| {
			work_metadata
				.as_ref()
				.and_then(|metadata| metadata.title.clone())
				.or_else(|| Some(work.title.clone()))
		})
		.filter(|title| !title.trim().is_empty())
		.unwrap_or_else(|| editions[0].title.clone());
	let authors = work
		.as_ref()
		.and_then(|work| {
			work_metadata
				.as_ref()
				.and_then(|metadata| metadata.author.clone())
				.or_else(|| Some(work.author.clone()))
		})
		.map(|author| {
			author
				.split(',')
				.map(str::trim)
				.filter(|author| !author.is_empty())
				.map(str::to_owned)
				.collect()
		})
		.unwrap_or_else(|| {
			metadata_authors(
				by_id
					.get(media_id)
					.and_then(|model| model.metadata.as_ref()),
			)
		});
	let read_aloud = build_read_aloud(core, user, &editions).await?;
	let mismatches = mismatch_rows(&editions, work.as_ref(), work_metadata.as_ref());
	Ok(Some(BookDetail {
		media_id: ID::from(media_id.to_owned()),
		work_id: work.as_ref().map(|work| ID::from(work.id.clone())),
		title,
		authors,
		work_metadata: work_metadata.map(|metadata| BookWorkMetadata {
			work_id: ID::from(metadata.work_id),
			title: metadata.title,
			author: metadata.author,
			metadata: Json(metadata.metadata.unwrap_or_else(|| serde_json::json!({}))),
			locked_fields: metadata
				.locked_fields
				.and_then(|value| serde_json::from_value(value).ok())
				.unwrap_or_default(),
		}),
		mismatches,
		review: review.map(Into::into),
		editions,
		read_aloud,
	}))
}

fn source_protocol(session: &reading_session::Model) -> String {
	if session.liseur_session_id.is_some() {
		"liseur".to_owned()
	} else if session.kobo_state.is_some() {
		"kobo".to_owned()
	} else if session.koreader_progress.is_some() {
		"koreader".to_owned()
	} else {
		"native".to_owned()
	}
}

async fn devices_for_ids(
	conn: &DatabaseConnection,
	user: &AuthUser,
	ids: &[String],
) -> Result<Vec<BookReadingDevice>, async_graphql::Error> {
	if ids.is_empty() {
		return Ok(Vec::new());
	}
	let devices = device::Entity::find_visible_to(user)
		.filter(device::Column::Id.is_in(ids.to_owned()))
		.all(conn)
		.await?;
	Ok(ids
		.iter()
		.filter_map(|id| devices.iter().find(|device| device.id == *id))
		.map(BookReadingDevice::from)
		.collect())
}

async fn load_reading_edition(
	core: &CoreContext,
	user: &AuthUser,
	media_id: &str,
	kind: BookEditionKind,
) -> Result<BookReadingEdition, async_graphql::Error> {
	let head = reading_head::Entity::find()
		.filter(reading_head::Column::UserId.eq(user.id.clone()))
		.filter(reading_head::Column::MediaId.eq(media_id))
		.one(core.conn.as_ref())
		.await?;
	let head = if let Some(head) = head {
		let source_device = if let Some(device_id) = &head.source_device_id {
			device::Entity::find_visible_to(user)
				.filter(device::Column::Id.eq(device_id.clone()))
				.one(core.conn.as_ref())
				.await?
				.as_ref()
				.map(BookReadingDevice::from)
		} else {
			None
		};
		Some(BookReadingHead {
			locator: head.locator,
			progression: head.progression,
			page: head.page,
			position_ms: head.position_ms,
			track_index: head.track_index,
			completed: head.completed,
			updated_at: head.updated_at.to_rfc3339(),
			source_protocol: head.source_protocol.to_string(),
			source_device_id: head.source_device_id.map(ID::from),
			source_device,
			revision: head.revision,
		})
	} else {
		None
	};
	let sessions = reading_session::Entity::find()
		.filter(reading_session::Column::UserId.eq(user.id.clone()))
		.filter(reading_session::Column::MediaId.eq(media_id))
		.order_by_asc(reading_session::Column::CreatedAt)
		.order_by_asc(reading_session::Column::Id)
		.all(core.conn.as_ref())
		.await?;
	let mut rows = Vec::with_capacity(sessions.len());
	for session in sessions {
		let ids = session
			.device_ids
			.as_ref()
			.map(|ids| ids.0.clone())
			.unwrap_or_default();
		let devices = devices_for_ids(core.conn.as_ref(), user, &ids).await?;
		let start_percentage = session.start_percentage.and_then(|value| value.to_f64());
		let end_percentage = session.end_percentage.and_then(|value| value.to_f64());
		let protocol = source_protocol(&session);
		rows.push(BookReadingSession {
			id: ID::from(session.id.to_string()),
			session_date: session.session_date.to_string(),
			status: session.status.to_string(),
			readthrough_number: session.readthrough_number,
			start_locator: session.start_locator,
			end_locator: session.end_locator,
			start_page: session.start_page,
			end_page: session.end_page,
			end_position_ms: session.end_position_ms,
			start_percentage,
			end_percentage,
			elapsed_seconds: session.elapsed_seconds,
			notes: session.notes,
			source_protocol: protocol,
			source_device_ids: ids.into_iter().map(ID::from).collect(),
			source_devices: devices,
			liseur_session_id: session.liseur_session_id.map(ID::from),
			created_at: session.created_at.to_rfc3339(),
			updated_at: session.updated_at.map(|value| value.to_rfc3339()),
		});
	}
	Ok(BookReadingEdition {
		media_id: ID::from(media_id.to_owned()),
		kind,
		head,
		sessions: rows,
	})
}

/// Return raw reading-head/session history grouped by the merged edition rows.
pub async fn load_book_reading_log(
	core: &CoreContext,
	user: &AuthUser,
	media_id: &str,
) -> Result<Option<BookReadingLog>, async_graphql::Error> {
	let Some(detail) = load_book_detail(core, user, media_id).await? else {
		return Ok(None);
	};
	let mut editions = Vec::with_capacity(detail.editions.len());
	for edition in &detail.editions {
		editions.push(
			load_reading_edition(core, user, edition.media_id.as_ref(), edition.kind)
				.await?,
		);
	}
	Ok(Some(BookReadingLog {
		media_id: detail.media_id,
		work_id: detail.work_id,
		editions,
	}))
}
