//! Audiobookshelf compatibility profile: the `stump_abs` router over a
//! server-side backend adapter, authenticated by Audiobookshelf's credential
//! rules.
//!
//! The profile authenticates a request with one of:
//! - `Authorization: Bearer <jwt>` minted by `POST /login` or
//!   `POST /auth/refresh` — the ordinary Lissen path, since its OkHttp
//!   interceptor puts the stored token on every request including the
//!   ExoPlayer data source (`channel/common/OkHttpClient.kt:47-59`,
//!   `playback/service/LissenDataSourceFactory.kt:37`);
//! - `Authorization: Bearer stump_...`, a Stump API key, so a device
//!   credential drives the profile without a password login;
//! - `?token=<jwt|api key>`, which abs-ref also accepts
//!   (`../komga-compat/abs/capture/negatives.txt`: `query token /api/me: 200`)
//!   and which casting clients need, because a cast receiver cannot set
//!   headers;
//! - `Authorization: Basic`, for scripts and the replay harness.
//!
//! A password login registers (or reuses) a [`DeviceKind::Abs`] device row and
//! stamps its id into the minted tokens, so the device's library scope narrows
//! every later request on that session.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
	body::Body,
	extract::{Request, State},
	http::{header, HeaderMap, Response},
	middleware::{self, Next},
	response::{IntoResponse, Response as AxumResponse},
	Extension, Router,
};
use base64::Engine;
use chrono::{DateTime, Utc};
use models::{
	entity::{
		device, media, media_audio, media_audio_chapter, media_audio_track, reading_head,
		user, user::AuthUser,
	},
	services::{reading_progress, reading_state as reading_state_service},
	shared::enums::DeviceKind,
};
use prefixed_api_key::PrefixedApiKey;
use sea_orm::{prelude::*, DatabaseConnection, QueryOrder};
use stump_abs::{
	errors::{AbsError, AbsResult},
	model::{
		AbsAudio, AbsAudioChapter, AbsAudioTrack, AbsBookmark, AbsImage,
		AbsPositionUpdate, AbsProgress,
	},
	routes::{AbsBackend, AbsSession},
	AbsIds, AbsSessions, IdKind,
};
use stump_api_types::RequestOrigin;
use stump_auth::AuthContext;
use stump_devices::{CredentialRef, Protocol};
use tower_http::services::ServeFile;

use crate::{
	config::{jwt::access_token_secret, state::AppState},
	errors::APIError,
	middleware::{
		auth::{bind_device, handle_bearer_auth, inject_avatar_url, validate_api_key},
		host::HostExtractor,
	},
	routers::api::v2::media as api_media,
	utils::verify_password,
};

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let backend: Arc<dyn AbsBackend> =
		Arc::new(AbsBackendAdapter::new(app_state.clone()));
	compose(app_state, backend)
}

/// The public routes sit at the server root (`/login`, `/auth/refresh`,
/// `/ping`, `/healthcheck`, `/status`) and the authenticated ones under
/// `/api`, exactly where an Audiobookshelf client looks for them.
fn compose(app_state: AppState, backend: Arc<dyn AbsBackend>) -> Router<AppState> {
	let protected = Router::new()
		.nest("/api", stump_abs::authenticated_router::<AppState>())
		.layer(middleware::from_fn_with_state(
			app_state,
			abs_auth_middleware,
		));
	stump_abs::public_router::<AppState>()
		.merge(protected)
		.layer(Extension(backend))
}

/// The `token` query parameter, if present.
fn token_query(query: Option<&str>) -> Option<String> {
	query?
		.split('&')
		.filter_map(|pair| pair.split_once('='))
		.find(|(key, _)| key.eq_ignore_ascii_case("token"))
		.map(|(_, value)| {
			urlencoding::decode(value)
				.map(|value| value.into_owned())
				.unwrap_or_else(|_| value.to_owned())
		})
		.filter(|value| !value.is_empty())
}

