use std::path::PathBuf;

use axum::{
	extract::{Path, Query, State},
	http::HeaderMap,
	response::IntoResponse,
	Extension,
};
use chrono::Utc;
use models::{
	domain::reading_progress::compute_page_based_percentage,
	entity::{library, media, media_metadata, reading_session, series, series_metadata},
	services::reading_progress::{upsert_reading_session, NormalizedProgression},
	shared::image_processor_options::{ImageProcessorOptions, SupportedImageFormat},
};
use sea_orm::{prelude::*, QueryOrder, QuerySelect, QueryTrait};
use serde::{Deserialize, Serialize};
use stump_api_types::OffsetPagination;
use stump_auth::AuthContext;
use stump_core::opds::{
	v1_2::{
		entry::{IntoOPDSEntry, OPDSEntryBuilder, OpdsEntry},
		feed::{
			OPDSFeedBuilder, OPDSFeedBuilderPageParams, OPDSFeedBuilderParams, OpdsFeed,
		},
		link::{OpdsLink, OpdsLinkRel, OpdsLinkType},
		opensearch::OpdsOpenSearch,
	},
	v2_0::entity::OPDSPublicationEntity,
};
use stump_media::{
	image::{GenericImageProcessor, ImageProcessor},
	media::get_page_async,
	ContentType,
};

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	routers::api::v2::media::get_media_thumbnail_by_id,
	utils::{
		http::{ImageResponse, Xml},
		serve_media,
	},
};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OPDSURLParams<D> {
	#[serde(flatten)]
	pub(crate) params: D,
	#[serde(default)]
	pub(crate) api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OPDSIDURLParams {
	pub(crate) id: String,
}

fn number_or_string_deserializer<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let value = String::deserialize(deserializer)?;
	value.parse::<i32>().map_err(serde::de::Error::custom)
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OPDSPageURLParams {
	pub(crate) id: String,
	#[serde(deserialize_with = "number_or_string_deserializer")]
	pub(crate) page: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OPDSFilenameURLParams {
	pub(crate) id: String,
	pub(crate) filename: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct OPDSSearchQuery {
	#[serde(default)]
	pub(crate) search: Option<String>,
}

fn catalog_url(req_ctx: &AuthContext, path: &str) -> String {
	if let Some(api_key) = req_ctx.api_key() {
		format!("/opds/{}/v1.2/{}", api_key, path)
	} else {
		format!("/opds/v1.2/{}", path)
	}
}

fn service_url(req_ctx: &AuthContext) -> String {
	if let Some(api_key) = req_ctx.api_key() {
		format!("/opds/{}/v1.2", api_key)
	} else {
		"/opds/v1.2".to_string()
	}
}

/// Visible page counts (duplicate-page skipping) for a batch of publication
/// entries. Media missing from the map fall back to their physical count.
async fn compute_visible_counts(
	ctx: &AppState,
	books: &[OPDSPublicationEntity],
) -> std::collections::HashMap<String, i32> {
	// Collected first: handing a borrowing `map` iterator to the generic
	// async fn trips the higher-ranked `FnOnce` lifetime check.
	let pages: Vec<(String, i32)> = books
		.iter()
		.map(|book| (book.media.id.clone(), book.media.pages))
		.collect();
	stump_core::filesystem::media::visible_pages::visible_page_counts(
		ctx.conn.as_ref(),
		&ctx.visible_pages_cache(),
		pages,
	)
	.await
	.unwrap_or_else(|error| {
		tracing::warn!(?error, "Failed to compute visible page counts");
		Default::default()
	})
}

async fn build_publication_entries(
	books: Vec<OPDSPublicationEntity>,
	visible_counts: &std::collections::HashMap<String, i32>,
	api_key: Option<String>,
) -> Vec<OpdsEntry> {
	let mut entries = Vec::with_capacity(books.len());
	for book in books {
		let visible_page_count = visible_counts.get(&book.media.id).copied();
		entries.push(
			OPDSEntryBuilder::<OPDSPublicationEntity>::new(book, api_key.clone())
				.with_visible_page_count(visible_page_count)
				.into_opds_entry()
				.await,
		);
	}
	entries
}

pub(crate) async fn catalog(Extension(req): Extension<AuthContext>) -> APIResult<Xml> {
	let entries = vec![
		OpdsEntry::new(
			"keepReading".to_string(),
			Utc::now().into(),
			"Keep reading".to_string(),
			None,
			Some(String::from("Continue reading your in progress books")),
			None,
			Some(vec![OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Subsection,
				href: catalog_url(&req, "keep-reading"),
			}]),
			None,
		),
		OpdsEntry::new(
			"allSeries".to_string(),
			Utc::now().into(),
			"All series".to_string(),
			None,
			Some(String::from("Browse by series")),
			None,
			Some(vec![OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Subsection,
				href: catalog_url(&req, "series"),
			}]),
			None,
		),
		OpdsEntry::new(
			"latestSeries".to_string(),
			Utc::now().into(),
			"Latest series".to_string(),
			None,
			Some(String::from("Browse latest series")),
			None,
			Some(vec![OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Subsection,
				href: catalog_url(&req, "series/latest"),
			}]),
			None,
		),
		OpdsEntry::new(
			"allLibraries".to_string(),
			Utc::now().into(),
			"All libraries".to_string(),
			None,
			Some(String::from("Browse by library")),
			None,
			Some(vec![OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Subsection,
				href: catalog_url(&req, "libraries"),
			}]),
			None,
		),
		OpdsEntry::new(
			"allBooks".to_string(),
			Utc::now().into(),
			"All books".to_string(),
			None,
			Some(String::from("Browse all books")),
			None,
			Some(vec![OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Subsection,
				href: catalog_url(&req, "books"),
			}]),
			None,
		),
		OpdsEntry::new(
			"latestBooks".to_string(),
			Utc::now().into(),
			"Latest books".to_string(),
			None,
			Some(String::from("Browse latest books")),
			None,
			Some(vec![OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Subsection,
				href: catalog_url(&req, "books/latest"),
			}]),
			None,
		),
	];

	let links = vec![
		OpdsLink {
			link_type: OpdsLinkType::Navigation,
			rel: OpdsLinkRel::ItSelf,
			href: catalog_url(&req, "catalog"),
		},
		OpdsLink {
			link_type: OpdsLinkType::Navigation,
			rel: OpdsLinkRel::Start,
			href: catalog_url(&req, "catalog"),
		},
		OpdsLink {
			link_type: OpdsLinkType::Search,
			rel: OpdsLinkRel::Search,
			href: catalog_url(&req, "search"),
		},
	];

	let feed = OpdsFeed::new(
		"root".to_string(),
		"Coppice OPDS catalog".to_string(),
		Some(links),
		entries,
	);

	Ok(Xml(feed.build()?))
}

