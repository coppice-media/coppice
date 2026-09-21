use std::{collections::HashSet, ops::Deref, path::PathBuf};

use axum::{
	extract::{Path, Query, State},
	http::{header, HeaderMap, HeaderValue},
	response::IntoResponse,
	Extension, Json,
};
use models::txn::begin_write;
use models::{
	domain::{
		reading_progress::compute_page_based_percentage,
		reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	},
	services::reading_progress::{upsert_reading_session, NormalizedProgression},
};
use models::{
	entity::{
		device, library, media, media_metadata, reading_session, series, series_metadata,
		user::AuthUser,
	},
	shared::enums::{DeviceKind, ReadingStatus},
};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{
	prelude::*, sea_query::Expr, ActiveValue::Set, Condition, Order, QueryOrder,
	QueryTrait,
};
use sea_orm::{PaginatorTrait, QuerySelect};
use serde::{Deserialize, Serialize};
use stump_api_types::OffsetPagination;
use stump_auth::AuthContext;
use stump_core::reading_state;
use stump_core::{
	opds::v2_0::{
		authentication::{
			OPDSAuthenticationDocument, OPDSAuthenticationDocumentBuilder,
			OPDSSupportedAuthFlow, OPDS_AUTHENTICATION_DOCUMENT_TYPE,
		},
		entity::{OPDSProgressionBookRef, OPDSProgressionEntity, OPDSPublicationEntity},
		feed::{OPDSFeed, OPDSFeedBuilder},
		group::OPDSFeedGroupBuilder,
		link::{
			OPDSBaseLinkBuilder, OPDSLink, OPDSLinkFinalizer, OPDSLinkRel, OPDSLinkType,
			OPDSNavigationLink, OPDSNavigationLinkBuilder,
		},
		metadata::{OPDSMetadata, OPDSMetadataBuilder, OPDSPaginationMetadataBuilder},
		progression::{OPDSProgression, OPDSProgressionInput},
		publication::OPDSPublication,
	},
	Ctx,
};
use stump_devices::{CredentialRef, Protocol};
#[cfg(feature = "readium")]
use stump_library::sync_maps;
use stump_media::media::get_page_async;
#[cfg(feature = "readium")]
use stump_worker::AlignGranularity;

use crate::{
	config::state::AppState,
	errors::{APIError, APIResult},
	middleware::host::HostExtractor,
	routers::{api::v2::media::get_media_thumbnail_by_id, relative_favicon_path},
	utils::{http::ImageResponse, serve_media},
};

const DEFAULT_LIMIT: u64 = 10;

/// The routes a v2 feed names in its own links. Every one of these is mounted
/// by `stump_opds::v2_router`; a link to an unmounted path is a 404 that a
/// client cannot tell apart from an empty feed.
const CATALOG_ROUTE: &str = "/opds/v2.0/catalog";
const SEARCH_ROUTE: &str = "/opds/v2.0/search";
const LIBRARIES_ROUTE: &str = "/opds/v2.0/libraries";
const LIBRARY_SEARCH_ROUTE: &str = "/opds/v2.0/libraries/search";
const SERIES_ROUTE: &str = "/opds/v2.0/series";
const SERIES_SEARCH_ROUTE: &str = "/opds/v2.0/series/search";
const BOOK_SEARCH_ROUTE: &str = "/opds/v2.0/books/search";
const BROWSE_BOOKS_ROUTE: &str = "/opds/v2.0/books/browse";
const LATEST_BOOKS_ROUTE: &str = "/opds/v2.0/books/latest";
const KEEP_READING_ROUTE: &str = "/opds/v2.0/books/keep-reading";

/// A wrapper struct for an OPDS authentication document, which is used to set the
/// appropriate content type header. The Json extractor would otherwise set it incorrectly
pub(crate) struct OPDSAuthDocWrapper(OPDSAuthenticationDocument);

impl IntoResponse for OPDSAuthDocWrapper {
	fn into_response(self) -> axum::http::Response<axum::body::Body> {
		let mut base_resp = Json(self.0).into_response();
		let Ok(header_value) = HeaderValue::from_str(OPDS_AUTHENTICATION_DOCUMENT_TYPE)
		else {
			tracing::error!(
				"Failed to convert OPDS_AUTHENTICATION_DOCUMENT_TYPE to HeaderValue"
			);
			return base_resp;
		};
		base_resp
			.headers_mut()
			.insert(header::CONTENT_TYPE, header_value);
		base_resp
	}
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct OPDSSearchQuery {
	#[serde(default)]
	pub(crate) query: Option<String>,
}

/// The filter options for browsing books, based on the OPDS/Readium spec
///
/// See https://readium.org/webpub-manifest/schema/metadata.schema.json
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OPDSBrowseFilter {
	pub(crate) author: Option<String>,
	pub(crate) penciler: Option<String>,
	pub(crate) colorist: Option<String>,
	pub(crate) inker: Option<String>,
	pub(crate) letterer: Option<String>,
	pub(crate) editor: Option<String>,
	pub(crate) cover_artist: Option<String>,
	pub(crate) subject: Option<String>,
	pub(crate) characters: Option<String>,
	pub(crate) teams: Option<String>,
}

impl OPDSBrowseFilter {
	/// Transform self into a valid SeaORM [Condition] to apply as a filter
	fn into_condition(self) -> Option<Condition> {
		let mut condition = Condition::all();
		let mut has_filter = false;

		let field_mappings: Vec<(Option<String>, media_metadata::Column)> = vec![
			(self.author, media_metadata::Column::Writers),
			(self.penciler, media_metadata::Column::Pencillers),
			(self.colorist, media_metadata::Column::Colorists),
			(self.inker, media_metadata::Column::Inkers),
			(self.letterer, media_metadata::Column::Letterers),
			(self.editor, media_metadata::Column::Editors),
			(self.cover_artist, media_metadata::Column::CoverArtists),
			(self.subject, media_metadata::Column::Genres),
			(self.characters, media_metadata::Column::Characters),
			(self.teams, media_metadata::Column::Teams),
		];

		for (value, column) in field_mappings {
			if let Some(val) = value {
				condition = condition.add(column.like(format!("%{val}%")));
				has_filter = true;
			}
		}

		if has_filter {
			Some(condition)
		} else {
			None
		}
	}

