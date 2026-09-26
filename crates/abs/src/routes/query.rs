//! The shared reads every item-serving route needs: which libraries have
//! audio, which books are in one, and how a page of them becomes
//! [`LibraryItemDto`]s without a query per row.

use std::collections::{HashMap, HashSet};

use models::entity::{library, media, media_metadata, series, user::AuthUser};
use sea_orm::{
	sea_query::{Expr, SimpleExpr},
	ColumnTrait, Condition, EntityTrait, FromQueryResult, IntoSimpleExpr, Order,
	PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Select,
};

use crate::{
	dto::{CollapsedSeriesDto, LibraryItemDto, MediaProgressDto},
	errors::{AbsError, AbsResult},
	mapper::{self, ItemInput},
	model::{AbsAudio, AbsEbookFile, AbsProgress, ItemShape},
	routes::AbsBackend,
};

/// The rows this profile serves are Stump's audio containers
/// (`models::entity::media::AUDIO_EXTENSIONS`). A row with any other
/// extension is not an ABS "library item" at all: Audiobookshelf has no
/// page-based reading position, so an ebook would be a book a client could
/// open and never track.
pub(crate) fn audio_condition() -> Condition {
	media::audio_extension_condition()
}

/// Books the request may see, narrowed to audio and to rows that still exist.
pub(crate) fn audio_media(user: &AuthUser) -> Select<media::Entity> {
	media::Entity::find_for_user(user)
		.filter(audio_condition())
		.filter(media::Column::DeletedAt.is_null())
}

/// The library ids that hold at least one audible book for this user.
///
/// `GET /api/libraries` hides everything else: a Lissen user who also has
/// comic libraries should see only the ones they can listen to, and an ABS
/// client offered an empty library has no way to say so.
pub(crate) async fn audio_library_ids(
	backend: &dyn AbsBackend,
	user: &AuthUser,
) -> AbsResult<HashSet<String>> {
	#[derive(FromQueryResult)]
	struct Row {
		library_id: String,
	}

	let rows = audio_media(user)
		.select_only()
		.column_as(series::Column::LibraryId, "library_id")
		.filter(series::Column::LibraryId.is_not_null())
		.distinct()
		.into_model::<Row>()
		.all(backend.conn())
		.await?;
	Ok(rows.into_iter().map(|row| row.library_id).collect())
}

/// One library the user may see, or `404`.
pub(crate) async fn library(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	library_id: &str,
) -> AbsResult<library::Model> {
	library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(library_id))
		.one(backend.conn())
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No library {library_id}")))
}

/// How `GET /api/libraries/{id}/items` was asked to sort.
///
/// The five values are exactly the ones Lissen sends
/// (`library/converter/LibraryOrderingRequestConverter.kt:14-20` for the
/// first four, `library/LibraryAudiobookshelfChannel.kt:85-89` for
/// `sequence`); anything else falls back to title, which is abs-ref's
/// behaviour for an unknown `sort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemSort {
	Title,
	AuthorName,
	AddedAt,
	ModifiedAt,
	Sequence,
}

impl ItemSort {
	pub(crate) fn parse(value: &str) -> Self {
		match value {
			"media.metadata.authorName" => ItemSort::AuthorName,
			"addedAt" => ItemSort::AddedAt,
			"mtimeMs" | "updatedAt" => ItemSort::ModifiedAt,
			"sequence" | "media.metadata.series.sequence" => ItemSort::Sequence,
			_ => ItemSort::Title,
		}
	}

	pub(crate) fn wire_value(self) -> &'static str {
		match self {
			ItemSort::Title => "media.metadata.title",
			ItemSort::AuthorName => "media.metadata.authorName",
			ItemSort::AddedAt => "addedAt",
			ItemSort::ModifiedAt => "mtimeMs",
			ItemSort::Sequence => "sequence",
		}
	}

	/// The sort key, as SQL. The title falls back to the file name the way
	/// the serialised `metadata.title` does, so the list order and the titles
	/// a client renders cannot disagree.
	fn key(self) -> SimpleExpr {
		match self {
			ItemSort::Title => {
				Expr::cust("COALESCE(NULLIF(media_metadata.title, ''), media.name)")
			},
			ItemSort::AuthorName => Expr::cust("COALESCE(media_metadata.writers, '')"),
			ItemSort::AddedAt => media::Column::CreatedAt.into_simple_expr(),
			ItemSort::ModifiedAt => media::Column::ModifiedAt.into_simple_expr(),
			ItemSort::Sequence => media_metadata::Column::Number.into_simple_expr(),
		}
	}
}

