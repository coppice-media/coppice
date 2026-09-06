//! The Audiobookshelf wire contract.
//!
//! Every struct here reproduces a shape captured from a live `abs-ref`
//! 2.36.0 container; the capture rig and the JSON files are in
//! `../komga-compat/abs/` and each type names the capture it came from. The
//! consuming client contract is Lissen `1.11.22-release` (`f30bf9be`),
//! `app/src/main/kotlin/org/grakovne/lissen/channel/audiobookshelf/**`,
//! whose Moshi models mark exactly which fields must exist.
//!
//! Two conventions run through the file:
//!
//! - Times on the wire are **seconds as JSON numbers** (`duration`,
//!   `currentTime`, `start`, `end`, `startOffset`, `time`) while timestamps
//!   are **milliseconds since the epoch** (`addedAt`, `lastUpdate`,
//!   `createdAt`, `mtimeMs`). Stump stores audio positions in milliseconds,
//!   so the mapper converts at the boundary and nowhere else.
//! - `Option<Option<T>>` marks a key abs-ref sometimes omits entirely and
//!   sometimes sends as `null`: the outer `None` omits the key, the inner
//!   `None` writes `null` (`user.refreshToken` is present-but-null without
//!   `x-return-tokens`, and absent from `GET /api/me`).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Server identity — capture/status.json, ping.json
// ---------------------------------------------------------------------------

/// `GET /status`. Lissen reads this before login to pick an auth method
/// (`AudiobookshelfAuthService.fetchAuthMethods`,
/// `common/api/AudiobookshelfAuthService.kt:107-146`) and only needs
/// `authMethods` and `authFormData`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
	pub app: String,
	pub server_version: String,
	pub is_init: bool,
	pub language: String,
	pub auth_methods: Vec<String>,
	pub auth_form_data: AuthFormDataDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthFormDataDto {
	pub auth_login_custom_message: String,
}

/// `GET /ping` → `{"success": true}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PingDto {
	pub success: bool,
}

// ---------------------------------------------------------------------------
// Identity — capture/login.json, refresh.json, authorize.json, me.json
// ---------------------------------------------------------------------------

/// `POST /login`, `POST /auth/refresh` and `POST /api/authorize` all answer
/// with this envelope (byte-identical key sets in
/// `capture/{login,refresh,authorize}.json`). Lissen decodes it twice: as
/// `LoggedUserResponse{user{id,token,refreshToken,accessToken,username},
/// userDefaultLibraryId}` and as `ConnectionInfoResponse{user{username},
/// serverSettings{version,buildNumber}}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponseDto {
	pub user: UserDto,
	pub user_default_library_id: Option<String>,
	pub server_settings: ServerSettingsDto,
	/// E-reader devices for "send to device"; Stump's Kindle registry is not
	/// exposed through this profile, so the list is always empty.
	pub ereader_devices: Vec<serde_json::Value>,
	/// Capitalised on the wire, exactly as abs-ref emits it.
	#[serde(rename = "Source")]
	pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
	pub id: String,
	pub username: String,
	pub email: Option<String>,
	/// `root`, `admin`, `user` or `guest`.
	#[serde(rename = "type")]
	pub user_type: String,
	/// The never-expiring legacy token; always present.
	pub token: String,
	pub media_progress: Vec<MediaProgressDto>,
	pub series_hide_from_continue_listening: Vec<String>,
	pub bookmarks: Vec<AudioBookmarkDto>,
	pub is_active: bool,
	pub is_locked: bool,
	pub last_seen: Option<i64>,
	pub created_at: i64,
	pub permissions: UserPermissionsDto,
	/// Empty means "all libraries", matching `accessAllLibraries`.
	pub libraries_accessible: Vec<String>,
	pub item_tags_selected: Vec<String>,
	#[serde(rename = "hasOpenIDLink")]
	pub has_openid_link: bool,
	/// `true` only on `POST /login`, where abs-ref flags that `token` is the
	/// old never-expiring credential (`capture/login.json` has the key,
	/// `refresh.json` and `authorize.json` do not).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub is_old_token: Option<bool>,
	/// Present on `POST /login` and `POST /auth/refresh` only:
	/// `capture/authorize.json` and `capture/me.json` both carry `token`
	/// alone.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub access_token: Option<String>,
	/// Same presence as `accessToken`, and `null` inside it when the client
	/// did not send `x-return-tokens: true`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub refresh_token: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserPermissionsDto {
	pub download: bool,
	pub update: bool,
	pub delete: bool,
	pub upload: bool,
	pub create_ereader: bool,
	pub access_all_libraries: bool,
	pub access_all_tags: bool,
	pub access_explicit_content: bool,
	pub selected_tags_not_accessible: bool,
}

