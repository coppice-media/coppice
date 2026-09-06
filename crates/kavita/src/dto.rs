//! Serde contract types for the Kavita 0.9.1.4 (`develop`
//! `d77d956b9551227d8be2ee488b08f14aa3a341e5`) OpenAPI document.
//!
//! Field names, enum discriminants and default values follow
//! `openapi.json` and the responses captured from the `kavita-ref` container;
//! every struct keeps Kavita's field order so key diffs against the reference
//! are direct.

use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// A Kavita timestamp: .NET `DateTime` rendered without an offset and with
/// seven fractional digits (`2026-09-05T00:30:44.5870451`). The unset value
/// is `0001-01-01T00:00:00`, exactly as Kavita emits `default(DateTime)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KavitaDateTime(pub Option<DateTime<Utc>>);

impl KavitaDateTime {
	pub const UNSET: &'static str = "0001-01-01T00:00:00";

	pub fn new(value: impl Into<Option<DateTime<Utc>>>) -> Self {
		Self(value.into())
	}

	pub fn render(&self) -> String {
		match self.0 {
			Some(value) => format!(
				"{}.{:07}",
				value.format("%Y-%m-%dT%H:%M:%S"),
				value.nanosecond() / 100
			),
			None => Self::UNSET.to_owned(),
		}
	}
}

impl<Tz: chrono::TimeZone> From<DateTime<Tz>> for KavitaDateTime {
	fn from(value: DateTime<Tz>) -> Self {
		Self(Some(value.with_timezone(&Utc)))
	}
}

impl<Tz: chrono::TimeZone> From<Option<DateTime<Tz>>> for KavitaDateTime {
	fn from(value: Option<DateTime<Tz>>) -> Self {
		Self(value.map(|value| value.with_timezone(&Utc)))
	}
}

impl Serialize for KavitaDateTime {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.render())
	}
}

impl<'de> Deserialize<'de> for KavitaDateTime {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let raw = Option::<String>::deserialize(deserializer)?;
		let Some(raw) = raw else {
			return Ok(Self(None));
		};
		if raw.is_empty() || raw.starts_with("0001-01-01") {
			return Ok(Self(None));
		}
		if let Ok(value) = DateTime::parse_from_rfc3339(&raw) {
			return Ok(Self(Some(value.with_timezone(&Utc))));
		}
		let trimmed = raw.trim_end_matches('Z');
		let parsed = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f")
			.or_else(|_| NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S"))
			.map_err(|_| de::Error::custom(format!("invalid Kavita timestamp: {raw}")))?;
		Ok(Self(Some(parsed.and_utc())))
	}
}

/// A .NET `float`: whole values serialize without a fractional part
/// (`100000`, not `100000.0`), as `System.Text.Json` renders them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KavitaFloat(pub f32);

impl From<f32> for KavitaFloat {
	fn from(value: f32) -> Self {
		Self(value)
	}
}

impl Serialize for KavitaFloat {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let value = self.0;
		if value.is_finite() && value.fract() == 0.0 && value.abs() < 1.0e9 {
			serializer.serialize_i64(value as i64)
		} else {
			serializer.serialize_f32(value)
		}
	}
}

impl<'de> Deserialize<'de> for KavitaFloat {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		f32::deserialize(deserializer).map(Self)
	}
}

int_enum! {
	/// `LibraryType` (`Kavita.Models/Entities/Enums/LibraryType.cs`).
	LibraryType {
		Manga = 0,
		Comic = 1,
		Book = 2,
		Image = 3,
		LightNovel = 4,
		ComicVine = 5,
	}
}

int_enum! {
	/// `MangaFormat` (`Kavita.Models/Entities/Enums/MangaFormat.cs`).
	MangaFormat {
		Image = 0,
		Archive = 1,
		Unknown = 2,
		Epub = 3,
		Pdf = 4,
	}
}

impl MangaFormat {
	/// Kavita derives the format from the file extension
	/// (`Parser.ParseFormat`); Stump stores the extension without the dot.
	pub fn from_extension(extension: &str) -> Self {
		match extension
			.trim_start_matches('.')
			.to_ascii_lowercase()
			.as_str()
		{
			"cbz" | "cbr" | "cb7" | "cbt" | "zip" | "rar" | "7z" | "tar" => Self::Archive,
			"epub" => Self::Epub,
			"pdf" => Self::Pdf,
			"png" | "jpg" | "jpeg" | "webp" | "gif" | "avif" | "bmp" | "tiff" | "tif"
			| "jxl" | "heif" | "heic" => Self::Image,
			_ => Self::Unknown,
		}
	}
}

int_enum! {
	/// `FileTypeGroup` used by `LibraryDto.libraryFileTypes`.
	FileTypeGroup {
		Archive = 1,
		Epub = 2,
		Pdf = 3,
		Images = 4,
	}
}

int_enum! {
	/// `AgeRating` (`Kavita.Models/Entities/Enums/AgeRating.cs`).
	AgeRating {
		NotApplicable = -1,
		Unknown = 0,
		RatingPending = 1,
		EarlyChildhood = 2,
		Everyone = 3,
		G = 4,
		Everyone10Plus = 5,
		PG = 6,
		KidsToAdults = 7,
		Teen = 8,
		Mature15Plus = 9,
		Mature17Plus = 10,
		Mature = 11,
		R18Plus = 12,
		AdultsOnly = 13,
		X18Plus = 14,
	}
}

