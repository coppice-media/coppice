//! `provider_sources.request_headers`: operator-configured HTTP headers sent
//! with every request one source instance makes, as a JSON object of strings
//! (`{"Cookie": "cf_clearance=…", "User-Agent": "…"}`).
//!
//! Some sources sit behind a Cloudflare managed challenge — since 2026-06
//! `readcomicsonline.ru` answers every HTML path with `403` +
//! `cf-mitigated: challenge` to anything that cannot run the challenge script.
//! The only way through without a headless browser is to replay the clearance
//! cookie a human already earned in a real browser, together with the same
//! User-Agent it is bound to. That is per-instance operator configuration, so
//! it belongs on the instance row next to `base_url`, not in a global config
//! key (each source has its own cookie) and not in a second settings table
//! (there is exactly one such value per instance and it has no history).
//!
//! Nullable because almost no source needs one, and `TEXT` because the shape
//! is a header map the server validates on the way in
//! (`stump_provider::http::RequestHeaders`) rather than a schema.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(ProviderSources::Table)
					.add_column(
						ColumnDef::new(ProviderSources::RequestHeaders)
							.text()
							.null(),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(ProviderSources::Table)
					.drop_column(ProviderSources::RequestHeaders)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum ProviderSources {
	#[sea_orm(iden = "provider_sources")]
	Table,
	RequestHeaders,
}