pub(crate) async fn search_description(
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	Ok(OpdsOpenSearch::new(Some(service_url(&req)))
		.build()
		.map(Xml)?)
}

pub(crate) async fn keep_reading(
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let books = OPDSPublicationEntity::find_for_user(&req.user())
		.filter(reading_session::Column::UserId.eq(req.id()))
		.order_by_desc(reading_session::Column::UpdatedAt)
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;

	let visible_counts = compute_visible_counts(&ctx, &books).await;
	let entries = build_publication_entries(books, &visible_counts, req.api_key()).await;

	let feed = OpdsFeed::new(
		"keepReading".to_string(),
		"Keep Reading".to_string(),
		Some(vec![
			OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::ItSelf,
				href: catalog_url(&req, "keep-reading"),
			},
			OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Start,
				href: catalog_url(&req, "catalog"),
			},
		]),
		entries,
	);

	Ok(Xml(feed.build()?))
}

/// A handler for GET /opds/v1.2/libraries, accepts a `search` URL param
pub(crate) async fn get_libraries(
	State(ctx): State<AppState>,
	Query(OPDSSearchQuery { search }): Query<OPDSSearchQuery>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let user = req.user();

	let libraries = library::Entity::find_for_user(&user)
		.apply_if(search, |query, search| {
			query.filter(library::Column::Name.contains(search))
		})
		.order_by_asc(library::Column::Name)
		.all(ctx.conn.as_ref())
		.await?;
	let mut entries = Vec::with_capacity(libraries.len());
	for library in libraries {
		entries.push(
			OPDSEntryBuilder::<library::Model>::new(library, req.api_key())
				.into_opds_entry()
				.await,
		);
	}

	let feed = OpdsFeed::new(
		"allLibraries".to_string(),
		"All libraries".to_string(),
		Some(vec![
			OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::ItSelf,
				href: catalog_url(&req, "libraries"),
			},
			OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Start,
				href: catalog_url(&req, "catalog"),
			},
		]),
		entries,
	);

	Ok(Xml(feed.build()?))
}

