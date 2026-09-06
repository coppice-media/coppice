//! Stump rows in, Audiobookshelf JSON out.
//!
//! Nothing here touches the database or the filesystem: every function takes
//! the rows a route already loaded plus the audio/progress facts the backend
//! supplied, so the whole wire contract is unit-testable. The reference values
//! for every constant are the abs-ref 2.36.0 captures under
//! `../komga-compat/abs/capture/`, named per function.
//!
//! Units, once, at this boundary: Stump stores audio positions and durations
//! in **milliseconds**; Audiobookshelf sends positions and durations as
//! **seconds** (JSON numbers, fractional) and timestamps as **milliseconds**
//! since the epoch. [`ms_to_secs`] and [`secs_to_ms`] are the only conversions
//! in the profile.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Utc};
use models::entity::{media, media_metadata, user::AuthUser};
use models::shared::enums::UserPermission;

use crate::{
	dto::*,
	model::{AbsAudio, AbsBookmark, AbsProgress, ItemShape},
	ABS_BUILD_NUMBER, ABS_VERSION,
};

// ---------------------------------------------------------------------------
// Units and small conversions
// ---------------------------------------------------------------------------

/// Milliseconds to the fractional seconds the ABS wire uses.
pub fn ms_to_secs(ms: i64) -> f64 {
	ms as f64 / 1000.0
}

/// Wire seconds back to milliseconds, rounded to the nearest millisecond.
///
/// A client that reports `4.5` means 4500 ms exactly; truncating would lose a
/// half second on every sync.
pub fn secs_to_ms(secs: f64) -> i64 {
	(secs * 1000.0).round() as i64
}

/// A timestamp as abs-ref emits it: milliseconds since the epoch.
pub fn ts_ms(at: DateTime<Utc>) -> i64 {
	at.timestamp_millis()
}

fn opt_ts_ms<Tz: chrono::TimeZone>(at: Option<DateTime<Tz>>) -> Option<i64> {
	at.map(|at| at.timestamp_millis())
}

/// One of Stump's comma-separated people/genre columns, in order, without
/// blanks or repeats.
pub fn csv(value: Option<&str>) -> Vec<String> {
	let mut seen = Vec::new();
	for part in value.unwrap_or_default().split(',') {
		let part = part.trim();
		if !part.is_empty() && !seen.iter().any(|kept: &String| kept == part) {
			seen.push(part.to_owned());
		}
	}
	seen
}

/// abs-ref's `sortingPrefixes` default (`capture/authorize.json`).
pub const SORTING_PREFIXES: [&str; 2] = ["the", "a"];

/// A title or series name with its leading article moved to the end, which is
/// what abs-ref's `titleIgnorePrefix`/`nameIgnorePrefix` hold. Without a
/// leading article the value is the title verbatim
/// (`capture/library_items_minified.json`: `"Analytical Engine"` for both).
pub fn ignore_prefix(title: &str) -> String {
	for prefix in SORTING_PREFIXES {
		let (head, rest) = match title.split_once(' ') {
			Some(split) => split,
			None => break,
		};
		if head.eq_ignore_ascii_case(prefix) {
			return format!("{rest}, {head}");
		}
	}
	title.to_owned()
}

/// `authorNameLF`/`lastFirst`: `"Ada Lovelace"` → `"Lovelace, Ada"`
/// (`capture/library_authors.json`). A single-word name is unchanged.
pub fn last_first(name: &str) -> String {
	match name.trim().rsplit_once(' ') {
		Some((first, last)) => format!("{last}, {first}"),
		None => name.trim().to_owned(),
	}
}

/// `media.metadata.seriesName`: `""` outside a series, `"Name"` without a
/// sequence and `"Name #1"` with one
/// (`capture/library_items_minified.json`).
pub fn series_name(series: Option<(&str, &str)>, sequence: Option<&str>) -> String {
	match (series, sequence) {
		(None, _) => String::new(),
		(Some((_, name)), None) => name.to_owned(),
		(Some((_, name)), Some(sequence)) => format!("{name} #{sequence}"),
	}
}

/// The `sequence` abs-ref reports for a book inside a series: Stump's
/// `media_metadata.number` without trailing zeros, or `None`.
pub fn sequence(metadata: Option<&media_metadata::Model>) -> Option<String> {
	metadata
		.and_then(|metadata| metadata.number)
		.map(|number| number.normalize().to_string())
}

// ---------------------------------------------------------------------------
// Server identity — capture/status.json, ping.json, authorize.json
// ---------------------------------------------------------------------------

