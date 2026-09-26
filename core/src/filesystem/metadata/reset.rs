use models::entity::{media, media_metadata, metadata_fetch_record, series_metadata};
use sea_orm::{
	ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, TransactionTrait,
};

use crate::CoreError;

/// Remove native provider metadata and match history for one series and its media.
pub async fn reset_series_metadata<C>(conn: &C, series_id: &str) -> Result<(), CoreError>
where
	C: ConnectionTrait + TransactionTrait,
{
	reset_metadata(conn, &[series_id.to_string()]).await
}

/// Remove native provider metadata and match history for the supplied series.
pub async fn reset_library_metadata<C>(
	conn: &C,
	series_ids: &[String],
) -> Result<(), CoreError>
where
	C: ConnectionTrait + TransactionTrait,
{
	reset_metadata(conn, series_ids).await
}

async fn reset_metadata<C>(conn: &C, series_ids: &[String]) -> Result<(), CoreError>
where
	C: ConnectionTrait + TransactionTrait,
{
	if series_ids.is_empty() {
		return Ok(());
	}

	let tx = conn.begin().await?;
	let media_ids = media::Entity::find()
		.filter(media::Column::SeriesId.is_in(series_ids.to_vec()))
		.select_only()
		.column(media::Column::Id)
		.into_tuple::<String>()
		.all(&tx)
		.await?;

	series_metadata::Entity::delete_many()
		.filter(series_metadata::Column::SeriesId.is_in(series_ids.to_vec()))
		.exec(&tx)
		.await?;
	if !media_ids.is_empty() {
		media_metadata::Entity::delete_many()
			.filter(media_metadata::Column::MediaId.is_in(media_ids.clone()))
			.exec(&tx)
			.await?;
	}
	metadata_fetch_record::Entity::delete_many()
		.filter(metadata_fetch_record::Column::SeriesId.is_in(series_ids.to_vec()))
		.exec(&tx)
		.await?;
	if !media_ids.is_empty() {
		metadata_fetch_record::Entity::delete_many()
			.filter(metadata_fetch_record::Column::MediaId.is_in(media_ids))
			.exec(&tx)
			.await?;
	}

	tx.commit().await?;
	Ok(())
}