/// The `filter` query parameter: `<key>.<base64(value)>`
/// (Lissen `common/api/EncodeLibraryFilter.kt`; the official app builds the
/// same shape with `Base64.encodeToString`, `ApiHandler.kt:528-533`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ItemFilter {
	Series(String),
	/// The **author id** the profile allocated in `abs_ids`, not the name:
	/// the official app encodes the id it read off `/libraries/{id}/authors`
	/// (`ApiHandler.kt:528-533`).
	Author(String),
	Progress(ProgressFilter),
	/// `ebooks.<base64>`: the official app's "Ebooks" filter, whose two
	/// values are `ebook` and `supplementary`
	/// (`components/modals/FilterModal.vue:237-248`). Ignoring it made the
	/// "Audiobooks with an ebook" shelf list the whole library.
	Ebook(EbookFilter),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EbookFilter {
	/// `ebook`: items that have a primary ebook file.
	Primary,
	/// `supplementary`: items whose ebook file is marked supplementary.
	/// Stump's pairing has no supplementary tier — an edition is an edition
	/// — so this selects nothing rather than everything.
	Supplementary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressFilter {
	NotFinished,
	Finished,
	InProgress,
}

impl ItemFilter {
	/// `None` for an absent, malformed or unsupported filter: abs-ref
	/// ignores a filter it does not know rather than erroring, and so does
	/// this.
	pub(crate) fn parse(value: &str) -> Option<Self> {
		use base64::Engine;

		let (key, encoded) = value.split_once('.')?;
		let decoded = base64::engine::general_purpose::STANDARD
			.decode(encoded)
			.or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded))
			.ok()?;
		let decoded = String::from_utf8(decoded).ok()?;
		match key {
			"series" => Some(ItemFilter::Series(decoded)),
			"authors" => Some(ItemFilter::Author(decoded)),
			"progress" => match decoded.as_str() {
				"not-finished" => Some(ItemFilter::Progress(ProgressFilter::NotFinished)),
				"finished" => Some(ItemFilter::Progress(ProgressFilter::Finished)),
				"in-progress" => Some(ItemFilter::Progress(ProgressFilter::InProgress)),
				_ => None,
			},
			"ebooks" => match decoded.as_str() {
				"ebook" => Some(ItemFilter::Ebook(EbookFilter::Primary)),
				"supplementary" => Some(ItemFilter::Ebook(EbookFilter::Supplementary)),
				_ => None,
			},
			_ => None,
		}
	}
}

/// One page of books plus the total the envelope reports.
pub(crate) struct ItemPage {
	pub rows: Vec<media::Model>,
	pub total: i64,
}

