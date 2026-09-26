use async_graphql::{Context, Object, Result, ID};
use chrono::Utc;
use metadata_integrations::{HardcoverClient, HardcoverJournalEntry, MetadataProvider};
use models::{
	domain::reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	entity::{
		hardcover_connection, hardcover_journal_provenance, hardcover_media_link,
		hardcover_metadata_cache, media, media_annotation,
	},
	services::reading_state,
	shared::readium::{ReadiumLocation, ReadiumLocator, ReadiumText},
};
use sea_orm::{
	prelude::DateTimeWithTimeZone, ActiveModelTrait, ColumnTrait, EntityTrait,
	IntoActiveModel, NotSet, QueryFilter, Set,
};
use serde_json::json;
use stump_core::utils::encryption::{decrypt_string, encrypt_string};

use crate::{
	data::CoreContext,
	object::hardcover::{
		HardcoverConnection, HardcoverMediaLink, HardcoverMetadataLookup,
		HardcoverSyncResult,
	},
};
#[derive(Default)]
pub struct HardcoverMutation;

#[Object]
impl HardcoverMutation {
	/// Verifies a personal PAT with `me` before storing it encrypted. The token
	/// is never returned or included in a job payload.
	async fn connect_hardcover(
		&self,
		ctx: &Context<'_>,
		api_token: String,
		use_for_metadata: Option<bool>,
		import_journals: Option<bool>,
		sync_progress: Option<bool>,
	) -> Result<HardcoverConnection> {
		let token = api_token.trim();
		if token.is_empty() {
			return Err("Hardcover API token cannot be blank".into());
		}
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let client = HardcoverClient::new(token.to_owned(), None);
		let identity = client
			.verify_identity()
			.await
			.map_err(public_provider_error)?;
		let capabilities = client
			.inspect_capabilities()
			.await
			.unwrap_or_else(|_| vec!["me".to_owned(), "metadata".to_owned()]);
		let key = core.get_encryption_key().await?;
		let encrypted = encrypt_string(token, &key)?;
		let conn = core.conn.as_ref();
		let now: DateTimeWithTimeZone = Utc::now().into();
		let existing = hardcover_connection::Entity::find_by_id(user.id.clone())
			.one(conn)
			.await?;
		let model = if let Some(existing) = existing {
			let credential_version = existing.credential_version + 1;
			let mut active = existing.into_active_model();
			active.encrypted_api_token = Set(encrypted);
			active.credential_version = Set(credential_version);
			active.remote_user_id = Set(identity.remote_user_id);
			active.remote_username = Set(identity.username);
			active.scopes = Set(Some(json!(["me", "metadata"])));
			active.capabilities = Set(Some(json!(capabilities)));
			active.use_for_metadata = Set(use_for_metadata.unwrap_or(true));
			active.import_journals = Set(import_journals.unwrap_or(false));
			active.sync_progress = Set(sync_progress.unwrap_or(false));
			active.verified_at = Set(Some(now.clone()));
			active.last_error = Set(None);
			active.updated_at = Set(Utc::now().into());
			active.update(conn).await?
		} else {
			hardcover_connection::ActiveModel {
				user_id: Set(user.id.clone()),
				encrypted_api_token: Set(encrypted),
				credential_version: Set(1),
				remote_user_id: Set(identity.remote_user_id),
				remote_username: Set(identity.username),
				scopes: Set(Some(json!(["me", "metadata"]))),
				capabilities: Set(Some(json!(capabilities))),
				use_for_metadata: Set(use_for_metadata.unwrap_or(true)),
				import_journals: Set(import_journals.unwrap_or(false)),
				sync_progress: Set(sync_progress.unwrap_or(false)),
				connected_at: Set(now),
				verified_at: Set(Some(Utc::now().into())),
				last_sync_at: Set(None),
				last_error: Set(None),
				updated_at: Set(Utc::now().into()),
			}
			.insert(conn)
			.await?
		};
		Ok(model.into())
	}

