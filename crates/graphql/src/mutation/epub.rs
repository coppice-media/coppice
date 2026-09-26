use crate::{
	data::CoreContext,
	input::media::{BookmarkInput, CreateAnnotationInput, UpdateAnnotationInput},
	object::{bookmark::Bookmark, media_annotation::MediaAnnotation},
};
use async_graphql::{Context, Object, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use models::{
	entity::{bookmark, media, media_annotation, user::AuthUser},
	shared::{
		liseur_annotation_projection::{
			is_liseur_sync_projection_id, liseur_sync_projection_id,
			parse_stump_native_annotation_id, projection_link_order_by,
			stump_native_annotation_id,
		},
		readium::{ReadiumLocator, ReadiumText},
	},
	txn::begin_write,
};
use sea_orm::{
	prelude::*, ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter,
	Set, Statement, Value as DbValue,
};

#[derive(Default)]
pub struct EpubMutation;

// TODO: Would it make sense to fold these into the media mutation? Do people want
// bookmarks/annotations/etc for non-epub content?

#[Object]
impl EpubMutation {
	/// Create a bookmark for a user
	async fn create_bookmark(
		&self,
		ctx: &Context<'_>,
		input: BookmarkInput,
	) -> Result<Bookmark> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let active_model = input.into_active_model(user);
		let bookmark = active_model.insert(conn).await?;
		core.note_annotation_activity(&user.id);

		Ok(Bookmark { model: bookmark })
	}

	/// Delete a bookmark by ID, only if the user created it
	async fn delete_bookmark(&self, ctx: &Context<'_>, id: String) -> Result<Bookmark> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		if is_liseur_sync_projection_id(&id) {
			return Err("Liseur annotations are owned by Liseur Sync".into());
		}

		let bookmark = bookmark::Entity::find_for_user(user)
			.filter(bookmark::Column::Id.eq(id))
			.one(conn)
			.await?
			.ok_or("Bookmark not found")?;

		let _ = bookmark.clone().delete(conn).await?;
		core.note_annotation_activity(&user.id);
		Ok(Bookmark { model: bookmark })
	}

	/// Create an annotation (highlight/note)
	async fn create_annotation(
		&self,
		ctx: &Context<'_>,
		input: CreateAnnotationInput,
	) -> Result<MediaAnnotation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		let annotation = input.into_active_model(user);
		let created_annotation = annotation.insert(conn).await?;
		core.note_annotation_activity(&user.id);

		Ok(MediaAnnotation::from(created_annotation))
	}

	/// Update an annotation's note text or color. Liseur-backed rows use CAS;
	/// expectedRevision rejects stale Home edits without changing the locator.
	///
	/// The id routes by shape: `liseur-sync:{user}:{record}` is the Liseur
	/// lane (the `AnnotationEntry`/`annotationsByMediaId` id of the record),
	/// anything else is a native `media_annotations` row. A bare Liseur record
	/// id is never accepted, because it is client-chosen and may equal either.
	async fn update_annotation(
		&self,
		ctx: &Context<'_>,
		input: UpdateAnnotationInput,
	) -> Result<MediaAnnotation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		if let Some(annotation_id) = liseur_target_id(&user.id, &input.id) {
			let updated = update_liseur_annotation(conn, user, &annotation_id, &input)
				.await?
				.ok_or("Annotation not found")?;
			core.note_annotation_activity(&user.id);
			return Ok(MediaAnnotation::from(updated));
		}
		if is_liseur_sync_projection_id(&input.id) {
			return Err("Annotation not found".into());
		}

		let annotation = media_annotation::Entity::find()
			.filter(media_annotation::Column::Id.eq(&input.id))
			.filter(media_annotation::Column::UserId.eq(&user.id))
			.one(conn)
			.await?
			.ok_or("Annotation not found")?;
		let native_cas_id = stump_native_annotation_id("annotation", &input.id);
		if let Some(state) =
			load_liseur_annotation(conn, &user.id, &native_cas_id).await?
		{
			if state.deleted {
				return Err("Liseur annotation is deleted".into());
			}
			let updated = update_liseur_annotation(conn, user, &native_cas_id, &input)
				.await?
				.ok_or("Annotation not found")?;
			core.note_annotation_activity(&user.id);
			return Ok(MediaAnnotation::from(updated));
		}
		if input.expected_revision.is_some() {
			return Err("expectedRevision only applies to Liseur annotations".into());
		}
		let mut active_model: media_annotation::ActiveModel = annotation.into();
		active_model.annotation_text = Set(input.annotation_text);
		if let Some(color) = input.color {
			active_model.color = Set((!color.is_empty()).then_some(color));
		}
		let updated = active_model.update(conn).await?;
		core.note_annotation_activity(&user.id);
		Ok(MediaAnnotation::from(updated))
	}

	/// Delete an annotation by ID, routed by shape exactly as
	/// `updateAnnotation`. Liseur-backed rows accept an optional CAS revision
	/// precondition and publish the tombstone through the sync feed.
	async fn delete_annotation(
		&self,
		ctx: &Context<'_>,
		id: String,
		expected_revision: Option<i64>,
	) -> Result<MediaAnnotation> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();

		if let Some(annotation_id) = liseur_target_id(&user.id, &id) {
			let deleted =
				delete_liseur_annotation(conn, user, &annotation_id, expected_revision)
					.await?
					.ok_or("Annotation not found")?;
			core.note_annotation_activity(&user.id);
			return Ok(MediaAnnotation::from(deleted));
		}
		if is_liseur_sync_projection_id(&id) {
			return Err("Annotation not found".into());
		}

		let annotation = media_annotation::Entity::find()
			.filter(media_annotation::Column::Id.eq(&id))
			.filter(media_annotation::Column::UserId.eq(&user.id))
			.one(conn)
			.await?
			.ok_or("Annotation not found")?;
		let native_cas_id = stump_native_annotation_id("annotation", &id);
		if let Some(state) =
			load_liseur_annotation(conn, &user.id, &native_cas_id).await?
		{
			if state.deleted {
				return Err("Liseur annotation is already deleted".into());
			}
			let deleted =
				delete_liseur_annotation(conn, user, &native_cas_id, expected_revision)
					.await?
					.ok_or("Annotation not found")?;
			core.note_annotation_activity(&user.id);
			return Ok(MediaAnnotation::from(deleted));
		}
		if expected_revision.is_some() {
			return Err("expectedRevision only applies to Liseur annotations".into());
		}
		let _ = annotation.clone().delete(conn).await?;
		core.note_annotation_activity(&user.id);
		Ok(MediaAnnotation::from(annotation))
	}
}

