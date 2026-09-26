use std::{
	collections::{HashMap, HashSet},
	sync::{Arc, LazyLock, Mutex},
	time::Duration,
};

use async_graphql::{Context, Object, Result, SimpleObject, ID};
use metadata_integrations::{
	AudibleClient, BriefSearchCache, ExternalMediaMetadata, HardcoverClient,
	MetadataProvider, SearchQuery,
};
use models::{
	entity::{
		book_request, hardcover_connection, media, media_metadata,
		metadata_provider_config, series,
	},
	shared::enums::MetadataProvider as MetadataProviderKind,
};
use sea_orm::{
	sea_query::{Expr, Func, IntoColumnRef, LikeExpr, SimpleExpr},
	ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use stump_auth::AuthContext;
use stump_core::{
	filesystem::metadata::ProviderClientCache, utils::encryption::decrypt_string,
};

use crate::data::CoreContext;

#[derive(Debug, Clone, SimpleObject)]
pub struct UnifiedLibraryHit {
	pub media_id: ID,
	pub title: String,
	pub authors: Option<String>,
	pub series_name: Option<String>,
	pub extension: String,
	pub is_audiobook: bool,
	pub thumbnail_url: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct UnifiedExternalHit {
	pub provider: String,
	pub remote_id: String,
	pub title: String,
	pub authors: Option<String>,
	pub year: Option<i32>,
	pub series_name: Option<String>,
	pub series_position: Option<String>,
	pub isbn13: Option<String>,
	pub isbn10: Option<String>,
	pub cover_url: Option<String>,
	/// Whether the provider knows an ebook edition exists; `null` when it
	/// does not say.
	pub has_ebook: Option<bool>,
	/// Whether the provider knows an audiobook edition exists; always `true`
	/// for Audible hits, `null` when the provider does not say.
	pub has_audiobook: Option<bool>,
	/// Length of the default audiobook edition, in seconds.
	pub audio_seconds: Option<i32>,
	/// The Audible product identifier; `null` for other providers.
	pub asin: Option<String>,
	/// The credited readers of an Audible edition; `null` when the provider
	/// does not carry narrators on a search hit.
	pub narrators: Option<Vec<String>>,
	pub in_library_media_ids: Vec<ID>,
	pub existing_request_id: Option<ID>,
	pub existing_request_status: Option<String>,
}

/// External (provider) hits for one query. `provider` is `null` when no
/// Hardcover credential is available to the caller; `error` carries a
/// provider failure without failing the whole operation.
#[derive(Debug, Clone, SimpleObject)]
pub struct ExternalBookSearch {
	pub provider: Option<String>,
	pub error: Option<String>,
	pub hits: Vec<UnifiedExternalHit>,
}

#[derive(Default)]
pub struct UnifiedSearchQuery;

#[derive(Clone, Copy)]
struct LibraryMatch<'a> {
	media_id: &'a str,
	title: &'a str,
	authors: Option<&'a str>,
	isbn: Option<&'a str>,
}

pub(super) type SearchProvider = Arc<dyn MetadataProvider + Send + Sync>;

const EXTERNAL_SEARCH_TTL: Duration = Duration::from_secs(5 * 60);
const EXTERNAL_SEARCH_CAPACITY: usize = 256;

/// Audible's catalog is keyless and answers quickly or not at all; a slow
/// answer is worth less than the Hardcover column it would hold up.
pub(super) const AUDIBLE_SEARCH_TIMEOUT: Duration = Duration::from_secs(3);

/// The longest an interactive Hardcover call (search, audio editions) may
/// take. The client has retries but no request timeout of its own, so a
/// connection Hardcover accepts and then stalls would otherwise pend for as
/// long as the caller waits; the search dialog and the narrator picker both
/// answer with an error instead.
pub(super) const HARDCOVER_TIMEOUT: Duration = Duration::from_secs(5);

/// The message a Hardcover search reports when [`HARDCOVER_TIMEOUT`] passes.
pub(super) const HARDCOVER_TIMED_OUT: &str = "Hardcover search timed out";

/// Cache scope for Audible hits: there is no credential, so every caller
/// shares one entry per query.
pub(super) const AUDIBLE_SCOPE: &str = "audible";

/// Process-wide state for external search: the provider hit cache and the
/// provider clients (one per credential scope) so a keystroke does not
/// rebuild an HTTP client and re-decrypt a token.
struct ExternalSearchState {
	results: BriefSearchCache,
	/// `scope -> (encrypted token the client was built from, client)`; a
	/// changed stored token invalidates the client.
	providers: Mutex<HashMap<String, (String, SearchProvider)>>,
}

impl ExternalSearchState {
	fn provider(&self, scope: &str, fingerprint: &str) -> Option<SearchProvider> {
		let providers = self.providers.lock().unwrap_or_else(|e| e.into_inner());
		providers
			.get(scope)
			.filter(|(stored, _)| stored == fingerprint)
			.map(|(_, provider)| Arc::clone(provider))
	}

	fn store_provider(
		&self,
		scope: String,
		fingerprint: String,
		provider: &SearchProvider,
	) {
		let mut providers = self.providers.lock().unwrap_or_else(|e| e.into_inner());
		if providers.len() >= EXTERNAL_SEARCH_CAPACITY {
			providers.clear();
		}
		providers.insert(scope, (fingerprint, Arc::clone(provider)));
	}
}

static EXTERNAL_SEARCH: LazyLock<ExternalSearchState> =
	LazyLock::new(|| ExternalSearchState {
		results: BriefSearchCache::new(EXTERNAL_SEARCH_TTL, EXTERNAL_SEARCH_CAPACITY),
		providers: Mutex::new(HashMap::new()),
	});

/// One keyless Audible client for the process, so its rate limiter counts
/// every caller's searches together.
static AUDIBLE: LazyLock<AudibleClient> = LazyLock::new(AudibleClient::new);

pub(super) fn audible_provider() -> &'static AudibleClient {
	&AUDIBLE
}

#[Object]
impl UnifiedSearchQuery {
	/// Search the caller's visible library by title, author, series, or
	/// ISBN. Never touches the network.
	async fn library_search(
		&self,
		ctx: &Context<'_>,
		query: String,
		#[graphql(default = 20)] limit: Option<i32>,
	) -> Result<Vec<UnifiedLibraryHit>> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let query = query.trim();
		if query.is_empty() {
			return Ok(Vec::new());
		}

