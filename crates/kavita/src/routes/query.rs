//! Shared loaders that turn user-visible Stump rows into mapper inputs.
//!
//! A Kavita series is a Stump series in Manga/Comic libraries and a single
//! media item in Book/LightNovel libraries ([`SeriesKey`]); everything that
//! takes a `seriesId` resolves through [`find_series_input`], and listings
//! select [`SeriesKey`]s with [`select_series_keys`] before loading them with
//! [`load_by_keys`].

use std::collections::{HashMap, HashSet};

use models::entity::{
	kavita_on_deck_removal, library, library_config, media, series, user::AuthUser,
};
use sea_orm::{
	prelude::*,
	sea_query::{Alias, Asterisk, Expr, Func, Query, SelectStatement, UnionType},
	QueryOrder, QueryTrait,
};

use crate::{
	errors::APIResult,
	ids::{IdKind, KavitaIds, LOOKUP_CHUNK},
	mapper::{sort_media, MediaInput, SeriesInput, SeriesKind, BOOK_LIBRARY_TYPES},
	progress::latest_sessions,
};

use super::{
	series_filter::{FilterPlan, SortKey, Target},
	KavitaBackend,
};

/// The Stump row behind a Kavita series.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SeriesKey {
	/// A Stump series (Manga/Comic libraries).
	Series(String),
	/// A media item of a Book/LightNovel library, presented as its own series.
	Book(String),
}

impl SeriesKey {
	/// The `(target_kind, target_id)` pair `kavita_on_deck_removals` stores.
	pub fn target(&self) -> (&'static str, &str) {
		match self {
			Self::Series(id) => (kavita_on_deck_removal::Model::TARGET_SERIES, id),
			Self::Book(id) => (kavita_on_deck_removal::Model::TARGET_MEDIA, id),
		}
	}

	fn from_target(kind: &str, id: String) -> Option<Self> {
		match kind {
			kavita_on_deck_removal::Model::TARGET_SERIES => Some(Self::Series(id)),
			kavita_on_deck_removal::Model::TARGET_MEDIA => Some(Self::Book(id)),
			_ => None,
		}
	}
}

impl SeriesInput {
	pub(crate) fn key(&self) -> SeriesKey {
		match self.book() {
			Some(book) => SeriesKey::Book(book.media.id.clone()),
			None => SeriesKey::Series(self.series.id.clone()),
		}
	}
}

/// Narrow a [`FilterPlan`] to a set of Kavita series keys. An empty key set
/// on either side yields a condition nothing satisfies, so a user with no
/// want-to-read entries gets an empty page rather than every series.
pub(crate) fn restrict_plan(plan: &mut FilterPlan, keys: &[SeriesKey]) {
	let mut series_ids = Vec::new();
	let mut media_ids = Vec::new();
	for key in keys {
		match key {
			SeriesKey::Series(id) => series_ids.push(id.clone()),
			SeriesKey::Book(id) => media_ids.push(id.clone()),
		}
	}
	plan.condition = plan
		.condition
		.clone()
		.add(series::Column::Id.is_in(series_ids));
	plan.book_condition = plan
		.book_condition
		.clone()
		.add(media::Column::Id.is_in(media_ids));
}

/// The libraries whose media are their own Kavita series: Stump `Book`,
/// `WebNovel` and `LightNovel` libraries (Kavita `Book`/`LightNovel`).
pub(crate) async fn book_library_ids(
	ctx: &dyn KavitaBackend,
) -> APIResult<HashSet<String>> {
	Ok(library_config::Entity::find()
		.filter(library_config::Column::LibraryType.is_in(BOOK_LIBRARY_TYPES))
		.all(ctx.conn())
		.await?
		.into_iter()
		.filter_map(|config| config.library_id)
		.collect())
}

/// The Kavita series ids a user removed from on-deck.
pub(crate) async fn on_deck_removals(
	ctx: &dyn KavitaBackend,
	user_id: &str,
) -> APIResult<HashSet<SeriesKey>> {
	Ok(kavita_on_deck_removal::Entity::find_for_user(user_id)
		.all(ctx.conn())
		.await?
		.into_iter()
		.filter_map(|removal| {
			SeriesKey::from_target(&removal.target_kind, removal.target_id)
		})
		.collect())
}

