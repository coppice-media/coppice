use sea_orm::{prelude::*, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set};

use crate::entity::tag;

/// Given desired tag names and the tags currently linked to an entity, resolves which tags
/// need to be created, connected, and disconnected. Returns `(tag_ids_to_connect, tag_ids_to_disconnect)`.
///
/// Tags in `desired` that don't exist in the database are created. Tags currently linked but
/// not in `desired` are marked for disconnection.
pub async fn sync_tags(
	txn: &DatabaseTransaction,
	desired: &[String],
	existing_linked: &[tag::Model],
) -> Result<(Vec<i32>, Vec<i32>), DbErr> {
	// Tags in desired that are NOT currently linked to this entity
	let tags_not_linked = desired
		.iter()
		.filter(|name| !existing_linked.iter().any(|t| t.name == **name))
		.collect::<Vec<_>>();

	// Of those, which already exist in the tags table (but aren't linked to this entity)?
	let tags_existing_but_not_linked = tag::Entity::find()
		.filter(tag::Column::Name.is_in(tags_not_linked.clone()))
		.all(txn)
		.await?;

	// The rest need to be created
	let tags_to_create = tags_not_linked
		.iter()
		.filter(|name| {
			!tags_existing_but_not_linked
				.iter()
				.any(|t| t.name == ***name)
		})
		.map(|name| tag::ActiveModel {
			name: Set(name.to_string()),
			..Default::default()
		})
		.collect::<Vec<_>>();

	let created_tags = if !tags_to_create.is_empty() {
		tag::Entity::insert_many(tags_to_create)
			.exec_with_returning_many(txn)
			.await?
	} else {
		vec![]
	};

	let to_connect = tags_existing_but_not_linked
		.iter()
		.chain(created_tags.iter())
		.map(|tag| tag.id)
		.collect::<Vec<_>>();

	let to_disconnect = existing_linked
		.iter()
		.filter(|tag| !desired.iter().any(|name| name == &tag.name))
		.map(|tag| tag.id)
		.collect::<Vec<_>>();

	Ok((to_connect, to_disconnect))
}
