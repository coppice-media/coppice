//! Shared loaders that turn user-visible Stump rows into mapper inputs.

use std::collections::HashMap;

use models::entity::{library, media, series, user::AuthUser};
use sea_orm::{prelude::*, QueryOrder};

use crate::{
	errors::{APIError, APIResult},
	ids::{IdKind, KavitaIds},
	mapper::{sort_media, MediaInput, SeriesInput},
	progress::latest_sessions,
};

use super::KavitaBackend;

/// Load one user-visible series by its Stump id.
pub(crate) async fn find_series(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	stump_id: &str,
) -> APIResult<Option<series::ModelWithMetadata>> {
	Ok(series::ModelWithMetadata::find_for_user(user)
		.filter(series::Column::Id.eq(stump_id))
		.filter(series::Column::DeletedAt.is_null())
		.into_model::<series::ModelWithMetadata>()
		.one(ctx.conn())
		.await?)
}

/// Load one user-visible series by its Kavita id.
pub(crate) async fn find_series_by_kavita_id(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	id: i32,
) -> APIResult<Option<series::ModelWithMetadata>> {
	let Some(stump_id) = KavitaIds::lookup(ctx.conn(), IdKind::Series, id).await? else {
		return Ok(None);
	};
	find_series(ctx, user, &stump_id).await
}

/// Build mapper inputs for a batch of series with a bounded number of
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
	let series_kavita_ids = KavitaIds::resolve_many(conn, IdKind::Series, &series_ids).await?;

	let mut library_ids = rows
		.iter()
		.filter_map(|row| row.series.library_id.clone())
		.collect::<Vec<_>>();
	library_ids.sort();
	library_ids.dedup();
	let libraries = library::Entity::find()
		.filter(library::Column::Id.is_in(library_ids.clone()))
		.all(conn)
		.await?
		.into_iter()
		.map(|library| (library.id.clone(), library.name))
		.collect::<HashMap<_, _>>();
	let library_kavita_ids = KavitaIds::resolve_many(conn, IdKind::Library, &library_ids).await?;

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
	let media_kavita_ids = KavitaIds::resolve_many(conn, IdKind::Media, &media_ids).await?;
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
				library_name: libraries.get(library_id).cloned().unwrap_or_default(),
				series: row.series,
				metadata: row.metadata,
				media,
			})
		})
		.collect()
}

pub(crate) async fn load_series_input(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	row: series::ModelWithMetadata,
) -> APIResult<SeriesInput> {
	load_series_inputs(ctx, user, vec![row])
		.await?
		.pop()
		.ok_or_else(|| APIError::NotFound("Series does not exist".to_owned()))
}

/// Load the series input holding the media behind a Kavita volume/chapter id,
/// together with that media's position in the series.
pub(crate) async fn find_media(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	id: i32,
) -> APIResult<Option<(SeriesInput, usize)>> {
	let conn = ctx.conn();
	let Some(media_id) = KavitaIds::lookup(conn, IdKind::Media, id).await? else {
		return Ok(None);
	};
	let Some(media_row) = media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(media_id.clone()))
		.filter(media::Column::DeletedAt.is_null())
		.one(conn)
		.await?
	else {
		return Ok(None);
	};
	let Some(series_id) = media_row.series_id else {
		return Ok(None);
	};
	let Some(series_row) = find_series(ctx, user, &series_id).await? else {
		return Ok(None);
	};
	let input = load_series_input(ctx, user, series_row).await?;
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
) -> APIResult<Option<(library::Model, Option<models::entity::library_config::Model>)>> {
	let Some(library_id) = input.series.library_id.as_deref() else {
		return Ok(None);
	};
	Ok(library::Entity::find_for_user(user)
		.filter(library::Column::Id.eq(library_id))
		.find_also_related(models::entity::library_config::Entity)
		.one(ctx.conn())
		.await?)
}
