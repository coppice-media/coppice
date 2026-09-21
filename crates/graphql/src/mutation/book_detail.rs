//! Explicit book-detail metadata and review mutations.
//!
//! Every metadata mutation carries a destination scope and a selected field set.
//! Edition updates preserve all unselected values and reject locked fields;
//! work metadata is persisted separately instead of being mirrored into either
//! edition.

use async_graphql::{Context, Object, Result, ID};
use metadata_integrations::{
	ExternalMediaMetadata, ExternalMetadata, MatchCandidate, MetadataField,
};
use models::{
	entity::{
		book_review, book_work_metadata, media, media_metadata, metadata_fetch_record,
	},
	shared::enums::{MetadataFetchStatus, UserPermission},
};
use sea_orm::{prelude::*, ActiveValue::Set, IntoActiveModel};
use stump_auth::AuthContext;

use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	input::{
		book_detail::{BookMetadataApplyInput, BookReviewInput},
		media::MediaMetadataInput,
	},
	object::{
		book_detail::{
			load_book_detail, metadata_field_name, BookDetail, BookEditionKind,
			BookMetadataScope,
		},
		book_review::BookReview,
	},
};

#[derive(Default)]
pub struct BookDetailMutation;

#[Object]
impl BookDetailMutation {
	/// Create or replace the current user's review for this work/edition.
	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn upsert_book_review(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		input: BookReviewInput,
	) -> Result<BookReview> {
		if !(0..=5).contains(&input.rating) {
			return Err("rating must be between 0 and 5".into());
		}
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let detail = load_book_detail(core, &auth.user, media_id.as_ref())
			.await?
			.ok_or("Media not found")?;
		let content = input
			.content
			.map(|content| content.trim().to_owned())
			.filter(|content| !content.is_empty());
		let updated = if let Some(work_id) = detail.work_id {
			let existing =
				book_review::Entity::find_for_work(&auth.user.id, work_id.as_ref())
					.one(core.conn.as_ref())
					.await?;
			if let Some(existing) = existing {
				let mut active = existing.into_active_model();
				active.rating = Set(input.rating);
				active.content = Set(content);
				active.is_private = Set(input.is_private);
				active.update(core.conn.as_ref()).await?
			} else {
				book_review::ActiveModel {
					work_id: Set(Some(work_id.to_string())),
					media_id: Set(None),
					user_id: Set(auth.user.id.clone()),
					rating: Set(input.rating),
					content: Set(content),
					is_private: Set(input.is_private),
					..Default::default()
				}
				.insert(core.conn.as_ref())
				.await?
			}
		} else {
			let existing =
				book_review::Entity::find_for_media(&auth.user.id, media_id.as_ref())
					.one(core.conn.as_ref())
					.await?;
			if let Some(existing) = existing {
				let mut active = existing.into_active_model();
				active.rating = Set(input.rating);
				active.content = Set(content);
				active.is_private = Set(input.is_private);
				active.update(core.conn.as_ref()).await?
			} else {
				book_review::ActiveModel {
					work_id: Set(None),
					media_id: Set(Some(media_id.to_string())),
					user_id: Set(auth.user.id.clone()),
					rating: Set(input.rating),
					content: Set(content),
					is_private: Set(input.is_private),
					..Default::default()
				}
				.insert(core.conn.as_ref())
				.await?
			}
		};
		Ok(updated.into())
	}

	/// Delete only the current user's explicit work/edition review.
	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn delete_book_review(&self, ctx: &Context<'_>, media_id: ID) -> Result<bool> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let detail = load_book_detail(core, &auth.user, media_id.as_ref())
			.await?
			.ok_or("Media not found")?;
		let result = if let Some(work_id) = detail.work_id {
			book_review::Entity::delete_many()
				.filter(book_review::Column::UserId.eq(auth.user.id.clone()))
				.filter(book_review::Column::WorkId.eq(work_id.to_string()))
				.exec(core.conn.as_ref())
				.await?
		} else {
			book_review::Entity::delete_many()
				.filter(book_review::Column::UserId.eq(auth.user.id.clone()))
				.filter(book_review::Column::MediaId.eq(media_id.to_string()))
				.exec(core.conn.as_ref())
				.await?
		};
		Ok(result.rows_affected > 0)
	}