impl AgeRating {
	/// Display title as rendered by `GET /api/Metadata/age-ratings`.
	pub fn title(self) -> &'static str {
		match self {
			Self::NotApplicable => "Not Applicable",
			Self::Unknown => "Unknown",
			Self::RatingPending => "Rating Pending",
			Self::EarlyChildhood => "Early Childhood",
			Self::Everyone => "Everyone",
			Self::G => "G",
			Self::Everyone10Plus => "Everyone 10+",
			Self::PG => "PG",
			Self::KidsToAdults => "Kids to Adults",
			Self::Teen => "Teen",
			Self::Mature15Plus => "MA15+",
			Self::Mature17Plus => "Mature 17+",
			Self::Mature => "M",
			Self::R18Plus => "R18+",
			Self::AdultsOnly => "Adults Only 18+",
			Self::X18Plus => "X18+",
		}
	}

	/// Map Stump's numeric minimum age onto the closest Kavita rating.
	pub fn from_min_age(age: Option<i32>) -> Self {
		match age {
			None => Self::Unknown,
			Some(age) if age <= 0 => Self::Everyone,
			Some(1..=6) => Self::EarlyChildhood,
			Some(7..=9) => Self::Everyone,
			Some(10..=12) => Self::Everyone10Plus,
			Some(13..=14) => Self::Teen,
			Some(15..=16) => Self::Mature15Plus,
			Some(17) => Self::Mature17Plus,
			Some(_) => Self::R18Plus,
		}
	}
}

int_enum! {
	/// `PublicationStatus` (`Kavita.Models/Entities/Enums/PublicationStatus.cs`).
	PublicationStatus {
		OnGoing = 0,
		Hiatus = 1,
		Completed = 2,
		Cancelled = 3,
		Ended = 4,
	}
}

impl PublicationStatus {
	/// Display title as rendered by `GET /api/Metadata/publication-status`.
	pub fn title(self) -> &'static str {
		match self {
			Self::OnGoing => "Ongoing",
			Self::Hiatus => "Hiatus",
			Self::Completed => "Completed",
			Self::Cancelled => "Cancelled",
			Self::Ended => "Ended",
		}
	}

	/// Stump stores the free-form ComicInfo/series.json status text.
	pub fn from_status_text(status: Option<&str>) -> Self {
		let normalized = status
			.map(|status| {
				status
					.trim()
					.to_ascii_lowercase()
					.replace([' ', '-', '_'], "")
			})
			.unwrap_or_default();
		match normalized.as_str() {
			"completed" | "complete" | "finished" => Self::Completed,
			"ended" | "end" => Self::Ended,
			"cancelled" | "canceled" | "abandoned" => Self::Cancelled,
			"hiatus" | "onhiatus" | "paused" => Self::Hiatus,
			_ => Self::OnGoing,
		}
	}
}

int_enum! {
	/// `PersonRole`.
	PersonRole {
		Writer = 3,
		Penciller = 4,
		Inker = 5,
		Colorist = 6,
		Letterer = 7,
		CoverArtist = 8,
		Editor = 9,
		Publisher = 10,
		Character = 11,
		Translator = 12,
		Imprint = 13,
		Team = 14,
		Location = 15,
	}
}

int_enum! {
	/// `IdentityProvider`.
	IdentityProvider {
		Kavita = 0,
		OpenIdConnect = 1,
	}
}

int_enum! {
	/// `MetadataProvider` (Kavita+ providers; Stump reports none).
	MetadataProvider {
		Hardcover = 2,
		Mangabaka = 3,
		ComicBookRoundup = 4,
	}
}

int_enum! {
	/// `QueryContext`, accepted on `POST /api/Series/all-v2`.
	QueryContext {
		None = 1,
		Search = 2,
		Dashboard = 4,
	}
}

int_enum! {
	/// `ReadingProfileKind`; `Default` is the server-wide profile every user
	/// falls back to.
	ReadingProfileKind {
		Default = 0,
		User = 1,
		Implicit = 2,
	}
}

int_enum! {
	/// `ReadingDirection`.
	ReadingDirection {
		LeftToRight = 0,
		RightToLeft = 1,
	}
}

int_enum! {
	/// `ScalingOption`.
	ScalingOption {
		FitToHeight = 0,
		FitToWidth = 1,
		Original = 2,
		Automatic = 3,
	}
}

int_enum! {
	/// `PageSplitOption`.
	PageSplitOption {
		SplitLeftToRight = 0,
		SplitRightToLeft = 1,
		NoSplit = 2,
		FitSplit = 3,
	}
}

int_enum! {
	/// `ReaderMode`.
	ReaderMode {
		LeftRight = 0,
		UpDown = 1,
		Webtoon = 2,
	}
}

int_enum! {
	/// `LayoutMode`.
	LayoutMode {
		Single = 1,
		Double = 2,
		DoubleReversed = 3,
	}
}

int_enum! {
	/// `BreakPoint`: the viewport width below which a width override stops
	/// applying; the discriminant is the width in pixels.
	BreakPoint {
		Never = 0,
		Mobile = 768,
		Tablet = 1280,
		Desktop = 1440,
	}
}

int_enum! {
	/// `WritingStyle`.
	WritingStyle {
		Horizontal = 0,
		Vertical = 1,
	}
}

int_enum! {
	/// `BookPageLayoutMode`.
	BookPageLayoutMode {
		Default = 0,
		Column1 = 1,
		Column2 = 2,
	}
}

int_enum! {
	/// `PdfTheme`.
	PdfTheme {
		Dark = 0,
		Light = 1,
	}
}

int_enum! {
	/// `PdfScrollMode` (`2` is unused by Kavita).
	PdfScrollMode {
		Vertical = 0,
		Horizontal = 1,
		Page = 3,
	}
}

int_enum! {
	/// `PdfSpreadMode`.
	PdfSpreadMode {
		None = 0,
		Odd = 1,
		Even = 2,
	}
}