/// `Basic` credentials, decoded.
fn basic_credentials(header_value: &str) -> Option<(String, String)> {
	let encoded = header_value.strip_prefix("Basic ")?.trim();
	let decoded = base64::engine::general_purpose::STANDARD
		.decode(encoded)
		.ok()?;
	let decoded = String::from_utf8(decoded).ok()?;
	let (username, password) = decoded.split_once(':')?;
	Some((username.to_owned(), password.to_owned()))
}

/// Resolve an Audiobookshelf token minted by this profile.
///
/// The `device` claim is the device row the login registered, so the session
/// keeps that device's library scope for its whole lifetime — the same
/// binding the Kobo and KOReader credentials get, without a second
/// round-trip to the credential store.
async fn authenticate_abs_token(
	ctx: &AppState,
	token: &str,
) -> Result<Option<AuthContext>, AxumResponse> {
	if !stump_abs::auth::looks_like_jwt(token) {
		return Ok(None);
	}
	let Ok(secret) = access_token_secret(ctx.conn.as_ref()).await else {
		return Ok(None);
	};
	let Ok(claims) = stump_abs::verify_token(secret.as_bytes(), token) else {
		return Ok(None);
	};

	let user = load_user(ctx.conn.as_ref(), &claims.user_id)
		.await
		.map_err(|error| APIError::from(error).into_response())?;
	let mut auth = AuthContext {
		user,
		api_key: None,
		device_id: None,
	};
	if let Some(device_id) = claims.device.as_deref() {
		apply_device_scope(ctx.conn.as_ref(), &mut auth, device_id)
			.await
			.map_err(|error| error.into_response())?;
	}
	Ok(Some(auth))
}

/// Narrow the request to the library scope of the device its token names.
async fn apply_device_scope(
	conn: &DatabaseConnection,
	auth: &mut AuthContext,
	device_id: &str,
) -> Result<(), APIError> {
	let device = device::Entity::find_by_id(device_id)
		.filter(device::Column::UserId.eq(auth.user.id.clone()))
		.one(conn)
		.await?;
	if let Some(device) = device {
		auth.user.device_library_scope = device.library_scope_ids();
		auth.device_id = Some(device.id);
	}
	Ok(())
}

async fn load_user(
	conn: &DatabaseConnection,
	user_id: &str,
) -> Result<AuthUser, APIError> {
	let user = user::LoginUser::find()
		.filter(user::Column::Id.eq(user_id))
		.filter(user::Column::DeletedAt.is_null())
		.into_model::<user::LoginUser>()
		.one(conn)
		.await?
		.ok_or(APIError::Unauthorized)?;
	if user.is_locked {
		return Err(APIError::Unauthorized);
	}
	Ok(AuthUser::from(user))
}

/// Authenticate an Audiobookshelf request: the profile's own token, then a
/// Stump API key or access token, then Basic. A present credential that fails
/// is a `401` and never falls through to the next one.
async fn abs_auth_middleware(
	State(ctx): State<AppState>,
	HostExtractor(host_details): HostExtractor,
	mut req: Request,
	next: Next,
) -> Result<AxumResponse, AxumResponse> {
	let service =
		RequestOrigin::new(host_details.host.clone(), host_details.scheme.clone());
	let headers = req.headers().clone();
	let authorization = headers
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok())
		.map(str::to_owned);
	let bearer = authorization
		.as_deref()
		.and_then(|value| value.strip_prefix("Bearer "))
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_owned)
		.or_else(|| token_query(req.uri().query()));

	let mut auth = if let Some(token) = bearer {
		match authenticate_abs_token(&ctx, &token).await? {
			Some(auth) => auth,
			None => {
				let mut auth = handle_bearer_auth(token, ctx.conn.as_ref())
					.await
					.map_err(|error| error.into_response())?;
				if let Some(api_key) = auth.api_key.clone() {
					bind_device(
						&ctx,
						&mut auth,
						CredentialRef::ApiKey(&api_key),
						Protocol::Api,
					)
					.await
					.map_err(|error| error.into_response())?;
				}
				auth
			},
		}
	} else if let Some((username, password)) =
		authorization.as_deref().and_then(basic_credentials)
	{
		let adapter = AbsBackendAdapter::new(ctx.clone());
		let (user, device_id) = adapter
			.authenticate_password(&username, &password)
			.await
			.map_err(|_| APIError::Unauthorized.into_response())?;
		let mut auth = AuthContext {
			user,
			api_key: None,
			device_id: None,
		};
		if let Some(device_id) = device_id {
			apply_device_scope(ctx.conn.as_ref(), &mut auth, &device_id)
				.await
				.map_err(|error| error.into_response())?;
		}
		auth
	} else {
		return Err(APIError::Unauthorized.into_response());
	};

	auth.user = inject_avatar_url(auth.user, service);
	req.extensions_mut().insert(auth.user.clone());
	req.extensions_mut().insert(auth);
	Ok(next.run(req).await)
}