pub fn status() -> StatusDto {
	StatusDto {
		app: "audiobookshelf".to_owned(),
		server_version: ABS_VERSION.to_owned(),
		is_init: true,
		language: "en-us".to_owned(),
		auth_methods: vec!["local".to_owned()],
		auth_form_data: AuthFormDataDto {
			auth_login_custom_message: String::new(),
		},
	}
}

/// `serverSettings`, verbatim from `capture/authorize.json`. Stump has no
/// analogue for most of it and a client that reasons about these would be
/// reasoning about an Audiobookshelf install, not a Stump one, so the values
/// are abs-ref's defaults; only `version`/`buildNumber` are meaningful.
pub fn server_settings() -> ServerSettingsDto {
	ServerSettingsDto {
		id: "server-settings".to_owned(),
		scanner_find_covers: false,
		scanner_cover_provider: "google".to_owned(),
		scanner_parse_subtitle: false,
		scanner_prefer_matched_metadata: false,
		scanner_disable_watcher: false,
		store_cover_with_item: false,
		store_metadata_with_item: false,
		metadata_file_format: "json".to_owned(),
		rate_limit_login_requests: 10,
		rate_limit_login_window: 600_000,
		allow_iframe: false,
		backup_path: "/metadata/backups".to_owned(),
		backup_schedule: false,
		backups_to_keep: 2,
		max_backup_size: 1,
		logger_daily_logs_to_keep: 7,
		logger_scanner_logs_to_keep: 2,
		home_bookshelf_view: 1,
		bookshelf_view: 1,
		podcast_episode_schedule: "0 * * * *".to_owned(),
		sorting_ignore_prefix: false,
		sorting_prefixes: SORTING_PREFIXES.iter().map(|p| (*p).to_owned()).collect(),
		chromecast_enabled: false,
		date_format: "MM/dd/yyyy".to_owned(),
		time_format: "HH:mm".to_owned(),
		language: "en-us".to_owned(),
		allowed_origins: Vec::new(),
		log_level: 2,
		version: ABS_VERSION.to_owned(),
		build_number: ABS_BUILD_NUMBER,
		auth_login_custom_message: None,
		auth_active_auth_methods: vec!["local".to_owned()],
		auth_openid_issuer_url: None,
		auth_openid_authorization_url: None,
		auth_openid_token_url: None,
		auth_openid_user_info_url: None,
		auth_openid_jwks_url: None,
		auth_openid_logout_url: None,
		auth_openid_token_signing_algorithm: "RS256".to_owned(),
		auth_openid_button_text: "Login with OpenId".to_owned(),
		auth_openid_auto_launch: false,
		auth_openid_auto_register: false,
		auth_openid_match_existing_by: None,
		time_zone: "UTC".to_owned(),
	}
}

// ---------------------------------------------------------------------------
// Identity — capture/login.json, me.json
// ---------------------------------------------------------------------------

/// Audiobookshelf's permission flags, resolved from Stump's grants. Two of
/// them are load-bearing rather than cosmetic: `accessAllLibraries` is false
/// for a device with a library scope, and `accessExplicitContent` is false for
/// an age-restricted user, so a client shows the same limits the server
/// enforces.
pub fn user_permissions(user: &AuthUser) -> UserPermissionsDto {
	let owner = user.is_server_owner;
	let granted = |permission| owner || user.has_permission(permission);
	UserPermissionsDto {
		download: granted(UserPermission::DownloadFile),
		update: granted(UserPermission::EditMetadata),
		delete: granted(UserPermission::DeleteLibrary),
		upload: granted(UserPermission::UploadFile),
		create_ereader: granted(UserPermission::EmailerCreate),
		access_all_libraries: user.device_library_scope.is_none(),
		access_all_tags: true,
		access_explicit_content: user.age_restriction.is_none(),
		selected_tags_not_accessible: false,
	}
}

/// Which token keys the `user` object carries, which abs-ref varies per
/// route and clients notice: Lissen reads `accessToken`/`refreshToken` off a
/// login (`common/model/user/LoggedUserResponse.kt`) and would take `null`
/// for "no session" if they showed up empty elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenPresentation {
	/// `POST /login`: the trio plus `isOldToken: true`
	/// (`capture/login.json`).
	Login {
		access: String,
		/// `None` writes `refreshToken: null`, which is what abs-ref answers
		/// without `x-return-tokens: true`.
		refresh: Option<String>,
	},
	/// `POST /auth/refresh`: the trio, no `isOldToken`
	/// (`capture/refresh.json`).
	Refresh {
		access: String,
		refresh: Option<String>,
	},
	/// `GET /api/me` and `POST /api/authorize`: `token` alone
	/// (`capture/me.json`, `capture/authorize.json`).
	LegacyOnly,
}