const LISEUR_ANNOTATION_COLORS: [&str; 10] = [
	"yellow", "green", "blue", "pink", "purple", "orange", "red", "olive", "cyan", "gray",
];

#[derive(Debug)]
struct LiseurAnnotationState {
	rev: i64,
	work_id: String,
	edition_sha: Option<String>,
	kind: String,
	locator: Option<String>,
	progression: Option<f64>,
	excerpt: String,
	color: String,
	drawer: Option<String>,
	body: String,
	client_ts: String,
	updated_at: String,
	deleted: bool,
}

/// The Liseur record a routed id names: the projection id shape for this
/// user, `liseur-sync:{user}:{record}`, decoded by stripping the fixed prefix
/// so a record id containing `:` or a `liseur-sync:` prefix of its own
/// round-trips unchanged.
fn liseur_target_id(user_id: &str, id: &str) -> Option<String> {
	let prefix = liseur_sync_projection_id(user_id, "");
	id.strip_prefix(&prefix)
		.filter(|annotation_id| !annotation_id.is_empty())
		.map(str::to_owned)
}

fn liseur_statement<C: ConnectionTrait>(
	conn: &C,
	sql: &str,
	values: Vec<DbValue>,
) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}

fn sync_now() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn sync_datetime(value: &str) -> Result<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(value)
		.map(|value| value.with_timezone(&Utc))
		.map_err(|_| "Liseur annotation has an invalid timestamp".into())
}

