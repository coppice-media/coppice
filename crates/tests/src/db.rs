use models::entity::{
	age_restriction, api_key, collection, collection_series, device, device_credential,
	device_pairing, kobo_sync_session, library, library_config, library_exclusion,
	liseur_sync_token, media, media_analysis, media_metadata, media_tag, provider_source,
	reading_head, reading_head_event, reading_list, reading_list_item, reading_list_rule,
	reading_session, refresh_token, series, series_metadata, server_config, session,
	source_health, tag, user, user_preferences,
};
use models::entity::{known_duplicate_page, page_hash};
use sea_orm::{ConnectionTrait, Database, DbBackend, DbConn, DbErr, Schema};
pub async fn test_database() -> DbConn {
	let db = Database::connect("sqlite::memory:")
		.await
		.expect("failed to connect to test database");

	create_database_tables(&db)
		.await
		.expect("failed to create test database tables");

	db
}

pub async fn create_database_tables(db: &DbConn) -> Result<(), DbErr> {
	let schema = Schema::new(DbBackend::Sqlite);

	let tables = [
		schema.create_table_from_entity(api_key::Entity),
		schema.create_table_from_entity(media::Entity),
		schema.create_table_from_entity(media_metadata::Entity),
		schema.create_table_from_entity(media_analysis::Entity),
		schema.create_table_from_entity(series::Entity),
		schema.create_table_from_entity(series_metadata::Entity),
		schema.create_table_from_entity(library_exclusion::Entity),
		schema.create_table_from_entity(kobo_sync_session::Entity),
		schema.create_table_from_entity(age_restriction::Entity),
		schema.create_table_from_entity(user::Entity),
		schema.create_table_from_entity(user_preferences::Entity),
		schema.create_table_from_entity(library::Entity),
		schema.create_table_from_entity(library_config::Entity),
		schema.create_table_from_entity(reading_session::Entity),
		schema.create_table_from_entity(device::Entity),
		schema.create_table_from_entity(device_credential::Entity),
		schema.create_table_from_entity(liseur_sync_token::Entity),
		schema.create_table_from_entity(tag::Entity),
		schema.create_table_from_entity(media_tag::Entity),
		schema.create_table_from_entity(server_config::Entity),
		schema.create_table_from_entity(refresh_token::Entity),
		schema.create_table_from_entity(session::Entity),
		schema.create_table_from_entity(reading_list::Entity),
		schema.create_table_from_entity(reading_list_item::Entity),
		schema.create_table_from_entity(reading_list_rule::Entity),
		schema.create_table_from_entity(collection::Entity),
		schema.create_table_from_entity(collection_series::Entity),
		schema.create_table_from_entity(device_pairing::Entity),
		schema.create_table_from_entity(provider_source::Entity),
		schema.create_table_from_entity(source_health::Entity),
		schema.create_table_from_entity(reading_head::Entity),
		schema.create_table_from_entity(reading_head_event::Entity),
		schema.create_table_from_entity(page_hash::Entity),
		schema.create_table_from_entity(known_duplicate_page::Entity),
	];

	for stmt in tables {
		db.execute(db.get_database_backend().build(&stmt)).await?;
	}

	Ok(())
}