	/// Generate a query string from the filter, used for the browse links
	fn to_query_string(&self) -> String {
		let mut parts = Vec::new();

		let fields: Vec<(&str, &Option<String>)> = vec![
			("author", &self.author),
			("penciler", &self.penciler),
			("colorist", &self.colorist),
			("inker", &self.inker),
			("letterer", &self.letterer),
			("editor", &self.editor),
			("coverArtist", &self.cover_artist),
			("subject", &self.subject),
			("characters", &self.characters),
			("teams", &self.teams),
		];

		for (key, value) in fields {
			if let Some(val) = value {
				parts.push(format!("{}={}", key, urlencoding::encode(val)));
			}
		}

		parts.join("&")
	}

	// TODO: There is an argument to have localization for the OPDS

	/// Generate a basic human-readable subtitle describing the active filters, basically
	/// just lists them out
	fn subtitle(&self) -> Option<String> {
		let parts: Vec<String> = [
			self.author.as_ref().map(|v| format!("author {v}")),
			self.penciler.as_ref().map(|v| format!("penciler {v}")),
			self.colorist.as_ref().map(|v| format!("colorist {v}")),
			self.inker.as_ref().map(|v| format!("inker {v}")),
			self.letterer.as_ref().map(|v| format!("letterer {v}")),
			self.editor.as_ref().map(|v| format!("editor {v}")),
			self.cover_artist
				.as_ref()
				.map(|v| format!("cover artist {v}")),
			self.subject.as_ref().map(|v| format!("subject {v}")),
			self.characters.as_ref().map(|v| format!("characters {v}")),
			self.teams.as_ref().map(|v| format!("teams {v}")),
		]
		.into_iter()
		.flatten()
		.collect();

		if parts.is_empty() {
			return None;
		}

		let list = match parts.len() {
			1 => parts[0].clone(),
			2 => format!("{} and {}", parts[0], parts[1]),
			_ => {
				let (last, rest) = parts.split_last().unwrap();
				format!("{}, and {last}", rest.join(", "))
			},
		};

		Some(format!("Filtered by {list}"))
	}
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OPDSBrowseParams {
	#[serde(flatten)]
	pub(crate) pagination: OffsetPagination,
	#[serde(flatten)]
	pub(crate) filter: OPDSBrowseFilter,
}

/// The page a group inside a feed stands for: the first [`DEFAULT_LIMIT`] items
/// of the route the group's `self` link names.
fn preview_pagination() -> OffsetPagination {
	OffsetPagination {
		page: 1,
		page_size: Some(DEFAULT_LIMIT),
		zero_based: Some(false),
	}
}

/// The URL of one page of `base_url`, which must carry no page of its own: a
/// repeated `page` makes the query string undeserializable.
fn page_url(base_url: &str, pagination: &OffsetPagination, page: u64) -> String {
	let separator = if base_url.contains('?') { "&" } else { "?" };
	let mut url = format!(
		"{base_url}{separator}page={page}&page_size={}",
		pagination.limit()
	);
	if pagination.zero_based.unwrap_or(false) {
		url.push_str("&zero_based=true");
	}
	url
}

/// The first and last page holding an item, in the numbering the request used.
/// An empty feed is one empty page, so both bounds are the page being served.
fn page_bounds(pagination: &OffsetPagination, total_items: u64) -> (u64, u64) {
	let first = u64::from(!pagination.zero_based.unwrap_or(false));
	let pages = total_items.div_ceil(pagination.limit().max(1)).max(1);
	(first, first + pages - 1)
}

/// The `first`/`previous`/`next`/`last` links for this page of `total_items`.
///
/// Each is emitted only when it names a page other than the one being served,
/// so the final page carries no `next` for a client to follow into an empty
/// feed, and a client landing past the end still gets `first`/`last` back.
fn pagination_links(
	base_url: &str,
	pagination: &OffsetPagination,
	total_items: u64,
) -> APIResult<Vec<OPDSLink>> {
	let page_link = |page: u64, rel: OPDSLinkRel| -> APIResult<OPDSLink> {
		Ok(OPDSLink::Link(
			OPDSBaseLinkBuilder::default()
				.href(page_url(base_url, pagination, page))
				.rel(rel.item())
				.build()?,
		))
	};

	let (first, last) = page_bounds(pagination, total_items);
	let has_next = pagination.offset().saturating_add(pagination.limit()) < total_items;

	let mut links = Vec::with_capacity(4);
	if pagination.page != first {
		links.push(page_link(first, OPDSLinkRel::First)?);
	}
	if let Some(previous) = pagination.previous_page() {
		links.push(page_link(previous, OPDSLinkRel::Previous)?);
	}
	if has_next {
		links.push(page_link(pagination.next_page(), OPDSLinkRel::Next)?);
	}
	if pagination.page != last {
		links.push(page_link(last, OPDSLinkRel::Last)?);
	}

	Ok(links)
}

/// The links of a paginated group inside a feed: `self`, plus the pages around
/// it. A group is one page of the route it names, which is where a client goes
/// to page that kind on its own.
fn paginated_group_links(
	link_finalizer: &OPDSLinkFinalizer,
	base_url: &str,
	pagination: &OffsetPagination,
	total_items: u64,
) -> APIResult<Vec<OPDSLink>> {
	let mut links = vec![OPDSLink::Link(
		OPDSBaseLinkBuilder::default()
			.href(page_url(base_url, pagination, pagination.page))
			.rel(OPDSLinkRel::SelfLink.item())
			.build()?,
	)];
	links.extend(pagination_links(base_url, pagination, total_items)?);

	Ok(link_finalizer.finalize_all(links))
}

/// A paginated feed's own links: `self`, `start`, and the pages around it.
fn paginated_feed_links(
	link_finalizer: &OPDSLinkFinalizer,
	base_url: &str,
	pagination: &OffsetPagination,
	total_items: u64,
) -> APIResult<Vec<OPDSLink>> {
	let mut links =
		paginated_group_links(link_finalizer, base_url, pagination, total_items)?;
	links.insert(
		1,
		link_finalizer.finalize(OPDSLink::Link(
			OPDSBaseLinkBuilder::default()
				.href(CATALOG_ROUTE.to_string())
				.rel(OPDSLinkRel::Start.item())
				.build()?,
		)),
	);

	Ok(links)
}

/// The metadata of one page of a feed or group: what it holds, and where the
/// page sits in the whole.
fn page_metadata(
	title: &str,
	subtitle: Option<String>,
	pagination: &OffsetPagination,
	total_items: u64,
) -> APIResult<OPDSMetadata> {
	// OPDSPaginationMetadata states currentPage 1-indexed, whichever numbering
	// the request used
	let current_page =
		pagination.page + u64::from(pagination.zero_based.unwrap_or(false));

	Ok(OPDSMetadataBuilder::default()
		.title(title.to_string())
		.subtitle(subtitle)
		.pagination(Some(
			OPDSPaginationMetadataBuilder::default()
				.number_of_items(total_items)
				.items_per_page(pagination.limit())
				.current_page(current_page)
				.build()?,
		))
		.build()?)
}

#[tracing::instrument(skip(ctx))]
pub(crate) async fn auth(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
) -> APIResult<OPDSAuthDocWrapper> {
	let mut links = vec![OPDSLink::help()];
	if let Some(favicon_path) = relative_favicon_path(ctx.config.protocols.enable_webui) {
		links.push(OPDSLink::logo(format!("{}{}", host.url(), favicon_path)));
	}

	Ok(OPDSAuthDocWrapper(
		OPDSAuthenticationDocumentBuilder::default()
			.description(OPDSSupportedAuthFlow::Basic.description().to_string())
			.links(links)
			.build()?,
	))
}

#[tracing::instrument(err, skip(ctx))]
pub(crate) async fn catalog(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();
	let link_finalizer = OPDSLinkFinalizer::from(host);
	let preview = preview_pagination();

	let libraries = library::Entity::find_for_user(&user)
		.order_by_asc(library::Column::Name)
		.limit(DEFAULT_LIMIT)
		.all(ctx.conn.as_ref())
		.await?;
	let library_count = library::Entity::find_for_user(&user)
		.count(ctx.conn.as_ref())
		.await?;
	let library_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata("Libraries", None, &preview, library_count)?)
		.links(paginated_group_links(
			&link_finalizer,
			LIBRARIES_ROUTE,
			&preview,
			library_count,
		)?)
		.navigation(
			libraries
				.into_iter()
				.map(OPDSNavigationLink::from)
				.map(|link| link.finalize(&link_finalizer))
				.collect::<Vec<OPDSNavigationLink>>(),
		)
		.build()?;