/// Apply the ABS list parameters to the audio books of one library.
///
/// A progress filter cannot be expressed in SQL here — the reading state
/// lives behind the backend — so the ids it selects are resolved first and
/// applied as an `IN`/`NOT IN`. The set is small by construction: it only
/// holds books the user has actually started.
pub(crate) async fn item_page(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	library_id: &str,
	sort: ItemSort,
	desc: bool,
	filter: Option<&ItemFilter>,
	limit: u64,
	page: u64,
) -> AbsResult<ItemPage> {
	let mut select = audio_media(user).filter(series::Column::LibraryId.eq(library_id));

	match filter {
		Some(ItemFilter::Series(series_id)) => {
			if let Some(name) = metadata_series_name(series_id, library_id) {
				select = select.filter(media_metadata::Column::Series.eq(name));
			} else {
				select = select.filter(series::Column::Id.eq(series_id.as_str()));
			}
		},
		Some(ItemFilter::Author(author_id)) => {
			// The id was allocated for a `media_metadata.writers` value, so
			// it is resolved back to the name and matched against the CSV
			// column the same way `GET /api/authors/{id}` does. An id with
			// no name behind it selects nothing rather than everything.
			let name = backend.author_name(author_id).await?;
			let ids = match name {
				Some(name) => media_ids_crediting(backend, user, &name).await?,
				None => Vec::new(),
			};
			select = select.filter(media::Column::Id.is_in(ids));
		},
		Some(ItemFilter::Progress(state)) => {
			let progress = backend.progress_all(user).await?;
			let ids = progress
				.iter()
				.filter(|(_, progress)| match state {
					ProgressFilter::NotFinished => !progress.is_finished,
					ProgressFilter::Finished => progress.is_finished,
					ProgressFilter::InProgress => {
						!progress.is_finished && progress.position_ms > 0
					},
				})
				.map(|(media_id, _)| media_id.clone())
				.collect::<Vec<_>>();
			select = match state {
				// "not finished" includes books never started, so the
				// exclusion is by *finished* id, not by the selected set.
				ProgressFilter::NotFinished => {
					let finished = progress
						.iter()
						.filter(|(_, progress)| progress.is_finished)
						.map(|(media_id, _)| media_id.clone())
						.collect::<Vec<_>>();
					if finished.is_empty() {
						select
					} else {
						select.filter(media::Column::Id.is_not_in(finished))
					}
				},
				_ => select.filter(media::Column::Id.is_in(ids)),
			};
		},
		Some(ItemFilter::Ebook(state)) => {
			// The set is the user's confirmed audiobook↔ebook pairs, small
			// by construction, so it is resolved and applied as an `IN` the
			// same way the progress filter is.
			let ids = match state {
				EbookFilter::Primary => backend.paired_ebook_media_ids(user).await?,
				EbookFilter::Supplementary => Vec::new(),
			};
			select = select.filter(media::Column::Id.is_in(ids));
		},
		None => {},
	}

	let total = select.clone().count(backend.conn()).await? as i64;
	let order = if desc { Order::Desc } else { Order::Asc };
	let select = select
		.order_by(sort.key(), order)
		// A stable tiebreak: two books with the same title must not swap
		// places between two requests for the same page.
		.order_by_asc(media::Column::Id);
	let select = if limit > 0 {
		select.limit(limit).offset(page.saturating_mul(limit))
	} else {
		select
	};

	Ok(ItemPage {
		rows: select.all(backend.conn()).await?,
		total,
	})
}

/// An ABS series is either explicit book metadata or a genuine multi-book
/// Stump series folder. The maps are scoped to the rows indexed here.
#[derive(Default)]
pub(crate) struct SeriesIndex {
	pub series: HashMap<String, series::Model>,
	pub media_series: HashMap<String, String>,
	pub members: HashMap<String, Vec<String>>,
}

const METADATA_SERIES_ID_PREFIX: &str = "metadata-series:";

fn metadata_series_id(library_id: &str, name: &str) -> String {
	format!("{METADATA_SERIES_ID_PREFIX}{library_id}:{}", name.trim())
}

pub(crate) fn metadata_series_name<'a>(
	series_id: &'a str,
	library_id: &str,
) -> Option<&'a str> {
	let prefix = format!("{METADATA_SERIES_ID_PREFIX}{library_id}:");
	series_id.strip_prefix(&prefix)
}

fn build_series_index(
	rows: &[media::Model],
	metadata: &HashMap<String, media_metadata::Model>,
	physical_series: &HashMap<String, series::Model>,
	physical_counts: &HashMap<String, i64>,
) -> SeriesIndex {
	let mut index = SeriesIndex::default();
	for row in rows {
		let Some(physical_id) = row.series_id.as_ref() else {
			continue;
		};
		let Some(folder) = physical_series.get(physical_id) else {
			continue;
		};
		let metadata_name = metadata
			.get(&row.id)
			.and_then(|metadata| metadata.series.as_deref())
			.map(str::trim)
			.filter(|name| !name.is_empty());

		let series_id = if let Some(name) = metadata_name {
			let Some(library_id) = folder.library_id.as_deref() else {
				continue;
			};
			let id = metadata_series_id(library_id, name);
			let projected = index.series.entry(id.clone()).or_insert_with(|| {
				let mut projected = folder.clone();
				projected.id = id.clone();
				projected.name = name.to_owned();
				projected.description = None;
				projected.created_at = row.created_at.clone();
				projected.updated_at = row.updated_at.clone();
				projected
			});
			if row.created_at < projected.created_at {
				projected.created_at = row.created_at.clone();
			}
			let updated_at = row
				.updated_at
				.clone()
				.unwrap_or_else(|| row.created_at.clone());
			let previous = projected
				.updated_at
				.clone()
				.unwrap_or_else(|| projected.created_at.clone());
			if updated_at > previous {
				projected.updated_at = Some(updated_at);
			}
			id
		} else if physical_counts.get(physical_id).copied().unwrap_or(0) > 1 {
			index
				.series
				.entry(physical_id.clone())
				.or_insert_with(|| folder.clone());
			physical_id.clone()
		} else {
			continue;
		};

		index.media_series.insert(row.id.clone(), series_id.clone());
		index
			.members
			.entry(series_id)
			.or_default()
			.push(row.id.clone());
	}
	index
}