/// `serverSettings`, verbatim key set from `capture/authorize.json`. Lissen
/// reads `version` and `buildNumber`; the rest exists so any ABS client
/// deserialises the envelope. Values Stump has no analogue for are the
/// abs-ref defaults and are constants — see `SERVER_SETTINGS` in
/// [`crate::mapper`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettingsDto {
	pub id: String,
	pub scanner_find_covers: bool,
	pub scanner_cover_provider: String,
	pub scanner_parse_subtitle: bool,
	pub scanner_prefer_matched_metadata: bool,
	pub scanner_disable_watcher: bool,
	pub store_cover_with_item: bool,
	pub store_metadata_with_item: bool,
	pub metadata_file_format: String,
	pub rate_limit_login_requests: i64,
	pub rate_limit_login_window: i64,
	pub allow_iframe: bool,
	pub backup_path: String,
	pub backup_schedule: bool,
	pub backups_to_keep: i64,
	pub max_backup_size: i64,
	pub logger_daily_logs_to_keep: i64,
	pub logger_scanner_logs_to_keep: i64,
	pub home_bookshelf_view: i64,
	pub bookshelf_view: i64,
	pub podcast_episode_schedule: String,
	pub sorting_ignore_prefix: bool,
	pub sorting_prefixes: Vec<String>,
	pub chromecast_enabled: bool,
	pub date_format: String,
	pub time_format: String,
	pub language: String,
	pub allowed_origins: Vec<String>,
	pub log_level: i64,
	pub version: String,
	pub build_number: i64,
	pub auth_login_custom_message: Option<String>,
	pub auth_active_auth_methods: Vec<String>,
	#[serde(rename = "authOpenIDIssuerURL")]
	pub auth_openid_issuer_url: Option<String>,
	#[serde(rename = "authOpenIDAuthorizationURL")]
	pub auth_openid_authorization_url: Option<String>,
	#[serde(rename = "authOpenIDTokenURL")]
	pub auth_openid_token_url: Option<String>,
	#[serde(rename = "authOpenIDUserInfoURL")]
	pub auth_openid_user_info_url: Option<String>,
	#[serde(rename = "authOpenIDJwksURL")]
	pub auth_openid_jwks_url: Option<String>,
	#[serde(rename = "authOpenIDLogoutURL")]
	pub auth_openid_logout_url: Option<String>,
	#[serde(rename = "authOpenIDTokenSigningAlgorithm")]
	pub auth_openid_token_signing_algorithm: String,
	#[serde(rename = "authOpenIDButtonText")]
	pub auth_openid_button_text: String,
	#[serde(rename = "authOpenIDAutoLaunch")]
	pub auth_openid_auto_launch: bool,
	#[serde(rename = "authOpenIDAutoRegister")]
	pub auth_openid_auto_register: bool,
	#[serde(rename = "authOpenIDMatchExistingBy")]
	pub auth_openid_match_existing_by: Option<String>,
	pub time_zone: String,
}

// ---------------------------------------------------------------------------
// Libraries — capture/libraries.json, library_include_filterdata.json
// ---------------------------------------------------------------------------