	let latest_books = OPDSPublicationEntity::find_for_user(&user)
		.limit(DEFAULT_LIMIT)
		.order_by_desc(media::Column::CreatedAt)
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let latest_books_count = media::Entity::find_for_user(&user)
		.count(ctx.conn.as_ref())
		.await?;
	let publications = OPDSPublication::vec_from_books(
		ctx.conn.as_ref(),
		link_finalizer.clone(),
		latest_books,
	)
	.await?;
	let latest_books_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata(
			"Latest Books",
			None,
			&preview,
			latest_books_count,
		)?)
		.links(paginated_group_links(
			&link_finalizer,
			LATEST_BOOKS_ROUTE,
			&preview,
			latest_books_count,
		)?)
		.publications(publications)
		.build()?;

	let in_progress_filter = Condition::all()
		.add(reading_session::Column::UserId.eq(user.id.clone()))
		.add(reading_session::Column::Status.eq(ReadingStatus::Reading));
	let continue_reading = OPDSPublicationEntity::find_for_user(&user)
		.filter(in_progress_filter.clone())
		.limit(DEFAULT_LIMIT)
		.order_by_desc(reading_session::Column::UpdatedAt)
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let continue_reading_count = OPDSPublicationEntity::find_for_user(&user)
		.filter(in_progress_filter)
		.count(ctx.conn.as_ref())
		.await?;

	let publications = OPDSPublication::vec_from_books(
		ctx.conn.as_ref(),
		link_finalizer.clone(),
		continue_reading,
	)
	.await?;
	let keep_reading_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata(
			"Keep Reading",
			None,
			&preview,
			continue_reading_count,
		)?)
		.links(paginated_group_links(
			&link_finalizer,
			KEEP_READING_ROUTE,
			&preview,
			continue_reading_count,
		)?)
		.publications(publications)
		.build()?;

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(
				OPDSMetadataBuilder::default()
					.title("Coppice OPDS V2 Catalog".to_string())
					.modified(OPDSMetadata::generate_modified())
					.build()?,
			)
			.links(link_finalizer.finalize_all(vec![
				OPDSBaseLinkBuilder::default()
					.href(CATALOG_ROUTE.to_string())
					.rel(OPDSLinkRel::SelfLink.item())
					.build()?
					.as_link(),
				OPDSBaseLinkBuilder::default()
					.href(CATALOG_ROUTE.to_string())
					.rel(OPDSLinkRel::Start.item())
					.build()?
					.as_link(),
				OPDSBaseLinkBuilder::default()
					.href(format!("{SEARCH_ROUTE}{{?query}}"))
					.rel(OPDSLinkRel::Search.item())
					._type(OPDSLinkType::OpdsJson)
					.templated(true)
					.build()?
					.as_link(),
			]))
			.navigation(vec![OPDSNavigationLinkBuilder::default()
				.title("Libraries".to_string())
				.base_link(
					OPDSBaseLinkBuilder::default()
						.href(link_finalizer.format_link(LIBRARIES_ROUTE))
						.rel(OPDSLinkRel::Subsection.item())
						.build()?,
				)
				.build()?])
			.groups(vec![library_group, latest_books_group, keep_reading_group])
			.build()?,
	))
}

