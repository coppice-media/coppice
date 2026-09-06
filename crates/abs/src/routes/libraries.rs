//! `GET /api/libraries` and everything hanging off one library: the item
//! page, the personalized shelves, the author list and search.

use std::collections::{HashMap, HashSet};

use axum::{
	extract::{Json, Path, Query},
	response::{IntoResponse, Response},
	Extension,
};
use models::entity::{library, media, media_metadata, series, user::AuthUser};
use sea_orm::{
	ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
};
use serde::Deserialize;

use crate::{
	dto::*,
	errors::AbsResult,
	mapper::{self, ts_ms},
	model::ItemShape,
	routes::{
		query::{self, ItemFilter, ItemSort},
		AbsBackend, Backend, User,
	},
};

/// `GET /api/libraries/{id}/items`. Lissen sends every one of these
/// (`AudiobookshelfApiClient.kt:80-90`); `include` and `expanded` come from
/// other ABS clients.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ItemsQuery {
	limit: Option<u64>,
	page: Option<u64>,
	sort: Option<String>,
	/// `"0"`/`"1"` on the wire, not a JSON bool.
	desc: Option<String>,
	minified: Option<String>,
	filter: Option<String>,
	collapseseries: Option<String>,
	include: Option<String>,
}

/// abs-ref treats any of `1`/`true` as set and everything else as unset.
fn flag(value: Option<&String>) -> bool {
	value
		.map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
		.unwrap_or(false)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct IncludeQuery {
	include: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AuthorsQuery {
	limit: Option<u64>,
	page: Option<u64>,
	sort: Option<String>,
	desc: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SearchQuery {
	q: Option<String>,
	limit: Option<u64>,
}

/// The libraries a client is offered: the user's visible libraries that hold
/// at least one audible book, in creation order, which is the `displayOrder`
/// Lissen sorts by (`common/AudiobookshelfChannel.kt:83-86`).
async fn visible_libraries(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<Vec<library::Model>> {
	let with_audio = query::audio_library_ids(backend, user).await?;
	let libraries = library::Entity::find_for_user(user)
		.order_by_asc(library::Column::CreatedAt)
		.order_by_asc(library::Column::Id)
		.all(backend.conn())
		.await?;
	Ok(libraries
		.into_iter()
		.filter(|library| with_audio.contains(&library.id))
		.collect())
}

async fn library_dto(
	backend: &dyn AbsBackend,
	library: &library::Model,
	display_order: i32,
) -> AbsResult<LibraryDto> {
	Ok(mapper::library_dto(mapper::LibraryInput {
		id: &library.id,
		name: &library.name,
		path: &library.path,
		folder_id: &backend.folder_id(&library.id).await?,
		display_order,
		created_at: library.created_at.into(),
		updated_at: library.updated_at.map(Into::into),
		last_scanned_at: library.last_scanned_at.map(Into::into),
	}))
}

pub(crate) async fn list(
	backend: Backend,
	Extension(user): User,
) -> AbsResult<Json<LibrariesResponseDto>> {
	let mut libraries = Vec::new();
	for (index, library) in visible_libraries(&**backend, &user)
		.await?
		.iter()
		.enumerate()
	{
		libraries.push(library_dto(&**backend, library, index as i32 + 1).await?);
	}
	Ok(Json(LibrariesResponseDto { libraries }))
}

/// The distinct values of one `media_metadata` text column across a library,
/// for `filterdata`.
async fn library_metadata(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	library_id: &str,
) -> AbsResult<Vec<media_metadata::Model>> {
	let ids = query::audio_media(user)
		.filter(series::Column::LibraryId.eq(library_id))
		.select_only()
		.column(media::Column::Id)
		.into_tuple::<String>()
		.all(backend.conn())
		.await?;
	Ok(media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.is_in(ids))
		.all(backend.conn())
		.await?)
}

/// `GET /api/libraries/{id}`. Lissen always asks for `include=filterdata`
/// (`AudiobookshelfApiClient.kt:43-47`) and decodes `{library, filterdata}`;
/// without the parameter abs-ref answers the bare library object.
pub(crate) async fn detail(
	backend: Backend,
	Extension(user): User,
	Path(library_id): Path<String>,
	Query(params): Query<IncludeQuery>,
) -> AbsResult<Response> {
	let library = query::library(&**backend, &user, &library_id).await?;
	let display_order = visible_libraries(&**backend, &user)
		.await?
		.iter()
		.position(|candidate| candidate.id == library.id)
		.map(|index| index as i32 + 1)
		.unwrap_or(1);
	let dto = library_dto(&**backend, &library, display_order).await?;

	let wants_filterdata = params
		.include
		.as_deref()
		.map(|include| include.split(',').any(|part| part.trim() == "filterdata"))
		.unwrap_or(false);
	if !wants_filterdata {
		return Ok(Json(dto).into_response());
	}

	let metadata = library_metadata(&**backend, &user, &library_id).await?;
	let mut authors = Vec::new();
	let mut author_names = Vec::new();
	for name in metadata
		.iter()
		.flat_map(|row| mapper::csv(row.writers.as_deref()))
	{
		if !author_names.contains(&name) {
			author_names.push(name);
		}
	}
	let author_ids = backend.author_ids(&author_names).await?;
	for name in author_names {
		authors.push(NamedIdDto {
			id: author_ids
				.get(&name)
				.cloned()
				.unwrap_or_else(|| name.clone()),
			name,
		});
	}

	let series_rows = series::Entity::find()
		.filter(series::Column::LibraryId.eq(library_id.as_str()))
		.order_by_asc(series::Column::Name)
		.all(backend.conn())
		.await?;

	let distinct = |values: Vec<String>| {
		let mut seen = Vec::new();
		for value in values {
			if !value.is_empty() && !seen.contains(&value) {
				seen.push(value);
			}
		}
		seen
	};

	let filterdata = mapper::filter_data(mapper::FilterDataInput {
		authors,
		genres: distinct(
			metadata
				.iter()
				.flat_map(|row| mapper::csv(row.genres.as_deref()))
				.collect(),
		),
		tags: Vec::new(),
		series: series_rows
			.iter()
			.map(|row| NamedIdDto {
				id: row.id.clone(),
				name: row.name.clone(),
			})
			.collect(),
		narrators: Vec::new(),
		languages: distinct(
			metadata
				.iter()
				.filter_map(|row| row.language.clone())
				.collect(),
		),
		publishers: distinct(
			metadata
				.iter()
				.filter_map(|row| row.publisher.clone())
				.collect(),
		),
		published_years: metadata.iter().filter_map(|row| row.year).collect(),
		book_count: metadata.len() as i64,
	});

	Ok(Json(LibraryWithFilterDataDto {
		filterdata,
		issues: 0,
		num_user_playlists: 0,
		library: dto,
	})
	.into_response())
}

/// The series a `collapseseries=1` page groups by, and the books in each.
///
/// abs-ref groups across the whole library, not across the requested page, so
/// the grouping is resolved from a light projection of the library before the
/// page is cut.
async fn collapse_groups(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	library_id: &str,
) -> AbsResult<HashMap<String, Vec<String>>> {
	#[derive(FromQueryResult)]
	struct Row {
		id: String,
		series_id: Option<String>,
	}

	let rows = query::audio_media(user)
		.filter(series::Column::LibraryId.eq(library_id))
		.select_only()
		.column(media::Column::Id)
		.column(media::Column::SeriesId)
		.order_by_asc(media::Column::Name)
		.into_model::<Row>()
		.all(backend.conn())
		.await?;

	let mut groups: HashMap<String, Vec<String>> = HashMap::new();
	for row in rows {
		if let Some(series_id) = row.series_id {
			groups.entry(series_id).or_default().push(row.id);
		}
	}
	Ok(groups)
}

pub(crate) async fn items(
	backend: Backend,
	Extension(user): User,
	Path(library_id): Path<String>,
	Query(params): Query<ItemsQuery>,
) -> AbsResult<Json<LibraryItemsPageDto>> {
	// The library must exist and be visible even when it holds nothing:
	// abs-ref answers 404 for an unknown id (`capture/negatives.txt`).
	query::library(&**backend, &user, &library_id).await?;

	let sort = ItemSort::parse(params.sort.as_deref().unwrap_or_default());
	let desc = flag(params.desc.as_ref());
	let minified = flag(params.minified.as_ref());
	let collapse = flag(params.collapseseries.as_ref());
	let expanded = params
		.include
		.as_deref()
		.map(|include| include.contains("progress"))
		.unwrap_or(false);
	let filter = params.filter.as_deref().and_then(ItemFilter::parse);
	let limit = params.limit.unwrap_or(0);
	let page_number = params.page.unwrap_or(0);

	let shape = if minified {
		ItemShape::Minified
	} else if expanded {
		ItemShape::Expanded
	} else {
		ItemShape::Detail
	};

	let groups = if collapse {
		collapse_groups(&**backend, &user, &library_id).await?
	} else {
		HashMap::new()
	};

	// Under `collapseseries=1` a whole series is one row, so the page is cut
	// after grouping; otherwise the database does the paging.
	let (rows, total) = if collapse {
		let all = query::item_page(
			&**backend,
			&user,
			&library_id,
			sort,
			desc,
			filter.as_ref(),
			0,
			0,
		)
		.await?;
		let mut seen = HashSet::new();
		let grouped = all
			.rows
			.into_iter()
			.filter(|row| match row.series_id.as_ref() {
				// A single-book folder is not a series, so it is never
				// folded away; see `ItemContext::series_of`.
				Some(series_id) if groups.get(series_id).map_or(0, Vec::len) > 1 => {
					seen.insert(series_id.clone())
				},
				_ => true,
			})
			.collect::<Vec<_>>();
		let total = grouped.len() as i64;
		let rows = if limit > 0 {
			grouped
				.into_iter()
				.skip((page_number * limit) as usize)
				.take(limit as usize)
				.collect()
		} else {
			grouped
		};
		(rows, total)
	} else {
		let page = query::item_page(
			&**backend,
			&user,
			&library_id,
			sort,
			desc,
			filter.as_ref(),
			limit,
			page_number,
		)
		.await?;
		(page.rows, page.total)
	};

	let context = query::context(&**backend, &user, &rows, expanded).await?;
	let results = rows
		.iter()
		.map(|row| {
			let collapsed = collapse.then(|| {
				let (series_id, _) = context.series_of(row)?;
				let series = context.series.get(series_id)?;
				let members = groups.get(series_id)?;
				Some(CollapsedSeriesDto {
					id: series.id.clone(),
					name: series.name.clone(),
					name_ignore_prefix: mapper::ignore_prefix(&series.name),
					sequence: mapper::sequence(context.metadata.get(&row.id)),
					num_books: members.len() as i64,
					library_item_ids: members.clone(),
				})
			});
			context.item(row, shape, &user.id, collapsed)
		})
		.collect();

	Ok(Json(LibraryItemsPageDto {
		results,
		total,
		limit: limit as i64,
		page: page_number as i64,
		sort_by: sort.wire_value().to_owned(),
		sort_desc: desc,
		media_type: "book".to_owned(),
		minified,
		collapse_series: collapse,
		include: params.include.unwrap_or_default(),
		offset: (page_number * limit) as i64,
	}))
}

/// `GET /api/libraries/{id}/personalized`. Lissen reads exactly one shelf,
/// the one whose `labelStringKey` is `LabelContinueListening`
/// (`common/converter/RecentListeningResponseConverter.kt:30`), and needs
/// minified items on it. The other two shelves are the ones abs-ref puts
/// beside it for a book library; its `recent-series` and `newest-authors`
/// shelves carry non-item entities no inventoried client reads, so they are
/// not served.
pub(crate) async fn personalized(
	backend: Backend,
	Extension(user): User,
	Path(library_id): Path<String>,
) -> AbsResult<Json<Vec<PersonalizedShelfDto>>> {
	query::library(&**backend, &user, &library_id).await?;

	let all = query::item_page(
		&**backend,
		&user,
		&library_id,
		ItemSort::AddedAt,
		true,
		None,
		0,
		0,
	)
	.await?;
	let context = query::context(&**backend, &user, &all.rows, true).await?;

	let mut in_progress = all
		.rows
		.iter()
		.filter_map(|row| {
			let progress = context.progress.get(&row.id)?;
			(!progress.is_finished && progress.position_ms > 0)
				.then_some((row, progress.last_update))
		})
		.collect::<Vec<_>>();
	in_progress.sort_by_key(|(_, last_update)| std::cmp::Reverse(ts_ms(*last_update)));

	let shelf = |id: &str, label: &str, key: &str, rows: Vec<&media::Model>| {
		let entities = rows
			.iter()
			.map(|row| {
				PersonalizedEntityDto::Item(Box::new(context.item(
					row,
					ItemShape::Minified,
					&user.id,
					None,
				)))
			})
			.collect::<Vec<_>>();
		PersonalizedShelfDto {
			id: id.to_owned(),
			label: label.to_owned(),
			label_string_key: key.to_owned(),
			shelf_type: "book".to_owned(),
			total: entities.len() as i64,
			entities,
		}
	};

	let mut shelves = Vec::new();
	if !in_progress.is_empty() {
		shelves.push(shelf(
			"continue-listening",
			"Continue Listening",
			"LabelContinueListening",
			in_progress.iter().map(|(row, _)| *row).collect(),
		));
	}
	shelves.push(shelf(
		"recently-added",
		"Recently Added",
		"LabelRecentlyAdded",
		all.rows.iter().take(10).collect(),
	));
	let untouched = all
		.rows
		.iter()
		.filter(|row| !context.progress.contains_key(&row.id))
		.take(10)
		.collect::<Vec<_>>();
	if !untouched.is_empty() {
		shelves.push(shelf("discover", "Discover", "LabelDiscover", untouched));
	}
	Ok(Json(shelves))
}

/// One author of a library: Stump has no author row, so an author is a
/// distinct `media_metadata.writers` value with an id allocated in
/// `abs_ids`.
pub(crate) struct AuthorFacts {
	pub name: String,
	pub num_books: i64,
	pub added_at: i64,
}

pub(crate) async fn author_facts(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	library_id: &str,
) -> AbsResult<Vec<AuthorFacts>> {
	#[derive(FromQueryResult)]
	struct Row {
		writers: Option<String>,
		created_at: sea_orm::prelude::DateTimeWithTimeZone,
	}

	let rows = query::audio_media(user)
		.filter(series::Column::LibraryId.eq(library_id))
		.select_only()
		.column(media_metadata::Column::Writers)
		.column(media::Column::CreatedAt)
		.into_model::<Row>()
		.all(backend.conn())
		.await?;

	let mut facts: Vec<AuthorFacts> = Vec::new();
	for row in rows {
		let added_at = row.created_at.timestamp_millis();
		for name in mapper::csv(row.writers.as_deref()) {
			match facts.iter_mut().find(|fact| fact.name == name) {
				Some(fact) => {
					fact.num_books += 1;
					fact.added_at = fact.added_at.min(added_at);
				},
				None => facts.push(AuthorFacts {
					name,
					num_books: 1,
					added_at,
				}),
			}
		}
	}
	facts.sort_by(|left, right| left.name.cmp(&right.name));
	Ok(facts)
}

pub(crate) fn author_dto(
	id: String,
	facts: &AuthorFacts,
	library_id: &str,
	with_counts: bool,
) -> AuthorDto {
	AuthorDto {
		id,
		asin: None,
		name: facts.name.clone(),
		description: None,
		image_path: None,
		library_id: library_id.to_owned(),
		added_at: facts.added_at,
		updated_at: facts.added_at,
		num_books: with_counts.then_some(facts.num_books),
		last_first: with_counts.then(|| mapper::last_first(&facts.name)),
		library_items: None,
	}
}

pub(crate) async fn authors(
	backend: Backend,
	Extension(user): User,
	Path(library_id): Path<String>,
	Query(params): Query<AuthorsQuery>,
) -> AbsResult<Json<AuthorsPageDto>> {
	query::library(&**backend, &user, &library_id).await?;

	let mut facts = author_facts(&**backend, &user, &library_id).await?;
	let desc = flag(params.desc.as_ref());
	if desc {
		facts.reverse();
	}
	let total = facts.len() as i64;
	let limit = params.limit.unwrap_or(0);
	let page = params.page.unwrap_or(0);
	let window = if limit > 0 {
		facts
			.into_iter()
			.skip((page * limit) as usize)
			.take(limit as usize)
			.collect::<Vec<_>>()
	} else {
		facts
	};

	let names = window
		.iter()
		.map(|fact| fact.name.clone())
		.collect::<Vec<_>>();
	let ids = backend.author_ids(&names).await?;
	let results = window
		.iter()
		.map(|fact| {
			author_dto(
				ids.get(&fact.name)
					.cloned()
					.unwrap_or_else(|| fact.name.clone()),
				fact,
				&library_id,
				true,
			)
		})
		.collect();

	Ok(Json(AuthorsPageDto {
		results,
		total,
		limit: limit as i64,
		page: page as i64,
		sort_by: params.sort.unwrap_or_else(|| "name".to_owned()),
		sort_desc: desc,
		minified: false,
	}))
}

/// `GET /api/libraries/{id}/search`. Lissen searches titles and fans the
/// author and series hits out into further requests
/// (`library/LibraryAudiobookshelfChannel.kt:181-238`), so all three arrays
/// are filled; `narrators`, `tags` and `genres` are always empty, which is
/// what abs-ref answers for a library with none.
pub(crate) async fn search(
	backend: Backend,
	Extension(user): User,
	Path(library_id): Path<String>,
	Query(params): Query<SearchQuery>,
) -> AbsResult<Json<SearchResultDto>> {
	query::library(&**backend, &user, &library_id).await?;

	let needle = params.q.unwrap_or_default().trim().to_lowercase();
	let limit = params.limit.unwrap_or(12).max(1) as usize;
	if needle.is_empty() {
		return Ok(Json(SearchResultDto {
			book: Vec::new(),
			narrators: Vec::new(),
			tags: Vec::new(),
			genres: Vec::new(),
			series: Vec::new(),
			authors: Vec::new(),
		}));
	}

	let all = query::item_page(
		&**backend,
		&user,
		&library_id,
		ItemSort::Title,
		false,
		None,
		0,
		0,
	)
	.await?;
	let context = query::context(&**backend, &user, &all.rows, false).await?;

	let mut book = Vec::new();
	for row in &all.rows {
		if book.len() >= limit {
			break;
		}
		let metadata = context.metadata.get(&row.id);
		let title = metadata
			.and_then(|metadata| metadata.title.clone())
			.unwrap_or_else(|| row.name.clone());
		let authors = metadata
			.map(|metadata| mapper::csv(metadata.writers.as_deref()))
			.unwrap_or_default();

		// abs-ref reports which field matched, and omits both keys for a
		// title hit (`capture/library_search.json`).
		let hit = if title.to_lowercase().contains(&needle) {
			Some((None, None))
		} else if let Some(author) = authors
			.iter()
			.find(|author| author.to_lowercase().contains(&needle))
		{
			Some((Some("authors".to_owned()), Some(author.clone())))
		} else {
			None
		};

		if let Some((match_key, match_text)) = hit {
			book.push(SearchBookDto {
				library_item: context.item(row, ItemShape::Expanded, &user.id, None),
				match_key,
				match_text,
			});
		}
	}

	let series_rows = series::Entity::find()
		.filter(series::Column::LibraryId.eq(library_id.as_str()))
		.order_by_asc(series::Column::Name)
		.all(backend.conn())
		.await?;
	let series = series_rows
		.iter()
		.filter(|row| row.name.to_lowercase().contains(&needle))
		// Lissen fans a series hit out into a series listing, so a
		// single-book folder must not appear as one.
		.filter(|row| {
			context
				.series_book_counts
				.get(&row.id)
				.copied()
				.unwrap_or(0)
				> 1
		})
		.take(limit)
		.map(|row| {
			let books = all
				.rows
				.iter()
				.filter(|media| media.series_id.as_deref() == Some(row.id.as_str()))
				.map(|media| context.item(media, ItemShape::Expanded, &user.id, None))
				.collect();
			SearchSeriesDto {
				series: NamedIdDto {
					id: row.id.clone(),
					name: row.name.clone(),
				},
				books,
			}
		})
		.collect();

	let facts = author_facts(&**backend, &user, &library_id).await?;
	let matching = facts
		.iter()
		.filter(|fact| fact.name.to_lowercase().contains(&needle))
		.take(limit)
		.collect::<Vec<_>>();
	let names = matching
		.iter()
		.map(|fact| fact.name.clone())
		.collect::<Vec<_>>();
	let ids = backend.author_ids(&names).await?;
	let authors = matching
		.iter()
		.map(|fact| {
			author_dto(
				ids.get(&fact.name)
					.cloned()
					.unwrap_or_else(|| fact.name.clone()),
				fact,
				&library_id,
				true,
			)
		})
		.collect();

	Ok(Json(SearchResultDto {
		book,
		narrators: Vec::new(),
		tags: Vec::new(),
		genres: Vec::new(),
		series,
		authors,
	}))
}