/// Build series groups from a complete visible-audio row set. Callers use
/// this for library-wide projections; paged item contexts additionally query
/// full-library Stump folder counts before invoking the shared indexer.
pub(crate) async fn series_index_for_rows(
	backend: &dyn AbsBackend,
	rows: &[media::Model],
) -> AbsResult<SeriesIndex> {
	if rows.is_empty() {
		return Ok(SeriesIndex::default());
	}
	let media_ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
	let metadata = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.is_in(media_ids))
		.all(backend.conn())
		.await?
		.into_iter()
		.filter_map(|row| row.media_id.clone().map(|id| (id, row)))
		.collect::<HashMap<_, _>>();
	let physical_ids = rows
		.iter()
		.filter_map(|row| row.series_id.clone())
		.collect::<HashSet<_>>();
	let physical_series = series::Entity::find()
		.filter(series::Column::Id.is_in(physical_ids))
		.all(backend.conn())
		.await?
		.into_iter()
		.map(|row| (row.id.clone(), row))
		.collect::<HashMap<_, _>>();
	let physical_counts = rows.iter().fold(HashMap::new(), |mut counts, row| {
		if let Some(id) = row.series_id.as_ref() {
			*counts.entry(id.clone()).or_default() += 1;
		}
		counts
	});
	Ok(build_series_index(
		rows,
		&metadata,
		&physical_series,
		&physical_counts,
	))
}

/// The side data a page of books needs, all of it batched.
pub(crate) struct ItemContext {
	pub metadata: HashMap<String, media_metadata::Model>,
	pub physical_series: HashMap<String, series::Model>,
	pub series: HashMap<String, series::Model>,
	pub media_series: HashMap<String, String>,
	pub libraries: HashMap<String, library::Model>,
	pub folder_ids: HashMap<String, String>,
	pub book_ids: HashMap<String, String>,
	pub author_ids: HashMap<String, String>,
	pub audio: HashMap<String, AbsAudio>,
	pub progress: HashMap<String, AbsProgress>,
	/// Confirmed EPUB editions of the page's audiobooks, by audiobook id.
	pub ebooks: HashMap<String, AbsEbookFile>,
}

