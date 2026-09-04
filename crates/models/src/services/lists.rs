use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

use crate::entity::{collection_series, reading_list_item};

/// Remove reading-list memberships for the given media records.
///
/// This must be called on the same transaction immediately before hard-deleting media because
/// the reading-list foreign key intentionally uses `ON DELETE RESTRICT`.
pub async fn remove_memberships_for_media(
	txn: &impl ConnectionTrait,
	media_ids: &[String],
) -> Result<(), sea_orm::DbErr> {
	if media_ids.is_empty() {
		return Ok(());
	}

	reading_list_item::Entity::delete_many()
		.filter(reading_list_item::Column::MediaId.is_in(media_ids))
		.exec(txn)
		.await?;

	Ok(())
}

/// Remove collection memberships for the given series records.
///
/// This must be called on the same transaction immediately before hard-deleting series because
/// the collection foreign key intentionally uses `ON DELETE RESTRICT`.
pub async fn remove_memberships_for_series(
	txn: &impl ConnectionTrait,
	series_ids: &[String],
) -> Result<(), sea_orm::DbErr> {
	if series_ids.is_empty() {
		return Ok(());
	}
	collection_series::Entity::delete_many()
		.filter(collection_series::Column::SeriesId.is_in(series_ids))
		.exec(txn)
		.await?;

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{remove_memberships_for_media, remove_memberships_for_series};
	use sea_orm::{DatabaseBackend, MockDatabase};

	#[test]
	fn empty_membership_removal_is_a_noop() {
		let conn = MockDatabase::new(DatabaseBackend::Sqlite).into_connection();

		tokio_test::block_on(remove_memberships_for_media(&conn, &[])).unwrap();
		tokio_test::block_on(remove_memberships_for_series(&conn, &[])).unwrap();
		assert!(conn.into_transaction_log().is_empty());
	}
}
