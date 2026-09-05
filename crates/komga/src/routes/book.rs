use std::{
	collections::{HashMap, HashSet},
	sync::Arc,
};

use crate::{
	sse::KomgaEvent, KomgaAuthor, KomgaBookMetadataUpdateRequest, KomgaWebLink,
	PatchValue,
};
use axum::{
	extract::Path,
	http::StatusCode,
	routing::{patch, post},
	Extension, Json, Router,
};
use chrono::Datelike;
use models::entity::{media, media_metadata, media_tag, series, tag, user::AuthUser};
use sea_orm::{
	prelude::*, ActiveValue::Set, DatabaseTransaction, IntoActiveModel, TransactionTrait,
};
use stump_auth::AuthContext;

use super::{KomgaBackend, KomgaEvents};
use crate::errors::{APIError, APIResult};

/// Komga book metadata compatibility routes.
///
/// Authentication is applied by the parent Komga router. Metadata writes are
/// deliberately restricted to users who can manage libraries, matching Komga's
/// ADMIN-level metadata permissions.
pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::<S>::new()
		.route("/api/v1/books/{id}/metadata", patch(patch_book_metadata))
		.route(
			"/api/v1/books/{id}/metadata/refresh",
			post(refresh_book_metadata),
		)
}
fn enforce_manage_library(auth: &AuthContext) -> APIResult<()> {
	auth.enforce_permissions(&[models::shared::enums::UserPermission::ManageLibrary])
		.map_err(|_| APIError::forbidden_discreet())
}

async fn find_visible_book(
	conn: &DatabaseConnection,
	user: &AuthUser,
	id: &str,
) -> APIResult<media::Model> {
	media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(id.to_owned()))
		.filter(media::Column::DeletedAt.is_null())
		.filter(series::Column::DeletedAt.is_null())
		.one(conn)
		.await?
		.ok_or_else(|| APIError::NotFound("Book not found".to_owned()))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AuthorColumns {
	writers: Vec<String>,
	pencillers: Vec<String>,
	inkers: Vec<String>,
	colorists: Vec<String>,
	letterers: Vec<String>,
	cover_artists: Vec<String>,
	editors: Vec<String>,
}

impl AuthorColumns {
	fn apply(&self, active: &mut media_metadata::ActiveModel) {
		active.writers = Set(csv_value(&self.writers));
		active.pencillers = Set(csv_value(&self.pencillers));
		active.inkers = Set(csv_value(&self.inkers));
		active.colorists = Set(csv_value(&self.colorists));
		active.letterers = Set(csv_value(&self.letterers));
		active.cover_artists = Set(csv_value(&self.cover_artists));
		active.editors = Set(csv_value(&self.editors));
	}
}

fn csv_value(values: &[String]) -> Option<String> {
	(!values.is_empty()).then(|| values.join(", "))
}

fn push_author(values: &mut Vec<String>, name: &str) {
	if !values.iter().any(|value| value == name) {
		values.push(name.to_owned());
	}
}

/// Map Komga's author roles back to the CSV columns used by Stump metadata.
///
/// Stump persists exactly these seven role columns, so accepting any other role
/// would make a PATCH silently lose data.
fn map_author_columns(authors: &[KomgaAuthor]) -> Result<AuthorColumns, String> {
	let mut columns = AuthorColumns::default();
	for author in authors {
		let name = author.name.trim();
		if name.is_empty() {
			return Err("authors contains an author with an empty name".to_owned());
		}
		let role = author.role.trim().to_ascii_lowercase();
		match role.as_str() {
			"writer" => push_author(&mut columns.writers, name),
			"penciller" => push_author(&mut columns.pencillers, name),
			"inker" => push_author(&mut columns.inkers, name),
			"colorist" => push_author(&mut columns.colorists, name),
			"letterer" => push_author(&mut columns.letterers, name),
			// Komga names this role `cover` (komga-client AuthorDto / Komf emits `COVER`);
			// Stump's CSV column is `cover_artists`. Accept both spellings.
			"cover" | "cover_artist" => push_author(&mut columns.cover_artists, name),
			"editor" => push_author(&mut columns.editors, name),
			_ => {
				return Err(format!(
					"authors role {:?} cannot be persisted",
					author.role
				))
			},
		}
	}
	Ok(columns)
}

fn links_csv_value(values: &[KomgaWebLink]) -> Option<String> {
	(!values.is_empty()).then(|| {
		values
			.iter()
			.map(|link| link.url.clone())
			.collect::<Vec<_>>()
			.join(", ")
	})
}