		let limit = limit.unwrap_or(20).clamp(1, 50) as u64;
		let pattern = contains_pattern(query);
		let mut local_condition = Condition::any()
			.add(folded_like((media::Entity, media::Column::Name), &pattern))
			.add(folded_like(
				(media_metadata::Entity, media_metadata::Column::Title),
				&pattern,
			))
			.add(folded_like(
				(media_metadata::Entity, media_metadata::Column::Writers),
				&pattern,
			))
			.add(folded_like(
				(media_metadata::Entity, media_metadata::Column::Series),
				&pattern,
			))
			.add(folded_like(
				(series::Entity, series::Column::Name),
				&pattern,
			))
			.add(folded_like(
				(
					media_metadata::Entity,
					media_metadata::Column::IdentifierIsbn,
				),
				&pattern,
			));
		if let Some(isbn) = query_isbn(query) {
			local_condition = local_condition.add(folded_like(
				(
					media_metadata::Entity,
					media_metadata::Column::IdentifierIsbn,
				),
				&contains_pattern(&isbn),
			));
		}
		let local_models = media::ModelWithMetadata::find_for_user(&auth.user)
			.filter(media::Column::DeletedAt.is_null())
			.filter(local_condition)
			.order_by_asc(media::Column::Name)
			.order_by_asc(media::Column::Id)
			.limit(limit)
			.into_model::<media::ModelWithMetadata>()
			.all(core.conn.as_ref())
			.await?;
		let series_ids = local_models
			.iter()
			.filter_map(|row| row.media.series_id.clone())
			.collect::<Vec<_>>();
		let series_names: HashMap<String, String> = if series_ids.is_empty() {
			HashMap::new()
		} else {
			series::Entity::find()
				.filter(series::Column::Id.is_in(series_ids))
				.all(core.conn.as_ref())
				.await?
				.into_iter()
				.map(|series| (series.id, series.name))
				.collect()
		};

		let origin = ctx.data::<stump_api_types::RequestOrigin>()?;
		Ok(local_models
			.into_iter()
			.map(|row| {
				let media = row.media;
				let (title, authors, metadata_series) =
					row.metadata.map_or((None, None, None), |metadata| {
						(metadata.title, metadata.writers, metadata.series)
					});
				let title = title
					.filter(|title| !title.trim().is_empty())
					.unwrap_or(media.name);
				let series_name = media
					.series_id
					.as_ref()
					.and_then(|series_id| series_names.get(series_id).cloned())
					.or(metadata_series);
				let is_audiobook = media::AUDIO_EXTENSIONS
					.iter()
					.any(|extension| extension.eq_ignore_ascii_case(&media.extension));
				let thumbnail_url = origin.cache_friendly_url(
					format!("/api/v2/media/{}/thumbnail", media.id),
					&media.updated_at,
				);

				UnifiedLibraryHit {
					media_id: ID::from(media.id),
					title,
					authors,
					series_name,
					extension: media.extension,
					is_audiobook,
					thumbnail_url,
				}
			})
			.collect())
	}

	/// Search Hardcover when a credential is available: the server's enabled
	/// Hardcover provider first, otherwise only this caller's own personal
	/// connection. Provider hits are served from a short in-memory cache;
	/// library matches and request status are recomputed for every call.
	async fn external_book_search(
		&self,
		ctx: &Context<'_>,
		query: String,
		#[graphql(default = 10)] limit: Option<i32>,
	) -> Result<ExternalBookSearch> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let query = query.trim();
		if query.is_empty() {
			return Ok(ExternalBookSearch {
				provider: None,
				error: None,
				hits: Vec::new(),
			});
		}

		let limit = limit.unwrap_or(10).clamp(1, 50) as u32;
		let (provider, error, external) =
			match get_hardcover_provider(core, &auth.user.id).await {
				Ok(Some((scope, provider))) => {
					let (error, external) = hardcover_search(
						&EXTERNAL_SEARCH.results,
						&scope,
						provider.as_ref(),
						query,
						limit,
						HARDCOVER_TIMEOUT,
					)
					.await;
					(Some("hardcover".to_owned()), error, external)
				},
				Ok(None) => (None, None, Vec::new()),
				Err(error) => (Some("hardcover".to_owned()), Some(error), Vec::new()),
			};

		let hits = annotate_external_hits(core, auth, external, limit as usize).await?;
		Ok(ExternalBookSearch {
			provider,
			error,
			hits,
		})
	}

	/// Search Audible's public catalog, independently of Hardcover: no
	/// credential, its own short cache, and a hard timeout so a slow answer
	/// never holds up the page. Hits keep the catalog's own ranking and are
	/// kept only when they plausibly answer the query (title equals or begins
	/// with it, or every query word appears in title and authors). Hits are
	/// not deduplicated against Hardcover; the caller hides those it already
	/// shows.
	async fn audible_book_search(
		&self,
		ctx: &Context<'_>,
		query: String,
		#[graphql(default = 5)] limit: Option<i32>,
		#[graphql(
			desc = "ISO 639 code or name of the language hits must be in; default `en`"
		)]
		language: Option<String>,
	) -> Result<ExternalBookSearch> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let query = query.trim();
		if query.is_empty() {
			return Ok(ExternalBookSearch {
				provider: Some(AUDIBLE_SCOPE.to_owned()),
				error: None,
				hits: Vec::new(),
			});
		}

		let limit = limit.unwrap_or(5).clamp(1, 50) as usize;
		let language = requested_language(language.as_deref());
		let (error, external) = audible_search(
			audible_provider(),
			&EXTERNAL_SEARCH.results,
			query,
			limit,
			&language,
		)
		.await;
		let hits = annotate_external_hits(core, auth, external, limit).await?;
		Ok(ExternalBookSearch {
			provider: Some(AUDIBLE_SCOPE.to_owned()),
			error,
			hits,
		})
	}
}

/// The language editions must be in: a lowercased ISO 639 code or name,
/// `en` when the caller says nothing.
pub(super) fn requested_language(language: Option<&str>) -> String {
	language
		.map(|value| value.trim().to_lowercase())
		.filter(|value| !value.is_empty())
		.unwrap_or_else(|| "en".to_owned())
}

/// Whether an edition's language, as its provider spells it (Hardcover
/// `code2`/`code3`/name, Audible `english`/`italian`), is the requested one.
/// Codes and names of one language share a prefix (`en`, `eng`, `english`),
/// which is what makes `en` match all three; an edition with no language is
/// kept, since the filter exists to drop known translations, not unknowns.
pub(super) fn language_matches(wanted: &str, found: Option<&str>) -> bool {
	match found.map(str::trim).filter(|found| !found.is_empty()) {
		None => true,
		Some(found) => {
			let found = found.to_lowercase();
			found == wanted || found.starts_with(wanted) || wanted.starts_with(&found)
		},
	}
}

/// One cached Hardcover index search under the caller's credential `scope`,
/// given up after `deadline`. A timeout is reported like any other provider
/// failure ([`HARDCOVER_TIMED_OUT`]) and, like one, never cached, so the
/// next keystroke asks again.
async fn hardcover_search(
	cache: &BriefSearchCache,
	scope: &str,
	provider: &dyn MetadataProvider,
	query: &str,
	limit: u32,
	deadline: Duration,
) -> (Option<String>, Vec<ExternalMediaMetadata>) {
	let query_spec = SearchQuery {
		title: query.to_owned(),
		isbn: query_isbn(query),
		limit: Some(limit),
		..Default::default()
	};
	let search = cache.search_media_brief(scope, provider, &query_spec);
	match tokio::time::timeout(deadline, search).await {
		Ok(Ok(outcome)) => (
			None,
			outcome
				.candidates
				.iter()
				.filter_map(|candidate| candidate.metadata.as_media().cloned())
				.collect(),
		),
		Ok(Err(error)) => (Some(error.to_string()), Vec::new()),
		Err(_) => (Some(HARDCOVER_TIMED_OUT.to_owned()), Vec::new()),
	}
}