pub(crate) async fn get_library_by_id(
	State(ctx): State<AppState>,
	Path(OPDSURLParams {
		params: OPDSIDURLParams { id },
		..
	}): Path<OPDSURLParams<OPDSIDURLParams>>,
	pagination: Query<OffsetPagination>,
	Query(OPDSSearchQuery { search }): Query<OPDSSearchQuery>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let user = req.user();

	// Note: This is just to enforce access to the library, otherwise we would get an
	// empty result set without informing the requester that the resource they are
	// requesting "does not exist"
	let library = library::Entity::find_for_user(&user)
		.select_only()
		.columns(library::LibraryIdentSelect::columns())
		.filter(library::Column::Id.eq(id.clone()))
		.into_model::<library::LibraryIdentSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Library not found".to_string()))?;

	// Mode B: a virtual library browses its provider source live; there are
	// no series rows to fall back to.
	if let Some(feed) =
		virtual_library_feed_or_none(&ctx, &req, &library, &pagination, search.as_deref())
			.await?
	{
		return Ok(feed);
	}

	let series = series::Entity::find_for_user(&user)
		.filter(series::Column::LibraryId.eq(library.id.clone()))
		.order_by_asc(series::Column::Name)
		.offset(pagination.offset())
		.limit(pagination.limit())
		.all(ctx.conn.as_ref())
		.await?;
	let count = series::Entity::find_for_user(&user)
		.filter(series::Column::LibraryId.eq(library.id.clone()))
		.count(ctx.conn.as_ref())
		.await?;

	let mut entries = Vec::with_capacity(series.len());
	for series in series {
		entries.push(
			OPDSEntryBuilder::<series::Model>::new(series, req.api_key())
				.into_opds_entry()
				.await,
		);
	}

	let feed = OPDSFeedBuilder::new(req.api_key()).paginated(OPDSFeedBuilderParams {
		id,
		title: library.name.clone(),
		entries,
		href_postfix: format!("libraries/{}", library.id),
		page_params: Some(OPDSFeedBuilderPageParams {
			page: pagination.page,
			count,
		}),
		search: None,
	})?;

	Ok(Xml(feed.build()?))
}

