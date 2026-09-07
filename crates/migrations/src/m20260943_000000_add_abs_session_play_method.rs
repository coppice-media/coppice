//! `abs_sessions.play_method`: how the listening in one Audiobookshelf
//! playback session reached the listener.
//!
//! Audiobookshelf numbers the four ways a session can be served — `0`
//! DirectPlay, `1` DirectStream, `2` Transcode, `3` Local — and reports the
//! number on every session it lists (`GET /api/me/listening-sessions`). Two
//! of them are reachable here: a session opened by
//! `POST /api/items/{id}/play` is served from the stored tracks (`0`), while
//! `POST /api/session/local` and `/local-all` upload sessions the official
//! app recorded **offline**, from its own downloaded copy (`3`).
//!
//! Without the column the listening history cannot tell offline listening
//! apart from streamed listening, so it would report every uploaded session
//! as if the server had served the bytes. The default is `0`: every row that
//! predates this migration was opened by a play request.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(AbsSessions::Table)
					.add_column(
						ColumnDef::new(AbsSessions::PlayMethod)
							.integer()
							.not_null()
							.default(0),
					)
					.to_owned(),
			)
			.await
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager
			.alter_table(
				Table::alter()
					.table(AbsSessions::Table)
					.drop_column(AbsSessions::PlayMethod)
					.to_owned(),
			)
			.await
	}
}

#[derive(DeriveIden)]
enum AbsSessions {
	#[sea_orm(iden = "abs_sessions")]
	Table,
	PlayMethod,
}
