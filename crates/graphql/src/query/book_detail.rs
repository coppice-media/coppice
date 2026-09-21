//! Book-detail, local search, reading history, and provider-candidate queries.
//!
//! Only the book-detail editor may invoke provider search. The page itself and
//! the local search/similarity helpers never leave the process or read provider
//! credentials.

use std::collections::{HashMap, HashSet};

use async_graphql::{Context, Object, Result, ID};
use metadata_integrations::SearchQuery;
use models::{
	domain::edition_pair::PairStatus,
	entity::{liseur_sync_media_link, media, media_metadata, metadata_fetch_record},
	shared::enums::UserPermission,
};
use sea_orm::{prelude::*, Condition, QueryOrder, QuerySelect};
use stump_auth::AuthContext;
use stump_core::filesystem::metadata::ProviderClientCache;

use crate::{
	data::CoreContext as GraphqlCoreContext,
	guard::PermissionGuard,
	input::media::MediaMetadataSearchInput,
	object::{
		book_detail::{
			load_book_detail, load_book_reading_log, metadata_authors, metadata_title,
			BookDetail, BookEditionKind, BookReadingLog, BookSearchResult,
		},
		metadata_fetch_record::MetadataFetchRecord,
	},
};

#[derive(Default)]
pub struct BookDetailQuery;