/// Mode B: the live-browse feed for a virtual library, or `None` when the
/// library is not provider-backed (or the providers feature is compiled
/// out).
#[cfg(feature = "providers")]
async fn virtual_library_feed_or_none(
	ctx: &AppState,
	req: &AuthContext,
	library: &library::LibraryIdentSelect,
	pagination: &OffsetPagination,
	search: Option<&str>,
) -> APIResult<Option<Xml>> {
	use stump_provider::virtual_path;

	let Some(source_id) =
		crate::routers::provider_virtual::virtual_library_source(ctx, &library.id).await
	else {
		return Ok(None);
	};
	let kind = crate::routers::provider_virtual::browse_kind(search, &[]);
	let size = pagination.limit().max(1);
	let page_index = (pagination.offset() / size) as u32;
	let result = crate::routers::provider_virtual::browse_page(
		ctx,
		&source_id,
		&library.id,
		&kind,
		page_index,
	)
	.await
	.map_err(APIError::InternalServerError)?;

	// Live rows have no `series_metadata` row for the per-user age
	// restriction to filter on, so the remote rating is checked here.
	let user = req.user();
	let adult_source =
		crate::routers::provider_virtual::source_is_adult(ctx, &source_id).await;
	let visible = result
		.items
		.iter()
		.filter(|remote| {
			crate::routers::provider_virtual::age_restriction_allows(
				&user,
				crate::routers::provider_virtual::remote_age_rating(remote, adult_source),
			)
		})
		.collect::<Vec<_>>();

	let entries = visible
		.iter()
		.map(|remote| {
			let stump_id = virtual_path::series_id(&source_id, &remote.remote_id);
			OpdsEntry::new(
				stump_id.clone(),
				Utc::now().fixed_offset(),
				remote.title.clone(),
				None,
				remote.description.clone(),
				Some(remote.authors.clone()),
				Some(vec![OpdsLink::new(
					OpdsLinkType::Navigation,
					OpdsLinkRel::Subsection,
					catalog_url(req, &format!("series/{}", stump_id)),
				)]),
				None,
			)
		})
		.collect::<Vec<OpdsEntry>>();

	// Live browse has no total count; one phantom element past the current
	// window marks a next page for OPDS clients when the source has one.
	let count = pagination.offset() + visible.len() as u64 + u64::from(result.has_next);

	let feed = OPDSFeedBuilder::new(req.api_key()).paginated(OPDSFeedBuilderParams {
		id: library.id.clone(),
		title: library.name.clone(),
		entries,
		href_postfix: format!("libraries/{}", library.id),
		page_params: Some(OPDSFeedBuilderPageParams {
			page: pagination.page,
			count,
		}),
		search: None,
	})?;

	Ok(Some(Xml(feed.build()?)))
}

#[cfg(not(feature = "providers"))]
async fn virtual_library_feed_or_none(
	_ctx: &AppState,
	_req: &AuthContext,
	_library: &library::LibraryIdentSelect,
	_pagination: &OffsetPagination,
	_search: Option<&str>,
) -> APIResult<Option<Xml>> {
	Ok(None)
}

/// Mode B: materialise a live-only virtual series before its books are
/// served. `None` when the id is not a live virtual series the user may
/// see (a stored row the caller could not find is simply inaccessible).
#[cfg(feature = "providers")]
async fn materialise_series_for_opds(
	ctx: &AppState,
	user: &models::entity::user::AuthUser,
	id: &str,
) -> APIResult<Option<series::ModelWithMetadata>> {
	if series::Entity::find_by_id(id)
		.one(ctx.conn.as_ref())
		.await?
		.is_some()
	{
		return Ok(None);
	}
	match crate::routers::provider_virtual::materialise_virtual_series(ctx, user, id)
		.await
	{
		None => return Ok(None),
		Some(Err(error)) => return Err(APIError::InternalServerError(error)),
		Some(Ok(_)) => {},
	}
	series::ModelWithMetadata::find_for_user(user)
		.filter(series::Column::Id.eq(id.to_string()))
		.into_model::<series::ModelWithMetadata>()
		.one(ctx.conn.as_ref())
		.await
		.map_err(APIError::from)
}

#[cfg(not(feature = "providers"))]
async fn materialise_series_for_opds(
	_ctx: &AppState,
	_user: &models::entity::user::AuthUser,
	_id: &str,
) -> APIResult<Option<series::ModelWithMetadata>> {
	Ok(None)
}

// FIXME: Based on testing with Panels, it seems like pagination isn't an expected default when
// a search is present? This feels both odd but understandable to support an "at a glance" view,
// but I feel like it should still support pagination...