/// The `user` object.
pub struct UserInput<'a> {
	pub user: &'a AuthUser,
	pub created_at: DateTime<Utc>,
	pub legacy_token: String,
	pub tokens: TokenPresentation,
	pub media_progress: Vec<MediaProgressDto>,
	pub bookmarks: Vec<AudioBookmarkDto>,
	/// The library ids a scoped device may see; empty means "all", exactly
	/// as abs-ref uses it alongside `accessAllLibraries`.
	pub libraries_accessible: Vec<String>,
}

pub fn user_dto(input: UserInput<'_>) -> UserDto {
	let (is_old_token, access_token, refresh_token) = match input.tokens {
		TokenPresentation::Login { access, refresh } => {
			(Some(true), Some(access), Some(refresh))
		},
		TokenPresentation::Refresh { access, refresh } => {
			(None, Some(access), Some(refresh))
		},
		TokenPresentation::LegacyOnly => (None, None, None),
	};
	UserDto {
		id: input.user.id.clone(),
		username: input.user.username.clone(),
		email: None,
		user_type: if input.user.is_server_owner {
			"root".to_owned()
		} else {
			"user".to_owned()
		},
		token: input.legacy_token,
		media_progress: input.media_progress,
		series_hide_from_continue_listening: Vec::new(),
		bookmarks: input.bookmarks,
		is_active: !input.user.is_locked,
		is_locked: input.user.is_locked,
		last_seen: None,
		created_at: ts_ms(input.created_at),
		permissions: user_permissions(input.user),
		libraries_accessible: input.libraries_accessible,
		item_tags_selected: Vec::new(),
		has_openid_link: false,
		is_old_token,
		access_token,
		refresh_token,
	}
}

/// `POST /login`, `POST /auth/refresh`, `POST /api/authorize`.
pub fn login_response(
	user: UserDto,
	default_library_id: Option<String>,
	is_docker: bool,
) -> LoginResponseDto {
	LoginResponseDto {
		user,
		user_default_library_id: default_library_id,
		server_settings: server_settings(),
		ereader_devices: Vec::new(),
		source: if is_docker {
			"docker".to_owned()
		} else {
			"local".to_owned()
		},
	}
}

// ---------------------------------------------------------------------------
// Libraries — capture/libraries.json
// ---------------------------------------------------------------------------

pub struct LibraryInput<'a> {
	pub id: &'a str,
	pub name: &'a str,
	pub path: &'a str,
	pub folder_id: &'a str,
	pub display_order: i32,
	pub created_at: DateTime<Utc>,
	pub updated_at: Option<DateTime<Utc>>,
	pub last_scanned_at: Option<DateTime<Utc>>,
}

pub fn library_dto(input: LibraryInput<'_>) -> LibraryDto {
	let created_at = ts_ms(input.created_at);
	LibraryDto {
		id: input.id.to_owned(),
		name: input.name.to_owned(),
		folders: vec![LibraryFolderDto {
			id: input.folder_id.to_owned(),
			full_path: input.path.to_owned(),
			library_id: input.id.to_owned(),
			added_at: created_at,
		}],
		display_order: input.display_order,
		icon: "audiobookshelf".to_owned(),
		media_type: "book".to_owned(),
		provider: "audible".to_owned(),
		settings: LibrarySettingsDto {
			cover_aspect_ratio: 1,
			disable_watcher: false,
			auto_scan_cron_expression: None,
			skip_matching_media_with_asin: false,
			skip_matching_media_with_isbn: false,
			audiobooks_only: false,
			epubs_allow_scripted_content: false,
			// A Stump series holding one audible book is that book's own
			// folder, and this profile does not report it as a series; the
			// setting says so to any client that reads it.
			hide_single_book_series: true,
			only_show_later_books_in_continue_series: false,
			metadata_precedence: [
				"folderStructure",
				"audioMetatags",
				"nfoFile",
				"txtFiles",
				"opfFile",
				"absMetadata",
			]
			.iter()
			.map(|value| (*value).to_owned())
			.collect(),
			mark_as_finished_percent_complete: None,
			mark_as_finished_time_remaining: Some(10),
		},
		last_scan: opt_ts_ms(input.last_scanned_at),
		last_scan_version: input.last_scanned_at.map(|_| ABS_VERSION.to_owned()),
		created_at,
		last_update: opt_ts_ms(input.updated_at).unwrap_or(created_at),
	}
}

