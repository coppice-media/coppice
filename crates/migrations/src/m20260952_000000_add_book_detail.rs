//! Canonical storage for merged book detail work metadata and user reviews.
//!
//! The old `reviews` table is deliberately left intact for account-import and
//! older clients. Rows are copied once into `book_reviews`; a confirmed liseur
//! link becomes the review target, while an unpaired row remains media-scoped.
//! A work review and an edition review are different records and are never
//! silently mirrored.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(BookWorkMetadata::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(BookWorkMetadata::WorkId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(BookWorkMetadata::UserId).text().not_null())
					.col(ColumnDef::new(BookWorkMetadata::Title).text())
					.col(ColumnDef::new(BookWorkMetadata::Author).text())
					.col(ColumnDef::new(BookWorkMetadata::Metadata).json())
					.col(ColumnDef::new(BookWorkMetadata::LockedFields).json())
					.col(
						ColumnDef::new(BookWorkMetadata::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(BookWorkMetadata::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-work-metadata-work")
							.from(BookWorkMetadata::Table, BookWorkMetadata::WorkId)
							.to(LiseurSyncWorks::Table, LiseurSyncWorks::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-work-metadata-user")
							.from(BookWorkMetadata::Table, BookWorkMetadata::UserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_index(
				Index::create()
					.name("uq-book-work-metadata-user-work")
					.table(BookWorkMetadata::Table)
					.col(BookWorkMetadata::UserId)
					.col(BookWorkMetadata::WorkId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(BookReviews::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(BookReviews::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(BookReviews::WorkId).text())
					.col(ColumnDef::new(BookReviews::MediaId).text())
					.col(ColumnDef::new(BookReviews::UserId).text().not_null())
					.col(ColumnDef::new(BookReviews::Rating).integer().not_null())
					.col(ColumnDef::new(BookReviews::Content).text())
					.col(ColumnDef::new(BookReviews::IsPrivate).boolean().not_null())
					.col(
						ColumnDef::new(BookReviews::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(BookReviews::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-reviews-work")
							.from(BookReviews::Table, BookReviews::WorkId)
							.to(LiseurSyncWorks::Table, LiseurSyncWorks::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-reviews-media")
							.from(BookReviews::Table, BookReviews::MediaId)
							.to(Media::Table, Media::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-reviews-user")
							.from(BookReviews::Table, BookReviews::UserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.check(Expr::cust("(work_id IS NOT NULL AND media_id IS NULL) OR (work_id IS NULL AND media_id IS NOT NULL)"))
					.check(Expr::cust("rating BETWEEN 0 AND 5"))
					.to_owned(),
			)
			.await?;

		let conn = manager.get_connection();
		let backend = conn.get_database_backend();
		// `legacy-<id>` is deterministic and makes the migration idempotent if a
		// deployment retries after creating the tables but before recording the
		// migration. Invalid historical ratings are retained at the nearest
		// representable value rather than making the whole migration fail.
		conn.execute(Statement::from_string(
			backend,
			r#"
INSERT INTO book_reviews
    (id, work_id, media_id, user_id, rating, content, is_private, created_at, updated_at)
SELECT
    'legacy-' || r.id,
    CASE WHEN l.work_id IS NOT NULL THEN l.work_id ELSE NULL END,
    CASE WHEN l.work_id IS NOT NULL THEN NULL ELSE r.media_id END,
    r.user_id,
    CASE WHEN r.rating < 0 THEN 0 WHEN r.rating > 5 THEN 5 ELSE r.rating END,
    r.content,
    r.is_private,
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM reviews r
LEFT JOIN liseur_sync_media_links l
    ON l.user_id = r.user_id
   AND l.media_id = r.media_id
   AND COALESCE(l.pair_status, 'confirmed') = 'confirmed'
WHERE NOT EXISTS (
    SELECT 1 FROM book_reviews b WHERE b.id = 'legacy-' || r.id
)
"#
			.to_owned(),
		))
		.await?;

		// A user may have accumulated duplicate legacy rows. Keep the newest
		// deterministic row for each explicit target before adding the unique
		// partial indexes required by the book-detail contract.
		conn.execute(Statement::from_string(
			backend,
			r#"
DELETE FROM book_reviews
WHERE id IN (
    SELECT older.id
    FROM book_reviews older
    JOIN book_reviews newer
      ON newer.user_id = older.user_id
     AND (
          (newer.work_id IS NOT NULL AND older.work_id = newer.work_id)
       OR (newer.media_id IS NOT NULL AND older.media_id = newer.media_id)
     )
     AND (newer.updated_at > older.updated_at
       OR (newer.updated_at = older.updated_at AND newer.id > older.id))
)
"#
			.to_owned(),
		))
		.await?;
		conn.execute(Statement::from_string(
			backend,
			"CREATE UNIQUE INDEX IF NOT EXISTS uq_book_reviews_user_work ON book_reviews(user_id, work_id) WHERE work_id IS NOT NULL".to_owned(),
		))
		.await?;
		conn.execute(Statement::from_string(
			backend,
			"CREATE UNIQUE INDEX IF NOT EXISTS uq_book_reviews_user_media ON book_reviews(user_id, media_id) WHERE media_id IS NOT NULL".to_owned(),
		))
		.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(BookReviews::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(BookWorkMetadata::Table)
					.if_exists()
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum BookWorkMetadata {
	#[sea_orm(iden = "book_work_metadata")]
	Table,
	WorkId,
	UserId,
	Title,
	Author,
	Metadata,
	LockedFields,
	CreatedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum BookReviews {
	#[sea_orm(iden = "book_reviews")]
	Table,
	Id,
	WorkId,
	MediaId,
	UserId,
	Rating,
	Content,
	IsPrivate,
	CreatedAt,
	UpdatedAt,
}

#[derive(DeriveIden)]
enum LiseurSyncWorks {
	#[sea_orm(iden = "liseur_sync_works")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Media {
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Users {
	Table,
	Id,
}