/// A handler for GET /opds/v1.2/series, accepts a `page` URL param. Note: OPDS
/// pagination is zero-indexed.
pub(crate) async fn get_series(
	State(ctx): State<AppState>,
	Query(pagination): Query<OffsetPagination>,
	Query(OPDSSearchQuery { search }): Query<OPDSSearchQuery>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let user = req.user();
	let search_cpy = search.clone();
	let series = series::Entity::find_for_user(&user)
		.apply_if(search_cpy, |query, search| {
			query.left_join(series_metadata::Entity).filter(
				series::Column::Name
					.contains(search.clone())
					.or(series_metadata::Column::Title.contains(search)),
			)
		})
		.order_by_asc(series::Column::Name)
		.offset(pagination.offset())
		.limit(pagination.limit())
		.all(ctx.conn.as_ref())
		.await?;
	let search_cpy = search.clone();
	let count = series::Entity::find_for_user(&user)
		.apply_if(search_cpy, |query, search| {
			query.left_join(series_metadata::Entity).filter(
				series::Column::Name
					.contains(search.clone())
					.or(series_metadata::Column::Title.contains(search)),
			)
		})
		.count(ctx.conn.as_ref())
		.await?;

	let mut entries = Vec::with_capacity(series.len());
	for series in series {
		entries.push(
			OPDSEntryBuilder::<series::Model>::new(series, req.api_key())
				.into_opds_entry()
				.await,
		);
	}

	let feed = OPDSFeedBuilder::new(req.api_key()).paginated(OPDSFeedBuilderParams {
		id: "allSeries".to_string(),
		title: "All Series".to_string(),
		entries,
		href_postfix: "series".to_string(),
		page_params: Some(OPDSFeedBuilderPageParams {
			page: pagination.page,
			count,
		}),
		search,
	})?;

	Ok(Xml(feed.build()?))
}

pub(crate) async fn get_latest_series(
	State(ctx): State<AppState>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let user = req.user();
	let series = series::Entity::find_for_user(&user)
		.order_by_desc(series::Column::UpdatedAt)
		.offset(pagination.offset())
		.limit(pagination.limit())
		.all(ctx.conn.as_ref())
		.await?;
	let count = series::Entity::find_for_user(&user)
		.count(ctx.conn.as_ref())
		.await?;

	let mut entries = Vec::with_capacity(series.len());
	for series in series {
		entries.push(
			OPDSEntryBuilder::<series::Model>::new(series, req.api_key())
				.into_opds_entry()
				.await,
		);
	}

	let feed = OPDSFeedBuilder::new(req.api_key()).paginated(OPDSFeedBuilderParams {
		id: "latestSeries".to_string(),
		title: "Latest Series".to_string(),
		entries,
		href_postfix: "series/latest".to_string(),
		page_params: Some(OPDSFeedBuilderPageParams {
			page: pagination.page,
			count,
		}),
		search: None,
	})?;

	Ok(Xml(feed.build()?))
}

