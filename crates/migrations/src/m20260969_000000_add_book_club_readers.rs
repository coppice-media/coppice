//! Separate, revocable per-participant reader sessions and their private state.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(ReaderSessions::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReaderSessions::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(ReaderSessions::BookClubId).text().not_null())
					.col(
						ColumnDef::new(ReaderSessions::CreatedByUserId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ReaderSessions::PublishedByUserId).text())
					.col(ColumnDef::new(ReaderSessions::Name).text().not_null())
					.col(ColumnDef::new(ReaderSessions::PublishedBookId).text())
					.col(
						ColumnDef::new(ReaderSessions::ExpiresAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(ReaderSessions::ClosedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(ReaderSessions::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReaderSessions::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-sessions-club")
							.from(ReaderSessions::Table, ReaderSessions::BookClubId)
							.to(BookClubs::Table, BookClubs::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-sessions-creator")
							.from(ReaderSessions::Table, ReaderSessions::CreatedByUserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-sessions-publisher")
							.from(
								ReaderSessions::Table,
								ReaderSessions::PublishedByUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::SetNull),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(ReaderParticipants::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReaderParticipants::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(ReaderParticipants::SessionId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ReaderParticipants::LinkedUserId).text())
					.col(ColumnDef::new(ReaderParticipants::CreatedByUserId).text())
					.col(
						ColumnDef::new(ReaderParticipants::DisplayName)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReaderParticipants::ShareProgress)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(ColumnDef::new(ReaderParticipants::TokenDigest).text())
					.col(
						ColumnDef::new(ReaderParticipants::CredentialExpiresAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(ReaderParticipants::RevokedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(ReaderParticipants::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReaderParticipants::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-participants-session")
							.from(
								ReaderParticipants::Table,
								ReaderParticipants::SessionId,
							)
							.to(ReaderSessions::Table, ReaderSessions::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-participants-user")
							.from(
								ReaderParticipants::Table,
								ReaderParticipants::LinkedUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::SetNull),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-participants-creator")
							.from(
								ReaderParticipants::Table,
								ReaderParticipants::CreatedByUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::SetNull),
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-book-club-reader-participant-token")
					.table(ReaderParticipants::Table)
					.col(ReaderParticipants::TokenDigest)
					.unique()
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-book-club-reader-participant-linked-user")
					.table(ReaderParticipants::Table)
					.col(ReaderParticipants::SessionId)
					.col(ReaderParticipants::LinkedUserId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(ReaderProgress::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReaderProgress::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(ReaderProgress::SessionId).text().not_null())
					.col(
						ColumnDef::new(ReaderProgress::ParticipantId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ReaderProgress::BookId).text().not_null())
					.col(
						ColumnDef::new(ReaderProgress::Progression)
							.double()
							.not_null()
							.default(0.0),
					)
					.col(ColumnDef::new(ReaderProgress::Locator).json())
					.col(ColumnDef::new(ReaderProgress::Page).integer())
					.col(ColumnDef::new(ReaderProgress::PositionMs).big_integer())
					.col(
						ColumnDef::new(ReaderProgress::IsComplete)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(ReaderProgress::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReaderProgress::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-progress-session")
							.from(ReaderProgress::Table, ReaderProgress::SessionId)
							.to(ReaderSessions::Table, ReaderSessions::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-progress-participant")
							.from(ReaderProgress::Table, ReaderProgress::ParticipantId)
							.to(ReaderParticipants::Table, ReaderParticipants::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-book-club-reader-progress-book")
					.table(ReaderProgress::Table)
					.col(ReaderProgress::SessionId)
					.col(ReaderProgress::BookId)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-book-club-reader-progress-participant-book")
					.table(ReaderProgress::Table)
					.col(ReaderProgress::ParticipantId)
					.col(ReaderProgress::BookId)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(ReaderAnnotations::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReaderAnnotations::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(ReaderAnnotations::SessionId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReaderAnnotations::ParticipantId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ReaderAnnotations::BookId).text().not_null())
					.col(ColumnDef::new(ReaderAnnotations::Kind).text().not_null())
					.col(ColumnDef::new(ReaderAnnotations::Locator).json())
					.col(ColumnDef::new(ReaderAnnotations::Page).integer())
					.col(ColumnDef::new(ReaderAnnotations::PositionMs).big_integer())
					.col(ColumnDef::new(ReaderAnnotations::Excerpt).text())
					.col(ColumnDef::new(ReaderAnnotations::Body).text())
					.col(ColumnDef::new(ReaderAnnotations::Color).text())
					.col(
						ColumnDef::new(ReaderAnnotations::Shared)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(ReaderAnnotations::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReaderAnnotations::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-annotations-session")
							.from(ReaderAnnotations::Table, ReaderAnnotations::SessionId)
							.to(ReaderSessions::Table, ReaderSessions::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-annotations-participant")
							.from(
								ReaderAnnotations::Table,
								ReaderAnnotations::ParticipantId,
							)
							.to(ReaderParticipants::Table, ReaderParticipants::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("idx-book-club-reader-annotations-book")
					.table(ReaderAnnotations::Table)
					.col(ReaderAnnotations::SessionId)
					.col(ReaderAnnotations::BookId)
					.to_owned(),
			)
			.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(ReaderAnnotations::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(ReaderProgress::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(ReaderParticipants::Table).to_owned())
			.await?;
		manager
			.drop_table(Table::drop().table(ReaderSessions::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum ReaderSessions {
	#[sea_orm(iden = "book_club_reader_sessions")]
	Table,
	Id,
	#[sea_orm(iden = "book_club_id")]
	BookClubId,
	#[sea_orm(iden = "created_by_user_id")]
	CreatedByUserId,
	#[sea_orm(iden = "published_by_user_id")]
	PublishedByUserId,
	Name,
	#[sea_orm(iden = "published_book_id")]
	PublishedBookId,
	#[sea_orm(iden = "expires_at")]
	ExpiresAt,
	#[sea_orm(iden = "closed_at")]
	ClosedAt,
	#[sea_orm(iden = "created_at")]
	CreatedAt,
	#[sea_orm(iden = "updated_at")]
	UpdatedAt,
}

#[derive(DeriveIden)]
enum ReaderParticipants {
	#[sea_orm(iden = "book_club_reader_participants")]
	Table,
	Id,
	#[sea_orm(iden = "session_id")]
	SessionId,
	#[sea_orm(iden = "linked_user_id")]
	LinkedUserId,
	#[sea_orm(iden = "created_by_user_id")]
	CreatedByUserId,
	#[sea_orm(iden = "display_name")]
	DisplayName,
	#[sea_orm(iden = "share_progress")]
	ShareProgress,
	#[sea_orm(iden = "token_digest")]
	TokenDigest,
	#[sea_orm(iden = "credential_expires_at")]
	CredentialExpiresAt,
	#[sea_orm(iden = "revoked_at")]
	RevokedAt,
	#[sea_orm(iden = "created_at")]
	CreatedAt,
	#[sea_orm(iden = "updated_at")]
	UpdatedAt,
}

#[derive(DeriveIden)]
enum ReaderProgress {
	#[sea_orm(iden = "book_club_reader_progress")]
	Table,
	Id,
	#[sea_orm(iden = "session_id")]
	SessionId,
	#[sea_orm(iden = "participant_id")]
	ParticipantId,
	#[sea_orm(iden = "book_id")]
	BookId,
	Progression,
	Locator,
	Page,
	#[sea_orm(iden = "position_ms")]
	PositionMs,
	#[sea_orm(iden = "is_complete")]
	IsComplete,
	#[sea_orm(iden = "created_at")]
	CreatedAt,
	#[sea_orm(iden = "updated_at")]
	UpdatedAt,
}

#[derive(DeriveIden)]
enum ReaderAnnotations {
	#[sea_orm(iden = "book_club_reader_annotations")]
	Table,
	Id,
	#[sea_orm(iden = "session_id")]
	SessionId,
	#[sea_orm(iden = "participant_id")]
	ParticipantId,
	#[sea_orm(iden = "book_id")]
	BookId,
	Kind,
	Locator,
	Page,
	#[sea_orm(iden = "position_ms")]
	PositionMs,
	Excerpt,
	Body,
	Color,
	Shared,
	#[sea_orm(iden = "created_at")]
	CreatedAt,
	#[sea_orm(iden = "updated_at")]
	UpdatedAt,
}

#[derive(DeriveIden)]
enum BookClubs {
	#[sea_orm(iden = "book_clubs")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum Users {
	#[sea_orm(iden = "users")]
	Table,
	Id,
}
