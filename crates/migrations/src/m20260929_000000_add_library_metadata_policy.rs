//! `library_configs.metadata_policy`: the per-library override of the
//! per-field metadata policy (which providers may fill a field, in what
//! order, and how their values combine).
//!
//! One nullable JSON-in-TEXT column, not a table: a policy is a single
//! document that is always read and written whole, has no rows worth
//! querying independently, and is meaningless outside its library. `NULL`
//! means "inherit the server default" (`stump_ingest::policy::MetadataPolicy::server_default`),
//! which is what every existing library gets, so the migration cannot change
//! behaviour for an existing install.
//!
//! It lives on `library_configs` rather than on `libraries` because it is
//! library *configuration* in exactly the sense `thumbnail_config` and
//! `ignore_rules` already are, and it is stored as TEXT rather than SeaORM's
//! `Json` because SQLite has no JSON column type and the surrounding columns
//! use the same shape.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LibraryConfigs::Table)
					.add_column(
						ColumnDef::new(LibraryConfigs::MetadataPolicy).text().null(),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(LibraryConfigs::Table)
					.drop_column(LibraryConfigs::MetadataPolicy)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum LibraryConfigs {
	Table,
	MetadataPolicy,
}