pub(crate) async fn get_series_by_id(
	Path(OPDSURLParams {
		params: OPDSIDURLParams { id },
		..
	}): Path<OPDSURLParams<OPDSIDURLParams>>,
	State(ctx): State<AppState>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let user = req.user();
	// Mode B: a live-only virtual series is materialised on first access,
	// then served through the ordinary database path below.
	let stored = series::ModelWithMetadata::find_for_user(&user)
		.filter(series::Column::Id.eq(id.clone()))
		.into_model::<series::ModelWithMetadata>()
		.one(ctx.conn.as_ref())
		.await?;
	let stored = match stored {
		Some(stored) => Some(stored),
		None => materialise_series_for_opds(&ctx, &user, &id).await?,
	};
	let Some(series::ModelWithMetadata { series, metadata }) = stored else {
		return Err(APIError::NotFound(format!("Series {id} not found")));
	};

	let books = OPDSPublicationEntity::find_for_user(&user)
		.filter(media::Column::SeriesId.eq(id.clone()))
		.order_by_asc(media::Column::Name)
		.offset(pagination.offset())
		.limit(pagination.limit())
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let count = OPDSPublicationEntity::find_for_user(&user)
		.filter(media::Column::SeriesId.eq(id.clone()))
		.count(ctx.conn.as_ref())
		.await?;

	let visible_counts = compute_visible_counts(&ctx, &books).await;
	let entries = build_publication_entries(books, &visible_counts, req.api_key()).await;

	let title = metadata
		.and_then(|m| m.title.clone())
		.unwrap_or_else(|| series.name.clone());

	let feed = OPDSFeedBuilder::new(req.api_key()).paginated(OPDSFeedBuilderParams {
		id: series.id.clone(),
		title,
		entries,
		href_postfix: format!("series/{}", series.id),
		page_params: Some(OPDSFeedBuilderPageParams {
			page: pagination.page,
			count,
		}),
		search: None,
	})?;

	Ok(Xml(feed.build()?))
}

/// A handler for GET /opds/v1.2/search/feed, unified search returning libraries + series + books
pub(crate) async fn search_feed(
	State(ctx): State<AppState>,
	Query(OPDSSearchQuery { search }): Query<OPDSSearchQuery>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let search = search.unwrap_or_default();
	if search.is_empty() {
		let feed = OpdsFeed::new(
			"searchFeed".to_string(),
			"Search Results".to_string(),
			Some(vec![
				OpdsLink {
					link_type: OpdsLinkType::Navigation,
					rel: OpdsLinkRel::ItSelf,
					href: catalog_url(&req, "search/feed"),
				},
				OpdsLink {
					link_type: OpdsLinkType::Navigation,
					rel: OpdsLinkRel::Start,
					href: catalog_url(&req, "catalog"),
				},
			]),
			vec![],
		);
		return Ok(Xml(feed.build()?));
	}

	let user = req.user();
	let mut entries = Vec::new();

	// Search libraries
	let libraries = library::Entity::find_for_user(&user)
		.filter(library::Column::Name.contains(&search))
		.order_by_asc(library::Column::Name)
		.all(ctx.conn.as_ref())
		.await?;
	for lib in libraries {
		entries.push(
			OPDSEntryBuilder::<library::Model>::new(lib, req.api_key())
				.into_opds_entry()
				.await,
		);
	}

	// Search series
	let series = series::Entity::find_for_user(&user)
		.left_join(series_metadata::Entity)
		.filter(
			series::Column::Name
				.contains(&search)
				.or(series_metadata::Column::Title.contains(&search)),
		)
		.order_by_asc(series::Column::Name)
		.all(ctx.conn.as_ref())
		.await?;
	for s in series {
		entries.push(
			OPDSEntryBuilder::<series::Model>::new(s, req.api_key())
				.into_opds_entry()
				.await,
		);
	}

	// Search books by name, metadata title, summary, and writers
	let books = OPDSPublicationEntity::find_for_user(&user)
		.filter(
			media::Column::Name
				.contains(&search)
				.or(media_metadata::Column::Title.contains(&search))
				.or(media_metadata::Column::Summary.contains(&search))
				.or(media_metadata::Column::Writers.contains(&search)),
		)
		.order_by_asc(media::Column::Name)
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let visible_counts = compute_visible_counts(&ctx, &books).await;
	for book in books {
		let visible_page_count = visible_counts.get(&book.media.id).copied();
		entries.push(
			OPDSEntryBuilder::<OPDSPublicationEntity>::new(book, req.api_key())
				.with_visible_page_count(visible_page_count)
				.into_opds_entry()
				.await,
		);
	}

	let feed = OpdsFeed::new(
		"searchFeed".to_string(),
		"Search Results".to_string(),
		Some(vec![
			OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::ItSelf,
				href: catalog_url(&req, &format!("search/feed?search={}", search)),
			},
			OpdsLink {
				link_type: OpdsLinkType::Navigation,
				rel: OpdsLinkRel::Start,
				href: catalog_url(&req, "catalog"),
			},
		]),
		entries,
	);

	Ok(Xml(feed.build()?))
}

