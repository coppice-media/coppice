//! `MetadataController` lookups used by the Kavita Tachiyomi extension's
//! filter sheet: genres, tags, age ratings, languages, people and
//! publication statuses.

use std::{collections::BTreeMap, sync::Arc};

use axum::{extract::Query, routing::get, Extension, Json, Router};
use models::entity::{
	media, media_metadata, media_tag, series, series_metadata, series_tag, tag,
	user::AuthUser,
};
use sea_orm::{prelude::*, QueryOrder, QuerySelect};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::{
		AgeRating, AgeRatingDto, GenreTagDto, LanguageDto, PersonDto, PersonRole,
		PublicationStatus, PublicationStatusDto, TagDto,
	},
	errors::APIResult,
	ids::{IdKind, KavitaIds},
	mapper::{name_id, split_csv},
};

use super::{route_ci, KavitaBackend};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryIdsQuery {
	/// Comma-separated Kavita library ids; empty means every visible library.
	#[serde(default)]
	library_ids: Option<String>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(router, "/api/Metadata/genres", get(genres));
	let router = route_ci(router, "/api/Metadata/tags", get(tags));
	let router = route_ci(router, "/api/Metadata/age-ratings", get(age_ratings));
	let router = route_ci(router, "/api/Metadata/languages", get(languages));
	let router = route_ci(router, "/api/Metadata/people", get(people));
	route_ci(
		router,
		"/api/Metadata/publication-status",
		get(publication_status),
	)
}

/// Stump library ids selected by the `libraryIds` query, if any.
async fn library_scope(
	ctx: &dyn KavitaBackend,
	query: &LibraryIdsQuery,
) -> APIResult<Option<Vec<String>>> {
	let Some(raw) = query.library_ids.as_deref() else {
		return Ok(None);
	};
	let ids = raw
		.split(',')
		.filter_map(|value| value.trim().parse::<i32>().ok())
		.collect::<Vec<_>>();
	if ids.is_empty() {
		return Ok(None);
	}
	let mut stump_ids = Vec::with_capacity(ids.len());
	for id in ids {
		if let Some(stump_id) = KavitaIds::lookup(ctx.conn(), IdKind::Library, id).await?
		{
			stump_ids.push(stump_id);
		}
	}
	Ok(Some(stump_ids))
}

fn scoped_series(user: &AuthUser, scope: Option<Vec<String>>) -> Select<series::Entity> {
	let query =
		series::Entity::find_for_user(user).filter(series::Column::DeletedAt.is_null());
	match scope {
		Some(ids) => query.filter(series::Column::LibraryId.is_in(ids)),
		None => query,
	}
}

/// Distinct values of a series-metadata column and a media-metadata column
/// across the user's visible series, keyed case-insensitively.
pub(crate) async fn distinct_values(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	scope: Option<Vec<String>>,
	series_column: Option<series_metadata::Column>,
	media_column: Option<media_metadata::Column>,
) -> APIResult<Vec<String>> {
	let series_ids = scoped_series(user, scope)
		.select_only()
		.column(series::Column::Id)
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;
	let mut values: BTreeMap<String, String> = BTreeMap::new();
	let mut push = |raw: Option<String>| {
		for value in split_csv(raw.as_deref()) {
			values.entry(value.to_lowercase()).or_insert(value);
		}
	};
	if let Some(column) = series_column {
		for chunk in series_ids.chunks(500) {
			let rows = series_metadata::Entity::find()
				.select_only()
				.column(column)
				.filter(series_metadata::Column::SeriesId.is_in(chunk.to_vec()))
				.into_tuple::<Option<String>>()
				.all(ctx.conn())
				.await?;
			rows.into_iter().for_each(&mut push);
		}
	}
	if let Some(column) = media_column {
		for chunk in series_ids.chunks(500) {
			let rows = media_metadata::Entity::find()
				.select_only()
				.column(column)
				.inner_join(media::Entity)
				.filter(media::Column::SeriesId.is_in(chunk.to_vec()))
				.filter(media::Column::DeletedAt.is_null())
				.into_tuple::<Option<String>>()
				.all(ctx.conn())
				.await?;
			rows.into_iter().for_each(&mut push);
		}
	}
	Ok(values.into_values().collect())
}