/// One cached, time-boxed catalog search, filtered for relevance and for
/// `language` (translations are listed beside the original; unknown
/// language is kept). Asks for twice the wanted count so noise dropped by
/// the filters does not leave the column short. An error (including the
/// timeout) yields no hits and a message; it is never cached.
async fn audible_search(
	provider: &dyn MetadataProvider,
	cache: &BriefSearchCache,
	query: &str,
	limit: usize,
	language: &str,
) -> (Option<String>, Vec<ExternalMediaMetadata>) {
	let query_spec = SearchQuery {
		title: query.to_owned(),
		limit: Some((limit * 2).min(50) as u32),
		..Default::default()
	};
	let search = cache.search_media_brief(AUDIBLE_SCOPE, provider, &query_spec);
	let outcome = match tokio::time::timeout(AUDIBLE_SEARCH_TIMEOUT, search).await {
		Ok(Ok(outcome)) => outcome,
		Ok(Err(error)) => return (Some(error.to_string()), Vec::new()),
		Err(_) => return (Some("Audible search timed out".to_owned()), Vec::new()),
	};
	let hits = outcome
		.candidates
		.iter()
		.filter_map(|candidate| candidate.metadata.as_media())
		.filter(|item| audible_hit_is_relevant(query, item))
		.filter(|item| language_matches(language, item.language.as_deref()))
		.take(limit)
		.cloned()
		.collect();
	(None, hits)
}

/// A product with no narrator and under an hour of audio (or no stated
/// length) is a radio hour, a sample or a placeholder listing, never the
/// audiobook someone would request.
const AUDIBLE_MIN_UNNARRATED_MINUTES: i32 = 60;

/// The catalog pads a search with loosely related products; keep a hit only
/// when its title is the query or begins with it as whole words ("Project
/// Hail Mary: A Novel"), or when every query word appears in the title
/// ("hail mary" for "Project Hail Mary"). Authors and descriptions do not
/// count: a talk-radio hour that mentions the book in passing carries the
/// words too. "Pride and Prejudice" for "project hail mary" fails and is
/// dropped, as is any unnarrated product shorter than an hour.
fn audible_hit_is_relevant(query: &str, item: &ExternalMediaMetadata) -> bool {
	let wanted = normalize_words(query);
	let title = item
		.title
		.as_deref()
		.map(normalize_words)
		.unwrap_or_default();
	if wanted.is_empty() || title.is_empty() {
		return false;
	}
	let unnarrated = item
		.narrators
		.as_ref()
		.is_none_or(|narrators| narrators.is_empty());
	if unnarrated && item.runtime_minutes.unwrap_or(0) < AUDIBLE_MIN_UNNARRATED_MINUTES {
		return false;
	}
	if title == wanted
		|| title
			.strip_prefix(wanted.as_str())
			.is_some_and(|rest| rest.starts_with(' '))
	{
		return true;
	}
	let words = title.split(' ').collect::<HashSet<_>>();
	wanted.split(' ').all(|word| words.contains(word))
}

/// Attach the caller-visible library matches and the caller-visible existing
/// request (if any) to each provider hit. Always computed fresh: these
/// depend on who is asking and change as requests are filed.
async fn annotate_external_hits(
	core: &CoreContext,
	auth: &AuthContext,
	external: Vec<ExternalMediaMetadata>,
	limit: usize,
) -> Result<Vec<UnifiedExternalHit>> {
	let mut external_library_models = Vec::new();
	let mut seen_media_ids = HashSet::new();
	for chunk in external.chunks(8) {
		let Some(condition) = external_library_match_condition(chunk) else {
			continue;
		};
		let matches = media::ModelWithMetadata::find_for_user(&auth.user)
			.filter(media::Column::DeletedAt.is_null())
			.filter(condition)
			.into_model::<media::ModelWithMetadata>()
			.all(core.conn.as_ref())
			.await?;
		for row in matches {
			if seen_media_ids.insert(row.media.id.clone()) {
				external_library_models.push(row);
			}
		}
	}
	let library_matches = external_library_models
		.iter()
		.map(|row| LibraryMatch {
			media_id: &row.media.id,
			title: row
				.metadata
				.as_ref()
				.and_then(|metadata| metadata.title.as_deref())
				.filter(|title| !title.trim().is_empty())
				.unwrap_or(&row.media.name),
			authors: row
				.metadata
				.as_ref()
				.and_then(|metadata| metadata.writers.as_deref()),
			isbn: row
				.metadata
				.as_ref()
				.and_then(|metadata| metadata.identifier_isbn.as_deref()),
		})
		.collect::<Vec<_>>();

	let mut providers = external
		.iter()
		.map(|item| item.provider.clone())
		.collect::<Vec<_>>();
	providers.sort_unstable();
	providers.dedup();
	let remote_ids = external
		.iter()
		.map(|item| item.external_id.clone())
		.collect::<Vec<_>>();
	let existing_requests = if remote_ids.is_empty() {
		HashMap::new()
	} else {
		let mut requests = book_request::Entity::find()
			.filter(book_request::Column::SourceProvider.is_in(providers))
			.filter(book_request::Column::RemoteId.is_in(remote_ids))
			.order_by_desc(book_request::Column::CreatedAt);
		if !super::book_request::can_manage(auth) {
			requests =
				requests.filter(book_request::Column::RequesterId.eq(&auth.user.id));
		}
		let rows = requests.all(core.conn.as_ref()).await?;
		let mut by_identity = HashMap::with_capacity(rows.len());
		for mut request in rows {
			if let Some((provider, remote_id)) =
				request.source_provider.take().zip(request.remote_id.take())
			{
				by_identity.entry((provider, remote_id)).or_insert(request);
			}
		}
		by_identity
	};

	Ok(external
		.into_iter()
		.take(limit)
		.filter_map(|item| {
			let ExternalMediaMetadata {
				provider,
				external_id: remote_id,
				title: Some(title),
				writers: remote_authors,
				year,
				series_name,
				number,
				isbn_13,
				isbn,
				cover_url,
				has_ebook,
				has_audiobook,
				audio_seconds,
				runtime_minutes,
				narrators,
				..
			} = item
			else {
				return None;
			};
			if title.trim().is_empty() || remote_id.trim().is_empty() {
				return None;
			}
			let authors = remote_authors
				.as_ref()
				.filter(|authors| !authors.is_empty())
				.map(|authors| authors.join(", "));
			let matches = matching_library_media_ids(
				&library_matches,
				isbn.as_deref(),
				isbn_13.as_deref(),
				&title,
				remote_authors.as_deref().unwrap_or_default(),
			);
			let request = existing_requests.get(&(provider.clone(), remote_id.clone()));
			Some(UnifiedExternalHit {
				asin: (provider == AUDIBLE_SCOPE).then(|| remote_id.clone()),
				provider,
				remote_id,
				title,
				authors,
				year,
				series_name,
				series_position: number.map(|number| number.to_string()),
				isbn13: isbn_13,
				isbn10: isbn,
				cover_url,
				has_ebook,
				has_audiobook,
				audio_seconds: audio_seconds.or_else(|| {
					runtime_minutes.map(|minutes| minutes.saturating_mul(60))
				}),
				narrators: narrators.filter(|narrators| !narrators.is_empty()),
				in_library_media_ids: matches
					.into_iter()
					.map(|id| ID::from(id.to_owned()))
					.collect(),
				existing_request_id: request.map(|request| ID::from(request.id.clone())),
				existing_request_status: request.map(|request| request.status.clone()),
			})
		})
		.collect())
}