int_enum! {
	/// `ReadingListProvider`; Stump reading lists are always server-local.
	ReadingListProvider {
		None = 0,
		File = 1,
		Url = 2,
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoginDto {
	#[serde(default)]
	pub username: Option<String>,
	#[serde(default)]
	pub password: Option<String>,
	#[serde(default)]
	pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgeRestrictionDto {
	pub age_rating: AgeRating,
	pub include_unknowns: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthKeyDto {
	pub id: i32,
	pub key: String,
	pub name: String,
	pub created_at_utc: KavitaDateTime,
	pub expires_at_utc: Option<KavitaDateTime>,
	pub last_accessed_at_utc: Option<KavitaDateTime>,
	pub provider: i32,
}

/// `UserDto` as returned by `POST /api/Plugin/authenticate` and
/// `POST /api/Account/login`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
	pub id: i32,
	pub oidc_id: Option<String>,
	pub username: String,
	pub email: Option<String>,
	pub roles: Vec<String>,
	pub token: Option<String>,
	pub refresh_token: Option<String>,
	pub api_key: Option<String>,
	pub preferences: Option<serde_json::Value>,
	pub age_restriction: Option<AgeRestrictionDto>,
	pub kavita_version: Option<String>,
	pub identity_provider: IdentityProvider,
	pub created: KavitaDateTime,
	pub created_utc: KavitaDateTime,
	pub auth_keys: Option<Vec<AuthKeyDto>>,
	pub cover_image: Option<String>,
	pub primary_color: Option<String>,
	pub secondary_color: Option<String>,
}

/// The `UserPreferencesDto` Kavita ships for a fresh account, captured from
/// `kavita-ref` (`POST /api/Account/login`). Stump has no equivalent settings
/// object, so every client sees the Kavita defaults.
pub fn default_user_preferences() -> serde_json::Value {
	serde_json::json!({
		"theme": {
			"id": 1,
			"name": "Dark",
			"normalizedName": "dark",
			"fileName": "dark.scss",
			"isDefault": true,
			"provider": 1,
			"previewUrls": [""],
			"description": "Default theme shipped with Kavita",
			"author": null,
			"compatibleVersion": null,
			"selector": "bg-dark"
		},
		"globalPageLayoutMode": 0,
		"blurUnreadSummaries": false,
		"promptForDownloadSize": true,
		"noTransitions": false,
		"collapseSeriesRelationships": false,
		"locale": "en",
		"colorScapeEnabled": true,
		"dataSaver": false,
		"promptForRereadsAfter": 30,
		"customKeyBinds": {},
		"aniListScrobblingEnabled": false,
		"wantToReadSync": false,
		"bookReaderHighlightSlots": [
			{"id": 1, "slotNumber": 0, "color": {"r": 0, "g": 255, "b": 255, "a": 0.4}},
			{"id": 2, "slotNumber": 1, "color": {"r": 0, "g": 255, "b": 0, "a": 0.4}},
			{"id": 3, "slotNumber": 2, "color": {"r": 255, "g": 255, "b": 0, "a": 0.4}},
			{"id": 4, "slotNumber": 3, "color": {"r": 255, "g": 165, "b": 0, "a": 0.4}},
			{"id": 5, "slotNumber": 4, "color": {"r": 255, "g": 0, "b": 255, "a": 0.4}}
		],
		"socialPreferences": {
			"shareReviews": false,
			"shareAnnotations": false,
			"viewOtherAnnotations": false,
			"socialLibraries": [],
			"socialMaxAgeRating": -1,
			"socialIncludeUnknowns": true,
			"shareProfile": false
		},
		"opdsPreferences": {
			"embedProgressIndicator": true,
			"includeContinueFrom": true
		}
	})
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfoSlimDto {
	pub install_id: String,
	pub is_docker: bool,
	pub kavita_version: String,
	pub first_install_date: Option<KavitaDateTime>,
	pub first_install_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDto {
	pub id: i32,
	pub name: String,
	pub r#type: LibraryType,
	pub last_scanned: KavitaDateTime,
	pub cover_image: Option<String>,
	pub folder_watching: bool,
	pub include_in_dashboard: bool,
	pub include_in_recommended: bool,
	pub manage_collections: bool,
	pub manage_reading_lists: bool,
	pub include_in_search: bool,
	pub allow_scrobbling: bool,
	pub folders: Vec<String>,
	pub collapse_series_relationships: bool,
	pub library_file_types: Vec<FileTypeGroup>,
	pub exclude_patterns: Vec<String>,
	pub allow_metadata_matching: bool,
	pub enable_metadata: bool,
	pub remove_prefix_for_sort_name: bool,
	pub inherit_web_links_from_first_chapter: bool,
	pub default_language: Option<String>,
	pub metadata_provider: Option<MetadataProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDto {
	pub id: i32,
	pub name: String,
	pub original_name: String,
	pub localized_name: String,
	pub sort_name: String,
	pub pages: i32,
	pub cover_image_locked: bool,
	pub last_chapter_added: KavitaDateTime,
	pub last_chapter_added_utc: KavitaDateTime,
	pub user_rating: KavitaFloat,
	pub has_user_rated: bool,
	pub total_reads: i32,
	pub pages_read: i32,
	pub latest_read_date: KavitaDateTime,
	pub format: MangaFormat,
	pub created: KavitaDateTime,
	pub sort_name_locked: bool,
	pub localized_name_locked: bool,
	pub name_locked: bool,
	pub word_count: i64,
	pub library_id: i32,
	pub library_name: String,
	pub min_hours_to_read: i32,
	pub max_hours_to_read: i32,
	pub avg_hours_to_read: KavitaFloat,
	pub folder_path: String,
	pub lowest_folder_path: String,
	pub last_folder_scanned: KavitaDateTime,
	pub dont_match: bool,
	pub is_blacklisted: bool,
	pub is_stand_alone: bool,
	pub metadata_provider_override: Option<MetadataProvider>,
	pub cover_image: String,
	pub primary_color: Option<String>,
	pub secondary_color: Option<String>,
	pub ani_list_id: i32,
	pub mal_id: i64,
	pub hardcover_id: i32,
	pub metron_id: i64,
	pub comic_vine_id: Option<String>,
	pub manga_baka_id: i32,
	pub manga_baka_edition_id: Option<String>,
	pub cbr_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MangaFileDto {
	pub id: i32,
	pub file_path: String,
	pub pages: i32,
	pub bytes: i64,
	pub format: MangaFormat,
	pub created: KavitaDateTime,
	pub extension: String,
	pub koreader_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenreTagDto {
	pub id: i32,
	pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
	pub id: i32,
	pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgeRatingDto {
	pub value: AgeRating,
	pub title: String,
}

/// `GET /api/Metadata/publication-status` reuses the `{value, title}` shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PublicationStatusDto {
	pub value: PublicationStatus,
	pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LanguageDto {
	pub iso_code: String,
	pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonDto {
	pub id: i32,
	pub name: String,
	pub cover_image_locked: bool,
	pub primary_color: Option<String>,
	pub secondary_color: Option<String>,
	pub cover_image: Option<String>,
	pub aliases: Vec<String>,
	pub description: Option<String>,
	pub asin: Option<String>,
	pub ani_list_id: i32,
	pub mal_id: i64,
	pub hardcover_id: Option<String>,
	pub web_links: Vec<String>,
	pub roles: Vec<PersonRole>,
}

impl PersonDto {
	pub fn new(id: i32, name: String, roles: Vec<PersonRole>) -> Self {
		Self {
			id,
			name,
			cover_image_locked: false,
			primary_color: None,
			secondary_color: None,
			cover_image: None,
			aliases: Vec::new(),
			description: None,
			asin: None,
			ani_list_id: 0,
			mal_id: 0,
			hardcover_id: None,
			web_links: Vec::new(),
			roles,
		}
	}
}

/// The people/genre/tag/lock block shared by `ChapterDto` and
/// `SeriesMetadataDto`; both DTOs flatten it in Kavita's field order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeopleDto {
	pub writers: Vec<PersonDto>,
	pub cover_artists: Vec<PersonDto>,
	pub publishers: Vec<PersonDto>,
	pub characters: Vec<PersonDto>,
	pub pencillers: Vec<PersonDto>,
	pub inkers: Vec<PersonDto>,
	pub imprints: Vec<PersonDto>,
	pub colorists: Vec<PersonDto>,
	pub letterers: Vec<PersonDto>,
	pub editors: Vec<PersonDto>,
	pub translators: Vec<PersonDto>,
	pub teams: Vec<PersonDto>,
	pub locations: Vec<PersonDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetadataLocksDto {
	pub language_locked: bool,
	pub summary_locked: bool,
	pub age_rating_locked: bool,
	pub publication_status_locked: bool,
	pub genres_locked: bool,
	pub tags_locked: bool,
	pub writer_locked: bool,
	pub character_locked: bool,
	pub colorist_locked: bool,
	pub editor_locked: bool,
	pub inker_locked: bool,
	pub imprint_locked: bool,
	pub letterer_locked: bool,
	pub penciller_locked: bool,
	pub publisher_locked: bool,
	pub translator_locked: bool,
	pub team_locked: bool,
	pub location_locked: bool,
	pub cover_artist_locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDto {
	pub id: i32,
	pub range: String,
	pub number: String,
	pub min_number: KavitaFloat,
	pub max_number: KavitaFloat,
	pub sort_order: KavitaFloat,
	pub pages: i32,
	pub is_special: bool,
	pub title: String,
	pub files: Vec<MangaFileDto>,
	pub pages_read: i32,
	pub total_reads: i32,
	pub last_reading_progress_utc: KavitaDateTime,
	pub last_reading_progress: KavitaDateTime,
	pub cover_image_locked: bool,
	pub volume_id: i32,
	pub created_utc: KavitaDateTime,
	pub last_modified_utc: KavitaDateTime,
	pub created: KavitaDateTime,
	pub release_date: KavitaDateTime,
	pub title_name: String,
	pub summary: Option<String>,
	pub age_rating: AgeRating,
	pub word_count: i64,
	pub volume_title: String,
	pub min_hours_to_read: i32,
	pub max_hours_to_read: i32,
	pub avg_hours_to_read: KavitaFloat,
	pub web_links: String,
	pub isbn: String,
	#[serde(flatten)]
	pub people: PeopleDto,
	pub genres: Vec<GenreTagDto>,
	pub tags: Vec<TagDto>,
	pub publication_status: PublicationStatus,
	pub language: Option<String>,
	pub count: i32,
	pub total_count: i32,
	#[serde(flatten)]
	pub locks: MetadataLocksDto,
	pub release_date_locked: bool,
	pub title_name_locked: bool,
	pub sort_order_locked: bool,
	pub cover_image: String,
	pub primary_color: Option<String>,
	pub secondary_color: Option<String>,
	pub format: MangaFormat,
	pub ani_list_id: i32,
	pub mal_id: i64,
	pub hardcover_id: i32,
	pub metron_id: i64,
	pub comic_vine_id: Option<String>,
	pub manga_baka_id: i32,
	pub cbr_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VolumeDto {
	pub id: i32,
	pub min_number: KavitaFloat,
	pub max_number: KavitaFloat,
	pub name: String,
	pub number: i32,
	pub pages: i32,
	pub pages_read: i32,
	pub last_modified_utc: KavitaDateTime,
	pub created_utc: KavitaDateTime,
	pub created: KavitaDateTime,
	pub last_modified: KavitaDateTime,
	pub series_id: i32,
	pub chapters: Vec<ChapterDto>,
	pub min_hours_to_read: i32,
	pub max_hours_to_read: i32,
	pub avg_hours_to_read: KavitaFloat,
	pub word_count: i64,
	pub cover_image: String,
	pub primary_color: Option<String>,
	pub secondary_color: Option<String>,
	pub ani_list_id: i32,
	pub mal_id: i64,
	pub hardcover_id: i32,
	pub metron_id: i64,
	pub comic_vine_id: Option<String>,
	pub manga_baka_id: i32,
	pub cbr_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMetadataDto {
	pub id: i32,
	pub summary: String,
	pub genres: Vec<GenreTagDto>,
	pub tags: Vec<TagDto>,
	#[serde(flatten)]
	pub people: PeopleDto,
	pub age_rating: AgeRating,
	pub release_year: i32,
	pub language: String,
	pub max_count: i32,
	pub total_count: i32,
	pub publication_status: PublicationStatus,
	pub web_links: String,
	#[serde(flatten)]
	pub locks: MetadataLocksDto,
	pub release_year_locked: bool,
	pub series_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDetailDto {
	pub specials: Vec<ChapterDto>,
	pub chapters: Vec<ChapterDto>,
	pub volumes: Vec<VolumeDto>,
	pub storyline_chapters: Vec<ChapterDto>,
	pub library_type: LibraryType,
	pub unread_count: i32,
	pub total_count: i32,
}

/// `GET /api/Series/recently-updated-series` item: one series with the number
/// of chapters added to it recently (`GetRecentlyUpdatedSeriesAsync`).
/// `chapterId`/`volumeId` are `0` as captured from `kavita-ref`; `id` is the
/// 0-based group index Kavita assigns per response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupedSeriesDto {
	pub series_name: String,
	pub localized_series_name: String,
	pub series_id: i32,
	pub library_id: i32,
	pub library_type: LibraryType,
	pub created: KavitaDateTime,
	pub chapter_id: i32,
	pub volume_id: i32,
	pub id: i32,
	pub format: MangaFormat,
	pub count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
	pub volume_id: i32,
	pub chapter_id: i32,
	pub page_num: i32,
	pub series_id: i32,
	pub library_id: i32,
	#[serde(default)]
	pub book_scroll_id: Option<String>,
	#[serde(default)]
	pub last_modified_utc: KavitaDateTime,
}

impl ProgressDto {
	/// The record Kavita returns when the user has no progress on a chapter.
	pub fn empty(chapter_id: i32) -> Self {
		Self {
			volume_id: 0,
			chapter_id,
			page_num: 0,
			series_id: 0,
			library_id: 0,
			book_scroll_id: None,
			last_modified_utc: KavitaDateTime::default(),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarkReadDto {
	pub series_id: i32,
	#[serde(default)]
	pub generate_reading_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarkVolumeReadDto {
	pub series_id: i32,
	pub volume_id: i32,
	#[serde(default)]
	pub generate_reading_session: bool,
}

/// `MarkVolumesReadDto`, the body of `POST /api/Reader/mark-multiple-read`
/// and `mark-multiple-unread`: volumes and chapters of one series.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MarkVolumesReadDto {
	pub series_id: i32,
	#[serde(default)]
	pub volume_ids: Vec<i32>,
	#[serde(default)]
	pub chapter_ids: Vec<i32>,
	#[serde(default)]
	pub generate_reading_session: bool,
}

/// `MarkChapterReadDto`, the body of `POST /api/Reader/mark-chapter-read`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarkChapterReadDto {
	pub series_id: i32,
	pub chapter_id: i32,
	#[serde(default)]
	pub generate_reading_session: bool,
}

/// `FileDimensionDto`, one entry of `ChapterInfoDto.pageDimensions`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileDimensionDto {
	pub width: i32,
	pub height: i32,
	/// 0-based page number, matching `GET /api/Reader/image?page=`.
	pub page_number: i32,
	pub file_name: String,
	/// Kavita's `IsWide`: a landscape page, which the double reader pairs
	/// with itself rather than a neighbour.
	pub is_wide: bool,
}

/// `ChapterInfoDto` from `GET /api/Reader/chapter-info`; the reader's first
/// call. `pageDimensions`/`doublePairs` are `null` unless
/// `includeDimensions=true`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterInfoDto {
	pub chapter_number: String,
	pub volume_number: String,
	pub volume_id: i32,
	pub series_name: String,
	pub series_format: MangaFormat,
	pub series_id: i32,
	pub library_id: i32,
	pub library_type: LibraryType,
	pub chapter_title: String,
	pub pages: i32,
	pub file_name: String,
	pub is_special: bool,
	pub subtitle: String,
	pub title: String,
	pub series_total_pages: i32,
	pub series_total_pages_read: i32,
	pub page_dimensions: Option<Vec<FileDimensionDto>>,
	/// Page → the page it is displayed next to in the double reader, keyed by
	/// the page number rendered as a string (a .NET `Dictionary<int, int>`).
	pub double_pairs: Option<std::collections::BTreeMap<String, i32>>,
}

/// `BookInfoDto` from `GET /api/Book/{chapterId}/book-info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookInfoDto {
	pub book_title: String,
	pub series_id: i32,
	pub volume_id: i32,
	pub series_format: MangaFormat,
	pub series_name: String,
	pub chapter_number: String,
	pub volume_number: String,
	pub library_id: i32,
	pub pages: i32,
	pub is_special: bool,
	pub chapter_title: Option<String>,
}

/// `BookChapterItem` from `GET /api/Book/{chapterId}/chapters`: one EPUB
/// navigation entry. `part` is the anchor within the page, `page` the
/// `book-page` index the entry lives on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookChapterItemDto {
	pub title: String,
	pub part: String,
	pub page: i32,
	pub children: Vec<BookChapterItemDto>,
}

/// `UserReadingProfileDto` from `GET /api/reading-profile/{libraryId}/{seriesId}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserReadingProfileDto {
	pub id: i32,
	pub user_id: i32,
	pub name: String,
	pub kind: ReadingProfileKind,
	pub device_ids: Vec<i32>,
	pub series_ids: Vec<i32>,
	pub library_ids: Vec<i32>,
	pub reading_direction: ReadingDirection,
	pub scaling_option: ScalingOption,
	pub page_split_option: PageSplitOption,
	pub reader_mode: ReaderMode,
	pub auto_close_menu: bool,
	pub show_screen_hints: bool,
	pub emulate_book: bool,
	pub layout_mode: LayoutMode,
	pub background_color: String,
	pub swipe_to_paginate: bool,
	pub allow_automatic_webtoon_reader_detection: bool,
	pub width_override: Option<i32>,
	pub disable_width_override: BreakPoint,
	pub book_reader_margin: i32,
	pub book_reader_line_spacing: i32,
	pub book_reader_font_size: i32,
	pub book_reader_font_family: String,
	pub book_reader_tap_to_paginate: bool,
	pub book_reader_reading_direction: ReadingDirection,
	pub book_reader_writing_style: WritingStyle,
	pub book_reader_theme_name: String,
	pub book_reader_layout_mode: BookPageLayoutMode,
	pub book_reader_immersive_mode: bool,
	pub book_reader_disable_bookmark_icon: bool,
	pub pdf_theme: PdfTheme,
	pub pdf_scroll_mode: PdfScrollMode,
	pub pdf_spread_mode: PdfSpreadMode,
}

impl UserReadingProfileDto {
	/// Kavita's `Default Profile`, the one a fresh install serves for every
	/// library and series, copied field-for-field from `kavita-ref`
	/// (`GET /api/reading-profile/2/4?skipImplicit=false`). Stump stores no
	/// per-series reader settings, so this is the only profile it serves.
	pub fn default_profile() -> Self {
		Self {
			id: 1,
			user_id: 0,
			name: "Default Profile".to_owned(),
			kind: ReadingProfileKind::Default,
			device_ids: Vec::new(),
			series_ids: Vec::new(),
			library_ids: Vec::new(),
			reading_direction: ReadingDirection::LeftToRight,
			scaling_option: ScalingOption::Automatic,
			page_split_option: PageSplitOption::FitSplit,
			reader_mode: ReaderMode::LeftRight,
			auto_close_menu: true,
			show_screen_hints: true,
			emulate_book: false,
			layout_mode: LayoutMode::Single,
			background_color: "#000000".to_owned(),
			swipe_to_paginate: false,
			allow_automatic_webtoon_reader_detection: false,
			width_override: None,
			disable_width_override: BreakPoint::Never,
			book_reader_margin: 15,
			book_reader_line_spacing: 100,
			book_reader_font_size: 100,
			book_reader_font_family: "Default".to_owned(),
			book_reader_tap_to_paginate: false,
			book_reader_reading_direction: ReadingDirection::LeftToRight,
			book_reader_writing_style: WritingStyle::Horizontal,
			book_reader_theme_name: "Dark".to_owned(),
			book_reader_layout_mode: BookPageLayoutMode::Default,
			book_reader_immersive_mode: false,
			book_reader_disable_bookmark_icon: false,
			pdf_theme: PdfTheme::Dark,
			pdf_scroll_mode: PdfScrollMode::Vertical,
			pdf_spread_mode: PdfSpreadMode::None,
		}
	}
}

/// `UpdateWantToReadDto`, the body of `POST /api/want-to-read/add-series`
/// and `remove-series`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWantToReadDto {
	#[serde(default)]
	pub series_ids: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingListTagDto {
	pub id: i32,
	pub title: String,
	pub normalized_title: String,
}

/// `ReadingListDto`; Kavita's reading-list metadata block. Stump has no CBL
/// import, promotion, age rating or colour scape, so those stay at the values
/// `kavita-ref` reports for a freshly created list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingListDto {
	pub id: i32,
	pub title: String,
	pub summary: String,
	pub promoted: bool,
	pub cover_image_locked: bool,
	pub cover_image: Option<String>,
	pub primary_color: Option<String>,
	pub secondary_color: Option<String>,
	pub item_count: i32,
	pub starting_year: i32,
	pub starting_month: i32,
	pub ending_year: i32,
	pub ending_month: i32,
	pub age_rating: AgeRating,
	pub owner_user_name: String,
	pub source_path: Option<String>,
	pub download_url: Option<String>,
	pub sha_hash: Option<String>,
	pub provider: ReadingListProvider,
	pub last_sync_check_utc: Option<KavitaDateTime>,
	pub last_synced_utc: Option<KavitaDateTime>,
	pub total_items_at_import: i32,
	pub tags: Vec<ReadingListTagDto>,
	pub can_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingListItemChapterDto {
	pub id: i32,
	pub range: String,
	pub title_name: String,
	pub min_number: KavitaFloat,
	pub max_number: KavitaFloat,
	pub sort_order: KavitaFloat,
	pub pages: i32,
	pub is_special: bool,
	pub release_date: KavitaDateTime,
	pub summary: String,
	pub writer_name: Option<String>,
	pub writer_id: Option<i32>,
	pub penciller_name: Option<String>,
	pub penciller_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingListItemVolumeDto {
	pub id: i32,
	pub name: String,
	pub min_number: KavitaFloat,
	pub max_number: KavitaFloat,
	pub series_id: i32,
}

/// `ReadingListItemDto` from `GET /api/ReadingList/items`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingListItemDto {
	pub id: i32,
	pub order: i32,
	pub chapter_id: i32,
	pub series_id: i32,
	pub series_name: String,
	pub series_sort_name: String,
	pub series_format: MangaFormat,
	pub pages_read: i32,
	pub pages_total: i32,
	pub chapter_number: String,
	pub volume_number: String,
	pub chapter_title_name: String,
	pub volume_id: i32,
	pub library_id: i32,
	pub title: String,
	pub library_type: LibraryType,
	pub library_name: String,
	pub release_date: KavitaDateTime,
	pub reading_list_id: i32,
	pub last_reading_progress_utc: KavitaDateTime,
	pub file_size: i64,
	pub summary: String,
	pub is_special: bool,
	pub chapter: ReadingListItemChapterDto,
	pub volume: ReadingListItemVolumeDto,
}

/// `CreateReadingListDto`, the body of `POST /api/ReadingList/create`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateReadingListDto {
	#[serde(default)]
	pub title: String,
}

/// `UpdateReadingListBySeriesDto`, the body of
/// `POST /api/ReadingList/update-by-series`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReadingListBySeriesDto {
	pub series_id: i32,
	pub reading_list_id: i32,
}

/// `UpdateReadingListByChapterDto`/`UpdateReadingListByVolumeDto`: the same
/// shape, with the member id under `chapterId` or `volumeId`. A Stump media
/// item is both the Kavita volume and its chapter, so one type serves both
/// routes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReadingListByItemDto {
	pub series_id: i32,
	pub reading_list_id: i32,
	#[serde(default)]
	pub chapter_id: Option<i32>,
	#[serde(default)]
	pub volume_id: Option<i32>,
}

impl UpdateReadingListByItemDto {
	/// The Kavita volume/chapter id the body names, whichever field carries it.
	pub fn item_id(&self) -> Option<i32> {
		self.chapter_id.or(self.volume_id)
	}
}

/// `TachiyomiChapterDto`: a `ChapterDto` whose `number` may carry the encoded
/// volume number (`volume / 10000`) instead of a real chapter number.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TachiyomiChapterDto {
	#[serde(flatten)]
	pub chapter: ChapterDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SmartFilterDto {
	pub id: i32,
	pub name: String,
	pub filter: String,
	pub entity_type: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DecodeFilterDto {
	#[serde(default)]
	pub encoded_filter: Option<String>,
}

/// `SearchResultDto`: the series rows of `GET /api/Search/search`. A trimmed
/// series projection — Kavita's search does not build full `SeriesDto`s.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultDto {
	pub series_id: i32,
	pub name: String,
	pub original_name: String,
	pub sort_name: String,
	pub localized_name: String,
	pub format: MangaFormat,
	pub library_name: String,
	pub library_id: i32,
	pub release_year: i32,
	pub volume_count: i32,
	pub chapter_count: i32,
}

/// `BookmarkSearchResultDto`: the `bookmarks` group of a search response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkSearchResultDto {
	pub library_id: i32,
	pub volume_id: i32,
	pub series_id: i32,
	pub chapter_id: i32,
	pub series_name: String,
	pub localized_series_name: String,
}

/// `SearchResultGroupDto`: the grouped answer of `GET /api/Search/search`.
/// Every group is present even when empty, as `kavita-ref` renders it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultGroupDto {
	pub libraries: Vec<LibraryDto>,
	pub series: Vec<SearchResultDto>,
	pub collections: Vec<AppUserCollectionDto>,
	pub reading_lists: Vec<ReadingListDto>,
	pub persons: Vec<PersonDto>,
	pub genres: Vec<GenreTagDto>,
	pub tags: Vec<TagDto>,
	pub files: Vec<MangaFileDto>,
	pub chapters: Vec<ChapterDto>,
	pub bookmarks: Vec<BookmarkSearchResultDto>,
	pub annotations: Vec<AnnotationDto>,
}

/// `BookmarkDto`: one saved page. Serves as both the response shape of the
/// bookmark reads and the request body of `POST /api/Reader/bookmark` and
/// `unbookmark`, which is the single type Kavita uses for both directions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkDto {
	#[serde(default)]
	pub id: i32,
	#[serde(default)]
	pub page: i32,
	#[serde(default)]
	pub volume_id: i32,
	#[serde(default)]
	pub series_id: i32,
	#[serde(default)]
	pub chapter_id: i32,
	#[serde(default)]
	pub image_offset: i32,
	#[serde(default)]
	pub x_path: Option<String>,
	#[serde(default)]
	pub series: Option<SeriesDto>,
	#[serde(default)]
	pub chapter_title: Option<String>,
}

/// `AppUserCollectionDto`: a Kavita collection. Stump collections carry no
/// promotion, colours, cover lock or external source, so those fields are the
/// constants `kavita-ref` reports for a locally created collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppUserCollectionDto {
	pub id: i32,
	pub title: String,
	pub summary: String,
	pub promoted: bool,
	pub age_rating: AgeRating,
	pub cover_image: Option<String>,
	pub primary_color: Option<String>,
	pub secondary_color: Option<String>,
	pub cover_image_locked: bool,
	pub item_count: i32,
	pub owner: String,
	pub last_sync_utc: KavitaDateTime,
	/// `ScrobbleProvider`; `0` is Kavita itself, the only source Stump has.
	pub source: i32,
	pub source_url: Option<String>,
	pub total_source_count: i32,
	pub missing_series_from_source: Option<String>,
}

/// `CollectionTagBulkAddDto`: the body of `POST /api/Collection/update-for-series`.
/// `collectionTagId == 0` means "create a collection named `collectionTagTitle`".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionTagBulkAddDto {
	#[serde(default)]
	pub collection_tag_id: i32,
	#[serde(default)]
	pub collection_tag_title: Option<String>,
	#[serde(default)]
	pub series_ids: Vec<i32>,
}

/// The `tag` member of [`UpdateSeriesForTagDto`]. Kavita types it as a whole
/// `AppUserCollectionDto` but only reads its `id`; the rest of the object is
/// ignored, so this narrow view deserializes any client's payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRefDto {
	#[serde(default)]
	pub id: i32,
}

/// `UpdateSeriesForTagDto`: the body of `POST /api/Collection/update-series`,
/// which removes series from a collection.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSeriesForTagDto {
	#[serde(default)]
	pub tag: CollectionRefDto,
	#[serde(default)]
	pub series_ids_to_remove: Vec<i32>,
}