/// The search term a search feed needs in order to run at all.
fn search_term(query: Option<String>) -> APIResult<String> {
	query.ok_or(APIError::BadRequest(
		"Query parameter is required".to_string(),
	))
}

/// The URL of a search route for `query`, carrying no page of its own: the
/// link builders add one page per link.
fn search_url(route: &str, query: &str) -> String {
	format!("{route}?query={}", urlencoding::encode(query))
}

/// One page of the libraries matching `query`, and the total number of matches.
///
/// The order is explicit because a page without one is not reproducible: the
/// second page of an unordered query can repeat or skip rows.
async fn search_libraries_page(
	ctx: &Ctx,
	link_finalizer: &OPDSLinkFinalizer,
	for_user: &AuthUser,
	query: &str,
	pagination: &OffsetPagination,
) -> APIResult<(Vec<OPDSNavigationLink>, u64)> {
	let matches = library::Column::Name.contains(query);

	let libraries = library::Entity::find_for_user(for_user)
		.filter(matches.clone())
		.order_by_asc(library::Column::Name)
		.limit(pagination.limit())
		.offset(pagination.offset())
		.all(ctx.conn.as_ref())
		.await?;
	let count = library::Entity::find_for_user(for_user)
		.filter(matches)
		.count(ctx.conn.as_ref())
		.await?;

	Ok((
		libraries
			.into_iter()
			.map(OPDSNavigationLink::from)
			.map(|link| link.finalize(link_finalizer))
			.collect(),
		count,
	))
}

/// One page of the series matching `query`, and the total number of matches.
async fn search_series_page(
	ctx: &Ctx,
	link_finalizer: &OPDSLinkFinalizer,
	for_user: &AuthUser,
	query: &str,
	pagination: &OffsetPagination,
) -> APIResult<(Vec<OPDSNavigationLink>, u64)> {
	let matches = Condition::any()
		.add(series::Column::Name.contains(query))
		.add(series_metadata::Column::Title.contains(query));

	let series = series::Entity::find_for_user(for_user)
		.left_join(series_metadata::Entity)
		.filter(matches.clone())
		.order_by_asc(series::Column::Name)
		.limit(pagination.limit())
		.offset(pagination.offset())
		.all(ctx.conn.as_ref())
		.await?;
	let count = series::Entity::find_for_user(for_user)
		.left_join(series_metadata::Entity)
		.filter(matches)
		.count(ctx.conn.as_ref())
		.await?;

	Ok((
		series
			.into_iter()
			.map(OPDSNavigationLink::from)
			.map(|link| link.finalize(link_finalizer))
			.collect(),
		count,
	))
}

/// One page of the books matching `query`, and the total number of matches.
async fn search_books_page(
	ctx: &Ctx,
	link_finalizer: &OPDSLinkFinalizer,
	for_user: &AuthUser,
	query: &str,
	pagination: &OffsetPagination,
) -> APIResult<(Vec<OPDSPublication>, u64)> {
	let matches = Condition::any()
		.add(media::Column::Name.contains(query))
		.add(media_metadata::Column::Title.contains(query));

	let books = OPDSPublicationEntity::find_for_user(for_user)
		.filter(matches.clone())
		.order_by_asc(media::Column::Name)
		.limit(pagination.limit())
		.offset(pagination.offset())
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let count = OPDSPublicationEntity::find_for_user(for_user)
		.filter(matches)
		.count(ctx.conn.as_ref())
		.await?;
	let publications =
		OPDSPublication::vec_from_books(ctx.conn.as_ref(), link_finalizer.clone(), books)
			.await?;

	Ok((publications, count))
}

/// A route handler which returns one page of each kind of match for a search.
/// Each group is one page of the per-kind route its `self` link names, so a
/// client that wants more of one kind pages that route.
#[tracing::instrument(err, skip(ctx))]
pub(crate) async fn search(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Query(OPDSSearchQuery { query }): Query<OPDSSearchQuery>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();
	let link_finalizer = OPDSLinkFinalizer::from(host);
	let query = search_term(query)?;
	let pagination = pagination.0;

	let (libraries, library_count) =
		search_libraries_page(&ctx, &link_finalizer, &user, &query, &pagination).await?;
	let library_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata(
			"Libraries",
			None,
			&pagination,
			library_count,
		)?)
		.links(paginated_group_links(
			&link_finalizer,
			&search_url(LIBRARY_SEARCH_ROUTE, &query),
			&pagination,
			library_count,
		)?)
		.navigation(libraries)
		.build()?;

	let (series, series_count) =
		search_series_page(&ctx, &link_finalizer, &user, &query, &pagination).await?;
	let series_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata("Series", None, &pagination, series_count)?)
		.links(paginated_group_links(
			&link_finalizer,
			&search_url(SERIES_SEARCH_ROUTE, &query),
			&pagination,
			series_count,
		)?)
		.navigation(series)
		.build()?;

	let (publications, book_count) =
		search_books_page(&ctx, &link_finalizer, &user, &query, &pagination).await?;
	let books_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata("Books", None, &pagination, book_count)?)
		.links(paginated_group_links(
			&link_finalizer,
			&search_url(BOOK_SEARCH_ROUTE, &query),
			&pagination,
			book_count,
		)?)
		.publications(publications)
		.build()?;

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(
				OPDSMetadataBuilder::default()
					.title(format!("Search - {query}"))
					.modified(OPDSMetadata::generate_modified())
					.build()?,
			)
			.links(link_finalizer.finalize_all(vec![
				OPDSBaseLinkBuilder::default()
					.href(page_url(
						&search_url(SEARCH_ROUTE, &query),
						&pagination,
						pagination.page,
					))
					.rel(OPDSLinkRel::SelfLink.item())
					.build()?
					.as_link(),
				OPDSBaseLinkBuilder::default()
					.href(CATALOG_ROUTE.to_string())
					.rel(OPDSLinkRel::Start.item())
					.build()?
					.as_link(),
			]))
			.groups(vec![library_group, series_group, books_group])
			.build()?,
	))
}