/// The `filterdata` block of `GET /api/libraries/{id}?include=filterdata`.
pub struct FilterDataInput {
	pub authors: Vec<NamedIdDto>,
	pub genres: Vec<String>,
	pub tags: Vec<String>,
	pub series: Vec<NamedIdDto>,
	pub narrators: Vec<String>,
	pub languages: Vec<String>,
	pub publishers: Vec<String>,
	/// Every publication year in the library; the decades are derived.
	pub published_years: Vec<i32>,
	pub book_count: i64,
}

pub fn filter_data(input: FilterDataInput) -> FilterDataDto {
	let mut decades: Vec<String> = Vec::new();
	for year in input.published_years {
		let decade = format!("{}", year - year.rem_euclid(10));
		if !decades.contains(&decade) {
			decades.push(decade);
		}
	}
	decades.sort();
	FilterDataDto {
		author_count: input.authors.len() as i64,
		series_count: input.series.len() as i64,
		published_decades: decades,
		authors: input.authors,
		genres: input.genres,
		tags: input.tags,
		series: input.series,
		narrators: input.narrators,
		languages: input.languages,
		publishers: input.publishers,
		book_count: input.book_count,
		podcast_count: 0,
		num_issues: 0,
		loaded_at: ts_ms(Utc::now()),
	}
}

// ---------------------------------------------------------------------------
// Library items — capture/{library_items_minified,item,item_expanded}.json
// ---------------------------------------------------------------------------

/// Everything one library item needs, in Stump's terms.
pub struct ItemInput<'a> {
	pub media: &'a media::Model,
	pub metadata: Option<&'a media_metadata::Model>,
	/// `(series id, series name)` of the Stump series the book belongs to.
	pub series: Option<(&'a str, &'a str)>,
	pub library_id: &'a str,
	/// The library's root path on disk, for the item's `relPath`.
	pub library_path: &'a str,
	pub folder_id: &'a str,
	/// The ABS `media.id` allocated for this row in `abs_ids`.
	pub book_id: &'a str,
	pub audio: Option<&'a AbsAudio>,
	/// Author name → the ABS author id allocated for it.
	pub author_ids: &'a HashMap<String, String>,
	pub progress: Option<MediaProgressDto>,
	/// `Some` only under `collapseseries=1`, and `Some(None)` there for a
	/// book outside a series (`capture/library_items_collapseseries.json`).
	pub collapsed: Option<Option<CollapsedSeriesDto>>,
}

/// The `.ext` of a Stump row, with the leading period abs-ref emits.
fn dotted_ext(extension: &str) -> String {
	format!(".{}", extension.trim_start_matches('.'))
}

fn file_name(path: &str) -> String {
	path.rsplit('/')
		.next()
		.filter(|name| !name.is_empty())
		.unwrap_or(path)
		.to_owned()
}

/// The item's `path`/`relPath`, and whether it is a single container file.
///
/// A Stump audiobook is either one container (`book.m4b`, one track whose
/// path *is* the media path) or a folder of tracks (`Book/01.mp3`, tracks
/// below it), which is exactly abs-ref's `isFile` distinction.
fn item_paths(
	media: &media::Model,
	library_path: &str,
	audio: Option<&AbsAudio>,
) -> (String, String, bool) {
	let path = media.path.clone();
	let is_file = audio
		.is_none_or(|audio| audio.tracks.iter().all(|track| track.path == media.path));
	let rel_path = path
		.strip_prefix(library_path)
		.filter(|_| !library_path.is_empty())
		.map(|rest| rest.trim_start_matches('/').to_owned())
		.unwrap_or_else(|| file_name(&path));
	(path, rel_path, is_file)
}

fn meta_tags(
	metadata: Option<&media_metadata::Model>,
	title: Option<&str>,
) -> MetaTagsDto {
	let metadata_field = |get: fn(&media_metadata::Model) -> Option<&String>| {
		metadata.and_then(get).cloned()
	};
	MetaTagsDto {
		tag_album: metadata_field(|m| m.series.as_ref()),
		tag_artist: metadata_field(|m| m.writers.as_ref()),
		tag_album_artist: metadata_field(|m| m.writers.as_ref()),
		tag_title: title.map(str::to_owned),
		tag_track: None,
		tag_genre: metadata_field(|m| m.genres.as_ref()),
		tag_date: metadata.and_then(|m| m.year).map(|year| year.to_string()),
		tag_composer: None,
		tag_publisher: metadata_field(|m| m.publisher.as_ref()),
		tag_subtitle: None,
		tag_series: metadata_field(|m| m.series.as_ref()),
		tag_series_part: sequence(metadata),
		tag_language: metadata_field(|m| m.language.as_ref()),
		tag_asin: metadata_field(|m| m.identifier_mobi_asin.as_ref()),
		tag_isbn: metadata_field(|m| m.identifier_isbn.as_ref()),
		tag_description: None,
	}
}