	/// Apply selected metadata fields to the explicitly selected work or
	/// edition destination. Unselected fields and other editions are untouched.
	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn apply_book_metadata(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		input: BookMetadataApplyInput,
	) -> Result<BookDetail> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		apply_metadata_input(core, &auth, media_id.as_ref(), &input).await?;
		load_book_detail(core, &auth.user, media_id.as_ref())
			.await?
			.ok_or_else(|| "Media not found".into())
	}

	/// Apply selected fields from one persisted provider candidate. Provider
	/// credentials are never returned and no unselected field is copied.
	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataFetchRecordManage)")]
	async fn apply_book_metadata_candidate(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		candidate_index: u32,
		scope: BookMetadataScope,
		selected_fields: Vec<MetadataField>,
	) -> Result<BookDetail> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let record = metadata_fetch_record::Entity::find()
			.filter(metadata_fetch_record::Column::MediaId.eq(media_id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("No fetch status found for this media")?;
		if record.status != MetadataFetchStatus::AwaitingReview {
			return Err("Fetch status is not awaiting review".into());
		}
		let candidates: Vec<MatchCandidate> = record
			.match_candidates
			.as_ref()
			.and_then(|value| serde_json::from_value(value.clone()).ok())
			.unwrap_or_default();
		let candidate = candidates
			.get(candidate_index as usize)
			.ok_or("Candidate index out of bounds")?;
		let ExternalMetadata::Media(external) = &candidate.metadata else {
			return Err("Selected candidate is not media metadata".into());
		};
		apply_external_metadata(
			core,
			&auth.user,
			media_id.as_ref(),
			scope,
			&selected_fields,
			external,
		)
		.await?;
		let mut active = record.into_active_model();
		active.status = Set(MetadataFetchStatus::Matched);
		active.accepted_match_candidate = Set(Some(serde_json::to_value(candidate)?));
		active.update(core.conn.as_ref()).await?;
		load_book_detail(core, &auth.user, media_id.as_ref())
			.await?
			.ok_or_else(|| "Media not found".into())
	}
}

async fn visible_detail(
	core: &CoreContext,
	auth: &AuthContext,
	media_id: &str,
) -> Result<BookDetail> {
	load_book_detail(core, &auth.user, media_id)
		.await?
		.ok_or_else(|| "Media not found".into())
}

async fn apply_metadata_input(
	core: &CoreContext,
	auth: &AuthContext,
	media_id: &str,
	input: &BookMetadataApplyInput,
) -> Result<()> {
	let detail = visible_detail(core, auth, media_id).await?;
	if input.selected_fields.is_empty() {
		return Err("selected_fields must not be empty".into());
	}
	match input.scope {
		BookMetadataScope::Work => {
			let Some(work_id) = detail.work_id else {
				return Err("WORK scope requires a confirmed work".into());
			};
			let Some(work) =
				models::entity::liseur_sync_work::Entity::find_by_id(work_id.to_string())
					.filter(
						models::entity::liseur_sync_work::Column::UserId
							.eq(auth.user.id.clone()),
					)
					.one(core.conn.as_ref())
					.await?
			else {
				return Err("Work not found".into());
			};
			let existing = book_work_metadata::Entity::find_by_id(work.id.clone())
				.filter(book_work_metadata::Column::UserId.eq(auth.user.id.clone()))
				.one(core.conn.as_ref())
				.await?;
			let mut active = if let Some(existing) = existing {
				existing.into_active_model()
			} else {
				book_work_metadata::ActiveModel {
					work_id: Set(work.id.clone()),
					user_id: Set(auth.user.id.clone()),
					..Default::default()
				}
			};
			let mut metadata = match active.metadata.clone() {
				Set(Some(value)) => value,
				_ => serde_json::json!({}),
			};
			for field in &input.selected_fields {
				let value = input_value(*field, &input.metadata).ok_or_else(|| {
					format!("field {field:?} is not supported by this editor")
				})?;
				match field {
					MetadataField::Title => active.title = Set(value_to_string(value)),
					MetadataField::Writers => active.author = Set(value_to_string(value)),
					_ => {
						metadata[metadata_field_name(*field)] = value;
					},
				}
			}
			active.metadata = Set(Some(metadata));
			if active.locked_fields.is_not_set() {
				active.locked_fields = Set(None);
			}
			if active.work_id.is_not_set() {
				active.work_id = Set(work.id);
			}
			if active.user_id.is_not_set() {
				active.user_id = Set(auth.user.id.clone());
			}
			active.save(core.conn.as_ref()).await?;
		},
		BookMetadataScope::Ebook
		| BookMetadataScope::Audiobook
		| BookMetadataScope::Both => {
			let targets: Vec<_> = detail
				.editions
				.iter()
				.filter(|edition| match input.scope {
					BookMetadataScope::Ebook => edition.kind == BookEditionKind::Ebook,
					BookMetadataScope::Audiobook => {
						edition.kind == BookEditionKind::Audiobook
					},
					BookMetadataScope::Both => {
						edition.kind == BookEditionKind::Ebook
							|| edition.kind == BookEditionKind::Audiobook
					},
					_ => false,
				})
				.map(|edition| edition.media_id.to_string())
				.collect();
			if targets.is_empty() {
				return Err("Selected edition scope has no edition".into());
			}
			for target in targets {
				apply_edition_input(
					core,
					auth,
					&target,
					&input.selected_fields,
					&input.metadata,
				)
				.await?;
			}
		},
	}
	Ok(())
}

