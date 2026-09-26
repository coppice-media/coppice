//! Native, transactional persistence for Komf's Kavita metadata writes.
//!
//! Kavita identities resolve to existing Stump series/media rows; this module
//! only updates those rows, their metadata rows, and native tag links.

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use models::{
	entity::{
		media, media_metadata, media_tag, series, series_metadata, series_tag, tag,
		user::AuthUser,
	},
	shared::image::ImageMetadata,
	txn::begin_write,
};
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
	EntityTrait, IntoActiveModel, QueryFilter,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
	dto::{
		AgeRating, ChapterMetadataUpdateDto, KavitaAuthorDto, SeriesMetadataUpdateDto,
		SeriesUpdateDto, TagDto,
	},
	errors::{APIError, APIResult},
};

use super::KavitaSeriesTarget;

const SERIES_NOT_FOUND: &str = "Series does not exist";
const CHAPTER_NOT_FOUND: &str = "Chapter does not exist";

fn new_series_metadata(id: String) -> series_metadata::ActiveModel {
	series_metadata::ActiveModel {
		series_id: Set(id),
		title_sort_lock: Set(false),
		reading_direction_lock: Set(false),
		language_lock: Set(false),
		alternate_titles_lock: Set(false),
		..Default::default()
	}
}

pub(super) async fn update_series(
	conn: &DatabaseConnection,
	user: &AuthUser,
	target: KavitaSeriesTarget,
	update: SeriesUpdateDto,
) -> APIResult<()> {
	let txn = begin_write(conn).await?;
	match target {
		KavitaSeriesTarget::Series(id) => {
			visible_series(&txn, user, &id).await?;
			let existing = series_metadata::Entity::find_by_id(id.clone())
				.one(&txn)
				.await?;
			let had_metadata = existing.is_some();
			let current_titles = existing
				.as_ref()
				.and_then(|metadata| metadata.alternate_titles.clone());
			let current_locks = existing
				.as_ref()
				.and_then(|metadata| metadata.locked_fields.clone());
			let mut active = existing
				.map(|metadata| metadata.into_active_model())
				.unwrap_or_else(|| new_series_metadata(id.clone()));
			active.title_sort = Set(Some(update.sort_name));
			active.title_sort_lock = Set(update.sort_name_locked);
			active.alternate_titles = Set(localized_title(
				current_titles.as_deref(),
				update.localized_name.as_deref(),
			)?);
			active.alternate_titles_lock = Set(update.localized_name_locked);
			active.locked_fields = Set(locked_fields(
				current_locks,
				&[("COVER", update.cover_image_locked)],
			));
			save_series_metadata(&txn, had_metadata, active).await?;
		},
		KavitaSeriesTarget::Book(id) => {
			visible_media(&txn, user, &id).await?;
			let existing = media_metadata::Entity::find()
				.filter(media_metadata::Column::MediaId.eq(Some(id.clone())))
				.one(&txn)
				.await?;
			let had_metadata = existing.is_some();
			let current_locks = existing
				.as_ref()
				.and_then(|metadata| metadata.locked_fields.clone());
			let mut active = existing
				.map(|metadata| metadata.into_active_model())
				.unwrap_or_else(|| media_metadata::ActiveModel {
					media_id: Set(Some(id.clone())),
					..Default::default()
				});
			active.title_sort = Set(Some(update.sort_name));
			active.locked_fields = Set(locked_fields(
				current_locks,
				&[
					("TITLE_SORT", update.sort_name_locked),
					("COVER", update.cover_image_locked),
				],
			));
			// Book-series metadata is stored on the media row. Stump has no
			// media-level alternate-title field, so localizedName is only
			// representable for ordinary grouped series.
			save_media_metadata(&txn, had_metadata, active).await?;
		},
	}
	let _ = txn.commit().await?;
	Ok(())
}

