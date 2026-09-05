use sea_orm_migration::prelude::*;

/// Library-wide rework targets: `ingest_quality_reports` and
/// `ingest_analysis_jobs` may reference a `media` row instead of a drop item.
///
/// SQLite cannot alter a column's nullability in place, so both tables are
/// rebuilt: `drop_item_id` becomes nullable, a nullable `media_id` with a
/// cascading foreign key is added, and existing rows keep their drop-item
/// target. `ingest_metadata_candidates` already carried a nullable
/// `media_id` since `m20260907_000000_add_ingest`, so it needs no change.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(AnalysisJobsMediaTargets::Table)
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(AnalysisJobsMediaTargets::DropItemId).text())
					.col(ColumnDef::new(AnalysisJobsMediaTargets::MediaId).text())
					.col(ColumnDef::new(AnalysisJobsMediaTargets::JobId).text())
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::Status)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::Phase)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::Priority)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::Attempts)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::Plan)
							.json()
							.not_null(),
					)
					.col(ColumnDef::new(AnalysisJobsMediaTargets::Error).text())
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::StartedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(AnalysisJobsMediaTargets::FinishedAt)
							.timestamp_with_time_zone(),
					)
					.foreign_key(
						ForeignKey::create()
							.from(
								AnalysisJobsMediaTargets::Table,
								AnalysisJobsMediaTargets::DropItemId,
							)
							.to(IngestDropItems::Table, IngestDropItems::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.from(
								AnalysisJobsMediaTargets::Table,
								AnalysisJobsMediaTargets::MediaId,
							)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.get_connection()
			.execute_unprepared(
				"INSERT INTO \"ingest_analysis_jobs_media_targets\" \
				 (\"id\", \"drop_item_id\", \"media_id\", \"job_id\", \"status\", \"phase\", \
				 \"priority\", \"attempts\", \"plan\", \"error\", \"created_at\", \
				 \"started_at\", \"finished_at\") \
				 SELECT \"id\", \"drop_item_id\", NULL, \"job_id\", \"status\", \"phase\", \
				 \"priority\", \"attempts\", \"plan\", \"error\", \"created_at\", \
				 \"started_at\", \"finished_at\" FROM \"ingest_analysis_jobs\"",
			)
			.await?;
		manager
			.drop_table(Table::drop().table(IngestAnalysisJobs::Table).to_owned())
			.await?;
		manager
			.rename_table(
				Table::rename()
					.table(AnalysisJobsMediaTargets::Table, IngestAnalysisJobs::Table)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx_ingest_analysis_jobs_drop_item")
					.table(IngestAnalysisJobs::Table)
					.col(IngestAnalysisJobs::DropItemId)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx_ingest_analysis_jobs_media")
					.table(IngestAnalysisJobs::Table)
					.col(IngestAnalysisJobs::MediaId)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(QualityReportsMediaTargets::Table)
					.col(
						ColumnDef::new(QualityReportsMediaTargets::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(QualityReportsMediaTargets::DropItemId).text())
					.col(ColumnDef::new(QualityReportsMediaTargets::MediaId).text())
					.col(
						ColumnDef::new(QualityReportsMediaTargets::SourceSha256)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsMediaTargets::AlgorithmVersion)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsMediaTargets::Score)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsMediaTargets::Checks)
							.json()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsMediaTargets::SettingsSnapshot)
							.json()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsMediaTargets::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.from(
								QualityReportsMediaTargets::Table,
								QualityReportsMediaTargets::DropItemId,
							)
							.to(IngestDropItems::Table, IngestDropItems::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.from(
								QualityReportsMediaTargets::Table,
								QualityReportsMediaTargets::MediaId,
							)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.get_connection()
			.execute_unprepared(
				"INSERT INTO \"ingest_quality_reports_media_targets\" \
				 (\"id\", \"drop_item_id\", \"media_id\", \"source_sha256\", \
				 \"algorithm_version\", \"score\", \"checks\", \"settings_snapshot\", \
				 \"created_at\") \
				 SELECT \"id\", \"drop_item_id\", NULL, \"source_sha256\", \
				 \"algorithm_version\", \"score\", \"checks\", \"settings_snapshot\", \
				 \"created_at\" FROM \"ingest_quality_reports\"",
			)
			.await?;
		manager
			.drop_table(Table::drop().table(IngestQualityReports::Table).to_owned())
			.await?;
		manager
			.rename_table(
				Table::rename()
					.table(
						QualityReportsMediaTargets::Table,
						IngestQualityReports::Table,
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx_ingest_quality_reports_drop_item")
					.table(IngestQualityReports::Table)
					.col(IngestQualityReports::DropItemId)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx_ingest_quality_reports_media")
					.table(IngestQualityReports::Table)
					.col(IngestQualityReports::MediaId)
					.to_owned(),
			)
			.await?;
		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		// Best effort: restore the pre-media-target shapes, keeping only rows
		// that still target a drop item.
		manager
			.create_table(
				Table::create()
					.table(AnalysisJobsLegacy::Table)
					.col(
						ColumnDef::new(AnalysisJobsLegacy::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(AnalysisJobsLegacy::DropItemId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(AnalysisJobsLegacy::JobId).text())
					.col(ColumnDef::new(AnalysisJobsLegacy::Status).text().not_null())
					.col(ColumnDef::new(AnalysisJobsLegacy::Phase).text().not_null())
					.col(
						ColumnDef::new(AnalysisJobsLegacy::Priority)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnalysisJobsLegacy::Attempts)
							.integer()
							.not_null(),
					)
					.col(ColumnDef::new(AnalysisJobsLegacy::Plan).json().not_null())
					.col(ColumnDef::new(AnalysisJobsLegacy::Error).text())
					.col(
						ColumnDef::new(AnalysisJobsLegacy::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(AnalysisJobsLegacy::StartedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(AnalysisJobsLegacy::FinishedAt)
							.timestamp_with_time_zone(),
					)
					.foreign_key(
						ForeignKey::create()
							.from(
								AnalysisJobsLegacy::Table,
								AnalysisJobsLegacy::DropItemId,
							)
							.to(IngestDropItems::Table, IngestDropItems::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.get_connection()
			.execute_unprepared(
				"INSERT INTO \"ingest_analysis_jobs_legacy\" \
				 (\"id\", \"drop_item_id\", \"job_id\", \"status\", \"phase\", \
				 \"priority\", \"attempts\", \"plan\", \"error\", \"created_at\", \
				 \"started_at\", \"finished_at\") \
				 SELECT \"id\", \"drop_item_id\", \"job_id\", \"status\", \"phase\", \
				 \"priority\", \"attempts\", \"plan\", \"error\", \"created_at\", \
				 \"started_at\", \"finished_at\" FROM \"ingest_analysis_jobs\" \
				 WHERE \"drop_item_id\" IS NOT NULL",
			)
			.await?;
		manager
			.drop_table(Table::drop().table(IngestAnalysisJobs::Table).to_owned())
			.await?;
		manager
			.rename_table(
				Table::rename()
					.table(AnalysisJobsLegacy::Table, IngestAnalysisJobs::Table)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(QualityReportsLegacy::Table)
					.col(
						ColumnDef::new(QualityReportsLegacy::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(QualityReportsLegacy::DropItemId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsLegacy::SourceSha256)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsLegacy::AlgorithmVersion)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsLegacy::Score)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsLegacy::Checks)
							.json()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsLegacy::SettingsSnapshot)
							.json()
							.not_null(),
					)
					.col(
						ColumnDef::new(QualityReportsLegacy::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.from(
								QualityReportsLegacy::Table,
								QualityReportsLegacy::DropItemId,
							)
							.to(IngestDropItems::Table, IngestDropItems::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.get_connection()
			.execute_unprepared(
				"INSERT INTO \"ingest_quality_reports_legacy\" \
				 SELECT \"id\", \"drop_item_id\", \"source_sha256\", \"algorithm_version\", \
				 \"score\", \"checks\", \"settings_snapshot\", \"created_at\" \
				 FROM \"ingest_quality_reports\" WHERE \"drop_item_id\" IS NOT NULL",
			)
			.await?;
		manager
			.drop_table(Table::drop().table(IngestQualityReports::Table).to_owned())
			.await?;
		manager
			.rename_table(
				Table::rename()
					.table(QualityReportsLegacy::Table, IngestQualityReports::Table)
					.to_owned(),
			)
			.await?;
		Ok(())
	}
}

#[derive(DeriveIden)]
enum IngestDropItems {
	#[sea_orm(iden = "ingest_drop_items")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum IngestAnalysisJobs {
	#[sea_orm(iden = "ingest_analysis_jobs")]
	Table,
	DropItemId,
	MediaId,
}

#[derive(DeriveIden)]
enum IngestQualityReports {
	#[sea_orm(iden = "ingest_quality_reports")]
	Table,
	DropItemId,
	MediaId,
}

#[derive(DeriveIden)]
enum Media {
	#[sea_orm(iden = "media")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum AnalysisJobsMediaTargets {
	#[sea_orm(iden = "ingest_analysis_jobs_media_targets")]
	Table,
	Id,
	DropItemId,
	MediaId,
	JobId,
	Status,
	Phase,
	Priority,
	Attempts,
	Plan,
	Error,
	CreatedAt,
	StartedAt,
	FinishedAt,
}

#[derive(DeriveIden)]
enum AnalysisJobsLegacy {
	#[sea_orm(iden = "ingest_analysis_jobs_legacy")]
	Table,
	Id,
	DropItemId,
	JobId,
	Status,
	Phase,
	Priority,
	Attempts,
	Plan,
	Error,
	CreatedAt,
	StartedAt,
	FinishedAt,
}

#[derive(DeriveIden)]
enum QualityReportsMediaTargets {
	#[sea_orm(iden = "ingest_quality_reports_media_targets")]
	Table,
	Id,
	DropItemId,
	MediaId,
	SourceSha256,
	AlgorithmVersion,
	Score,
	Checks,
	SettingsSnapshot,
	CreatedAt,
}

#[derive(DeriveIden)]
enum QualityReportsLegacy {
	#[sea_orm(iden = "ingest_quality_reports_legacy")]
	Table,
	Id,
	DropItemId,
	SourceSha256,
	AlgorithmVersion,
	Score,
	Checks,
	SettingsSnapshot,
	CreatedAt,
}