fn parse_number(value: &str) -> Result<Decimal, String> {
	value
		.parse::<Decimal>()
		.map_err(|_| format!("number value {:?} cannot be persisted", value))
}

fn number_sort_from_decimal(number: Option<Decimal>) -> f32 {
	number
		.and_then(|number| number.to_string().parse::<f32>().ok())
		.filter(|number| number.is_finite())
		.unwrap_or_default()
}

fn validate_number_patch(patch: &KomgaBookMetadataUpdateRequest) -> APIResult<()> {
	if let PatchValue::Some(value) = &patch.number {
		parse_number(value)
			.map(|_| ())
			.map_err(APIError::BadRequest)
	} else {
		Ok(())
	}
}

fn validate_number_sort(
	patch: &KomgaBookMetadataUpdateRequest,
	metadata: Option<&media_metadata::Model>,
) -> APIResult<()> {
	match &patch.number_sort {
		PatchValue::Unset => Ok(()),
		PatchValue::None => Err(APIError::BadRequest(
			"numberSort cannot be cleared independently; it is derived from number"
				.to_owned(),
		)),
		PatchValue::Some(value) => {
			let expected = match &patch.number {
				PatchValue::Unset => {
					number_sort_from_decimal(metadata.and_then(|item| item.number))
				},
				PatchValue::None => 0.0,
				PatchValue::Some(number) => {
					let number = parse_number(number).map_err(APIError::BadRequest)?;
					number_sort_from_decimal(Some(number))
				},
			};
			if *value == expected {
				Ok(())
			} else {
				Err(APIError::BadRequest(format!(
					"numberSort must equal number ({expected})"
				)))
			}
		},
	}
}

fn metadata_patch_has_changes(patch: &KomgaBookMetadataUpdateRequest) -> bool {
	!patch.title.is_unset()
		|| !patch.title_lock.is_unset()
		|| !patch.summary.is_unset()
		|| !patch.summary_lock.is_unset()
		|| !patch.number.is_unset()
		|| !patch.number_lock.is_unset()
		|| !patch.number_sort_lock.is_unset()
		|| !patch.release_date.is_unset()
		|| !patch.release_date_lock.is_unset()
		|| !patch.authors.is_unset()
		|| !patch.authors_lock.is_unset()
		|| !patch.tags_lock.is_unset()
		|| !patch.isbn.is_unset()
		|| !patch.isbn_lock.is_unset()
		|| !patch.links.is_unset()
		|| !patch.links_lock.is_unset()
}

fn lock_matches(value: &serde_json::Value, field: &str) -> bool {
	value
		.as_str()
		.is_some_and(|value| value.eq_ignore_ascii_case(field))
}

fn lock_matches_any(value: &serde_json::Value, fields: &[&str]) -> bool {
	fields.iter().any(|field| lock_matches(value, field))
}

fn apply_lock_patch(
	values: &mut Vec<serde_json::Value>,
	fields: &[&str],
	canonical: &str,
	patch: &PatchValue<bool>,
) {
	match patch {
		PatchValue::Unset => {},
		PatchValue::Some(true) => {
			if !values.iter().any(|value| lock_matches(value, canonical)) {
				values.push(serde_json::Value::String(canonical.to_owned()));
			}
		},
		PatchValue::Some(false) | PatchValue::None => {
			values.retain(|value| !lock_matches_any(value, fields));
		},
	}
}

fn lock_values(metadata: Option<&media_metadata::Model>) -> Vec<serde_json::Value> {
	metadata
		.and_then(|metadata| metadata.locked_fields.as_ref())
		.and_then(serde_json::Value::as_array)
		.cloned()
		.unwrap_or_default()
}

fn lock_patch_requested(patch: &KomgaBookMetadataUpdateRequest) -> bool {
	!patch.title_lock.is_unset()
		|| !patch.summary_lock.is_unset()
		|| !patch.number_lock.is_unset()
		|| !patch.number_sort_lock.is_unset()
		|| !patch.release_date_lock.is_unset()
		|| !patch.authors_lock.is_unset()
		|| !patch.tags_lock.is_unset()
		|| !patch.isbn_lock.is_unset()
		|| !patch.links_lock.is_unset()
}