/// `RefreshSeriesDto`: the body of `POST /api/Series/{scan,analyze,refresh-metadata}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSeriesDto {
	#[serde(default)]
	pub library_id: i32,
	#[serde(default)]
	pub series_id: i32,
	#[serde(default)]
	pub force_update: bool,
	#[serde(default)]
	pub force_colorscape: bool,
}

/// `AnnotationDto`: a highlight and/or note. Stump stores the Readium locator
/// and one note per annotation, so the social, spoiler and slot fields of
/// Kavita's annotation model are the constants an unshared annotation has.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationDto {
	pub id: i32,
	pub x_path: String,
	pub ending_x_path: Option<String>,
	pub selected_text: Option<String>,
	pub comment: Option<String>,
	pub comment_html: Option<String>,
	pub comment_plain_text: Option<String>,
	pub chapter_title: Option<String>,
	pub context: Option<String>,
	pub highlight_count: i32,
	pub contains_spoiler: bool,
	pub page_number: i32,
	pub selected_slot_index: i32,
	pub likes: Vec<i32>,
	pub series_name: String,
	pub library_name: String,
	pub chapter_id: i32,
	pub volume_id: i32,
	pub series_id: i32,
	pub library_id: i32,
	pub owner_user_id: i32,
	pub owner_username: String,
	pub age_rating: AgeRating,
	pub created_utc: KavitaDateTime,
	pub last_modified_utc: KavitaDateTime,
}