	async fn update_hardcover_connection(
		&self,
		ctx: &Context<'_>,
		use_for_metadata: bool,
		import_journals: bool,
		sync_progress: bool,
	) -> Result<HardcoverConnection> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let conn = hardcover_connection::Entity::find_by_id(user.id.clone())
			.one(core.conn.as_ref())
			.await?
			.ok_or("Hardcover is not connected")?;
		let mut active = conn.into_active_model();
		active.use_for_metadata = Set(use_for_metadata);
		active.import_journals = Set(import_journals);
		active.sync_progress = Set(sync_progress);
		Ok(active.update(core.conn.as_ref()).await?.into())
	}

	async fn disconnect_hardcover(&self, ctx: &Context<'_>) -> Result<bool> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let result = hardcover_connection::Entity::delete_by_id(user.id.clone())
			.exec(core.conn.as_ref())
			.await?;
		Ok(result.rows_affected > 0)
	}

	async fn link_hardcover_media(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
		remote_id: String,
	) -> Result<HardcoverMediaLink> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let remote_id = remote_id.trim().to_owned();
		if remote_id.is_empty() {
			return Err("Hardcover remote id cannot be blank".into());
		}
		let local = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id.to_string()))
			.one(core.conn.as_ref())
			.await?
			.ok_or("Media not found")?;
		let conn = core.conn.as_ref();
		hardcover_media_link::Entity::delete_many()
			.filter(hardcover_media_link::Column::UserId.eq(&user.id))
			.filter(hardcover_media_link::Column::MediaId.eq(&local.id))
			.exec(conn)
			.await?;
		let model = hardcover_media_link::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			user_id: Set(user.id.clone()),
			media_id: Set(local.id),
			remote_id: Set(remote_id),
			remote_title: Set(None),
			linked_at: Set(Utc::now().into()),
			updated_at: Set(Utc::now().into()),
		}
		.insert(conn)
		.await?;
		Ok(model.into())
	}

	async fn unlink_hardcover_media(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
	) -> Result<bool> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let result = hardcover_media_link::Entity::delete_many()
			.filter(hardcover_media_link::Column::UserId.eq(&user.id))
			.filter(hardcover_media_link::Column::MediaId.eq(media_id.to_string()))
			.exec(core.conn.as_ref())
			.await?;
		Ok(result.rows_affected > 0)
	}
	/// Looks up public metadata through the current user's Hardcover PAT only
	/// when that user enabled the interactive metadata toggle. The persistent
	/// cache contains public payloads keyed by provider/query/schema/TTL and
	/// never stores account, journal, or progress responses.
	async fn lookup_hardcover_metadata(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
	) -> Result<Option<HardcoverMetadataLookup>> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let db = core.conn.as_ref();
		let connection = hardcover_connection::Entity::find_by_id(user.id.clone())
			.one(db)
			.await?;
		let Some(connection) = connection else {
			return Ok(None);
		};
		if !connection.use_for_metadata {
			return Ok(None);
		}
		let link = hardcover_media_link::Entity::find()
			.filter(hardcover_media_link::Column::UserId.eq(&user.id))
			.filter(hardcover_media_link::Column::MediaId.eq(media_id.to_string()))
			.one(db)
			.await?;
		let Some(link) = link else {
			return Ok(None);
		};
		let now = Utc::now();
		let cached = hardcover_metadata_cache::Entity::find_by_id((
			"hardcover".to_owned(),
			link.remote_id.clone(),
			"1".to_owned(),
		))
		.one(db)
		.await?
		.filter(|row| row.expires_at.to_utc() > now);
		let payload = if let Some(cached) = cached {
			cached.payload
		} else {
			let key = core.get_encryption_key().await?;
			let token = decrypt_string(&connection.encrypted_api_token, &key)?;
			let client = HardcoverClient::new(token, None);
			let metadata = client
				.fetch_media_metadata(&link.remote_id)
				.await
				.map_err(public_provider_error)?;
			let payload = serde_json::to_value(&metadata)?;
			hardcover_metadata_cache::ActiveModel {
				provider: Set("hardcover".to_owned()),
				query_key: Set(link.remote_id.clone()),
				schema_version: Set("1".to_owned()),
				payload: Set(payload.clone()),
				expires_at: Set((now + chrono::Duration::hours(24)).into()),
				created_at: Set(now.into()),
			}
			.insert(db)
			.await?;
			payload
		};
		let metadata: metadata_integrations::ExternalMediaMetadata =
			serde_json::from_value(payload)?;
		Ok(Some(HardcoverMetadataLookup {
			provider: metadata.provider,
			remote_id: metadata.external_id,
			title: metadata.title,
			summary: metadata.summary,
			writers: metadata.writers.unwrap_or_default(),
			year: metadata.year,
			page_count: metadata.page_count,
			cover_url: metadata.cover_url,
			provider_url: metadata.provider_url,
		}))
	}

	/// Runs one explicit, user-scoped synchronization. No scheduler calls this
	/// continuously: the caller opts in through the manual mutation and the
	/// connection toggles. Quotes are imported only with an exact local link and
	/// page locator; all other records remain unresolved provenance.
	async fn sync_hardcover_now(&self, ctx: &Context<'_>) -> Result<HardcoverSyncResult> {
		let core = ctx.data::<CoreContext>()?;
		let user = &ctx.data::<stump_auth::AuthContext>()?.user;
		let db = core.conn.as_ref();
		let row = hardcover_connection::Entity::find_by_id(user.id.clone())
			.one(db)
			.await?
			.ok_or("Hardcover is not connected")?;
		let key = core.get_encryption_key().await?;
		let token = decrypt_string(&row.encrypted_api_token, &key)?;
		let client = HardcoverClient::new(token, None);
		let identity = match client.verify_identity().await {
			Ok(identity) => identity,
			Err(error) => {
				let message = provider_error_message(&error);
				let mut active = row.into_active_model();
				active.last_error = Set(Some(message.clone()));
				active.update(db).await?;
				return Ok(HardcoverSyncResult {
					status: "error".to_owned(),
					imported: 0,
					unresolved: 0,
					projected: 0,
					skipped: 0,
					last_sync_at: None,
					error: Some(message),
				});
			},
		};
		let capabilities = client
			.inspect_capabilities()
			.await
			.unwrap_or_else(|_| vec!["me".to_owned(), "metadata".to_owned()]);
		let import_journals = row.import_journals;
		let sync_progress = row.sync_progress;
		let mut active = row.into_active_model();
		active.remote_user_id = Set(identity.remote_user_id);
		active.remote_username = Set(identity.username);
		active.capabilities = Set(Some(json!(capabilities.clone())));
		active.verified_at = Set(Some(Utc::now().into()));
		active.last_error = Set(None);
		let settings = (import_journals, sync_progress);
		let entries = if settings.0 {
			if !capabilities
				.iter()
				.any(|capability| capability == "user_books")
			{
				Vec::new()
			} else {
				client.fetch_journal_entries().await.unwrap_or_default()
			}
		} else {
			Vec::new()
		};
		let mut imported = 0;
		let mut unresolved = 0;
		let mut projected = 0;
		let mut skipped = 0;
		for entry in entries {
			let existing = hardcover_journal_provenance::Entity::find()
				.filter(hardcover_journal_provenance::Column::UserId.eq(&user.id))
				.filter(
					hardcover_journal_provenance::Column::RemoteEntryId
						.eq(&entry.remote_entry_id),
				)
				.one(db)
				.await?;
			if existing.is_some() {
				skipped += 1;
				continue;
			}
			let link = entry.remote_book_id.as_deref().map(|remote_id| async move {
				hardcover_media_link::Entity::find()
					.filter(hardcover_media_link::Column::UserId.eq(&user.id))
					.filter(hardcover_media_link::Column::RemoteId.eq(remote_id))
					.one(db)
					.await
			});
			let link = match link {
				Some(future) => future.await?,
				None => None,
			};
			let Some(link) = link else {
				insert_unresolved(db, user.id.as_str(), &entry, "no exact media link")
					.await?;
				unresolved += 1;
				continue;
			};
			let Some(page) = entry.page.filter(|page| *page > 0) else {
				insert_unresolved(
					db,
					user.id.as_str(),
					&entry,
					"missing exact page locator",
				)
				.await?;
				unresolved += 1;
				continue;
			};
			let local = media::Entity::find_by_id(link.media_id.clone())
				.one(db)
				.await?
				.ok_or("linked media disappeared")?;
			if local.pages <= 0 || page > local.pages {
				insert_unresolved(
					db,
					user.id.as_str(),
					&entry,
					"page is outside local publication",
				)
				.await?;
				unresolved += 1;
				continue;
			}
			if let Some(text) = entry.quote.clone().or(entry.note.clone()) {
				let locator = ReadiumLocator {
					chapter_title: String::new(),
					href: format!("hardcover://{}", entry.remote_entry_id),
					title: None,
					locations: Some(ReadiumLocation {
						fragments: None,
						// Page is the exact, durable locator. A remote
						// progression is retained in provenance and is
						// never guessed into a Readium decimal.
						progression: None,
						position: Some(page),
						total_progression: None,
						css_selector: None,
						partial_cfi: None,
					}),
					text: Some(ReadiumText {
						after: None,
						before: None,
						highlight: Some(text.clone()),
					}),
					kobo_span: None,
					r#type: "application/xhtml+xml".to_owned(),
				};
				let annotation = media_annotation::ActiveModel {
					id: NotSet,
					locator: Set(locator),
					annotation_text: Set(entry.note.clone()),
					color: Set(None),
					media_id: Set(local.id.clone()),
					user_id: Set(user.id.clone()),
					created_at: NotSet,
					updated_at: NotSet,
				}
				.insert(db)
				.await?;
				insert_imported(db, user.id.as_str(), &entry, &local.id, &annotation.id)
					.await?;
				imported += 1;
			} else {
				insert_unresolved(
					db,
					user.id.as_str(),
					&entry,
					"remote entry has no quote or note",
				)
				.await?;
				unresolved += 1;
			}
			if settings.1 {
				let progression = entry
					.progression
					.filter(|progression| {
						progression.is_finite() && (0.0..=1.0).contains(progression)
					})
					.or_else(|| Some(page as f64 / local.pages as f64));
				if let Some(progression) = progression {
					let applied = reading_state::apply(
						db,
						&user.id,
						Publication::from(&local),
						ProtocolUpdate {
							protocol: SourceProtocol::Stump,
							device_id: None,
							updated_at: None,
							position: Position::Page(page),
							progression: Some(progression),
							completed: None,
							raw_payload: entry.raw.clone(),
						},
					)
					.await?;
					if applied.accepted() {
						projected += 1;
					}
				}
			}
		}
		let now = Utc::now();
		active.last_sync_at = Set(Some(now.into()));
		active.updated_at = Set(now.into());
		active.update(db).await?;
		Ok(HardcoverSyncResult {
			status: "ok".to_owned(),
			imported,
			unresolved,
			projected,
			skipped,
			last_sync_at: Some(now.into()),
			error: None,
		})
	}
}