pub(super) async fn update_series_metadata(
	conn: &DatabaseConnection,
	user: &AuthUser,
	target: KavitaSeriesTarget,
	update: SeriesMetadataUpdateDto,
) -> APIResult<()> {
	let txn = begin_write(conn).await?;
	match target {
		KavitaSeriesTarget::Series(id) => {
			visible_series(&txn, user, &id).await?;
			let existing = series_metadata::Entity::find_by_id(id.clone())
				.one(&txn)
				.await?;
			let had_metadata = existing.is_some();
			let current_locks = existing
				.as_ref()
				.and_then(|metadata| metadata.locked_fields.clone());
			let mut active = existing
				.map(|metadata| metadata.into_active_model())
				.unwrap_or_else(|| new_series_metadata(id.clone()));
			active.summary = Set(update.summary.clone());
			active.genres = Set(csv(&update
				.genres
				.iter()
				.map(|item| item.title.clone())
				.collect::<Vec<_>>()));
			active.age_rating = Set(min_age(update.age_rating));
			active.language = Set(update.language.clone());
			active.language_lock = Set(update.language_locked);
			active.year = Set(nonzero(update.release_year));
			active.total_issues = Set(nonzero(if update.total_count > 0 {
				update.total_count
			} else {
				update.max_count
			}));
			active.status = Set(Some(
				publication_status_name(update.publication_status).to_owned(),
			));
			active.links = Set(nonempty(update.web_links.clone()));
			active.writers = Set(people_csv(&update.writers));
			active.publisher = Set(people_csv(&update.publishers));
			active.characters = Set(people_csv(&update.characters));
			active.imprint = Set(people_csv(&update.imprints));
			active.locked_fields = Set(series_metadata_locks(current_locks, &update));
			save_series_metadata(&txn, had_metadata, active).await?;

			let desired_tags = tag_names(&update.tags);
			replace_series_tags(&txn, &id, &desired_tags).await?;
			let series_row = visible_series(&txn, user, &id).await?;
			if series_row.description != update.summary {
				let mut active = series_row.into_active_model();
				active.description = Set(update.summary.clone());
				active.updated_at = Set(Some(chrono::Utc::now().into()));
				active.update(&txn).await?;
			}
		},
		KavitaSeriesTarget::Book(id) => {
			visible_media(&txn, user, &id).await?;
			let existing = media_metadata::Entity::find()
				.filter(media_metadata::Column::MediaId.eq(Some(id.clone())))
				.one(&txn)
				.await?;
			let had_metadata = existing.is_some();
			let current_locks = existing
				.as_ref()
				.and_then(|metadata| metadata.locked_fields.clone());
			let mut active = existing
				.map(|metadata| metadata.into_active_model())
				.unwrap_or_else(|| media_metadata::ActiveModel {
					media_id: Set(Some(id.clone())),
					..Default::default()
				});
			apply_book_metadata(&mut active, &update, current_locks);
			save_media_metadata(&txn, had_metadata, active).await?;
			replace_media_tags(&txn, &id, &tag_names(&update.tags)).await?;
		},
	}
	let _ = txn.commit().await?;
	Ok(())
}

