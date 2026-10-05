use crate::{
	data::CoreContext,
	object::hardcover::{
		HardcoverConnection, HardcoverMediaLink, HardcoverMetadataLookup,
		HardcoverSyncResult,
	},
};
use async_graphql::{Context, Object, Result, ID};
use chrono::{DateTime, Utc};
use metadata_integrations::{HardcoverClient, HardcoverJournalEntry, MetadataProvider};
use models::{
	domain::reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	entity::{
		hardcover_connection, hardcover_journal_provenance, hardcover_media_link,
		hardcover_metadata_cache, media,
	},
	services::reading_state,
	txn::begin_write,
};
use sea_orm::{
	prelude::DateTimeWithTimeZone, ActiveModelTrait, ColumnTrait, ConnectionTrait,
	EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde_json::json;
use stump_core::utils::encryption::{decrypt_string, encrypt_string};
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
		let capabilities = client.inspect_capabilities().await.ok();
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
			active.capabilities = Set(capabilities.clone().map(|value| json!(value)));
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
				capabilities: Set(capabilities.map(|value| json!(value))),
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

	/// Runs one explicit, user-scoped read-only synchronization. Journal
	/// quotes are retained as provenance: Hardcover page numbers cannot
	/// identify an EPUB resource/CFI, so no annotation is manufactured.
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
		sync_with_client(db, &user.id, row, &client).await
	}
}