/// Resolve a Kavita `seriesId` to the row it names, whichever kind it is.
pub(crate) async fn resolve_series_key(
	ctx: &dyn KavitaBackend,
	id: i32,
) -> APIResult<Option<SeriesKey>> {
	Ok(match KavitaIds::lookup_any(ctx.conn(), id).await? {
		Some((IdKind::Series, stump_id)) => Some(SeriesKey::Series(stump_id)),
		Some((IdKind::BookSeries, media_id)) => Some(SeriesKey::Book(media_id)),
		_ => None,
	})
}

/// Load one user-visible Kavita series by its Kavita id.
pub(crate) async fn find_series_input(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	id: i32,
) -> APIResult<Option<SeriesInput>> {
	let Some(key) = resolve_series_key(ctx, id).await? else {
		return Ok(None);
	};
	Ok(load_by_keys(ctx, user, std::slice::from_ref(&key))
		.await?
		.pop())
}

/// Load the Kavita series behind `keys`, in key order. Keys that are deleted,
/// hidden from the user or (for books) no longer in a Book library are
/// skipped.
pub(crate) async fn load_by_keys(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	keys: &[SeriesKey],
) -> APIResult<Vec<SeriesInput>> {
	let mut series_ids = Vec::new();
	let mut media_ids = Vec::new();
	for key in keys {
		match key {
			SeriesKey::Series(id) => series_ids.push(id.clone()),
			SeriesKey::Book(id) => media_ids.push(id.clone()),
		}
	}
	let mut loaded: HashMap<SeriesKey, SeriesInput> = HashMap::with_capacity(keys.len());
	for chunk in series_ids.chunks(LOOKUP_CHUNK) {
		let rows = series::ModelWithMetadata::find_for_user(user)
			.filter(series::Column::Id.is_in(chunk.to_vec()))
			.filter(series::Column::DeletedAt.is_null())
			.into_model::<series::ModelWithMetadata>()
			.all(ctx.conn())
			.await?;
		for input in load_series_inputs(ctx, user, rows).await? {
			loaded.insert(input.key(), input);
		}
	}
	if !media_ids.is_empty() {
		let book_libraries = book_library_ids(ctx).await?;
		for chunk in media_ids.chunks(LOOKUP_CHUNK) {
			let rows = media::ModelWithMetadata::find_for_user(user)
				.filter(media::Column::Id.is_in(chunk.to_vec()))
				.filter(media::Column::DeletedAt.is_null())
				.into_model::<media::ModelWithMetadata>()
				.all(ctx.conn())
				.await?;
			for input in load_book_inputs(ctx, user, rows, &book_libraries).await? {
				loaded.insert(input.key(), input);
			}
		}
	}
	Ok(keys.iter().filter_map(|key| loaded.remove(key)).collect())
}

/// Turn user-visible series rows into Kavita series: rows in Book libraries
/// become one input per media item, the rest keep their volumes.
pub(crate) async fn load_kavita_series(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	rows: Vec<series::ModelWithMetadata>,
) -> APIResult<Vec<SeriesInput>> {
	let book_libraries = book_library_ids(ctx).await?;
	let (book_rows, grouped_rows): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| {
		row.series
			.library_id
			.as_ref()
			.is_some_and(|library_id| book_libraries.contains(library_id))
	});
	let mut inputs = load_series_inputs(ctx, user, grouped_rows).await?;
	if book_rows.is_empty() {
		return Ok(inputs);
	}
	let series_ids = book_rows
		.iter()
		.map(|row| row.series.id.clone())
		.collect::<Vec<_>>();
	for chunk in series_ids.chunks(LOOKUP_CHUNK) {
		let media_rows = media::ModelWithMetadata::find_for_user(user)
			.filter(media::Column::SeriesId.is_in(chunk.to_vec()))
			.filter(media::Column::DeletedAt.is_null())
			.order_by_asc(media::Column::Name)
			.into_model::<media::ModelWithMetadata>()
			.all(ctx.conn())
			.await?;
		inputs.extend(load_book_inputs(ctx, user, media_rows, &book_libraries).await?);
	}
	Ok(inputs)
}