/// `media.chapters[]`. `id` is the 0-based chapter index and `end` falls back
/// to the publication duration for the last chapter, which is what abs-ref
/// emits for a container whose final chapter has no explicit end.
pub fn chapters(audio: &AbsAudio) -> Vec<BookChapterDto> {
	audio
		.chapters
		.iter()
		.map(|chapter| BookChapterDto {
			start: ms_to_secs(chapter.start_ms),
			end: ms_to_secs(chapter.end_ms.unwrap_or(audio.duration_ms)),
			title: chapter.title.clone().unwrap_or_else(|| {
				format!("Chapter {}", chapter.index.saturating_add(1))
			}),
			id: i64::from(chapter.index),
		})
		.collect()
}

/// `media.audioFiles[]`. `index` is 1-based display order; `ino` is the
/// 0-based Stump track index, which is also the `{ino}` of
/// `GET /api/items/{id}/file/{ino}`. abs-ref uses a filesystem inode there,
/// which Stump has no stable equivalent of across rescans.
pub fn audio_files(
	audio: &AbsAudio,
	metadata: Option<&media_metadata::Model>,
	added_at: i64,
	updated_at: i64,
) -> Vec<AudioFileDto> {
	audio
		.tracks
		.iter()
		.map(|track| {
			let filename = file_name(&track.path);
			let ext = filename
				.rsplit_once('.')
				.map(|(_, ext)| dotted_ext(ext))
				.unwrap_or_default();
			AudioFileDto {
				index: i64::from(track.index) + 1,
				ino: track.index.to_string(),
				metadata: FileMetadataDto {
					filename: filename.clone(),
					ext,
					path: track.path.clone(),
					rel_path: filename.clone(),
					size: track.byte_size,
					mtime_ms: updated_at,
					ctime_ms: updated_at,
					birthtime_ms: added_at,
				},
				added_at,
				updated_at,
				track_num_from_meta: Some(i64::from(track.index) + 1),
				disc_num_from_meta: None,
				track_num_from_filename: None,
				disc_num_from_filename: None,
				manually_verified: false,
				exclude: false,
				error: None,
				format: None,
				duration: ms_to_secs(track.duration_ms),
				bit_rate: audio.bitrate.map(i64::from),
				language: metadata.and_then(|m| m.language.clone()),
				codec: Some(audio.codec.clone()),
				time_base: None,
				channels: audio.channels.map(i64::from),
				channel_layout: None,
				chapters: Vec::new(),
				embedded_cover_art: None,
				meta_tags: meta_tags(metadata, Some(&filename)),
				mime_type: track.mime.clone(),
			}
		})
		.collect()
}

/// `media.tracks[]` / `playbackSession.audioTracks[]`: the audio files plus
/// the three keys abs-ref adds for playback (`capture/play_session.json`).
/// `contentUrl` is the relative route the client resolves against its host,
/// which is the same URL Lissen builds itself from `(itemId, ino)`
/// (`common/AudiobookshelfChannel.kt:48-64`).
pub fn audio_tracks(
	item_id: &str,
	audio: &AbsAudio,
	metadata: Option<&media_metadata::Model>,
	added_at: i64,
	updated_at: i64,
) -> Vec<AudioTrackDto> {
	audio_files(audio, metadata, added_at, updated_at)
		.into_iter()
		.zip(audio.tracks.iter())
		.map(|(file, track)| AudioTrackDto {
			title: file.metadata.filename.clone(),
			start_offset: ms_to_secs(track.start_offset_ms),
			content_url: format!("/api/items/{item_id}/file/{}", file.ino),
			file,
		})
		.collect()
}

/// `libraryFiles[]`, the detail-shape file listing: one entry per audio
/// track, `fileType: "audio"`.
fn library_files(files: &[AudioFileDto]) -> Vec<LibraryFileDto> {
	files
		.iter()
		.map(|file| LibraryFileDto {
			ino: file.ino.clone(),
			metadata: file.metadata.clone(),
			is_supplementary: None,
			added_at: file.added_at,
			updated_at: file.updated_at,
			file_type: "audio".to_owned(),
		})
		.collect()
}