pub(super) async fn update_chapter_metadata(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_id: String,
	update: ChapterMetadataUpdateDto,
) -> APIResult<()> {
	let release_date = parse_local_datetime(&update.release_date)?;
	let number = rust_decimal::Decimal::try_from(update.sort_order)
		.map_err(|error| APIError::BadRequest(format!("Invalid sortOrder: {error}")))?;
	let txn = begin_write(conn).await?;
	visible_media(&txn, user, &media_id).await?;
	let existing = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.eq(Some(media_id.clone())))
		.one(&txn)
		.await?;
	let had_metadata = existing.is_some();
	let current_locks = existing
		.as_ref()
		.and_then(|metadata| metadata.locked_fields.clone());
	let next_locked_fields = chapter_metadata_locks(current_locks, &update);
	let mut active = existing
		.map(|metadata| metadata.into_active_model())
		.unwrap_or_else(|| media_metadata::ActiveModel {
			media_id: Set(Some(media_id.clone())),
			..Default::default()
		});
	active.summary = Set(nonempty(update.summary));
	active.genres = Set(csv(&update
		.genres
		.iter()
		.map(|item| item.title.clone())
		.collect::<Vec<_>>()));
	active.age_rating = Set(min_age(update.age_rating));
	active.language = Set(update.language);
	active.links = Set(nonempty(Some(update.web_links)));
	active.identifier_isbn = Set(nonempty(Some(update.isbn)));
	active.year = Set(Some(release_date.year()));
	active.month = Set(Some(release_date.month() as i32));
	active.day = Set(Some(release_date.day() as i32));
	active.title = Set(nonempty(Some(update.title_name)));
	active.number = Set(Some(number));
	active.writers = Set(people_csv(&update.writers));
	active.cover_artists = Set(people_csv(&update.cover_artists));
	active.publisher = Set(people_csv(&update.publishers));
	active.characters = Set(people_csv(&update.characters));
	active.pencillers = Set(people_csv(&update.pencillers));
	active.inkers = Set(people_csv(&update.inkers));
	active.colorists = Set(people_csv(&update.colorists));
	active.letterers = Set(people_csv(&update.letterers));
	active.editors = Set(people_csv(&update.editors));
	active.teams = Set(people_csv(&update.teams));
	active.locked_fields = Set(next_locked_fields);
	save_media_metadata(&txn, had_metadata, active).await?;
	replace_media_tags(&txn, &media_id, &tag_names(&update.tags)).await?;
	let _ = txn.commit().await?;
	Ok(())
}

pub(super) async fn persist_series_cover(
	conn: &DatabaseConnection,
	user: &AuthUser,
	target: KavitaSeriesTarget,
	path: String,
	metadata: ImageMetadata,
	lock_cover: bool,
) -> APIResult<()> {
	let txn = begin_write(conn).await?;
	match target {
		KavitaSeriesTarget::Series(id) => {
			let row = visible_series(&txn, user, &id).await?;
			series::ActiveModel {
				thumbnail_path: Set(Some(path)),
				thumbnail_meta: Set(Some(metadata)),
				..row.into_active_model()
			}
			.update(&txn)
			.await?;
			set_series_cover_lock(&txn, &id, lock_cover).await?;
		},
		KavitaSeriesTarget::Book(id) => {
			let row = visible_media(&txn, user, &id).await?;
			media::ActiveModel {
				thumbnail_path: Set(Some(path)),
				thumbnail_meta: Set(Some(metadata)),
				..row.into_active_model()
			}
			.update(&txn)
			.await?;
			set_media_cover_lock(&txn, &id, lock_cover).await?;
		},
	}
	let _ = txn.commit().await?;
	Ok(())
}

pub(super) async fn persist_media_cover(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_id: String,
	path: String,
	metadata: ImageMetadata,
	lock_cover: bool,
) -> APIResult<()> {
	let txn = begin_write(conn).await?;
	let row = visible_media(&txn, user, &media_id).await?;
	media::ActiveModel {
		thumbnail_path: Set(Some(path)),
		thumbnail_meta: Set(Some(metadata)),
		..row.into_active_model()
	}
	.update(&txn)
	.await?;
	set_media_cover_lock(&txn, &media_id, lock_cover).await?;
	let _ = txn.commit().await?;
	Ok(())
}

pub(super) async fn reset_media_cover_lock(
	conn: &DatabaseConnection,
	user: &AuthUser,
	media_id: &str,
) -> APIResult<()> {
	let txn = begin_write(conn).await?;
	visible_media(&txn, user, media_id).await?;
	set_media_cover_lock(&txn, media_id, false).await?;
	let _ = txn.commit().await?;
	Ok(())
}

