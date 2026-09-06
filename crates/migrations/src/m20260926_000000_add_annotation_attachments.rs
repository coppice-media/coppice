use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Binary side objects hanging off a liseur-sync annotation: the handwritten
/// markup SVG, its page snapshot, and exported notebooks.
///
/// The rows are metadata only; the bytes live under
/// `<config_dir>/attachments/<annotation_id>/<sha256>.<ext>` and `storage_path`
/// is that path relative to the attachment root. `user_id` is carried so the
/// foreign key can reference the annotation's `(user_id, annotation_id)`
/// unique index (`uq-liseur-sync-annotations-user-id`): a liseur annotation id
/// is client-chosen and therefore unique per user, never globally.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(AnnotationAttachments::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(AnnotationAttachments::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::AnnotationId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::Kind)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::MediaType)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::ByteSize)
							.big_integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::Sha256)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::StoragePath)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(AnnotationAttachments::CreatedAt)
							.text()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-annotation-attachments-annotation")
							.from_tbl(AnnotationAttachments::Table)
							.from_col(AnnotationAttachments::UserId)
							.from_col(AnnotationAttachments::AnnotationId)
							.to_tbl(LiseurSyncAnnotations::Table)
							.to_col(LiseurSyncAnnotations::UserId)
							.to_col(LiseurSyncAnnotations::AnnotationId)
							.on_delete(ForeignKeyAction::Cascade)
							.on_update(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		for index in [
			// One attachment per annotation and kind: a re-upload with a new
			// digest replaces the previous file.
			Index::create()
				.name("uq-annotation-attachments-annotation-kind")
				.table(AnnotationAttachments::Table)
				.col(AnnotationAttachments::UserId)
				.col(AnnotationAttachments::AnnotationId)
				.col(AnnotationAttachments::Kind)
				.unique()
				.to_owned(),
			Index::create()
				.name("idx-annotation-attachments-annotation")
				.table(AnnotationAttachments::Table)
				.col(AnnotationAttachments::UserId)
				.col(AnnotationAttachments::AnnotationId)
				.to_owned(),
		] {
			manager.create_index(index).await?;
		}

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(AnnotationAttachments::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum AnnotationAttachments {
	Table,
	Id,
	UserId,
	AnnotationId,
	Kind,
	MediaType,
	ByteSize,
	Sha256,
	StoragePath,
	CreatedAt,
}

#[derive(DeriveIden)]
enum LiseurSyncAnnotations {
	#[sea_orm(iden = "liseur_sync_annotations")]
	Table,
	UserId,
	AnnotationId,
}
