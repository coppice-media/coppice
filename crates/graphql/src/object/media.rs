use async_graphql::{
	dataloader::DataLoader, ComplexObject, Context, Result, SimpleObject, ID,
};

use models::{
	domain::{
		edition_pair::{self, PairStatus},
		reading_state::{map_locator_to_time, map_time_to_locator, ChapterMapping},
	},
	entity::{library, media, media_analysis, media_audio, reading_head, series, tag},
	services::audio,
	shared::{analysis::MediaAnalysisData, image::ImageRef},
};
use num_traits::cast::ToPrimitive;
use sea_orm::{prelude::*, sea_query::Query, FromQueryResult, QuerySelect};
use stump_library::editions;

use crate::{
	data::CoreContext,
	loader::{
		favorite::{FavoriteMediaLoaderKey, FavoritesLoader},
		library_config::{LibraryConfigLoader, LibraryConfigLoaderKey},
		media_analysis::{MediaAnalysisLoader, PageDimensionLoaderKey},
		reading_session::{
			ReadingSessionLoader, ReadthroughRecordLoaderKey,
			ResumeReadingCursorLoaderKey,
		},
		series::SeriesLoader,
	},
	object::epub::Epub,
	pagination::{CursorPagination, CursorPaginationInfo, PaginatedResponse, Pagination},
	utils::db_statement,
};

use super::{
	audio::MediaAudio,
	edition_pair::{EditionSuggestion, MappedPosition},
	library::Library,
	library_config::LibraryConfig,
	media_metadata::MediaMetadata,
	readthrough_record::ReadthroughRecord,
	resume_reading_cursor::ResumeReadingCursor,
	series::Series,
	tag::Tag,
};

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct Media {
	#[graphql(flatten)]
	pub model: media::Model,
	pub metadata: Option<MediaMetadata>,
}

impl From<media::ModelWithMetadata> for Media {
	fn from(entity: media::ModelWithMetadata) -> Self {
		Self {
			model: entity.media,
			metadata: entity.metadata.map(MediaMetadata::from),
		}
	}
}

impl Media {
	pub fn self_cursor_params(&self) -> CursorPagination {
		CursorPagination {
			after: Some(self.model.name.clone()),
			limit: 1,
		}
	}
}

#[ComplexObject]
impl Media {
	/// If the media is an epub, this will return the parsed epub data from the file
	async fn ebook(&self) -> Result<Option<Epub>> {
		if self.model.extension.to_lowercase() != "epub" {
			return Ok(None);
		}

		let model = media::MediaIdentSelect {
			id: self.model.id.clone(),
			path: self.model.path.clone(),
		};

		Epub::try_from(model).map(Some)
	}

	/// Whether the media is marked as a favorite by the current user
	async fn is_favorite(&self, ctx: &Context<'_>) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let loader = ctx.data::<DataLoader<FavoritesLoader>>()?;

		let is_favorite = loader
			.load_one(FavoriteMediaLoaderKey {
				user_id: user.id.clone(),
				media_id: self.model.id.clone(),
			})
			.await?;

		Ok(is_favorite.unwrap_or(false))
	}

	/// The tags associated with the media
	async fn tags(&self, ctx: &Context<'_>) -> Result<Vec<Tag>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();
		let model = tag::Entity::find_for_media_id(&self.model.id.clone())
			.all(conn)
			.await?;
		Ok(model.into_iter().map(Tag::from).collect())
	}

	/// The series the media belongs to
	async fn series(&self, ctx: &Context<'_>) -> Result<Series> {
		let loader = ctx.data::<DataLoader<SeriesLoader>>()?;

		let series_id = self.model.series_id.clone().ok_or("Series ID not set")?;

		let series = loader
			.load_one(series_id)
			.await?
			.ok_or("Series not found")?;

		Ok(series)
	}

	async fn library_id(&self, ctx: &Context<'_>) -> Result<String> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let series_id = self.model.series_id.clone().ok_or("Series ID not set")?;
		let id: String = library::Entity::find()
			.select_only()
			.column(library::Column::Id)
			.filter(
				library::Column::Id.in_subquery(
					Query::select()
						.column(series::Column::LibraryId)
						.from(series::Entity)
						.and_where(series::Column::Id.eq(series_id))
						.to_owned(),
				),
			)
			.into_tuple()
			.one(conn)
			.await?
			.ok_or("Library not found")?;

		Ok(id)
	}