pub(crate) struct AbsBackendAdapter {
	ctx: AppState,
}

impl AbsBackendAdapter {
	pub(crate) fn new(ctx: AppState) -> Self {
		Self { ctx }
	}

	/// The `media_audio` row plus its tracks and chapters, in one shape.
	fn assemble(
		audio: media_audio::Model,
		tracks: Vec<media_audio_track::Model>,
		chapters: Vec<media_audio_chapter::Model>,
	) -> AbsAudio {
		AbsAudio {
			duration_ms: audio.duration_ms,
			codec: audio.codec,
			sample_rate: audio.sample_rate,
			channels: audio.channels,
			bitrate: audio.bitrate,
			tracks: tracks
				.into_iter()
				.map(|track| AbsAudioTrack {
					index: track.index,
					path: track.path,
					duration_ms: track.duration_ms,
					start_offset_ms: track.start_offset_ms,
					byte_size: track.byte_size,
					mime: track.mime,
				})
				.collect(),
			chapters: chapters
				.into_iter()
				.map(|chapter| AbsAudioChapter {
					index: chapter.index,
					title: chapter.title,
					start_ms: chapter.start_ms,
					end_ms: chapter.end_ms,
				})
				.collect(),
		}
	}

	/// The reading head of a user for one book, which is where every
	/// protocol's position lands.
	async fn head(
		&self,
		user_id: &str,
		media_id: &str,
	) -> AbsResult<Option<reading_head::Model>> {
		Ok(reading_state_service::head(self.conn(), user_id, media_id).await?)
	}

	/// The device row an Audiobookshelf client is registered as: one per
	/// user, reused across logins so the registry does not grow a row per
	/// sign-in. A user who may not hold API keys simply gets no device, and
	/// the session falls back to their own visibility.
	async fn register_device(&self, user: &AuthUser) -> Option<String> {
		let existing = self
			.ctx
			.devices()
			.list(user)
			.await
			.ok()?
			.into_iter()
			.find(|device| device.kind == DeviceKind::Abs);
		if let Some(device) = existing {
			return Some(device.id);
		}
		match self
			.ctx
			.devices()
			.create_device(user, DeviceKind::Abs, None)
			.await
		{
			Ok((device, _)) => Some(device.id),
			Err(error) => {
				tracing::debug!(
					?error,
					"Could not register an Audiobookshelf device for this user"
				);
				None
			},
		}
	}
}

fn map_server_error(error: APIError) -> AbsError {
	match error {
		APIError::NotFound(message) => AbsError::NotFound(message),
		APIError::BadRequest(message) => AbsError::BadRequest(message),
		APIError::Unauthorized => AbsError::Unauthorized,
		APIError::Forbidden(message) => AbsError::Forbidden(message),
		other => AbsError::InternalServerError(other.to_string()),
	}
}

/// A head's position as this profile reports it. A head with no `position_ms`
/// is a book read by a page- or locator-addressed client; its progression
/// still resumes an audiobook, so it is projected back onto the timeline.
fn progress_from_head(head: &reading_head::Model, duration_ms: i64) -> AbsProgress {
	let position_ms = head.position_ms.unwrap_or_else(|| {
		if duration_ms > 0 {
			(head.progression.clamp(0.0, 1.0) * duration_ms as f64).round() as i64
		} else {
			0
		}
	});
	AbsProgress {
		position_ms,
		track_index: head.track_index,
		is_finished: head.completed,
		started_at: head.created_at.to_utc(),
		last_update: head.updated_at.to_utc(),
		finished_at: head.completed.then(|| head.updated_at.to_utc()),
	}
}

