use std::{
	collections::{BTreeMap, HashMap, HashSet},
	fs::File,
	io::{self, Read},
	path::Path,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, SecondsFormat, Utc};
use models::txn::begin_write;
use models::{
	domain::{
		reading_progress::calculate_logical_date,
		reading_state::{Position, ProtocolUpdate, Publication, SourceProtocol},
	},
	entity::{
		bookmark, library, liseur_sync_series_name, media, media_annotation,
		media_metadata, reading_session, series,
		user::{self, AuthUser, LoginUser},
		user_preferences,
	},
	services::{reading_progress::derive_readthrough_number, reading_state},
	shared::{
		enums::{FileStatus, ReadingStatus},
		liseur_annotation_projection::{
			is_liseur_sync_projection_id, is_stump_native_annotation_id,
			liseur_sync_projection_id, parse_stump_native_annotation_id,
			stump_native_annotation_id, STUMP_NATIVE_ANNOTATION_ID_PREFIX,
		},
		readium::{ReadiumLocator, ReadiumText},
	},
};
use sea_orm::{
	prelude::Decimal, ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait,
	DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
	QueryResult, Statement, Value as DbValue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use stump_auth::AuthContext;
use stump_devices::{
	liseur_token::{hash_secret, new_secret, DEVICE_TOKEN_TTL_SECS},
	CredentialRef, Protocol,
};
use stump_liseur_sync::{
	AnnotationInput, AnnotationRecord, AnnotationResult, CatalogBook, CatalogBookSeries,
	CatalogBooksPage, CatalogContributor, CatalogCover, CatalogDownload, CatalogFolder,
	CatalogFoldersPage, CatalogResolveResult, CatalogSeriesMembership, CatalogSeriesName,
	ChangesPage, DeleteAnnotationResult, HeadsPage, Identifier, LiseurSyncError,
	LiseurToken, LiseurTokenKind, LoginResult, OpInput, OpRecord, OpResult,
	ResolveRequest, ResolveResult, SessionInput, SettingUpdate, SettingValue,
	TokenCreateResult, ANNOTATION_COLORS, ANNOTATION_DRAWERS, LISEUR_ANNOTATION_COLORS,
	MAX_SERIES_NAME_BYTES, MAX_SETTINGS_PER_ACCOUNT,
};
use uuid::Uuid;

use crate::config::state::AppState;
use crate::utils::verify_password;

const TOKEN_TTL_SECS: i64 = 60 * 60;

pub(super) fn db_statement<C: ConnectionTrait>(
	conn: &C,
	sql: &str,
	values: Vec<DbValue>,
) -> Statement {
	Statement::from_sql_and_values(conn.get_database_backend(), sql, values)
}

pub(super) fn internal(error: impl ToString) -> LiseurSyncError {
	LiseurSyncError::Internal(error.to_string())
}

fn now_string() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn ctx_conn(ctx: &AppState) -> &DatabaseConnection {
	ctx.conn.as_ref()
}

/// The Stump media a liseur work resolved to for `user_id`, when any.
async fn linked_media<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	work_id: &str,
) -> Result<Option<media::Model>, LiseurSyncError> {
	let row = conn
		.query_one(db_statement(
			conn,
			"SELECT media_id FROM liseur_sync_media_links
             WHERE user_id = $1 AND work_id = $2",
			vec![user_id.to_owned().into(), work_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	let Some(row) = row else {
		return Ok(None);
	};
	let media_id: String = row.try_get("", "media_id").map_err(internal)?;
	media::Entity::find_by_id(media_id)
		.one(conn)
		.await
		.map_err(internal)
}

/// Project an accepted liseur position op onto the unified reading head of
/// the media its work resolved to. The opaque locator is parsed as a Readium
/// locator only when it validates; the op itself stays the raw provenance.
async fn apply_op_to_head<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	device_id: &str,
	op: &OpInput,
) -> Result<(), LiseurSyncError> {
	let Some(media) = linked_media(conn, user_id, &op.work_id).await? else {
		return Ok(());
	};
	let locator = op
		.locator
		.clone()
		.and_then(|value| serde_json::from_value::<ReadiumLocator>(value).ok())
		.filter(|locator| !locator.href.is_empty());
	let progression = op.progression.expect("validated progression");
	reading_state::apply(
		conn,
		user_id,
		Publication::from(&media),
		ProtocolUpdate {
			protocol: SourceProtocol::Liseur,
			device_id: Some(device_id.to_owned()),
			updated_at: DateTime::parse_from_rfc3339(&op.client_ts)
				.ok()
				.map(|at| at.to_utc()),
			position: locator.map_or(Position::None, Position::Locator),
			progression: Some(progression),
			completed: (progression >= 1.0).then_some(true),
			raw_payload: serde_json::to_value(op).map_err(internal)?,
		},
	)
	.await
	.map_err(internal)?;
	Ok(())
}

/// Idempotency key of the op that mirrors a head materialized by `event_id`.
fn mirrored_op_id(event_id: i64) -> String {
	format!("stump-head:{event_id}")
}

/// Append one op per linked head that another protocol moved since it was
/// last mirrored, so liseur clients pull Kobo/Komga/KOReader/OPDS progress
/// through the ordinary change feed. Liseur-originated heads are already in
/// the log and are skipped; the op id is derived from the winning event, so a
/// head is mirrored at most once per change.
async fn mirror_heads(ctx: &AppState, user_id: &str) -> Result<(), LiseurSyncError> {
	let conn = ctx_conn(ctx);
	let links = conn
		.query_all(db_statement(
			conn,
			"SELECT media_id, work_id, edition_sha FROM liseur_sync_media_links
             WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	if links.is_empty() {
		return Ok(());
	}
	let mut by_media = HashMap::with_capacity(links.len());
	for row in links {
		let media_id: String = row.try_get("", "media_id").map_err(internal)?;
		let work_id: String = row.try_get("", "work_id").map_err(internal)?;
		let edition_sha: String = row.try_get("", "edition_sha").map_err(internal)?;
		by_media.insert(media_id, (work_id, edition_sha));
	}
	let media_ids = by_media.keys().cloned().collect::<Vec<_>>();
	let heads = reading_state::heads(conn, user_id, &media_ids)
		.await
		.map_err(internal)?;

	let txn = begin_write(conn).await.map_err(internal)?;
	ensure_counter(&txn, user_id).await?;
	for (media_id, head) in heads {
		if head.source_protocol == SourceProtocol::Liseur {
			continue;
		}
		let op_id = mirrored_op_id(head.event_id);
		if find_op(&txn, user_id, &op_id).await?.is_some() {
			continue;
		}
		let Some((work_id, edition_sha)) = by_media.get(&media_id) else {
			continue;
		};
		let seq = next_op_seq(&txn, user_id).await?;
		let locator = head
			.locator
			.as_ref()
			.map(serde_json::to_value)
			.transpose()
			.map_err(internal)?;
		txn.execute(db_statement(
			&txn,
			"INSERT INTO liseur_sync_ops
                (id, user_id, seq, op_id, work_id, edition_sha, device_id,
                 client_ts, progression, locator, foreign_pos, origin, received_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
			vec![
				Uuid::new_v4().to_string().into(),
				user_id.to_owned().into(),
				seq.into(),
				op_id.into(),
				work_id.clone().into(),
				Some(edition_sha.clone()).into(),
				head.source_device_id
					.clone()
					.unwrap_or_else(|| format!("stump:{}", head.source_protocol))
					.into(),
				head.updated_at
					.to_utc()
					.to_rfc3339_opts(SecondsFormat::Nanos, true)
					.into(),
				head.progression.into(),
				locator_to_db(&locator)?.into(),
				Option::<String>::None.into(),
				head.source_protocol.to_string().into(),
				now_string().into(),
			],
		))
		.await
		.map_err(internal)?;
	}
	txn.commit().await.map_err(internal)
}
#[derive(Debug, Deserialize, Serialize)]
struct FolderCursor {
	name: String,
	id: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct BookCursor {
	title: String,
	id: String,
}

fn encode_cursor<T: Serialize>(cursor: &T) -> Result<String, LiseurSyncError> {
	let bytes = serde_json::to_vec(cursor).map_err(internal)?;
	Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_cursor<T: for<'de> Deserialize<'de>>(
	value: &str,
	kind: &str,
) -> Result<T, LiseurSyncError> {
	let bytes = URL_SAFE_NO_PAD
		.decode(value)
		.map_err(|_| LiseurSyncError::BadRequest(format!("invalid {kind} cursor")))?;
	serde_json::from_slice(&bytes)
		.map_err(|_| LiseurSyncError::BadRequest(format!("invalid {kind} cursor")))
}

#[derive(Clone)]
struct CatalogRow {
	media: media::Model,
	metadata: Option<media_metadata::Model>,
	series: series::Model,
}

async fn visible_media(
	ctx: &AppState,
	auth: &AuthContext,
	folder_id: Option<&str>,
	book_id: Option<&str>,
) -> Result<Vec<CatalogRow>, LiseurSyncError> {
	let user = auth.user();
	// Liseur reads paginated documents; an audiobook has no pages and a
	// folder book has no single file to download, so audio rows are not
	// catalogue books here (the ABS profile serves them).
	let query = media::Entity::find_for_user(&user)
		.filter(media::Column::SeriesId.is_not_null())
		.filter(series::Column::LibraryId.is_not_null())
		.filter(media::audio_extension_condition().not());
	let query = if let Some(folder_id) = folder_id {
		query.filter(series::Column::LibraryId.eq(folder_id))
	} else {
		query
	};
	let query = if let Some(book_id) = book_id {
		query.filter(media::Column::Id.eq(book_id))
	} else {
		query
	};
	let media_rows = query.all(ctx_conn(ctx)).await.map_err(internal)?;
	if media_rows.is_empty() {
		return Ok(Vec::new());
	}

	let media_ids = media_rows
		.iter()
		.map(|media| media.id.clone())
		.collect::<Vec<_>>();
	let metadata_rows = media_metadata::Entity::find()
		.filter(media_metadata::Column::MediaId.is_in(media_ids))
		.all(ctx_conn(ctx))
		.await
		.map_err(internal)?;
	let metadata_by_media = metadata_rows
		.into_iter()
		.filter_map(|metadata| {
			metadata
				.media_id
				.clone()
				.map(|media_id| (media_id, metadata))
		})
		.collect::<HashMap<_, _>>();

	let series_ids = media_rows
		.iter()
		.filter_map(|media| media.series_id.clone())
		.collect::<Vec<_>>();
	let series_rows = series::Entity::find()
		.filter(series::Column::Id.is_in(series_ids))
		.all(ctx_conn(ctx))
		.await
		.map_err(internal)?;
	let series_by_id = series_rows
		.into_iter()
		.map(|series| (series.id.clone(), series))
		.collect::<HashMap<_, _>>();

	Ok(media_rows
		.into_iter()
		.filter_map(|media| {
			let series_id = media.series_id.as_ref()?;
			let series = series_by_id.get(series_id)?.clone();
			Some(CatalogRow {
				metadata: metadata_by_media.get(&media.id).cloned(),
				media,
				series,
			})
		})
		.collect())
}

async fn visible_library(
	ctx: &AppState,
	auth: &AuthContext,
	folder_id: &str,
) -> Result<(), LiseurSyncError> {
	let user = auth.user();
	let exists = library::Entity::find_for_user(&user)
		.filter(library::Column::Id.eq(folder_id))
		.one(ctx_conn(ctx))
		.await
		.map_err(internal)?
		.is_some();
	if exists {
		Ok(())
	} else {
		Err(LiseurSyncError::NotFound("folder not found".into()))
	}
}

fn writer_names(metadata: Option<&media_metadata::Model>) -> Vec<String> {
	metadata
		.and_then(|metadata| metadata.writers.as_deref())
		.into_iter()
		.flat_map(|writers| writers.split(','))
		.map(str::trim)
		.filter(|writer| !writer.is_empty())
		.map(str::to_owned)
		.collect()
}

fn catalog_book(row: &CatalogRow) -> CatalogBook {
	let title = row
		.metadata
		.as_ref()
		.and_then(|metadata| metadata.title.clone())
		.filter(|title| !title.trim().is_empty())
		.unwrap_or_else(|| row.media.name.clone());
	let writers = writer_names(row.metadata.as_ref());
	let position = row
		.metadata
		.as_ref()
		.and_then(|metadata| metadata.number)
		.and_then(|position| position.to_string().parse::<f64>().ok());
	let membership = CatalogSeriesMembership {
		id: row.series.id.clone(),
		name: row.series.name.clone(),
		position,
		source: "folder".into(),
	};
	let missing = !matches!(row.media.status, FileStatus::Ready);
	let updated_at = row
		.media
		.updated_at
		.unwrap_or(row.media.created_at)
		.to_rfc3339_opts(SecondsFormat::Nanos, true);
	CatalogBook {
		book_id: row.media.id.clone(),
		folder_id: row.series.library_id.clone().unwrap_or_default(),
		title,
		author: (!writers.is_empty()).then(|| writers.join(", ")),
		contributors: writers
			.into_iter()
			.map(|name| CatalogContributor {
				name,
				role: "author".into(),
			})
			.collect(),
		series: vec![membership],
		series_claim_updated_at: None,
		series_source: "folder".into(),
		size_bytes: row.media.size,
		status: if missing {
			"missing".into()
		} else {
			"present".into()
		},
		missing,
		cover_url: None,
		updated_at,
		sha256: None,
	}
}

fn sort_books(books: &mut [CatalogBook]) {
	books.sort_unstable_by(|left, right| {
		left.title
			.cmp(&right.title)
			.then_with(|| left.book_id.cmp(&right.book_id))
	});
}
async fn apply_personal_series_names(
	ctx: &AppState,
	auth: &AuthContext,
	books: &mut [CatalogBook],
) -> Result<(), LiseurSyncError> {
	if books.is_empty() {
		return Ok(());
	}
	let names = liseur_sync_series_name::Entity::find()
		.filter(liseur_sync_series_name::Column::UserId.eq(auth.id()))
		.all(ctx_conn(ctx))
		.await
		.map_err(internal)?;
	let names = names
		.into_iter()
		.map(|name| (name.series_id, name.name))
		.collect::<HashMap<_, _>>();
	for book in books {
		for series in &mut book.series {
			if let Some(name) = names.get(&series.id) {
				series.name.clone_from(name);
			}
		}
	}
	Ok(())
}

fn normalize_series_name(name: &str) -> String {
	name.split_whitespace()
		.collect::<Vec<_>>()
		.join(" ")
		.to_lowercase()
}

async fn series_name_response(
	ctx: &AppState,
	auth: &AuthContext,
	series_id: &str,
) -> Result<CatalogSeriesName, LiseurSyncError> {
	let user = auth.user();
	let conn = ctx_conn(ctx);
	let series = series::Entity::find_for_user(&user)
		.filter(series::Column::Id.eq(series_id.to_owned()))
		.one(conn)
		.await
		.map_err(internal)?
		.ok_or_else(|| LiseurSyncError::NotFound("series not found".into()))?;
	let personal = liseur_sync_series_name::Entity::find()
		.filter(liseur_sync_series_name::Column::UserId.eq(auth.id()))
		.filter(liseur_sync_series_name::Column::SeriesId.eq(series_id.to_owned()))
		.one(conn)
		.await
		.map_err(internal)?;
	let book_count = media::Entity::find_for_user(&user)
		.filter(media::Column::SeriesId.eq(series_id.to_owned()))
		.filter(media::audio_extension_condition().not())
		.count(conn)
		.await
		.map_err(internal)?;
	Ok(CatalogSeriesName {
		id: series.id,
		name: personal
			.as_ref()
			.map(|name| name.name.clone())
			.unwrap_or_else(|| series.name.clone()),
		scanned_name: series.name,
		name_source: if personal.is_some() {
			"personal"
		} else {
			"folder"
		}
		.into(),
		book_count: i64::try_from(book_count).map_err(internal)?,
	})
}

pub(crate) async fn set_series_name(
	ctx: &AppState,
	auth: &AuthContext,
	series_id: &str,
	scope: &str,
	requested_name: &str,
) -> Result<CatalogSeriesName, LiseurSyncError> {
	if scope != "personal" {
		return Err(LiseurSyncError::BadRequest(
			"only personal series names are supported".into(),
		));
	}
	let name = requested_name.trim();
	if name.is_empty() {
		return Err(LiseurSyncError::BadRequest(
			"series name cannot be empty".into(),
		));
	}
	if name.len() > MAX_SERIES_NAME_BYTES {
		return Err(LiseurSyncError::BadRequest(
			"series name is too long".into(),
		));
	}
	let normalized_name = normalize_series_name(name);
	let user_id = auth.id();
	let user = auth.user();
	let txn = begin_write(ctx_conn(ctx)).await.map_err(internal)?;
	let visible_series = series::Entity::find_for_user(&user)
		.all(&txn)
		.await
		.map_err(internal)?;
	let Some(target) = visible_series.iter().find(|series| series.id == series_id) else {
		return Err(LiseurSyncError::NotFound("series not found".into()));
	};
	let personal_names = liseur_sync_series_name::Entity::find()
		.filter(liseur_sync_series_name::Column::UserId.eq(user_id.clone()))
		.all(&txn)
		.await
		.map_err(internal)?;
	let personal_names = personal_names
		.into_iter()
		.map(|name| (name.series_id, name.name))
		.collect::<HashMap<_, _>>();
	for series in &visible_series {
		if series.id == target.id {
			continue;
		}
		let display_name = personal_names
			.get(&series.id)
			.map(String::as_str)
			.unwrap_or(&series.name);
		if normalize_series_name(display_name) == normalized_name {
			return Err(LiseurSyncError::Conflict(
				"another visible series already uses that name".into(),
			));
		}
	}
	let now = now_string();
	txn.execute(db_statement(
		&txn,
		"INSERT INTO liseur_sync_series_names
            (user_id, series_id, name, normalized_name, updated_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT(user_id, series_id) DO UPDATE SET
            name = excluded.name,
            normalized_name = excluded.normalized_name,
            updated_at = excluded.updated_at",
		vec![
			user_id.into(),
			series_id.to_owned().into(),
			name.to_owned().into(),
			normalized_name.into(),
			now.into(),
		],
	))
	.await
	.map_err(internal)?;
	txn.commit().await.map_err(internal)?;
	series_name_response(ctx, auth, series_id).await
}

pub(crate) async fn clear_series_name(
	ctx: &AppState,
	auth: &AuthContext,
	series_id: &str,
	scope: &str,
) -> Result<CatalogSeriesName, LiseurSyncError> {
	if scope != "personal" {
		return Err(LiseurSyncError::BadRequest(
			"only personal series names are supported".into(),
		));
	}
	let user_id = auth.id();
	let user = auth.user();
	let txn = begin_write(ctx_conn(ctx)).await.map_err(internal)?;
	let visible = series::Entity::find_for_user(&user)
		.filter(series::Column::Id.eq(series_id.to_owned()))
		.one(&txn)
		.await
		.map_err(internal)?
		.is_some();
	if !visible {
		return Err(LiseurSyncError::NotFound("series not found".into()));
	}
	txn.execute(db_statement(
		&txn,
		"DELETE FROM liseur_sync_series_names
         WHERE user_id = $1 AND series_id = $2",
		vec![user_id.into(), series_id.to_owned().into()],
	))
	.await
	.map_err(internal)?;
	txn.commit().await.map_err(internal)?;
	series_name_response(ctx, auth, series_id).await
}

pub(crate) async fn folders(
	ctx: &AppState,
	auth: &AuthContext,
	after: Option<String>,
	limit: usize,
) -> Result<CatalogFoldersPage, LiseurSyncError> {
	let user = auth.user();
	let mut libraries = library::Entity::find_for_user(&user)
		.order_by_asc(library::Column::Name)
		.order_by_asc(library::Column::Id)
		.all(ctx_conn(ctx))
		.await
		.map_err(internal)?;
	let cursor = after
		.as_deref()
		.map(|value| decode_cursor::<FolderCursor>(value, "folder"))
		.transpose()?;
	let start = cursor.map_or(0, |cursor| {
		libraries
			.iter()
			.position(|library| {
				library.name > cursor.name
					|| (library.name == cursor.name && library.id > cursor.id)
			})
			.unwrap_or(libraries.len())
	});
	let end = (start + limit + 1).min(libraries.len());
	let has_next = end > start + limit;
	libraries.truncate(end);
	let folders = libraries
		.into_iter()
		.skip(start)
		.take(limit)
		.map(|library| CatalogFolder {
			folder_id: library.id,
			name: library.name,
			accepts_uploads: false,
		})
		.collect::<Vec<_>>();
	let next_after = has_next.then(|| {
		let folder = folders.last().expect("a next page has a folder");
		encode_cursor(&FolderCursor {
			name: folder.name.clone(),
			id: folder.folder_id.clone(),
		})
	});
	Ok(CatalogFoldersPage {
		folders,
		next_after: next_after.transpose()?,
	})
}

pub(crate) async fn folder_books(
	ctx: &AppState,
	auth: &AuthContext,
	folder_id: &str,
	cursor: Option<String>,
	limit: usize,
) -> Result<CatalogBooksPage, LiseurSyncError> {
	visible_library(ctx, auth, folder_id).await?;
	let rows = visible_media(ctx, auth, Some(folder_id), None).await?;
	let mut books = rows.iter().map(catalog_book).collect::<Vec<_>>();
	apply_personal_series_names(ctx, auth, &mut books).await?;
	sort_books(&mut books);
	let cursor = cursor
		.as_deref()
		.map(|value| decode_cursor::<BookCursor>(value, "book"))
		.transpose()?;
	let start = cursor.map_or(0, |cursor| {
		books
			.iter()
			.position(|book| {
				book.title > cursor.title
					|| (book.title == cursor.title && book.book_id > cursor.id)
			})
			.unwrap_or(books.len())
	});
	let end = (start + limit + 1).min(books.len());
	let has_next = end > start + limit;
	let page = books.drain(start..end).take(limit).collect::<Vec<_>>();
	let next_cursor = has_next.then(|| {
		let book = page.last().expect("a next page has a book");
		encode_cursor(&BookCursor {
			title: book.title.clone(),
			id: book.book_id.clone(),
		})
	});
	Ok(CatalogBooksPage {
		books: page,
		next_cursor: next_cursor.transpose()?,
	})
}

pub(crate) async fn folder_search(
	ctx: &AppState,
	auth: &AuthContext,
	folder_id: &str,
	query: &str,
) -> Result<Vec<CatalogBook>, LiseurSyncError> {
	visible_library(ctx, auth, folder_id).await?;
	let query = query.trim().to_lowercase();
	let rows = visible_media(ctx, auth, Some(folder_id), None).await?;
	let mut books = rows.iter().map(catalog_book).collect::<Vec<_>>();
	apply_personal_series_names(ctx, auth, &mut books).await?;
	books.retain(|book| {
		query.is_empty()
			|| book.title.to_lowercase().contains(&query)
			|| book
				.author
				.as_deref()
				.is_some_and(|author| author.to_lowercase().contains(&query))
			|| book
				.series
				.iter()
				.any(|series| series.name.to_lowercase().contains(&query))
	});
	sort_books(&mut books);
	Ok(books)
}

pub(crate) async fn book(
	ctx: &AppState,
	auth: &AuthContext,
	book_id: &str,
) -> Result<CatalogBook, LiseurSyncError> {
	let row = visible_media(ctx, auth, None, Some(book_id))
		.await?
		.into_iter()
		.next()
		.ok_or_else(|| LiseurSyncError::NotFound("book not found".into()))?;
	let mut book = catalog_book(&row);
	apply_personal_series_names(ctx, auth, std::slice::from_mut(&mut book)).await?;
	Ok(book)
}

pub(crate) async fn book_download(
	ctx: &AppState,
	auth: &AuthContext,
	book_id: &str,
) -> Result<CatalogDownload, LiseurSyncError> {
	let row = visible_media(ctx, auth, None, Some(book_id))
		.await?
		.into_iter()
		.next()
		.ok_or_else(|| LiseurSyncError::NotFound("book not found".into()))?;
	if !matches!(row.media.status, FileStatus::Ready) {
		return Err(LiseurSyncError::Gone("book file is missing".into()));
	}
	let path = Path::new(&row.media.path);
	let filename = path
		.file_name()
		.and_then(|name| name.to_str())
		.map(str::to_owned)
		.unwrap_or_else(|| format!("{}.{}", row.media.name, row.media.extension));
	Ok(CatalogDownload {
		path: row.media.path,
		filename,
		media_type: stump_media::ContentType::from_extension(&row.media.extension)
			.to_string(),
	})
}

pub(crate) async fn book_cover(
	ctx: &AppState,
	auth: &AuthContext,
	book_id: &str,
) -> Result<CatalogCover, LiseurSyncError> {
	let row = visible_media(ctx, auth, None, Some(book_id))
		.await?
		.into_iter()
		.next()
		.ok_or_else(|| LiseurSyncError::NotFound("book not found".into()))?;
	if !matches!(row.media.status, FileStatus::Ready) {
		return Err(LiseurSyncError::Gone("book file is missing".into()));
	}
	let image = crate::routers::api::v2::media::get_media_thumbnail_by_id(
		ctx.as_ref(),
		&auth.user(),
		book_id.to_owned(),
	)
	.await
	.map_err(|error| match error {
		crate::errors::APIError::NotFound(message) => LiseurSyncError::NotFound(message),
		other => internal(other),
	})?;
	Ok(CatalogCover {
		content_type: image.content_type.to_string(),
		bytes: image.data,
	})
}

pub(crate) async fn book_series(
	ctx: &AppState,
	auth: &AuthContext,
	book_id: &str,
	scope: Option<String>,
) -> Result<CatalogBookSeries, LiseurSyncError> {
	if scope
		.as_deref()
		.is_some_and(|scope| !matches!(scope, "folder" | "shared" | "personal"))
	{
		return Err(LiseurSyncError::BadRequest("invalid series scope".into()));
	}
	let book = book(ctx, auth, book_id).await?;
	Ok(CatalogBookSeries {
		book_id: book.book_id,
		source: book.series_source,
		series: book.series.clone(),
		folder: book.series,
		shared: None,
		personal: None,
		shared_updated_at: None,
		personal_updated_at: None,
		outcome: None,
	})
}

pub(crate) async fn resolve_catalog_book(
	ctx: &AppState,
	auth: &AuthContext,
	book_id: &str,
	confirmed: bool,
) -> Result<CatalogResolveResult, LiseurSyncError> {
	let row = visible_media(ctx, auth, None, Some(book_id))
		.await?
		.into_iter()
		.next()
		.ok_or_else(|| LiseurSyncError::NotFound("book not found".into()))?;
	let fallback_sha = full_file_sha256(row.media.path.clone()).await?;
	let existing_link = ctx_conn(ctx)
		.query_one(db_statement(
			ctx_conn(ctx),
			"SELECT work_id, edition_sha, pair_status, pair_evidence
             FROM liseur_sync_media_links
             WHERE user_id = $1 AND media_id = $2",
			vec![auth.id().to_owned().into(), book_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	if let Some(link) = existing_link {
		let mut work_id: String = link.try_get("", "work_id").map_err(internal)?;
		let linked_edition_sha: Option<String> =
			link.try_get("", "edition_sha").map_err(internal)?;
		let pair_status: Option<String> =
			link.try_get("", "pair_status").map_err(internal)?;
		let pair_evidence: Option<String> =
			link.try_get("", "pair_evidence").map_err(internal)?;
		let sha = fallback_sha.as_deref().or(linked_edition_sha.as_deref());
		let alias_work_ids = strong_alias_work_ids(
			ctx_conn(ctx),
			&auth.id(),
			&row.media.id,
			row.media.koreader_hash.as_deref(),
			sha,
		)
		.await?;
		if alias_work_ids.len() > 1 {
			let mut works = alias_work_ids;
			works.push(work_id);
			works.sort();
			works.dedup();
			return Err(LiseurSyncError::IdentityConflict(works));
		}
		if let Some(alias_work_id) = alias_work_ids.first() {
			if alias_work_id != &work_id {
				let pair_created = pair_evidence.is_some()
					&& matches!(
						pair_status.as_deref().unwrap_or("suggested"),
						"suggested" | "confirmed"
					);
				if pair_created {
					merge_aliasless_pair_work(
						ctx,
						&auth.id(),
						&row.media.id,
						&work_id,
						alias_work_id,
						row.media.koreader_hash.as_deref(),
						sha,
					)
					.await?;
					work_id = alias_work_id.clone();
				} else {
					let mut works = vec![work_id, alias_work_id.clone()];
					works.sort();
					works.dedup();
					return Err(LiseurSyncError::IdentityConflict(works));
				}
			}
		}
		let edition_sha = fallback_sha.or(linked_edition_sha);
		return Ok(CatalogResolveResult {
			book_id: book_id.to_owned(),
			work_id,
			confidence: "high".into(),
			created: false,
			identifiers: edition_sha
				.into_iter()
				.map(|value| Identifier {
					kind: "sha256".into(),
					value,
				})
				.collect(),
		});
	}
	let mut identifiers = Vec::with_capacity(2);
	if let Some(sha) = fallback_sha.clone() {
		identifiers.push(Identifier {
			kind: "sha256".into(),
			value: sha,
		});
	}
	// The catalog id is the stable binding that lets the backend verify a
	// caller-provided digest against the file it actually serves. `media.hash`
	// is a sampled Stump fingerprint, not a full-file SHA-256.
	identifiers.push(Identifier {
		kind: "source".into(),
		value: row.media.id.clone(),
	});
	let catalog = catalog_book(&row);
	let result = resolve_work(
		ctx,
		&auth.id(),
		ResolveRequest {
			identifiers,
			title: Some(catalog.title),
			author: catalog.author,
			confirmed,
		},
	)
	.await?;
	let edition_sha = ctx_conn(ctx)
		.query_one(db_statement(
			ctx_conn(ctx),
			"SELECT edition_sha FROM liseur_sync_media_links
             WHERE user_id = $1 AND media_id = $2 AND work_id = $3",
			vec![
				auth.id().into(),
				book_id.to_owned().into(),
				result.work_id.clone().into(),
			],
		))
		.await
		.map_err(internal)?
		.and_then(|row| row.try_get("", "edition_sha").ok())
		.or(fallback_sha);
	Ok(CatalogResolveResult {
		book_id: book_id.to_owned(),
		work_id: result.work_id,
		confidence: result.confidence,
		created: result.created,
		identifiers: edition_sha
			.into_iter()
			.map(|value| Identifier {
				kind: "sha256".into(),
				value,
			})
			.collect(),
	})
}

pub(crate) async fn login(
	ctx: &AppState,
	username: &str,
	password: &str,
) -> Result<LoginResult, LiseurSyncError> {
	let user = LoginUser::find()
		.filter(user::Column::Username.eq(username.to_owned()))
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<LoginUser>()
		.one(ctx_conn(ctx))
		.await
		.map_err(internal)?
		.ok_or(LiseurSyncError::Unauthorized)?;

	let password_matches =
		verify_password(&user.hashed_password, password).map_err(internal)?;
	if !password_matches {
		return Err(LiseurSyncError::Unauthorized);
	}
	if user.is_locked {
		return Err(LiseurSyncError::Forbidden("account is locked".into()));
	}

	// Keep the login response compatible with the existing endpoint while
	// recording this credential as a token-management-only session. Device
	// tokens minted through /v1/tokens are separate rows and carry scopes and
	// a device name of their own.
	let secret = new_secret();
	let token_id = Uuid::new_v4().to_string();
	let device_id = Uuid::new_v4().to_string();
	let created_at = now_string();
	let expires_at = (Utc::now() + chrono::Duration::seconds(TOKEN_TTL_SECS))
		.to_rfc3339_opts(SecondsFormat::Nanos, true);
	let scopes = serde_json::to_string(&Vec::<&str>::new()).map_err(internal)?;

	ctx_conn(ctx)
		.execute(db_statement(
			ctx_conn(ctx),
			"INSERT INTO liseur_sync_tokens
                (id, user_id, device_id, secret_hash, scopes, created_at, expires_at,
                 name, token_kind)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
			vec![
				token_id.into(),
				user.id.clone().into(),
				device_id.into(),
				hash_secret(&secret).into(),
				scopes.into(),
				created_at.into(),
				expires_at.into(),
				"login".into(),
				"session".into(),
			],
		))
		.await
		.map_err(internal)?;

	Ok(LoginResult {
		auth_token: secret,
		expires_in: TOKEN_TTL_SECS,
	})
}

pub(crate) async fn mint_token(
	ctx: &AppState,
	user_id: &str,
	name: &str,
	scopes: Vec<String>,
	expires_in_seconds: Option<i64>,
) -> Result<TokenCreateResult, LiseurSyncError> {
	let token_id = Uuid::new_v4().to_string();
	let device_id = Uuid::new_v4().to_string();
	let secret = new_secret();
	let created_at = now_string();
	let expires_at = expires_in_seconds
		.filter(|seconds| *seconds > 0)
		.map(|seconds| {
			(Utc::now() + chrono::Duration::seconds(seconds))
				.to_rfc3339_opts(SecondsFormat::Nanos, true)
		});
	// The existing table predates nullable expiry values.  Keep a far-future
	// sentinel in that NOT NULL column while the wire response preserves the
	// OpenAPI null for an unbounded device token.
	let stored_expires_at = expires_at.clone().unwrap_or_else(|| {
		(Utc::now() + chrono::Duration::seconds(DEVICE_TOKEN_TTL_SECS))
			.to_rfc3339_opts(SecondsFormat::Nanos, true)
	});
	let scopes_json = serde_json::to_string(&scopes).map_err(internal)?;
	ctx_conn(ctx)
		.execute(db_statement(
			ctx_conn(ctx),
			"INSERT INTO liseur_sync_tokens
                (id, user_id, device_id, secret_hash, scopes, created_at, expires_at,
                 name, token_kind)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
			vec![
				token_id.clone().into(),
				user_id.to_owned().into(),
				device_id.clone().into(),
				hash_secret(&secret).into(),
				scopes_json.into(),
				created_at.into(),
				stored_expires_at.into(),
				name.to_owned().into(),
				"device".into(),
			],
		))
		.await
		.map_err(internal)?;

	let scope = (scopes.len() == 1).then(|| scopes[0].clone());
	Ok(TokenCreateResult {
		token_id,
		device_id,
		name: name.to_owned(),
		scope,
		scopes,
		secret,
		expires_at,
	})
}

pub(crate) async fn revoke_token(
	ctx: &AppState,
	user_id: &str,
	token_id: &str,
) -> Result<(), LiseurSyncError> {
	let revoked_at = now_string();
	let result = ctx_conn(ctx)
		.execute(db_statement(
			ctx_conn(ctx),
			"UPDATE liseur_sync_tokens
             SET revoked_at = $1
             WHERE id = $2 AND user_id = $3 AND token_kind = 'device'
               AND revoked_at IS NULL",
			vec![
				revoked_at.into(),
				token_id.to_owned().into(),
				user_id.to_owned().into(),
			],
		))
		.await
		.map_err(internal)?;
	if result.rows_affected() == 0 {
		return Err(LiseurSyncError::NotFound("token not found".into()));
	}
	Ok(())
}
pub(crate) async fn settings(
	ctx: &AppState,
	user_id: &str,
) -> Result<BTreeMap<String, SettingValue>, LiseurSyncError> {
	let rows = ctx_conn(ctx)
		.query_all(db_statement(
			ctx_conn(ctx),
			"SELECT setting_key, value, updated_at
             FROM liseur_sync_settings WHERE user_id = $1
             ORDER BY setting_key",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	let mut settings = BTreeMap::new();
	for row in rows {
		settings.insert(
			row.try_get("", "setting_key").map_err(internal)?,
			SettingValue {
				value: row.try_get("", "value").map_err(internal)?,
				updated_at: row.try_get("", "updated_at").map_err(internal)?,
			},
		);
	}
	Ok(settings)
}

pub(crate) async fn put_settings(
	ctx: &AppState,
	user_id: &str,
	mut settings: Vec<SettingUpdate>,
) -> Result<(), LiseurSyncError> {
	if settings.is_empty() {
		return Err(LiseurSyncError::BadRequest("no settings provided".into()));
	}
	if settings.len() > MAX_SETTINGS_PER_ACCOUNT {
		return Err(LiseurSyncError::BadRequest(
			"too many settings in one request".into(),
		));
	}
	settings.sort_by(|left, right| left.key.cmp(&right.key));

	let conn = ctx_conn(ctx);
	let txn = begin_write(conn).await.map_err(internal)?;
	ensure_counter(&txn, user_id).await?;
	txn.execute(db_statement(
		&txn,
		"UPDATE liseur_sync_counters SET op_seq = op_seq WHERE user_id = $1",
		vec![user_id.to_owned().into()],
	))
	.await
	.map_err(internal)?;
	let rows = txn
		.query_all(db_statement(
			&txn,
			"SELECT setting_key FROM liseur_sync_settings WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	let existing = rows
		.iter()
		.map(|row| row.try_get::<String>("", "setting_key").map_err(internal))
		.collect::<Result<HashSet<_>, _>>()?;
	let new_keys = settings
		.iter()
		.filter(|setting| !existing.contains(&setting.key))
		.map(|setting| setting.key.as_str())
		.collect::<HashSet<_>>();
	if existing.len() + new_keys.len() > MAX_SETTINGS_PER_ACCOUNT {
		return Err(LiseurSyncError::Conflict(
			"too many settings for this account".into(),
		));
	}

	for setting in settings {
		txn.execute(db_statement(
			&txn,
			"INSERT INTO liseur_sync_settings
                (user_id, setting_key, value, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(user_id, setting_key) DO UPDATE
             SET value = excluded.value, updated_at = excluded.updated_at
             WHERE excluded.updated_at > liseur_sync_settings.updated_at",
			vec![
				user_id.to_owned().into(),
				setting.key.into(),
				setting.value.into(),
				setting.updated_at.into(),
			],
		))
		.await
		.map_err(internal)?;
	}
	txn.commit().await.map_err(internal)?;
	Ok(())
}

pub(crate) async fn authenticate(
	ctx: &AppState,
	token: &str,
) -> Result<LiseurToken, LiseurSyncError> {
	let Some(row) = ctx_conn(ctx)
		.query_one(db_statement(
			ctx_conn(ctx),
			"SELECT id, user_id, device_id, name, scopes, expires_at, token_kind
             FROM liseur_sync_tokens
             WHERE secret_hash = $1 AND revoked_at IS NULL",
			vec![hash_secret(token).into()],
		))
		.await
		.map_err(internal)?
	else {
		return Err(LiseurSyncError::Unauthorized);
	};

	let token_id: String = row.try_get("", "id").map_err(internal)?;
	let user_id: String = row.try_get("", "user_id").map_err(internal)?;
	let device_id: String = row.try_get("", "device_id").map_err(internal)?;
	let name: Option<String> = row.try_get("", "name").map_err(internal)?;
	let scopes_json: String = row.try_get("", "scopes").map_err(internal)?;
	let expires_at: String = row.try_get("", "expires_at").map_err(internal)?;
	let token_kind: Option<String> = row.try_get("", "token_kind").map_err(internal)?;
	let expires_at = DateTime::parse_from_rfc3339(&expires_at).map_err(internal)?;
	if expires_at.with_timezone(&Utc) <= Utc::now() {
		return Err(LiseurSyncError::Unauthorized);
	}
	let scopes = serde_json::from_str::<Vec<String>>(&scopes_json).map_err(internal)?;

	let user = LoginUser::find_by_id(user_id)
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<LoginUser>()
		.one(ctx_conn(ctx))
		.await
		.map_err(internal)?
		.ok_or(LiseurSyncError::Unauthorized)?;
	if user.is_locked {
		return Err(LiseurSyncError::Forbidden("account is locked".into()));
	}

	let last_used_at = now_string();
	if let Err(error) = ctx_conn(ctx)
		.execute(db_statement(
			ctx_conn(ctx),
			"UPDATE liseur_sync_tokens SET last_used_at = $1 WHERE id = $2",
			vec![last_used_at.into(), token_id.clone().into()],
		))
		.await
	{
		tracing::warn!(
			?error,
			"failed to update liseur-sync token last-used timestamp"
		);
	}

	let mut context = AuthContext {
		user: AuthUser::from(user),
		api_key: None,
		device_id: None,
	};
	// Records the sighting on the device this token belongs to and narrows the
	// request to that device's library scope. A registry failure means the
	// device could not be determined, and a request whose visible library set
	// is unknown must not be served.
	crate::middleware::auth::bind_device(
		ctx,
		&mut context,
		CredentialRef::LiseurToken(&token_id),
		Protocol::Liseur,
	)
	.await
	.map_err(internal)?;

	let kind = if token_kind.as_deref() == Some("session") {
		LiseurTokenKind::LoginSession
	} else {
		LiseurTokenKind::Device
	};
	Ok(LiseurToken {
		context,
		device_id,
		name: name.unwrap_or_else(|| {
			if kind == LiseurTokenKind::LoginSession {
				"login".into()
			} else {
				"liseur-sync".into()
			}
		}),
		scopes,
		kind,
		token_id: Some(token_id),
	})
}

#[derive(Clone, Debug)]
struct MediaCandidate {
	id: String,
	pages: i32,
	sampled_hash: Option<String>,
	koreader_hash: Option<String>,
	path: String,
}

#[derive(Clone, Debug)]
struct EditionCandidate {
	edition_sha: String,
	sampled_hash: Option<String>,
	koreader_hash: Option<String>,
	media_id: Option<String>,
	page_count: Option<i64>,
	resolution_status: String,
}

async fn find_media_for_identifier(
	conn: &DatabaseConnection,
	identifier: &Identifier,
) -> Result<Option<MediaCandidate>, LiseurSyncError> {
	let (sql, values) = match identifier.kind.as_str() {
		// Stump's `media.hash` is a sampled fingerprint, not a full-file
		// SHA-256. A caller-provided digest is only verified when a stable
		// source identifier lets us open the corresponding media row.
		"sha256" => return Ok(None),
		"partial-md5" => (
			"SELECT id, pages, hash, koreader_hash, path
             FROM media WHERE deleted_at IS NULL AND koreader_hash = $1 LIMIT 1",
			vec![identifier.value.clone().into()],
		),
		"source" => (
			"SELECT id, pages, hash, koreader_hash, path
             FROM media WHERE deleted_at IS NULL AND id = $1 LIMIT 1",
			vec![identifier.value.clone().into()],
		),
		_ => return Ok(None),
	};
	let Some(row) = conn
		.query_one(db_statement(conn, sql, values))
		.await
		.map_err(internal)?
	else {
		return Ok(None);
	};
	Ok(Some(MediaCandidate {
		id: row.try_get("", "id").map_err(internal)?,
		pages: row.try_get("", "pages").map_err(internal)?,
		sampled_hash: row.try_get("", "hash").map_err(internal)?,
		koreader_hash: row.try_get("", "koreader_hash").map_err(internal)?,
		path: row.try_get("", "path").map_err(internal)?,
	}))
}

async fn full_file_sha256(path: String) -> Result<Option<String>, LiseurSyncError> {
	let result = tokio::task::spawn_blocking(move || -> io::Result<String> {
		let mut file = File::open(path)?;
		let mut hasher = Sha256::new();
		let mut buffer = [0_u8; 1024 * 1024];
		loop {
			let read = file.read(&mut buffer)?;
			if read == 0 {
				break;
			}
			hasher.update(&buffer[..read]);
		}
		Ok(format!("{:x}", hasher.finalize()))
	})
	.await
	.map_err(internal)?;
	match result {
		Ok(hash) => Ok(Some(hash)),
		Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
		Err(error) => Err(internal(error)),
	}
}

async fn alias_matches(
	conn: &DatabaseConnection,
	user_id: &str,
	identifier: &Identifier,
) -> Result<Option<(String, Option<String>)>, LiseurSyncError> {
	let Some(row) = conn
		.query_one(db_statement(
			conn,
			"SELECT work_id, edition_sha FROM liseur_sync_aliases
             WHERE user_id = $1 AND kind = $2 AND value = $3",
			vec![
				user_id.to_owned().into(),
				identifier.kind.clone().into(),
				identifier.value.clone().into(),
			],
		))
		.await
		.map_err(internal)?
	else {
		return Ok(None);
	};
	Ok(Some((
		row.try_get("", "work_id").map_err(internal)?,
		row.try_get("", "edition_sha").map_err(internal)?,
	)))
}

/// Return each work identified by a strong alias of this media. Catalog
/// resolution emits the raw media id as a source identifier; Komga-imported
/// identity aliases use `komga:`, and verified stored file hashes also count.
async fn strong_alias_work_ids<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	media_id: &str,
	koreader_hash: Option<&str>,
	sha256: Option<&str>,
) -> Result<Vec<String>, LiseurSyncError> {
	let rows = conn
		.query_all(db_statement(
			conn,
			"SELECT DISTINCT alias.work_id
             FROM liseur_sync_aliases AS alias
             WHERE alias.user_id = $1
               AND (
                    (alias.kind = 'source' AND alias.value IN ($2, 'komga:' || $2))
                 OR (alias.kind = 'partial-md5' AND $3 IS NOT NULL AND alias.value = $3)
                 OR (alias.kind = 'sha256' AND $4 IS NOT NULL AND alias.value = $4)
                 OR (alias.kind = 'sha256' AND EXISTS (
                       SELECT 1
                       FROM liseur_sync_editions AS edition
                       WHERE edition.user_id = alias.user_id
                         AND edition.work_id = alias.work_id
                         AND edition.media_id = $2
                         AND edition.edition_sha = alias.value
                    ))
                 OR (alias.kind = 'sha256' AND EXISTS (
                       SELECT 1
                       FROM media_locations AS location
                       WHERE location.media_id = $2
                         AND location.sha256 = alias.value
                    ))
               )",
			vec![
				user_id.to_owned().into(),
				media_id.to_owned().into(),
				koreader_hash.map(str::to_owned).into(),
				sha256.map(str::to_owned).into(),
			],
		))
		.await
		.map_err(internal)?;
	let mut work_ids = rows
		.into_iter()
		.map(|row| row.try_get("", "work_id").map_err(internal))
		.collect::<Result<Vec<_>, _>>()?;
	work_ids.sort();
	work_ids.dedup();
	Ok(work_ids)
}

async fn merge_aliasless_pair_work(
	ctx: &AppState,
	user_id: &str,
	media_id: &str,
	losing_work_id: &str,
	identity_work_id: &str,
	koreader_hash: Option<&str>,
	sha256: Option<&str>,
) -> Result<(), LiseurSyncError> {
	let tx = begin_write(ctx_conn(ctx)).await.map_err(internal)?;
	let link = tx
		.query_one(db_statement(
			&tx,
			"SELECT work_id, pair_status, pair_evidence
             FROM liseur_sync_media_links
             WHERE user_id = $1 AND media_id = $2",
			vec![user_id.to_owned().into(), media_id.to_owned().into()],
		))
		.await
		.map_err(internal)?
		.ok_or_else(|| {
			LiseurSyncError::NotFound("book link changed during resolution".into())
		})?;
	let current_work_id: String = link.try_get("", "work_id").map_err(internal)?;
	let pair_status: Option<String> =
		link.try_get("", "pair_status").map_err(internal)?;
	let pair_evidence: Option<String> =
		link.try_get("", "pair_evidence").map_err(internal)?;
	let alias_work_ids =
		strong_alias_work_ids(&tx, user_id, media_id, koreader_hash, sha256).await?;
	let unique_identity_match =
		alias_work_ids.len() == 1 && alias_work_ids[0].as_str() == identity_work_id;
	if current_work_id == identity_work_id && unique_identity_match {
		tx.commit().await.map_err(internal)?;
		return Ok(());
	}
	let has_alias = tx
		.query_one(db_statement(
			&tx,
			"SELECT id FROM liseur_sync_aliases
             WHERE user_id = $1 AND work_id = $2 LIMIT 1",
			vec![user_id.to_owned().into(), losing_work_id.to_owned().into()],
		))
		.await
		.map_err(internal)?
		.is_some();
	let pair_created = pair_evidence.is_some()
		&& matches!(
			pair_status.as_deref().unwrap_or("suggested"),
			"suggested" | "confirmed"
		);
	if current_work_id == losing_work_id
		&& !has_alias
		&& pair_created
		&& unique_identity_match
	{
		migrations::merge_liseur_work(&tx, user_id, losing_work_id, identity_work_id)
			.await
			.map_err(internal)?;
		tx.commit().await.map_err(internal)?;
		return Ok(());
	}

	let mut works = alias_work_ids;
	works.push(current_work_id);
	works.sort();
	works.dedup();
	Err(LiseurSyncError::IdentityConflict(works))
}

fn normalized_identifier(identifier: &Identifier) -> Identifier {
	let kind = identifier.kind.trim().to_owned();
	let value = match kind.as_str() {
		"sha256" | "partial-md5" => identifier.value.trim().to_ascii_lowercase(),
		_ => identifier.value.trim().to_owned(),
	};
	Identifier { kind, value }
}

pub(crate) async fn resolve_work(
	ctx: &AppState,
	user_id: &str,
	request: ResolveRequest,
) -> Result<ResolveResult, LiseurSyncError> {
	let conn = ctx_conn(ctx);
	let identifiers: Vec<Identifier> = request
		.identifiers
		.iter()
		.map(normalized_identifier)
		.collect();

	let mut matched_work_ids = Vec::new();
	for identifier in &identifiers {
		if let Some((work_id, _edition_sha)) =
			alias_matches(conn, user_id, identifier).await?
		{
			matched_work_ids.push(work_id);
		}
	}

	matched_work_ids.sort();
	matched_work_ids.dedup();
	if matched_work_ids.len() > 1 {
		return Err(LiseurSyncError::IdentityConflict(matched_work_ids));
	}
	let existing_work_id = matched_work_ids.first().cloned();
	let strong_match = identifiers.iter().any(|identifier| identifier.kind != "ta");
	if let Some(work_id) = existing_work_id
		.as_ref()
		.filter(|_| !strong_match && !request.confirmed)
	{
		return Ok(ResolveResult {
			work_id: work_id.clone(),
			confidence: "low".into(),
			created: false,
		});
	}

	let work_id = existing_work_id
		.clone()
		.unwrap_or_else(|| Uuid::new_v4().to_string());
	let now = now_string();
	let title = request.title.unwrap_or_default();
	let author = request.author.unwrap_or_default();

	// Re-fetch media once per identifier and compute the full digest before
	// opening the graph transaction.  The existing `media.hash` is a sampled
	// Stump fingerprint; it is retained as evidence but never presented as a
	// verified full-file edition SHA when the bytes can be read.
	let mut editions = Vec::new();
	let mut seen_edition_shas = HashSet::new();
	for identifier in &identifiers {
		let Some(media) = find_media_for_identifier(conn, identifier).await? else {
			if identifier.kind == "sha256"
				&& seen_edition_shas.insert(identifier.value.clone())
			{
				editions.push(EditionCandidate {
					edition_sha: identifier.value.clone(),
					sampled_hash: None,
					koreader_hash: None,
					media_id: None,
					page_count: None,
					resolution_status: "unverified".into(),
				});
			}
			continue;
		};
		let full_sha = full_file_sha256(media.path.clone()).await?;
		let verified = full_sha.is_some();
		let Some(edition_sha) = full_sha
			.or_else(|| (identifier.kind == "sha256").then(|| identifier.value.clone()))
		else {
			continue;
		};
		let resolution_status = if verified { "verified" } else { "unverified" };
		let candidate = EditionCandidate {
			edition_sha: edition_sha.clone(),
			sampled_hash: media.sampled_hash,
			koreader_hash: media.koreader_hash,
			media_id: Some(media.id),
			page_count: (media.pages >= 0).then_some(i64::from(media.pages)),
			resolution_status: resolution_status.into(),
		};
		if let Some(existing_index) = editions
			.iter()
			.position(|edition| edition.edition_sha == edition_sha)
		{
			// A full digest may arrive before the source identifier. Keep the
			// same edition, but upgrade its provenance once the media row is
			// available and the bytes verify.
			if verified && editions[existing_index].resolution_status != "verified" {
				editions[existing_index] = candidate;
			}
			continue;
		}
		seen_edition_shas.insert(edition_sha);
		editions.push(candidate);
	}

	let txn = begin_write(conn).await.map_err(internal)?;
	let existing = txn
		.query_one(db_statement(
			&txn,
			"SELECT id FROM liseur_sync_works WHERE user_id = $1 AND id = $2",
			vec![user_id.to_owned().into(), work_id.clone().into()],
		))
		.await
		.map_err(internal)?;
	if existing.is_none() {
		txn.execute(db_statement(
			&txn,
			"INSERT INTO liseur_sync_works
                (id, user_id, title, author, pending, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
			vec![
				work_id.clone().into(),
				user_id.to_owned().into(),
				title.into(),
				author.into(),
				false.into(),
				now.clone().into(),
			],
		))
		.await
		.map_err(internal)?;
	} else if !title.is_empty() || !author.is_empty() {
		txn.execute(db_statement(
			&txn,
			"UPDATE liseur_sync_works
             SET title = CASE WHEN $1 <> '' THEN $2 ELSE title END,
                 author = CASE WHEN $3 <> '' THEN $4 ELSE author END,
                 pending = FALSE
             WHERE user_id = $5 AND id = $6",
			vec![
				title.clone().into(),
				title.into(),
				author.clone().into(),
				author.into(),
				user_id.to_owned().into(),
				work_id.clone().into(),
			],
		))
		.await
		.map_err(internal)?;
	}

	for edition in &editions {
		let existing_edition = txn
			.query_one(db_statement(
				&txn,
				"SELECT work_id FROM liseur_sync_editions
                 WHERE user_id = $1 AND edition_sha = $2",
				vec![
					user_id.to_owned().into(),
					edition.edition_sha.clone().into(),
				],
			))
			.await
			.map_err(internal)?;
		if let Some(row) = existing_edition {
			let mapped_work: String = row.try_get("", "work_id").map_err(internal)?;
			if mapped_work != work_id {
				return Err(LiseurSyncError::IdentityConflict(vec![
					mapped_work,
					work_id.clone(),
				]));
			}
		} else {
			txn.execute(db_statement(
				&txn,
				"INSERT INTO liseur_sync_editions
                    (id, user_id, edition_sha, sampled_hash, koreader_hash,
                     work_id, media_id, page_count, char_count, metadata, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
				vec![
					Uuid::new_v4().to_string().into(),
					user_id.to_owned().into(),
					edition.edition_sha.clone().into(),
					edition.sampled_hash.clone().into(),
					edition.koreader_hash.clone().into(),
					work_id.clone().into(),
					edition.media_id.clone().into(),
					edition.page_count.into(),
					Option::<i64>::None.into(),
					serde_json::json!({"resolution_status": edition.resolution_status})
						.to_string()
						.into(),
					now.clone().into(),
				],
			))
			.await
			.map_err(internal)?;
		}
	}

	for identifier in &identifiers {
		let edition_sha = if identifier.kind == "sha256" {
			editions
				.iter()
				.find(|edition| {
					edition.edition_sha == identifier.value
						|| edition.sampled_hash.as_deref()
							== Some(identifier.value.as_str())
				})
				.map(|edition| edition.edition_sha.clone())
		} else if identifier.kind == "partial-md5" {
			editions
				.iter()
				.find(|edition| {
					edition.koreader_hash.as_deref() == Some(identifier.value.as_str())
				})
				.map(|edition| edition.edition_sha.clone())
		} else {
			None
		};
		let existing_alias = txn
			.query_one(db_statement(
				&txn,
				"SELECT work_id, edition_sha FROM liseur_sync_aliases
                 WHERE user_id = $1 AND kind = $2 AND value = $3",
				vec![
					user_id.to_owned().into(),
					identifier.kind.clone().into(),
					identifier.value.clone().into(),
				],
			))
			.await
			.map_err(internal)?;
		if let Some(row) = existing_alias {
			let mapped_work: String = row.try_get("", "work_id").map_err(internal)?;
			if mapped_work != work_id {
				return Err(LiseurSyncError::IdentityConflict(vec![
					mapped_work,
					work_id.clone(),
				]));
			}
			let stored_edition: Option<String> =
				row.try_get("", "edition_sha").map_err(internal)?;
			if stored_edition.is_none() {
				if let Some(edition_sha) = edition_sha.as_deref() {
					txn.execute(db_statement(
						&txn,
						"UPDATE liseur_sync_aliases
                         SET edition_sha = $1
                         WHERE user_id = $2 AND kind = $3 AND value = $4",
						vec![
							edition_sha.to_owned().into(),
							user_id.to_owned().into(),
							identifier.kind.clone().into(),
							identifier.value.clone().into(),
						],
					))
					.await
					.map_err(internal)?;
				}
			}
		} else {
			txn.execute(db_statement(
				&txn,
				"INSERT INTO liseur_sync_aliases
                    (id, user_id, kind, value, work_id, edition_sha, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
				vec![
					Uuid::new_v4().to_string().into(),
					user_id.to_owned().into(),
					identifier.kind.clone().into(),
					identifier.value.clone().into(),
					work_id.clone().into(),
					edition_sha.into(),
					now.clone().into(),
				],
			))
			.await
			.map_err(internal)?;
		}
	}
	for edition in &editions {
		let Some(media_id) = &edition.media_id else {
			continue;
		};
		let existing_link = txn
			.query_one(db_statement(
				&txn,
				"SELECT id, work_id FROM liseur_sync_media_links
                 WHERE user_id = $1 AND media_id = $2",
				vec![user_id.to_owned().into(), media_id.clone().into()],
			))
			.await
			.map_err(internal)?;
		if let Some(row) = existing_link {
			let id: String = row.try_get("", "id").map_err(internal)?;
			let mapped_work: String = row.try_get("", "work_id").map_err(internal)?;
			if mapped_work != work_id {
				return Err(LiseurSyncError::IdentityConflict(vec![
					mapped_work,
					work_id.clone(),
				]));
			}
			txn.execute(db_statement(
				&txn,
				"UPDATE liseur_sync_media_links
                 SET edition_sha = $1, resolution_status = $2
                 WHERE id = $3",
				vec![
					edition.edition_sha.clone().into(),
					edition.resolution_status.clone().into(),
					id.into(),
				],
			))
			.await
			.map_err(internal)?;
		} else {
			txn.execute(db_statement(
                &txn,
                "INSERT INTO liseur_sync_media_links
                    (id, user_id, media_id, work_id, edition_sha, resolution_status, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                vec![
                    Uuid::new_v4().to_string().into(),
                    user_id.to_owned().into(),
                    media_id.clone().into(),
                    work_id.clone().into(),
                    edition.edition_sha.clone().into(),
                    edition.resolution_status.clone().into(),
                    now.clone().into(),
                ],
            ))
            .await
            .map_err(internal)?;
		}
	}

	txn.execute(db_statement(
		&txn,
		"INSERT INTO liseur_sync_counters (user_id, op_seq, annotation_seq)
         VALUES ($1, 0, 0) ON CONFLICT(user_id) DO NOTHING",
		vec![user_id.to_owned().into()],
	))
	.await
	.map_err(internal)?;
	reconcile_native_annotations(&txn, user_id).await?;
	reconcile_annotation_projections(&txn, user_id).await?;
	txn.commit().await.map_err(internal)?;

	Ok(ResolveResult {
		work_id,
		confidence: "high".into(),
		created: existing_work_id.is_none(),
	})
}

async fn work_exists<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	work_id: &str,
) -> Result<bool, LiseurSyncError> {
	Ok(conn
		.query_one(db_statement(
			conn,
			"SELECT id FROM liseur_sync_works WHERE user_id = $1 AND id = $2",
			vec![user_id.to_owned().into(), work_id.to_owned().into()],
		))
		.await
		.map_err(internal)?
		.is_some())
}

async fn ensure_counter<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<(), LiseurSyncError> {
	conn.execute(db_statement(
		conn,
		"INSERT INTO liseur_sync_counters (user_id, op_seq, annotation_seq)
         VALUES ($1, 0, 0) ON CONFLICT(user_id) DO NOTHING",
		vec![user_id.to_owned().into()],
	))
	.await
	.map_err(internal)?;
	Ok(())
}

async fn next_op_seq<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<i64, LiseurSyncError> {
	conn.execute(db_statement(
		conn,
		"UPDATE liseur_sync_counters SET op_seq = op_seq + 1 WHERE user_id = $1",
		vec![user_id.to_owned().into()],
	))
	.await
	.map_err(internal)?;
	let row = conn
		.query_one(db_statement(
			conn,
			"SELECT op_seq FROM liseur_sync_counters WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?
		.ok_or_else(|| internal("liseur-sync counter disappeared"))?;
	row.try_get("", "op_seq").map_err(internal)
}

async fn next_annotation_seq<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<i64, LiseurSyncError> {
	conn.execute(db_statement(
		conn,
		"UPDATE liseur_sync_counters
         SET annotation_seq = annotation_seq + 1 WHERE user_id = $1",
		vec![user_id.to_owned().into()],
	))
	.await
	.map_err(internal)?;
	let row = conn
		.query_one(db_statement(
			conn,
			"SELECT annotation_seq FROM liseur_sync_counters WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?
		.ok_or_else(|| internal("liseur-sync counter disappeared"))?;
	row.try_get("", "annotation_seq").map_err(internal)
}

fn locator_from_db(raw: Option<String>) -> Option<Value> {
	raw.and_then(|value| serde_json::from_str(&value).ok())
}

fn locator_to_db(locator: &Option<Value>) -> Result<Option<String>, LiseurSyncError> {
	locator
		.as_ref()
		.map(serde_json::to_string)
		.transpose()
		.map_err(internal)
}

fn op_record(row: &QueryResult) -> Result<OpRecord, LiseurSyncError> {
	Ok(OpRecord {
		op_id: row.try_get("", "op_id").map_err(internal)?,
		work_id: row.try_get("", "work_id").map_err(internal)?,
		edition_sha: row.try_get("", "edition_sha").map_err(internal)?,
		client_ts: row.try_get("", "client_ts").map_err(internal)?,
		progression: row.try_get("", "progression").map_err(internal)?,
		locator: locator_from_db(row.try_get("", "locator").map_err(internal)?),
		foreign_pos: row.try_get("", "foreign_pos").map_err(internal)?,
		seq: row.try_get("", "seq").map_err(internal)?,
		device_id: row.try_get("", "device_id").map_err(internal)?,
		origin: row.try_get("", "origin").map_err(internal)?,
		received_at: row.try_get("", "received_at").map_err(internal)?,
	})
}

const OP_COLUMNS: &str = "op_id, work_id, edition_sha, client_ts, progression,
    locator, foreign_pos, seq, device_id, origin, received_at";

async fn find_op<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	op_id: &str,
) -> Result<Option<QueryResult>, LiseurSyncError> {
	conn.query_one(db_statement(
		conn,
		&format!(
			"SELECT {OP_COLUMNS} FROM liseur_sync_ops
             WHERE user_id = $1 AND op_id = $2"
		),
		vec![user_id.to_owned().into(), op_id.to_owned().into()],
	))
	.await
	.map_err(internal)
}

fn op_matches(row: &QueryResult, op: &OpInput) -> Result<bool, LiseurSyncError> {
	let stored_locator: Option<String> = row.try_get("", "locator").map_err(internal)?;
	let incoming_locator = locator_to_db(&op.locator)?;
	Ok(
		row.try_get::<String>("", "work_id").map_err(internal)? == op.work_id
			&& row
				.try_get::<Option<String>>("", "edition_sha")
				.map_err(internal)?
				== op.edition_sha
			&& row.try_get::<String>("", "client_ts").map_err(internal)? == op.client_ts
			&& (row.try_get::<f64>("", "progression").map_err(internal)?
				- op.progression.expect("validated progression"))
			.abs() < f64::EPSILON
			&& stored_locator == incoming_locator
			&& row
				.try_get::<Option<String>>("", "foreign_pos")
				.map_err(internal)?
				== op.foreign_pos,
	)
}

pub(crate) async fn append_ops(
	ctx: &AppState,
	user_id: &str,
	device_id: &str,
	ops: Vec<OpInput>,
) -> Result<Vec<OpResult>, LiseurSyncError> {
	let conn = ctx_conn(ctx);
	let txn = begin_write(conn).await.map_err(internal)?;
	ensure_counter(&txn, user_id).await?;
	let mut results = Vec::with_capacity(ops.len());
	for (item_index, op) in ops.into_iter().enumerate() {
		if !work_exists(&txn, user_id, &op.work_id).await? {
			return Err(LiseurSyncError::ItemRefusal {
				status: axum::http::StatusCode::BAD_REQUEST,
				code: "unknown_work",
				message: format!("unknown work: {}", op.work_id),
				item_index: Some(item_index),
				session_id: None,
				op_id: Some(op.op_id.clone()),
				work_id: Some(op.work_id.clone()),
				limit: None,
			});
		}
		if let Some(existing) = find_op(&txn, user_id, &op.op_id).await? {
			let seq: i64 = existing.try_get("", "seq").map_err(internal)?;
			if op_matches(&existing, &op)? {
				results.push(OpResult {
					op_id: op.op_id,
					status: "duplicate".into(),
					seq: Some(seq),
					reason: None,
				});
			} else {
				results.push(OpResult {
					op_id: op.op_id,
					status: "conflict".into(),
					seq: None,
					reason: Some(
						"op_id was already used with a different payload".into(),
					),
				});
			}
			continue;
		}
		let seq = next_op_seq(&txn, user_id).await?;
		let received_at = now_string();
		txn.execute(db_statement(
			&txn,
			"INSERT INTO liseur_sync_ops
                (id, user_id, seq, op_id, work_id, edition_sha, device_id,
                 client_ts, progression, locator, foreign_pos, origin, received_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
			vec![
				Uuid::new_v4().to_string().into(),
				user_id.to_owned().into(),
				seq.into(),
				op.op_id.clone().into(),
				op.work_id.clone().into(),
				op.edition_sha.clone().into(),
				device_id.to_owned().into(),
				op.client_ts.clone().into(),
				op.progression.expect("validated progression").into(),
				locator_to_db(&op.locator)?.into(),
				op.foreign_pos.clone().into(),
				"native".into(),
				received_at.into(),
			],
		))
		.await
		.map_err(internal)?;
		results.push(OpResult {
			op_id: op.op_id.clone(),
			status: "applied".into(),
			seq: Some(seq),
			reason: None,
		});
		apply_op_to_head(&txn, user_id, device_id, &op).await?;
	}
	txn.commit().await.map_err(internal)?;
	record_device_sync(ctx, user_id, device_id, &results).await;
	Ok(results)
}

/// Records an accepted ops push as a sync on the device the liseur token is
/// bound to. Registry-minted tokens carry the device id, and a token minted by
/// the liseur login flow is bound to no device, so a miss is silent; a
/// registry failure never fails the push.
async fn record_device_sync(
	ctx: &AppState,
	user_id: &str,
	device_id: &str,
	results: &[OpResult],
) {
	let token_id = match ctx_conn(ctx)
		.query_one(db_statement(
			ctx_conn(ctx),
			"SELECT id FROM liseur_sync_tokens
             WHERE user_id = $1 AND device_id = $2 AND revoked_at IS NULL
             LIMIT 1",
			vec![user_id.to_owned().into(), device_id.to_owned().into()],
		))
		.await
		.map(|row| row.and_then(|row| row.try_get::<String>("", "id").ok()))
	{
		Ok(Some(token_id)) => token_id,
		Ok(None) => return,
		Err(error) => {
			tracing::warn!(?error, "failed to resolve the liseur token of a device");
			return;
		},
	};
	let count = |status: &str| results.iter().filter(|op| op.status == status).count();
	let summary = serde_json::json!({
		"protocol": "liseur",
		"ops": results.len(),
		"applied": count("applied"),
		"duplicate": count("duplicate"),
		"conflict": count("conflict"),
	});
	if let Err(error) = ctx
		.devices()
		.touch(
			CredentialRef::LiseurToken(&token_id),
			Protocol::Liseur,
			Some(summary),
		)
		.await
	{
		tracing::warn!(?error, "failed to record the liseur ops push on its device");
	}
}

async fn high_water<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<i64, LiseurSyncError> {
	let row = conn
		.query_one(db_statement(
			conn,
			"SELECT op_seq FROM liseur_sync_counters WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	Ok(row
		.map(|row| row.try_get("", "op_seq"))
		.transpose()
		.map_err(internal)?
		.unwrap_or(0))
}

pub(crate) async fn changes(
	ctx: &AppState,
	user_id: &str,
	since: i64,
	limit: usize,
) -> Result<ChangesPage, LiseurSyncError> {
	mirror_heads(ctx, user_id).await?;
	let conn = ctx_conn(ctx);
	let high_water = high_water(conn, user_id).await?;
	let rows = conn
		.query_all(db_statement(
			conn,
			&format!(
				"SELECT {OP_COLUMNS} FROM liseur_sync_ops
                 WHERE user_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3"
			),
			vec![
				user_id.to_owned().into(),
				since.into(),
				((limit + 1) as i64).into(),
			],
		))
		.await
		.map_err(internal)?;
	let has_more = rows.len() > limit;
	let ops = rows
		.into_iter()
		.take(limit)
		.map(|row| op_record(&row))
		.collect::<Result<Vec<_>, _>>()?;
	Ok(ChangesPage {
		ops,
		high_water,
		has_more,
	})
}

pub(crate) async fn heads(
	ctx: &AppState,
	user_id: &str,
) -> Result<HeadsPage, LiseurSyncError> {
	mirror_heads(ctx, user_id).await?;
	let conn = ctx_conn(ctx);
	let snapshot_seq = high_water(conn, user_id).await?;
	let rows = conn
		.query_all(db_statement(
			conn,
			&format!(
				"SELECT {OP_COLUMNS} FROM liseur_sync_ops
                 WHERE user_id = $1 ORDER BY seq DESC"
			),
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	let mut seen = HashSet::new();
	let mut ops = Vec::new();
	for row in rows {
		let work_id: String = row.try_get("", "work_id").map_err(internal)?;
		let device_id: String = row.try_get("", "device_id").map_err(internal)?;
		if seen.insert((work_id, device_id)) {
			ops.push(op_record(&row)?);
		}
	}
	Ok(HeadsPage { ops, snapshot_seq })
}

pub(crate) async fn positions(
	ctx: &AppState,
	user_id: &str,
	work_id: &str,
	limit: usize,
) -> Result<Vec<OpRecord>, LiseurSyncError> {
	mirror_heads(ctx, user_id).await?;
	let conn = ctx_conn(ctx);
	if !work_exists(conn, user_id, work_id).await? {
		return Err(LiseurSyncError::NotFound("work not found".into()));
	}
	let rows = conn
		.query_all(db_statement(
			conn,
			&format!(
				"SELECT {OP_COLUMNS} FROM liseur_sync_ops
                 WHERE user_id = $1 AND work_id = $2 ORDER BY seq DESC LIMIT $3"
			),
			vec![
				user_id.to_owned().into(),
				work_id.to_owned().into(),
				(limit as i64).into(),
			],
		))
		.await
		.map_err(internal)?;
	rows.iter().map(op_record).collect()
}

/// Materialize one immutable liseur closed session in the native activity
/// history. The source row is written by [`append_sessions`] in the same
/// transaction, so a projection failure rolls back both writes.
async fn project_liseur_session<C: ConnectionTrait>(
	txn: &C,
	user_id: &str,
	device_id: &str,
	session: &SessionInput,
) -> Result<(), LiseurSyncError> {
	let Some(media) = linked_media(txn, user_id, &session.work_id).await? else {
		// Keep an unresolved source session immutable; a later retry can
		// project it once its work is linked to Stump media.
		return Ok(());
	};

	let already_projected = reading_session::Entity::find()
		.filter(reading_session::Column::UserId.eq(user_id))
		.filter(reading_session::Column::LiseurSessionId.eq(session.session_id.clone()))
		.one(txn)
		.await
		.map_err(internal)?
		.is_some();
	if already_projected {
		return Ok(());
	}

	let started_at =
		DateTime::parse_from_rfc3339(&session.started_at).map_err(internal)?;
	let ended_at = DateTime::parse_from_rfc3339(&session.ended_at).map_err(internal)?;
	let elapsed_millis = session
		.active_ms
		.unwrap_or_else(|| (ended_at - started_at).num_milliseconds() - session.idle_ms);
	let elapsed_seconds = elapsed_millis / 1000;
	let day_reset_hour_offset = user_preferences::Entity::find()
		.filter(user_preferences::Column::UserId.eq(user_id))
		.one(txn)
		.await
		.map_err(internal)?
		.map(|preferences| preferences.day_reset_hour_offset)
		.unwrap_or_default();
	let start_percentage =
		Decimal::from_f64_retain(session.start_progression.ok_or_else(|| {
			internal("validated liseur session has no start progression")
		})?)
		.ok_or_else(|| internal("invalid liseur start progression"))?;
	let end_percentage =
		Decimal::from_f64_retain(session.end_progression.ok_or_else(|| {
			internal("validated liseur session has no end progression")
		})?)
		.ok_or_else(|| internal("invalid liseur end progression"))?;
	let readthrough_number = derive_readthrough_number(txn, user_id, &media.id)
		.await
		.map_err(internal)?;

	let projected = reading_session::ActiveModel {
		session_date: Set(calculate_logical_date(
			started_at.with_timezone(&Utc),
			day_reset_hour_offset,
		)),
		start_percentage: Set(Some(start_percentage)),
		end_percentage: Set(Some(end_percentage)),
		elapsed_seconds: Set(Some(elapsed_seconds)),
		readthrough_number: Set(readthrough_number),
		status: Set(if session.end_progression.unwrap_or_default() >= 1.0 {
			ReadingStatus::Finished
		} else {
			ReadingStatus::Reading
		}),
		device_ids: Set(Some(reading_session::DeviceIds(vec![device_id.to_owned()]))),
		liseur_session_id: Set(Some(session.session_id.clone())),
		media_id: Set(media.id),
		user_id: Set(user_id.to_owned()),
		created_at: Set(started_at.clone()),
		updated_at: Set(Some(ended_at.clone())),
		..Default::default()
	}
	.insert(txn)
	.await
	.map_err(internal)?;

	// `reading_session::ActiveModelBehavior` stamps native writes with now.
	// Restore the source timestamps after the insert so this immutable
	// projection retains the closed-session interval.
	txn.execute(db_statement(
		txn,
		"UPDATE reading_sessions SET created_at = $1, updated_at = $2 WHERE id = $3",
		vec![started_at.into(), ended_at.into(), projected.id.into()],
	))
	.await
	.map_err(internal)?;

	Ok(())
}
pub(crate) async fn append_sessions(
	ctx: &AppState,
	user_id: &str,
	device_id: &str,
	sessions: Vec<SessionInput>,
) -> Result<usize, LiseurSyncError> {
	let conn = ctx_conn(ctx);
	let txn = begin_write(conn).await.map_err(internal)?;
	let mut accepted = 0;
	for (item_index, session) in sessions.into_iter().enumerate() {
		if !work_exists(&txn, user_id, &session.work_id).await? {
			return Err(LiseurSyncError::ItemRefusal {
				status: axum::http::StatusCode::BAD_REQUEST,
				code: "unknown_work",
				message: format!("unknown work: {}", session.work_id),
				item_index: Some(item_index),
				session_id: Some(session.session_id.clone()),
				op_id: None,
				work_id: Some(session.work_id.clone()),
				limit: None,
			});
		}
		let payload = serde_json::to_string(&session).map_err(internal)?;
		let existing = txn
			.query_one(db_statement(
				&txn,
				"SELECT payload FROM liseur_sync_sessions
                 WHERE user_id = $1 AND session_id = $2",
				vec![user_id.to_owned().into(), session.session_id.clone().into()],
			))
			.await
			.map_err(internal)?;
		if let Some(row) = existing {
			let stored: String = row.try_get("", "payload").map_err(internal)?;
			if stored != payload {
				return Err(LiseurSyncError::ItemRefusal {
					status: axum::http::StatusCode::CONFLICT,
					code: "id_reused",
					message: "session_id reused with a different payload".into(),
					item_index: Some(item_index),
					session_id: Some(session.session_id.clone()),
					op_id: None,
					work_id: None,
					limit: None,
				});
			}
			project_liseur_session(&txn, user_id, device_id, &session).await?;
			accepted += 1;
			continue;
		}
		txn.execute(db_statement(
			&txn,
			"INSERT INTO liseur_sync_sessions
                (id, user_id, session_id, work_id, edition_sha, device_id,
                 started_at, ended_at, start_progression, end_progression,
                 idle_ms, payload, received_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
			vec![
				Uuid::new_v4().to_string().into(),
				user_id.to_owned().into(),
				session.session_id.clone().into(),
				session.work_id.clone().into(),
				session.edition_sha.clone().into(),
				device_id.to_owned().into(),
				session.started_at.clone().into(),
				session.ended_at.clone().into(),
				session
					.start_progression
					.expect("validated progression")
					.into(),
				session
					.end_progression
					.expect("validated progression")
					.into(),
				session.idle_ms.into(),
				payload.into(),
				now_string().into(),
			],
		))
		.await
		.map_err(internal)?;
		project_liseur_session(&txn, user_id, device_id, &session).await?;
		accepted += 1;
	}
	txn.commit().await.map_err(internal)?;
	Ok(accepted)
}

#[derive(Clone, Debug)]
struct StoredAnnotation {
	id: String,
	rev: i64,
	seq: i64,
	work_id: String,
	edition_sha: Option<String>,
	kind: String,
	locator: Option<Value>,
	progression: Option<f64>,
	excerpt: String,
	color: String,
	drawer: Option<String>,
	body: String,
	device_id: String,
	client_ts: String,
	updated_at: String,
	deleted: bool,
	deleted_at: Option<String>,
	payload: String,
}

#[derive(Clone, Debug)]
struct NativeMediaLink {
	work_id: String,
	edition_sha: Option<String>,
}

#[derive(Clone, Debug)]
struct NativeAnnotationCandidate {
	id: String,
	work_id: String,
	edition_sha: Option<String>,
	kind: String,
	locator: Value,
	progression: Option<f64>,
	excerpt: String,
	color: String,
	body: String,
	client_ts: String,
	updated_at: String,
}

impl NativeAnnotationCandidate {
	fn input(&self, base_rev: i64, client_ts: String) -> AnnotationInput {
		AnnotationInput {
			id: self.id.clone(),
			base_rev,
			work_id: self.work_id.clone(),
			edition_sha: self.edition_sha.clone(),
			kind: self.kind.clone(),
			locator: Some(self.locator.clone()),
			progression: self.progression,
			excerpt: self.excerpt.clone(),
			color: (!self.color.is_empty()).then(|| self.color.clone()),
			drawer: None,
			body: self.body.clone(),
			client_ts,
		}
	}
}

fn native_progression(locator: &ReadiumLocator) -> Option<f64> {
	let locations = locator.locations.as_ref()?;
	let progression = locations
		.total_progression
		.or(locations.progression)?
		.to_string()
		.parse::<f64>()
		.ok()?;
	(progression.is_finite() && (0.0..=1.0).contains(&progression)).then_some(progression)
}

fn native_timestamp(value: DateTime<Utc>) -> String {
	value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn native_candidate_matches(
	stored: &StoredAnnotation,
	candidate: &NativeAnnotationCandidate,
) -> bool {
	!stored.deleted
		&& stored.work_id == candidate.work_id
		&& stored.edition_sha == candidate.edition_sha
		&& stored.kind == candidate.kind
		&& stored.locator.as_ref() == Some(&candidate.locator)
		&& stored.progression == candidate.progression
		&& stored.excerpt == candidate.excerpt
		&& stored.color == candidate.color
		&& stored.body == candidate.body
		&& (stored.kind == "bookmark"
			|| timestamps_match(&stored.updated_at, &candidate.updated_at))
}

fn timestamps_match(left: &str, right: &str) -> bool {
	match (
		DateTime::parse_from_rfc3339(left),
		DateTime::parse_from_rfc3339(right),
	) {
		(Ok(left), Ok(right)) => left == right,
		_ => left == right,
	}
}

const ANNOTATION_COLUMNS: &str =
	"annotation_id AS id, rev, seq, work_id, edition_sha, kind,
    locator, progression, excerpt, color, drawer, body, device_id, client_ts,
    updated_at, deleted, deleted_at, payload";

fn annotation_record(annotation: &StoredAnnotation) -> AnnotationRecord {
	if annotation.deleted {
		return AnnotationRecord {
			id: annotation.id.clone(),
			rev: annotation.rev,
			seq: annotation.seq,
			work_id: Some(annotation.work_id.clone()),
			edition_sha: None,
			kind: None,
			locator: None,
			progression: None,
			excerpt: None,
			color: None,
			drawer: None,
			body: None,
			device_id: None,
			client_ts: None,
			updated_at: annotation.updated_at.clone(),
			deleted: true,
			deleted_at: annotation.deleted_at.clone(),
		};
	}
	AnnotationRecord {
		id: annotation.id.clone(),
		rev: annotation.rev,
		seq: annotation.seq,
		work_id: Some(annotation.work_id.clone()),
		edition_sha: annotation.edition_sha.clone(),
		kind: Some(annotation.kind.clone()),
		locator: annotation.locator.clone(),
		progression: annotation.progression,
		excerpt: (!annotation.excerpt.is_empty()).then(|| annotation.excerpt.clone()),
		color: (!annotation.color.is_empty()).then(|| annotation.color.clone()),
		body: (!annotation.body.is_empty()).then(|| annotation.body.clone()),
		drawer: annotation_drawer(annotation),
		device_id: Some(annotation.device_id.clone()),
		client_ts: Some(annotation.client_ts.clone()),
		updated_at: annotation.updated_at.clone(),
		deleted: false,
		deleted_at: None,
	}
}

fn stored_annotation(row: &QueryResult) -> Result<StoredAnnotation, LiseurSyncError> {
	Ok(StoredAnnotation {
		id: row.try_get("", "id").map_err(internal)?,
		rev: row.try_get("", "rev").map_err(internal)?,
		seq: row.try_get("", "seq").map_err(internal)?,
		work_id: row.try_get("", "work_id").map_err(internal)?,
		edition_sha: row.try_get("", "edition_sha").map_err(internal)?,
		kind: row.try_get("", "kind").map_err(internal)?,
		locator: locator_from_db(row.try_get("", "locator").map_err(internal)?),
		progression: row.try_get("", "progression").map_err(internal)?,
		excerpt: row.try_get("", "excerpt").map_err(internal)?,
		color: row.try_get("", "color").map_err(internal)?,
		drawer: row.try_get("", "drawer").map_err(internal)?,
		body: row.try_get("", "body").map_err(internal)?,
		device_id: row.try_get("", "device_id").map_err(internal)?,
		client_ts: row.try_get("", "client_ts").map_err(internal)?,
		updated_at: row.try_get("", "updated_at").map_err(internal)?,
		deleted: row.try_get("", "deleted").map_err(internal)?,
		deleted_at: row.try_get("", "deleted_at").map_err(internal)?,
		payload: row.try_get("", "payload").map_err(internal)?,
	})
}
fn annotation_drawer(annotation: &StoredAnnotation) -> Option<String> {
	if let Some(drawer) = annotation.drawer.as_deref() {
		return ANNOTATION_DRAWERS
			.contains(&drawer)
			.then(|| drawer.to_owned());
	}
	if let Ok(payload) = serde_json::from_str::<Value>(&annotation.payload) {
		if let Some(drawer) = payload.get("drawer") {
			return drawer
				.as_str()
				.filter(|drawer| ANNOTATION_DRAWERS.contains(drawer))
				.map(str::to_owned);
		}
	}
	let drawer = annotation.locator.as_ref()?.get("drawer")?.as_str()?;
	ANNOTATION_DRAWERS
		.contains(&drawer)
		.then(|| drawer.to_owned())
}

fn annotation_color_for_update(
	incoming: &AnnotationInput,
	stored: &StoredAnnotation,
) -> String {
	if incoming.kind != "highlight" {
		return String::new();
	}
	match incoming.color.as_deref() {
		Some(color) => color.to_owned(),
		None if ANNOTATION_COLORS.contains(&stored.color.as_str())
			&& !LISEUR_ANNOTATION_COLORS.contains(&stored.color.as_str()) =>
		{
			stored.color.clone()
		},
		None => String::new(),
	}
}

fn annotation_drawer_for_update(
	incoming: &AnnotationInput,
	stored: &StoredAnnotation,
) -> Option<String> {
	if incoming.kind != "highlight" {
		return None;
	}
	match incoming.drawer.as_deref() {
		Some("") => None,
		Some(drawer) => Some(drawer.to_owned()),
		None => annotation_drawer(stored),
	}
}

fn native_update_locator(
	kind: &str,
	incoming: &AnnotationInput,
	stored: &StoredAnnotation,
) -> Option<ReadiumLocator> {
	let value = incoming
		.locator
		.as_ref()
		.filter(|value| !value.is_null())
		.cloned()
		.or_else(|| stored.locator.clone())?;
	let mut locator = serde_json::from_value::<ReadiumLocator>(value).ok()?;
	if locator.href.trim().is_empty() || locator.locations.is_none() {
		return None;
	}
	match kind {
		"note" => {
			if let Some(text) = &mut locator.text {
				text.highlight = None;
			}
		},
		"highlight" => {
			if !incoming.excerpt.trim().is_empty() {
				locator
					.text
					.get_or_insert(ReadiumText {
						after: None,
						before: None,
						highlight: None,
					})
					.highlight = Some(incoming.excerpt.clone());
			}
			if locator
				.text
				.as_ref()
				.and_then(|text| text.highlight.as_deref())
				.map_or(true, |highlight| highlight.trim().is_empty())
			{
				return None;
			}
		},
		"bookmark" => {},
		_ => return None,
	}
	Some(locator)
}

fn native_update_invalid_reason(
	incoming: &AnnotationInput,
	stored: &StoredAnnotation,
) -> Option<&'static str> {
	let Some((kind, _)) = parse_stump_native_annotation_id(&incoming.id) else {
		return Some("native annotation identity is invalid");
	};
	if incoming.work_id != stored.work_id {
		return Some("native annotation work cannot change");
	}
	match kind {
		"annotation" if matches!(incoming.kind.as_str(), "note" | "highlight") => {
			native_update_locator(&incoming.kind, incoming, stored)
				.is_none()
				.then_some("native annotations require a valid Readium locator")
		},
		"bookmark" if incoming.kind == "bookmark" => {
			native_update_locator("bookmark", incoming, stored)
				.is_none()
				.then_some("native bookmarks require a valid Readium locator")
		},
		_ => Some("native annotation kind cannot change"),
	}
}

async fn writeback_native_annotation<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	incoming: &mut AnnotationInput,
	stored: &StoredAnnotation,
	updated_at: &str,
) -> Result<(), LiseurSyncError> {
	let (kind, native_id) = parse_stump_native_annotation_id(&incoming.id)
		.ok_or_else(|| internal("native annotation identity is invalid"))?;
	let locator = native_update_locator(&incoming.kind, incoming, stored)
		.ok_or_else(|| internal("native annotation locator is invalid"))?;
	incoming.edition_sha = stored.edition_sha.clone();
	incoming.progression = native_progression(&locator);

	match kind {
		"annotation" => {
			let color = annotation_color_for_update(incoming, stored);
			incoming.excerpt = if incoming.kind == "highlight" {
				locator
					.text
					.as_ref()
					.and_then(|text| text.highlight.clone())
					.unwrap_or_default()
			} else {
				String::new()
			};
			incoming.color = Some(color.clone());
			let result = conn
				.execute(db_statement(
					conn,
					"UPDATE media_annotations
                     SET locator = $1, annotation_text = $2, color = $3, updated_at = $4
                     WHERE id = $5 AND user_id = $6",
					vec![
						serde_json::to_string(&locator).map_err(internal)?.into(),
						(!incoming.body.is_empty())
							.then(|| incoming.body.clone())
							.into(),
						(!color.is_empty()).then_some(color).into(),
						updated_at.to_owned().into(),
						native_id.to_owned().into(),
						user_id.to_owned().into(),
					],
				))
				.await
				.map_err(internal)?;
			if result.rows_affected() != 1 {
				return Err(internal("native annotation disappeared during update"));
			}
		},
		"bookmark" => {
			let preview = if !incoming.excerpt.is_empty() {
				Some(incoming.excerpt.clone())
			} else {
				locator
					.text
					.as_ref()
					.and_then(|text| text.highlight.clone())
			};
			let page = locator
				.locations
				.as_ref()
				.and_then(|locations| locations.position);
			let result = conn
				.execute(db_statement(
					conn,
					"UPDATE bookmarks SET preview_content = $1, locator = $2, page = $3
                     WHERE id = $4 AND user_id = $5",
					vec![
						preview.clone().into(),
						Some(serde_json::to_string(&locator).map_err(internal)?).into(),
						page.into(),
						native_id.to_owned().into(),
						user_id.to_owned().into(),
					],
				))
				.await
				.map_err(internal)?;
			if result.rows_affected() != 1 {
				return Err(internal("native bookmark disappeared during update"));
			}
			incoming.excerpt = preview.unwrap_or_default();
			incoming.color = Some(String::new());
			incoming.body.clear();
		},
		_ => return Err(internal("unsupported native annotation identity")),
	}

	incoming.locator = Some(serde_json::to_value(&locator).map_err(internal)?);
	Ok(())
}
async fn delete_native_annotation_source<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	id: &str,
) -> Result<(), LiseurSyncError> {
	let Some((kind, native_id)) = parse_stump_native_annotation_id(id) else {
		return Err(LiseurSyncError::BadRequest(
			"native annotation identity is invalid".into(),
		));
	};
	let table = match kind {
		"annotation" => "media_annotations",
		"bookmark" => "bookmarks",
		_ => {
			return Err(LiseurSyncError::BadRequest(
				"native annotation identity is invalid".into(),
			))
		},
	};
	let result = conn
		.execute(db_statement(
			conn,
			&format!("DELETE FROM {table} WHERE id = $1 AND user_id = $2"),
			vec![native_id.to_owned().into(), user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	if result.rows_affected() != 1 {
		return Err(LiseurSyncError::NotFound(
			"native annotation not found".into(),
		));
	}
	Ok(())
}
async fn delete_native_annotation_projection<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	projection_id: &str,
) -> Result<(), LiseurSyncError> {
	for table in ["media_annotations", "bookmarks"] {
		conn.execute(db_statement(
			conn,
			&format!("DELETE FROM {table} WHERE id = $1 AND user_id = $2"),
			vec![projection_id.to_owned().into(), user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	}
	Ok(())
}

async fn linked_annotation_media_id<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation: &StoredAnnotation,
) -> Result<Option<String>, LiseurSyncError> {
	let row = conn
		.query_one(db_statement(
			conn,
			"SELECT media_id FROM liseur_sync_media_links
             WHERE user_id = $1 AND work_id = $2
             ORDER BY CASE WHEN $3 <> '' AND edition_sha = $3 THEN 0 ELSE 1 END,
                      created_at ASC, id ASC
             LIMIT 1",
			vec![
				user_id.to_owned().into(),
				annotation.work_id.clone().into(),
				annotation.edition_sha.clone().unwrap_or_default().into(),
			],
		))
		.await
		.map_err(internal)?;
	row.map(|row| row.try_get("", "media_id").map_err(internal))
		.transpose()
}

fn readium_projection_locator(annotation: &StoredAnnotation) -> Option<ReadiumLocator> {
	let locator: ReadiumLocator =
		serde_json::from_value(annotation.locator.clone()?).ok()?;
	(!locator.href.trim().is_empty() && locator.locations.is_some()).then_some(locator)
}

async fn project_annotation<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	annotation: &StoredAnnotation,
) -> Result<bool, LiseurSyncError> {
	if is_stump_native_annotation_id(&annotation.id) {
		return Ok(true);
	}
	let projection_id = liseur_sync_projection_id(user_id, &annotation.id);
	if annotation.deleted
		|| !matches!(annotation.kind.as_str(), "highlight" | "note" | "bookmark")
	{
		delete_native_annotation_projection(conn, user_id, &projection_id).await?;
		return Ok(true);
	}
	let Some(mut locator) = readium_projection_locator(annotation) else {
		delete_native_annotation_projection(conn, user_id, &projection_id).await?;
		return Ok(true);
	};
	let Some(media_id) = linked_annotation_media_id(conn, user_id, annotation).await?
	else {
		delete_native_annotation_projection(conn, user_id, &projection_id).await?;
		return Ok(false);
	};
	delete_native_annotation_projection(conn, user_id, &projection_id).await?;
	let created_at = DateTime::parse_from_rfc3339(&annotation.client_ts)
		.map_err(internal)?
		.with_timezone(&Utc);
	let updated_at = DateTime::parse_from_rfc3339(&annotation.updated_at)
		.map_err(internal)?
		.with_timezone(&Utc);
	if matches!(annotation.kind.as_str(), "highlight" | "note") {
		if annotation.kind == "note" {
			if let Some(text) = &mut locator.text {
				text.highlight = None;
			}
		} else if locator
			.text
			.as_ref()
			.and_then(|text| text.highlight.as_deref())
			.map_or(true, |highlight| highlight.trim().is_empty())
			&& !annotation.excerpt.is_empty()
		{
			let text = locator.text.get_or_insert(ReadiumText {
				after: None,
				before: None,
				highlight: None,
			});
			text.highlight = Some(annotation.excerpt.clone());
		}
		let color = (annotation.kind == "highlight"
			&& !annotation.color.is_empty()
			&& stump_liseur_sync::ANNOTATION_COLORS.contains(&annotation.color.as_str()))
		.then(|| annotation.color.clone());
		media_annotation::ActiveModel {
			id: Set(projection_id.clone()),
			locator: Set(locator),
			annotation_text: Set(
				(!annotation.body.is_empty()).then(|| annotation.body.clone())
			),
			color: Set(color),
			media_id: Set(media_id),
			user_id: Set(user_id.to_owned()),
			created_at: Set(created_at.clone()),
			updated_at: Set(updated_at.clone()),
		}
		.insert(conn)
		.await
		.map_err(internal)?;
		conn.execute(db_statement(
			conn,
			"UPDATE media_annotations SET created_at = $1, updated_at = $2
             WHERE id = $3 AND user_id = $4",
			vec![
				created_at.into(),
				updated_at.into(),
				projection_id.into(),
				user_id.to_owned().into(),
			],
		))
		.await
		.map_err(internal)?;
	} else {
		let preview_content = if !annotation.excerpt.is_empty() {
			Some(annotation.excerpt.clone())
		} else {
			locator
				.text
				.as_ref()
				.and_then(|text| text.highlight.clone())
		};
		let page = locator
			.locations
			.as_ref()
			.and_then(|locations| locations.position);
		bookmark::ActiveModel {
			id: Set(projection_id.clone()),
			preview_content: Set(preview_content),
			locator: Set(Some(locator)),
			page: Set(page),
			position_ms: Set(None),
			media_id: Set(media_id),
			user_id: Set(user_id.to_owned()),
			created_at: Set(created_at.clone()),
		}
		.insert(conn)
		.await
		.map_err(internal)?;
		conn.execute(db_statement(
			conn,
			"UPDATE bookmarks SET created_at = $1 WHERE id = $2 AND user_id = $3",
			vec![
				created_at.into(),
				projection_id.into(),
				user_id.to_owned().into(),
			],
		))
		.await
		.map_err(internal)?;
	}
	Ok(true)
}

async fn reconcile_annotation_projections<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<(), LiseurSyncError> {
	let Some(counter) = conn
		.query_one(db_statement(
			conn,
			"SELECT annotation_seq, projected_annotation_seq
             FROM liseur_sync_counters WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?
	else {
		return Ok(());
	};
	let high_water: i64 = counter.try_get("", "annotation_seq").map_err(internal)?;
	let projected: i64 = counter
		.try_get("", "projected_annotation_seq")
		.map_err(internal)?;
	if high_water <= projected {
		return Ok(());
	}
	let rows = conn
		.query_all(db_statement(
			conn,
			&format!(
				"SELECT {ANNOTATION_COLUMNS} FROM liseur_sync_annotations
                 WHERE user_id = $1 AND seq > $2 AND seq <= $3 ORDER BY seq ASC"
			),
			vec![
				user_id.to_owned().into(),
				projected.into(),
				high_water.into(),
			],
		))
		.await
		.map_err(internal)?;
	let mut completed = high_water;
	for row in &rows {
		let annotation = stored_annotation(row)?;
		if !project_annotation(conn, user_id, &annotation).await? {
			completed = completed.min(annotation.seq.saturating_sub(1));
		}
	}
	if completed > projected {
		conn.execute(db_statement(
			conn,
			"UPDATE liseur_sync_counters
             SET projected_annotation_seq = $1
             WHERE user_id = $2 AND projected_annotation_seq < $1",
			vec![completed.into(), user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	}
	Ok(())
}

fn media_annotation_candidate(
	row: &media_annotation::Model,
	link: &NativeMediaLink,
) -> Result<Option<NativeAnnotationCandidate>, LiseurSyncError> {
	if is_liseur_sync_projection_id(&row.id)
		|| row.locator.href.trim().is_empty()
		|| row.locator.locations.is_none()
	{
		return Ok(None);
	}
	let excerpt = row
		.locator
		.text
		.as_ref()
		.and_then(|text| text.highlight.clone())
		.unwrap_or_default();
	let kind = if excerpt.trim().is_empty() {
		"note"
	} else {
		"highlight"
	};
	let body = row.annotation_text.clone().unwrap_or_default();
	if kind == "note" && body.is_empty() {
		return Ok(None);
	}
	let id = stump_native_annotation_id("annotation", &row.id);
	if id.len() > 64 {
		return Ok(None);
	}
	let color = if kind == "highlight" {
		row.color
			.clone()
			.filter(|color| ANNOTATION_COLORS.contains(&color.as_str()))
			.unwrap_or_default()
	} else {
		String::new()
	};
	Ok(Some(NativeAnnotationCandidate {
		id,
		work_id: link.work_id.clone(),
		edition_sha: link.edition_sha.clone(),
		kind: kind.to_owned(),
		locator: serde_json::to_value(&row.locator).map_err(internal)?,
		progression: native_progression(&row.locator),
		excerpt,
		color,
		body,
		client_ts: native_timestamp(row.created_at),
		updated_at: native_timestamp(row.updated_at),
	}))
}

fn bookmark_candidate(
	row: &bookmark::Model,
	link: &NativeMediaLink,
) -> Result<Option<NativeAnnotationCandidate>, LiseurSyncError> {
	if is_liseur_sync_projection_id(&row.id) {
		return Ok(None);
	}
	let Some(locator) = row
		.locator
		.as_ref()
		.filter(|locator| !locator.href.trim().is_empty() && locator.locations.is_some())
	else {
		return Ok(None);
	};
	let id = stump_native_annotation_id("bookmark", &row.id);
	if id.len() > 64 {
		return Ok(None);
	}
	let excerpt = row
		.preview_content
		.clone()
		.filter(|preview| !preview.is_empty())
		.or_else(|| {
			locator
				.text
				.as_ref()
				.and_then(|text| text.highlight.clone())
		})
		.unwrap_or_default();
	Ok(Some(NativeAnnotationCandidate {
		id,
		work_id: link.work_id.clone(),
		edition_sha: link.edition_sha.clone(),
		kind: "bookmark".to_owned(),
		locator: serde_json::to_value(locator).map_err(internal)?,
		progression: native_progression(locator),
		excerpt,
		color: String::new(),
		body: String::new(),
		client_ts: native_timestamp(row.created_at),
		updated_at: native_timestamp(row.created_at),
	}))
}

async fn native_annotation_candidates<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<Vec<NativeAnnotationCandidate>, LiseurSyncError> {
	let rows = conn
		.query_all(db_statement(
			conn,
			"SELECT media_id, work_id, edition_sha FROM liseur_sync_media_links
             WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	let mut links: HashMap<String, NativeMediaLink> = HashMap::with_capacity(rows.len());
	for row in rows {
		let media_id: String = row.try_get("", "media_id").map_err(internal)?;
		let work_id: String = row.try_get("", "work_id").map_err(internal)?;
		let edition_sha: Option<String> =
			row.try_get("", "edition_sha").map_err(internal)?;
		links.insert(
			media_id,
			NativeMediaLink {
				work_id,
				edition_sha,
			},
		);
	}
	if links.is_empty() {
		return Ok(Vec::new());
	}

	let media_ids = links.keys().cloned().collect::<Vec<_>>();
	let auth_user = LoginUser::find_by_id(user_id.to_owned())
		.into_model::<LoginUser>()
		.one(conn)
		.await
		.map_err(internal)?
		.map(AuthUser::from);
	let Some(auth_user) = auth_user else {
		return Ok(Vec::new());
	};
	let visible_media = media::Entity::find_for_user(&auth_user)
		.filter(media::Column::Id.is_in(media_ids))
		.filter(media::Column::DeletedAt.is_null())
		.filter(media::Column::SeriesId.is_not_null())
		.filter(series::Column::LibraryId.is_not_null())
		.filter(media::audio_extension_condition().not())
		.all(conn)
		.await
		.map_err(internal)?;
	links.retain(|media_id, _| visible_media.iter().any(|media| &media.id == media_id));
	if links.is_empty() {
		return Ok(Vec::new());
	}

	let visible_ids = links.keys().cloned().collect::<Vec<_>>();
	let annotations = media_annotation::Entity::find()
		.filter(media_annotation::Column::UserId.eq(user_id))
		.filter(media_annotation::Column::MediaId.is_in(visible_ids.clone()))
		.all(conn)
		.await
		.map_err(internal)?;
	let bookmarks = bookmark::Entity::find()
		.filter(bookmark::Column::UserId.eq(user_id))
		.filter(bookmark::Column::MediaId.is_in(visible_ids))
		.all(conn)
		.await
		.map_err(internal)?;

	let mut candidates = Vec::with_capacity(annotations.len() + bookmarks.len());
	for row in annotations {
		if let Some(link) = links.get(&row.media_id) {
			if let Some(candidate) = media_annotation_candidate(&row, link)? {
				candidates.push(candidate);
			}
		}
	}
	for row in bookmarks {
		if let Some(link) = links.get(&row.media_id) {
			if let Some(candidate) = bookmark_candidate(&row, link)? {
				candidates.push(candidate);
			}
		}
	}
	Ok(candidates)
}

async fn reconcile_native_candidate<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	candidate: &NativeAnnotationCandidate,
	stored: Option<StoredAnnotation>,
) -> Result<(), LiseurSyncError> {
	let Some(stored) = stored else {
		let seq = next_annotation_seq(conn, user_id).await?;
		let payload =
			annotation_payload(&candidate.input(0, candidate.client_ts.clone()))?;
		return conn
			.execute(db_statement(
				conn,
				"INSERT INTO liseur_sync_annotations
                    (user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
                     progression, excerpt, color, drawer, body, device_id, origin_device_id,
                     client_ts, updated_at, deleted, deleted_at, payload)
                 VALUES ($1, $2, 1, $3, $4, $5, $6, $7, $8, $9, $10, NULL, $11,
                         'stump-native', 'stump-native', $12, $13, FALSE, NULL, $14)",
				vec![
					user_id.to_owned().into(),
					candidate.id.clone().into(),
					seq.into(),
					candidate.work_id.clone().into(),
					candidate.edition_sha.clone().into(),
					candidate.kind.clone().into(),
					Some(serde_json::to_string(&candidate.locator).map_err(internal)?).into(),
					candidate.progression.into(),
					candidate.excerpt.clone().into(),
					candidate.color.clone().into(),
					candidate.body.clone().into(),
					candidate.client_ts.clone().into(),
					candidate.updated_at.clone().into(),
					payload.into(),
				],
			))
			.await
			.map(|_| ())
			.map_err(internal);
	};
	if native_candidate_matches(&stored, candidate) {
		return Ok(());
	}

	let seq = next_annotation_seq(conn, user_id).await?;
	let client_ts = stored.client_ts.clone();
	let payload = annotation_payload(&candidate.input(stored.rev, client_ts))?;
	conn.execute(db_statement(
		conn,
		"UPDATE liseur_sync_annotations
         SET rev = $1, seq = $2, work_id = $3, edition_sha = $4, kind = $5,
             locator = $6, progression = $7, excerpt = $8, color = $9, body = $10,
             device_id = 'stump-native', updated_at = $11, deleted = FALSE,
             deleted_at = NULL, payload = $12
         WHERE user_id = $13 AND annotation_id = $14",
		vec![
			(stored.rev + 1).into(),
			seq.into(),
			candidate.work_id.clone().into(),
			candidate.edition_sha.clone().into(),
			candidate.kind.clone().into(),
			Some(serde_json::to_string(&candidate.locator).map_err(internal)?).into(),
			candidate.progression.into(),
			candidate.excerpt.clone().into(),
			candidate.color.clone().into(),
			candidate.body.clone().into(),
			candidate.updated_at.clone().into(),
			payload.into(),
			user_id.to_owned().into(),
			candidate.id.clone().into(),
		],
	))
	.await
	.map_err(internal)?;
	Ok(())
}

async fn reconcile_native_annotations<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<(), LiseurSyncError> {
	let candidates = native_annotation_candidates(conn, user_id).await?;
	let rows = conn
		.query_all(db_statement(
			conn,
			&format!(
				"SELECT {ANNOTATION_COLUMNS} FROM liseur_sync_annotations
                 WHERE user_id = $1 AND annotation_id LIKE $2"
			),
			vec![
				user_id.to_owned().into(),
				format!("{STUMP_NATIVE_ANNOTATION_ID_PREFIX}%").into(),
			],
		))
		.await
		.map_err(internal)?;
	let mut existing = HashMap::with_capacity(rows.len());
	for row in &rows {
		let annotation = stored_annotation(row)?;
		if parse_stump_native_annotation_id(&annotation.id).is_some() {
			existing.insert(annotation.id.clone(), annotation);
		}
	}

	for candidate in candidates {
		let stored = existing.remove(&candidate.id);
		reconcile_native_candidate(conn, user_id, &candidate, stored).await?;
	}
	for annotation in existing
		.into_values()
		.filter(|annotation| !annotation.deleted)
	{
		let seq = next_annotation_seq(conn, user_id).await?;
		let updated_at = now_string();
		conn.execute(db_statement(
			conn,
			"UPDATE liseur_sync_annotations
             SET rev = $1, seq = $2, updated_at = $3, deleted = TRUE, deleted_at = $3
             WHERE user_id = $4 AND annotation_id = $5",
			vec![
				(annotation.rev + 1).into(),
				seq.into(),
				updated_at.into(),
				user_id.to_owned().into(),
				annotation.id.into(),
			],
		))
		.await
		.map_err(internal)?;
	}
	Ok(())
}

async fn find_annotation<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
	id: &str,
) -> Result<Option<StoredAnnotation>, LiseurSyncError> {
	conn.query_one(db_statement(
		conn,
		&format!(
			"SELECT {ANNOTATION_COLUMNS} FROM liseur_sync_annotations
             WHERE user_id = $1 AND annotation_id = $2"
		),
		vec![user_id.to_owned().into(), id.to_owned().into()],
	))
	.await
	.map_err(internal)?
	.as_ref()
	.map(stored_annotation)
	.transpose()
}

fn annotation_payload(annotation: &AnnotationInput) -> Result<String, LiseurSyncError> {
	serde_json::to_string(annotation).map_err(internal)
}

fn annotations_same_payload(
	stored: &StoredAnnotation,
	incoming_payload: &str,
	device_id: &str,
) -> bool {
	stored.payload == incoming_payload && stored.device_id == device_id
}

pub(crate) async fn append_annotations(
	ctx: &AppState,
	user_id: &str,
	device_id: &str,
	annotations: Vec<AnnotationInput>,
) -> Result<Vec<AnnotationResult>, LiseurSyncError> {
	let conn = ctx_conn(ctx);
	let txn = begin_write(conn).await.map_err(internal)?;
	ensure_counter(&txn, user_id).await?;
	reconcile_native_annotations(&txn, user_id).await?;
	let mut results = Vec::with_capacity(annotations.len());

	for mut annotation in annotations {
		if !work_exists(&txn, user_id, &annotation.work_id).await? {
			results.push(AnnotationResult {
				id: annotation.id,
				status: "invalid".into(),
				rev: None,
				seq: None,
				reason: Some("unknown work".into()),
				server: None,
			});
			continue;
		}
		let payload = annotation_payload(&annotation)?;
		let existing = find_annotation(&txn, user_id, &annotation.id).await?;
		if let Some(stored) = existing {
			if stored.deleted {
				results.push(AnnotationResult {
					id: annotation.id,
					status: "conflict".into(),
					rev: None,
					seq: None,
					reason: Some("annotation is deleted".into()),
					server: Some(annotation_record(&stored)),
				});
				continue;
			}
			if annotation.base_rev != stored.rev {
				if annotation.base_rev + 1 == stored.rev
					&& annotations_same_payload(&stored, &payload, device_id)
				{
					results.push(AnnotationResult {
						id: annotation.id,
						status: "duplicate".into(),
						rev: Some(stored.rev),
						seq: Some(stored.seq),
						reason: None,
						server: None,
					});
				} else {
					results.push(AnnotationResult {
						id: annotation.id,
						status: "conflict".into(),
						rev: None,
						seq: None,
						reason: Some("annotation revision conflict".into()),
						server: Some(annotation_record(&stored)),
					});
				}
				continue;
			}
			if is_stump_native_annotation_id(&annotation.id) {
				if let Some(reason) = native_update_invalid_reason(&annotation, &stored) {
					results.push(AnnotationResult {
						id: annotation.id,
						status: "invalid".into(),
						rev: None,
						seq: None,
						reason: Some(reason.into()),
						server: Some(annotation_record(&stored)),
					});
					continue;
				}
			}
			let updated_at = now_string();
			if is_stump_native_annotation_id(&annotation.id) {
				writeback_native_annotation(
					&txn,
					user_id,
					&mut annotation,
					&stored,
					&updated_at,
				)
				.await?;
			}
			let payload = annotation_payload(&annotation)?;
			let color = annotation_color_for_update(&annotation, &stored);
			let drawer = annotation_drawer_for_update(&annotation, &stored);
			let seq = next_annotation_seq(&txn, user_id).await?;
			txn.execute(db_statement(
				&txn,
				"UPDATE liseur_sync_annotations
                 SET rev = $1, seq = $2, edition_sha = $3, kind = $4, locator = $5,
                     progression = $6, excerpt = $7, color = $8, drawer = $9,
                     body = $10, device_id = $11, client_ts = $12, updated_at = $13,
                     deleted = FALSE, deleted_at = NULL, payload = $14
                 WHERE user_id = $15 AND annotation_id = $16",
				vec![
					(stored.rev + 1).into(),
					seq.into(),
					annotation.edition_sha.into(),
					annotation.kind.into(),
					locator_to_db(&annotation.locator)?.into(),
					annotation.progression.into(),
					annotation.excerpt.into(),
					color.into(),
					drawer.into(),
					annotation.body.into(),
					device_id.to_owned().into(),
					annotation.client_ts.into(),
					updated_at.into(),
					payload.into(),
					user_id.to_owned().into(),
					annotation.id.clone().into(),
				],
			))
			.await
			.map_err(internal)?;
			results.push(AnnotationResult {
				id: annotation.id,
				status: "applied".into(),
				rev: Some(stored.rev + 1),
				seq: Some(seq),
				reason: None,
				server: None,
			});
			continue;
		}

		if annotation.base_rev != 0 {
			results.push(AnnotationResult {
				id: annotation.id,
				status: "invalid".into(),
				rev: None,
				seq: None,
				reason: Some("annotation does not exist".into()),
				server: None,
			});
			continue;
		}
		if is_stump_native_annotation_id(&annotation.id) {
			results.push(AnnotationResult {
				id: annotation.id,
				status: "invalid".into(),
				rev: None,
				seq: None,
				reason: Some("native annotation ids are reserved".into()),
				server: None,
			});
			continue;
		}
		let seq = next_annotation_seq(&txn, user_id).await?;
		let color = annotation.color.unwrap_or_default();
		let drawer = annotation.drawer.filter(|drawer| !drawer.is_empty());
		let updated_at = now_string();
		txn.execute(db_statement(
			&txn,
			"INSERT INTO liseur_sync_annotations
                (user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
                 progression, excerpt, color, drawer, body, device_id, origin_device_id,
                 client_ts, updated_at, deleted, deleted_at, payload)
             VALUES ($1, $2, 1, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13,
                     $14, $15, FALSE, NULL, $16)",
			vec![
				user_id.to_owned().into(),
				annotation.id.clone().into(),
				seq.into(),
				annotation.work_id.into(),
				annotation.edition_sha.into(),
				annotation.kind.into(),
				locator_to_db(&annotation.locator)?.into(),
				annotation.progression.into(),
				annotation.excerpt.into(),
				color.into(),
				drawer.into(),
				annotation.body.into(),
				device_id.to_owned().into(),
				annotation.client_ts.into(),
				updated_at.into(),
				payload.into(),
			],
		))
		.await
		.map_err(internal)?;
		results.push(AnnotationResult {
			id: annotation.id,
			status: "applied".into(),
			rev: Some(1),
			seq: Some(seq),
			reason: None,
			server: None,
		});
	}
	if results.iter().any(|result| result.status == "applied") {
		reconcile_annotation_projections(&txn, user_id).await?;
	}
	txn.commit().await.map_err(internal)?;
	if results.iter().any(|result| result.status == "applied") {
		ctx.note_annotation_activity(user_id);
	}
	Ok(results)
}

async fn annotation_high_water<C: ConnectionTrait>(
	conn: &C,
	user_id: &str,
) -> Result<i64, LiseurSyncError> {
	let row = conn
		.query_one(db_statement(
			conn,
			"SELECT annotation_seq FROM liseur_sync_counters WHERE user_id = $1",
			vec![user_id.to_owned().into()],
		))
		.await
		.map_err(internal)?;
	Ok(row
		.map(|row| row.try_get("", "annotation_seq"))
		.transpose()
		.map_err(internal)?
		.unwrap_or(0))
}

pub(crate) async fn annotation_changes(
	ctx: &AppState,
	user_id: &str,
	since: i64,
	limit: usize,
) -> Result<(Vec<AnnotationRecord>, i64, bool), LiseurSyncError> {
	let conn = ctx_conn(ctx);
	let txn = begin_write(conn).await.map_err(internal)?;
	ensure_counter(&txn, user_id).await?;
	reconcile_native_annotations(&txn, user_id).await?;
	reconcile_annotation_projections(&txn, user_id).await?;
	txn.commit().await.map_err(internal)?;
	let high_water = annotation_high_water(conn, user_id).await?;
	let rows = conn
		.query_all(db_statement(
			conn,
			&format!(
				"SELECT {ANNOTATION_COLUMNS} FROM liseur_sync_annotations
                 WHERE user_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3"
			),
			vec![
				user_id.to_owned().into(),
				since.into(),
				((limit + 1) as i64).into(),
			],
		))
		.await
		.map_err(internal)?;
	let has_more = rows.len() > limit;
	let annotations = rows
		.iter()
		.take(limit)
		.map(stored_annotation)
		.collect::<Result<Vec<_>, _>>()?
		.iter()
		.map(annotation_record)
		.collect();
	Ok((annotations, high_water, has_more))
}

pub(crate) async fn work_annotations(
	ctx: &AppState,
	user_id: &str,
	work_id: &str,
) -> Result<Vec<AnnotationRecord>, LiseurSyncError> {
	work_annotations_with_deleted(ctx, user_id, work_id, false).await
}

pub(crate) async fn work_annotations_with_deleted(
	ctx: &AppState,
	user_id: &str,
	work_id: &str,
	include_deleted: bool,
) -> Result<Vec<AnnotationRecord>, LiseurSyncError> {
	let conn = ctx_conn(ctx);
	if !work_exists(conn, user_id, work_id).await? {
		return Err(LiseurSyncError::NotFound("work not found".into()));
	}
	let txn = begin_write(conn).await.map_err(internal)?;
	ensure_counter(&txn, user_id).await?;
	reconcile_native_annotations(&txn, user_id).await?;
	reconcile_annotation_projections(&txn, user_id).await?;
	txn.commit().await.map_err(internal)?;
	let rows = conn
		.query_all(db_statement(
			conn,
			&format!(
				"SELECT {ANNOTATION_COLUMNS} FROM liseur_sync_annotations
                 WHERE user_id = $1 AND work_id = $2
                   AND ($3 = TRUE OR deleted = FALSE)
                 ORDER BY deleted ASC, progression IS NULL ASC,
                          progression ASC, client_ts ASC, seq ASC"
			),
			vec![
				user_id.to_owned().into(),
				work_id.to_owned().into(),
				include_deleted.into(),
			],
		))
		.await
		.map_err(internal)?;
	rows.iter()
		.map(stored_annotation)
		.collect::<Result<Vec<_>, _>>()
		.map(|annotations| annotations.iter().map(annotation_record).collect())
}

pub(crate) async fn delete_annotation(
	ctx: &AppState,
	user_id: &str,
	id: &str,
	rev: i64,
) -> Result<DeleteAnnotationResult, LiseurSyncError> {
	let conn = ctx_conn(ctx);
	let txn = begin_write(conn).await.map_err(internal)?;
	ensure_counter(&txn, user_id).await?;
	reconcile_native_annotations(&txn, user_id).await?;
	let Some(stored) = find_annotation(&txn, user_id, id).await? else {
		return Err(LiseurSyncError::NotFound("annotation not found".into()));
	};
	if stored.deleted {
		txn.commit().await.map_err(internal)?;
		return Ok(DeleteAnnotationResult {
			id: id.into(),
			status: "duplicate".into(),
			rev: stored.rev,
			seq: stored.seq,
			server: None,
		});
	}
	if stored.rev != rev {
		return Ok(DeleteAnnotationResult {
			id: id.into(),
			status: "conflict".into(),
			rev: stored.rev,
			seq: stored.seq,
			server: Some(annotation_record(&stored)),
		});
	}
	if is_stump_native_annotation_id(id) {
		delete_native_annotation_source(&txn, user_id, id).await?;
	}
	let seq = next_annotation_seq(&txn, user_id).await?;
	let updated_at = now_string();
	txn.execute(db_statement(
		&txn,
		"UPDATE liseur_sync_annotations
         SET rev = $1, seq = $2, updated_at = $3, deleted = TRUE, deleted_at = $4
         WHERE user_id = $5 AND annotation_id = $6",
		vec![
			(stored.rev + 1).into(),
			seq.into(),
			updated_at.clone().into(),
			updated_at.clone().into(),
			user_id.to_owned().into(),
			id.to_owned().into(),
		],
	))
	.await
	.map_err(internal)?;
	// A tombstone keeps identity and revision, not content: the annotation's
	// side objects go with its body.
	let detached = super::attachments::detach_all(&txn, user_id, id).await?;
	reconcile_annotation_projections(&txn, user_id).await?;
	txn.commit().await.map_err(internal)?;
	super::attachments::unlink_all(ctx, &detached).await;
	ctx.note_annotation_activity(user_id);
	Ok(DeleteAnnotationResult {
		id: id.into(),
		status: "applied".into(),
		rev: stored.rev + 1,
		seq,
		server: None,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn token_hash_is_stable_and_not_plaintext() {
		let hash = hash_secret("secret");
		assert_eq!(hash.len(), 64);
		assert_ne!(hash, "secret");
	}

	#[test]
	fn timestamp_format_is_rfc3339() {
		let value = now_string();
		assert!(DateTime::parse_from_rfc3339(&value).is_ok());
	}
	#[tokio::test]
	async fn work_annotations_reads_annotation_id_column() {
		use std::sync::Arc;

		use sea_orm::{Database, DatabaseBackend};

		let conn = Database::connect("sqlite::memory:").await.unwrap();
		for sql in [
			"CREATE TABLE liseur_sync_works (
                id TEXT NOT NULL,
                user_id TEXT NOT NULL
            )",
			"CREATE TABLE liseur_sync_counters (
                user_id TEXT PRIMARY KEY,
                op_seq BIGINT NOT NULL DEFAULT 0,
                annotation_seq BIGINT NOT NULL DEFAULT 0,
                projected_annotation_seq BIGINT NOT NULL DEFAULT 0
            )",
			"CREATE TABLE liseur_sync_media_links (
                user_id TEXT NOT NULL, media_id TEXT NOT NULL,
                work_id TEXT NOT NULL, edition_sha TEXT
            )",
			"CREATE TABLE liseur_sync_annotations (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT NOT NULL,
                annotation_id TEXT NOT NULL,
                rev BIGINT NOT NULL,
                seq BIGINT NOT NULL,
                work_id TEXT NOT NULL,
                edition_sha TEXT,
                kind TEXT NOT NULL,
                locator TEXT,
                progression DOUBLE,
                excerpt TEXT NOT NULL,
                color TEXT NOT NULL,
                drawer TEXT,
                body TEXT NOT NULL,
                device_id TEXT NOT NULL,
                client_ts TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted BOOLEAN NOT NULL DEFAULT FALSE,
                deleted_at TEXT,
                payload TEXT NOT NULL
            )",
			"INSERT INTO liseur_sync_works (id, user_id) VALUES ('work-1', 'user-1')",
			"INSERT INTO liseur_sync_annotations
                (user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
                 progression, excerpt, color, drawer, body, device_id, client_ts, updated_at,
                 deleted, deleted_at, payload)
             VALUES
                ('user-1', 'annotation-1', 1, 1, 'work-1', NULL, 'highlight', NULL,
                 0.5, 'excerpt', 'yellow', NULL, 'body', 'device-1',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', FALSE, NULL, '{}')",
		] {
			conn.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
				.await
				.unwrap();
		}
		let ctx = Arc::new(stump_core::Ctx::for_testing(conn));
		let annotations = work_annotations(&ctx, "user-1", "work-1").await.unwrap();
		assert_eq!(annotations.len(), 1);
		assert_eq!(annotations[0].id, "annotation-1");
	}

	#[tokio::test]
	async fn annotations_preserve_extended_styles_and_reconcile_native_projections() {
		use std::sync::Arc;

		use ::tests::{db::test_database, fake_data};
		use sea_orm::{DatabaseBackend, Schema};

		let db = test_database().await;
		let user = fake_data::User::new("liseur-annotation-user")
			.insert(&db)
			.await;
		let library = fake_data::Library::default().insert(&db).await;
		let series = fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&db)
		.await;
		let media = fake_data::Media {
			series_id: series.id,
			id: Some("liseur-annotation-media".to_owned()),
			..Default::default()
		}
		.insert(&db)
		.await;
		let schema = Schema::new(DatabaseBackend::Sqlite);
		for statement in [
			schema.create_table_from_entity(media_annotation::Entity),
			schema.create_table_from_entity(bookmark::Entity),
		] {
			db.execute(db.get_database_backend().build(&statement))
				.await
				.unwrap();
		}
		for sql in [
			"CREATE TABLE liseur_sync_works (id TEXT NOT NULL, user_id TEXT NOT NULL)",
			"CREATE TABLE liseur_sync_media_links (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, work_id TEXT NOT NULL,
                media_id TEXT NOT NULL, edition_sha TEXT NOT NULL, created_at TEXT NOT NULL
            )",
			"CREATE TABLE liseur_sync_counters (
                user_id TEXT PRIMARY KEY, op_seq BIGINT NOT NULL DEFAULT 0,
                annotation_seq BIGINT NOT NULL DEFAULT 0,
                projected_annotation_seq BIGINT NOT NULL DEFAULT 0
            )",
			"CREATE TABLE annotation_attachments (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, annotation_id TEXT NOT NULL,
                kind TEXT NOT NULL, media_type TEXT NOT NULL, byte_size BIGINT NOT NULL,
                sha256 TEXT NOT NULL, storage_path TEXT NOT NULL, created_at TEXT NOT NULL
            )",
			"CREATE TABLE liseur_sync_annotations (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT NOT NULL,
                annotation_id TEXT NOT NULL, rev BIGINT NOT NULL, seq BIGINT NOT NULL,
                work_id TEXT NOT NULL, edition_sha TEXT, kind TEXT NOT NULL,
                locator TEXT, progression DOUBLE, excerpt TEXT NOT NULL, color TEXT NOT NULL,
                drawer TEXT, body TEXT NOT NULL, device_id TEXT NOT NULL,
                origin_device_id TEXT, client_ts TEXT NOT NULL,
                updated_at TEXT NOT NULL, deleted BOOLEAN NOT NULL DEFAULT FALSE,
                deleted_at TEXT, payload TEXT NOT NULL,
                UNIQUE (user_id, annotation_id)
            )",
		] {
			db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
				.await
				.unwrap();
		}
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_works (id, user_id) VALUES ($1, $2)",
			vec!["work-1".to_owned().into(), user.id.clone().into()],
		))
		.await
		.unwrap();
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_media_links
                (id, user_id, work_id, media_id, edition_sha, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
			vec![
				"link-1".to_owned().into(),
				user.id.clone().into(),
				"work-1".to_owned().into(),
				media.id.clone().into(),
				"edition-1".to_owned().into(),
				"2026-09-11T12:00:00Z".to_owned().into(),
			],
		))
		.await
		.unwrap();
		let ctx = Arc::new(stump_core::Ctx::for_testing(db));
		let highlight = AnnotationInput {
			id: "highlight-1".to_owned(),
			base_rev: 0,
			work_id: "work-1".to_owned(),
			edition_sha: Some("edition-1".to_owned()),
			kind: "highlight".to_owned(),
			locator: Some(serde_json::json!({
				"chapterTitle": "Chapter one",
				"href": "OPS/chapter.xhtml",
				"locations": {"position": 17},
				"text": {"highlight": "Quoted passage"}
			})),
			progression: Some(0.42),
			excerpt: "Quoted passage".to_owned(),
			color: Some("red".to_owned()),
			drawer: Some("invert".to_owned()),
			body: "A note".to_owned(),
			client_ts: "2026-09-11T12:00:00Z".to_owned(),
		};
		let bookmark_input = AnnotationInput {
			id: "bookmark-1".to_owned(),
			kind: "bookmark".to_owned(),
			color: None,
			drawer: None,
			body: String::new(),
			..highlight.clone()
		};
		let inserted = append_annotations(
			&ctx,
			&user.id,
			"device-1",
			vec![highlight.clone(), bookmark_input],
		)
		.await
		.unwrap();
		assert!(inserted.iter().all(|result| result.status == "applied"));

		let highlight_projection_id = liseur_sync_projection_id(&user.id, &highlight.id);
		let projected = media_annotation::Entity::find_by_id(&highlight_projection_id)
			.one(ctx.conn.as_ref())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(projected.color.as_deref(), Some("red"));
		assert_eq!(
			projected
				.locator
				.text
				.as_ref()
				.and_then(|text| text.highlight.as_deref()),
			Some("Quoted passage")
		);
		assert!(bookmark::Entity::find_by_id(liseur_sync_projection_id(
			&user.id,
			"bookmark-1"
		))
		.one(ctx.conn.as_ref())
		.await
		.unwrap()
		.is_some());
		let live = work_annotations(&ctx, &user.id, "work-1").await.unwrap();
		let stored = live.iter().find(|row| row.id == highlight.id).unwrap();
		assert_eq!(stored.color.as_deref(), Some("red"));
		assert_eq!(stored.drawer.as_deref(), Some("invert"));

		let update_without_extended_fields = AnnotationInput {
			base_rev: 1,
			excerpt: "Updated passage".to_owned(),
			body: "Updated note".to_owned(),
			color: None,
			drawer: None,
			..highlight.clone()
		};
		assert_eq!(
			append_annotations(
				&ctx,
				&user.id,
				"device-1",
				vec![update_without_extended_fields]
			)
			.await
			.unwrap()[0]
				.status,
			"applied"
		);
		let live = work_annotations(&ctx, &user.id, "work-1").await.unwrap();
		let stored = live.iter().find(|row| row.id == highlight.id).unwrap();
		assert_eq!(stored.color.as_deref(), Some("red"));
		assert_eq!(stored.drawer.as_deref(), Some("invert"));

		let clear_extended_fields = AnnotationInput {
			base_rev: 2,
			color: Some(String::new()),
			drawer: Some(String::new()),
			..highlight.clone()
		};
		assert_eq!(
			append_annotations(&ctx, &user.id, "device-1", vec![clear_extended_fields])
				.await
				.unwrap()[0]
				.status,
			"applied"
		);
		let live = work_annotations(&ctx, &user.id, "work-1").await.unwrap();
		let stored = live.iter().find(|row| row.id == highlight.id).unwrap();
		assert_eq!(stored.color, None);
		assert_eq!(stored.drawer, None);

		let change_to_note = AnnotationInput {
			base_rev: 3,
			kind: "note".to_owned(),
			excerpt: String::new(),
			color: Some(String::new()),
			drawer: Some(String::new()),
			body: "Note after highlight".to_owned(),
			..highlight.clone()
		};
		assert_eq!(
			append_annotations(&ctx, &user.id, "device-1", vec![change_to_note])
				.await
				.unwrap()[0]
				.status,
			"applied"
		);
		let projected_note =
			media_annotation::Entity::find_by_id(&highlight_projection_id)
				.one(ctx.conn.as_ref())
				.await
				.unwrap()
				.unwrap();
		assert_eq!(
			projected_note.annotation_text.as_deref(),
			Some("Note after highlight")
		);
		assert_eq!(
			projected_note
				.locator
				.text
				.as_ref()
				.and_then(|text| text.highlight.as_deref()),
			None
		);
		let live = work_annotations(&ctx, &user.id, "work-1").await.unwrap();
		let note = live.iter().find(|row| row.id == highlight.id).unwrap();
		assert_eq!(note.kind.as_deref(), Some("note"));
		assert_eq!(note.color, None);
		assert_eq!(note.drawer, None);

		let home_highlight_id = "home-highlight-1";
		let home_note_id = "home-note-1";
		let home_bookmark_id = "home-bookmark-1";
		let home_locator = serde_json::from_value::<ReadiumLocator>(serde_json::json!({
			"chapterTitle": "Chapter two",
			"href": "OPS/chapter.xhtml",
			"locations": {
				"position": 22,
				"progression": 0.37,
				"totalProgression": 0.57
			},
			"text": {
				"before": "words before",
				"highlight": "Home selected passage",
				"after": "words after"
			}
		}))
		.unwrap();
		media_annotation::ActiveModel {
			id: Set(home_highlight_id.to_owned()),
			locator: Set(home_locator.clone()),
			annotation_text: Set(Some("Home note on highlight".to_owned())),
			color: Set(Some("red".to_owned())),
			media_id: Set(media.id.clone()),
			user_id: Set(user.id.clone()),
			..Default::default()
		}
		.insert(ctx.conn.as_ref())
		.await
		.unwrap();
		let home_note_locator =
			serde_json::from_value::<ReadiumLocator>(serde_json::json!({
				"chapterTitle": "Chapter three",
				"href": "OPS/chapter.xhtml",
				"locations": {"position": 30, "progression": 0.72},
				"text": {"before": "note before", "after": "note after"}
			}))
			.unwrap();
		media_annotation::ActiveModel {
			id: Set(home_note_id.to_owned()),
			locator: Set(home_note_locator),
			annotation_text: Set(Some("Home standalone note".to_owned())),
			color: Set(None),
			media_id: Set(media.id.clone()),
			user_id: Set(user.id.clone()),
			..Default::default()
		}
		.insert(ctx.conn.as_ref())
		.await
		.unwrap();
		bookmark::ActiveModel {
			id: Set(home_bookmark_id.to_owned()),
			preview_content: Set(Some("Home bookmark preview".to_owned())),
			locator: Set(Some(home_locator)),
			page: Set(Some(22)),
			position_ms: Set(None),
			media_id: Set(media.id.clone()),
			user_id: Set(user.id.clone()),
			..Default::default()
		}
		.insert(ctx.conn.as_ref())
		.await
		.unwrap();

		let before_native = annotation_high_water(ctx.conn.as_ref(), &user.id)
			.await
			.unwrap();
		let (native_records, native_high_water, has_more) =
			annotation_changes(&ctx, &user.id, before_native, 500)
				.await
				.unwrap();
		assert!(!has_more);
		assert!(native_high_water > before_native);
		let native_highlight_id =
			stump_native_annotation_id("annotation", home_highlight_id);
		let exported = native_records
			.iter()
			.find(|record| record.id == native_highlight_id)
			.unwrap();
		assert_eq!(exported.rev, 1);
		assert_eq!(exported.work_id.as_deref(), Some("work-1"));
		assert_eq!(exported.edition_sha.as_deref(), Some("edition-1"));
		assert_eq!(exported.kind.as_deref(), Some("highlight"));
		assert_eq!(exported.excerpt.as_deref(), Some("Home selected passage"));
		assert_eq!(exported.body.as_deref(), Some("Home note on highlight"));
		assert_eq!(exported.color.as_deref(), Some("red"));
		assert_eq!(exported.progression, Some(0.57));
		assert_eq!(
			exported
				.locator
				.as_ref()
				.unwrap()
				.pointer("/text/before")
				.and_then(Value::as_str),
			Some("words before")
		);
		assert_eq!(
			exported
				.locator
				.as_ref()
				.unwrap()
				.pointer("/text/after")
				.and_then(Value::as_str),
			Some("words after")
		);
		assert!(
			DateTime::parse_from_rfc3339(exported.client_ts.as_deref().unwrap()).is_ok()
		);
		assert!(DateTime::parse_from_rfc3339(&exported.updated_at).is_ok());
		let native_note_id = stump_native_annotation_id("annotation", home_note_id);
		let exported_note = native_records
			.iter()
			.find(|record| record.id == native_note_id)
			.unwrap();
		assert_eq!(exported_note.kind.as_deref(), Some("note"));
		assert_eq!(exported_note.body.as_deref(), Some("Home standalone note"));
		let native_bookmark_id = stump_native_annotation_id("bookmark", home_bookmark_id);
		assert!(native_records
			.iter()
			.any(|record| record.id == native_bookmark_id
				&& record.kind.as_deref() == Some("bookmark")));
		let work_snapshot = work_annotations(&ctx, &user.id, "work-1").await.unwrap();
		for id in [
			native_highlight_id.as_str(),
			native_note_id.as_str(),
			native_bookmark_id.as_str(),
		] {
			assert_eq!(
				work_snapshot
					.iter()
					.filter(|record| record.id.as_str() == id)
					.count(),
				1,
				"native annotation {id} should occur once in the work snapshot"
			);
		}
		assert!(
			media_annotation::Entity::find_by_id(liseur_sync_projection_id(
				&user.id,
				&native_highlight_id,
			))
			.one(ctx.conn.as_ref())
			.await
			.unwrap()
			.is_none()
		);
		let origin_row = ctx
			.conn
			.query_one(db_statement(
				ctx.conn.as_ref(),
				"SELECT origin_device_id FROM liseur_sync_annotations
                 WHERE user_id = $1 AND annotation_id = $2",
				vec![user.id.clone().into(), native_highlight_id.clone().into()],
			))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			origin_row
				.try_get::<String>("", "origin_device_id")
				.unwrap(),
			"stump-native"
		);

		let original_locator = media_annotation::Entity::find_by_id(home_highlight_id)
			.one(ctx.conn.as_ref())
			.await
			.unwrap()
			.unwrap()
			.locator;
		let ko_reader_edit = AnnotationInput {
			id: native_highlight_id.clone(),
			base_rev: 1,
			work_id: "work-1".to_owned(),
			edition_sha: Some("edition-1".to_owned()),
			kind: "highlight".to_owned(),
			locator: exported.locator.clone(),
			progression: exported.progression,
			excerpt: exported.excerpt.clone().unwrap(),
			color: Some("orange".to_owned()),
			drawer: None,
			body: "Edited in KOReader".to_owned(),
			client_ts: now_string(),
		};
		let applied = append_annotations(
			&ctx,
			&user.id,
			"koreader-device",
			vec![ko_reader_edit.clone()],
		)
		.await
		.unwrap();
		assert_eq!(applied[0].status, "applied");
		assert_eq!(applied[0].rev, Some(2));
		let updated_native = media_annotation::Entity::find_by_id(home_highlight_id)
			.one(ctx.conn.as_ref())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			updated_native.annotation_text.as_deref(),
			Some("Edited in KOReader")
		);
		assert_eq!(updated_native.color.as_deref(), Some("orange"));
		assert_eq!(updated_native.locator, original_locator);
		let creator_after_edit = ctx
			.conn
			.query_one(db_statement(
				ctx.conn.as_ref(),
				"SELECT origin_device_id, device_id FROM liseur_sync_annotations
                 WHERE user_id = $1 AND annotation_id = $2",
				vec![user.id.clone().into(), native_highlight_id.clone().into()],
			))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			creator_after_edit
				.try_get::<String>("", "origin_device_id")
				.unwrap(),
			"stump-native"
		);
		assert_eq!(
			creator_after_edit
				.try_get::<String>("", "device_id")
				.unwrap(),
			"koreader-device"
		);
		let stale = AnnotationInput {
			base_rev: 1,
			body: "Stale edit".to_owned(),
			..ko_reader_edit
		};
		assert_eq!(
			append_annotations(&ctx, &user.id, "another-device", vec![stale])
				.await
				.unwrap()[0]
				.status,
			"conflict"
		);
		let deleted = delete_annotation(&ctx, &user.id, &native_highlight_id, 2)
			.await
			.unwrap();
		assert_eq!(deleted.status, "applied");
		assert!(media_annotation::Entity::find_by_id(home_highlight_id)
			.one(ctx.conn.as_ref())
			.await
			.unwrap()
			.is_none());
		media_annotation::Entity::delete_by_id(home_note_id)
			.exec(ctx.conn.as_ref())
			.await
			.unwrap();
		bookmark::Entity::delete_by_id(home_bookmark_id)
			.exec(ctx.conn.as_ref())
			.await
			.unwrap();
		let (after_all_deletes, _, _) =
			annotation_changes(&ctx, &user.id, native_high_water, 500)
				.await
				.unwrap();
		let client_tombstone = after_all_deletes
			.iter()
			.find(|record| record.id == native_highlight_id)
			.unwrap();
		assert!(client_tombstone.deleted);
		assert_eq!(client_tombstone.rev, 3);
		let note_tombstone = after_all_deletes
			.iter()
			.find(|record| record.id == native_note_id)
			.unwrap();
		assert!(note_tombstone.deleted);
		assert_eq!(note_tombstone.rev, 2);
		let bookmark_tombstone = after_all_deletes
			.iter()
			.find(|record| record.id == native_bookmark_id)
			.unwrap();
		assert!(bookmark_tombstone.deleted);
		assert_eq!(bookmark_tombstone.rev, 2);
	}
	#[cfg(feature = "graphql")]
	#[tokio::test]
	async fn home_graphql_edits_liseur_note_into_cas_change_feed() {
		use std::sync::Arc;

		use ::tests::{db::test_database, fake_data};
		use async_graphql::Request;
		use models::entity::user::AuthUser;
		use sea_orm::{DatabaseBackend, Schema};

		let db = test_database().await;
		let user = fake_data::User::new("liseur-home-edit").insert(&db).await;
		let library = fake_data::Library::default().insert(&db).await;
		let series = fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&db)
		.await;
		let media = fake_data::Media {
			id: Some("liseur-home-edit-media".to_owned()),
			series_id: series.id,
			..Default::default()
		}
		.insert(&db)
		.await;
		let schema_builder = Schema::new(DatabaseBackend::Sqlite);
		for statement in [
			schema_builder.create_table_from_entity(media_annotation::Entity),
			schema_builder.create_table_from_entity(bookmark::Entity),
		] {
			db.execute(db.get_database_backend().build(&statement))
				.await
				.unwrap();
		}
		for sql in [
			"CREATE TABLE liseur_sync_works (id TEXT NOT NULL, user_id TEXT NOT NULL)",
			"CREATE TABLE liseur_sync_media_links (
				id TEXT PRIMARY KEY, user_id TEXT NOT NULL, work_id TEXT NOT NULL,
				media_id TEXT NOT NULL, edition_sha TEXT, created_at TEXT NOT NULL
			)",
			"CREATE TABLE liseur_sync_counters (
				user_id TEXT PRIMARY KEY, op_seq BIGINT NOT NULL DEFAULT 0,
				annotation_seq BIGINT NOT NULL DEFAULT 0,
				projected_annotation_seq BIGINT NOT NULL DEFAULT 0
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
			db.execute(Statement::from_string(
				DatabaseBackend::Sqlite,
				sql.to_owned(),
			))
			.await
			.unwrap();
		}
		let locator = serde_json::json!({
			"chapterTitle": "Chapter One",
			"href": "OPS/chapter.xhtml",
			"locations": {"position": 12, "progression": 0.25},
			"text": {"before": "before", "highlight": "selected words", "after": "after"}
		});
		let locator_json = serde_json::to_string(&locator).unwrap();
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_works (id, user_id) VALUES ('work-1', $1)",
			vec![user.id.clone().into()],
		))
		.await
		.unwrap();
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_media_links
			 (id, user_id, work_id, media_id, edition_sha, created_at)
			 VALUES ('link-1', $1, 'work-1', $2, 'edition-1', '2026-09-11T12:00:00Z')",
			vec![user.id.clone().into(), media.id.clone().into()],
		))
		.await
		.unwrap();
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_counters
			 (user_id, op_seq, annotation_seq, projected_annotation_seq)
			 VALUES ($1, 0, 1, 0)",
			vec![user.id.clone().into()],
		))
		.await
		.unwrap();
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_annotations
			 (user_id, annotation_id, rev, seq, work_id, edition_sha, kind, locator,
			  progression, excerpt, color, drawer, body, device_id, origin_device_id,
			  client_ts, updated_at, deleted, deleted_at, payload)
			 VALUES ($1, 'liseur-home-note', 1, 1, 'work-1', 'edition-1', 'highlight', $2,
			  0.25, 'selected words', 'yellow', NULL, 'Original note', 'koreader-device',
			  'koreader-device', '2026-09-11T12:00:00Z', '2026-09-11T12:00:00Z',
			  FALSE, NULL, '{}')",
			vec![user.id.clone().into(), locator_json.into()],
		))
		.await
		.unwrap();

		let ctx = Arc::new(stump_core::Ctx::for_testing(db));
		let schema = graphql::schema::build_schema(ctx.clone()).await;
		let auth = stump_auth::AuthContext {
			user: AuthUser {
				id: user.id.clone(),
				username: user.username.clone(),
				is_server_owner: true,
				..Default::default()
			},
			api_key: None,
			device_id: None,
		};
		let response = schema
			.execute(
				Request::new(
					r#"mutation {
						updateAnnotation(input: {
							id: "liseur-home-note"
							annotationText: "Edited in Home"
							color: "blue"
							expectedRevision: 1
						}) { id annotationText color }
					}"#,
				)
				.data(auth.clone()),
			)
			.await;
		assert!(response.errors.is_empty(), "{:?}", response.errors);

		let (feed, high_water, has_more) =
			annotation_changes(&ctx, &user.id, 1, 500).await.unwrap();
		assert_eq!(high_water, 2);
		assert!(!has_more);
		let changed = feed
			.iter()
			.find(|record| record.id == "liseur-home-note")
			.unwrap();
		assert_eq!(changed.rev, 2);
		assert_eq!(changed.seq, 2);
		assert_eq!(changed.body.as_deref(), Some("Edited in Home"));
		assert_eq!(changed.color.as_deref(), Some("blue"));
		assert_eq!(changed.locator.as_ref(), Some(&locator));
		assert!(!changed.deleted);

		let stale_response = schema
			.execute(
				Request::new(
					r#"mutation {
						updateAnnotation(input: {
							id: "liseur-home-note"
							annotationText: "Stale edit"
							expectedRevision: 1
						}) { id }
					}"#,
				)
				.data(auth.clone()),
			)
			.await;
		assert_eq!(
			stale_response.errors[0].message,
			"annotation revision conflict: expected 1, current 2"
		);
		assert!(
			serde_json::to_value(&stale_response.data)
				.unwrap()
				.is_null(),
			"the non-null root mutation field should bubble its conflict to data: null"
		);

		let stale_delete = schema
			.execute(
				Request::new(
					r#"mutation {
						deleteAnnotation(id: "liseur-home-note", expectedRevision: 1) { id }
					}"#,
				)
				.data(auth.clone()),
			)
			.await;
		assert_eq!(
			stale_delete.errors[0].message,
			"annotation revision conflict: expected 1, current 2"
		);
		assert!(
			serde_json::to_value(&stale_delete.data).unwrap().is_null(),
			"the non-null delete field should bubble its conflict to data: null"
		);
		let deleted_response = schema
			.execute(
				Request::new(
					r#"mutation {
						deleteAnnotation(id: "liseur-home-note", expectedRevision: 2) { id }
					}"#,
				)
				.data(auth),
			)
			.await;
		assert!(
			deleted_response.errors.is_empty(),
			"{:?}",
			deleted_response.errors
		);
		let (tombstones, deleted_high_water, deleted_has_more) =
			annotation_changes(&ctx, &user.id, 2, 500).await.unwrap();
		assert_eq!(deleted_high_water, 3);
		assert!(!deleted_has_more);
		let tombstone = tombstones
			.iter()
			.find(|record| record.id == "liseur-home-note")
			.unwrap();
		assert_eq!(tombstone.rev, 3);
		assert_eq!(tombstone.seq, 3);
		assert!(tombstone.deleted);
		assert_eq!(tombstone.body, None);

		let row = ctx
			.conn
			.query_one(db_statement(
				ctx.conn.as_ref(),
				"SELECT rev, origin_device_id, device_id, body, deleted
				 FROM liseur_sync_annotations WHERE user_id = $1 AND annotation_id = $2",
				vec![user.id.into(), "liseur-home-note".into()],
			))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(row.try_get::<i64>("", "rev").unwrap(), 3);
		assert_eq!(
			row.try_get::<String>("", "origin_device_id").unwrap(),
			"koreader-device"
		);
		assert_eq!(
			row.try_get::<String>("", "device_id").unwrap(),
			"stump-native"
		);
		assert_eq!(row.try_get::<String>("", "body").unwrap(), "Edited in Home");
		assert!(row.try_get::<bool>("", "deleted").unwrap());
	}
	#[tokio::test]
	async fn series_names_are_personal_overlays_and_conflicts_are_normalized() {
		use std::sync::Arc;

		use ::tests::{db::test_database, fake_data};
		use sea_orm::{DatabaseBackend, Statement};

		let db = test_database().await;
		let first_user = fake_data::User::new("liseur-series-first")
			.auth_user(&db)
			.await;
		let second_user = fake_data::User::new("liseur-series-second")
			.auth_user(&db)
			.await;
		let first_auth = AuthContext {
			user: first_user.clone(),
			api_key: None,
			device_id: None,
		};
		let second_auth = AuthContext {
			user: second_user,
			api_key: None,
			device_id: None,
		};
		let library = fake_data::Library::default().insert(&db).await;
		let first_series = fake_data::Series {
			id: Some("series-one".to_owned()),
			name: Some("Scanned series".to_owned()),
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&db)
		.await;
		let second_series = fake_data::Series {
			id: Some("series-two".to_owned()),
			name: Some("Other scanned series".to_owned()),
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&db)
		.await;
		let first_media = fake_data::Media {
			id: Some("media-one".to_owned()),
			name: Some("first.epub".to_owned()),
			series_id: first_series.id.clone(),
			..Default::default()
		}
		.insert(&db)
		.await;
		let second_media = fake_data::Media {
			id: Some("media-two".to_owned()),
			name: Some("second.epub".to_owned()),
			series_id: second_series.id.clone(),
			..Default::default()
		}
		.insert(&db)
		.await;
		db.execute(Statement::from_string(
			DatabaseBackend::Sqlite,
			"CREATE TABLE liseur_sync_series_names (
                user_id TEXT NOT NULL, series_id TEXT NOT NULL, name TEXT NOT NULL,
                normalized_name TEXT NOT NULL, updated_at TEXT NOT NULL,
                PRIMARY KEY (user_id, series_id)
            )",
		))
		.await
		.unwrap();
		let ctx = Arc::new(stump_core::Ctx::for_testing(db));

		let renamed = set_series_name(
			&ctx,
			&first_auth,
			&first_series.id,
			"personal",
			"  New   Display Name  ",
		)
		.await
		.unwrap();
		assert_eq!(renamed.name, "New   Display Name");
		assert_eq!(renamed.scanned_name, "Scanned series");
		assert_eq!(renamed.name_source, "personal");
		assert_eq!(renamed.book_count, 1);

		let visible_to_owner = book(&ctx, &first_auth, &first_media.id).await.unwrap();
		let visible_to_other = book(&ctx, &second_auth, &first_media.id).await.unwrap();
		assert_eq!(visible_to_owner.series[0].name, "New   Display Name");
		assert_eq!(visible_to_other.series[0].name, "Scanned series");
		let scanned = series::Entity::find_by_id(&first_series.id)
			.one(ctx.conn.as_ref())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(scanned.name, "Scanned series");

		set_series_name(
			&ctx,
			&first_auth,
			&second_series.id,
			"personal",
			"Other Display",
		)
		.await
		.unwrap();
		assert!(matches!(
			set_series_name(
				&ctx,
				&first_auth,
				&first_series.id,
				"personal",
				" other   display ",
			)
			.await,
			Err(LiseurSyncError::Conflict(_))
		));
		let cleared = clear_series_name(&ctx, &first_auth, &first_series.id, "personal")
			.await
			.unwrap();
		assert_eq!(cleared.name, "Scanned series");
		assert_eq!(cleared.name_source, "folder");
		let still_private = book(&ctx, &second_auth, &second_media.id).await.unwrap();
		assert_eq!(still_private.series[0].name, "Other scanned series");
	}
	#[tokio::test]
	async fn append_sessions_projects_and_deduplicates_liseur_history() {
		use std::sync::Arc;

		use ::tests::{db::test_database, fake_data};
		use models::entity::reading_session;
		use sea_orm::{DatabaseBackend, EntityTrait, QueryFilter, Statement};

		let db = test_database().await;
		let user = fake_data::User::new("liseur-session-user")
			.insert(&db)
			.await;
		let library = fake_data::Library::default().insert(&db).await;
		let series = fake_data::Series {
			library_id: Some(library.id.clone()),
			..Default::default()
		}
		.insert(&db)
		.await;
		let media = fake_data::Media {
			series_id: series.id.clone(),
			id: Some("liseur-session-media".to_owned()),
			..Default::default()
		}
		.insert(&db)
		.await;

		for sql in [
			"CREATE TABLE liseur_sync_works (
                id TEXT NOT NULL,
                user_id TEXT NOT NULL
            )",
			"CREATE TABLE liseur_sync_media_links (
                user_id TEXT NOT NULL,
                work_id TEXT NOT NULL,
                media_id TEXT NOT NULL
            )",
			"CREATE TABLE liseur_sync_sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                work_id TEXT NOT NULL,
                edition_sha TEXT,
                device_id TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                start_progression DOUBLE NOT NULL,
                end_progression DOUBLE NOT NULL,
                idle_ms BIGINT NOT NULL,
                payload TEXT NOT NULL,
                received_at TEXT NOT NULL
            )",
		] {
			db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
				.await
				.unwrap();
		}
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_works (id, user_id) VALUES ($1, $2)",
			vec!["work-1".to_owned().into(), user.id.clone().into()],
		))
		.await
		.unwrap();
		db.execute(db_statement(
			&db,
			"INSERT INTO liseur_sync_media_links (user_id, work_id, media_id)
             VALUES ($1, $2, $3)",
			vec![
				user.id.clone().into(),
				"work-1".to_owned().into(),
				media.id.clone().into(),
			],
		))
		.await
		.unwrap();

		let ctx = Arc::new(stump_core::Ctx::for_testing(db));
		let session = SessionInput {
			session_id: "session-1".to_owned(),
			work_id: "work-1".to_owned(),
			edition_sha: None,
			started_at: "2026-09-11T12:00:00Z".to_owned(),
			ended_at: "2026-09-11T12:00:10Z".to_owned(),
			start_progression: Some(0.25),
			end_progression: Some(0.5),
			idle_ms: 1_000,
			active_ms: None,
		};

		assert_eq!(
			append_sessions(&ctx, &user.id, "liseur-device", vec![session.clone()])
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			append_sessions(&ctx, &user.id, "liseur-device", vec![session.clone()])
				.await
				.unwrap(),
			1
		);
		let measured = SessionInput {
			session_id: "session-2".to_owned(),
			active_ms: Some(2_000),
			..session
		};
		assert_eq!(
			append_sessions(&ctx, &user.id, "liseur-device", vec![measured.clone()])
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			append_sessions(&ctx, &user.id, "liseur-device", vec![measured])
				.await
				.unwrap(),
			1
		);

		let rows = reading_session::Entity::find()
			.filter(reading_session::Column::LiseurSessionId.eq("session-1"))
			.all(ctx.conn.as_ref())
			.await
			.unwrap();
		assert_eq!(rows.len(), 1);
		assert_eq!(rows[0].media_id, media.id);
		assert_eq!(rows[0].elapsed_seconds, Some(9));
		assert_eq!(
			rows[0].device_ids,
			Some(reading_session::DeviceIds(vec!["liseur-device".to_owned()]))
		);

		let measured_rows = reading_session::Entity::find()
			.filter(reading_session::Column::LiseurSessionId.eq("session-2"))
			.all(ctx.conn.as_ref())
			.await
			.unwrap();
		assert_eq!(measured_rows.len(), 1);
		assert_eq!(measured_rows[0].elapsed_seconds, Some(2));

		let source_rows = ctx
			.conn
			.query_all(db_statement(
				ctx.conn.as_ref(),
				"SELECT session_id FROM liseur_sync_sessions
                 WHERE user_id = $1",
				vec![user.id.into()],
			))
			.await
			.unwrap();
		assert_eq!(source_rows.len(), 2);
	}
	#[tokio::test]
	async fn settings_persistence_is_lww_and_account_scoped() {
		use std::sync::Arc;

		use ::tests::{db::test_database, fake_data};
		use sea_orm::{DatabaseBackend, Statement};

		let db = test_database().await;
		let user = fake_data::User::new("liseur-settings-user")
			.insert(&db)
			.await;
		let other_user = fake_data::User::new("liseur-settings-other")
			.insert(&db)
			.await;
		for sql in [
			"CREATE TABLE liseur_sync_settings (
                user_id TEXT NOT NULL,
                setting_key TEXT NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (user_id, setting_key)
            )",
			"CREATE TABLE liseur_sync_counters (
                user_id TEXT PRIMARY KEY,
                op_seq BIGINT NOT NULL DEFAULT 0,
                annotation_seq BIGINT NOT NULL DEFAULT 0
            )",
		] {
			db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
				.await
				.unwrap();
		}
		let ctx = Arc::new(stump_core::Ctx::for_testing(db));
		put_settings(
			&ctx,
			&user.id,
			vec![SettingUpdate {
				key: "reader.theme".into(),
				value: "dark".into(),
				updated_at: "2026-01-01T00:00:00.000000Z".into(),
			}],
		)
		.await
		.unwrap();
		put_settings(
			&ctx,
			&user.id,
			vec![
				SettingUpdate {
					key: "reader.theme".into(),
					value: "light".into(),
					updated_at: "2025-12-31T23:59:59.000000Z".into(),
				},
				SettingUpdate {
					key: "reader.font".into(),
					value: "serif".into(),
					updated_at: "2026-01-02T00:00:00.000000Z".into(),
				},
			],
		)
		.await
		.unwrap();

		let stored = settings(&ctx, &user.id).await.unwrap();
		assert_eq!(stored.len(), 2);
		assert_eq!(stored["reader.theme"].value, "dark");
		assert_eq!(
			stored["reader.theme"].updated_at,
			"2026-01-01T00:00:00.000000Z"
		);
		assert_eq!(stored["reader.font"].value, "serif");
		assert!(settings(&ctx, &other_user.id).await.unwrap().is_empty());
	}
}