/// A route handler which returns one page of the libraries matching a search.
#[tracing::instrument(err, skip(ctx))]
pub(crate) async fn search_libraries(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Query(OPDSSearchQuery { query }): Query<OPDSSearchQuery>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();
	let link_finalizer = OPDSLinkFinalizer::from(host);
	let query = search_term(query)?;
	let pagination = pagination.0;

	let (libraries, count) =
		search_libraries_page(&ctx, &link_finalizer, &user, &query, &pagination).await?;

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(page_metadata(
				&format!("Libraries - {query}"),
				None,
				&pagination,
				count,
			)?)
			.links(paginated_feed_links(
				&link_finalizer,
				&search_url(LIBRARY_SEARCH_ROUTE, &query),
				&pagination,
				count,
			)?)
			.navigation(libraries)
			.build()?,
	))
}

/// A route handler which returns one page of the series matching a search.
#[tracing::instrument(err, skip(ctx))]
pub(crate) async fn search_series(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Query(OPDSSearchQuery { query }): Query<OPDSSearchQuery>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();
	let link_finalizer = OPDSLinkFinalizer::from(host);
	let query = search_term(query)?;
	let pagination = pagination.0;

	let (series, count) =
		search_series_page(&ctx, &link_finalizer, &user, &query, &pagination).await?;

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(page_metadata(
				&format!("Series - {query}"),
				None,
				&pagination,
				count,
			)?)
			.links(paginated_feed_links(
				&link_finalizer,
				&search_url(SERIES_SEARCH_ROUTE, &query),
				&pagination,
				count,
			)?)
			.navigation(series)
			.build()?,
	))
}

/// A route handler which returns one page of the books matching a search.
#[tracing::instrument(err, skip(ctx))]
pub(crate) async fn search_books(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Query(OPDSSearchQuery { query }): Query<OPDSSearchQuery>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();
	let link_finalizer = OPDSLinkFinalizer::from(host);
	let query = search_term(query)?;
	let pagination = pagination.0;

	let (publications, count) =
		search_books_page(&ctx, &link_finalizer, &user, &query, &pagination).await?;

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(page_metadata(
				&format!("Books - {query}"),
				None,
				&pagination,
				count,
			)?)
			.links(paginated_feed_links(
				&link_finalizer,
				&search_url(BOOK_SEARCH_ROUTE, &query),
				&pagination,
				count,
			)?)
			.publications(publications)
			.build()?,
	))
}

/// A route handler which returns a feed of libraries for a user. The feed includes groups for
/// series and books in each library.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn browse_libraries(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let link_finalizer = OPDSLinkFinalizer::from(host);

	let user = req.user();

	let pagination = pagination.0;
	let preview = preview_pagination();
	let take = pagination.limit();

	let libraries = library::Entity::find_for_user(&user)
		.limit(take)
		.offset(pagination.offset())
		.order_by_asc(library::Column::Name)
		.all(ctx.conn.as_ref())
		.await?;
	let library_count = library::Entity::find_for_user(&user)
		.count(ctx.conn.as_ref())
		.await?;

	let series = series::Entity::find_for_user(&user)
		.limit(DEFAULT_LIMIT)
		.order_by_asc(series::Column::Name)
		.all(ctx.conn.as_ref())
		.await?;
	let series_count = series::Entity::find_for_user(&user)
		.count(ctx.conn.as_ref())
		.await?;

	let series_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata("Series", None, &preview, series_count)?)
		.links(paginated_group_links(
			&link_finalizer,
			SERIES_ROUTE,
			&preview,
			series_count,
		)?)
		.navigation(
			series
				.into_iter()
				.map(OPDSNavigationLink::from)
				.map(|link| link.finalize(&link_finalizer))
				.collect::<Vec<OPDSNavigationLink>>(),
		)
		.build()?;

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(page_metadata(
				"Browse Libraries",
				None,
				&pagination,
				library_count,
			)?)
			.links(paginated_feed_links(
				&link_finalizer,
				LIBRARIES_ROUTE,
				&pagination,
				library_count,
			)?)
			.navigation(
				libraries
					.into_iter()
					.map(OPDSNavigationLink::from)
					.map(|link| link.finalize(&link_finalizer))
					.collect::<Vec<OPDSNavigationLink>>(),
			)
			.groups(vec![series_group])
			.build()?,
	))
}