/// A handler for GET /opds/v1.2/books, paginated book listing with optional search
pub(crate) async fn get_books(
	State(ctx): State<AppState>,
	Query(pagination): Query<OffsetPagination>,
	Query(OPDSSearchQuery { search }): Query<OPDSSearchQuery>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let user = req.user();
	let search_cpy = search.clone();

	let books = OPDSPublicationEntity::find_for_user(&user)
		.apply_if(search_cpy, |query, search| {
			query.filter(
				media::Column::Name
					.contains(search.clone())
					.or(media_metadata::Column::Title.contains(search.clone()))
					.or(media_metadata::Column::Summary.contains(search.clone()))
					.or(media_metadata::Column::Writers.contains(search)),
			)
		})
		.order_by_asc(media::Column::Name)
		.offset(pagination.offset())
		.limit(pagination.limit())
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;

	let search_cpy = search.clone();
	let count = OPDSPublicationEntity::find_for_user(&user)
		.apply_if(search_cpy, |query, search| {
			query.filter(
				media::Column::Name
					.contains(search.clone())
					.or(media_metadata::Column::Title.contains(search.clone()))
					.or(media_metadata::Column::Summary.contains(search.clone()))
					.or(media_metadata::Column::Writers.contains(search)),
			)
		})
		.count(ctx.conn.as_ref())
		.await?;

	let visible_counts = compute_visible_counts(&ctx, &books).await;
	let entries = build_publication_entries(books, &visible_counts, req.api_key()).await;

	let feed = OPDSFeedBuilder::new(req.api_key()).paginated(OPDSFeedBuilderParams {
		id: "allBooks".to_string(),
		title: "All Books".to_string(),
		entries,
		href_postfix: "books".to_string(),
		page_params: Some(OPDSFeedBuilderPageParams {
			page: pagination.page,
			count,
		}),
		search,
	})?;

	Ok(Xml(feed.build()?))
}