#[async_trait]
impl AbsBackend for AbsBackendAdapter {
	fn conn(&self) -> &DatabaseConnection {
		self.ctx.conn.as_ref()
	}

	async fn token_secret(&self) -> AbsResult<Vec<u8>> {
		access_token_secret(self.conn())
			.await
			.map(String::into_bytes)
			.map_err(map_server_error)
	}

	async fn authenticate_password(
		&self,
		username: &str,
		password: &str,
	) -> AbsResult<(AuthUser, Option<String>)> {
		let candidate = user::LoginUser::find()
			.filter(user::Column::Username.eq(username))
			.filter(user::Column::DeletedAt.is_null())
			.into_model::<user::LoginUser>()
			.one(self.conn())
			.await?
			.ok_or(AbsError::Unauthorized)?;

		// A device API key may stand in for the password, so a client that
		// only has a key can still use the login form.
		let verified = if let Ok(api_key) = PrefixedApiKey::from_string(password) {
			validate_api_key(api_key, self.conn())
				.await
				.map(|user| user.id == candidate.id)
				.unwrap_or(false)
		} else {
			verify_password(&candidate.hashed_password, password)
				.map_err(|error| AbsError::InternalServerError(error.to_string()))?
		};
		if !verified || candidate.is_locked {
			return Err(AbsError::Unauthorized);
		}

		let user = AuthUser::from(candidate);
		let device_id = self.register_device(&user).await;
		Ok((user, device_id))
	}

	fn is_docker(&self) -> bool {
		std::path::Path::new("/.dockerenv").exists()
	}

	async fn user(&self, user_id: &str) -> AbsResult<(AuthUser, DateTime<Utc>)> {
		let row = user::Entity::find_by_id(user_id)
			.filter(user::Column::DeletedAt.is_null())
			.one(self.conn())
			.await?
			.ok_or(AbsError::Unauthorized)?;
		let created_at = row.created_at.to_utc();
		let user = load_user(self.conn(), user_id)
			.await
			.map_err(map_server_error)?;
		Ok((user, created_at))
	}