#[tracing::instrument(skip(ctx))]
pub(crate) async fn browse_library_by_id(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Path(id): Path<String>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let link_finalizer = OPDSLinkFinalizer::from(host);

	let user = req.user();
	let preview = preview_pagination();

	let library = library::Entity::find_for_user(&user)
		.filter(library::Column::Id.eq(id.clone()))
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Library not found".to_string()))?;

	let library_books = OPDSPublicationEntity::find_for_user(&user)
		.filter(series::Column::LibraryId.eq(id.clone()))
		.limit(DEFAULT_LIMIT)
		.order_by_asc(media::Column::Name)
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let library_books_count = OPDSPublicationEntity::find_for_user(&user)
		.filter(series::Column::LibraryId.eq(id.clone()))
		.count(ctx.conn.as_ref())
		.await?;

	let books_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata(
			"Library Books - All",
			None,
			&preview,
			library_books_count,
		)?)
		.links(paginated_group_links(
			&link_finalizer,
			&format!("{LIBRARIES_ROUTE}/{id}/books"),
			&preview,
			library_books_count,
		)?)
		.publications(
			OPDSPublication::vec_from_books(
				ctx.conn.as_ref(),
				link_finalizer.clone(),
				library_books,
			)
			.await?,
		)
		.build()?;

	let latest_library_books = OPDSPublicationEntity::find_for_user(&user)
		.filter(series::Column::LibraryId.eq(id.clone()))
		.limit(DEFAULT_LIMIT)
		.order_by_desc(media::Column::CreatedAt)
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let latest_books_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata(
			"Library Books - Latest",
			None,
			&preview,
			library_books_count,
		)?)
		.links(paginated_group_links(
			&link_finalizer,
			&format!("{LIBRARIES_ROUTE}/{id}/books/latest"),
			&preview,
			library_books_count,
		)?)
		.publications(
			OPDSPublication::vec_from_books(
				ctx.conn.as_ref(),
				link_finalizer.clone(),
				latest_library_books,
			)
			.await?,
		)
		.build()?;

	let library_series = series::Entity::find_for_user(&user)
		.filter(series::Column::LibraryId.eq(id.clone()))
		.limit(DEFAULT_LIMIT)
		.order_by_asc(series::Column::Name)
		.all(ctx.conn.as_ref())
		.await?;
	let library_series_count = series::Entity::find_for_user(&user)
		.filter(series::Column::LibraryId.eq(id.clone()))
		.count(ctx.conn.as_ref())
		.await?;

	let series_group = OPDSFeedGroupBuilder::default()
		.metadata(page_metadata(
			"Library Series",
			None,
			&preview,
			library_series_count,
		)?)
		// .links(vec![OPDSLink::Link(
		// 	OPDSBaseLinkBuilder::default()
		// 		.href(format!("/opds/v2.0/libraries/{id}/series"))
		// 		.rel(OPDSLinkRel::SelfLink.item()) // TODO(OPDS-V2): Not self
		// 		.build()?,
		// )])
		.navigation(
			library_series
				.into_iter()
				.map(OPDSNavigationLink::from)
				.map(|link| link.finalize(&link_finalizer))
				.collect::<Vec<OPDSNavigationLink>>(),
		)
		.build()?;

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(OPDSMetadataBuilder::default().title(library.name).build()?)
			.links(link_finalizer.finalize_all(vec![OPDSLink::Link(
				OPDSBaseLinkBuilder::default()
					.href(format!("{LIBRARIES_ROUTE}/{id}"))
					.rel(OPDSLinkRel::SelfLink.item())
					.build()?,
			)]))
			.groups(vec![books_group, latest_books_group, series_group])
			.build()?,
	))
}

/// A helper function to fetch books and generate an OPDS feed for a user. This is not a route
#[allow(clippy::too_many_arguments)]
async fn fetch_books_and_generate_feed<C>(
	ctx: &Ctx,
	link_finalizer: OPDSLinkFinalizer,
	for_user: &AuthUser,
	condition: Option<Condition>,
	order: (C, Order),
	pagination: OffsetPagination,
	title: &str,
	subtitle: Option<String>,
	base_url: &str,
) -> APIResult<Json<OPDSFeed>>
where
	C: ColumnTrait,
{
	let take = pagination.limit();

	let order_by_entity = order.0.entity_name().deref().to_string();
	let for_user_id = for_user.id.clone();
	let books = OPDSPublicationEntity::find_for_user(for_user)
		.apply_if(condition.clone(), |query, condition| {
			query.filter(condition)
		})
		.apply_if(
			(order_by_entity == *"reading_sessions").then_some(()),
			|query, _| {
				query.filter(reading_session::Column::UserId.eq(for_user_id.clone()))
			},
		)
		.limit(take)
		.offset(pagination.offset())
		.order_by(order.0, order.1)
		.into_model::<OPDSPublicationEntity>()
		.all(ctx.conn.as_ref())
		.await?;
	let for_user_id = for_user.id.clone();
	let books_count = OPDSPublicationEntity::find_for_user(for_user)
		.apply_if(condition, |query, condition| query.filter(condition))
		.apply_if(
			(order_by_entity == *"reading_sessions").then_some(()),
			|query, _| {
				query.filter(reading_session::Column::UserId.eq(for_user_id.clone()))
			},
		)
		.count(ctx.conn.as_ref())
		.await?;
	let mut publications = OPDSPublication::vec_from_books(
		ctx.conn.as_ref(),
		link_finalizer.clone(),
		books.clone(),
	)
	.await?;
	#[cfg(feature = "readium")]
	{
		let ready = read_aloud_ready_books(ctx, for_user, &books).await?;
		for (publication, book) in publications.iter_mut().zip(&books) {
			if ready.contains(&book.media.id) {
				publication.add_read_aloud_link(&book.media.id, &link_finalizer)?;
			}
		}
	}

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(page_metadata(title, subtitle, &pagination, books_count)?)
			.links(paginated_feed_links(
				&link_finalizer,
				base_url,
				&pagination,
				books_count,
			)?)
			.publications(publications)
			.build()?,
	))
}

#[cfg(feature = "readium")]
async fn read_aloud_ready_books(
	ctx: &Ctx,
	user: &AuthUser,
	books: &[OPDSPublicationEntity],
) -> APIResult<HashSet<String>> {
	let mut ready = HashSet::new();
	for book in books {
		let Some(map) = sync_maps::latest_sync_map_for_media_user(
			ctx.conn.as_ref(),
			&user.id,
			&book.media.id,
			AlignGranularity::Sentence,
		)
		.await
		.map_err(|error| APIError::InternalServerError(error.to_string()))?
		else {
			continue;
		};
		let path =
			sync_maps::read_aloud_cache_path(ctx.config.get_transform_cache_dir(), &map)
				.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		if tokio::fs::metadata(path).await.is_ok() {
			ready.insert(book.media.id.clone());
		}
	}
	Ok(ready)
}

/// A route handler which returns a feed of books for a library.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn browse_library_books(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Path(id): Path<String>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();

	fetch_books_and_generate_feed(
		&ctx,
		OPDSLinkFinalizer::from(host),
		&user,
		Some(Condition::all().add(series::Column::LibraryId.eq(id.clone()))),
		(media::Column::Name, Order::Asc),
		pagination.0,
		"Library Books - All",
		None,
		format!("{LIBRARIES_ROUTE}/{id}/books").as_str(),
	)
	.await
}

