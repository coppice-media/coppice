//! `ReadingProfileController`: the reader settings a client applies.
//!
//! Kavita 0.8.7+ resolves a profile per series by walking series → library →
//! default. Stump stores no per-user reader settings for the Kavita surface,
//! so the walk always lands on Kavita's own `Default Profile`; the response is
//! the `kavita-ref` default copied field-for-field
//! ([`UserReadingProfileDto::default_profile`]).

use axum::{extract::Path, routing::get, Json, Router};

use crate::{dto::UserReadingProfileDto, errors::APIResult};

use super::route_ci;

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	route_ci(
		Router::<S>::new(),
		"/api/reading-profile/{libraryId}/{seriesId}",
		get(reading_profile_for_series),
	)
}

/// `GET /api/reading-profile/{libraryId}/{seriesId}`.
///
/// `skipImplicit` and `deviceId` change nothing: there is no implicit or
/// device-scoped profile to skip, and `kind` is `Default`, which is what
/// Kamigura keys its "no per-series direction, use the app setting" branch on
/// (`reader/ReaderScreen.kt:557-560`).
async fn reading_profile_for_series(
	Path((_library_id, _series_id)): Path<(i32, i32)>,
) -> APIResult<Json<UserReadingProfileDto>> {
	Ok(Json(UserReadingProfileDto::default_profile()))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The served profile is `kavita-ref`'s `Default Profile`, byte for byte:
	/// Kamigura only reads `kind` and `readingDirection`, but the whole object
	/// is a copy so a stricter client cannot trip on a missing field.
	#[tokio::test]
	async fn default_profile_matches_the_reference_json() {
		let Json(profile) = reading_profile_for_series(Path((2, 4))).await.unwrap();
		let json = serde_json::to_value(&profile).unwrap();
		assert_eq!(
			json,
			serde_json::json!({
				"id": 1,
				"userId": 0,
				"name": "Default Profile",
				"kind": 0,
				"deviceIds": [],
				"seriesIds": [],
				"libraryIds": [],
				"readingDirection": 0,
				"scalingOption": 3,
				"pageSplitOption": 3,
				"readerMode": 0,
				"autoCloseMenu": true,
				"showScreenHints": true,
				"emulateBook": false,
				"layoutMode": 1,
				"backgroundColor": "#000000",
				"swipeToPaginate": false,
				"allowAutomaticWebtoonReaderDetection": false,
				"widthOverride": null,
				"disableWidthOverride": 0,
				"bookReaderMargin": 15,
				"bookReaderLineSpacing": 100,
				"bookReaderFontSize": 100,
				"bookReaderFontFamily": "Default",
				"bookReaderTapToPaginate": false,
				"bookReaderReadingDirection": 0,
				"bookReaderWritingStyle": 0,
				"bookReaderThemeName": "Dark",
				"bookReaderLayoutMode": 0,
				"bookReaderImmersiveMode": false,
				"bookReaderDisableBookmarkIcon": false,
				"pdfTheme": 0,
				"pdfScrollMode": 0,
				"pdfSpreadMode": 0
			})
		);
	}
}