/// Build mapper inputs for a batch of grouped series with a bounded number of
/// queries: libraries, media, sessions and id allocations are each fetched
/// once for the whole batch.
pub(crate) async fn load_series_inputs(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	rows: Vec<series::ModelWithMetadata>,
) -> APIResult<Vec<SeriesInput>> {
	if rows.is_empty() {
		return Ok(Vec::new());
	}
	let conn = ctx.conn();
	let series_ids = rows
		.iter()
		.map(|row| row.series.id.clone())
		.collect::<Vec<_>>();
	let series_kavita_ids =
		KavitaIds::resolve_many(conn, IdKind::Series, &series_ids).await?;

	let library_ids = rows
		.iter()
		.filter_map(|row| row.series.library_id.clone())
		.collect::<Vec<_>>();
	let (libraries, library_kavita_ids) = load_libraries(ctx, library_ids).await?;

	let media_rows = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::SeriesId.is_in(series_ids.clone()))
		.filter(media::Column::DeletedAt.is_null())
		.order_by_asc(media::Column::Name)
		.into_model::<media::ModelWithMetadata>()
		.all(conn)
		.await?;
	let media_ids = media_rows
		.iter()
		.map(|row| row.media.id.clone())
		.collect::<Vec<_>>();
	let media_kavita_ids =
		KavitaIds::resolve_many(conn, IdKind::Media, &media_ids).await?;
	let mut sessions = latest_sessions(conn, user, &media_ids).await?;

	let mut media_by_series: HashMap<String, Vec<MediaInput>> = HashMap::new();
	for row in media_rows {
		let Some(series_id) = row.media.series_id.clone() else {
			continue;
		};
		let entry = media_by_series.entry(series_id).or_default();
		let ordinal = i32::try_from(entry.len() + 1)?;
		entry.push(MediaInput {
			id: media_kavita_ids[&row.media.id],
			session: sessions.remove(&row.media.id),
			media: row.media,
			metadata: row.metadata,
			ordinal,
			book: false,
		});
	}

	rows.into_iter()
		.map(|row| {
			let library_id = row.series.library_id.as_deref().unwrap_or_default();
			let mut media = media_by_series.remove(&row.series.id).unwrap_or_default();
			sort_media(&mut media);
			Ok(SeriesInput {
				id: series_kavita_ids[&row.series.id],
				library_id: library_kavita_ids.get(library_id).copied().unwrap_or(0),
				library_name: libraries
					.get(library_id)
					.map(|library| library.name.clone())
					.unwrap_or_default(),
				series: row.series,
				metadata: row.metadata,
				media,
				kind: SeriesKind::Grouped,
			})
		})
		.collect()
}