enum HardcoverSearchSource {
	Server(metadata_provider_config::Model),
	Personal(hardcover_connection::Model),
}

fn select_hardcover_source(
	server_config: Option<metadata_provider_config::Model>,
	personal_connection: Option<hardcover_connection::Model>,
) -> Option<HardcoverSearchSource> {
	server_config
		.map(HardcoverSearchSource::Server)
		.or_else(|| personal_connection.map(HardcoverSearchSource::Personal))
}

/// Resolve the Hardcover client the caller may search with, plus the
/// credential scope its results are cached under. Clients are reused across
/// requests until the stored (encrypted) token changes.
pub(super) async fn get_hardcover_provider(
	core: &CoreContext,
	user_id: &str,
) -> std::result::Result<Option<(String, SearchProvider)>, String> {
	let conn = core.conn.as_ref();
	let server_config = metadata_provider_config::Entity::find()
		.filter(
			metadata_provider_config::Column::ProviderType
				.eq(MetadataProviderKind::Hardcover),
		)
		.filter(metadata_provider_config::Column::Enabled.eq(true))
		.order_by_asc(metadata_provider_config::Column::Id)
		.one(conn)
		.await
		.map_err(|error| error.to_string())?;
	let personal_connection = if server_config.is_none() {
		hardcover_connection::Entity::find_by_id(user_id.to_owned())
			.one(conn)
			.await
			.map_err(|error| error.to_string())?
	} else {
		None
	};

	let Some(source) = select_hardcover_source(server_config, personal_connection) else {
		return Ok(None);
	};
	let (scope, fingerprint) = match &source {
		HardcoverSearchSource::Server(config) => (
			format!("server:{}", config.id),
			config.encrypted_api_token.clone().unwrap_or_default(),
		),
		HardcoverSearchSource::Personal(connection) => (
			format!("user:{}", connection.user_id),
			connection.encrypted_api_token.clone(),
		),
	};
	if let Some(provider) = EXTERNAL_SEARCH.provider(&scope, &fingerprint) {
		return Ok(Some((scope, provider)));
	}

	let encryption_key = core
		.get_encryption_key()
		.await
		.map_err(|error| error.to_string())?;
	let provider: SearchProvider = match source {
		HardcoverSearchSource::Server(config) => ProviderClientCache::new(encryption_key)
			.get_or_create(&config)
			.await
			.map_err(|error| error.to_string())?,
		HardcoverSearchSource::Personal(connection) => {
			let token = decrypt_string(&connection.encrypted_api_token, &encryption_key)
				.map_err(|error| error.to_string())?;
			Arc::new(HardcoverClient::new(token, None))
		},
	};
	EXTERNAL_SEARCH.store_provider(scope.clone(), fingerprint, &provider);
	Ok(Some((scope, provider)))
}

fn query_isbn(query: &str) -> Option<String> {
	let isbn = normalize_isbn(query);
	matches!(isbn.len(), 10 | 13).then_some(isbn)
}

/// The `LIKE` escape character for library searches. Not a backslash: SQLite
/// and PostgreSQL both take an explicit `ESCAPE`, but a backslash literal is
/// rendered doubled for PostgreSQL and would no longer be one character.
const LIKE_ESCAPE: char = '!';

/// A `%…%` pattern matching text that contains `query` literally: the
/// wildcards `%`, `_` and [`LIKE_ESCAPE`] itself are escaped, and the whole
/// pattern is lowercased to meet the lowercased column of [`folded_like`].
fn contains_pattern(query: &str) -> String {
	let mut pattern = String::with_capacity(query.len() + 2);
	pattern.push('%');
	for character in query.to_lowercase().chars() {
		if character == '%' || character == '_' || character == LIKE_ESCAPE {
			pattern.push(LIKE_ESCAPE);
		}
		pattern.push(character);
	}
	pattern.push('%');
	pattern
}

/// `LOWER(column) LIKE pattern ESCAPE '!'`: case-insensitive on PostgreSQL,
/// whose `LIKE` is case-sensitive, as well as on SQLite, and with the same
/// escape character on both (SQLite has none by default, so a backslash in
/// the pattern would have been searched for literally).
fn folded_like<C>(column: C, pattern: &str) -> SimpleExpr
where
	C: IntoColumnRef,
{
	Expr::expr(Func::lower(Expr::col(column)))
		.like(LikeExpr::new(pattern).escape(LIKE_ESCAPE))
}

fn external_library_match_condition(
	items: &[ExternalMediaMetadata],
) -> Option<Condition> {
	let mut all_candidates = Condition::any();
	let mut has_candidate = false;

	for item in items {
		let mut item_condition = Condition::any();
		let mut has_item_condition = false;
		let mut isbn_values = Vec::new();
		let mut isbn_suffixes = Vec::new();
		for isbn in [item.isbn.as_deref(), item.isbn_13.as_deref()]
			.into_iter()
			.flatten()
		{
			let normalized = normalize_isbn(isbn);
			if !normalized.is_empty() {
				isbn_values.push(isbn.to_owned());
				isbn_values.push(normalized.clone());
				if normalized.len() >= 6 {
					isbn_suffixes
						.push(normalized[normalized.len() - 6..].to_ascii_lowercase());
				}
			}
		}
		if !isbn_values.is_empty() {
			let mut isbn_condition = Condition::any()
				.add(media_metadata::Column::IdentifierIsbn.is_in(isbn_values));
			for suffix in isbn_suffixes {
				isbn_condition = isbn_condition.add(
					Expr::expr(Func::lower(Expr::col((
						media_metadata::Entity,
						media_metadata::Column::IdentifierIsbn,
					))))
					.like(format!("%{suffix}%")),
				);
			}
			item_condition = item_condition.add(isbn_condition);
			has_item_condition = true;
		}

		let title_words = item
			.title
			.as_deref()
			.map(normalize_words)
			.unwrap_or_default()
			.split_whitespace()
			.map(str::to_owned)
			.collect::<Vec<_>>();
		let mut author_condition = Condition::any();
		let mut has_author = false;
		for author in item.writers.as_deref().unwrap_or_default() {
			let author_words = normalize_words(author)
				.split_whitespace()
				.map(str::to_owned)
				.collect::<Vec<_>>();
			if author_words.is_empty() {
				continue;
			}
			let mut words_match = Condition::all();
			for word in author_words {
				words_match = words_match.add(
					Expr::expr(Func::lower(Expr::col((
						media_metadata::Entity,
						media_metadata::Column::Writers,
					))))
					.like(format!("%{word}%")),
				);
			}
			author_condition = author_condition.add(words_match);
			has_author = true;
		}
		if !title_words.is_empty() && has_author {
			let mut filename_match = Condition::all();
			let mut metadata_title_match = Condition::all();
			for word in title_words {
				let pattern = format!("%{word}%");
				filename_match = filename_match.add(
					Expr::expr(Func::lower(Expr::col((
						media::Entity,
						media::Column::Name,
					))))
					.like(&pattern),
				);
				metadata_title_match = metadata_title_match.add(
					Expr::expr(Func::lower(Expr::col((
						media_metadata::Entity,
						media_metadata::Column::Title,
					))))
					.like(&pattern),
				);
			}
			let title_condition = Condition::any()
				.add(filename_match)
				.add(metadata_title_match);
			item_condition = item_condition
				.add(Condition::all().add(title_condition).add(author_condition));
			has_item_condition = true;
		}

		if has_item_condition {
			all_candidates = all_candidates.add(item_condition);
			has_candidate = true;
		}
	}

	has_candidate.then_some(all_candidates)
}
fn normalize_isbn(value: &str) -> String {
	value
		.chars()
		.filter(|character| character.is_ascii_alphanumeric())
		.map(|character| character.to_ascii_uppercase())
		.collect()
}

