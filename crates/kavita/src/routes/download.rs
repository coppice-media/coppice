//! `DownloadController`: the size clients quote before downloading a chapter.

use std::sync::Arc;

use axum::{extract::Query, routing::get, Extension, Json, Router};
use models::entity::user::AuthUser;
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::errors::APIResult;

use super::{query::find_media, route_ci, KavitaBackend};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	route_ci(
		Router::<S>::new(),
		"/api/Download/chapter-size",
		get(chapter_size),
	)
}

/// `DownloadController.GetChapterSize`: the chapter's size in bytes. Kavita
/// sums the sizes of the chapter's files and answers `200` with `0` for a
/// chapter it cannot resolve (`kavita-ref` `chapterId=9999` → `0`), so an
/// unknown or invisible chapter is not an error here either.
pub(crate) async fn chapter_size_for(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	chapter_id: i32,
) -> APIResult<i64> {
	let Some((input, index)) = find_media(ctx, user, chapter_id).await? else {
		return Ok(0);
	};
	Ok(input.media[index].media.size)
}

async fn chapter_size(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterQuery>,
) -> APIResult<Json<i64>> {
	let user = auth.user();
	Ok(Json(
		chapter_size_for(ctx.as_ref(), &user, query.chapter_id.unwrap_or_default())
			.await?,
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ids::{IdKind, KavitaIds};
	use crate::routes::series::list_volumes;
	use crate::test_support::{
		auth_user, db, library_of_type, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use models::shared::enums::LibraryType as StumpLibraryType;
	use sea_orm::{ActiveModelTrait, ActiveValue::Set};

	/// `chapter-size` reports the media's byte size, and `0` — not an error —
	/// for a chapter id that resolves to nothing.
	#[tokio::test]
	async fn chapter_size_is_the_media_file_size_in_bytes() {
		let conn = db().await;
		let user_row = fake_data::User::new("sizer").insert(&conn).await;
		let user = auth_user(&user_row);
		let backend = TestBackend::new(conn);
		let library = library_of_type(&backend.conn, StumpLibraryType::Comic).await;
		let (series, files) = series_with_files(
			&backend.conn,
			&library.id,
			"Science Comics",
			&[("science_comics_001", "cbz", 36)],
		)
		.await;
		models::entity::media::ActiveModel {
			size: Set(3_522_110),
			..files[0].clone().into()
		}
		.update(&backend.conn)
		.await
		.unwrap();
		let series_id = KavitaIds::resolve(&backend.conn, IdKind::Series, &series.id)
			.await
			.unwrap();
		let volumes = list_volumes(&backend, &user, series_id).await.unwrap();
		let chapter_id = volumes[0].chapters[0].id;

		assert_eq!(
			chapter_size_for(&backend, &user, chapter_id).await.unwrap(),
			3_522_110
		);
		assert_eq!(
			chapter_size_for(&backend, &user, 9999).await.unwrap(),
			0,
			"an unresolvable chapter is 0 bytes, not a 404"
		);
	}
}