/// Build one Kavita series per media row of a Book library. Rows whose
/// library is not (or no longer) a Book library are skipped: their media are
/// volumes of a grouped series, not series of their own.
pub(crate) async fn load_book_inputs(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	rows: Vec<media::ModelWithMetadata>,
	book_libraries: &HashSet<String>,
) -> APIResult<Vec<SeriesInput>> {
	if rows.is_empty() {
		return Ok(Vec::new());
	}
	let conn = ctx.conn();
	let mut series_ids = rows
		.iter()
		.filter_map(|row| row.media.series_id.clone())
		.collect::<Vec<_>>();
	series_ids.sort();
	series_ids.dedup();
	let mut series_rows: HashMap<String, series::Model> =
		HashMap::with_capacity(series_ids.len());
	for chunk in series_ids.chunks(LOOKUP_CHUNK) {
		for row in series::Entity::find()
			.filter(series::Column::Id.is_in(chunk.to_vec()))
			.filter(series::Column::DeletedAt.is_null())
			.all(conn)
			.await?
		{
			series_rows.insert(row.id.clone(), row);
		}
	}
	let library_ids = series_rows
		.values()
		.filter_map(|row| row.library_id.clone())
		.collect::<Vec<_>>();
	let (libraries, library_kavita_ids) = load_libraries(ctx, library_ids).await?;

	let rows = rows
		.into_iter()
		.filter(|row| {
			row.media
				.series_id
				.as_ref()
				.and_then(|series_id| series_rows.get(series_id))
				.and_then(|series| series.library_id.as_ref())
				.is_some_and(|library_id| book_libraries.contains(library_id))
		})
		.collect::<Vec<_>>();
	let media_ids = rows
		.iter()
		.map(|row| row.media.id.clone())
		.collect::<Vec<_>>();
	let book_kavita_ids =
		KavitaIds::resolve_many(conn, IdKind::BookSeries, &media_ids).await?;
	let media_kavita_ids =
		KavitaIds::resolve_many(conn, IdKind::Media, &media_ids).await?;
	let mut sessions = latest_sessions(conn, user, &media_ids).await?;

	Ok(rows
		.into_iter()
		.map(|row| {
			let series =
				series_rows[row.media.series_id.as_deref().unwrap_or_default()].clone();
			let library_id = series.library_id.as_deref().unwrap_or_default();
			SeriesInput {
				id: book_kavita_ids[&row.media.id],
				library_id: library_kavita_ids.get(library_id).copied().unwrap_or(0),
				library_name: libraries
					.get(library_id)
					.map(|library| library.name.clone())
					.unwrap_or_default(),
				series,
				metadata: None,
				media: vec![MediaInput {
					id: media_kavita_ids[&row.media.id],
					session: sessions.remove(&row.media.id),
					media: row.media,
					metadata: row.metadata,
					ordinal: 1,
					book: true,
				}],
				kind: SeriesKind::Book,
			}
		})
		.collect())
}

/// Library rows and their Kavita ids for a (possibly repetitive) id list.
async fn load_libraries(
	ctx: &dyn KavitaBackend,
	mut library_ids: Vec<String>,
) -> APIResult<(HashMap<String, library::Model>, HashMap<String, i32>)> {
	library_ids.sort();
	library_ids.dedup();
	let libraries = library::Entity::find()
		.filter(library::Column::Id.is_in(library_ids.clone()))
		.all(ctx.conn())
		.await?
		.into_iter()
		.map(|library| (library.id.clone(), library))
		.collect::<HashMap<_, _>>();
	let kavita_ids =
		KavitaIds::resolve_many(ctx.conn(), IdKind::Library, &library_ids).await?;
	Ok((libraries, kavita_ids))
}

/// Load the Kavita series holding the media behind a Kavita volume/chapter
/// id, together with that media's position in it. In a Book library the
/// series is the book itself, so the position is `0`.
pub(crate) async fn find_media(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	id: i32,
) -> APIResult<Option<(SeriesInput, usize)>> {
	let conn = ctx.conn();
	let Some(media_id) = KavitaIds::lookup(conn, IdKind::Media, id).await? else {
		return Ok(None);
	};
	let Some(media_row) = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::Id.eq(media_id.clone()))
		.filter(media::Column::DeletedAt.is_null())
		.into_model::<media::ModelWithMetadata>()
		.one(conn)
		.await?
	else {
		return Ok(None);
	};
	let Some(series_id) = media_row.media.series_id.clone() else {
		return Ok(None);
	};
	let Some(series_row) = series::ModelWithMetadata::find_for_user(user)
		.filter(series::Column::Id.eq(series_id))
		.filter(series::Column::DeletedAt.is_null())
		.into_model::<series::ModelWithMetadata>()
		.one(conn)
		.await?
	else {
		return Ok(None);
	};
	let book_libraries = match series_row.series.library_id.as_ref() {
		Some(_) => book_library_ids(ctx).await?,
		None => HashSet::new(),
	};
	let in_book_library = series_row
		.series
		.library_id
		.as_ref()
		.is_some_and(|library_id| book_libraries.contains(library_id));
	if in_book_library {
		return Ok(
			load_book_inputs(ctx, user, vec![media_row], &book_libraries)
				.await?
				.pop()
				.map(|input| (input, 0)),
		);
	}
	let Some(input) = load_series_inputs(ctx, user, vec![series_row]).await?.pop() else {
		return Ok(None);
	};
	let index = input
		.media
		.iter()
		.position(|media| media.media.id == media_id);
	Ok(index.map(|index| (input, index)))
}