	async fn library(&self, ctx: &Context<'_>) -> Result<Library> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let series_id = self.model.series_id.clone().ok_or("Series ID not set")?;
		let model = library::Entity::find()
			.filter(
				library::Column::Id.in_subquery(
					Query::select()
						.column(series::Column::LibraryId)
						.from(series::Entity)
						.and_where(series::Column::Id.eq(series_id))
						.to_owned(),
				),
			)
			.one(conn)
			.await?
			.ok_or("Library not found")?;

		Ok(Library::from(model))
	}

	async fn library_config(&self, ctx: &Context<'_>) -> Result<LibraryConfig> {
		let loader = ctx.data::<DataLoader<LibraryConfigLoader>>()?;
		let series_id = self.model.series_id.clone().ok_or("Series ID not set")?;

		loader
			.load_one(LibraryConfigLoaderKey { series_id })
			.await?
			.map(LibraryConfig::from)
			.ok_or_else(|| "Library config not found".into())
	}

	async fn analysis_data(
		&self,
		ctx: &Context<'_>,
	) -> Result<Option<MediaAnalysisData>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let model = media_analysis::Entity::find()
			.filter(media_analysis::Column::MediaId.eq(self.model.id.clone()))
			.one(conn)
			.await?;

		Ok(model.map(|m| m.data))
	}

	/// The audio shape of the book, or `null` when it is not an audiobook.
	///
	/// `null` and "an audiobook with no tracks" are different states, and only
	/// one of them is a real publication, so this is `Option` rather than an
	/// empty [`MediaAudio`].
	async fn audio(&self, ctx: &Context<'_>) -> Result<Option<MediaAudio>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		Ok(audio::book(conn, &self.model.id)
			.await?
			.map(MediaAudio::from))
	}

	/// Other editions of the same work: the audiobook of this ebook, or the
	/// ebook of this audiobook.
	///
	/// Confirmed pairs only, and a plain read — a confirmed pair is two link
	/// rows, so this costs one indexed query and is safe to select on a grid.
	/// Unconfirmed guesses are [`Media::edition_suggestions`], which is what
	/// actually runs the heuristics.
	async fn editions(&self, ctx: &Context<'_>) -> Result<Vec<Media>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let links = edition_pair::linked_media(
			conn,
			&user.id,
			&self.model.id,
			Some(PairStatus::Confirmed),
		)
		.await?;
		editions_by_id(conn, user, links.iter().map(|link| link.media_id.clone())).await
	}

	/// Unconfirmed pairings for this book, recomputed on demand.
	///
	/// This is the field a book page selects: it runs the three pairing rules
	/// (shared work, identifier through a provider's edition list, normalised
	/// title and author) and caches every verdict as a link row, so a
	/// rejection sticks and a second load is a read. Provider lookups are
	/// best-effort — an upstream that is down costs the *evidence* of a
	/// suggestion, never the page.
	async fn edition_suggestions(
		&self,
		ctx: &Context<'_>,
	) -> Result<Vec<EditionSuggestion>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let lookups = editions::enabled_edition_lookups(conn).await?;
		let links = editions::pair_editions(conn, user, &self.model.id, &lookups)
			.await?
			.into_iter()
			.filter(|link| link.status == PairStatus::Suggested)
			.collect::<Vec<_>>();
		let media =
			editions_by_id(conn, user, links.iter().map(|link| link.media_id.clone()))
				.await?;

		Ok(links
			.into_iter()
			.filter_map(|link| {
				media
					.iter()
					.find(|candidate| candidate.model.id == link.media_id)
					.map(|candidate| EditionSuggestion {
						media: candidate.clone(),
						work_id: async_graphql::ID::from(link.work_id),
						status: link.status,
						evidence: link.evidence,
					})
			})
			.collect())
	}

	/// This book's position, derived from the reader's real position in
	/// `fromMediaId` — the other edition of the same work.
	///
	/// `null` when the two are not a confirmed pair, when there is no reading
	/// head to convert, or when the position falls in front or back matter
	/// that has no counterpart: guessing there would land a listener on the
	/// copyright page. The result is always
	/// [`MappedPosition::approximate`] and is never written to a head.
	async fn paired_position(
		&self,
		ctx: &Context<'_>,
		from_media_id: ID,
	) -> Result<Option<MappedPosition>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		mapped_position(conn, user, &self.model, from_media_id.as_str()).await
	}

	/// A reference to the thumbnail image for the media. This will be a fully
	/// qualified URL to the image.
	async fn thumbnail(&self, ctx: &Context<'_>) -> Result<ImageRef> {
		let service = ctx.data::<stump_api_types::RequestOrigin>()?;
		let loader = ctx.data::<DataLoader<MediaAnalysisLoader>>()?;

		let dimensions = match self
			.model
			.thumbnail_meta
			.as_ref()
			.and_then(|meta| meta.dimensions.as_ref())
		{
			Some(dim) => Some((dim.width, dim.height)),
			None => {
				let page_dimension = loader
					.load_one(PageDimensionLoaderKey {
						media_id: self.model.id.clone(),
					})
					.await?;
				page_dimension.map(|dim| (dim.width, dim.height))
			},
		};

		Ok(ImageRef {
			url: service.format_url(format!("/api/v2/media/{}/thumbnail", self.model.id)),
			height: dimensions.as_ref().map(|dim| dim.1),
			width: dimensions.as_ref().map(|dim| dim.0),
			metadata: self.model.thumbnail_meta.clone(),
			..Default::default()
		})
	}

	/// The resolved name of the media, which will prioritize the title pulled from
	/// metatadata, if available, and fallback to the name derived from the file name
	async fn resolved_name(&self) -> String {
		self.metadata
			.as_ref()
			.and_then(|meta| meta.model.title.as_ref())
			.unwrap_or(&self.model.name)
			.to_string()
	}

	async fn read_progress(
		&self,
		ctx: &Context<'_>,
	) -> Result<Option<ResumeReadingCursor>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let loader = ctx.data::<DataLoader<ReadingSessionLoader>>()?;

		let progress = loader
			.load_one(ResumeReadingCursorLoaderKey {
				user_id: user.id.clone(),
				media_id: self.model.id.clone(),
			})
			.await?;

		Ok(progress)
	}

	// TODO(graphql): Create object to query for device used (e.g., KoReader device ID)
	async fn read_history(&self, ctx: &Context<'_>) -> Result<Vec<ReadthroughRecord>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let loader = ctx.data::<DataLoader<ReadingSessionLoader>>()?;

		let history = loader
			.load_one(ReadthroughRecordLoaderKey {
				user_id: user.id.clone(),
				media_id: self.model.id.clone(),
			})
			.await?
			.unwrap_or_default();

		Ok(history)
	}

	async fn series_position(&self, ctx: &Context<'_>) -> Result<Option<i64>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		if let Some(position) = self.metadata.as_ref().and_then(|m| m.model.number) {
			if position.fract().is_zero() {
				return Ok(Some(position.to_i64().unwrap_or(0)));
			}
		}

		#[derive(Debug, FromQueryResult)]
		struct PositionResult {
			position: i64,
		}

		let series_id = self.model.series_id.clone().ok_or("Series ID not set")?;

		let position = PositionResult::find_by_statement(db_statement(
			conn,
			r#"
            SELECT position
            FROM (
                SELECT
                    id,
                    ROW_NUMBER() OVER (
                        PARTITION BY series_id
                        ORDER BY name
                    ) as position
                FROM media
                WHERE series_id = $1
                AND deleted_at IS NULL
            ) ranked
            WHERE id = $2
            "#,
			[series_id.into(), self.model.id.clone().into()],
		))
		.one(conn)
		.await?
		.map(|result| result.position);

		Ok(position)
	}

	/// The next media in the series, ordered by name
	async fn next_in_series(
		&self,
		ctx: &Context<'_>,
		#[graphql(default)] pagination: Pagination,
	) -> Result<PaginatedResponse<Media>> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let pagination = match pagination {
			Pagination::Cursor(pagination) => pagination,
			_ => {
				return Err(
					"Only cursor pagination is supported for this operation".into()
				)
			},
		};

		let mut cursor = media::ModelWithMetadata::find_for_user(user)
			.filter(media::Column::SeriesId.eq(self.model.series_id.clone()))
			.cursor_by(media::Column::Name);

		let after = match pagination.after.clone() {
			Some(after) if after != self.model.id => {
				let media =
					media::Entity::find_for_user(user)
						.select_only()
						.column(media::Column::Name)
						.filter(media::Column::Id.eq(after).and(
							media::Column::SeriesId.eq(self.model.series_id.clone()),
						))
						.into_model::<media::MediaNameCmpSelect>()
						.one(conn)
						.await?
						.ok_or("Cursor not found")?;
				media.name
			},
			_ => self.model.name.clone(),
		};

		cursor.after(after).first(pagination.limit);

		let next = cursor
			.into_model::<media::ModelWithMetadata>()
			.all(conn)
			.await?;
		let current_cursor = pagination
			.after
			.or_else(|| next.first().map(|m| m.media.id.clone()));
		let next_cursor = match next.last().map(|m| m.media.id.clone()) {
			Some(id) if next.len() == pagination.limit as usize => Some(id),
			_ => None,
		};

		Ok(PaginatedResponse {
			nodes: next.into_iter().map(Media::from).collect(),
			page_info: CursorPaginationInfo {
				current_cursor,
				next_cursor,
				limit: pagination.limit,
			}
			.into(),
		})
	}

	/// The path to the media file **relative** to the library path. This is only useful for
	/// displaying a truncated path when in the context of a library, e.g. limited space
	/// on a mobile device.
	async fn relative_library_path(&self, ctx: &Context<'_>) -> Result<String> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let (library_path,) = library::Entity::find()
			.select_only()
			.column(library::Column::Path)
			.filter(
				library::Column::Id.in_subquery(
					Query::select()
						.column(series::Column::LibraryId)
						.from(series::Entity)
						.and_where(series::Column::Id.eq(self.model.series_id.clone()))
						.to_owned(),
				),
			)
			.into_tuple::<(String,)>()
			.one(conn)
			.await?
			.ok_or("Library not found")?;

		Ok(self.model.path.replace(&library_path, ""))
	}
}