async fn visible_series<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	id: &str,
) -> APIResult<series::Model> {
	series::Entity::find_for_user(user)
		.filter(series::Column::Id.eq(id.to_owned()))
		.filter(series::Column::DeletedAt.is_null())
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound(SERIES_NOT_FOUND.to_owned()))
}

async fn visible_media<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	id: &str,
) -> APIResult<media::Model> {
	media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(id.to_owned()))
		.filter(media::Column::DeletedAt.is_null())
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound(CHAPTER_NOT_FOUND.to_owned()))
}

async fn save_series_metadata<C: ConnectionTrait>(
	conn: &C,
	had_metadata: bool,
	active: series_metadata::ActiveModel,
) -> APIResult<()> {
	if had_metadata {
		active.update(conn).await?;
	} else {
		active.insert(conn).await?;
	}
	Ok(())
}

async fn save_media_metadata<C: ConnectionTrait>(
	conn: &C,
	had_metadata: bool,
	active: media_metadata::ActiveModel,
) -> APIResult<()> {
	if had_metadata {
		active.update(conn).await?;
	} else {
		active.insert(conn).await?;
	}
	Ok(())
}

async fn set_series_cover_lock<C: ConnectionTrait>(
	conn: &C,
	id: &str,
	locked: bool,
) -> APIResult<()> {
	let existing = series_metadata::Entity::find_by_id(id.to_owned())
		.one(conn)
		.await?;
	let had_metadata = existing.is_some();
	let current_locks = existing
		.as_ref()
		.and_then(|metadata| metadata.locked_fields.clone());
	let mut active = existing
		.map(|metadata| metadata.into_active_model())
		.unwrap_or_else(|| new_series_metadata(id.to_owned()));
	active.locked_fields = Set(locked_fields(current_locks, &[("COVER", locked)]));
	save_series_metadata(conn, had_metadata, active).await
}

async fn set_media_cover_lock<C: ConnectionTrait>(
	conn: &C,
	id: &str,
	locked: bool,
) -> APIResult<()> {
	let existing = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.eq(Some(id.to_owned())))
		.one(conn)
		.await?;
	let had_metadata = existing.is_some();
	let current_locks = existing
		.as_ref()
		.and_then(|metadata| metadata.locked_fields.clone());
	let mut active = existing
		.map(|metadata| metadata.into_active_model())
		.unwrap_or_else(|| media_metadata::ActiveModel {
			media_id: Set(Some(id.to_owned())),
			..Default::default()
		});
	active.locked_fields = Set(locked_fields(current_locks, &[("COVER", locked)]));
	save_media_metadata(conn, had_metadata, active).await
}

fn apply_book_metadata(
	active: &mut media_metadata::ActiveModel,
	update: &SeriesMetadataUpdateDto,
	current_locks: Option<Value>,
) {
	active.summary = Set(update.summary.clone());
	active.genres = Set(csv(&update
		.genres
		.iter()
		.map(|item| item.title.clone())
		.collect::<Vec<_>>()));
	active.age_rating = Set(min_age(update.age_rating));
	active.year = Set(nonzero(update.release_year));
	active.language = Set(update.language.clone());
	active.links = Set(nonempty(update.web_links.clone()));
	active.writers = Set(people_csv(&update.writers));
	active.cover_artists = Set(people_csv(&update.cover_artists));
	active.publisher = Set(people_csv(&update.publishers));
	active.characters = Set(people_csv(&update.characters));
	active.pencillers = Set(people_csv(&update.pencillers));
	active.inkers = Set(people_csv(&update.inkers));
	active.colorists = Set(people_csv(&update.colorists));
	active.letterers = Set(people_csv(&update.letterers));
	active.editors = Set(people_csv(&update.editors));
	active.teams = Set(people_csv(&update.teams));
	active.locked_fields = Set(series_metadata_locks(current_locks, update));
}

