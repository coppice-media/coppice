//! Social authorization, recommendation and explicit annotation-sharing schema.
//!
//! This migration keeps sender-private identifiers as non-authoritative hints
//! and persists immutable snapshots for every recipient-visible record. Existing
//! BookClub invitations gain an auditable lifecycle instead of being deleted on
//! response.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		// Invitation lifecycle is append-only: one column per ALTER keeps SQLite
		// migrations compatible with the project's migration guardrails.
		for column in [
			ColumnDef::new(BookClubInvitations::Status)
				.text()
				.not_null()
				.default("PENDING"),
			ColumnDef::new(BookClubInvitations::CreatedAt)
				.timestamp_with_time_zone()
				.not_null()
				.default("1970-01-01T00:00:00Z"),
			ColumnDef::new(BookClubInvitations::ExpiresAt).timestamp_with_time_zone(),
			ColumnDef::new(BookClubInvitations::CreatedByUserId).text(),
			ColumnDef::new(BookClubInvitations::AcceptedAt).timestamp_with_time_zone(),
			ColumnDef::new(BookClubInvitations::DeclinedAt).timestamp_with_time_zone(),
			ColumnDef::new(BookClubInvitations::RevokedAt).timestamp_with_time_zone(),
			ColumnDef::new(BookClubInvitations::RevokedByUserId).text(),
			ColumnDef::new(BookClubInvitations::AuditNote).text(),
		] {
			manager
				.alter_table(
					Table::alter()
						.table(BookClubInvitations::Table)
						.add_column(column)
						.to_owned(),
				)
				.await?;
		}
		// SQLite requires a constant default for an ADD COLUMN. Existing rows
		// receive a real migration-time timestamp; new model inserts set now.
		let conn = manager.get_connection();
		let backend = conn.get_database_backend();
		conn.execute(Statement::from_string(
			backend,
			"UPDATE book_club_invitations \
			 SET created_at = CURRENT_TIMESTAMP \
			 WHERE created_at = '1970-01-01T00:00:00Z'"
				.to_owned(),
		))
		.await?;
		conn.execute(Statement::from_string(
			backend,
			"CREATE UNIQUE INDEX IF NOT EXISTS uq_book_club_invitations_live_recipient \
			 ON book_club_invitations(book_club_id, user_id) \
			 WHERE status IN ('PENDING', 'ACTIVE')"
				.to_owned(),
		))
		.await?;

		manager
			.create_table(
				Table::create()
					.table(SocialRecommendations::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(SocialRecommendations::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::SenderUserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::RecipientUserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::TargetKind)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(SocialRecommendations::MediaId).text())
					.col(ColumnDef::new(SocialRecommendations::WorkId).text())
					.col(ColumnDef::new(SocialRecommendations::SourceProvider).text())
					.col(ColumnDef::new(SocialRecommendations::RemoteId).text())
					.col(ColumnDef::new(SocialRecommendations::ExternalKey).text())
					.col(
						ColumnDef::new(SocialRecommendations::Title)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::Authors)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(SocialRecommendations::CoverUrl).text())
					.col(ColumnDef::new(SocialRecommendations::Message).text())
					.col(
						ColumnDef::new(SocialRecommendations::State)
							.text()
							.not_null()
							.default("PENDING"),
					)
					.col(
						ColumnDef::new(SocialRecommendations::HandoffState)
							.text()
							.not_null()
							.default("NONE"),
					)
					.col(ColumnDef::new(SocialRecommendations::RequestId).text())
					.col(ColumnDef::new(SocialRecommendations::DestinationShelfId).text())
					.col(
						ColumnDef::new(SocialRecommendations::DestinationDeviceId).text(),
					)
					.col(ColumnDef::new(SocialRecommendations::IdempotencyKey).text())
					.col(
						ColumnDef::new(SocialRecommendations::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(SocialRecommendations::ExpiresAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::AcceptedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::DeclinedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::RevokedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialRecommendations::DismissedAt)
							.timestamp_with_time_zone(),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-recommendations-sender")
							.from(
								SocialRecommendations::Table,
								SocialRecommendations::SenderUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-recommendations-recipient")
							.from(
								SocialRecommendations::Table,
								SocialRecommendations::RecipientUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.check(Expr::cust("sender_user_id <> recipient_user_id"))
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-social-recommendations-idempotency")
					.table(SocialRecommendations::Table)
					.col(SocialRecommendations::SenderUserId)
					.col(SocialRecommendations::RecipientUserId)
					.col(SocialRecommendations::IdempotencyKey)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(SocialShareGrants::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(SocialShareGrants::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(SocialShareGrants::TargetKey)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(SocialShareGrants::TargetMediaId).text())
					.col(ColumnDef::new(SocialShareGrants::TargetWorkId).text())
					.col(ColumnDef::new(SocialShareGrants::SourceProvider).text())
					.col(ColumnDef::new(SocialShareGrants::RemoteId).text())
					.col(ColumnDef::new(SocialShareGrants::ExternalKey).text())
					.col(ColumnDef::new(SocialShareGrants::Title).text().not_null())
					.col(ColumnDef::new(SocialShareGrants::Authors).text().not_null())
					.col(ColumnDef::new(SocialShareGrants::CoverUrl).text())
					.col(
						ColumnDef::new(SocialShareGrants::SourceUserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialShareGrants::RecipientUserId)
							.text()
							.not_null(),
					)
					.col(ColumnDef::new(SocialShareGrants::RecommendationId).text())
					.col(
						ColumnDef::new(SocialShareGrants::ScopeMask)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialShareGrants::State)
							.text()
							.not_null()
							.default("PENDING"),
					)
					.col(ColumnDef::new(SocialShareGrants::IdempotencyKey).text())
					.col(
						ColumnDef::new(SocialShareGrants::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(
						ColumnDef::new(SocialShareGrants::ExpiresAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialShareGrants::AcceptedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialShareGrants::DeclinedAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialShareGrants::RevokedAt)
							.timestamp_with_time_zone(),
					)
					.col(ColumnDef::new(SocialShareGrants::RevokedByUserId).text())
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-share-grants-source")
							.from(
								SocialShareGrants::Table,
								SocialShareGrants::SourceUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-share-grants-recipient")
							.from(
								SocialShareGrants::Table,
								SocialShareGrants::RecipientUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-share-grants-recommendation")
							.from(
								SocialShareGrants::Table,
								SocialShareGrants::RecommendationId,
							)
							.to(SocialRecommendations::Table, SocialRecommendations::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::SetNull),
					)
					.check(Expr::cust("source_user_id <> recipient_user_id"))
					.to_owned(),
			)
			.await?;
		manager
			.create_index(
				Index::create()
					.name("uq-social-share-grants-idempotency")
					.table(SocialShareGrants::Table)
					.col(SocialShareGrants::SourceUserId)
					.col(SocialShareGrants::RecipientUserId)
					.col(SocialShareGrants::TargetKey)
					.col(SocialShareGrants::IdempotencyKey)
					.unique()
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(SocialShareOverlays::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(SocialShareOverlays::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(ColumnDef::new(SocialShareOverlays::GrantId).text().not_null())
					.col(ColumnDef::new(SocialShareOverlays::SourceUserId).text().not_null())
					.col(ColumnDef::new(SocialShareOverlays::TargetKey).text().not_null())
					.col(ColumnDef::new(SocialShareOverlays::Kind).text().not_null())
					.col(ColumnDef::new(SocialShareOverlays::Locator).json())
					.col(ColumnDef::new(SocialShareOverlays::Progression).double())
					.col(ColumnDef::new(SocialShareOverlays::Percentage).integer())
					.col(ColumnDef::new(SocialShareOverlays::Excerpt).text())
					.col(ColumnDef::new(SocialShareOverlays::Body).text())
					.col(ColumnDef::new(SocialShareOverlays::Color).text())
					.col(
						ColumnDef::new(SocialShareOverlays::CapturedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.col(ColumnDef::new(SocialShareOverlays::Revision).integer().not_null().default(1))
					.col(ColumnDef::new(SocialShareOverlays::TombstonedAt).timestamp_with_time_zone())
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-share-overlays-grant")
							.from(SocialShareOverlays::Table, SocialShareOverlays::GrantId)
							.to(SocialShareGrants::Table, SocialShareGrants::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-share-overlays-source")
							.from(SocialShareOverlays::Table, SocialShareOverlays::SourceUserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.check(Expr::cust("kind IN ('HIGHLIGHT', 'NOTE', 'BOOKMARK')"))
					.check(Expr::cust("color IS NULL OR color IN ('yellow', 'green', 'blue', 'pink', 'purple', 'orange')"))
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(SocialShareOverlayPreferences::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(SocialShareOverlayPreferences::RecipientUserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialShareOverlayPreferences::OverlayId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialShareOverlayPreferences::HiddenAt)
							.timestamp_with_time_zone(),
					)
					.col(
						ColumnDef::new(SocialShareOverlayPreferences::RestoredAt)
							.timestamp_with_time_zone(),
					)
					.primary_key(
						Index::create()
							.col(SocialShareOverlayPreferences::RecipientUserId)
							.col(SocialShareOverlayPreferences::OverlayId),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-share-overlay-preferences-recipient")
							.from(
								SocialShareOverlayPreferences::Table,
								SocialShareOverlayPreferences::RecipientUserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-share-overlay-preferences-overlay")
							.from(
								SocialShareOverlayPreferences::Table,
								SocialShareOverlayPreferences::OverlayId,
							)
							.to(SocialShareOverlays::Table, SocialShareOverlays::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(SocialRecommendationSignals::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(SocialRecommendationSignals::Id)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(SocialRecommendationSignals::UserId)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendationSignals::TargetKey)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendationSignals::SignalKind)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendationSignals::Weight)
							.integer()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendationSignals::RecommendationId)
							.text(),
					)
					.col(
						ColumnDef::new(SocialRecommendationSignals::ReasonCode)
							.text()
							.not_null(),
					)
					.col(
						ColumnDef::new(SocialRecommendationSignals::CreatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-signals-user")
							.from(
								SocialRecommendationSignals::Table,
								SocialRecommendationSignals::UserId,
							)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-signals-recommendation")
							.from(
								SocialRecommendationSignals::Table,
								SocialRecommendationSignals::RecommendationId,
							)
							.to(SocialRecommendations::Table, SocialRecommendations::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::SetNull),
					)
					.to_owned(),
			)
			.await?;

		manager
			.create_table(
				Table::create()
					.table(SocialPreferences::Table)
					.if_not_exists()
					.col(
						ColumnDef::new(SocialPreferences::UserId)
							.text()
							.not_null()
							.primary_key(),
					)
					.col(
						ColumnDef::new(SocialPreferences::RecommendationsOptOut)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(SocialPreferences::SharingOptOut)
							.boolean()
							.not_null()
							.default(false),
					)
					.col(
						ColumnDef::new(SocialPreferences::UpdatedAt)
							.timestamp_with_time_zone()
							.not_null()
							.default(Expr::current_timestamp()),
					)
					.foreign_key(
						ForeignKey::create()
							.name("fk-social-preferences-user")
							.from(SocialPreferences::Table, SocialPreferences::UserId)
							.to(Users::Table, Users::Id)
							.on_update(ForeignKeyAction::Cascade)
							.on_delete(ForeignKeyAction::Cascade),
					)
					.to_owned(),
			)
			.await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.drop_table(
				Table::drop()
					.table(SocialPreferences::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(SocialRecommendationSignals::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(SocialShareOverlayPreferences::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(SocialShareOverlays::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(SocialShareGrants::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_table(
				Table::drop()
					.table(SocialRecommendations::Table)
					.if_exists()
					.to_owned(),
			)
			.await?;
		manager
			.drop_index(
				Index::drop()
					.name("uq_book_club_invitations_live_recipient")
					.table(BookClubInvitations::Table)
					.to_owned(),
			)
			.await?;
		for column in [
			BookClubInvitations::AuditNote,
			BookClubInvitations::RevokedByUserId,
			BookClubInvitations::RevokedAt,
			BookClubInvitations::DeclinedAt,
			BookClubInvitations::AcceptedAt,
			BookClubInvitations::CreatedByUserId,
			BookClubInvitations::ExpiresAt,
			BookClubInvitations::CreatedAt,
			BookClubInvitations::Status,
		] {
			manager
				.alter_table(
					Table::alter()
						.table(BookClubInvitations::Table)
						.drop_column(column)
						.to_owned(),
				)
				.await?;
		}
		Ok(())
	}
}

#[derive(DeriveIden)]
enum BookClubInvitations {
	#[sea_orm(iden = "book_club_invitations")]
	Table,
	Status,
	CreatedAt,
	ExpiresAt,
	CreatedByUserId,
	AcceptedAt,
	DeclinedAt,
	RevokedAt,
	RevokedByUserId,
	AuditNote,
}

#[derive(DeriveIden)]
enum Users {
	#[sea_orm(iden = "users")]
	Table,
	Id,
}

#[derive(DeriveIden)]
enum SocialRecommendations {
	Table,
	Id,
	SenderUserId,
	RecipientUserId,
	TargetKind,
	MediaId,
	WorkId,
	SourceProvider,
	RemoteId,
	ExternalKey,
	Title,
	Authors,
	CoverUrl,
	Message,
	State,
	HandoffState,
	RequestId,
	DestinationShelfId,
	DestinationDeviceId,
	IdempotencyKey,
	CreatedAt,
	ExpiresAt,
	AcceptedAt,
	DeclinedAt,
	RevokedAt,
	DismissedAt,
}

#[derive(DeriveIden)]
enum SocialShareGrants {
	Table,
	Id,
	TargetKey,
	TargetMediaId,
	TargetWorkId,
	SourceProvider,
	RemoteId,
	ExternalKey,
	Title,
	Authors,
	CoverUrl,
	SourceUserId,
	RecipientUserId,
	RecommendationId,
	ScopeMask,
	State,
	IdempotencyKey,
	CreatedAt,
	ExpiresAt,
	AcceptedAt,
	DeclinedAt,
	RevokedAt,
	RevokedByUserId,
}

#[derive(DeriveIden)]
enum SocialShareOverlays {
	Table,
	Id,
	GrantId,
	SourceUserId,
	TargetKey,
	Kind,
	Locator,
	Progression,
	Percentage,
	Excerpt,
	Body,
	Color,
	CapturedAt,
	Revision,
	TombstonedAt,
}

#[derive(DeriveIden)]
enum SocialShareOverlayPreferences {
	Table,
	RecipientUserId,
	OverlayId,
	HiddenAt,
	RestoredAt,
}

#[derive(DeriveIden)]
enum SocialRecommendationSignals {
	Table,
	Id,
	UserId,
	TargetKey,
	SignalKind,
	Weight,
	RecommendationId,
	ReasonCode,
	CreatedAt,
}

#[derive(DeriveIden)]
enum SocialPreferences {
	Table,
	UserId,
	RecommendationsOptOut,
	SharingOptOut,
	UpdatedAt,
}