/// A handler for GET /opds/v1.2/books/latest, latest books ordered by created_at DESC
pub(crate) async fn get_latest_books(
	State(ctx): State<AppState>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Xml> {
	let user = req.user();

	let books = OPDSPublicationEntity::find_for_user(&user)
		.order_by_desc(media::Column::CreatedAt)
		.offset(pagination.offset())
		.limit(pagination.limit())
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;

	let count = OPDSPublicationEntity::find_for_user(&user)
		.count(ctx.conn.as_ref())
		.await?;

	let visible_counts = compute_visible_counts(&ctx, &books).await;
	let entries = build_publication_entries(books, &visible_counts, req.api_key()).await;

	let feed = OPDSFeedBuilder::new(req.api_key()).paginated(OPDSFeedBuilderParams {
		id: "latestBooks".to_string(),
		title: "Latest Books".to_string(),
		entries,
		href_postfix: "books/latest".to_string(),
		page_params: Some(OPDSFeedBuilderPageParams {
			page: pagination.page,
			count,
		}),
		search: None,
	})?;

	Ok(Xml(feed.build()?))
}

// TODO: support something like `STRICT_OPDS` to enforce OPDS compliance conditionally
fn handle_opds_image_response(
	content_type: ContentType,
	image_buffer: Vec<u8>,
) -> APIResult<ImageResponse> {
	if content_type.is_opds_legacy_image() {
		Ok(ImageResponse::new(content_type, image_buffer))
	} else if content_type.is_decodable_image() {
		tracing::debug!("Converting image to JPEG for legacy OPDS compatibility");
		let jpeg_buffer = tokio::task::block_in_place(|| {
			let converted = GenericImageProcessor::generate(
				&image_buffer,
				ImageProcessorOptions {
					format: SupportedImageFormat::Jpeg,
					..Default::default()
				},
			)?;
			Ok::<Vec<u8>, APIError>(converted)
		})?;
		Ok(ImageResponse::new(ContentType::JPEG, jpeg_buffer))
	} else {
		tracing::warn!(
			?content_type,
			"Encountered image which does not conform to legacy OPDS image requirements"
		);
		Ok(ImageResponse::new(content_type, image_buffer))
	}
}

/// A handler for GET /opds/v1.2/books/{id}/thumbnail, returns the thumbnail
///
/// The same stored-or-generated thumbnail every other profile serves, then
/// narrowed to the image types OPDS 1.2 readers accept.
pub(crate) async fn get_book_thumbnail(
	Path(OPDSURLParams {
		params: OPDSIDURLParams { id },
		..
	}): Path<OPDSURLParams<OPDSIDURLParams>>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<ImageResponse> {
	let ImageResponse { content_type, data } =
		get_media_thumbnail_by_id(&ctx, &req.user(), id).await?;
	handle_opds_image_response(content_type, data)
}

/// A handler for GET /opds/v1.2/books/{id}/page/{page}, returns the page
///
/// Note: Reading progression tracking can be disabled via the ENABLE_OPDS_PROGRESSION
/// configuration variable to avoid inaccurate tracking when clients preload pages.
#[tracing::instrument(skip(ctx, req), fields(book_id = %id, page, zero_based = ?pagination.zero_based))]
pub(crate) async fn get_book_page(
	Path(OPDSURLParams {
		params: OPDSPageURLParams { id, page },
		..
	}): Path<OPDSURLParams<OPDSPageURLParams>>,
	State(ctx): State<AppState>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<ImageResponse> {
	// OPDS defaults to zero-indexed pages, I don't even think it allows the
	// zero_based query param to be set.
	let zero_based = pagination.zero_based.unwrap_or(true);
	let mut correct_page = page;
	if zero_based {
		correct_page = page + 1;
	}

	let user = req.user();
	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(id.clone()))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	// Duplicate-page skipping renumbers visible pages onto the physical file.
	// Progression stays in visible numbering so it lines up with the PSE
	// count; the file read uses the mapped physical page.
	let cache = ctx.visible_pages_cache();
	let visible = stump_core::filesystem::media::visible_pages::visible_pages(
		ctx.conn.as_ref(),
		&cache,
		&book.id,
		book.pages,
	)
	.await?;
	let physical_page = stump_core::filesystem::media::visible_pages::physical_page(
		&visible,
		correct_page as i32,
	)
	.ok_or(APIError::NotFound("Page not found".to_string()))?;
	let visible_count = visible.len() as i32;

	if ctx.config.protocols.enable_opds_progression {
		let percentage = compute_page_based_percentage(correct_page, visible_count);
		let progression = NormalizedProgression {
			page: Some(correct_page),
			percentage: Some(percentage),
			did_complete: visible_count == correct_page as i32,
			..Default::default()
		};

		let reading_session =
			upsert_reading_session(ctx.conn.as_ref(), &user, &id, progression).await?;
		tracing::trace!(?reading_session, "Upserted active reading session");
	}

	let (content_type, image_buffer) = get_page_async(
		PathBuf::from(book.path),
		physical_page as i32,
		&ctx.config.media,
	)
	.await?;

	handle_opds_image_response(content_type, image_buffer)
}

/// A handler for GET /opds/v1.2/books/{id}/file/{filename}, returns the book
pub(crate) async fn download_book(
	Path(OPDSURLParams {
		params: OPDSFilenameURLParams { id, .. },
		..
	}): Path<OPDSURLParams<OPDSFilenameURLParams>>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	serve_media::serve_media_file(req, headers, ctx.conn.as_ref(), id).await
}