async fn genres(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LibraryIdsQuery>,
) -> APIResult<Json<Vec<GenreTagDto>>> {
	let user = auth.user();
	let scope = library_scope(ctx.as_ref(), &query).await?;
	let values = distinct_values(
		ctx.as_ref(),
		&user,
		scope,
		Some(series_metadata::Column::Genres),
		Some(media_metadata::Column::Genres),
	)
	.await?;
	Ok(Json(
		values
			.into_iter()
			.map(|title| GenreTagDto {
				id: name_id(&title),
				title,
			})
			.collect(),
	))
}

/// The tags attached to the user's visible series and to their media,
/// ordered by title: a Kavita series' `tags` list is the union of both
/// (`map_series_metadata`), and a book series carries only its file's. The
/// search `tags` group narrows the same list, so a tag found by one is found
/// by the other.
pub(crate) async fn visible_tags(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	scope: Option<Vec<String>>,
) -> APIResult<Vec<TagDto>> {
	let series_ids = scoped_series(user, scope)
		.select_only()
		.column(series::Column::Id)
		.into_tuple::<String>()
		.all(ctx.conn())
		.await?;
	let mut seen = std::collections::BTreeSet::new();
	let mut result = Vec::new();
	for chunk in series_ids.chunks(500) {
		let tagged_series = sea_orm::sea_query::Query::select()
			.column(series_tag::Column::TagId)
			.from(series_tag::Entity)
			.and_where(series_tag::Column::SeriesId.is_in(chunk.to_vec()))
			.to_owned();
		let tagged_media = sea_orm::sea_query::Query::select()
			.column(media_tag::Column::TagId)
			.from(media_tag::Entity)
			.and_where(
				media_tag::Column::MediaId.in_subquery(
					sea_orm::sea_query::Query::select()
						.column(media::Column::Id)
						.from(media::Entity)
						.and_where(media::Column::DeletedAt.is_null())
						.and_where(media::Column::SeriesId.is_in(chunk.to_vec()))
						.to_owned(),
				),
			)
			.to_owned();
		let rows = tag::Entity::find()
			.filter(
				tag::Column::Id
					.in_subquery(tagged_series)
					.or(tag::Column::Id.in_subquery(tagged_media)),
			)
			.order_by_asc(tag::Column::Name)
			.all(ctx.conn())
			.await?;
		for row in rows {
			if seen.insert(row.id) {
				result.push(TagDto {
					id: row.id,
					title: row.name,
				});
			}
		}
	}
	result.sort_by(|left, right| {
		left.title.to_lowercase().cmp(&right.title.to_lowercase())
	});
	Ok(result)
}

async fn tags(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LibraryIdsQuery>,
) -> APIResult<Json<Vec<TagDto>>> {
	let user = auth.user();
	let scope = library_scope(ctx.as_ref(), &query).await?;
	Ok(Json(visible_tags(ctx.as_ref(), &user, scope).await?))
}