pub(super) fn normalize_words(value: &str) -> String {
	let mut normalized = String::with_capacity(value.len());
	let mut separated = true;
	for character in value.chars() {
		if character.is_alphanumeric() {
			if separated && !normalized.is_empty() {
				normalized.push(' ');
			}
			for lower in character.to_lowercase() {
				normalized.push(lower);
			}
			separated = false;
		} else {
			separated = true;
		}
	}
	normalized.trim().to_owned()
}

fn matching_library_media_ids<'a>(
	library: &[LibraryMatch<'a>],
	isbn10: Option<&str>,
	isbn13: Option<&str>,
	title: &str,
	authors: &[String],
) -> Vec<&'a str> {
	let remote_isbns = [isbn10, isbn13]
		.into_iter()
		.flatten()
		.map(normalize_isbn)
		.filter(|isbn| !isbn.is_empty())
		.collect::<Vec<_>>();
	let isbn_matches = library
		.iter()
		.filter(|local| {
			local.isbn.is_some_and(|isbn| {
				let isbn = normalize_isbn(isbn);
				remote_isbns.iter().any(|remote| remote == &isbn)
			})
		})
		.map(|local| local.media_id)
		.collect::<Vec<_>>();
	if !isbn_matches.is_empty() {
		return isbn_matches;
	}

	let remote_title = normalize_words(title);
	if remote_title.is_empty() {
		return Vec::new();
	}
	let remote_authors = authors
		.iter()
		.map(|author| normalize_words(author))
		.filter(|author| !author.is_empty())
		.collect::<Vec<_>>();
	if remote_authors.is_empty() {
		return Vec::new();
	}
	library
		.iter()
		.filter(|local| {
			if normalize_words(local.title) != remote_title {
				return false;
			}
			let Some(local_authors) = local.authors else {
				return false;
			};
			local_authors.split(',').any(|local_author| {
				let local_author = normalize_words(local_author);
				!local_author.is_empty()
					&& remote_authors
						.iter()
						.any(|remote_author| remote_author == &local_author)
			})
		})
		.map(|local| local.media_id)
		.collect()
}

#[cfg(test)]
pub(super) mod tests {
	use super::*;

	#[test]
	fn isbn_matches_take_precedence_over_title_and_author_matches() {
		let library = [
			LibraryMatch {
				media_id: "isbn-match",
				title: "Different edition title",
				authors: Some("Different Author"),
				isbn: Some("978-0-306-40615-7"),
			},
			LibraryMatch {
				media_id: "title-match",
				title: "The Book!",
				authors: Some("A. Writer"),
				isbn: None,
			},
		];
		let matches = matching_library_media_ids(
			&library,
			Some("0306406152"),
			Some("9780306406157"),
			"The Book",
			&["A Writer".to_owned()],
		);

		assert_eq!(matches, ["isbn-match"]);
	}

	#[test]
	fn title_author_fallback_normalizes_punctuation_and_requires_an_author() {
		let library = [
			LibraryMatch {
				media_id: "same-work",
				title: "The Book!",
				authors: Some("A. Writer"),
				isbn: None,
			},
			LibraryMatch {
				media_id: "different-author",
				title: "The Book",
				authors: Some("Other Writer"),
				isbn: None,
			},
		];
		let matches = matching_library_media_ids(
			&library,
			None,
			None,
			"The Book",
			&["A Writer".to_owned()],
		);

		assert_eq!(matches, ["same-work"]);
		assert!(
			matching_library_media_ids(&library, None, None, "The Book", &[]).is_empty()
		);
	}

	#[test]
	fn isbn_searches_are_normalized_for_provider_scoring() {
		assert_eq!(
			query_isbn("978-0-306-40615-7"),
			Some("9780306406157".to_owned())
		);
		assert_eq!(query_isbn("The Book"), None);
	}
	#[test]
	fn server_config_precedes_personal_credentials_and_personal_is_fallback() {
		use chrono::Utc;

		let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();
		let server = metadata_provider_config::Model {
			id: 1,
			provider_type: MetadataProviderKind::Hardcover,
			enabled: true,
			encrypted_api_token: Some("server-cipher".to_owned()),
			api_token_expires_at: None,
			auto_apply_config: None,
			created_at: now.clone(),
			updated_at: None,
		};
		let personal = hardcover_connection::Model {
			user_id: "requesting-user".to_owned(),
			encrypted_api_token: "personal-cipher".to_owned(),
			credential_version: 1,
			remote_user_id: None,
			remote_username: None,
			scopes: None,
			capabilities: None,
			use_for_metadata: true,
			import_journals: false,
			sync_progress: false,
			connected_at: now.clone(),
			verified_at: None,
			last_sync_at: None,
			last_error: None,
			updated_at: now,
		};

		match select_hardcover_source(Some(server), Some(personal.clone())) {
			Some(HardcoverSearchSource::Server(config)) => {
				assert_eq!(config.encrypted_api_token.as_deref(), Some("server-cipher"));
			},
			_ => panic!("the enabled server provider must take precedence"),
		}
		match select_hardcover_source(None, Some(personal)) {
			Some(HardcoverSearchSource::Personal(connection)) => {
				assert_eq!(connection.user_id, "requesting-user");
				assert_eq!(connection.encrypted_api_token, "personal-cipher");
			},
			_ => panic!("the caller's personal connection is the fallback"),
		}
	}