async fn insert_unresolved(
	conn: &sea_orm::DatabaseConnection,
	user_id: &str,
	entry: &HardcoverJournalEntry,
	reason: &str,
) -> Result<(), sea_orm::DbErr> {
	hardcover_journal_provenance::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		user_id: Set(user_id.to_owned()),
		remote_entry_id: Set(entry.remote_entry_id.clone()),
		media_id: Set(None),
		local_annotation_id: Set(None),
		locator_key: Set(entry.page.map(|page| page.to_string())),
		status: Set("unresolved".to_owned()),
		unresolved_reason: Set(Some(reason.to_owned())),
		payload: Set(Some(entry.raw.clone())),
		created_at: Set(Utc::now().into()),
		updated_at: Set(Utc::now().into()),
	}
	.insert(conn)
	.await
	.map(|_| ())
}

async fn insert_imported(
	conn: &sea_orm::DatabaseConnection,
	user_id: &str,
	entry: &HardcoverJournalEntry,
	media_id: &str,
	annotation_id: &str,
) -> Result<(), sea_orm::DbErr> {
	hardcover_journal_provenance::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		user_id: Set(user_id.to_owned()),
		remote_entry_id: Set(entry.remote_entry_id.clone()),
		media_id: Set(Some(media_id.to_owned())),
		local_annotation_id: Set(Some(annotation_id.to_owned())),
		locator_key: Set(entry.page.map(|page| page.to_string())),
		status: Set("imported".to_owned()),
		unresolved_reason: Set(None),
		payload: Set(Some(entry.raw.clone())),
		created_at: Set(Utc::now().into()),
		updated_at: Set(Utc::now().into()),
	}
	.insert(conn)
	.await
	.map(|_| ())
}
fn public_provider_error(
	error: metadata_integrations::MetadataProviderError,
) -> async_graphql::Error {
	async_graphql::Error::new(provider_error_message(&error))
}

fn provider_error_message(
	error: &metadata_integrations::MetadataProviderError,
) -> String {
	let message = error.to_string();
	let lower = message.to_ascii_lowercase();
	let class = if lower.contains("401") || lower.contains("unauthorized") {
		"Hardcover rejected the credential (401)"
	} else if lower.contains("403") || lower.contains("forbidden") {
		"Hardcover denied this capability (403)"
	} else if lower.contains("408") || lower.contains("timeout") {
		"Hardcover timed out (408)"
	} else if lower.contains("429") || lower.contains("rate") {
		"Hardcover rate limited the request (429)"
	} else if lower.contains("500")
		|| lower.contains("502")
		|| lower.contains("503")
		|| lower.contains("504")
	{
		"Hardcover is temporarily unavailable (5xx)"
	} else {
		"Hardcover request failed"
	};
	class.to_owned()
}