#[tracing::instrument(skip(ctx))]
pub(crate) async fn latest_library_books(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Path(id): Path<String>,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();

	fetch_books_and_generate_feed(
		&ctx,
		OPDSLinkFinalizer::from(host),
		&user,
		Some(Condition::all().add(series::Column::LibraryId.eq(id.clone()))),
		(media::Column::CreatedAt, Order::Desc),
		pagination.0,
		"Library Books - Latest",
		None,
		format!("{LIBRARIES_ROUTE}/{id}/books/latest").as_str(),
	)
	.await
}

#[tracing::instrument(skip(ctx))]
pub(crate) async fn browse_series(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();

	let pagination = pagination.0;
	let take = pagination.limit();
	let series = series::Entity::find_for_user(&user)
		.limit(take)
		.offset(pagination.offset())
		.order_by_asc(series::Column::Name)
		.all(ctx.conn.as_ref())
		.await?;
	let series_count = series::Entity::find_for_user(&user)
		.count(ctx.conn.as_ref())
		.await?;

	let link_finalizer = OPDSLinkFinalizer::from(host);

	Ok(Json(
		OPDSFeedBuilder::default()
			.metadata(page_metadata(
				"Browse Series",
				None,
				&pagination,
				series_count,
			)?)
			.links(paginated_feed_links(
				&link_finalizer,
				SERIES_ROUTE,
				&pagination,
				series_count,
			)?)
			.navigation(
				series
					.into_iter()
					.map(OPDSNavigationLink::from)
					.map(|link| link.finalize(&link_finalizer))
					.collect::<Vec<OPDSNavigationLink>>(),
			)
			.build()?,
	))
}

#[tracing::instrument(skip(ctx))]
pub(crate) async fn browse_series_by_id(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	pagination: Query<OffsetPagination>,
	Path(id): Path<String>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();

	let series::ModelWithMetadata { series, metadata } =
		series::ModelWithMetadata::find_for_user(&user)
			.filter(series::Column::Id.eq(id.clone()))
			.into_model::<series::ModelWithMetadata>()
			.one(ctx.conn.as_ref())
			.await?
			.ok_or(APIError::NotFound("Series not found".to_string()))?;
	let name = series.name.clone();

	let title = metadata
		.and_then(|m| m.title)
		.or(Some(name))
		.unwrap_or_else(|| format!("Series {}", id));

	fetch_books_and_generate_feed(
		&ctx,
		OPDSLinkFinalizer::from(host),
		&user,
		Some(Condition::all().add(media::Column::SeriesId.eq(id.clone()))),
		(media::Column::Name, Order::Asc),
		pagination.0,
		&title,
		None,
		&format!("{SERIES_ROUTE}/{id}"),
	)
	.await
}

/// A route handler which returns a feed of books for a user
#[tracing::instrument(skip(ctx))]
pub(crate) async fn browse_books(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Query(params): Query<OPDSBrowseParams>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();

	let filter_query_string = params.filter.to_query_string();
	let subtitle = params.filter.subtitle();

	let base_url = if filter_query_string.is_empty() {
		BROWSE_BOOKS_ROUTE.to_string()
	} else {
		format!("{BROWSE_BOOKS_ROUTE}?{filter_query_string}")
	};

	let condition = params.filter.into_condition();

	fetch_books_and_generate_feed(
		&ctx,
		OPDSLinkFinalizer::from(host),
		&user,
		condition,
		(media::Column::Name, Order::Asc),
		params.pagination,
		"Browse Books",
		subtitle,
		&base_url,
	)
	.await
}

/// A route handler which returns the latest books for a user as an OPDS feed.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn latest_books(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();

	fetch_books_and_generate_feed(
		&ctx,
		OPDSLinkFinalizer::from(host),
		&user,
		None,
		(media::Column::CreatedAt, Order::Desc),
		pagination.0,
		"Latest Books",
		None,
		LATEST_BOOKS_ROUTE,
	)
	.await
}

/// A route handler which returns the books a user is currently reading. A user is currently reading
/// a book if there exists an active reading session for the user.
///
/// Completed books are not included in this feed.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn keep_reading(
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	pagination: Query<OffsetPagination>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSFeed>> {
	let user = req.user();
	let newer_exists = reading_session::Entity::newer_session_exists_subquery();

	fetch_books_and_generate_feed(
		&ctx,
		OPDSLinkFinalizer::from(host),
		&user,
		Some(
			Condition::all()
				.add(reading_session::Column::UserId.eq(user.id.clone()))
				.add(reading_session::Column::Status.eq(ReadingStatus::Reading))
				.add(Expr::expr(Expr::exists(newer_exists)).not()),
		),
		(reading_session::Column::UpdatedAt, Order::Desc),
		pagination.0,
		"Currently Reading",
		None,
		KEEP_READING_ROUTE,
	)
	.await
}

#[tracing::instrument(skip(ctx))]
pub(crate) async fn get_book_by_id(
	Path(id): Path<String>,
	HostExtractor(host): HostExtractor,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSPublication>> {
	let book = OPDSPublicationEntity::find_for_user(&req.user())
		.filter(media::Column::Id.eq(id.clone()))
		.into_model::<OPDSPublicationEntity>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	let link_finalizer = OPDSLinkFinalizer::from(host);
	let mut publication = OPDSPublication::from_book(
		ctx.conn.as_ref(),
		link_finalizer.clone(),
		book.clone(),
	)
	.await?;
	#[cfg(feature = "readium")]
	if let Some(map) = sync_maps::latest_sync_map_for_media_user(
		ctx.conn.as_ref(),
		&req.user().id,
		&book.media.id,
		AlignGranularity::Sentence,
	)
	.await
	.map_err(|error| APIError::InternalServerError(error.to_string()))?
	{
		let path =
			sync_maps::read_aloud_cache_path(ctx.config.get_transform_cache_dir(), &map)
				.map_err(|error| APIError::InternalServerError(error.to_string()))?;
		if tokio::fs::metadata(path).await.is_ok() {
			publication.add_read_aloud_link(&book.media.id, &link_finalizer)?;
		}
	}
	Ok(Json(publication))
}

/// A route handler which returns a book thumbnail for a user as a valid image response.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn get_book_thumbnail(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<ImageResponse> {
	get_media_thumbnail_by_id(&ctx, &req.user(), id).await
}

