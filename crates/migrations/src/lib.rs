//! Append-only SeaORM schema migrations. `stump_core` runs [`Migrator::up`] on
//! every connect. Never reorder or remove published entries; one SQLite
//! column per `alter_table`. See `crates/migrations/README.md`.

use sea_orm_migration::async_trait::async_trait;
pub use sea_orm_migration::*;

mod m20250807_202824_init;
mod m20251013_233701_add_media_metadata_fields;
mod m20251020_145410_add_thumbnail_ratio;
mod m20251112_000000_add_oidc_to_users;
mod m20251116_000000_book_club_enhancements;
mod m20251117_220701_thumbnail_placeholders;
mod m20251118_183043_media_analysis;
mod m20251220_000000_library_view_mode;
mod m20251229_185620_fancy_animations_pref;
mod m20251229_200000_thumbnail_placeholder_style_pref;
mod m20260108_000000_add_series_metadata_fields;
mod m20260116_000000_rewrite_media_annotations;
mod m20260118_204601_add_bookmark_created_at;
mod m20260128_000000_add_library_type;
mod m20260207_000000_metadata_provider_integration;
mod m20260220_000000_user_avatar_path;
mod m20260307_000000_library_skip_book_overview;
mod m20260311_000000_scheduled_jobs_redesign;
mod m20260404_185829_add_name_indexes;
mod m20260406_000000_add_kobo_sync_sessions;
mod m20260505_231341_jwt_secrets;
mod m20260519_192218_reading_sessions_v2;
mod m20260523_220757_rename_registered_reading_devices;
mod m20260525_165704_roundness_preference;
mod m20260601_000000_add_scanned_directory;
mod m20260603_164540_thumbnail_roundness_preference;
mod m20260613_000000_add_series_path_index;
mod m20260617_200820_rm_thumbnails_in_headers_pref;
mod m20260702_000000_metadata_fetch_partial_results;
mod m20260804_000000_smart_list_role_to_integer;
mod m20260815_205755_avatar_image_metadata;
mod m20260816_000000_drop_legacy_epubcfi;
mod m20260830_015110_oneshots;
mod m20260902_000000_add_reading_lists_and_collections;
mod m20260904_000000_add_liseur_sync;
mod m20260905_000000_add_kobo_reading_state;
mod m20260906_000000_add_liseur_token_metadata;
mod m20260907_000000_add_ingest;
mod m20260908_000000_add_series_metadata_komga_fields;
mod m20260909_000000_add_ingest_media_targets;
mod m20260910_000000_add_devices;
mod m20260911_000000_add_device_pairings;
mod m20260912_000000_add_kavita_compat;
mod m20260913_000000_add_provider_sources;
mod m20260914_000000_add_reading_heads;
mod m20260917_000000_add_page_hashes;
mod m20260918_000000_add_container_shelves;
mod m20260919_000000_add_notification_rules;
mod m20260920_000000_add_annotation_sink_configs;
mod m20260922_kavita_on_deck_removals;
mod m20260923_000000_backfill_reading_heads;
mod m20260924_000000_add_provider_series_links;
mod m20260925_000000_add_device_library_scope;
mod m20260926_000000_add_annotation_attachments;
mod m20260927_000000_add_ingest_preprocess;
mod m20260928_000000_add_device_kindle_email;
mod m20260929_000000_add_library_metadata_policy;
mod m20260930_000000_add_media_audio;
mod m20260931_000000_add_media_audio_tracks;
mod m20260932_000000_add_media_audio_chapters;
mod m20260933_000000_add_time_positions;
mod m20260934_000000_add_bookmark_position_ms;
mod m20260935_000000_add_abs_compat;
mod m20260940_000000_add_kindle_deliveries;
mod m20260941_000000_add_provider_request_headers;
mod m20260942_000000_add_media_metadata_narrators;
mod m20260943_000000_add_abs_session_play_method;
mod m20260944_000000_add_worker_jobs;
mod m20260945_000000_add_ingest_drop_groups;
mod m20260946_000000_add_edition_pairing;
mod m20260947_000000_add_media_sync_maps;
mod m20260949_000000_add_liseur_session_id;
mod m20260950_000000_add_connections;
mod m20260951_000000_add_crosspoint;
mod m20260951_100000_add_crosspoint_target_created_at;
mod m20260952_000000_add_book_detail;
mod m20260953_000000_add_component_runtime;
mod m20260954_000000_social_recommendations;
mod m20260955_000000_add_book_requests;

// Keep newly added migrations appended in chronological order; do not reorder
// already-published migrations.

pub struct Migrator;