async fn apply_edition_input(
	core: &CoreContext,
	auth: &AuthContext,
	media_id: &str,
	selected_fields: &[MetadataField],
	input: &MediaMetadataInput,
) -> Result<()> {
	let model = media::ModelWithMetadata::find_for_user(&auth.user)
		.filter(media::Column::Id.eq(media_id))
		.into_model::<media::ModelWithMetadata>()
		.one(core.conn.as_ref())
		.await?
		.ok_or("Media not found")?;
	let locked = model
		.metadata
		.as_ref()
		.and_then(|metadata| metadata.locked_fields.clone())
		.and_then(|value| serde_json::from_value::<Vec<MetadataField>>(value).ok())
		.unwrap_or_default();
	if let Some(field) = selected_fields.iter().find(|field| locked.contains(field)) {
		return Err(format!("metadata field {field:?} is locked").into());
	}
	let mut active = if let Some(metadata) = model.metadata {
		metadata.into_active_model()
	} else {
		media_metadata::ActiveModel {
			media_id: Set(Some(media_id.to_owned())),
			..Default::default()
		}
	};
	for field in selected_fields {
		let value = input_value(*field, input)
			.ok_or_else(|| format!("field {field:?} is not supported by this editor"))?;
		apply_media_field(&mut active, *field, value);
	}
	active.media_id = Set(Some(media_id.to_owned()));
	active.save(core.conn.as_ref()).await?;
	Ok(())
}

async fn apply_external_metadata(
	core: &CoreContext,
	user: &models::entity::user::AuthUser,
	media_id: &str,
	scope: BookMetadataScope,
	selected_fields: &[MetadataField],
	external: &ExternalMediaMetadata,
) -> Result<()> {
	let detail = load_book_detail(core, user, media_id)
		.await?
		.ok_or("Media not found")?;
	if selected_fields.is_empty() {
		return Err("selected_fields must not be empty".into());
	}
	if scope == BookMetadataScope::Work {
		let Some(work_id) = detail.work_id else {
			return Err("WORK scope requires a confirmed work".into());
		};
		let mut work = book_work_metadata::Entity::find_by_id(work_id.to_string())
			.filter(book_work_metadata::Column::UserId.eq(user.id.clone()))
			.one(core.conn.as_ref())
			.await?
			.map(IntoActiveModel::into_active_model)
			.unwrap_or_else(|| book_work_metadata::ActiveModel {
				work_id: Set(work_id.to_string()),
				user_id: Set(user.id.clone()),
				..Default::default()
			});
		let mut metadata = match work.metadata.clone() {
			Set(Some(value)) => value,
			_ => serde_json::json!({}),
		};
		for field in selected_fields {
			let value = external_value(*field, external)
				.ok_or_else(|| format!("candidate does not provide {field:?}"))?;
			match field {
				MetadataField::Title => work.title = Set(value_to_string(value)),
				MetadataField::Writers => work.author = Set(value_to_string(value)),
				_ => metadata[metadata_field_name(*field)] = value,
			}
		}
		work.metadata = Set(Some(metadata));
		work.save(core.conn.as_ref()).await?;
		return Ok(());
	}
	let targets: Vec<_> = detail
		.editions
		.iter()
		.filter(|edition| match scope {
			BookMetadataScope::Ebook => edition.kind == BookEditionKind::Ebook,
			BookMetadataScope::Audiobook => edition.kind == BookEditionKind::Audiobook,
			BookMetadataScope::Both => {
				edition.kind == BookEditionKind::Ebook
					|| edition.kind == BookEditionKind::Audiobook
			},
			_ => false,
		})
		.map(|edition| edition.media_id.to_string())
		.collect();
	if targets.is_empty() {
		return Err("Selected edition scope has no edition".into());
	}
	for target in targets {
		let model = media::ModelWithMetadata::find_for_user(user)
			.filter(media::Column::Id.eq(target.clone()))
			.into_model::<media::ModelWithMetadata>()
			.one(core.conn.as_ref())
			.await?
			.ok_or("Media not found")?;
		let locked = model
			.metadata
			.as_ref()
			.and_then(|metadata| metadata.locked_fields.clone())
			.and_then(|value| serde_json::from_value::<Vec<MetadataField>>(value).ok())
			.unwrap_or_default();
		if let Some(field) = selected_fields.iter().find(|field| locked.contains(field)) {
			return Err(format!("metadata field {field:?} is locked").into());
		}
		let mut active = model
			.metadata
			.map(IntoActiveModel::into_active_model)
			.unwrap_or_else(|| media_metadata::ActiveModel {
				media_id: Set(Some(target.clone())),
				..Default::default()
			});
		for field in selected_fields {
			let value = external_value(*field, external)
				.ok_or_else(|| format!("candidate does not provide {field:?}"))?;
			apply_media_field(&mut active, *field, value);
		}
		active.media_id = Set(Some(target));
		active.save(core.conn.as_ref()).await?;
	}
	Ok(())
}