fn liseur_state(row: &sea_orm::QueryResult) -> Result<LiseurAnnotationState> {
	Ok(LiseurAnnotationState {
		rev: row.try_get("", "rev")?,
		work_id: row.try_get("", "work_id")?,
		edition_sha: row.try_get("", "edition_sha")?,
		kind: row.try_get("", "kind")?,
		locator: row.try_get("", "locator")?,
		progression: row.try_get("", "progression")?,
		excerpt: row.try_get("", "excerpt")?,
		color: row.try_get("", "color")?,
		drawer: row.try_get("", "drawer")?,
		body: row.try_get("", "body")?,
		client_ts: row.try_get("", "client_ts")?,
		updated_at: row.try_get("", "updated_at")?,
		deleted: row.try_get("", "deleted")?,
	})
}

async fn load_liseur_annotation<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation_id: &str,
) -> Result<Option<LiseurAnnotationState>> {
	let row = conn
		.query_one(liseur_statement(
			conn,
			"SELECT rev, work_id, edition_sha, kind, locator, progression, excerpt,
                    color, drawer, body, client_ts, updated_at, deleted
             FROM liseur_sync_annotations
             WHERE user_id = $1 AND annotation_id = $2",
			vec![user_id.to_owned().into(), annotation_id.to_owned().into()],
		))
		.await?;
	row.as_ref().map(liseur_state).transpose()
}

async fn next_liseur_annotation_seq<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<i64> {
	conn.execute(liseur_statement(
		conn,
		"INSERT INTO liseur_sync_counters (user_id, op_seq, annotation_seq)
         VALUES ($1, 0, 0) ON CONFLICT(user_id) DO NOTHING",
		vec![user_id.to_owned().into()],
	))
	.await?;
	conn.execute(liseur_statement(
		conn,
		"UPDATE liseur_sync_counters
         SET annotation_seq = annotation_seq + 1 WHERE user_id = $1",
		vec![user_id.to_owned().into()],
	))
	.await?;
	conn.query_one(liseur_statement(
		conn,
		"SELECT annotation_seq FROM liseur_sync_counters WHERE user_id = $1",
		vec![user_id.to_owned().into()],
	))
	.await?
	.ok_or_else(|| async_graphql::Error::new("Liseur annotation counter disappeared"))
	.and_then(|row| row.try_get("", "annotation_seq").map_err(Into::into))
}

async fn visible_liseur_media_id<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	annotation: &LiseurAnnotationState,
) -> Result<String> {
	let row = conn
		.query_one(liseur_statement(
			conn,
			&format!(
				"SELECT l.media_id FROM liseur_sync_media_links l
             WHERE l.user_id = $1 AND l.work_id = $2
             ORDER BY {}
             LIMIT 1",
				projection_link_order_by("l", "$3")
			),
			vec![
				user.id.clone().into(),
				annotation.work_id.clone().into(),
				annotation.edition_sha.clone().unwrap_or_default().into(),
			],
		))
		.await?
		.ok_or_else(|| {
			async_graphql::Error::new("Liseur annotation has no linked book")
		})?;
	let media_id: String = row.try_get("", "media_id")?;
	let visible = media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(&media_id))
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::SeriesId.is_not_null())
		.filter(media::audio_extension_condition().not())
		.one(conn)
		.await?;
	if visible.is_none() {
		return Err("Liseur annotation's book is not visible".into());
	}
	Ok(media_id)
}

async fn native_source_annotation<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	annotation_id: &str,
	state: &LiseurAnnotationState,
) -> Result<Option<media_annotation::Model>> {
	let Some(("annotation", native_id)) = parse_stump_native_annotation_id(annotation_id)
	else {
		return Ok(None);
	};
	let annotation = media_annotation::Entity::find_by_id(native_id)
		.filter(media_annotation::Column::UserId.eq(&user.id))
		.one(conn)
		.await?
		.ok_or("Native annotation no longer exists")?;
	let linked = conn
		.query_one(liseur_statement(
			conn,
			"SELECT 1 FROM liseur_sync_media_links
			 WHERE user_id = $1 AND work_id = $2 AND media_id = $3
			   AND (($4 IS NULL AND edition_sha IS NULL) OR edition_sha = $4)
			 LIMIT 1",
			vec![
				user.id.clone().into(),
				state.work_id.clone().into(),
				annotation.media_id.clone().into(),
				state.edition_sha.clone().into(),
			],
		))
		.await?
		.is_some();
	if !linked {
		return Err("Native annotation is no longer linked to this Liseur work".into());
	}
	let visible = media::Entity::find_for_user(user)
		.filter(media::Column::Id.eq(&annotation.media_id))
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::SeriesId.is_not_null())
		.filter(media::audio_extension_condition().not())
		.one(conn)
		.await?;
	if visible.is_none() {
		return Err("Native annotation's book is not visible".into());
	}
	Ok(Some(annotation))
}