	/// A library, one series, and one EPUB with metadata, plus the tables
	/// the search resolvers read that `test_database` does not create.
	async fn seed_project_hail_mary() -> sea_orm::DatabaseConnection {
		use sea_orm::{
			ActiveModelTrait, ConnectionTrait, DatabaseBackend, Schema as SeaSchema,
		};

		let db = ::tests::db::test_database().await;
		let schema = SeaSchema::new(DatabaseBackend::Sqlite);
		for statement in [
			schema.create_table_from_entity(metadata_provider_config::Entity),
			schema.create_table_from_entity(hardcover_connection::Entity),
			schema.create_table_from_entity(book_request::Entity),
		] {
			db.execute(db.get_database_backend().build(&statement))
				.await
				.unwrap();
		}

		let library = ::tests::fake_data::Library::default().insert(&db).await;
		let series = ::tests::fake_data::Series {
			name: Some("Project Hail Mary".to_owned()),
			library_id: Some(library.id),
			..Default::default()
		}
		.insert(&db)
		.await;
		let media = ::tests::fake_data::Media {
			id: Some("project-hail-mary-media".to_owned()),
			name: Some("Project Hail Mary.epub".to_owned()),
			series_id: series.id,
			..Default::default()
		}
		.insert(&db)
		.await;
		media_metadata::ActiveModel {
			media_id: sea_orm::Set(Some(media.id.clone())),
			title: sea_orm::Set(Some("Project Hail Mary".to_owned())),
			writers: sea_orm::Set(Some("Andy Weir".to_owned())),
			identifier_isbn: sea_orm::Set(Some("9780593135204".to_owned())),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();
		db
	}

	fn search_schema(
		db: sea_orm::DatabaseConnection,
		user: models::entity::user::AuthUser,
	) -> async_graphql::Schema<
		UnifiedSearchQuery,
		async_graphql::EmptyMutation,
		async_graphql::EmptySubscription,
	> {
		use async_graphql::{EmptyMutation, EmptySubscription, Schema};

		Schema::build(UnifiedSearchQuery, EmptyMutation, EmptySubscription)
			.data(AuthContext {
				user,
				api_key: None,
				device_id: None,
			})
			.data(std::sync::Arc::new(stump_core::Ctx::for_testing(db)))
			.data(stump_api_types::RequestOrigin::default())
			.finish()
	}

	#[tokio::test]
	async fn library_search_qualifies_matching_series_and_media_names() {
		let db = seed_project_hail_mary().await;
		let user = crate::tests::common::get_default_user();
		let external_candidate = ExternalMediaMetadata {
			title: Some("Project Hail Mary".to_owned()),
			writers: Some(vec!["Andy Weir".to_owned()]),
			..Default::default()
		};
		let condition = external_library_match_condition(&[external_candidate])
			.expect("title-author query should produce a local match condition");
		let matched = media::ModelWithMetadata::find_for_user(&user)
			.filter(media::Column::DeletedAt.is_null())
			.filter(condition)
			.into_model::<media::ModelWithMetadata>()
			.all(&db)
			.await
			.unwrap();
		assert_eq!(
			matched
				.iter()
				.map(|row| row.media.id.as_str())
				.collect::<Vec<_>>(),
			["project-hail-mary-media"]
		);

		let schema = search_schema(db, user);
		let response = schema
			.execute(
				r#"{
					librarySearch(query: "Project Hail Mary", limit: 5) {
						mediaId
						title
						authors
						seriesName
						extension
						isAudiobook
						thumbnailUrl
					}
					missing: librarySearch(query: "Artemis") { mediaId }
				}"#,
			)
			.await;

		assert!(response.errors.is_empty(), "{:?}", response.errors);
		let data = response.data.into_json().unwrap();
		let result = &data["librarySearch"];
		assert_eq!(result.as_array().unwrap().len(), 1);
		assert_eq!(result[0]["mediaId"], "project-hail-mary-media");
		assert_eq!(result[0]["title"], "Project Hail Mary");
		assert_eq!(result[0]["authors"], "Andy Weir");
		assert_eq!(result[0]["seriesName"], "Project Hail Mary");
		assert_eq!(result[0]["extension"], "epub");
		assert_eq!(result[0]["isAudiobook"], false);
		assert!(data["missing"].as_array().unwrap().is_empty());
	}

	/// `%`, `_` and the escape character in a query are searched for
	/// literally on the one `ESCAPE` both databases honour, and both sides
	/// are lowercased so PostgreSQL's case-sensitive `LIKE` folds like
	/// SQLite's. Before, the pattern escaped with a backslash and no
	/// `ESCAPE`, which SQLite matched literally: `100%` found nothing.
	#[tokio::test]
	async fn library_search_folds_case_and_matches_wildcards_literally() {
		use sea_orm::ActiveModelTrait;

		let db = seed_project_hail_mary().await;
		let library = ::tests::fake_data::Library::default().insert(&db).await;
		let series = ::tests::fake_data::Series {
			name: Some("Odds".to_owned()),
			library_id: Some(library.id),
			..Default::default()
		}
		.insert(&db)
		.await;
		let media = ::tests::fake_data::Media {
			id: Some("percent-media".to_owned()),
			name: Some("100% Sure_Thing!.epub".to_owned()),
			series_id: series.id,
			..Default::default()
		}
		.insert(&db)
		.await;
		media_metadata::ActiveModel {
			media_id: sea_orm::Set(Some(media.id.clone())),
			title: sea_orm::Set(Some("100% Sure_Thing!".to_owned())),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();

		let schema = search_schema(db, crate::tests::common::get_default_user());
		let response = schema
			.execute(
				r#"{
					percent: librarySearch(query: "100%") { mediaId }
					underscore: librarySearch(query: "_THING") { mediaId }
					bang: librarySearch(query: "thing!") { mediaId }
					notWildcard: librarySearch(query: "100_") { mediaId }
					lower: librarySearch(query: "project HAIL") { mediaId }
				}"#,
			)
			.await;

		assert!(response.errors.is_empty(), "{:?}", response.errors);
		let data = response.data.into_json().unwrap();
		let ids = |field: &str| {
			data[field]
				.as_array()
				.unwrap()
				.iter()
				.map(|hit| hit["mediaId"].as_str().unwrap().to_owned())
				.collect::<Vec<_>>()
		};
		assert_eq!(ids("percent"), ["percent-media"], "a literal percent sign");
		assert_eq!(ids("underscore"), ["percent-media"], "a literal underscore");
		assert_eq!(
			ids("bang"),
			["percent-media"],
			"the escape character itself"
		);
		assert!(
			ids("notWildcard").is_empty(),
			"an underscore never stands for any character"
		);
		assert_eq!(ids("lower"), ["project-hail-mary-media"]);

		assert_eq!(
			contains_pattern("100% Sure_Thing!"),
			"%100!% sure!_thing!!%"
		);
		let postgres = sea_orm::sea_query::Query::select()
			.column(media::Column::Id)
			.from(media::Entity)
			.and_where(folded_like(
				(media::Entity, media::Column::Name),
				&contains_pattern("A%"),
			))
			.to_string(sea_orm::sea_query::PostgresQueryBuilder);
		assert!(
			postgres.ends_with(r#"WHERE LOWER("media"."name") LIKE '%a!%%' ESCAPE '!'"#),
			"{postgres}"
		);
	}

	#[tokio::test]
	async fn external_book_search_without_a_credential_reports_no_provider() {
		let db = seed_project_hail_mary().await;
		let schema = search_schema(db, crate::tests::common::get_default_user());
		let response = schema
			.execute(
				r#"{
					externalBookSearch(query: "Project Hail Mary", limit: 5) {
						provider
						error
						hits { title }
					}
					blank: externalBookSearch(query: "   ") { provider hits { title } }
				}"#,
			)
			.await;

		assert!(response.errors.is_empty(), "{:?}", response.errors);
		let data = response.data.into_json().unwrap();
		let result = &data["externalBookSearch"];
		assert!(result["provider"].is_null());
		assert!(result["error"].is_null());
		assert!(result["hits"].as_array().unwrap().is_empty());
		assert!(data["blank"]["provider"].is_null());
		assert!(data["blank"]["hits"].as_array().unwrap().is_empty());
	}