/// Load everything a page of books needs in a fixed number of queries,
/// regardless of the page size.
pub(crate) async fn context(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	rows: &[media::Model],
	with_progress: bool,
) -> AbsResult<ItemContext> {
	let media_ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
	let series_ids = rows
		.iter()
		.filter_map(|row| row.series_id.clone())
		.collect::<HashSet<_>>()
		.into_iter()
		.collect::<Vec<_>>();

	let metadata = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.is_in(media_ids.clone()))
		.all(backend.conn())
		.await?
		.into_iter()
		.filter_map(|row| row.media_id.clone().map(|id| (id, row)))
		.collect::<HashMap<_, _>>();

	let physical_series = series::Entity::find()
		.filter(series::Column::Id.is_in(series_ids.clone()))
		.all(backend.conn())
		.await?
		.into_iter()
		.map(|row| (row.id.clone(), row))
		.collect::<HashMap<String, series::Model>>();

	// A Stump series is a folder, not automatically an ABS series. Explicit
	// `media_metadata.series` creates a metadata-backed group even with one
	// book; without that metadata only a folder containing multiple audible
	// books qualifies. Count beyond the page so pagination cannot hide a
	// genuine multi-book folder from its remaining items.
	let mut series_book_counts: HashMap<String, i64> = HashMap::new();
	if !series_ids.is_empty() {
		#[derive(FromQueryResult)]
		struct CountRow {
			series_id: String,
		}

		let rows = audio_media(user)
			.select_only()
			.column_as(series::Column::Id, "series_id")
			.filter(series::Column::Id.is_in(series_ids))
			.into_model::<CountRow>()
			.all(backend.conn())
			.await?;
		for row in rows {
			*series_book_counts.entry(row.series_id).or_default() += 1;
		}
	}
	let series_index =
		build_series_index(rows, &metadata, &physical_series, &series_book_counts);

	let library_ids = physical_series
		.values()
		.filter_map(|row| row.library_id.clone())
		.collect::<HashSet<_>>();
	let libraries = library::Entity::find()
		.filter(library::Column::Id.is_in(library_ids.iter().cloned()))
		.all(backend.conn())
		.await?
		.into_iter()
		.map(|row| (row.id.clone(), row))
		.collect::<HashMap<String, library::Model>>();

	let mut folder_ids = HashMap::with_capacity(libraries.len());
	for library_id in libraries.keys() {
		folder_ids.insert(library_id.clone(), backend.folder_id(library_id).await?);
	}

	let author_names = metadata
		.values()
		.flat_map(|row| mapper::csv(row.writers.as_deref()))
		.collect::<HashSet<_>>()
		.into_iter()
		.collect::<Vec<_>>();

	let progress = if with_progress {
		backend
			.progress_all(user)
			.await?
			.into_iter()
			.filter(|(media_id, _)| media_ids.contains(media_id))
			.collect()
	} else {
		HashMap::new()
	};

	Ok(ItemContext {
		metadata,
		physical_series,
		series: series_index.series,
		media_series: series_index.media_series,
		libraries,
		folder_ids,
		book_ids: backend.book_ids(&media_ids).await?,
		author_ids: backend.author_ids(&author_names).await?,
		audio: backend.audio_batch(&media_ids).await?,
		progress,
		ebooks: backend.ebook_editions(user, &media_ids).await?,
	})
}

impl ItemContext {
	/// The `(library id, library path)` a book lives under, resolved through
	/// its series.
	fn library_of(&self, row: &media::Model) -> (String, String) {
		row.series_id
			.as_ref()
			.and_then(|series_id| self.physical_series.get(series_id))
			.and_then(|series| series.library_id.as_ref())
			.and_then(|library_id| self.libraries.get(library_id))
			.map(|library| (library.id.clone(), library.path.clone()))
			.unwrap_or_default()
	}

	/// The ABS series represented by explicit book metadata or a genuine
	/// multi-book Stump series folder.
	pub(crate) fn series_of(&self, row: &media::Model) -> Option<(&str, &str)> {
		let series_id = self.media_series.get(&row.id)?;
		let series = self.series.get(series_id)?;
		Some((series_id.as_str(), series.name.as_str()))
	}

	/// One book, in the requested shape.
	pub(crate) fn item(
		&self,
		row: &media::Model,
		shape: ItemShape,
		user_id: &str,
		collapsed: Option<Option<CollapsedSeriesDto>>,
	) -> LibraryItemDto {
		let (library_id, library_path) = self.library_of(row);
		let series = self.series_of(row);
		let audio = self.audio.get(&row.id);
		let book_id = self
			.book_ids
			.get(&row.id)
			.map(String::as_str)
			.unwrap_or(row.id.as_str());
		let progress = self.progress.get(&row.id).map(|progress| {
			mapper::progress_dto(
				user_id,
				&row.id,
				book_id,
				audio.map_or(0, |audio| audio.duration_ms),
				progress,
			)
		});

		mapper::item_dto(
			ItemInput {
				media: row,
				metadata: self.metadata.get(&row.id),
				series,
				library_id: &library_id,
				library_path: &library_path,
				folder_id: self
					.folder_ids
					.get(&library_id)
					.map(String::as_str)
					.unwrap_or_default(),
				book_id,
				audio,
				ebook: self.ebooks.get(&row.id),
				author_ids: &self.author_ids,
				progress,
				collapsed,
			},
			shape,
		)
	}

	/// The progress DTOs for every book in the page, for `GET /api/me`.
	pub(crate) fn progress_dtos(
		&self,
		user_id: &str,
		rows: &[media::Model],
	) -> Vec<MediaProgressDto> {
		rows.iter()
			.filter_map(|row| {
				let progress = self.progress.get(&row.id)?;
				let book_id = self
					.book_ids
					.get(&row.id)
					.map(String::as_str)
					.unwrap_or(row.id.as_str());
				Some(mapper::progress_dto(
					user_id,
					&row.id,
					book_id,
					self.audio.get(&row.id).map_or(0, |audio| audio.duration_ms),
					progress,
				))
			})
			.collect()
	}
}