/// `GET /api/libraries`. Lissen sorts client-side by `displayOrder`
/// (`common/AudiobookshelfChannel.kt:83-86`) and needs
/// `{id,name,mediaType,displayOrder}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibrariesResponseDto {
	pub libraries: Vec<LibraryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDto {
	pub id: String,
	pub name: String,
	pub folders: Vec<LibraryFolderDto>,
	pub display_order: i32,
	pub icon: String,
	/// Always `book`: podcast libraries are not served by this profile.
	pub media_type: String,
	pub provider: String,
	pub settings: LibrarySettingsDto,
	pub last_scan: Option<i64>,
	pub last_scan_version: Option<String>,
	pub created_at: i64,
	pub last_update: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolderDto {
	pub id: String,
	pub full_path: String,
	pub library_id: String,
	pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySettingsDto {
	pub cover_aspect_ratio: i64,
	pub disable_watcher: bool,
	pub auto_scan_cron_expression: Option<String>,
	pub skip_matching_media_with_asin: bool,
	pub skip_matching_media_with_isbn: bool,
	pub audiobooks_only: bool,
	pub epubs_allow_scripted_content: bool,
	pub hide_single_book_series: bool,
	pub only_show_later_books_in_continue_series: bool,
	pub metadata_precedence: Vec<String>,
	pub mark_as_finished_percent_complete: Option<i64>,
	pub mark_as_finished_time_remaining: Option<i64>,
}

/// `GET /api/libraries/{id}?include=filterdata`, the shape Lissen's
/// `LibraryResponse{library, filterdata}` decodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryWithFilterDataDto {
	pub filterdata: FilterDataDto,
	pub issues: i64,
	#[serde(rename = "numUserPlaylists")]
	pub num_user_playlists: i64,
	pub library: LibraryDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FilterDataDto {
	pub authors: Vec<NamedIdDto>,
	pub genres: Vec<String>,
	pub tags: Vec<String>,
	pub series: Vec<NamedIdDto>,
	pub narrators: Vec<String>,
	pub languages: Vec<String>,
	pub publishers: Vec<String>,
	pub published_decades: Vec<String>,
	pub book_count: i64,
	pub author_count: i64,
	pub series_count: i64,
	pub podcast_count: i64,
	pub num_issues: i64,
	pub loaded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamedIdDto {
	pub id: String,
	pub name: String,
}

// ---------------------------------------------------------------------------
// Library items — capture/library_items_minified.json, item.json,
// item_expanded.json, library_items_collapseseries.json
// ---------------------------------------------------------------------------

/// `GET /api/libraries/{id}/items`. `collapseseries` is lower-case on the
/// wire, unlike every neighbouring key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItemsPageDto {
	pub results: Vec<LibraryItemDto>,
	pub total: i64,
	pub limit: i64,
	pub page: i64,
	pub sort_by: String,
	pub sort_desc: bool,
	pub media_type: String,
	pub minified: bool,
	#[serde(rename = "collapseseries")]
	pub collapse_series: bool,
	pub include: String,
	pub offset: i64,
}

/// One library item in any of its three abs-ref shapes — minified (list),
/// detail (`GET /api/items/{id}`) and expanded (`?expanded=1`, search hits,
/// `batch/get`, `playbackSession.libraryItem`). The optional fields are
/// exactly the ones that differ between them, so a single struct serialises
/// all three without inventing keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItemDto {
	pub id: String,
	/// abs-ref reports the filesystem inode of the item folder. Stump has no
	/// stable inode across rescans, so the item id is echoed here; no client
	/// resolves anything through it.
	pub ino: String,
	pub old_library_item_id: Option<String>,
	pub library_id: String,
	pub folder_id: String,
	pub path: String,
	pub rel_path: String,
	pub is_file: bool,
	pub mtime_ms: i64,
	pub ctime_ms: i64,
	pub birthtime_ms: i64,
	pub added_at: i64,
	pub updated_at: i64,
	pub is_missing: bool,
	pub is_invalid: bool,
	pub media_type: String,
	pub media: BookDto,
	/// Minified and expanded only.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub num_files: Option<i64>,
	/// Minified and expanded only.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub size: Option<i64>,
	/// Detail and expanded only.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub last_scan: Option<i64>,
	/// Detail and expanded only.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub scan_version: Option<String>,
	/// Detail and expanded only.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub library_files: Option<Vec<LibraryFileDto>>,
	/// `?expanded=1&include=progress` only.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub user_media_progress: Option<MediaProgressDto>,
	/// `collapseseries=1` only, and `null` there for items outside a series.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub collapsed_series: Option<Option<CollapsedSeriesDto>>,
}

/// The `media` object of a book item. `metadata` switches between the
/// minified and the full author/series shape, which abs-ref signals only by
/// which keys are present, so both are merged here and the mapper fills the
/// pair the requested shape needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookDto {
	pub id: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub library_item_id: Option<String>,
	pub metadata: BookMetadataDto,
	pub cover_path: Option<String>,
	pub tags: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub num_tracks: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub num_audio_files: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub num_chapters: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub duration: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub size: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub audio_files: Option<Vec<AudioFileDto>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub chapters: Option<Vec<BookChapterDto>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ebook_file: Option<Option<serde_json::Value>>,
	/// Expanded only: the audio files with playback fields attached.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tracks: Option<Vec<AudioTrackDto>>,
}