	async fn audio(&self, media_id: &str) -> AbsResult<Option<AbsAudio>> {
		let Some(audio) = media_audio::Entity::find_by_id(media_id)
			.one(self.conn())
			.await?
		else {
			return Ok(None);
		};
		let tracks = media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.eq(media_id))
			.order_by_asc(media_audio_track::Column::Index)
			.all(self.conn())
			.await?;
		let chapters = media_audio_chapter::Entity::find()
			.filter(media_audio_chapter::Column::MediaId.eq(media_id))
			.order_by_asc(media_audio_chapter::Column::Index)
			.all(self.conn())
			.await?;
		Ok(Some(Self::assemble(audio, tracks, chapters)))
	}

	async fn audio_batch(
		&self,
		media_ids: &[String],
	) -> AbsResult<std::collections::HashMap<String, AbsAudio>> {
		use std::collections::HashMap;

		if media_ids.is_empty() {
			return Ok(HashMap::new());
		}
		let rows = media_audio::Entity::find()
			.filter(media_audio::Column::MediaId.is_in(media_ids.to_vec()))
			.all(self.conn())
			.await?;
		if rows.is_empty() {
			return Ok(HashMap::new());
		}
		let mut tracks: HashMap<String, Vec<media_audio_track::Model>> = HashMap::new();
		for track in media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.is_in(media_ids.to_vec()))
			.order_by_asc(media_audio_track::Column::Index)
			.all(self.conn())
			.await?
		{
			tracks
				.entry(track.media_id.clone())
				.or_default()
				.push(track);
		}
		let mut chapters: HashMap<String, Vec<media_audio_chapter::Model>> =
			HashMap::new();
		for chapter in media_audio_chapter::Entity::find()
			.filter(media_audio_chapter::Column::MediaId.is_in(media_ids.to_vec()))
			.order_by_asc(media_audio_chapter::Column::Index)
			.all(self.conn())
			.await?
		{
			chapters
				.entry(chapter.media_id.clone())
				.or_default()
				.push(chapter);
		}

		Ok(rows
			.into_iter()
			.map(|audio| {
				let media_id = audio.media_id.clone();
				let tracks = tracks.remove(&media_id).unwrap_or_default();
				let chapters = chapters.remove(&media_id).unwrap_or_default();
				let audio = Self::assemble(audio, tracks, chapters);
				(media_id, audio)
			})
			.collect())
	}

	async fn progress(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Option<AbsProgress>> {
		let Some(head) = self.head(&user.id, media_id).await? else {
			return Ok(None);
		};
		let duration_ms = media_audio::Entity::find_by_id(media_id)
			.one(self.conn())
			.await?
			.map(|audio| audio.duration_ms)
			.unwrap_or(0);
		Ok(Some(progress_from_head(&head, duration_ms)))
	}

	async fn progress_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsProgress)>> {
		let heads = reading_head::Entity::find()
			.filter(reading_head::Column::UserId.eq(user.id.clone()))
			.order_by_desc(reading_head::Column::UpdatedAt)
			.all(self.conn())
			.await?;
		if heads.is_empty() {
			return Ok(Vec::new());
		}
		let media_ids = heads
			.iter()
			.map(|head| head.media_id.clone())
			.collect::<Vec<_>>();
		let durations = media_audio::Entity::find()
			.filter(media_audio::Column::MediaId.is_in(media_ids))
			.all(self.conn())
			.await?
			.into_iter()
			.map(|audio| (audio.media_id, audio.duration_ms))
			.collect::<std::collections::HashMap<_, _>>();

		Ok(heads
			.into_iter()
			.map(|head| {
				let duration_ms =
					durations.get(&head.media_id).copied().unwrap_or_default();
				let media_id = head.media_id.clone();
				(media_id, progress_from_head(&head, duration_ms))
			})
			.collect())
	}

	async fn apply_position(
		&self,
		user: &AuthUser,
		media_id: &str,
		update: AbsPositionUpdate,
	) -> AbsResult<()> {
		let book = media::Entity::find_for_user(user)
			.filter(media::Column::Id.eq(media_id))
			.one(self.conn())
			.await?
			.ok_or_else(|| AbsError::NotFound(format!("No library item {media_id}")))?;

		let progression = (update.duration_ms > 0).then(|| {
			(update.position_ms as f64 / update.duration_ms as f64).clamp(0.0, 1.0)
		});
		let publication = stump_core::reading_state::Publication::from(&book);
		let publication = if update.duration_ms > 0 {
			publication.with_duration_ms(update.duration_ms)
		} else {
			publication
		};

		let txn = models::txn::begin_write(self.conn()).await?;
		let applied = reading_state_service::apply(
			&txn,
			&user.id,
			publication,
			stump_core::reading_state::ProtocolUpdate {
				protocol: stump_core::reading_state::SourceProtocol::Abs,
				device_id: None,
				updated_at: None,
				position: stump_core::reading_state::Position::Time {
					position_ms: update.position_ms,
					track_index: update.track_index,
				},
				progression,
				completed: update.is_finished,
				raw_payload: serde_json::json!({
					"currentTime": update.position_ms as f64 / 1000.0,
					"timeListened": update.elapsed_ms as f64 / 1000.0,
					"duration": update.duration_ms as f64 / 1000.0,
				}),
			},
		)
		.await?;

		// The listening session is what turns "positions arrived" into time
		// spent listening, which the reading-session history reports.
		reading_progress::upsert_reading_session(
			&txn,
			user,
			media_id,
			reading_progress::NormalizedProgression {
				percentage: progression
					.and_then(|value| sea_orm::prelude::Decimal::from_f64_retain(value)),
				elapsed_seconds_delta: Some(update.elapsed_ms / 1000),
				did_complete: applied.head.completed,
				..Default::default()
			},
		)
		.await?;
		txn.commit().await?;

		stump_core::reading_state::announce(self.ctx.as_ref(), &book, &applied);
		Ok(())
	}

	async fn bookmarks(
		&self,
		user: &AuthUser,
		media_id: &str,
	) -> AbsResult<Vec<AbsBookmark>> {
		Ok(audio_bookmarks(self.conn(), &user.id, Some(media_id))
			.await?
			.into_iter()
			.map(|(_, bookmark)| bookmark)
			.collect())
	}

	async fn bookmarks_all(
		&self,
		user: &AuthUser,
	) -> AbsResult<Vec<(String, AbsBookmark)>> {
		audio_bookmarks(self.conn(), &user.id, None).await
	}

	async fn upsert_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
		title: &str,
	) -> AbsResult<AbsBookmark> {
		use models::entity::bookmark;

		let existing = bookmark::Entity::find()
			.filter(bookmark::Column::UserId.eq(user.id.clone()))
			.filter(bookmark::Column::MediaId.eq(media_id))
			.filter(bookmark::Column::PositionMs.eq(position_ms))
			.one(self.conn())
			.await?;

		let row = match existing {
			Some(row) => {
				let created_at = row.created_at;
				let mut active: bookmark::ActiveModel = row.into();
				active.preview_content =
					sea_orm::ActiveValue::Set(Some(title.to_owned()));
				let updated = active.update(self.conn()).await?;
				bookmark::Model {
					created_at,
					..updated
				}
			},
			None => {
				bookmark::ActiveModel {
					user_id: sea_orm::ActiveValue::Set(user.id.clone()),
					media_id: sea_orm::ActiveValue::Set(media_id.to_owned()),
					position_ms: sea_orm::ActiveValue::Set(Some(position_ms)),
					preview_content: sea_orm::ActiveValue::Set(Some(title.to_owned())),
					created_at: sea_orm::ActiveValue::Set(Utc::now().into()),
					..Default::default()
				}
				.insert(self.conn())
				.await?
			},
		};

		Ok(AbsBookmark {
			position_ms,
			title: row.preview_content.unwrap_or_default(),
			created_at: row.created_at.to_utc(),
		})
	}

	async fn delete_bookmark(
		&self,
		user: &AuthUser,
		media_id: &str,
		position_ms: i64,
	) -> AbsResult<()> {
		use models::entity::bookmark;

		bookmark::Entity::delete_many()
			.filter(bookmark::Column::UserId.eq(user.id.clone()))
			.filter(bookmark::Column::MediaId.eq(media_id))
			.filter(bookmark::Column::PositionMs.eq(position_ms))
			.exec(self.conn())
			.await?;
		Ok(())
	}

	async fn cover(&self, user: &AuthUser, media_id: &str) -> AbsResult<AbsImage> {
		let image = api_media::get_media_thumbnail_by_id(
			self.ctx.as_ref(),
			user,
			media_id.to_owned(),
		)
		.await
		.map_err(map_server_error)?;
		Ok(AbsImage {
			content_type: image.content_type.to_string(),
			data: image.data,
		})
	}

	async fn serve_track(
		&self,
		headers: HeaderMap,
		media_id: &str,
		track_index: i32,
	) -> AbsResult<Response<Body>> {
		let track = media_audio_track::Entity::find()
			.filter(media_audio_track::Column::MediaId.eq(media_id))
			.filter(media_audio_track::Column::Index.eq(track_index))
			.one(self.conn())
			.await?
			.ok_or_else(|| {
				AbsError::NotFound(format!("No file {track_index} for {media_id}"))
			})?;

		// The original headers are reused so `Range` reaches `ServeFile`:
		// seeking in an audiobook is a range request per seek.
		let mut serve_req = Request::new(Body::empty());
		*serve_req.headers_mut() = headers;
		let mut response = ServeFile::new(&track.path)
			.try_call(serve_req)
			.await
			.map_err(|error| {
				AbsError::InternalServerError(format!(
					"Failed to serve {}: {error}",
					track.path
				))
			})?;
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			track
				.mime
				.parse()
				.unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
		);
		Ok(response.map(Body::new))
	}

	async fn create_session(&self, session: AbsSession) -> AbsResult<()> {
		Ok(AbsSessions::insert(self.conn(), &session).await?)
	}

	async fn session(
		&self,
		user_id: &str,
		session_id: &str,
	) -> AbsResult<Option<AbsSession>> {
		Ok(AbsSessions::get(self.conn(), user_id, session_id).await?)
	}

	async fn update_session(
		&self,
		session_id: &str,
		current_time_ms: i64,
		time_listening_ms: i64,
	) -> AbsResult<()> {
		Ok(AbsSessions::update(
			self.conn(),
			session_id,
			current_time_ms,
			time_listening_ms,
		)
		.await?)
	}

	async fn close_session(&self, session_id: &str) -> AbsResult<()> {
		Ok(AbsSessions::close(self.conn(), session_id).await?)
	}

	async fn book_ids(
		&self,
		media_ids: &[String],
	) -> AbsResult<std::collections::HashMap<String, String>> {
		Ok(AbsIds::resolve_many(self.conn(), IdKind::Book, media_ids).await?)
	}

	async fn folder_id(&self, library_id: &str) -> AbsResult<String> {
		Ok(AbsIds::resolve(self.conn(), IdKind::Folder, library_id).await?)
	}

	async fn author_ids(
		&self,
		names: &[String],
	) -> AbsResult<std::collections::HashMap<String, String>> {
		Ok(AbsIds::resolve_many(self.conn(), IdKind::Author, names).await?)
	}

	async fn author_name(&self, author_id: &str) -> AbsResult<Option<String>> {
		Ok(AbsIds::lookup(self.conn(), IdKind::Author, author_id).await?)
	}
}