#[async_trait]
impl MigratorTrait for Migrator {
	fn migrations() -> Vec<Box<dyn MigrationTrait>> {
		vec![
			Box::new(m20250807_202824_init::Migration),
			Box::new(m20251013_233701_add_media_metadata_fields::Migration),
			Box::new(m20251020_145410_add_thumbnail_ratio::Migration),
			Box::new(m20251112_000000_add_oidc_to_users::Migration),
			Box::new(m20251116_000000_book_club_enhancements::Migration),
			Box::new(m20251117_220701_thumbnail_placeholders::Migration),
			Box::new(m20251118_183043_media_analysis::Migration),
			Box::new(m20251220_000000_library_view_mode::Migration),
			Box::new(m20251229_185620_fancy_animations_pref::Migration),
			Box::new(m20251229_200000_thumbnail_placeholder_style_pref::Migration),
			Box::new(m20260108_000000_add_series_metadata_fields::Migration),
			Box::new(m20260116_000000_rewrite_media_annotations::Migration),
			Box::new(m20260118_204601_add_bookmark_created_at::Migration),
			Box::new(m20260128_000000_add_library_type::Migration),
			Box::new(m20260207_000000_metadata_provider_integration::Migration),
			Box::new(m20260220_000000_user_avatar_path::Migration),
			Box::new(m20260307_000000_library_skip_book_overview::Migration),
			Box::new(m20260311_000000_scheduled_jobs_redesign::Migration),
			Box::new(m20260404_185829_add_name_indexes::Migration),
			Box::new(m20260406_000000_add_kobo_sync_sessions::Migration),
			Box::new(m20260505_231341_jwt_secrets::Migration),
			Box::new(m20260519_192218_reading_sessions_v2::Migration),
			Box::new(m20260523_220757_rename_registered_reading_devices::Migration),
			Box::new(m20260525_165704_roundness_preference::Migration),
			Box::new(m20260601_000000_add_scanned_directory::Migration),
			Box::new(m20260603_164540_thumbnail_roundness_preference::Migration),
			Box::new(m20260613_000000_add_series_path_index::Migration),
			Box::new(m20260617_200820_rm_thumbnails_in_headers_pref::Migration),
			Box::new(m20260702_000000_metadata_fetch_partial_results::Migration),
			Box::new(m20260804_000000_smart_list_role_to_integer::Migration),
			Box::new(m20260815_205755_avatar_image_metadata::Migration),
			Box::new(m20260816_000000_drop_legacy_epubcfi::Migration),
			Box::new(m20260830_015110_oneshots::Migration),
			Box::new(m20260902_000000_add_reading_lists_and_collections::Migration),
			Box::new(m20260904_000000_add_liseur_sync::Migration),
			Box::new(m20260905_000000_add_kobo_reading_state::Migration),
			Box::new(m20260906_000000_add_liseur_token_metadata::Migration),
			Box::new(m20260907_000000_add_ingest::Migration),
			Box::new(m20260908_000000_add_series_metadata_komga_fields::Migration),
			Box::new(m20260909_000000_add_ingest_media_targets::Migration),
			Box::new(m20260910_000000_add_devices::Migration),
			Box::new(m20260911_000000_add_device_pairings::Migration),
			Box::new(m20260912_000000_add_kavita_compat::Migration),
			Box::new(m20260913_000000_add_provider_sources::Migration),
			Box::new(m20260914_000000_add_reading_heads::Migration),
			Box::new(m20260917_000000_add_page_hashes::Migration),
			Box::new(m20260918_000000_add_container_shelves::Migration),
			Box::new(m20260919_000000_add_notification_rules::Migration),
			Box::new(m20260920_000000_add_annotation_sink_configs::Migration),
			Box::new(m20260922_kavita_on_deck_removals::Migration),
			Box::new(m20260923_000000_backfill_reading_heads::Migration),
			Box::new(m20260924_000000_add_provider_series_links::Migration),
			Box::new(m20260925_000000_add_device_library_scope::Migration),
			Box::new(m20260926_000000_add_annotation_attachments::Migration),
			Box::new(m20260927_000000_add_ingest_preprocess::Migration),
			Box::new(m20260928_000000_add_device_kindle_email::Migration),
			Box::new(m20260929_000000_add_library_metadata_policy::Migration),
			Box::new(m20260930_000000_add_media_audio::Migration),
			Box::new(m20260931_000000_add_media_audio_tracks::Migration),
			Box::new(m20260932_000000_add_media_audio_chapters::Migration),
			Box::new(m20260933_000000_add_time_positions::Migration),
			Box::new(m20260934_000000_add_bookmark_position_ms::Migration),
			Box::new(m20260935_000000_add_abs_compat::Migration),
			Box::new(m20260940_000000_add_kindle_deliveries::Migration),
			Box::new(m20260941_000000_add_provider_request_headers::Migration),
			Box::new(m20260942_000000_add_media_metadata_narrators::Migration),
			Box::new(m20260943_000000_add_abs_session_play_method::Migration),
			Box::new(m20260944_000000_add_worker_jobs::Migration),
			Box::new(m20260945_000000_add_ingest_drop_groups::Migration),
			Box::new(m20260946_000000_add_edition_pairing::Migration),
			Box::new(m20260947_000000_add_media_sync_maps::Migration),
			Box::new(m20260949_000000_add_liseur_session_id::Migration),
			Box::new(m20260950_000000_add_connections::Migration),
			Box::new(m20260951_000000_add_crosspoint::Migration),
			Box::new(m20260951_100000_add_crosspoint_target_created_at::Migration),
			Box::new(m20260952_000000_add_book_detail::Migration),
			Box::new(m20260953_000000_add_component_runtime::Migration),
			Box::new(m20260954_000000_social_recommendations::Migration),
			Box::new(m20260955_000000_add_book_requests::Migration),
		]
	}
}