/// Book metadata. abs-ref emits `authors`/`narrators`/`series` in the full
/// shape and `authorName`/`narratorName`/`seriesName` in the minified one;
/// the expanded shape emits both. Lissen's `BookResponse` reads the full
/// shape and its `LibraryItemsResponse` the minified one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadataDto {
	pub title: Option<String>,
	pub subtitle: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub authors: Option<Vec<NamedIdDto>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub narrators: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub series: Option<Vec<SeriesSequenceDto>>,
	pub genres: Vec<String>,
	pub published_year: Option<String>,
	pub published_date: Option<String>,
	pub publisher: Option<String>,
	pub description: Option<String>,
	pub isbn: Option<String>,
	pub asin: Option<String>,
	pub language: Option<String>,
	pub explicit: bool,
	pub abridged: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title_ignore_prefix: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub author_name: Option<String>,
	#[serde(rename = "authorNameLF", skip_serializing_if = "Option::is_none")]
	pub author_name_lf: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub narrator_name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub series_name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub description_plain: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeriesSequenceDto {
	pub id: String,
	pub name: String,
	pub sequence: Option<String>,
}

/// `collapseseries=1` replaces the per-book rows of a series with one row
/// carrying this object (`capture/library_items_collapseseries.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollapsedSeriesDto {
	pub id: String,
	pub name: String,
	pub name_ignore_prefix: String,
	pub sequence: Option<String>,
	pub num_books: i64,
	pub library_item_ids: Vec<String>,
}

/// One chapter of a book. Field order is abs-ref's: `start`, `end`,
/// `title`, `id`, with `id` a 0-based integer index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookChapterDto {
	pub start: f64,
	pub end: f64,
	pub title: String,
	pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioFileDto {
	/// 1-based, in playback order.
	pub index: i64,
	/// The file id used by `GET /api/items/{id}/file/{ino}`; Stump uses the
	/// track index, which is stable across rescans where an inode is not.
	pub ino: String,
	pub metadata: FileMetadataDto,
	pub added_at: i64,
	pub updated_at: i64,
	pub track_num_from_meta: Option<i64>,
	pub disc_num_from_meta: Option<i64>,
	pub track_num_from_filename: Option<i64>,
	pub disc_num_from_filename: Option<i64>,
	pub manually_verified: bool,
	pub exclude: bool,
	pub error: Option<String>,
	pub format: Option<String>,
	pub duration: f64,
	pub bit_rate: Option<i64>,
	pub language: Option<String>,
	pub codec: Option<String>,
	pub time_base: Option<String>,
	pub channels: Option<i64>,
	pub channel_layout: Option<String>,
	pub chapters: Vec<BookChapterDto>,
	pub embedded_cover_art: Option<String>,
	pub meta_tags: MetaTagsDto,
	pub mime_type: String,
}

/// An audio file with the three playback fields abs-ref adds when it turns
/// `audioFiles[]` into `tracks[]`/`audioTracks[]`
/// (`capture/play_session.json:127-135`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackDto {
	#[serde(flatten)]
	pub file: AudioFileDto,
	pub title: String,
	/// Seconds of the book before this track starts.
	pub start_offset: f64,
	/// `/api/items/{itemId}/file/{ino}`; Lissen builds the same URL itself
	/// from the item id and the file's `ino`
	/// (`common/AudiobookshelfChannel.kt:48-64`).
	pub content_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadataDto {
	pub filename: String,
	/// With the leading period, as abs-ref emits it (`.m4b`).
	pub ext: String,
	pub path: String,
	pub rel_path: String,
	pub size: i64,
	pub mtime_ms: i64,
	pub ctime_ms: i64,
	pub birthtime_ms: i64,
}

/// The container tags abs-ref surfaces per audio file. Only tags Stump
/// actually read are emitted; abs-ref omits absent tags the same way.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct MetaTagsDto {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_album: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_artist: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_album_artist: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_track: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_genre: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_date: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_composer: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_publisher: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_subtitle: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_series: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_series_part: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_language: Option<String>,
	#[serde(rename = "tagASIN", skip_serializing_if = "Option::is_none")]
	pub tag_asin: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_isbn: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFileDto {
	pub ino: String,
	pub metadata: FileMetadataDto,
	pub is_supplementary: Option<bool>,
	pub added_at: i64,
	pub updated_at: i64,
	/// `audio`, `ebook`, `image` or `unknown`.
	pub file_type: String,
}