#[Object]
impl BookDetailQuery {
	/// Resolve a visible media id into one merged work view. Work identity is
	/// internal: the route and this query remain anchored by media id.
	async fn book_detail(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
	) -> Result<Option<BookDetail>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<GraphqlCoreContext>()?;
		load_book_detail(core, &auth.user, media_id.as_ref()).await
	}

	/// Raw reading heads and sessions grouped by the detail's edition rows.
	async fn book_reading_log(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
	) -> Result<Option<BookReadingLog>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<GraphqlCoreContext>()?;
		load_book_reading_log(core, &auth.user, media_id.as_ref()).await
	}

	/// Search only the current user's visible library. This intentionally does
	/// not invoke a metadata provider; provider search is an editor operation.
	async fn search_books(
		&self,
		ctx: &Context<'_>,
		query: String,
		#[graphql(default = 20)] limit: i32,
	) -> Result<Vec<BookSearchResult>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<GraphqlCoreContext>()?;
		let query = query.trim();
		if query.is_empty() {
			return Ok(Vec::new());
		}
		let limit = limit.clamp(1, 50) as u64;
		let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
		let models = media::ModelWithMetadata::find_for_user(&auth.user)
			.filter(media::Column::DeletedAt.is_null())
			.filter(
				Condition::any()
					.add(media::Column::Name.like(&pattern))
					.add(media_metadata::Column::Title.like(&pattern))
					.add(media_metadata::Column::Writers.like(&pattern))
					.add(media_metadata::Column::Summary.like(&pattern))
					.add(media_metadata::Column::Genres.like(&pattern)),
			)
			.order_by_asc(media::Column::Name)
			.order_by_asc(media::Column::Id)
			.limit(limit)
			.into_model::<media::ModelWithMetadata>()
			.all(core.conn.as_ref())
			.await?;
		let links = liseur_sync_media_link::Entity::find()
			.filter(liseur_sync_media_link::Column::UserId.eq(auth.user.id.clone()))
			.all(core.conn.as_ref())
			.await?;
		Ok(dedupe_results(models, links))
	}

	/// Rank similar books using only visible local metadata. Confirmed editions
	/// of the current work are excluded, as are duplicate editions of another
	/// work already represented by a higher-ranked row.
	async fn similar_books(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		#[graphql(default = 20)] limit: i32,
	) -> Result<Vec<BookSearchResult>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<GraphqlCoreContext>()?;
		let limit = limit.clamp(1, 50) as usize;
		let Some(anchor) = media::ModelWithMetadata::find_for_user(&auth.user)
			.filter(media::Column::Id.eq(media_id.to_string()))
			.filter(media::Column::DeletedAt.is_null())
			.into_model::<media::ModelWithMetadata>()
			.one(core.conn.as_ref())
			.await?
		else {
			return Ok(Vec::new());
		};
		let links = liseur_sync_media_link::Entity::find()
			.filter(liseur_sync_media_link::Column::UserId.eq(auth.user.id.clone()))
			.all(core.conn.as_ref())
			.await?;
		let current_work = links
			.iter()
			.find(|link| link.media_id == media_id.to_string())
			.filter(|link| {
				PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
			})
			.map(|link| link.work_id.clone());
		let excluded_ids: HashSet<String> = current_work
			.as_ref()
			.map(|work_id| {
				links
					.iter()
					.filter(|link| {
						link.work_id == *work_id
							&& PairStatus::from_stored(&link.pair_status)
								== PairStatus::Confirmed
					})
					.map(|link| link.media_id.clone())
					.collect()
			})
			.unwrap_or_default();
		let models = media::ModelWithMetadata::find_for_user(&auth.user)
			.filter(media::Column::DeletedAt.is_null())
			.limit(500)
			.into_model::<media::ModelWithMetadata>()
			.all(core.conn.as_ref())
			.await?;
		let anchor_title = metadata_title(anchor.metadata.as_ref(), &anchor.media.name);
		let anchor_authors = metadata_authors(anchor.metadata.as_ref());
		let anchor_genres =
			split_list(anchor.metadata.as_ref().and_then(|m| m.genres.as_deref()));
		let anchor_series = anchor.metadata.as_ref().and_then(|m| m.series.as_deref());
		let mut seen_works = HashSet::new();
		let mut ranked = Vec::new();
		for model in models {
			if model.media.id == media_id.to_string()
				|| excluded_ids.contains(&model.media.id)
			{
				continue;
			}
			let work_id = links
				.iter()
				.find(|link| link.media_id == model.media.id)
				.filter(|link| {
					PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
				})
				.map(|link| link.work_id.clone());
			if let Some(work_id) = &work_id {
				if !seen_works.insert(work_id.clone()) {
					continue;
				}
			}
			let title = metadata_title(model.metadata.as_ref(), &model.media.name);
			let authors = metadata_authors(model.metadata.as_ref());
			let genres =
				split_list(model.metadata.as_ref().and_then(|m| m.genres.as_deref()));
			let score = similarity_score(
				&anchor_title,
				&anchor_authors,
				&anchor_genres,
				anchor_series,
				&title,
				&authors,
				&genres,
				model.metadata.as_ref().and_then(|m| m.series.as_deref()),
			);
			if score <= 0.0 {
				continue;
			}
			let kind = BookEditionKind::from_model(&model.media, false);
			ranked.push(BookSearchResult {
				media_id: ID::from(model.media.id),
				work_id: work_id.map(ID::from),
				title,
				authors,
				kind,
				score,
			});
		}
		ranked.sort_by(|left, right| {
			right
				.score
				.total_cmp(&left.score)
				.then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
				.then_with(|| left.media_id.cmp(&right.media_id))
		});
		ranked.truncate(limit);
		Ok(ranked)
	}

	/// Return the persisted provider candidates without making another request.
	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataFetchRecordRead)")]
	async fn book_metadata_candidates(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
	) -> Result<Option<MetadataFetchRecord>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<GraphqlCoreContext>()?;
		let visible = media::ModelWithMetadata::find_for_user(&auth.user)
			.filter(media::Column::Id.eq(media_id.to_string()))
			.into_model::<media::ModelWithMetadata>()
			.one(core.conn.as_ref())
			.await?;
		if visible.is_none() {
			return Ok(None);
		}
		Ok(metadata_fetch_record::Entity::find()
			.filter(metadata_fetch_record::Column::MediaId.eq(media_id.to_string()))
			.order_by_desc(metadata_fetch_record::Column::AddedAt)
			.one(core.conn.as_ref())
			.await?
			.map(MetadataFetchRecord::from))
	}

	/// Search external metadata providers for the full metadata editor. The
	/// result is persisted and returned for review; no fields are auto-applied.
	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataFetchRecordManage)")]
	async fn search_book_metadata(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		search: Option<MediaMetadataSearchInput>,
	) -> Result<MetadataFetchRecord> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<GraphqlCoreContext>()?;
		let model = media::ModelWithMetadata::find_for_user(&auth.user)
			.filter(media::Column::Id.eq(media_id.to_string()))
			.into_model::<media::ModelWithMetadata>()
			.one(core.conn.as_ref())
			.await?
			.ok_or("Media not found")?;
		let encryption_key = core.get_encryption_key().await?;
		let provider_cache = ProviderClientCache::new(encryption_key);
		let title = search
			.as_ref()
			.and_then(|value| value.title.clone())
			.or_else(|| {
				model
					.metadata
					.as_ref()
					.and_then(|value| value.title.clone())
			})
			.unwrap_or_else(|| model.media.name.clone());
		let author = search
			.as_ref()
			.and_then(|value| value.author.clone())
			.or_else(|| {
				model.metadata.as_ref().and_then(|value| {
					value
						.writers
						.as_deref()
						.and_then(|writers| {
							writers.split(',').map(str::trim).find(|v| !v.is_empty())
						})
						.map(str::to_owned)
				})
			});
		let isbn = search
			.as_ref()
			.and_then(|value| value.isbn.clone())
			.or_else(|| {
				model
					.metadata
					.as_ref()
					.and_then(|value| value.identifier_isbn.clone())
			});
		let number = search
			.as_ref()
			.and_then(|value| value.number.map(|value| value as f32));
		let year = search.as_ref().and_then(|value| value.year);
		let mut provider_hints = HashMap::new();
		if let Some(value) = search
			.as_ref()
			.and_then(|value| value.comic_vine_volume_id.clone())
		{
			provider_hints.insert("comic_vine_volume_id".to_owned(), value);
		}
		let provider_filter = search.as_ref().and_then(|value| value.provider.clone());
		let limit = search
			.as_ref()
			.and_then(|value| value.limit)
			.map(|value| value.max(0) as u32);
		stump_core::filesystem::metadata::fetch_media_metadata(
			core.conn.as_ref(),
			&model.media.id,
			SearchQuery {
				title,
				author,
				isbn,
				year,
				number,
				limit: limit.or(Some(10)),
				provider_hints,
			},
			&provider_cache,
			provider_filter,
			true,
		)
		.await?;
		Ok(metadata_fetch_record::Entity::find()
			.filter(metadata_fetch_record::Column::MediaId.eq(model.media.id))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Failed to load fetch record after search")?
			.into())
	}
}