/// The user's audio bookmarks — the rows that carry a millisecond position —
/// newest position first, optionally for one book.
async fn audio_bookmarks(
	conn: &DatabaseConnection,
	user_id: &str,
	media_id: Option<&str>,
) -> AbsResult<Vec<(String, AbsBookmark)>> {
	use models::entity::bookmark;

	let mut select = bookmark::Entity::find()
		.filter(bookmark::Column::UserId.eq(user_id))
		.filter(bookmark::Column::PositionMs.is_not_null())
		.order_by_asc(bookmark::Column::PositionMs);
	if let Some(media_id) = media_id {
		select = select.filter(bookmark::Column::MediaId.eq(media_id));
	}

	Ok(select
		.all(conn)
		.await?
		.into_iter()
		.filter_map(|row| {
			let position_ms = row.position_ms?;
			Some((
				row.media_id.clone(),
				AbsBookmark {
					position_ms,
					title: row.preview_content.clone().unwrap_or_default(),
					created_at: row.created_at.to_utc(),
				},
			))
		})
		.collect())
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{DatabaseBackend, MockDatabase};

	/// Axum panics on overlapping routes when the router is built, and this
	/// profile adds routes at the server root next to `/api`; catch a
	/// collision here rather than at server start.
	#[tokio::test]
	async fn abs_router_composes_without_route_collisions() {
		let ctx = Arc::new(stump_core::Ctx::mock_sea(MockDatabase::new(
			DatabaseBackend::Sqlite,
		)));
		let backend: Arc<dyn AbsBackend> = Arc::new(AbsBackendAdapter::new(ctx.clone()));
		let _router: Router<()> = compose(ctx.clone(), backend).with_state(ctx);
	}

	#[test]
	fn token_query_is_case_insensitive_and_decoded() {
		// abs-ref accepts `?token=`; a cast receiver cannot set headers.
		assert_eq!(
			token_query(Some("raw=1&token=abc%2Bdef")),
			Some("abc+def".to_owned())
		);
		assert_eq!(token_query(Some("TOKEN=x")), Some("x".to_owned()));
		assert_eq!(token_query(Some("token=")), None);
		assert_eq!(token_query(None), None);
	}

	#[test]
	fn basic_credentials_split_on_the_first_colon() {
		let header = format!(
			"Basic {}",
			base64::engine::general_purpose::STANDARD.encode("ada:pa:ss")
		);
		assert_eq!(
			basic_credentials(&header),
			Some(("ada".to_owned(), "pa:ss".to_owned()))
		);
		assert_eq!(basic_credentials("Bearer abc"), None);
		assert_eq!(basic_credentials("Basic !!!"), None);
	}
}