/// Load paired media in one query, keeping the caller's order and dropping
/// anything the request may not see. A link is per user, but visibility is
/// per request: a device-scoped credential must not learn about a book
/// outside its libraries just because the account paired it.
async fn editions_by_id(
	conn: &DatabaseConnection,
	user: &models::entity::user::AuthUser,
	ids: impl Iterator<Item = String>,
) -> Result<Vec<Media>> {
	let ids: Vec<String> = ids.collect();
	if ids.is_empty() {
		return Ok(Vec::new());
	}

	let models = media::ModelWithMetadata::find_for_user(user)
		.filter(media::Column::Id.is_in(ids.clone()))
		.filter(media::Column::DeletedAt.is_null())
		.into_model::<media::ModelWithMetadata>()
		.all(conn)
		.await?;

	Ok(ids
		.iter()
		.filter_map(|id| {
			models
				.iter()
				.find(|model| &model.media.id == id)
				.map(|model| {
					Media::from(media::ModelWithMetadata {
						media: model.media.clone(),
						metadata: model.metadata.clone(),
					})
				})
		})
		.collect())
}

/// Convert the reader's head on `from_media_id` into `target`'s coordinates.
///
/// The direction is decided by which side is the audio edition, not by what
/// the client asked for: the ebook is always the canonical text and the
/// audiobook the canonical time, so there are exactly two conversions and
/// each needs the *other* edition's chapter geometry.
async fn mapped_position(
	conn: &DatabaseConnection,
	user: &models::entity::user::AuthUser,
	target: &media::Model,
	from_media_id: &str,
) -> Result<Option<MappedPosition>> {
	if from_media_id == target.id {
		return Ok(None);
	}

	// Only a confirmed pair converts. A suggestion is a guess about identity;
	// converting through it would show a position from a different book.
	let paired = edition_pair::linked_media(
		conn,
		&user.id,
		&target.id,
		Some(PairStatus::Confirmed),
	)
	.await?
	.into_iter()
	.any(|link| link.media_id == from_media_id);
	if !paired {
		return Ok(None);
	}

	let Some(head) = reading_head::Entity::find()
		.filter(reading_head::Column::UserId.eq(user.id.clone()))
		.filter(reading_head::Column::MediaId.eq(from_media_id))
		.one(conn)
		.await?
	else {
		return Ok(None);
	};

	let source = media::Entity::find_by_id(from_media_id.to_owned())
		.one(conn)
		.await?
		.ok_or("Paired media not found")?;
	let source_is_audio = media_audio::Entity::find()
		.filter(media_audio::Column::MediaId.eq(from_media_id))
		.one(conn)
		.await?
		.is_some();

	let (ebook, audio) = if source_is_audio {
		(target, &source)
	} else {
		(&source, target)
	};

	let spine = editions::ebook_spine(&ebook.path)?;
	let chapters = editions::audio_chapter_spans(conn, &audio.id).await?;
	let mappings: Vec<ChapterMapping> = editions::chapter_map(conn, &ebook.id, &audio.id)
		.await?
		.into_iter()
		.map(|entry| ChapterMapping {
			ebook_spine_index: entry.ebook_spine_index,
			audio_chapter_index: entry.audio_chapter_index,
			confidence: entry.confidence,
		})
		.collect();
	if mappings.is_empty() {
		return Ok(None);
	}

	let mapped = if source_is_audio {
		let Some(position_ms) = head.position_ms else {
			return Ok(None);
		};
		map_time_to_locator(position_ms, &chapters, &spine, &mappings)
	} else {
		let Some(locator) = head.locator.as_ref() else {
			return Ok(None);
		};
		map_locator_to_time(locator, &spine, &chapters, &mappings)
	};

	Ok(mapped.map(|mapped| MappedPosition::new(from_media_id, mapped)))
}