/// `POST /api/items/batch/get` request and response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ItemsBatchRequestDto {
	#[serde(default)]
	pub library_item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemsBatchResponseDto {
	pub library_items: Vec<LibraryItemDto>,
}

// ---------------------------------------------------------------------------
// Personalized shelves — capture/personalized.json
// ---------------------------------------------------------------------------

/// One shelf of `GET /api/libraries/{id}/personalized`. Lissen reads
/// `{id,labelStringKey,entities[]}` and uses the shelf for its
/// "continue listening" screen (`RecentListeningResponseConverter`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonalizedShelfDto {
	pub id: String,
	pub label: String,
	pub label_string_key: String,
	/// `book` or `authors`, naming what `entities` holds.
	#[serde(rename = "type")]
	pub shelf_type: String,
	pub entities: Vec<PersonalizedEntityDto>,
	pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PersonalizedEntityDto {
	Item(Box<LibraryItemDto>),
	Author(Box<AuthorDto>),
}

// ---------------------------------------------------------------------------
// Authors — capture/library_authors.json, personalized.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorDto {
	pub id: String,
	pub asin: Option<String>,
	pub name: String,
	pub description: Option<String>,
	pub image_path: Option<String>,
	pub library_id: String,
	pub added_at: i64,
	pub updated_at: i64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub num_books: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub last_first: Option<String>,
	/// `GET /api/authors/{id}?include=items` only; Lissen decodes exactly
	/// this key (`AuthorItemsResponse{libraryItems}`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub library_items: Option<Vec<LibraryItemDto>>,
}

/// `GET /api/libraries/{id}/authors`; the page envelope has no
/// `mediaType`/`collapseseries`/`include`/`offset`, unlike the item page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorsPageDto {
	pub results: Vec<AuthorDto>,
	pub total: i64,
	pub limit: i64,
	pub page: i64,
	pub sort_by: String,
	pub sort_desc: bool,
	pub minified: bool,
}

// ---------------------------------------------------------------------------
// Search — capture/library_search.json
// ---------------------------------------------------------------------------

/// `GET /api/libraries/{id}/search?q=&limit=` and `GET /api/search/books`.
/// Lissen decodes `{book[], authors[], series[]}` and fans the author and
/// series hits out into further requests
/// (`library/LibraryAudiobookshelfChannel.kt:181-238`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResultDto {
	pub book: Vec<SearchBookDto>,
	pub narrators: Vec<SearchNarratorDto>,
	pub tags: Vec<String>,
	pub genres: Vec<String>,
	pub series: Vec<SearchSeriesDto>,
	pub authors: Vec<AuthorDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchBookDto {
	pub library_item: LibraryItemDto,
	/// Which metadata field matched; abs-ref omits both keys on a title hit.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub match_key: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub match_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchSeriesDto {
	pub series: NamedIdDto,
	pub books: Vec<LibraryItemDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchNarratorDto {
	pub name: String,
	pub num_books: i64,
}

// ---------------------------------------------------------------------------
// Playback — capture/play_session.json
// ---------------------------------------------------------------------------

/// `POST /api/items/{id}/play` request body. Every field is optional: the
/// official app sends `manufacturer`/`model`/`sdkVersion` too, Lissen sends
/// only `clientName`/`deviceId`/`deviceName`
/// (`common/AudiobookshelfChannel.kt:141-158`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct PlaybackStartRequestDto {
	pub device_info: Option<DeviceInfoRequestDto>,
	pub supported_mime_types: Option<Vec<String>>,
	pub media_player: Option<String>,
	pub force_transcode: Option<bool>,
	pub force_direct_play: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct DeviceInfoRequestDto {
	pub client_name: Option<String>,
	pub client_version: Option<String>,
	pub device_id: Option<String>,
	pub device_name: Option<String>,
	pub manufacturer: Option<String>,
	pub model: Option<String>,
	/// A number from the official app, a string from others; echoed as sent.
	pub sdk_version: Option<serde_json::Value>,
}

/// The `PlaybackSession` a play request answers with. Lissen keeps only
/// `{id, libraryItemId}` (`PlaybackSessionResponse`), the official app reads
/// `audioTracks`, `chapters`, `currentTime` and `duration`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSessionDto {
	pub id: String,
	pub user_id: String,
	pub library_id: String,
	pub library_item_id: String,
	pub book_id: String,
	pub episode_id: Option<String>,
	pub media_type: String,
	pub media_metadata: BookMetadataDto,
	pub chapters: Vec<BookChapterDto>,
	pub display_title: String,
	pub display_author: Option<String>,
	pub cover_path: Option<String>,
	pub duration: f64,
	/// `0` DirectPlay, `1` DirectStream, `2` Transcode, `3` Local. Stump
	/// never transcodes, so this is always `0`.
	pub play_method: i64,
	pub media_player: String,
	pub device_info: DeviceInfoDto,
	pub server_version: String,
	/// `YYYY-MM-DD` in the server's timezone.
	pub date: String,
	pub day_of_week: String,
	pub time_listening: f64,
	pub start_time: f64,
	pub current_time: f64,
	pub started_at: i64,
	pub updated_at: i64,
	pub audio_tracks: Vec<AudioTrackDto>,
	pub library_item: LibraryItemDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfoDto {
	pub id: String,
	pub user_id: String,
	pub device_id: Option<String>,
	pub ip_address: Option<String>,
	pub client_version: Option<String>,
	pub client_name: Option<String>,
	/// The four device descriptors abs-ref echoes from the play request, and
	/// emits as `null` when the client did not send them.
	pub device_name: Option<String>,
	pub manufacturer: Option<String>,
	pub model: Option<String>,
	pub sdk_version: Option<serde_json::Value>,
}

/// `POST /api/session/{id}/sync` and `.../close`. Lissen sends
/// `{timeListened, currentTime}` (`ProgressSyncRequest`), the official app
/// adds `duration` (`media/MediaProgressSyncer.kt:17-19`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct ProgressSyncRequestDto {
	pub current_time: Option<f64>,
	pub time_listened: Option<f64>,
	pub duration: Option<f64>,
}