/// `UserReadStatistics`: the answer of `GET /api/Stats/user/{userId}/read`.
///
/// `chaptersRead` and `lastActive` are not in `kavita-ref` 0.9.1.4 (which
/// renamed the route to `/api/Stats/user-read` and reports `lastActiveUtc`);
/// Inkita reads both, so both are served alongside the reference field set.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserReadStatisticsDto {
	pub total_pages_read: i64,
	pub total_words_read: i64,
	pub time_spent_reading: i64,
	pub chapters_read: i64,
	pub last_active: KavitaDateTime,
	pub last_active_utc: KavitaDateTime,
	pub avg_hours_per_week_spent_reading: f64,
}

/// `ReadHistoryEvent`: one row of `GET /api/Stats/user/reading-history`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadHistoryEventDto {
	pub user_id: i32,
	pub user_name: String,
	pub library_id: i32,
	pub series_id: i32,
	pub series_name: String,
	pub read_date: KavitaDateTime,
	pub read_date_utc: KavitaDateTime,
	pub chapter_id: i32,
	pub chapter_number: KavitaFloat,
}

/// The `Pagination` response header Kavita attaches to paged lists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaginationHeader {
	pub current_page: i32,
	pub items_per_page: i32,
	pub total_items: i32,
	pub total_pages: i32,
}