fn liseur_projection_locator(
	annotation: &LiseurAnnotationState,
) -> Result<ReadiumLocator> {
	let raw = annotation
		.locator
		.as_deref()
		.ok_or_else(|| "Liseur annotation has no editable Readium locator")?;
	let mut locator: ReadiumLocator = serde_json::from_str(raw)
		.map_err(|_| "Liseur annotation has an invalid Readium locator")?;
	if locator.href.trim().is_empty() || locator.locations.is_none() {
		return Err("Liseur annotation has no editable Readium locator".into());
	}
	if annotation.kind == "note" {
		if let Some(text) = &mut locator.text {
			text.highlight = None;
		}
	} else if annotation.kind == "highlight"
		&& locator
			.text
			.as_ref()
			.and_then(|text| text.highlight.as_deref())
			.map_or(true, |highlight| highlight.trim().is_empty())
		&& !annotation.excerpt.is_empty()
	{
		locator
			.text
			.get_or_insert(ReadiumText {
				after: None,
				before: None,
				highlight: None,
			})
			.highlight = Some(annotation.excerpt.clone());
	}
	if annotation.kind == "highlight"
		&& locator
			.text
			.as_ref()
			.and_then(|text| text.highlight.as_deref())
			.map_or(true, |highlight| highlight.trim().is_empty())
	{
		return Err("Liseur highlight has no selected passage".into());
	}
	Ok(locator)
}

async fn ensure_liseur_projection<C: ConnectionTrait>(
	conn: &C,
	user: &AuthUser,
	annotation_id: &str,
	state: &LiseurAnnotationState,
	body: &str,
	color: &str,
	updated_at: &str,
) -> Result<media_annotation::Model> {
	if !matches!(state.kind.as_str(), "note" | "highlight") {
		return Err("Only Liseur notes and highlights are editable in Home".into());
	}
	let media_id = visible_liseur_media_id(conn, user, state).await?;
	let projection_id = liseur_sync_projection_id(&user.id, annotation_id);
	let color = (!color.is_empty()).then(|| color.to_owned());
	if media_annotation::Entity::find_by_id(&projection_id)
		.filter(media_annotation::Column::UserId.eq(&user.id))
		.one(conn)
		.await?
		.is_some()
	{
		conn.execute(liseur_statement(
			conn,
			"UPDATE media_annotations
             SET annotation_text = $1, color = $2, updated_at = $3, media_id = $4
             WHERE id = $5 AND user_id = $6",
			vec![
				(!body.is_empty()).then(|| body.to_owned()).into(),
				color.into(),
				updated_at.to_owned().into(),
				media_id.into(),
				projection_id.clone().into(),
				user.id.clone().into(),
			],
		))
		.await?;
	} else {
		let locator = liseur_projection_locator(state)?;
		let created_at = sync_datetime(&state.client_ts)?;
		let updated_at_value = sync_datetime(updated_at)?;
		media_annotation::ActiveModel {
			id: Set(projection_id.clone()),
			locator: Set(locator),
			annotation_text: Set((!body.is_empty()).then(|| body.to_owned())),
			color: Set(color.clone()),
			media_id: Set(media_id),
			user_id: Set(user.id.clone()),
			created_at: Set(created_at.clone()),
			updated_at: Set(updated_at_value.clone()),
		}
		.insert(conn)
		.await?;
		conn.execute(liseur_statement(
			conn,
			"UPDATE media_annotations SET created_at = $1, updated_at = $2
             WHERE id = $3 AND user_id = $4",
			vec![
				created_at.clone().into(),
				updated_at_value.into(),
				projection_id.clone().into(),
				user.id.clone().into(),
			],
		))
		.await?;
	}
	media_annotation::Entity::find_by_id(projection_id)
		.filter(media_annotation::Column::UserId.eq(&user.id))
		.one(conn)
		.await?
		.ok_or_else(|| {
			async_graphql::Error::new("Liseur annotation projection disappeared")
		})
}