// ---------------------------------------------------------------------------
// Progress — capture/progress.json
// ---------------------------------------------------------------------------

/// `GET /api/me/progress/{itemId}` and the `mediaProgress[]` entries of
/// `GET /api/me`. Lissen reads
/// `{libraryItemId,episodeId,currentTime,isFinished,lastUpdate,progress}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaProgressDto {
	pub id: String,
	pub user_id: String,
	pub library_item_id: String,
	pub episode_id: Option<String>,
	/// abs-ref's `media.id`; the Stump media row's ABS book id.
	pub media_item_id: String,
	pub media_item_type: String,
	pub duration: f64,
	/// `0..=1`.
	pub progress: f64,
	pub current_time: f64,
	pub is_finished: bool,
	pub hide_from_continue_listening: bool,
	pub ebook_location: Option<String>,
	pub ebook_progress: f64,
	pub last_update: i64,
	pub started_at: i64,
	pub finished_at: Option<i64>,
}

/// `PATCH /api/me/progress/{itemId}`; a partial update, e.g.
/// `{"isFinished": true}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct ProgressPatchDto {
	pub duration: Option<f64>,
	pub progress: Option<f64>,
	pub current_time: Option<f64>,
	pub is_finished: Option<bool>,
	pub hide_from_continue_listening: Option<bool>,
	pub ebook_location: Option<String>,
	pub ebook_progress: Option<f64>,
	pub finished_at: Option<i64>,
	pub last_update: Option<i64>,
}

// ---------------------------------------------------------------------------
// Bookmarks — capture/bookmark_created.json, me_bookmarks.json
// ---------------------------------------------------------------------------

/// An `AudioBookmark`. `time` is whole seconds on the wire; abs-ref stores
/// an integer and Lissen sends `Int` (`BookmarkRequest{time,title}`) but
/// reads `Double` (`BookmarksItemResponse{time}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioBookmarkDto {
	pub library_item_id: String,
	pub time: f64,
	pub title: String,
	pub created_at: i64,
}

