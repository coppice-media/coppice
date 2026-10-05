use async_graphql::{InputObject, OneofObject};
use models::{
	domain::reading_state::time_progression,
	entity::{bookmark, media_annotation, media_metadata, user::AuthUser},
	services::audio,
	shared::{
		enums::MetadataProvider,
		readium::{ReadiumLocation, ReadiumLocator},
	},
};
use sea_orm::{prelude::*, ActiveValue::Set, IntoActiveModel};

#[derive(Debug, Clone, InputObject)]
pub struct EpubProgressInput {
	pub locator: ReadiumLocator,
	pub percentage: Option<Decimal>,
	pub is_complete: Option<bool>,
	pub elapsed_seconds_delta: Option<i64>,
	pub device_id: Option<String>,
	pub reset_elapsed_seconds: Option<bool>,
}

#[derive(Default, Debug, Clone, InputObject)]
pub struct PagedProgressInput {
	pub page: i32,
	pub elapsed_seconds_delta: Option<i64>,
	pub device_id: Option<String>,
	pub reset_elapsed_seconds: Option<bool>,
}

/// A listening position: milliseconds from the start of the publication,
/// plus the file the client was in for a multi-file audiobook. A recording
/// has no pages, so this carries neither a page nor a locator.
#[derive(Default, Debug, Clone, InputObject)]
pub struct AudioProgressInput {
	pub position_ms: i64,
	/// The 0-based `mediaAudioTracks.index` the position fell in.
	pub track_index: Option<i32>,
	pub is_complete: Option<bool>,
	pub elapsed_seconds_delta: Option<i64>,
	pub device_id: Option<String>,
	pub reset_elapsed_seconds: Option<bool>,
}

#[derive(Debug, Clone, OneofObject)]
pub enum MediaProgressInput {
	Epub(Box<EpubProgressInput>),
	Paged(PagedProgressInput),
	Audio(AudioProgressInput),
}

impl MediaProgressInput {
	/// whether applying the input should first reset the elapsed seconds for the media progress
	pub fn reset_elapsed_seconds(&self) -> bool {
		match self {
			MediaProgressInput::Epub(epub_progress) => {
				epub_progress.reset_elapsed_seconds
			},
			MediaProgressInput::Paged(paged_progress) => {
				paged_progress.reset_elapsed_seconds
			},
			MediaProgressInput::Audio(audio_progress) => {
				audio_progress.reset_elapsed_seconds
			},
		}
		.unwrap_or(false)
	}
}

#[derive(InputObject)]
pub struct BookmarkInput {
	pub media_id: String,
	pub locator: ReadiumLocator,
	pub preview_content: Option<String>,
}

impl BookmarkInput {
	pub fn into_active_model(&self, user: &AuthUser) -> bookmark::ActiveModel {
		bookmark::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			locator: Set(Some(self.locator.clone())),
			preview_content: Set(self.preview_content.clone()),
			media_id: Set(self.media_id.clone()),
			user_id: Set(user.id.clone()),
			page: Set(Some(-1)),
			..Default::default()
		}
	}
}

#[derive(Debug, Clone, InputObject)]
pub struct MediaMetadataInput {
	pub title: Option<String>,
	pub title_sort: Option<String>,
	pub series: Option<String>,
	pub series_group: Option<String>,
	pub story_arc: Option<String>,
	pub story_arc_number: Option<Decimal>,
	pub number: Option<Decimal>,
	pub volume: Option<i32>,
	pub summary: Option<String>,
	pub notes: Option<String>,
	pub genres: Option<Vec<String>>,
	pub format: Option<String>,
	pub year: Option<i32>,
	pub month: Option<i32>,
	pub day: Option<i32>,
	pub writers: Option<Vec<String>>,
	pub pencillers: Option<Vec<String>>,
	pub inkers: Option<Vec<String>>,
	pub colorists: Option<Vec<String>>,
	pub letterers: Option<Vec<String>>,
	pub cover_artists: Option<Vec<String>>,
	pub editors: Option<Vec<String>>,
	/// The audiobook's readers. A separate credit from `writers`: an
	/// audiobook's author wrote it and its narrator did not.
	pub narrators: Option<Vec<String>>,
	pub publisher: Option<String>,
	pub links: Option<Vec<String>>,
	pub characters: Option<Vec<String>>,
	pub teams: Option<Vec<String>>,
	pub page_count: Option<i32>,
	pub age_rating: Option<i32>,
	pub identifier_amazon: Option<String>,
	pub identifier_calibre: Option<String>,
	pub identifier_google: Option<String>,
	pub identifier_isbn: Option<String>,
	pub identifier_mobi_asin: Option<String>,
	pub identifier_uuid: Option<String>,
	pub language: Option<String>,
}