/// Library id/name for a series input's library, resolving the Kavita id.
pub(crate) async fn library_for_series(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	input: &SeriesInput,
) -> APIResult<Option<(library::Model, Option<library_config::Model>)>> {
	let Some(library_id) = input.series.library_id.as_deref() else {
		return Ok(None);
	};
	Ok(library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(library_id))
		.find_also_related(library_config::Entity)
		.one(ctx.conn())
		.await?)
}

const KIND_COLUMN: &str = "kind";
const ID_COLUMN: &str = "id";
const KIND_SERIES: &str = "series";
const KIND_BOOK: &str = "book";

fn sort_alias(index: usize) -> Alias {
	Alias::new(format!("k{index}"))
}

/// Project a filtered series or media query onto `(kind, id, k0, k1, ...)`,
/// the common shape both halves of the listing union share.
fn project_keys(
	mut select: SelectStatement,
	target: Target,
	sort: &[SortKey],
) -> SelectStatement {
	select.clear_selects();
	let (kind, id) = match target {
		Target::Series => (KIND_SERIES, Expr::col((series::Entity, series::Column::Id))),
		Target::Book => (KIND_BOOK, Expr::col((media::Entity, media::Column::Id))),
	};
	select.expr_as(Expr::val(kind), Alias::new(KIND_COLUMN));
	select.expr_as(id, Alias::new(ID_COLUMN));
	for (index, key) in sort.iter().enumerate() {
		select.expr_as(key.expr(target).clone(), sort_alias(index));
	}
	select
}

/// Select the Kavita series matching `plan`, sorted by `sort`: Stump series
/// outside Book libraries `UNION ALL` media inside them, so one SQL pass
/// orders, counts and pages both kinds together. Returns the page (or every
/// key when `page` is `None`) and the total before paging.
pub(crate) async fn select_series_keys(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	plan: &FilterPlan,
	sort: &[SortKey],
	book_libraries: &HashSet<String>,
	page: Option<(u64, u64)>,
) -> APIResult<(Vec<SeriesKey>, i32)> {
	let mut series_select =
		series::ModelWithMetadata::find_for_user(user).filter(plan.condition.clone());
	if !book_libraries.is_empty() {
		series_select = series_select.filter(
			series::Column::LibraryId
				.is_not_in(book_libraries.iter().cloned())
				.or(series::Column::LibraryId.is_null()),
		);
	}
	let mut union = project_keys(series_select.into_query(), Target::Series, sort);
	if !book_libraries.is_empty() {
		let book_select = media::ModelWithMetadata::find_for_user(user)
			.filter(series::Column::LibraryId.is_in(book_libraries.iter().cloned()))
			.filter(plan.book_condition.clone());
		union.union(
			UnionType::All,
			project_keys(book_select.into_query(), Target::Book, sort),
		);
	}

	let conn = ctx.conn();
	let backend = conn.get_database_backend();
	let source = Alias::new("kavita_series");

	let mut count = Query::select();
	count
		.expr_as(Func::count(Expr::col(Asterisk)), Alias::new("total"))
		.from_subquery(union.clone(), source.clone());
	let total = conn
		.query_one(backend.build(&count))
		.await?
		.map(|row| row.try_get::<i64>("", "total"))
		.transpose()?
		.unwrap_or(0);

	let mut keys = Query::select();
	keys.column(Alias::new(KIND_COLUMN))
		.column(Alias::new(ID_COLUMN))
		.from_subquery(union, source);
	for (index, key) in sort.iter().enumerate() {
		keys.order_by(sort_alias(index), key.order.clone());
	}
	if let Some((offset, limit)) = page {
		keys.offset(offset).limit(limit);
	}
	let rows = conn.query_all(backend.build(&keys)).await?;
	let mut selected = Vec::with_capacity(rows.len());
	for row in rows {
		let kind = row.try_get::<String>("", KIND_COLUMN)?;
		let id = row.try_get::<String>("", ID_COLUMN)?;
		selected.push(match kind.as_str() {
			KIND_BOOK => SeriesKey::Book(id),
			_ => SeriesKey::Series(id),
		});
	}
	Ok((selected, i32::try_from(total)?))
}