/// A route handler which returns a single page of a book for a user as a valid image
/// response.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn get_book_page(
	Path((id, page)): Path<(String, i32)>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
) -> APIResult<ImageResponse> {
	let book = media::Entity::find_for_user(&req.user())
		.columns(vec![media::Column::Id, media::Column::Path])
		.filter(media::Column::Id.eq(id))
		.into_model::<media::MediaIdentSelect>()
		.one(ctx.conn.as_ref())
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	let (content_type, image_buffer) =
		get_page_async(PathBuf::from(book.path), page, &ctx.config.media).await?;

	Ok(ImageResponse::new(content_type, image_buffer))
}

// // .route("/chapter/{chapter}", get(get_epub_chapter))
// // .route("/{root}/{resource}", get(get_epub_meta)),
// // async fn get_book_resource() {}

/// A route handler which returns the progression of a book for a user.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn get_book_progression(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	HostExtractor(host): HostExtractor,
	Extension(req): Extension<AuthContext>,
) -> APIResult<Json<OPDSProgression>> {
	let link_finalizer = OPDSLinkFinalizer::from(host);

	let user = req.user();
	let conn = ctx.conn.as_ref();
	let Some(head) = reading_state::head(conn, &user.id, &id).await? else {
		return Ok(Json(OPDSProgression::default()));
	};
	let Some(book) = OPDSProgressionBookRef::find_by_media_id(&id)
		.one(conn)
		.await?
	else {
		return Ok(Json(OPDSProgression::default()));
	};
	let device = match head.source_device_id.as_deref() {
		Some(device_id) => device::Entity::find_by_id(device_id).one(conn).await?,
		None => None,
	};

	Ok(Json(OPDSProgression::new(
		OPDSProgressionEntity { head, device, book },
		link_finalizer,
	)?))
}

/// A route handler which updates the progression of a book for a user
///
/// Returns 204 on success, 409 Conflict if the timestamp is older.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn update_book_progression(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	Json(input): Json<OPDSProgressionInput>,
) -> APIResult<axum::http::StatusCode> {
	let user = req.user();
	let conn = ctx.conn.as_ref();

	let book = media::Entity::find_for_user(&user)
		.filter(media::Column::Id.eq(id.clone()))
		.one(conn)
		.await?
		.ok_or(APIError::NotFound("Book not found".to_string()))?;

	let device_id = if let Some(input_device) = input.device() {
		let existing_device = device::Entity::find_by_id(&input_device.id)
			.one(conn)
			.await?;

		if existing_device.is_none() {
			// OPDS 2.0 progression clients register themselves by the device
			// they report; the row is owned by the syncing user.
			let new_device = device::ActiveModel {
				id: Set(input_device.id.clone()),
				user_id: Set(user.id.clone()),
				name: Set(input_device.name.clone()),
				kind: Set(DeviceKind::Opds),
				last_seen_at: Set(Some(chrono::Utc::now().into())),
				created_at: Set(chrono::Utc::now().into()),
				..Default::default()
			};
			device::Entity::insert(new_device).exec(conn).await?;
		}

		Some(input_device.id.clone())
	} else {
		None
	};

	let page = input.page();
	let percentage = match page {
		Some(p) => Some(compute_page_based_percentage(p, book.pages)),
		None => input.percentage_completed(),
	};
	let did_complete = match page {
		Some(p) => p >= book.pages,
		None => percentage.unwrap_or_default() >= Decimal::new(1, 0),
	};

	match page {
		Some(p) if book.pages > -1 && (p < 1 || p > book.pages) => {
			return Err(APIError::BadRequest(format!(
				"Page {} is out of bounds (1-{})",
				p, book.pages
			)));
		},
		_ => {},
	}

	let locator = input.locator();
	let progression = NormalizedProgression {
		page,
		locator: locator.clone(),
		percentage,
		elapsed_seconds_delta: None,
		did_complete,
		device_id: device_id.clone(),
		reset_elapsed_seconds: false,
	};
	let head_update = ProtocolUpdate {
		protocol: SourceProtocol::Opds,
		device_id,
		updated_at: Some(input.modified.to_utc()),
		position: locator.map_or(Position::None, Position::Locator),
		progression: input
			.percentage_completed()
			.and_then(|value| value.to_f64()),
		completed: did_complete.then_some(true),
		raw_payload: serde_json::to_value(&input)
			.map_err(|error| APIError::InternalServerError(error.to_string()))?,
	};
	let sync_summary = serde_json::json!({
		"protocol": "opds",
		"media_id": id.clone(),
		"progression": percentage.as_ref().and_then(|value| value.to_f64()),
		"device": input.device.clone(),
	});

	let txn = begin_write(conn).await?;
	let applied =
		reading_state::apply(&txn, &user.id, Publication::from(&book), head_update)
			.await?;
	if applied.accepted() {
		upsert_reading_session(&txn, &user, &id, progression).await?;
	}
	txn.commit().await?;
	reading_state::announce(&ctx, &book, &applied);

	if !applied.accepted() {
		return Err(APIError::Conflict(
			"Progression timestamp is older than existing head".to_string(),
		));
	}
	if let Some(api_key) = req.api_key.as_deref() {
		if let Err(error) = ctx
			.devices()
			.touch(
				CredentialRef::ApiKey(api_key),
				Protocol::Opds,
				Some(sync_summary),
			)
			.await
		{
			tracing::warn!(?error, "Failed to record the OPDS sync on its device");
		}
	}

	Ok(axum::http::StatusCode::NO_CONTENT)
}

/// A route handler which downloads a book for a user.
#[tracing::instrument(skip(ctx))]
pub(crate) async fn download_book(
	Path(id): Path<String>,
	State(ctx): State<AppState>,
	Extension(req): Extension<AuthContext>,
	headers: HeaderMap,
) -> APIResult<impl IntoResponse> {
	serve_media::serve_media_file(req, headers, ctx.conn.as_ref(), id).await
}