fn series_metadata_locks(
	current: Option<Value>,
	update: &SeriesMetadataUpdateDto,
) -> Option<Value> {
	locked_fields(
		current,
		&[
			("LANGUAGE", update.language_locked),
			("SUMMARY", update.summary_locked),
			("AGE_RATING", update.age_rating_locked),
			("STATUS", update.publication_status_locked),
			("GENRES", update.genres_locked),
			("TAGS", update.tags_locked),
			("WRITERS", update.writer_locked),
			("CHARACTERS", update.character_locked),
			("COLORISTS", update.colorist_locked),
			("EDITORS", update.editor_locked),
			("INKERS", update.inker_locked),
			("IMPRINT", update.imprint_locked),
			("LETTERERS", update.letterer_locked),
			("PENCILLERS", update.penciller_locked),
			("PUBLISHER", update.publisher_locked),
			("TEAMS", update.team_locked),
			("COVER_ARTISTS", update.cover_artist_locked),
			("YEAR", update.release_year_locked),
		],
	)
}

fn chapter_metadata_locks(
	current: Option<Value>,
	update: &ChapterMetadataUpdateDto,
) -> Option<Value> {
	locked_fields(
		current,
		&[
			("AGE_RATING", update.age_rating_locked),
			("TITLE", update.title_name_locked),
			("GENRES", update.genres_locked),
			("TAGS", update.tags_locked),
			("WRITERS", update.writer_locked),
			("CHARACTERS", update.character_locked),
			("COLORISTS", update.colorist_locked),
			("EDITORS", update.editor_locked),
			("INKERS", update.inker_locked),
			("IMPRINT", update.imprint_locked),
			("LETTERERS", update.letterer_locked),
			("PENCILLERS", update.penciller_locked),
			("PUBLISHER", update.publisher_locked),
			("TEAMS", update.team_locked),
			("COVER_ARTISTS", update.cover_artist_locked),
			("LANGUAGE", update.language_locked),
			("SUMMARY", update.summary_locked),
			("ISBN", update.isbn_locked),
			("RELEASE_DATE", update.release_date_locked),
			("NUMBER", update.sort_order_locked),
		],
	)
}

fn locked_fields(current: Option<Value>, updates: &[(&str, bool)]) -> Option<Value> {
	let mut fields = current
		.and_then(|value| value.as_array().cloned())
		.unwrap_or_default()
		.into_iter()
		.filter_map(|value| value.as_str().map(str::to_owned))
		.collect::<Vec<_>>();
	for (name, locked) in updates {
		fields.retain(|field| field != name);
		if *locked {
			fields.push((*name).to_owned());
		}
	}
	Some(json!(fields))
}

fn localized_title(
	current: Option<&str>,
	localized_name: Option<&str>,
) -> APIResult<Option<String>> {
	let mut titles = current
		.map(serde_json::from_str::<Vec<AlternateTitle>>)
		.transpose()?
		.unwrap_or_default();
	titles.retain(|title| !title.label.eq_ignore_ascii_case("localized"));
	if let Some(title) = localized_name
		.map(str::trim)
		.filter(|title| !title.is_empty())
	{
		titles.push(AlternateTitle {
			label: "Localized".to_owned(),
			title: title.to_owned(),
		});
	}
	if titles.is_empty() {
		Ok(None)
	} else {
		Ok(Some(serde_json::to_string(&titles)?))
	}
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AlternateTitle {
	label: String,
	title: String,
}

fn min_age(rating: AgeRating) -> Option<i32> {
	match rating {
		AgeRating::NotApplicable | AgeRating::Unknown => None,
		AgeRating::RatingPending | AgeRating::Everyone | AgeRating::G => Some(0),
		AgeRating::EarlyChildhood => Some(3),
		AgeRating::KidsToAdults => Some(6),
		AgeRating::PG => Some(8),
		AgeRating::Everyone10Plus => Some(10),
		AgeRating::Teen => Some(13),
		AgeRating::Mature15Plus => Some(15),
		AgeRating::Mature17Plus | AgeRating::Mature => Some(17),
		AgeRating::R18Plus | AgeRating::AdultsOnly | AgeRating::X18Plus => Some(18),
	}
}

fn publication_status_name(status: crate::dto::PublicationStatus) -> &'static str {
	match status {
		crate::dto::PublicationStatus::OnGoing => "Ongoing",
		crate::dto::PublicationStatus::Hiatus => "Hiatus",
		crate::dto::PublicationStatus::Completed => "Completed",
		crate::dto::PublicationStatus::Cancelled => "Cancelled",
		crate::dto::PublicationStatus::Ended => "Ended",
	}
}