fn value_to_string(value: serde_json::Value) -> Option<String> {
	match value {
		serde_json::Value::String(value) => Some(value),
		serde_json::Value::Array(values) => Some(
			values
				.into_iter()
				.filter_map(|value| value.as_str().map(str::to_owned))
				.collect::<Vec<_>>()
				.join(", "),
		),
		_ => None,
	}
}

fn string_value(value: Option<String>) -> serde_json::Value {
	value.map_or(serde_json::Value::Null, serde_json::Value::String)
}

fn list_value(value: Option<Vec<String>>) -> serde_json::Value {
	value.map_or(serde_json::Value::Null, |value| {
		serde_json::Value::Array(
			value.into_iter().map(serde_json::Value::String).collect(),
		)
	})
}

fn input_value(
	field: MetadataField,
	input: &MediaMetadataInput,
) -> Option<serde_json::Value> {
	Some(match field {
		MetadataField::Title => string_value(input.title.clone()),
		MetadataField::TitleSort => string_value(input.title_sort.clone()),
		MetadataField::Series => string_value(input.series.clone()),
		MetadataField::SeriesGroup => string_value(input.series_group.clone()),
		MetadataField::StoryArc => string_value(input.story_arc.clone()),
		MetadataField::StoryArcNumber => {
			input.story_arc_number.map_or(serde_json::Value::Null, |v| {
				serde_json::json!(v.to_string())
			})
		},
		MetadataField::Number => input.number.map_or(serde_json::Value::Null, |v| {
			serde_json::json!(v.to_string())
		}),
		MetadataField::VolumeCount => input
			.volume
			.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
		MetadataField::Summary => string_value(input.summary.clone()),
		MetadataField::Notes => string_value(input.notes.clone()),
		MetadataField::Genres => list_value(input.genres.clone()),
		MetadataField::Format => string_value(input.format.clone()),
		MetadataField::Year => input
			.year
			.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
		MetadataField::Writers => list_value(input.writers.clone()),
		MetadataField::Pencillers => list_value(input.pencillers.clone()),
		MetadataField::Inkers => list_value(input.inkers.clone()),
		MetadataField::Colorists => list_value(input.colorists.clone()),
		MetadataField::Letterers => list_value(input.letterers.clone()),
		MetadataField::CoverArtists => list_value(input.cover_artists.clone()),
		MetadataField::Editors => list_value(input.editors.clone()),
		MetadataField::Narrators => list_value(input.narrators.clone()),
		MetadataField::Publisher => string_value(input.publisher.clone()),
		MetadataField::Links => list_value(input.links.clone()),
		MetadataField::Characters => list_value(input.characters.clone()),
		MetadataField::Teams => list_value(input.teams.clone()),
		MetadataField::PageCount => input
			.page_count
			.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
		MetadataField::AgeRating => input
			.age_rating
			.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
		MetadataField::IdentifierAmazon => string_value(input.identifier_amazon.clone()),
		MetadataField::IdentifierCalibre => {
			string_value(input.identifier_calibre.clone())
		},
		MetadataField::IdentifierGoogle => string_value(input.identifier_google.clone()),
		MetadataField::Isbn => string_value(input.identifier_isbn.clone()),
		MetadataField::IdentifierUuid => string_value(input.identifier_uuid.clone()),
		MetadataField::Language => string_value(input.language.clone()),
		_ => return None,
	})
}