fn apply_book_locks(
	active: &mut media_metadata::ActiveModel,
	metadata: Option<&media_metadata::Model>,
	patch: &KomgaBookMetadataUpdateRequest,
) {
	if !lock_patch_requested(patch) {
		return;
	}
	let mut values = lock_values(metadata);
	apply_lock_patch(&mut values, &["TITLE"], "TITLE", &patch.title_lock);
	apply_lock_patch(&mut values, &["SUMMARY"], "SUMMARY", &patch.summary_lock);
	apply_lock_patch(&mut values, &["NUMBER"], "NUMBER", &patch.number_lock);
	apply_lock_patch(&mut values, &["NUMBER"], "NUMBER", &patch.number_sort_lock);
	apply_lock_patch(
		&mut values,
		&["RELEASE_DATE", "YEAR"],
		"RELEASE_DATE",
		&patch.release_date_lock,
	);
	apply_lock_patch(
		&mut values,
		&[
			"WRITERS",
			"PENCILLERS",
			"INKERS",
			"COLORISTS",
			"LETTERERS",
			"COVER_ARTISTS",
			"EDITORS",
		],
		"WRITERS",
		&patch.authors_lock,
	);
	apply_lock_patch(&mut values, &["TAGS"], "TAGS", &patch.tags_lock);
	apply_lock_patch(&mut values, &["ISBN"], "ISBN", &patch.isbn_lock);
	apply_lock_patch(&mut values, &["LINKS"], "LINKS", &patch.links_lock);
	active.locked_fields = Set(Some(serde_json::Value::Array(values)));
}

fn unique_tag_names(values: &[String]) -> Vec<String> {
	let mut seen = HashSet::with_capacity(values.len());
	values
		.iter()
		.filter(|value| seen.insert((*value).clone()))
		.cloned()
		.collect()
}

async fn replace_book_tags(
	txn: &DatabaseTransaction,
	book_id: &str,
	desired: &[String],
) -> APIResult<()> {
	let desired = unique_tag_names(desired);
	let existing = if desired.is_empty() {
		Vec::new()
	} else {
		tag::Entity::find()
			.filter(tag::Column::Name.is_in(desired.clone()))
			.all(txn)
			.await?
	};
	let mut tag_ids = existing
		.into_iter()
		.map(|model| (model.name, model.id))
		.collect::<HashMap<_, _>>();

	let missing = desired
		.iter()
		.filter(|name| !tag_ids.contains_key(*name))
		.map(|name| tag::ActiveModel {
			name: Set((*name).clone()),
			..Default::default()
		})
		.collect::<Vec<_>>();
	if !missing.is_empty() {
		for model in tag::Entity::insert_many(missing)
			.exec_with_returning_many(txn)
			.await?
		{
			tag_ids.insert(model.name, model.id);
		}
	}

	media_tag::Entity::delete_many()
		.filter(media_tag::Column::MediaId.eq(book_id.to_owned()))
		.exec(txn)
		.await?;
	if !desired.is_empty() {
		let links = desired
			.iter()
			.filter_map(|name| tag_ids.get(name).copied())
			.map(|tag_id| media_tag::ActiveModel {
				media_id: Set(book_id.to_owned()),
				tag_id: Set(tag_id),
				..Default::default()
			})
			.collect::<Vec<_>>();
		if !links.is_empty() {
			media_tag::Entity::insert_many(links).exec(txn).await?;
		}
	}
	Ok(())
}