impl IntoActiveModel<media_metadata::ActiveModel> for MediaMetadataInput {
	fn into_active_model(self) -> media_metadata::ActiveModel {
		media_metadata::ActiveModel {
			title: Set(self.title),
			title_sort: Set(self.title_sort),
			series: Set(self.series),
			series_group: Set(self.series_group),
			story_arc: Set(self.story_arc),
			story_arc_number: Set(self.story_arc_number),
			number: Set(self.number),
			volume: Set(self.volume),
			summary: Set(self.summary),
			notes: Set(self.notes),
			genres: Set(into_array_string(self.genres)),
			format: Set(self.format),
			year: Set(self.year),
			month: Set(self.month),
			day: Set(self.day),
			writers: Set(into_array_string(self.writers)),
			pencillers: Set(into_array_string(self.pencillers)),
			inkers: Set(into_array_string(self.inkers)),
			colorists: Set(into_array_string(self.colorists)),
			letterers: Set(into_array_string(self.letterers)),
			cover_artists: Set(into_array_string(self.cover_artists)),
			editors: Set(into_array_string(self.editors)),
			narrators: Set(into_array_string(self.narrators)),
			publisher: Set(self.publisher),
			links: Set(into_array_string(self.links)),
			characters: Set(into_array_string(self.characters)),
			teams: Set(into_array_string(self.teams)),
			page_count: Set(self.page_count),
			age_rating: Set(self.age_rating),
			identifier_amazon: Set(self.identifier_amazon),
			identifier_calibre: Set(self.identifier_calibre),
			identifier_google: Set(self.identifier_google),
			identifier_isbn: Set(self.identifier_isbn),
			identifier_mobi_asin: Set(self.identifier_mobi_asin),
			identifier_uuid: Set(self.identifier_uuid),
			language: Set(self.language),
			..Default::default()
		}
	}
}

fn into_array_string(s: Option<Vec<String>>) -> Option<String> {
	match s {
		Some(v) if !v.is_empty() => Some(v.join(", ")),
		_ => None,
	}
}

/// A new annotation, anchored by exactly one of `locator` (text or a page)
/// and `positionMs` (a moment in an audiobook).
#[derive(Debug, Clone, InputObject)]
pub struct CreateAnnotationInput {
	pub media_id: String,
	/// The Readium anchor: a text selection in an EPUB, or the
	/// `locations.position` page of a comic or PDF.
	pub locator: Option<ReadiumLocator>,
	/// A moment in an audiobook: milliseconds from the start of the
	/// publication, within `0..=durationMs`. Rejected on any other media.
	pub position_ms: Option<i64>,
	pub annotation_text: Option<String>,
}

impl CreateAnnotationInput {
	/// The row this input creates, validated against the media it names.
	pub async fn into_active_model<C: ConnectionTrait>(
		self,
		conn: &C,
		user: &AuthUser,
	) -> async_graphql::Result<media_annotation::ActiveModel> {
		let locator = match (self.locator, self.position_ms) {
			(Some(locator), None) => locator,
			(None, Some(position_ms)) => {
				audio_annotation_locator(conn, &self.media_id, position_ms).await?
			},
			(Some(_), Some(_)) => {
				return Err("An annotation takes a locator or positionMs, not both".into())
			},
			(None, None) => {
				return Err("An annotation needs a locator or positionMs".into())
			},
		};
		Ok(media_annotation::ActiveModel {
			locator: Set(locator),
			position_ms: Set(self.position_ms),
			annotation_text: Set(self.annotation_text),
			media_id: Set(self.media_id),
			user_id: Set(user.id.clone()),
			..Default::default()
		})
	}
}

/// The locator stored beside a time anchor. A moment in a recording has no
/// resource to point into, so `href` is empty and there is no `text`, which
/// every lane needing a Readium anchor already treats as unanchored. It
/// carries what an annotation list shows: the chapter the moment falls in
/// and the whole-publication progression.
async fn audio_annotation_locator<C: ConnectionTrait>(
	conn: &C,
	media_id: &str,
	position_ms: i64,
) -> async_graphql::Result<ReadiumLocator> {
	let duration_ms = audio::duration_ms(conn, media_id)
		.await?
		.ok_or("positionMs only applies to an audiobook")?;
	if !(0..=duration_ms).contains(&position_ms) {
		return Err(format!(
			"positionMs must be within the audiobook (0 to {duration_ms} ms)"
		)
		.into());
	}
	let chapter = audio::chapter_at(conn, media_id, position_ms).await?;
	Ok(ReadiumLocator {
		chapter_title: chapter
			.and_then(|chapter| chapter.title)
			.unwrap_or_default(),
		href: String::new(),
		title: None,
		locations: Some(ReadiumLocation {
			fragments: None,
			progression: None,
			position: None,
			total_progression: time_progression(position_ms, duration_ms)
				.and_then(Decimal::from_f64_retain),
			css_selector: None,
			partial_cfi: None,
		}),
		text: None,
		kobo_span: None,
		r#type: String::new(),
	})
}

#[derive(Debug, Clone, InputObject)]
pub struct UpdateAnnotationInput {
	pub id: String,
	pub annotation_text: Option<String>,
	pub color: Option<String>,
	pub expected_revision: Option<i64>,
}

/// A manual override for searching metadata providers for a single media item. When
/// provided, the caller's fields take precedence over whatever is already stored on the
/// media, and the search is restricted to a single provider if one is given.
#[derive(Debug, Clone, Default, InputObject)]
pub struct MediaMetadataSearchInput {
	pub title: Option<String>,
	pub author: Option<String>,
	pub isbn: Option<String>,
	pub year: Option<i32>,
	/// The issue number (for comics/manga)
	pub number: Option<f64>,
	/// The volume ID to search within, which will swap to a more precise lookup if provided alongside
	/// `number`
	pub comic_vine_volume_id: Option<String>,
	/// Restrict the search to this provider only. If omitted, all enabled providers
	/// configured for the media's library type are searched.
	pub provider: Option<MetadataProvider>,
	pub limit: Option<i32>,
}
