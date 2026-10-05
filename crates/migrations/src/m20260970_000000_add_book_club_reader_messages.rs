//! The per-session guest-reader discussion: plain-text messages visible only to
//! the session's active participants and its organizers.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.create_table(
				Table::create()
					.table(ReaderMessages::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(ReaderMessages::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(ReaderMessages::SessionId).text().not_null())
					.col(
						ColumnDef::new(ReaderMessages::ParticipantId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(ReaderMessages::BookId).text())
					.col(ColumnDef::new(ReaderMessages::Body).text().not_null())
					.col(
						ColumnDef::new(ReaderMessages::CreatedAt)
							.timestamp_with_time_zone()
							.not_null(),
					)
					.col(
						ColumnDef::new(ReaderMessages::EditedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(ReaderMessages::DeletedAt)
							.timestamp_with_time_zone(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-messages-session")
							.from(ReaderMessages::Table, ReaderMessages::SessionId)
							.to(ReaderSessions::Table, ReaderSessions::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-book-club-reader-messages-participant")
							.from(ReaderMessages::Table, ReaderMessages::ParticipantId)
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
					.name("idx-book-club-reader-messages-session-created")
					.table(ReaderMessages::Table)
					.col(ReaderMessages::SessionId)
					.col(ReaderMessages::CreatedAt)
					.to_owned(),
			)
			.await?;
		// Serves the per-participant posting-rate window and the participant
		// foreign key's cascade.
		manager
			.create_index(
				Index::create()
					.name("idx-book-club-reader-messages-participant-created")
					.table(ReaderMessages::Table)
					.col(ReaderMessages::ParticipantId)
					.col(ReaderMessages::CreatedAt)
					.to_owned(),
			)
			.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(Table::drop().table(ReaderMessages::Table).to_owned())
			.await
	}
}

#[derive(DeriveIden)]
enum ReaderMessages {
	#[sea_orm(iden = "book_club_reader_messages")]
	Table,
	Id,
	#[sea_orm(iden = "session_id")]
	SessionId,
	#[sea_orm(iden = "participant_id")]
	ParticipantId,
	#[sea_orm(iden = "book_id")]
	BookId,
	Body,
	#[sea_orm(iden = "created_at")]
	CreatedAt,
	#[sea_orm(iden = "edited_at")]
	EditedAt,
	#[sea_orm(iden = "deleted_at")]
	DeletedAt,
}

#[derive(DeriveIden)]
enum ReaderSessions {
	#[sea_orm(iden = "book_club_reader_sessions")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum ReaderParticipants {
	#[sea_orm(iden = "book_club_reader_participants")]
	Table,
	Id,
}