async fn patch_book_metadata(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Extension(events): Extension<KomgaEvents>,
	Json(patch): Json<KomgaBookMetadataUpdateRequest>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;
	validate_number_patch(&patch)?;
	let author_columns = match &patch.authors {
		PatchValue::Unset => None,
		PatchValue::None => Some(AuthorColumns::default()),
		PatchValue::Some(values) => {
			Some(map_author_columns(values).map_err(APIError::BadRequest)?)
		},
	};
	let user = auth.user();
	let book = find_visible_book(ctx.conn(), &user, &id).await?;
	let parent_series = if let Some(series_id) = book.series_id.as_deref() {
		series::Entity::find_for_user(&user)
			.filter(series::Column::Id.eq(series_id))
			.one(ctx.conn())
			.await?
	} else {
		None
	};
	let event_series_id = book.series_id.clone().unwrap_or_default();
	let event_library_id = parent_series
		.and_then(|series| series.library_id)
		.unwrap_or_default();
	let txn = ctx.conn().begin().await?;
	let existing_metadata = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.eq(book.id.clone()))
		.one(&txn)
		.await?;
	validate_number_sort(&patch, existing_metadata.as_ref())?;
	let had_metadata = existing_metadata.is_some();
	let mut active = match existing_metadata.as_ref() {
		Some(metadata) => metadata.clone().into_active_model(),
		None => media_metadata::ActiveModel {
			media_id: Set(Some(book.id.clone())),
			..Default::default()
		},
	};

	match &patch.title {
		PatchValue::Unset => {},
		PatchValue::None => active.title = Set(None),
		PatchValue::Some(value) => active.title = Set(Some(value.clone())),
	}
	match &patch.summary {
		PatchValue::Unset => {},
		PatchValue::None => active.summary = Set(None),
		PatchValue::Some(value) => active.summary = Set(Some(value.clone())),
	}
	match &patch.number {
		PatchValue::Unset => {},
		PatchValue::None => active.number = Set(None),
		PatchValue::Some(value) => {
			active.number = Set(Some(parse_number(value).map_err(APIError::BadRequest)?));
		},
	}
	match &patch.release_date {
		PatchValue::Unset => {},
		PatchValue::None => {
			active.year = Set(None);
			active.month = Set(None);
			active.day = Set(None);
		},
		PatchValue::Some(value) => {
			active.year = Set(Some(value.year()));
			active.month = Set(Some(value.month() as i32));
			active.day = Set(Some(value.day() as i32));
		},
	}
	if let Some(columns) = author_columns.as_ref() {
		columns.apply(&mut active);
	}
	match &patch.isbn {
		PatchValue::Unset => {},
		PatchValue::None => active.identifier_isbn = Set(None),
		PatchValue::Some(value) => active.identifier_isbn = Set(Some(value.clone())),
	}
	match &patch.links {
		PatchValue::Unset => {},
		PatchValue::None => active.links = Set(None),
		PatchValue::Some(value) => active.links = Set(links_csv_value(value)),
	}
	apply_book_locks(&mut active, existing_metadata.as_ref(), &patch);

	if had_metadata || metadata_patch_has_changes(&patch) {
		if had_metadata {
			active.update(&txn).await?;
		} else {
			active.insert(&txn).await?;
		}
	}
	if let Some(desired) = match &patch.tags {
		PatchValue::Unset => None,
		PatchValue::None => Some(Vec::new()),
		PatchValue::Some(values) => Some(values.clone()),
	} {
		replace_book_tags(&txn, &book.id, &desired).await?;
	}

	txn.commit().await?;
	events.send(KomgaEvent::BookChanged {
		book_id: book.id.into(),
		series_id: event_series_id.into(),
		library_id: event_library_id.into(),
	});
	Ok(StatusCode::NO_CONTENT)
}

async fn refresh_book_metadata(
	Path(id): Path<String>,
	Extension(ctx): Extension<Arc<dyn KomgaBackend>>,
	Extension(auth): Extension<AuthContext>,
) -> APIResult<StatusCode> {
	enforce_manage_library(&auth)?;
	let user = auth.user();
	let book = find_visible_book(ctx.conn(), &user, &id).await?;
	ctx.enqueue_book_analysis(book.id).await?;
	Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn patch_authors_map_to_mapper_role_columns() {
		let columns = map_author_columns(&[
			KomgaAuthor {
				name: "Writer A".to_owned(),
				role: "writer".to_owned(),
			},
			KomgaAuthor {
				name: "Artist".to_owned(),
				role: "penciller".to_owned(),
			},
			KomgaAuthor {
				name: "Writer A".to_owned(),
				role: "writer".to_owned(),
			},
			KomgaAuthor {
				name: "Editor".to_owned(),
				role: "editor".to_owned(),
			},
		])
		.unwrap();
		assert_eq!(columns.writers, vec!["Writer A"]);
		assert_eq!(columns.pencillers, vec!["Artist"]);
		assert_eq!(columns.editors, vec!["Editor"]);
		assert!(columns.inkers.is_empty());
	}

	#[test]
	fn patch_authors_accept_komga_cover_role_spellings() {
		// Komf sends `COVER` (Komga's role name); older clients may send `cover_artist`.
		let columns = map_author_columns(&[
			KomgaAuthor {
				name: "A".into(),
				role: "COVER".into(),
			},
			KomgaAuthor {
				name: "B".into(),
				role: "cover_artist".into(),
			},
		])
		.unwrap();
		assert_eq!(
			columns.cover_artists,
			vec!["A".to_string(), "B".to_string()]
		);
	}

	#[test]
	fn patch_authors_reject_unpersistable_role() {
		let error = map_author_columns(&[KomgaAuthor {
			name: "Artist".to_owned(),
			role: "artist".to_owned(),
		}])
		.unwrap_err();
		assert!(error.contains("authors"));
	}
}