/// Kavita lists every rating except `NotApplicable`, in enum order.
async fn age_ratings(
	Extension(_ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(_auth): Extension<AuthContext>,
) -> Json<Vec<AgeRatingDto>> {
	Json(
		AgeRating::ALL
			.iter()
			.filter(|rating| **rating != AgeRating::NotApplicable)
			.map(|rating| AgeRatingDto {
				value: *rating,
				title: rating.title().to_owned(),
			})
			.collect(),
	)
}

/// ISO 639-1 titles for the languages stored on visible series. Kavita
/// renders the culture display name; the common codes are listed here and
/// anything else echoes its code.
fn language_title(code: &str) -> String {
	let base = code
		.split(['-', '_'])
		.next()
		.unwrap_or(code)
		.to_ascii_lowercase();
	let title = match base.as_str() {
		"en" => "English",
		"ja" => "Japanese",
		"ko" => "Korean",
		"zh" => "Chinese",
		"fr" => "French",
		"de" => "German",
		"es" => "Spanish",
		"it" => "Italian",
		"pt" => "Portuguese",
		"ru" => "Russian",
		"nl" => "Dutch",
		"pl" => "Polish",
		"sv" => "Swedish",
		"tr" => "Turkish",
		"ar" => "Arabic",
		"id" => "Indonesian",
		"vi" => "Vietnamese",
		"th" => "Thai",
		"uk" => "Ukrainian",
		"cs" => "Czech",
		"hu" => "Hungarian",
		"fi" => "Finnish",
		"da" => "Danish",
		"no" | "nb" => "Norwegian",
		"el" => "Greek",
		"he" => "Hebrew",
		"hi" => "Hindi",
		"ro" => "Romanian",
		"bg" => "Bulgarian",
		"ca" => "Catalan",
		_ => return code.to_owned(),
	};
	title.to_owned()
}

async fn languages(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LibraryIdsQuery>,
) -> APIResult<Json<Vec<LanguageDto>>> {
	let user = auth.user();
	let scope = library_scope(ctx.as_ref(), &query).await?;
	let values = distinct_values(
		ctx.as_ref(),
		&user,
		scope,
		Some(series_metadata::Column::Language),
		Some(media_metadata::Column::Language),
	)
	.await?;
	Ok(Json(
		values
			.into_iter()
			.map(|code| LanguageDto {
				title: language_title(&code),
				iso_code: code,
			})
			.collect(),
	))
}

/// Every people role Stump records, with the metadata columns that carry it.
/// `GET /api/Metadata/people` and the search `persons` group walk the same
/// list, so a name found by one is found by the other.
pub(crate) fn people_sources() -> [(
	Option<series_metadata::Column>,
	Option<media_metadata::Column>,
	PersonRole,
); 11] {
	[
		(
			Some(series_metadata::Column::Writers),
			Some(media_metadata::Column::Writers),
			PersonRole::Writer,
		),
		(
			Some(series_metadata::Column::Publisher),
			Some(media_metadata::Column::Publisher),
			PersonRole::Publisher,
		),
		(
			Some(series_metadata::Column::Characters),
			Some(media_metadata::Column::Characters),
			PersonRole::Character,
		),
		(
			Some(series_metadata::Column::Imprint),
			None,
			PersonRole::Imprint,
		),
		(
			None,
			Some(media_metadata::Column::Pencillers),
			PersonRole::Penciller,
		),
		(
			None,
			Some(media_metadata::Column::Inkers),
			PersonRole::Inker,
		),
		(
			None,
			Some(media_metadata::Column::Colorists),
			PersonRole::Colorist,
		),
		(
			None,
			Some(media_metadata::Column::Letterers),
			PersonRole::Letterer,
		),
		(
			None,
			Some(media_metadata::Column::CoverArtists),
			PersonRole::CoverArtist,
		),
		(
			None,
			Some(media_metadata::Column::Editors),
			PersonRole::Editor,
		),
		(None, Some(media_metadata::Column::Teams), PersonRole::Team),
	]
}

async fn people(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<LibraryIdsQuery>,
) -> APIResult<Json<Vec<PersonDto>>> {
	let user = auth.user();
	let scope = library_scope(ctx.as_ref(), &query).await?;
	let mut people: BTreeMap<String, PersonDto> = BTreeMap::new();
	for (series_column, media_column, role) in people_sources() {
		let values = distinct_values(
			ctx.as_ref(),
			&user,
			scope.clone(),
			series_column,
			media_column,
		)
		.await?;
		for name in values {
			let entry = people.entry(name.to_lowercase()).or_insert_with(|| {
				PersonDto::new(name_id(&name), name.clone(), Vec::new())
			});
			if !entry.roles.contains(&role) {
				entry.roles.push(role);
			}
		}
	}
	Ok(Json(people.into_values().collect()))
}

/// Kavita orders these by title.
async fn publication_status(
	Extension(_ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(_auth): Extension<AuthContext>,
) -> Json<Vec<PublicationStatusDto>> {
	let mut statuses = PublicationStatus::ALL
		.iter()
		.map(|status| PublicationStatusDto {
			value: *status,
			title: status.title().to_owned(),
		})
		.collect::<Vec<_>>();
	statuses.sort_by(|left, right| left.title.cmp(&right.title));
	Json(statuses)
}