async fn update_liseur_annotation(
	conn: &DatabaseConnection,
	user: &AuthUser,
	annotation_id: &str,
	input: &UpdateAnnotationInput,
) -> Result<Option<media_annotation::Model>> {
	let txn = begin_write(conn).await?;
	let Some(state) = load_liseur_annotation(&txn, &user.id, annotation_id).await? else {
		txn.commit().await?;
		return Ok(None);
	};
	if state.deleted {
		return Err("Liseur annotation is deleted".into());
	}
	if let Some(expected) = input.expected_revision {
		if expected != state.rev {
			return Err(format!(
				"annotation revision conflict: expected {expected}, current {}",
				state.rev
			)
			.into());
		}
	}
	if !matches!(state.kind.as_str(), "note" | "highlight") {
		return Err("Only Liseur notes and highlights can be edited in Home".into());
	}
	let body = input
		.annotation_text
		.clone()
		.unwrap_or_else(|| state.body.clone());
	if state.kind == "note" && body.trim().is_empty() {
		return Err("A Liseur note cannot have an empty body".into());
	}
	let color = input.color.clone().unwrap_or_else(|| state.color.clone());
	if state.kind == "note" && !color.is_empty() {
		return Err("Liseur note colors are not supported".into());
	}
	if !color.is_empty() && !LISEUR_ANNOTATION_COLORS.contains(&color.as_str()) {
		return Err("color must be one of the Liseur annotation palette tokens".into());
	}

	let updated_at = sync_now();

	let native_source =
		native_source_annotation(&txn, user, annotation_id, &state).await?;
	let projection = if let Some(source) = native_source {
		let result = txn
			.execute(liseur_statement(
				&txn,
				"UPDATE media_annotations
				 SET annotation_text = $1, color = $2, updated_at = $3
				 WHERE id = $4 AND user_id = $5",
				vec![
					(!body.is_empty()).then(|| body.clone()).into(),
					(!color.is_empty()).then(|| color.clone()).into(),
					sync_datetime(&updated_at)?.into(),
					source.id.clone().into(),
					user.id.clone().into(),
				],
			))
			.await?;
		if result.rows_affected() != 1 {
			return Err("Native annotation no longer exists".into());
		}
		media_annotation::Entity::find_by_id(source.id)
			.filter(media_annotation::Column::UserId.eq(&user.id))
			.one(&txn)
			.await?
			.ok_or_else(|| async_graphql::Error::new("Native annotation disappeared"))?
	} else {
		ensure_liseur_projection(
			&txn,
			user,
			annotation_id,
			&state,
			&body,
			&color,
			&updated_at,
		)
		.await?
	};
	let seq = next_liseur_annotation_seq(&txn, &user.id).await?;
	let payload = serde_json::json!({
		"id": annotation_id,
		"base_rev": state.rev,
		"work_id": state.work_id,
		"edition_sha": state.edition_sha,
		"kind": state.kind,
		"locator": state.locator.as_deref().and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
		"progression": state.progression,
		"excerpt": state.excerpt,
		"color": color,
		"drawer": state.drawer,
		"body": body,
		"client_ts": state.client_ts,
	});
	let result = txn
		.execute(liseur_statement(
			&txn,
			"UPDATE liseur_sync_annotations
             SET rev = $1, seq = $2, body = $3, color = $4, device_id = 'stump-native',
                 updated_at = $5, payload = $6
             WHERE user_id = $7 AND annotation_id = $8 AND rev = $9 AND deleted = FALSE",
			vec![
				(state.rev + 1).into(),
				seq.into(),
				body.into(),
				color.into(),
				updated_at.into(),
				serde_json::to_string(&payload)?.into(),
				user.id.clone().into(),
				annotation_id.to_owned().into(),
				state.rev.into(),
			],
		))
		.await?;
	if result.rows_affected() != 1 {
		return Err("annotation revision conflict".into());
	}
	let updated = media_annotation::Entity::find_by_id(projection.id)
		.filter(media_annotation::Column::UserId.eq(&user.id))
		.one(&txn)
		.await?
		.ok_or_else(|| {
			async_graphql::Error::new("Liseur annotation projection disappeared")
		})?;
	txn.commit().await?;
	Ok(Some(updated))
}