fn dedupe_results(
	models: Vec<media::ModelWithMetadata>,
	links: Vec<liseur_sync_media_link::Model>,
) -> Vec<BookSearchResult> {
	let mut seen_work = HashSet::new();
	models
		.into_iter()
		.filter_map(|model| {
			let title = metadata_title(model.metadata.as_ref(), &model.media.name);
			let authors = metadata_authors(model.metadata.as_ref());
			let link = links
				.iter()
				.find(|link| link.media_id == model.media.id)
				.filter(|link| {
					PairStatus::from_stored(&link.pair_status) == PairStatus::Confirmed
				});
			let kind = BookEditionKind::from_model(&model.media, false);
			if let Some(link) = link {
				if !seen_work.insert(link.work_id.clone()) {
					return None;
				}
				Some(BookSearchResult {
					media_id: ID::from(model.media.id),
					work_id: Some(ID::from(link.work_id.clone())),
					title,
					authors,
					kind,
					score: 1.0,
				})
			} else {
				Some(BookSearchResult {
					media_id: ID::from(model.media.id),
					work_id: None,
					title,
					authors,
					kind,
					score: 1.0,
				})
			}
		})
		.collect()
}

fn split_list(value: Option<&str>) -> Vec<String> {
	value
		.map(|value| {
			value
				.split(',')
				.map(|part| part.trim().to_ascii_lowercase())
				.filter(|part| !part.is_empty())
				.collect()
		})
		.unwrap_or_default()
}

fn overlap(left: &[String], right: &[String]) -> f64 {
	left.iter().filter(|value| right.contains(value)).count() as f64
}

fn title_tokens(value: &str) -> Vec<String> {
	value
		.split(|ch: char| !ch.is_alphanumeric())
		.map(str::trim)
		.filter(|part| !part.is_empty())
		.map(str::to_ascii_lowercase)
		.collect()
}
fn similarity_score(
	anchor_title: &str,
	anchor_authors: &[String],
	anchor_genres: &[String],
	anchor_series: Option<&str>,
	title: &str,
	authors: &[String],
	genres: &[String],
	series: Option<&str>,
) -> f64 {
	let title_words = title_tokens(anchor_title);
	let candidate_title_words = title_tokens(title);
	let mut score = overlap(&title_words, &candidate_title_words) * 5.0;
	score += overlap(anchor_authors, authors) * 4.0;
	score += overlap(anchor_genres, genres) * 2.0;
	if anchor_series
		.zip(series)
		.is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
	{
		score += 3.0;
	}
	score
}