pub fn book_metadata(input: &ItemInput<'_>, shape: ItemShape) -> BookMetadataDto {
	let metadata = input.metadata;
	let title = metadata
		.and_then(|m| m.title.clone())
		.unwrap_or_else(|| input.media.name.clone());
	let authors = csv(metadata.and_then(|m| m.writers.as_deref()));
	// Always empty: no Stump column holds a narrator (see
	// [`crate::model::AbsAudio`]).
	let narrators: Vec<String> = Vec::new();
	let sequence = sequence(metadata);
	let description = metadata.and_then(|m| m.summary.clone());

	BookMetadataDto {
		title: Some(title.clone()),
		subtitle: None,
		authors: shape.has_detail_keys().then(|| {
			authors
				.iter()
				.map(|name| NamedIdDto {
					id: input
						.author_ids
						.get(name)
						.cloned()
						.unwrap_or_else(|| name.clone()),
					name: name.clone(),
				})
				.collect()
		}),
		narrators: shape.has_detail_keys().then(|| narrators.clone()),
		series: shape.has_detail_keys().then(|| {
			input
				.series
				.map(|(id, name)| {
					vec![SeriesSequenceDto {
						id: id.to_owned(),
						name: name.to_owned(),
						sequence: sequence.clone(),
					}]
				})
				.unwrap_or_default()
		}),
		genres: csv(metadata.and_then(|m| m.genres.as_deref())),
		published_year: metadata.and_then(|m| m.year).map(|year| year.to_string()),
		published_date: None,
		publisher: metadata.and_then(|m| m.publisher.clone()),
		description: description.clone(),
		isbn: metadata.and_then(|m| m.identifier_isbn.clone()),
		asin: metadata.and_then(|m| m.identifier_mobi_asin.clone()),
		language: metadata.and_then(|m| m.language.clone()),
		explicit: false,
		abridged: false,
		title_ignore_prefix: shape.has_minified_keys().then(|| ignore_prefix(&title)),
		author_name: shape.has_minified_keys().then(|| authors.join(", ")),
		author_name_lf: shape.has_minified_keys().then(|| {
			authors
				.iter()
				.map(|name| last_first(name))
				.collect::<Vec<_>>()
				.join(", ")
		}),
		narrator_name: shape.has_minified_keys().then(|| narrators.join(", ")),
		series_name: shape
			.has_minified_keys()
			.then(|| series_name(input.series, sequence.as_deref())),
		description_plain: shape.has_tracks().then(|| description),
	}
}

pub fn item_dto(input: ItemInput<'_>, shape: ItemShape) -> LibraryItemDto {
	let added_at = ts_ms(input.media.created_at.into());
	let updated_at = input
		.media
		.updated_at
		.map(|at| at.timestamp_millis())
		.unwrap_or(added_at);
	let mtime_ms = input
		.media
		.modified_at
		.map(|at| at.timestamp_millis())
		.unwrap_or(added_at);

	let metadata = book_metadata(&input, shape);
	let files = input
		.audio
		.map(|audio| audio_files(audio, input.metadata, added_at, updated_at));
	let tracks = input.audio.filter(|_| shape.has_tracks()).map(|audio| {
		audio_tracks(&input.media.id, audio, input.metadata, added_at, updated_at)
	});
	let cover_path = input
		.media
		.thumbnail_path
		.clone()
		.or_else(|| Some(input.media.path.clone()));

	let media = BookDto {
		id: input.book_id.to_owned(),
		library_item_id: shape.has_detail_keys().then(|| input.media.id.clone()),
		metadata,
		cover_path,
		tags: Vec::new(),
		num_tracks: shape
			.has_minified_keys()
			.then(|| input.audio.map_or(0, |audio| audio.tracks.len() as i64)),
		num_audio_files: shape
			.has_minified_keys()
			.then(|| input.audio.map_or(0, |audio| audio.tracks.len() as i64)),
		num_chapters: shape
			.has_minified_keys()
			.then(|| input.audio.map_or(0, |audio| audio.chapters.len() as i64)),
		duration: shape.has_minified_keys().then(|| {
			input
				.audio
				.map_or(0.0, |audio| ms_to_secs(audio.duration_ms))
		}),
		size: shape.has_minified_keys().then_some(input.media.size),
		audio_files: shape
			.has_detail_keys()
			.then(|| files.clone().unwrap_or_default()),
		chapters: shape
			.has_detail_keys()
			.then(|| input.audio.map(chapters).unwrap_or_default()),
		ebook_file: shape.has_detail_keys().then_some(None),
		tracks,
	};

	let (path, rel_path, is_file) =
		item_paths(input.media, input.library_path, input.audio);
	LibraryItemDto {
		id: input.media.id.clone(),
		ino: input.media.id.clone(),
		old_library_item_id: None,
		library_id: input.library_id.to_owned(),
		folder_id: input.folder_id.to_owned(),
		path,
		rel_path,
		is_file,
		mtime_ms,
		ctime_ms: mtime_ms,
		birthtime_ms: added_at,
		added_at,
		updated_at,
		is_missing: false,
		is_invalid: false,
		media_type: "book".to_owned(),
		media,
		num_files: shape
			.has_minified_keys()
			.then(|| files.as_ref().map_or(0, |files| files.len() as i64)),
		size: shape.has_minified_keys().then_some(input.media.size),
		last_scan: shape.has_detail_keys().then_some(updated_at),
		scan_version: shape.has_detail_keys().then(|| ABS_VERSION.to_owned()),
		library_files: shape
			.has_detail_keys()
			.then(|| files.as_deref().map(library_files).unwrap_or_default()),
		user_media_progress: input.progress,
		collapsed_series: input.collapsed,
	}
}