async fn delete_liseur_annotation(
	conn: &DatabaseConnection,
	user: &AuthUser,
	annotation_id: &str,
	expected_revision: Option<i64>,
) -> Result<Option<media_annotation::Model>> {
	let txn = begin_write(conn).await?;
	let Some(state) = load_liseur_annotation(&txn, &user.id, annotation_id).await? else {
		txn.commit().await?;
		return Ok(None);
	};
	if state.deleted {
		return Err("Liseur annotation is already deleted".into());
	}
	if let Some(expected) = expected_revision {
		if expected != state.rev {
			return Err(format!(
				"annotation revision conflict: expected {expected}, current {}",
				state.rev
			)
			.into());
		}
	}
	let updated_at = sync_now();

	let native_source =
		native_source_annotation(&txn, user, annotation_id, &state).await?;
	let projection = if let Some(source) = native_source {
		source
	} else {
		ensure_liseur_projection(
			&txn,
			user,
			annotation_id,
			&state,
			&state.body,
			&state.color,
			&state.updated_at,
		)
		.await?
	};
	let seq = next_liseur_annotation_seq(&txn, &user.id).await?;
	let result = txn
		.execute(liseur_statement(
			&txn,
			"UPDATE liseur_sync_annotations
             SET rev = $1, seq = $2, device_id = 'stump-native',
                 updated_at = $3, deleted = TRUE, deleted_at = $3
             WHERE user_id = $4 AND annotation_id = $5 AND rev = $6 AND deleted = FALSE",
			vec![
				(state.rev + 1).into(),
				seq.into(),
				updated_at.into(),
				user.id.clone().into(),
				annotation_id.to_owned().into(),
				state.rev.into(),
			],
		))
		.await?;
	if result.rows_affected() != 1 {
		return Err("annotation revision conflict".into());
	}
	txn.execute(liseur_statement(
		&txn,
		"DELETE FROM media_annotations WHERE id = $1 AND user_id = $2",
		vec![projection.id.clone().into(), user.id.clone().into()],
	))
	.await?;
	txn.commit().await?;
	Ok(Some(projection))
}
#[cfg(test)]
mod tests {
	use ::tests::{db::test_database, fake_data};
	use chrono::TimeZone;
	use models::{
		entity::media_annotation,
		shared::liseur_annotation_projection::{
			liseur_sync_projection_id, stump_native_annotation_id,
		},
	};
	use sea_orm::{
		ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DbBackend, EntityTrait,
		Schema, Statement,
	};

	use super::*;

	fn ts(secs: i64) -> String {
		Utc.timestamp_opt(1_700_000_000 + secs, 0)
			.unwrap()
			.to_rfc3339()
	}