/// One audible book the user may see, by id, or `404`.
pub(crate) async fn media_for_user(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	media_id: &str,
) -> AbsResult<media::Model> {
	audio_media(user)
		.filter(media::Column::Id.eq(media_id))
		.one(backend.conn())
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No library item {media_id}")))
}

/// One audible, undeleted book by id, with no user to scope it — the cover
/// lane's resolution when the request carries no credential.
///
/// It is deliberately *not* `find_for_user`: there is no user. What it does
/// keep is the profile's own funnel, so an anonymous cover request cannot
/// name a comic, a deleted row or anything that is not an ABS library item.
pub(crate) async fn audio_media_row(
	backend: &dyn AbsBackend,
	media_id: &str,
) -> AbsResult<media::Model> {
	media::Entity::find()
		.filter(audio_condition())
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::Id.eq(media_id))
		.one(backend.conn())
		.await?
		.ok_or_else(|| AbsError::NotFound(format!("No library item {media_id}")))
}

/// The ids of the audible books that credit `name` as a writer.
///
/// `media_metadata.writers` is a comma-separated column, so the SQL
/// `contains` is only a prefilter: a name that is a substring of another
/// author's would drag their books in, and the CSV values decide. Shared by
/// `GET /api/authors/{id}` and by the `authors.<base64>` item filter, so
/// browsing an author two ways cannot answer two different sets.
pub(crate) async fn media_ids_crediting(
	backend: &dyn AbsBackend,
	user: &AuthUser,
	name: &str,
) -> AbsResult<Vec<String>> {
	let candidates = media_metadata::Entity::find()
		.filter(media_metadata::Column::Writers.contains(name))
		.all(backend.conn())
		.await?;
	let ids = candidates
		.into_iter()
		.filter(|row| {
			mapper::csv(row.writers.as_deref())
				.iter()
				.any(|w| w == name)
		})
		.filter_map(|row| row.media_id)
		.collect::<Vec<_>>();
	if ids.is_empty() {
		return Ok(Vec::new());
	}
	Ok(audio_media(user)
		.filter(media::Column::Id.is_in(ids))
		.select_only()
		.column(media::Column::Id)
		.into_tuple::<String>()
		.all(backend.conn())
		.await?)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sort_values_are_the_ones_lissen_sends() {
		assert_eq!(ItemSort::parse("media.metadata.title"), ItemSort::Title);
		assert_eq!(
			ItemSort::parse("media.metadata.authorName"),
			ItemSort::AuthorName
		);
		assert_eq!(ItemSort::parse("addedAt"), ItemSort::AddedAt);
		assert_eq!(ItemSort::parse("mtimeMs"), ItemSort::ModifiedAt);
		assert_eq!(ItemSort::parse("sequence"), ItemSort::Sequence);
		// abs-ref ignores a sort it does not know rather than erroring.
		assert_eq!(ItemSort::parse("media.metadata.nope"), ItemSort::Title);
		assert_eq!(ItemSort::parse(""), ItemSort::Title);
	}

	#[test]
	fn filter_parses_the_base64_encoding_lissen_builds() {
		// The literal Lissen ships for "hide completed":
		// library/converter/LibraryFilteringRequestConverter.kt:14.
		assert_eq!(
			ItemFilter::parse("progress.bm90LWZpbmlzaGVk"),
			Some(ItemFilter::Progress(ProgressFilter::NotFinished))
		);
		assert_eq!(
			ItemFilter::parse("series.YWJj"),
			Some(ItemFilter::Series("abc".to_owned()))
		);
		// The official app browses an author by encoding the id it read off
		// `/libraries/{id}/authors` the same way.
		assert_eq!(
			ItemFilter::parse("authors.YWJj"),
			Some(ItemFilter::Author("abc".to_owned()))
		);
		// An unknown key, an unknown progress state and malformed input all
		// mean "no filter", never an error.
		assert_eq!(ItemFilter::parse("narrators.YWJj"), None);
		assert_eq!(ItemFilter::parse("progress.bm9wZQ=="), None);
		assert_eq!(ItemFilter::parse("progress.!!!"), None);
		assert_eq!(ItemFilter::parse("nodot"), None);
	}
}