fn parse_local_datetime(value: &str) -> APIResult<NaiveDate> {
	let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
		.or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
		.map_err(|error| APIError::BadRequest(format!("Invalid releaseDate: {error}")))?;
	Ok(parsed.date())
}

fn nonzero(value: i32) -> Option<i32> {
	(value != 0).then_some(value)
}

fn nonempty(value: Option<String>) -> Option<String> {
	value.filter(|value| !value.trim().is_empty())
}

fn csv(values: &[String]) -> Option<String> {
	let mut seen = std::collections::BTreeSet::new();
	let values = values
		.iter()
		.map(|value| value.trim())
		.filter(|value| !value.is_empty())
		.filter(|value| seen.insert(value.to_lowercase()))
		.collect::<Vec<_>>();
	(!values.is_empty()).then(|| values.join(", "))
}

fn people_csv(people: &[KavitaAuthorDto]) -> Option<String> {
	csv(&people
		.iter()
		.map(|person| person.name.clone())
		.collect::<Vec<_>>())
}

fn tag_names(tags: &[TagDto]) -> Vec<String> {
	let mut seen = std::collections::BTreeSet::new();
	tags.iter()
		.map(|tag| tag.title.trim())
		.filter(|title| !title.is_empty())
		.filter(|title| seen.insert(title.to_lowercase()))
		.map(str::to_owned)
		.collect()
}

async fn tag_ids<C: ConnectionTrait>(conn: &C, names: &[String]) -> APIResult<Vec<i32>> {
	let mut ids = Vec::with_capacity(names.len());
	for name in names {
		let existing = tag::Entity::find()
			.filter(tag::Column::Name.eq(name.clone()))
			.one(conn)
			.await?;
		let id = match existing {
			Some(tag) => tag.id,
			None => {
				tag::ActiveModel {
					name: Set(name.clone()),
					kind: Set("tag".to_owned()),
					..Default::default()
				}
				.insert(conn)
				.await?
				.id
			},
		};
		ids.push(id);
	}
	Ok(ids)
}

async fn replace_series_tags<C: ConnectionTrait>(
	conn: &C,
	series_id: &str,
	names: &[String],
) -> APIResult<()> {
	series_tag::Entity::delete_many()
		.filter(series_tag::Column::SeriesId.eq(series_id.to_owned()))
		.exec(conn)
		.await?;
	for id in tag_ids(conn, names).await? {
		series_tag::ActiveModel {
			series_id: Set(series_id.to_owned()),
			tag_id: Set(id),
			..Default::default()
		}
		.insert(conn)
		.await?;
	}
	Ok(())
}

async fn replace_media_tags<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
	names: &[String],
) -> APIResult<()> {
	media_tag::Entity::delete_many()
		.filter(media_tag::Column::MediaId.eq(media_id.to_owned()))
		.exec(conn)
		.await?;
	for id in tag_ids(conn, names).await? {
		media_tag::ActiveModel {
			media_id: Set(media_id.to_owned()),
			tag_id: Set(id),
			..Default::default()
		}
		.insert(conn)
		.await?;
	}
	Ok(())
}