	async fn fixture() -> (DatabaseConnection, AuthUser, ReadiumLocator, String, String) {
		let conn = test_database().await;
		let schema = Schema::new(DbBackend::Sqlite);
		conn.execute(
			conn.get_database_backend()
				.build(&schema.create_table_from_entity(media_annotation::Entity)),
		)
		.await
		.unwrap();
		let user_row = fake_data::User::new("annotation-home").insert(&conn).await;
		let user = AuthUser {
			id: user_row.id,
			username: user_row.username,
			is_server_owner: true,
			..Default::default()
		};
		let library = fake_data::Library::default().insert(&conn).await;
		let series = fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&conn)
		.await;
		let media = fake_data::Media {
			id: Some("native-home-media".to_owned()),
			series_id: series.id,
			..Default::default()
		}
		.insert(&conn)
		.await;
		let locator = serde_json::from_value::<ReadiumLocator>(serde_json::json!({
			"chapterTitle": "Chapter One",
			"href": "OPS/chapter.xhtml",
			"locations": {
				"position": 12,
				"progression": 0.25,
				"totalProgression": 0.5
			},
			"text": {
				"before": "before words",
				"highlight": "selected words",
				"after": "after words"
			}
		}))
		.unwrap();
		media_annotation::ActiveModel {
			id: Set("native-home-annotation".to_owned()),
			locator: Set(locator.clone()),
			annotation_text: Set(Some("Original Home note".to_owned())),
			color: Set(Some("red".to_owned())),
			media_id: Set(media.id.clone()),
			user_id: Set(user.id.clone()),
			created_at: Set(Utc.timestamp_opt(1_700_000_010, 0).unwrap()),
			updated_at: Set(Utc.timestamp_opt(1_700_000_020, 0).unwrap()),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		for sql in [
			"CREATE TABLE liseur_sync_media_links (
				id TEXT PRIMARY KEY, user_id TEXT NOT NULL, work_id TEXT NOT NULL,
				media_id TEXT NOT NULL, edition_sha TEXT, created_at TEXT NOT NULL
			)",
			"CREATE TABLE liseur_sync_counters (
				user_id TEXT PRIMARY KEY, op_seq BIGINT NOT NULL DEFAULT 0,
				annotation_seq BIGINT NOT NULL DEFAULT 0
			)",
			"CREATE TABLE liseur_sync_annotations (
				row_id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT NOT NULL,
				annotation_id TEXT NOT NULL, rev BIGINT NOT NULL, seq BIGINT NOT NULL,
				work_id TEXT NOT NULL, edition_sha TEXT, kind TEXT NOT NULL, locator TEXT,
				progression DOUBLE, excerpt TEXT NOT NULL, color TEXT NOT NULL, drawer TEXT,
				body TEXT NOT NULL, device_id TEXT NOT NULL, origin_device_id TEXT,
				client_ts TEXT NOT NULL, updated_at TEXT NOT NULL,
				deleted BOOLEAN NOT NULL DEFAULT FALSE, deleted_at TEXT, payload TEXT NOT NULL,
				UNIQUE(user_id, annotation_id)
			)",
		] {
			conn.execute(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
				.await
				.unwrap();
		}
		conn.execute(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			"INSERT INTO liseur_sync_media_links
			 (id, user_id, work_id, media_id, edition_sha, created_at)
			 VALUES ('link-1', ?, 'work-1', ?, 'edition-1', ?)",
			[user.id.clone().into(), media.id.into(), ts(0).into()],
		))
		.await
		.unwrap();
		conn.execute(Statement::from_sql_and_values(
			DbBackend::Sqlite,
			"INSERT INTO liseur_sync_counters (user_id, op_seq, annotation_seq)
			 VALUES (?, 0, 8)",
			[user.id.clone().into()],
		))
		.await
		.unwrap();

		let native_id = "native-home-annotation".to_owned();
		let native_cas_id = stump_native_annotation_id("annotation", &native_id);
		let locator_json = serde_json::to_string(&locator).unwrap();
		for (annotation_id, rev, seq, device_id, origin_device_id) in [
			(
				native_cas_id.as_str(),
				3,
				7,
				"koreader-device",
				"stump-native",
			),
			("liseur-created", 2, 8, "koreader-device", "koreader-device"),
		] {
			conn.execute(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"INSERT INTO liseur_sync_annotations
				 (user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
				  progression, excerpt, color, drawer, body, device_id, origin_device_id,
				  client_ts, updated_at, deleted, deleted_at, payload)
				 VALUES (?, ?, ?, ?, 'work-1', 'edition-1', 'highlight', ?,
				  0.5, 'selected words', 'red', NULL, 'Original Home note', ?, ?, ?, ?,
				  FALSE, NULL, '{}')",
				[
					user.id.clone().into(),
					annotation_id.to_owned().into(),
					rev.into(),
					seq.into(),
					locator_json.clone().into(),
					device_id.into(),
					origin_device_id.into(),
					ts(10).into(),
					ts(20).into(),
				],
			))
			.await
			.unwrap();
		}

		(conn, user, locator, native_id, native_cas_id)
	}

	fn update_input(
		id: &str,
		revision: i64,
		body: &str,
		color: &str,
	) -> UpdateAnnotationInput {
		UpdateAnnotationInput {
			id: id.to_owned(),
			annotation_text: Some(body.to_owned()),
			color: Some(color.to_owned()),
			expected_revision: Some(revision),
		}
	}

	#[tokio::test]
	async fn home_cas_edits_preserve_creator_and_locator_and_tombstone_both_lanes() {
		let (conn, user, original_locator, native_id, native_cas_id) = fixture().await;

		let native_edit = update_liseur_annotation(
			&conn,
			&user,
			&native_cas_id,
			&update_input(&native_id, 3, "Edited in Home", "green"),
		)
		.await
		.unwrap()
		.unwrap();
		assert_eq!(native_edit.id, native_id);
		assert_eq!(
			native_edit.annotation_text.as_deref(),
			Some("Edited in Home")
		);
		assert_eq!(native_edit.color.as_deref(), Some("green"));
		assert_eq!(native_edit.locator, original_locator);

		let row = conn
			.query_one(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"SELECT rev, seq, origin_device_id, device_id, body, color, deleted
				 FROM liseur_sync_annotations WHERE user_id = ? AND annotation_id = ?",
				[user.id.clone().into(), native_cas_id.clone().into()],
			))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(row.try_get::<i64>("", "rev").unwrap(), 4);
		assert_eq!(row.try_get::<i64>("", "seq").unwrap(), 9);
		assert_eq!(
			row.try_get::<String>("", "origin_device_id").unwrap(),
			"stump-native"
		);
		assert_eq!(
			row.try_get::<String>("", "device_id").unwrap(),
			"stump-native"
		);
		assert_eq!(row.try_get::<String>("", "body").unwrap(), "Edited in Home");
		assert_eq!(row.try_get::<String>("", "color").unwrap(), "green");

		let stale = update_liseur_annotation(
			&conn,
			&user,
			&native_cas_id,
			&update_input(&native_id, 3, "Stale edit", "pink"),
		)
		.await
		.unwrap_err();
		assert!(stale.message.contains("expected 3, current 4"));
		assert_eq!(
			media_annotation::Entity::find_by_id(&native_id)
				.one(&conn)
				.await
				.unwrap()
				.unwrap()
				.annotation_text
				.as_deref(),
			Some("Edited in Home")
		);

		let native_deleted =
			delete_liseur_annotation(&conn, &user, &native_cas_id, Some(4))
				.await
				.unwrap()
				.unwrap();
		assert_eq!(native_deleted.id, native_id);
		assert!(media_annotation::Entity::find_by_id(&native_id)
			.one(&conn)
			.await
			.unwrap()
			.is_none());

		let liseur_edit = update_liseur_annotation(
			&conn,
			&user,
			"liseur-created",
			&update_input("liseur-created", 2, "Home updated Liseur note", "blue"),
		)
		.await
		.unwrap()
		.unwrap();
		let projection_id = liseur_sync_projection_id(&user.id, "liseur-created");
		assert_eq!(liseur_edit.id, projection_id);
		assert_eq!(
			liseur_edit.annotation_text.as_deref(),
			Some("Home updated Liseur note")
		);
		assert_eq!(liseur_edit.color.as_deref(), Some("blue"));
		assert_eq!(liseur_edit.locator.href, original_locator.href);

		let liseur_deleted =
			delete_liseur_annotation(&conn, &user, "liseur-created", Some(3))
				.await
				.unwrap()
				.unwrap();
		assert_eq!(liseur_deleted.id, projection_id);
		assert!(media_annotation::Entity::find_by_id(projection_id)
			.one(&conn)
			.await
			.unwrap()
			.is_none());
		let tombstones = conn
			.query_all(Statement::from_sql_and_values(
				DbBackend::Sqlite,
				"SELECT annotation_id, rev, origin_device_id, deleted
				 FROM liseur_sync_annotations WHERE user_id = ? ORDER BY annotation_id",
				[user.id.into()],
			))
			.await
			.unwrap();
		let native_tombstone = tombstones
			.iter()
			.find(|row| {
				row.try_get::<String>("", "annotation_id").unwrap() == native_cas_id
			})
			.unwrap();
		assert_eq!(native_tombstone.try_get::<i64>("", "rev").unwrap(), 5);
		assert!(native_tombstone.try_get::<bool>("", "deleted").unwrap());
		let liseur_tombstone = tombstones
			.iter()
			.find(|row| {
				row.try_get::<String>("", "annotation_id").unwrap() == "liseur-created"
			})
			.unwrap();
		assert_eq!(liseur_tombstone.try_get::<i64>("", "rev").unwrap(), 4);
		assert_eq!(
			liseur_tombstone
				.try_get::<String>("", "origin_device_id")
				.unwrap(),
			"koreader-device"
		);
		assert!(liseur_tombstone.try_get::<bool>("", "deleted").unwrap());
	}
}