/// `POST`/`PATCH /api/me/item/{id}/bookmark`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct BookmarkRequestDto {
	pub time: Option<f64>,
	pub title: Option<String>,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn chapter(id: i64, start: f64, end: f64, title: &str) -> BookChapterDto {
		BookChapterDto {
			start,
			end,
			title: title.to_owned(),
			id,
		}
	}

	/// abs-ref's chapter object is exactly four keys in this order.
	#[test]
	fn chapters_serialize_as_start_end_title_id() {
		assert_eq!(
			serde_json::to_string(&chapter(1, 4.0, 8.0, "Chapter Two")).expect("json"),
			r#"{"start":4.0,"end":8.0,"title":"Chapter Two","id":1}"#
		);
	}

	/// `tracks[]`/`audioTracks[]` are the audio file's own keys plus exactly
	/// `title`, `startOffset` and `contentUrl` — flattened, not nested.
	#[test]
	fn audio_tracks_flatten_the_file_and_add_three_keys() {
		let track = AudioTrackDto {
			file: AudioFileDto {
				index: 2,
				ino: "2".to_owned(),
				metadata: FileMetadataDto {
					filename: "02 - Part 2.mp3".to_owned(),
					ext: ".mp3".to_owned(),
					path: "/books/Three Track Book/02 - Part 2.mp3".to_owned(),
					rel_path: "02 - Part 2.mp3".to_owned(),
					size: 10_450,
					mtime_ms: 1,
					ctime_ms: 1,
					birthtime_ms: 1,
				},
				added_at: 1,
				updated_at: 1,
				track_num_from_meta: Some(2),
				disc_num_from_meta: None,
				track_num_from_filename: Some(2),
				disc_num_from_filename: None,
				manually_verified: false,
				exclude: false,
				error: None,
				format: None,
				duration: 5.0,
				bit_rate: Some(16_000),
				language: None,
				codec: Some("mp3".to_owned()),
				time_base: None,
				channels: Some(1),
				channel_layout: Some("mono".to_owned()),
				chapters: Vec::new(),
				embedded_cover_art: None,
				meta_tags: MetaTagsDto {
					tag_title: Some("Part 2".to_owned()),
					..Default::default()
				},
				mime_type: "audio/mpeg".to_owned(),
			},
			title: "Part 2".to_owned(),
			start_offset: 5.0,
			content_url: "/api/items/item-1/file/2".to_owned(),
		};
		let json = serde_json::to_value(&track).expect("json");
		let object = json.as_object().expect("object");
		assert_eq!(object["index"], 2);
		assert_eq!(object["ino"], "2");
		assert_eq!(object["startOffset"], 5.0);
		assert_eq!(object["contentUrl"], "/api/items/item-1/file/2");
		assert_eq!(object["mimeType"], "audio/mpeg");
		assert_eq!(
			object["metaTags"],
			serde_json::json!({"tagTitle": "Part 2"})
		);
		assert!(
			object.get("file").is_none(),
			"the file must be flattened, not nested"
		);
		// The absent tags are omitted, exactly as abs-ref omits them.
		assert_eq!(
			object["metaTags"].as_object().expect("tags").len(),
			1,
			"only the tags we read are emitted"
		);
	}

	/// `collapseseries` is the one lower-case key of the item page, next to
	/// ten camelCase neighbours; a `rename_all` that swept it up would ship
	/// `collapseSeries` and Lissen's `LibraryItemsResponse` would stop
	/// reading the grouping.
	#[test]
	fn envelope_keys_match_abs_refs_casing() {
		let page = LibraryItemsPageDto {
			results: Vec::new(),
			total: 0,
			limit: 10,
			page: 0,
			sort_by: "media.metadata.title".to_owned(),
			sort_desc: false,
			media_type: "book".to_owned(),
			minified: true,
			collapse_series: false,
			include: String::new(),
			offset: 0,
		};
		let json = serde_json::to_value(&page).expect("json");
		let mut keys = json
			.as_object()
			.expect("object")
			.keys()
			.cloned()
			.collect::<Vec<_>>();
		keys.sort();
		// The key *set* is the contract; JSON objects are unordered and
		// `serde_json` sorts them.
		assert_eq!(
			keys,
			vec![
				"collapseseries",
				"include",
				"limit",
				"mediaType",
				"minified",
				"offset",
				"page",
				"results",
				"sortBy",
				"sortDesc",
				"total",
			]
		);
	}

	#[test]
	fn token_keys_distinguish_absent_from_null() {
		let base = UserDto {
			id: "user-1".to_owned(),
			username: "ada".to_owned(),
			email: None,
			user_type: "root".to_owned(),
			token: "legacy".to_owned(),
			media_progress: Vec::new(),
			series_hide_from_continue_listening: Vec::new(),
			bookmarks: Vec::new(),
			is_active: true,
			is_locked: false,
			last_seen: None,
			created_at: 1,
			permissions: UserPermissionsDto {
				download: true,
				update: false,
				delete: false,
				upload: false,
				create_ereader: false,
				access_all_libraries: true,
				access_all_tags: true,
				access_explicit_content: true,
				selected_tags_not_accessible: false,
			},
			libraries_accessible: Vec::new(),
			item_tags_selected: Vec::new(),
			has_openid_link: false,
			is_old_token: None,
			access_token: None,
			refresh_token: None,
		};

		// GET /api/me: neither key exists.
		let me = serde_json::to_value(&base).expect("json");
		assert!(me.get("accessToken").is_none());
		assert!(me.get("refreshToken").is_none());
		assert_eq!(me["hasOpenIDLink"], false);

		// A login without `x-return-tokens`: refreshToken is present, null.
		let login = serde_json::to_value(&UserDto {
			access_token: Some("access".to_owned()),
			refresh_token: Some(None),
			..base.clone()
		})
		.expect("json");
		assert_eq!(login["accessToken"], "access");
		assert_eq!(login["refreshToken"], serde_json::Value::Null);
		assert!(login
			.as_object()
			.expect("object")
			.contains_key("refreshToken"));

		// With the header: the token itself.
		let with_tokens = serde_json::to_value(&UserDto {
			access_token: Some("access".to_owned()),
			refresh_token: Some(Some("refresh".to_owned())),
			..base
		})
		.expect("json");
		assert_eq!(with_tokens["refreshToken"], "refresh");
	}

	/// Lissen sends `{"timeListened":..,"currentTime":..}` and the official
	/// app adds `duration`; both must parse, and so must an empty body.
	#[test]
	fn sync_bodies_from_both_clients_parse() {
		let lissen: ProgressSyncRequestDto =
			serde_json::from_str(r#"{"timeListened":45.0,"currentTime":128.5}"#)
				.expect("lissen body");
		assert_eq!(lissen.time_listened, Some(45.0));
		assert_eq!(lissen.current_time, Some(128.5));
		assert_eq!(lissen.duration, None);

		let official: ProgressSyncRequestDto = serde_json::from_str(
			r#"{"timeListened":15,"duration":3600.0,"currentTime":42}"#,
		)
		.expect("official body");
		assert_eq!(official.duration, Some(3600.0));

		assert_eq!(
			serde_json::from_str::<ProgressSyncRequestDto>("{}").expect("empty"),
			ProgressSyncRequestDto::default()
		);
	}

	/// `PATCH /api/me/progress/{id}` is partial: `{"isFinished":true}` must
	/// leave every other field untouched.
	#[test]
	fn progress_patch_is_partial() {
		let patch: ProgressPatchDto =
			serde_json::from_str(r#"{"isFinished":true}"#).expect("patch");
		assert_eq!(patch.is_finished, Some(true));
		assert_eq!(patch.current_time, None);
		assert_eq!(patch.progress, None);
		assert_eq!(patch.duration, None);
	}

	/// Lissen posts `{"time":<int>,"title":<string>}`.
	#[test]
	fn bookmark_request_accepts_integer_seconds() {
		let request: BookmarkRequestDto =
			serde_json::from_str(r#"{"time":3,"title":"Captured bookmark"}"#)
				.expect("bookmark");
		assert_eq!(request.time, Some(3.0));
		assert_eq!(request.title.as_deref(), Some("Captured bookmark"));
	}

	/// A personalized shelf mixes item and author entities in one array.
	#[test]
	fn personalized_entities_are_untagged() {
		let author = AuthorDto {
			id: "author-1".to_owned(),
			asin: None,
			name: "Ada Lovelace".to_owned(),
			description: None,
			image_path: None,
			library_id: "library-1".to_owned(),
			added_at: 1,
			updated_at: 1,
			num_books: Some(1),
			last_first: Some("Lovelace, Ada".to_owned()),
			library_items: None,
		};
		let shelf = PersonalizedShelfDto {
			id: "newest-authors".to_owned(),
			label: "Newest Authors".to_owned(),
			label_string_key: "LabelNewestAuthors".to_owned(),
			shelf_type: "authors".to_owned(),
			entities: vec![PersonalizedEntityDto::Author(Box::new(author))],
			total: 1,
		};
		let json = serde_json::to_value(&shelf).expect("json");
		assert_eq!(json["labelStringKey"], "LabelNewestAuthors");
		assert_eq!(json["type"], "authors");
		assert_eq!(json["entities"][0]["name"], "Ada Lovelace");
		assert_eq!(json["entities"][0]["numBooks"], 1);
		assert!(
			json["entities"][0].get("libraryItems").is_none(),
			"a shelf entity carries no item list"
		);
	}
}