fn external_value(
	field: MetadataField,
	external: &ExternalMediaMetadata,
) -> Option<serde_json::Value> {
	Some(match field {
		MetadataField::Title => string_value(external.title.clone()),
		MetadataField::Summary => string_value(external.summary.clone()),
		MetadataField::Genres => list_value(external.genres.clone()),
		MetadataField::Writers => list_value(external.writers.clone()),
		MetadataField::Narrators => list_value(external.narrators.clone()),
		MetadataField::Publisher => string_value(external.publisher.clone()),
		MetadataField::Year => external
			.year
			.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
		MetadataField::PageCount => external
			.page_count
			.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
		MetadataField::Isbn => {
			string_value(external.isbn.clone().or(external.isbn_13.clone()))
		},
		MetadataField::Series => string_value(external.series_name.clone()),
		MetadataField::Number => external.number.map_or(serde_json::Value::Null, |v| {
			serde_json::json!(v.to_string())
		}),
		MetadataField::Colorists => list_value(external.colorists.clone()),
		MetadataField::Letterers => list_value(external.letterers.clone()),
		MetadataField::CoverArtists => list_value(external.cover_artists.clone()),
		MetadataField::Pencillers => list_value(external.artists.clone()),
		_ => return None,
	})
}

fn apply_media_field(
	active: &mut media_metadata::ActiveModel,
	field: MetadataField,
	value: serde_json::Value,
) {
	let string = value.as_str().map(str::to_owned);
	let list = value.as_array().map(|values| {
		values
			.iter()
			.filter_map(|value| value.as_str())
			.collect::<Vec<_>>()
			.join(", ")
	});
	match field {
		MetadataField::Title => active.title = Set(string),
		MetadataField::TitleSort => active.title_sort = Set(string),
		MetadataField::Series => active.series = Set(string),
		MetadataField::SeriesGroup => active.series_group = Set(string),
		MetadataField::StoryArc => active.story_arc = Set(string),
		MetadataField::StoryArcNumber => {
			active.story_arc_number = Set(string.and_then(|v| v.parse().ok()))
		},
		MetadataField::Number => active.number = Set(string.and_then(|v| v.parse().ok())),
		MetadataField::VolumeCount => {
			active.volume = Set(value.as_i64().and_then(|v| i32::try_from(v).ok()))
		},
		MetadataField::Summary => active.summary = Set(string),
		MetadataField::Notes => active.notes = Set(string),
		MetadataField::Genres => active.genres = Set(list),
		MetadataField::Format => active.format = Set(string),
		MetadataField::Year => {
			active.year = Set(value.as_i64().and_then(|v| i32::try_from(v).ok()))
		},
		MetadataField::Writers => active.writers = Set(list),
		MetadataField::Pencillers => active.pencillers = Set(list),
		MetadataField::Inkers => active.inkers = Set(list),
		MetadataField::Colorists => active.colorists = Set(list),
		MetadataField::Letterers => active.letterers = Set(list),
		MetadataField::CoverArtists => active.cover_artists = Set(list),
		MetadataField::Editors => active.editors = Set(list),
		MetadataField::Narrators => active.narrators = Set(list),
		MetadataField::Publisher => active.publisher = Set(string),
		MetadataField::Links => active.links = Set(list),
		MetadataField::Characters => active.characters = Set(list),
		MetadataField::Teams => active.teams = Set(list),
		MetadataField::PageCount => {
			active.page_count = Set(value.as_i64().and_then(|v| i32::try_from(v).ok()))
		},
		MetadataField::AgeRating => {
			active.age_rating = Set(value.as_i64().and_then(|v| i32::try_from(v).ok()))
		},
		MetadataField::IdentifierAmazon => active.identifier_amazon = Set(string),
		MetadataField::IdentifierCalibre => active.identifier_calibre = Set(string),
		MetadataField::IdentifierGoogle => active.identifier_google = Set(string),
		MetadataField::Isbn => active.identifier_isbn = Set(string),
		MetadataField::IdentifierMobiAsin => active.identifier_mobi_asin = Set(string),
		MetadataField::IdentifierUuid => active.identifier_uuid = Set(string),
		MetadataField::Language => active.language = Set(string),
		_ => {},
	}
}