async fn sync_with_client(
	db: &sea_orm::DatabaseConnection,
	user_id: &str,
	row: hardcover_connection::Model,
	client: &HardcoverClient,
) -> Result<HardcoverSyncResult> {
	let previous_sync_at = row.last_sync_at;
	let import_journals = row.import_journals;
	let sync_progress = row.sync_progress;
	let mut active = row.into_active_model();
	let identity = match client.verify_identity().await {
		Ok(identity) => identity,
		Err(error) => {
			return sync_failure(
				db,
				active,
				previous_sync_at,
				format!(
					"Hardcover identity verification failed: {}",
					provider_error_message(&error)
				),
			)
			.await;
		},
	};
	active.remote_user_id = Set(identity.remote_user_id.clone());
	active.remote_username = Set(identity.username);
	active.verified_at = Set(Some(Utc::now().into()));

	let entries = if import_journals {
		let capabilities = match client.inspect_capabilities().await {
			Ok(capabilities) => capabilities,
			Err(error) => {
				return sync_failure(
					db,
					active,
					previous_sync_at,
					format!(
						"Hardcover capability check failed: {}",
						provider_error_message(&error)
					),
				)
				.await;
			},
		};
		active.capabilities = Set(Some(json!(capabilities)));
		if !capabilities.iter().any(|name| name == "reading_journals") {
			return sync_failure(
				db,
				active,
				previous_sync_at,
				"Hardcover does not advertise the reading_journals capability".into(),
			)
			.await;
		}
		let Some(remote_user_id) = identity.remote_user_id.as_deref() else {
			return sync_failure(
				db,
				active,
				previous_sync_at,
				"Hardcover did not return a user id for journal access".into(),
			)
			.await;
		};
		match client.fetch_journal_entries(remote_user_id).await {
			Ok(entries) => entries,
			Err(error) => {
				return sync_failure(
					db,
					active,
					previous_sync_at,
					format!(
						"Hardcover journal fetch failed: {}",
						provider_error_message(&error)
					),
				)
				.await;
			},
		}
	} else {
		Vec::new()
	};
	// An upstream page failure above must never leave a partially imported
	// journal. A local persistence failure below rolls back all new records.
	let transaction = begin_write(db).await?;
	let mut unresolved = 0;
	let mut projected = 0;
	let mut skipped = 0;
	for entry in entries {
		let existing = hardcover_journal_provenance::Entity::find()
			.filter(hardcover_journal_provenance::Column::UserId.eq(user_id))
			.filter(
				hardcover_journal_provenance::Column::RemoteEntryId
					.eq(&entry.remote_entry_id),
			)
			.one(&transaction)
			.await?;
		if existing.is_some() {
			skipped += 1;
			continue;
		}
		let link = if let Some(remote_id) = entry.remote_book_id.as_deref() {
			hardcover_media_link::Entity::find()
				.filter(hardcover_media_link::Column::UserId.eq(user_id))
				.filter(hardcover_media_link::Column::RemoteId.eq(remote_id))
				.one(&transaction)
				.await?
		} else {
			None
		};
		let reason = if link.is_none() {
			"no exact media link"
		} else if entry.quote.is_none() && entry.note.is_none() {
			"journal entry has no quote text"
		} else {
			"no exact publication resource locator"
		};
		insert_unresolved(
			&transaction,
			user_id,
			&entry,
			link.as_ref().map(|link| link.media_id.as_str()),
			reason,
		)
		.await?;
		unresolved += 1;

		// `sync_progress` is local-only and depends on `import_journals`.
		// A quote's metadata.position is an edition-relative *fraction*,
		// never a local page. An absent timestamp/position cannot move a head.
		if !sync_progress {
			continue;
		}
		let Some(link) = link else { continue };
		let Some(progression) = entry.progression else {
			continue;
		};
		let Some(source_time) = entry
			.raw
			.get("action_at")
			.and_then(serde_json::Value::as_str)
			.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
			.map(|value| value.to_utc())
		else {
			continue;
		};
		// Reject future source times: they would pin the head past subsequent
		// local updates, which reading-state conflict resolution treats as stale.
		if source_time > Utc::now() {
			continue;
		}
		let local = media::Entity::find_by_id(&link.media_id)
			.one(&transaction)
			.await?
			.ok_or("linked media disappeared")?;
		if reading_state::head(&transaction, user_id, &local.id)
			.await?
			.is_some_and(|head| source_time <= head.updated_at.to_utc())
		{
			continue;
		}
		let applied = reading_state::apply(
			&transaction,
			user_id,
			Publication::from(&local),
			ProtocolUpdate {
				protocol: SourceProtocol::Stump,
				device_id: None,
				updated_at: Some(source_time),
				position: Position::None,
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
	let now = Utc::now();
	active.last_sync_at = Set(Some(now.into()));
	active.last_error = Set(None);
	active.updated_at = Set(now.into());
	active.update(&transaction).await?;
	transaction.commit().await?;
	Ok(HardcoverSyncResult {
		status: "ok".to_owned(),
		imported: 0,
		unresolved,
		projected,
		skipped,
		last_sync_at: Some(now.into()),
		error: None,
	})
}

async fn sync_failure(
	db: &sea_orm::DatabaseConnection,
	mut active: hardcover_connection::ActiveModel,
	previous_sync_at: Option<DateTimeWithTimeZone>,
	message: String,
) -> Result<HardcoverSyncResult> {
	active.last_error = Set(Some(message.clone()));
	active.updated_at = Set(Utc::now().into());
	active.update(db).await?;
	Ok(HardcoverSyncResult {
		status: "error".to_owned(),
		imported: 0,
		unresolved: 0,
		projected: 0,
		skipped: 0,
		last_sync_at: previous_sync_at,
		error: Some(message),
	})
}

async fn insert_unresolved(
	conn: &impl ConnectionTrait,
	user_id: &str,
	entry: &HardcoverJournalEntry,
	media_id: Option<&str>,
	reason: &str,
) -> Result<(), sea_orm::DbErr> {
	hardcover_journal_provenance::ActiveModel {
		id: Set(uuid::Uuid::new_v4().to_string()),
		user_id: Set(user_id.to_owned()),
		remote_entry_id: Set(entry.remote_entry_id.clone()),
		media_id: Set(media_id.map(str::to_owned)),
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

#[cfg(test)]
mod tests {
	use super::*;
	use metadata_integrations::mock_http::{render_ok, MockServer};
	use sea_orm::{
		ActiveModelTrait, ConnectionTrait, DatabaseBackend, PaginatorTrait, Schema,
	};

	async fn setup_connection() -> (
		sea_orm::DatabaseConnection,
		String,
		hardcover_connection::Model,
	) {
		let db = ::tests::db::test_database().await;
		let schema = Schema::new(DatabaseBackend::Sqlite);
		for statement in [
			schema.create_table_from_entity(hardcover_connection::Entity),
			schema.create_table_from_entity(hardcover_media_link::Entity),
			schema.create_table_from_entity(hardcover_journal_provenance::Entity),
			schema.create_table_from_entity(models::entity::media_annotation::Entity),
		] {
			db.execute(db.get_database_backend().build(&statement))
				.await
				.unwrap();
		}
		let user = ::tests::fake_data::User::new("journal-import")
			.insert(&db)
			.await;
		let now = Utc::now().into();
		let row = hardcover_connection::ActiveModel {
			user_id: Set(user.id.clone()),
			encrypted_api_token: Set("test-cipher".to_owned()),
			credential_version: Set(1),
			use_for_metadata: Set(false),
			import_journals: Set(true),
			sync_progress: Set(true),
			connected_at: Set(now),
			updated_at: Set(now),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();
		(db, user.id, row)
	}

	fn identity() -> String {
		serde_json::json!({
			"data": { "me": [{ "id": 42, "username": "journal-import" }] }
		})
		.to_string()
	}

	fn capabilities(names: &[&str]) -> String {
		serde_json::json!({
			"data": { "__schema": { "queryType": { "fields":
				names.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>()
			} } }
		})
		.to_string()
	}

	fn client(server: &MockServer) -> HardcoverClient {
		HardcoverClient::new("test-token".into(), Some(u32::MAX)).pointed_at(&server.url)
	}

	#[tokio::test]
	async fn capability_and_journal_failures_preserve_last_success_time() {
		let (db, user_id, row) = setup_connection().await;
		let prior_sync_at: DateTimeWithTimeZone =
			(Utc::now() - chrono::Duration::days(1)).into();
		let mut active = row.into_active_model();
		active.last_sync_at = Set(Some(prior_sync_at));
		let row = active.update(&db).await.unwrap();
		let introspection_error = MockServer::spawn(vec![
			render_ok(&identity()),
			render_ok(r#"{"errors":[{"message":"capabilities denied"}]}"#),
		]);
		let result =
			sync_with_client(&db, &user_id, row.clone(), &client(&introspection_error))
				.await
				.unwrap();
		assert_eq!(result.status, "error");
		assert_eq!(result.imported, 0);
		assert!(result
			.error
			.as_deref()
			.unwrap()
			.contains("capability check failed"));
		assert_eq!(result.last_sync_at, Some(prior_sync_at));
		assert_eq!(introspection_error.requests().len(), 2);
		let saved = hardcover_connection::Entity::find_by_id(&user_id)
			.one(&db)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(saved.last_error.as_deref(), result.error.as_deref());
		assert_eq!(saved.last_sync_at, Some(prior_sync_at));

		let missing = MockServer::spawn(vec![
			render_ok(&identity()),
			render_ok(&capabilities(&["me", "user_books"])),
		]);
		let result = sync_with_client(&db, &user_id, saved.clone(), &client(&missing))
			.await
			.unwrap();
		assert_eq!(result.status, "error");
		assert!(result.error.unwrap().contains("reading_journals"));
		assert_eq!(missing.requests().len(), 2);

		let upstream_error = MockServer::spawn(vec![
			render_ok(&identity()),
			render_ok(&capabilities(&["me", "reading_journals"])),
			render_ok(r#"{"errors":[{"message":"journal denied"}]}"#),
		]);
		let result = sync_with_client(
			&db,
			&user_id,
			hardcover_connection::Entity::find_by_id(&user_id)
				.one(&db)
				.await
				.unwrap()
				.unwrap(),
			&client(&upstream_error),
		)
		.await
		.unwrap();
		assert_eq!(result.status, "error");
		assert!(result
			.error
			.as_deref()
			.unwrap()
			.contains("journal fetch failed"));
		assert_eq!(upstream_error.requests().len(), 3);
		assert_eq!(result.unresolved, 0);
		assert_eq!(
			hardcover_journal_provenance::Entity::find()
				.count(&db)
				.await
				.unwrap(),
			0
		);
		let saved = hardcover_connection::Entity::find_by_id(&user_id)
			.one(&db)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(saved.last_error.as_deref(), result.error.as_deref());
		assert_eq!(saved.last_sync_at, Some(prior_sync_at));
	}

	#[tokio::test]
	async fn quote_import_deduplicates_and_projects_only_dated_fractions() {
		let (db, user_id, row) = setup_connection().await;
		let library = ::tests::fake_data::Library::default().insert(&db).await;
		let series = ::tests::fake_data::Series {
			library_id: Some(library.id),
			..Default::default()
		}
		.insert(&db)
		.await;
		let media = ::tests::fake_data::Media {
			series_id: series.id,
			pages: Some(100),
			..Default::default()
		}
		.insert(&db)
		.await;
		hardcover_media_link::ActiveModel {
			id: Set(uuid::Uuid::new_v4().to_string()),
			user_id: Set(user_id.clone()),
			media_id: Set(media.id.clone()),
			remote_id: Set("91".to_owned()),
			linked_at: Set(Utc::now().into()),
			updated_at: Set(Utc::now().into()),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();
		// The old importer keyed user_books by a bare numeric id. A true
		// reading_journals row with the same id must not be skipped.
		insert_unresolved(
			&db,
			&user_id,
			&HardcoverJournalEntry {
				remote_entry_id: "100".to_owned(),
				remote_book_id: Some("91".to_owned()),
				quote: None,
				note: None,
				page: None,
				progression: None,
				raw: json!({ "id": 100, "source": "user_books" }),
			},
			None,
			"legacy user_books row",
		)
		.await
		.unwrap();
		let entries = serde_json::json!({
			"data": { "reading_journals": [
				{ "id": 100, "user_id": 42, "book_id": 91, "edition_id": 12,
				  "event": "quote", "entry": "Text\n━━━\nNote",
				  "action_at": "2026-10-04T19:00:00+00:00",
				  "metadata": { "position": { "type": "pages", "value": 150,
												  "possible": 600, "percent": 25 } } },
				{ "id": 101, "user_id": 42, "book_id": 91,
				  "event": "quote", "entry": null,
				  "action_at": "2026-10-04T20:00:00+00:00",
				  "metadata": { "position": { "type": "pages", "percent": 30 } } }
			] }
		})
		.to_string();
		let responses = || {
			vec![
				render_ok(&identity()),
				render_ok(&capabilities(&["reading_journals"])),
				render_ok(&entries),
			]
		};
		let first = MockServer::spawn(responses());
		let result = sync_with_client(&db, &user_id, row, &client(&first))
			.await
			.unwrap();
		assert_eq!(result.status, "ok");
		assert_eq!(
			(
				result.imported,
				result.unresolved,
				result.projected,
				result.skipped
			),
			(0, 2, 2, 0)
		);
		assert!(result.error.is_none());
		let provenance = hardcover_journal_provenance::Entity::find()
			.all(&db)
			.await
			.unwrap();
		assert_eq!(provenance.len(), 3);
		assert!(provenance
			.iter()
			.any(|entry| entry.remote_entry_id == "100"));
		assert!(provenance
			.iter()
			.filter(|entry| entry.remote_entry_id.starts_with("reading_journals:"))
			.all(|entry| {
				entry.status == "unresolved"
					&& entry.local_annotation_id.is_none()
					&& entry.media_id.as_deref() == Some(media.id.as_str())
			}));
		assert_eq!(
			models::entity::media_annotation::Entity::find()
				.count(&db)
				.await
				.unwrap(),
			0
		);
		let head = reading_state::head(&db, &user_id, &media.id)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(head.progression, 0.3);
		assert!(head.page.is_none() && head.locator.is_none());

		let repeated = MockServer::spawn(responses());
		let result = sync_with_client(
			&db,
			&user_id,
			hardcover_connection::Entity::find_by_id(&user_id)
				.one(&db)
				.await
				.unwrap()
				.unwrap(),
			&client(&repeated),
		)
		.await
		.unwrap();
		assert_eq!(
			(result.unresolved, result.projected, result.skipped),
			(0, 0, 2)
		);
		assert_eq!(
			hardcover_journal_provenance::Entity::find()
				.count(&db)
				.await
				.unwrap(),
			3
		);
		let old_quote = serde_json::json!({
			"data": { "reading_journals": [{
				"id": 102, "user_id": 42, "book_id": 91,
				"event": "quote", "entry": "An older quote",
				"action_at": "2026-10-03T00:00:00+00:00",
				"metadata": { "position": { "type": "pages", "percent": 95 } }
			}] }
		})
		.to_string();
		let stale = MockServer::spawn(vec![
			render_ok(&identity()),
			render_ok(&capabilities(&["reading_journals"])),
			render_ok(&old_quote),
		]);
		let result = sync_with_client(
			&db,
			&user_id,
			hardcover_connection::Entity::find_by_id(&user_id)
				.one(&db)
				.await
				.unwrap()
				.unwrap(),
			&client(&stale),
		)
		.await
		.unwrap();
		assert_eq!((result.unresolved, result.projected), (1, 0));
		let head = reading_state::head(&db, &user_id, &media.id)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(head.progression, 0.3, "old quotes never move a newer head");
		let future_action_at = (Utc::now() + chrono::Duration::days(1)).to_rfc3339();
		let future_quote = serde_json::json!({
			"data": { "reading_journals": [{
				"id": 103, "user_id": 42, "book_id": 91,
				"event": "quote", "entry": "A future-dated quote",
				"action_at": future_action_at,
				"metadata": { "position": { "type": "pages", "percent": 90 } }
			}] }
		})
		.to_string();
		let future = MockServer::spawn(vec![
			render_ok(&identity()),
			render_ok(&capabilities(&["reading_journals"])),
			render_ok(&future_quote),
		]);
		let result = sync_with_client(
			&db,
			&user_id,
			hardcover_connection::Entity::find_by_id(&user_id)
				.one(&db)
				.await
				.unwrap()
				.unwrap(),
			&client(&future),
		)
		.await
		.unwrap();
		assert_eq!((result.unresolved, result.projected), (1, 0));
		let head = reading_state::head(&db, &user_id, &media.id)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			head.progression, 0.3,
			"future external timestamps cannot pin a reading head"
		);
	}
}