// ---------------------------------------------------------------------------
// Progress and bookmarks — capture/progress.json, bookmark_created.json
// ---------------------------------------------------------------------------

/// `GET /api/me/progress/{itemId}`. `progress` is the fraction of the
/// publication listened to, clamped to `0..=1`, and a finished book reports
/// exactly `1` no matter where the position sits
/// (`capture/progress.json`: `currentTime: 4.5`, `duration: 12`,
/// `isFinished: true`, `progress: 1`).
pub fn progress_dto(
	user_id: &str,
	item_id: &str,
	book_id: &str,
	duration_ms: i64,
	progress: &AbsProgress,
) -> MediaProgressDto {
	let fraction = if progress.is_finished {
		1.0
	} else if duration_ms > 0 {
		(progress.position_ms as f64 / duration_ms as f64).clamp(0.0, 1.0)
	} else {
		0.0
	};
	MediaProgressDto {
		id: format!("{user_id}-{item_id}"),
		user_id: user_id.to_owned(),
		library_item_id: item_id.to_owned(),
		episode_id: None,
		media_item_id: book_id.to_owned(),
		media_item_type: "book".to_owned(),
		duration: ms_to_secs(duration_ms),
		progress: fraction,
		current_time: ms_to_secs(progress.position_ms),
		is_finished: progress.is_finished,
		hide_from_continue_listening: false,
		ebook_location: None,
		ebook_progress: 0.0,
		last_update: ts_ms(progress.last_update),
		started_at: ts_ms(progress.started_at),
		finished_at: progress.finished_at.map(ts_ms),
	}
}

pub fn bookmark_dto(item_id: &str, bookmark: &AbsBookmark) -> AudioBookmarkDto {
	AudioBookmarkDto {
		library_item_id: item_id.to_owned(),
		time: ms_to_secs(bookmark.position_ms),
		title: bookmark.title.clone(),
		created_at: ts_ms(bookmark.created_at),
	}
}

// ---------------------------------------------------------------------------
// Playback — capture/play_session.json
// ---------------------------------------------------------------------------

pub struct SessionInput<'a> {
	pub session_id: &'a str,
	pub user_id: &'a str,
	pub item: LibraryItemDto,
	pub audio: &'a AbsAudio,
	pub device_id: Option<String>,
	pub client_name: Option<String>,
	pub client_version: Option<String>,
	pub media_player: Option<String>,
	pub current_time_ms: i64,
	pub time_listening_ms: i64,
	pub started_at: DateTime<Utc>,
	pub updated_at: DateTime<Utc>,
}