	/// The per-request annotation layer is what must never be cached: library
	/// matches follow `find_for_user` and existing requests are only visible
	/// to their requester unless the caller manages requests.
	#[tokio::test]
	async fn external_hits_are_annotated_per_caller() {
		use chrono::Utc;
		use sea_orm::{ActiveModelTrait, Set};

		let db = seed_project_hail_mary().await;
		let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();
		let requester = ::tests::fake_data::User::new("someone-else")
			.insert(&db)
			.await;
		book_request::ActiveModel {
			id: Set("req-1".to_owned()),
			requester_id: Set(requester.id),
			source_provider: Set(Some("hardcover".to_owned())),
			remote_id: Set(Some("52709".to_owned())),
			format: Set("EPUB".to_owned()),
			title: Set("Project Hail Mary".to_owned()),
			status: Set("PENDING".to_owned()),
			approval_policy: Set("MANUAL".to_owned()),
			created_at: Set(now.clone()),
			updated_at: Set(now),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();
		let core: CoreContext = std::sync::Arc::new(stump_core::Ctx::for_testing(db));

		let hit = || ExternalMediaMetadata {
			provider: "hardcover".to_owned(),
			external_id: "52709".to_owned(),
			title: Some("Project Hail Mary".to_owned()),
			writers: Some(vec!["Andy Weir".to_owned()]),
			isbn_13: Some("9780593135204".to_owned()),
			..Default::default()
		};
		let owner = AuthContext {
			user: crate::tests::common::get_default_user(),
			api_key: None,
			device_id: None,
		};
		let hits = annotate_external_hits(&core, &owner, vec![hit()], 10)
			.await
			.unwrap();
		assert_eq!(hits.len(), 1);
		assert_eq!(
			hits[0].in_library_media_ids,
			[ID::from("project-hail-mary-media")]
		);
		assert_eq!(hits[0].existing_request_id, Some(ID::from("req-1")));
		assert_eq!(hits[0].existing_request_status.as_deref(), Some("PENDING"));

		let member = AuthContext {
			user: models::entity::user::AuthUser {
				is_server_owner: false,
				..crate::tests::common::get_default_user()
			},
			api_key: None,
			device_id: None,
		};
		let hits = annotate_external_hits(&core, &member, vec![hit()], 10)
			.await
			.unwrap();
		assert_eq!(hits.len(), 1);
		assert!(
			hits[0].existing_request_id.is_none(),
			"another user's request is invisible to a non-manager"
		);

		let untitled = ExternalMediaMetadata {
			title: None,
			..hit()
		};
		assert!(annotate_external_hits(&core, &owner, vec![untitled], 10)
			.await
			.unwrap()
			.is_empty());
	}

	#[test]
	fn audible_relevance_keeps_the_work_and_drops_catalog_padding() {
		let hit = |title: &str, narrators: &[&str], minutes: Option<i32>| {
			ExternalMediaMetadata {
				title: Some(title.to_owned()),
				writers: Some(vec!["Andy Weir".to_owned()]),
				narrators: Some(narrators.iter().map(|name| name.to_string()).collect()),
				runtime_minutes: minutes,
				..Default::default()
			}
		};
		let narrated = |title: &str| hit(title, &["Ray Porter"], Some(941));
		assert!(audible_hit_is_relevant(
			"project hail mary",
			&narrated("Project Hail Mary")
		));
		assert!(audible_hit_is_relevant(
			"Project Hail Mary",
			&narrated("Project Hail Mary: A Novel")
		));
		assert!(audible_hit_is_relevant(
			"hail mary",
			&narrated("Project Hail Mary")
		));
		assert!(
			!audible_hit_is_relevant("hail mary weir", &narrated("Project Hail Mary")),
			"authors no longer count toward the query words"
		);
		assert!(!audible_hit_is_relevant(
			"project hail mary",
			&narrated("Pride and Prejudice")
		));
		assert!(!audible_hit_is_relevant(
			"Project Hail Mary",
			&narrated("Project Hail Maryland")
		));
		// A prefix is accepted for the subtitle case above, so a sequel that
		// extends the title rides along; the catalog's own ranking orders it.
		assert!(audible_hit_is_relevant("Dune", &narrated("Dune Messiah")));
		assert!(!audible_hit_is_relevant(
			"Dune",
			&ExternalMediaMetadata::default()
		));
		assert!(!audible_hit_is_relevant("  ", &narrated("Dune")));

		// Live noise for "project hail mary": a talk-radio hour naming the
		// book (no narrator, 40 min) and an unnarrated placeholder listing.
		assert!(!audible_hit_is_relevant(
			"project hail mary",
			&hit(
				"3/25 Wednesday Hr 1: Harry Potter T… Project Hail Mary",
				&[],
				Some(40)
			)
		));
		assert!(!audible_hit_is_relevant(
			"project hail mary",
			&hit("Project Hail Mary (2026)", &[], None)
		));
		assert!(
			audible_hit_is_relevant(
				"project hail mary",
				&hit("Project Hail Mary", &[], Some(60))
			),
			"an hour-long unnarrated listing is still a book"
		);
	}

	#[tokio::test]
	async fn audible_search_maps_products_filters_noise_and_serves_repeats_from_cache() {
		use metadata_integrations::{
			mock_http::{render_ok, MockServer},
			AudibleClient,
		};

		let body = serde_json::json!({ "products": [
			{
				"asin": "B08G9PRS1K",
				"title": "Project Hail Mary",
				"authors": [{ "name": "Andy Weir" }],
				"narrators": [{ "name": "Ray Porter" }],
				"runtime_length_min": 941,
				"release_date": "2021-05-04",
				"language": "english"
			},
			{
				"asin": "B0CTZVXP8H",
				"title": "Project Hail Mary (Italian edition)",
				"authors": [{ "name": "Andy Weir" }],
				"narrators": [{ "name": "William Angiuli" }],
				"runtime_length_min": 980,
				"language": "italian"
			},
			{
				"asin": "B0NOISE001",
				"title": "Pride and Prejudice",
				"authors": [{ "name": "Jane Austen" }],
				"narrators": [{ "name": "Rosamund Pike" }],
				"language": "english"
			}
		] })
		.to_string();
		let server = MockServer::spawn(vec![render_ok(&body)]);
		let audible = AudibleClient::new().pointed_at(&server.url);
		let cache = BriefSearchCache::new(EXTERNAL_SEARCH_TTL, EXTERNAL_SEARCH_CAPACITY);

		let (error, external) =
			audible_search(&audible, &cache, "project hail mary", 5, "en").await;
		assert_eq!(error, None);
		assert_eq!(
			external.len(),
			1,
			"the padding product and the Italian translation are dropped"
		);
		let (_, italian) =
			audible_search(&audible, &cache, "project hail mary", 5, "it").await;
		assert_eq!(italian.len(), 1);
		assert_eq!(
			italian[0].external_id, "B0CTZVXP8H",
			"asked for Italian, got the translation"
		);
		let requests = server.requests();
		assert_eq!(requests.len(), 1);
		assert!(requests[0].contains("num_results=10"), "{}", requests[0]);

		let db = seed_project_hail_mary().await;
		let core: CoreContext = std::sync::Arc::new(stump_core::Ctx::for_testing(db));
		let auth = AuthContext {
			user: crate::tests::common::get_default_user(),
			api_key: None,
			device_id: None,
		};
		let hits = annotate_external_hits(&core, &auth, external, 5)
			.await
			.unwrap();
		assert_eq!(hits.len(), 1);
		let hit = &hits[0];
		assert_eq!(hit.provider, "audible");
		assert_eq!(hit.remote_id, "B08G9PRS1K");
		assert_eq!(hit.asin.as_deref(), Some("B08G9PRS1K"));
		assert_eq!(hit.authors.as_deref(), Some("Andy Weir"));
		assert_eq!(hit.year, Some(2021));
		assert_eq!(hit.has_audiobook, Some(true));
		assert_eq!(hit.has_ebook, None);
		assert_eq!(hit.audio_seconds, Some(941 * 60));
		assert_eq!(
			hit.narrators.as_deref(),
			Some(["Ray Porter".to_owned()].as_slice())
		);
		assert_eq!(
			hit.in_library_media_ids,
			[ID::from("project-hail-mary-media")],
			"title and author match the library copy"
		);

		let (error, again) =
			audible_search(&audible, &cache, "Project  Hail Mary", 5, "en").await;
		assert_eq!(error, None);
		assert_eq!(again.len(), 1);
		assert_eq!(server.requests().len(), 1, "served from cache");

		let failing =
			"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
		let server = MockServer::spawn(vec![failing.to_owned()]);
		let audible = AudibleClient::new().pointed_at(&server.url);
		let (error, external) =
			audible_search(&audible, &cache, "Artemis", 5, "en").await;
		assert!(
			error.is_some(),
			"a failed search reports instead of failing"
		);
		assert!(external.is_empty());
	}

	/// A server that accepts every connection and answers none for `hold`;
	/// what a stalled Hardcover looks like to the client.
	pub(in crate::query) fn stalled_server(hold: Duration) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
		let url = format!("http://{}", listener.local_addr().unwrap());
		std::thread::spawn(move || {
			let mut held = Vec::new();
			for connection in listener.incoming().flatten() {
				held.push(connection);
				std::thread::sleep(hold);
			}
		});
		url
	}