impl PaginationHeader {
	pub const NAME: &'static str = "Pagination";

	pub fn new(current_page: i32, items_per_page: i32, total_items: i32) -> Self {
		let total_pages = if items_per_page <= 0 {
			1
		} else {
			(f64::from(total_items) / f64::from(items_per_page)).ceil() as i32
		};
		Self {
			current_page,
			items_per_page,
			total_items,
			total_pages,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::TimeZone;

	#[test]
	fn datetime_renders_like_dotnet() {
		let value = Utc.with_ymd_and_hms(2026, 9, 5, 0, 30, 44).unwrap()
			+ chrono::Duration::nanoseconds(587_045_100);
		assert_eq!(
			KavitaDateTime::from(value).render(),
			"2026-09-05T00:30:44.5870451"
		);
		assert_eq!(KavitaDateTime::default().render(), "0001-01-01T00:00:00");
	}

	#[test]
	fn datetime_parses_dotnet_and_rfc3339() {
		let parsed: KavitaDateTime =
			serde_json::from_str("\"2026-09-05T00:30:44.5870451\"").unwrap();
		assert_eq!(parsed.render(), "2026-09-05T00:30:44.5870451");
		let parsed: KavitaDateTime =
			serde_json::from_str("\"2026-09-05T00:30:44.587Z\"").unwrap();
		assert_eq!(parsed.render(), "2026-09-05T00:30:44.5870000");
		let parsed: KavitaDateTime =
			serde_json::from_str("\"0001-01-01T00:00:00\"").unwrap();
		assert_eq!(parsed, KavitaDateTime::default());
		let parsed: KavitaDateTime = serde_json::from_str("null").unwrap();
		assert_eq!(parsed, KavitaDateTime::default());
	}

	#[test]
	fn floats_render_whole_values_as_integers() {
		assert_eq!(
			serde_json::to_string(&KavitaFloat(100000.0)).unwrap(),
			"100000"
		);
		assert_eq!(
			serde_json::to_string(&KavitaFloat(-100000.0)).unwrap(),
			"-100000"
		);
		assert_eq!(serde_json::to_string(&KavitaFloat(1.5)).unwrap(), "1.5");
	}

	#[test]
	fn int_enums_round_trip_discriminants() {
		assert_eq!(serde_json::to_string(&MangaFormat::Pdf).unwrap(), "4");
		assert_eq!(
			serde_json::from_str::<LibraryType>("5").unwrap(),
			LibraryType::ComicVine
		);
		assert!(serde_json::from_str::<MangaFormat>("9").is_err());
		assert_eq!(AgeRating::try_from(-1), Ok(AgeRating::NotApplicable));
	}

	#[test]
	fn manga_format_follows_extension() {
		assert_eq!(MangaFormat::from_extension("cbz"), MangaFormat::Archive);
		assert_eq!(MangaFormat::from_extension("CBR"), MangaFormat::Archive);
		assert_eq!(MangaFormat::from_extension("epub"), MangaFormat::Epub);
		assert_eq!(MangaFormat::from_extension(".pdf"), MangaFormat::Pdf);
		assert_eq!(MangaFormat::from_extension("png"), MangaFormat::Image);
		assert_eq!(MangaFormat::from_extension("mobi"), MangaFormat::Unknown);
	}

	#[test]
	fn pagination_header_matches_paged_list_math() {
		let header = PaginationHeader::new(1, 2, 3);
		assert_eq!(
			serde_json::to_string(&header).unwrap(),
			"{\"currentPage\":1,\"itemsPerPage\":2,\"totalItems\":3,\"totalPages\":2}"
		);
		assert_eq!(PaginationHeader::new(1, i32::MAX, 0).total_pages, 0);
	}
}