/// The `PlaybackSession` a play request answers with. `playMethod` is always
/// `0` (DirectPlay): Stump never transcodes, so a client must play the tracks
/// as served.
pub fn session_dto(input: SessionInput<'_>) -> PlaybackSessionDto {
	let item = input.item;
	let metadata = item.media.metadata.clone();
	let display_title = metadata.title.clone().unwrap_or_default();
	let display_author = metadata
		.author_name
		.clone()
		.or_else(|| {
			metadata.authors.as_ref().map(|authors| {
				authors
					.iter()
					.map(|author| author.name.as_str())
					.collect::<Vec<_>>()
					.join(", ")
			})
		})
		.filter(|name| !name.is_empty());
	let tracks =
		audio_tracks(&item.id, input.audio, None, item.added_at, item.updated_at);
	PlaybackSessionDto {
		id: input.session_id.to_owned(),
		user_id: input.user_id.to_owned(),
		library_id: item.library_id.clone(),
		library_item_id: item.id.clone(),
		book_id: item.media.id.clone(),
		episode_id: None,
		media_type: "book".to_owned(),
		media_metadata: metadata,
		chapters: chapters(input.audio),
		display_title,
		display_author,
		cover_path: item.media.cover_path.clone(),
		duration: ms_to_secs(input.audio.duration_ms),
		play_method: 0,
		media_player: input.media_player.unwrap_or_else(|| "unknown".to_owned()),
		device_info: DeviceInfoDto {
			id: input.session_id.to_owned(),
			user_id: input.user_id.to_owned(),
			device_id: input.device_id,
			ip_address: None,
			client_version: input.client_version,
			client_name: input.client_name,
		},
		server_version: ABS_VERSION.to_owned(),
		date: input.started_at.format("%Y-%m-%d").to_string(),
		day_of_week: input.started_at.weekday().to_string(),
		time_listening: ms_to_secs(input.time_listening_ms),
		start_time: ms_to_secs(input.current_time_ms),
		current_time: ms_to_secs(input.current_time_ms),
		started_at: ts_ms(input.started_at),
		updated_at: ts_ms(input.updated_at),
		audio_tracks: tracks,
		library_item: item,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn seconds_round_trip_without_losing_a_half_second() {
		assert_eq!(secs_to_ms(4.5), 4_500);
		assert_eq!(ms_to_secs(4_500), 4.5);
		// Truncating instead of rounding would drop this to 1233.
		assert_eq!(secs_to_ms(1.2339), 1_234);
		assert_eq!(secs_to_ms(0.0), 0);
	}

	#[test]
	fn csv_drops_blanks_and_repeats_but_keeps_order() {
		assert_eq!(
			csv(Some("Ada Lovelace, , Grace Hopper,Ada Lovelace")),
			["Ada Lovelace", "Grace Hopper"]
		);
		assert!(csv(None).is_empty());
		assert!(csv(Some("  ,  ")).is_empty());
	}

	#[test]
	fn ignore_prefix_moves_a_leading_article_to_the_end() {
		// abs-ref's sortingPrefixes are ["the", "a"], case-insensitive.
		assert_eq!(ignore_prefix("The Hobbit"), "Hobbit, The");
		assert_eq!(ignore_prefix("a Study in Scarlet"), "Study in Scarlet, a");
		// No article: verbatim, as captured for "Analytical Engine".
		assert_eq!(ignore_prefix("Analytical Engine"), "Analytical Engine");
		// "Theory" starts with "The" but is not the article.
		assert_eq!(
			ignore_prefix("Theory of Everything"),
			"Theory of Everything"
		);
		assert_eq!(ignore_prefix("The"), "The");
	}

	#[test]
	fn last_first_matches_the_captured_author_name_lf() {
		assert_eq!(last_first("Ada Lovelace"), "Lovelace, Ada");
		assert_eq!(last_first("Various Narrators"), "Narrators, Various");
		assert_eq!(last_first("Ursula K. Le Guin"), "Guin, Ursula K. Le");
		assert_eq!(last_first("Plato"), "Plato");
	}

	#[test]
	fn series_name_matches_the_captured_shapes() {
		assert_eq!(series_name(None, None), "");
		assert_eq!(series_name(None, Some("1")), "");
		assert_eq!(
			series_name(Some(("id", "Compiler Chronicles")), Some("1")),
			"Compiler Chronicles #1"
		);
		assert_eq!(
			series_name(Some(("id", "Compiler Chronicles")), None),
			"Compiler Chronicles"
		);
	}

	#[test]
	fn a_finished_book_reports_full_progress_wherever_the_position_sits() {
		let progress = AbsProgress {
			position_ms: 4_500,
			track_index: Some(0),
			is_finished: true,
			started_at: Utc::now(),
			last_update: Utc::now(),
			finished_at: Some(Utc::now()),
		};
		let dto = progress_dto("user", "item", "book", 12_000, &progress);
		assert_eq!(dto.progress, 1.0);
		assert_eq!(dto.current_time, 4.5);
		assert_eq!(dto.duration, 12.0);
		assert!(dto.is_finished);
	}

	#[test]
	fn progress_is_a_clamped_fraction_and_survives_a_zero_duration() {
		let base = AbsProgress {
			position_ms: 6_000,
			track_index: None,
			is_finished: false,
			started_at: Utc::now(),
			last_update: Utc::now(),
			finished_at: None,
		};
		assert_eq!(progress_dto("u", "i", "b", 12_000, &base).progress, 0.5);
		// A position past the end must not report >1 to a client that draws
		// it as a progress bar.
		assert_eq!(progress_dto("u", "i", "b", 3_000, &base).progress, 1.0);
		// An unprobed row has no duration; dividing by it would be NaN.
		assert_eq!(progress_dto("u", "i", "b", 0, &base).progress, 0.0);
	}

	#[test]
	fn server_settings_pin_the_reference_release() {
		let settings = server_settings();
		assert_eq!(settings.version, "2.36.0");
		assert_eq!(settings.build_number, 1);
		assert_eq!(settings.auth_active_auth_methods, ["local"]);
		assert_eq!(settings.sorting_prefixes, ["the", "a"]);
	}
}