	/// Hardcover's client retries but never times out on its own; the search
	/// gives up at the deadline, reports it like any provider failure, and
	/// caches nothing, so the next keystroke asks a recovered Hardcover again.
	#[tokio::test]
	async fn hardcover_search_gives_up_at_its_deadline_and_asks_again_later() {
		use metadata_integrations::{
			mock_http::{render_ok, MockServer},
			HardcoverClient,
		};

		let cache = BriefSearchCache::new(EXTERNAL_SEARCH_TTL, EXTERNAL_SEARCH_CAPACITY);
		let stalled = HardcoverClient::new("token".to_owned(), Some(u32::MAX))
			.pointed_at(&stalled_server(Duration::from_secs(3)));
		let started = std::time::Instant::now();
		let (error, hits) = hardcover_search(
			&cache,
			"server:1",
			&stalled,
			"Project Hail Mary",
			5,
			Duration::from_millis(200),
		)
		.await;
		assert_eq!(error.as_deref(), Some(HARDCOVER_TIMED_OUT));
		assert!(hits.is_empty());
		assert!(
			started.elapsed() < Duration::from_secs(2),
			"gave up after {:?}, not when the server let go",
			started.elapsed()
		);
		assert_eq!(cache.len(), 0, "a timeout is never cached");

		let index = serde_json::json!({
			"data": { "search": { "results": { "hits": [
				{ "document": {
					"id": 52709,
					"title": "Project Hail Mary",
					"author_names": ["Andy Weir"],
					"release_year": 2021
				} }
			] } } }
		})
		.to_string();
		let recovered = MockServer::spawn(vec![render_ok(&index)]);
		let hardcover = HardcoverClient::new("token".to_owned(), Some(u32::MAX))
			.pointed_at(&recovered.url);
		let (error, hits) = hardcover_search(
			&cache,
			"server:1",
			&hardcover,
			"Project Hail Mary",
			5,
			Duration::from_millis(200),
		)
		.await;
		assert_eq!(error, None);
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].title.as_deref(), Some("Project Hail Mary"));
		assert_eq!(cache.len(), 1);
	}

	/// Hardcover's index flags ride through to the hit; a hit from the same
	/// work on Audible is a different request identity.
	#[tokio::test]
	async fn hit_availability_and_existing_requests_follow_the_provider() {
		use chrono::Utc;
		use sea_orm::{ActiveModelTrait, Set};

		let db = seed_project_hail_mary().await;
		let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();
		let requester = ::tests::fake_data::User::new("requester").insert(&db).await;
		book_request::ActiveModel {
			id: Set("req-audible".to_owned()),
			requester_id: Set(requester.id),
			source_provider: Set(Some("audible".to_owned())),
			remote_id: Set(Some("B08G9PRS1K".to_owned())),
			format: Set("AUDIOBOOK".to_owned()),
			title: Set("Project Hail Mary".to_owned()),
			status: Set("PENDING".to_owned()),
			approval_policy: Set("MANUAL".to_owned()),
			created_at: Set(now.clone()),
			updated_at: Set(now),
			..Default::default()
		}
		.insert(&db)
		.await
		.unwrap();
		let core: CoreContext = std::sync::Arc::new(stump_core::Ctx::for_testing(db));
		let auth = AuthContext {
			user: crate::tests::common::get_default_user(),
			api_key: None,
			device_id: None,
		};

		let hits = annotate_external_hits(
			&core,
			&auth,
			vec![
				ExternalMediaMetadata {
					provider: "hardcover".to_owned(),
					external_id: "B08G9PRS1K".to_owned(),
					title: Some("Project Hail Mary".to_owned()),
					has_ebook: Some(true),
					has_audiobook: Some(true),
					audio_seconds: Some(57000),
					..Default::default()
				},
				ExternalMediaMetadata {
					provider: "audible".to_owned(),
					external_id: "B08G9PRS1K".to_owned(),
					title: Some("Project Hail Mary".to_owned()),
					narrators: Some(Vec::new()),
					..Default::default()
				},
			],
			10,
		)
		.await
		.unwrap();

		assert_eq!(hits.len(), 2);
		assert_eq!(hits[0].has_ebook, Some(true));
		assert_eq!(hits[0].audio_seconds, Some(57000));
		assert_eq!(hits[0].asin, None);
		assert_eq!(hits[0].narrators, None);
		assert_eq!(
			hits[0].existing_request_id, None,
			"a same-looking id on another provider is not this request"
		);
		assert_eq!(hits[1].existing_request_id, Some(ID::from("req-audible")));
		assert_eq!(hits[1].asin.as_deref(), Some("B08G9PRS1K"));
		assert_eq!(hits[1].narrators, None, "an empty narrator list is unknown");
	}
}
