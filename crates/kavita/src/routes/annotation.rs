//! `AnnotationController` reads: the highlights and notes Inkita lists next
//! to a series.
//!
//! Stump's `media_annotations` are the same rows the native API and the
//! annotation sync surface expose: a Readium locator (whose `text.highlight`
//! is the selected passage) plus one note. Kavita models a richer, shareable
//! annotation — likes, spoiler flags, slot indexes, an ending anchor — none of
//! which Stump records, so those fields carry the values an unshared
//! annotation has. Inkita only ever reads annotations
//! (`GET api/Annotation/all-for-series`, `KavitaApi.kt:131`), so this profile
//! is read-only: `create`, `update`, `like`, `unlike`, `bulk-delete` and the
//! export routes stay unmounted and answer `404`.

use std::sync::Arc;

use axum::{extract::Query, routing::get, Extension, Json, Router};
use models::entity::{media_annotation, user, user::AuthUser};
use sea_orm::{prelude::*, QueryOrder};
use serde::Deserialize;
use stump_auth::AuthContext;

use crate::{
	dto::AnnotationDto,
	errors::APIResult,
	ids::{IdKind, KavitaIds, LOOKUP_CHUNK},
	mapper::map_annotation,
};

use super::{
	query::{find_series_input, group_by_media},
	route_ci, KavitaBackend,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesIdQuery {
	#[serde(default)]
	series_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterIdQuery {
	#[serde(default)]
	chapter_id: Option<i32>,
}

pub(crate) fn routes<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	let router = Router::<S>::new();
	let router = route_ci(
		router,
		"/api/Annotation/all-for-series",
		get(all_for_series),
	);
	route_ci(router, "/api/Annotation/all", get(all_for_chapter))
}

/// The user's annotations on `media_scope` (every media item when `None`),
/// mapped onto Kavita annotations in creation order.
pub(crate) async fn load_annotations(
	ctx: &dyn KavitaBackend,
	user: &AuthUser,
	media_scope: Option<&[String]>,
) -> APIResult<Vec<AnnotationDto>> {
	let mut rows = Vec::new();
	match media_scope {
		Some(media_ids) if media_ids.is_empty() => return Ok(Vec::new()),
		Some(media_ids) => {
			for chunk in media_ids.chunks(LOOKUP_CHUNK) {
				rows.extend(
					annotation_query(user)
						.filter(media_annotation::Column::MediaId.is_in(chunk.to_vec()))
						.all(ctx.conn())
						.await?,
				);
			}
			rows.sort_by(|left, right| {
				left.created_at
					.cmp(&right.created_at)
					.then_with(|| left.id.cmp(&right.id))
			});
		},
		None => rows.extend(annotation_query(user).all(ctx.conn()).await?),
	}
	if rows.is_empty() {
		return Ok(Vec::new());
	}
	let ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
	let kavita_ids =
		KavitaIds::resolve_many(ctx.conn(), IdKind::Annotation, &ids).await?;
	let owner_id = KavitaIds::resolve(ctx.conn(), IdKind::User, &user.id).await?;
	let owner_username = user::Entity::find_by_id(user.id.clone())
		.one(ctx.conn())
		.await?
		.map(|row| row.username)
		.unwrap_or_else(|| user.username.clone());
	let grouped = group_by_media(ctx, user, rows, |row| row.media_id.as_str()).await?;
	Ok(grouped
		.rows
		.iter()
		.map(|(annotation, series_index, media_index)| {
			let input = &grouped.inputs[*series_index];
			map_annotation(
				kavita_ids[&annotation.id],
				annotation,
				input,
				&input.media[*media_index],
				owner_id,
				owner_username.clone(),
			)
		})
		.collect())
}

fn annotation_query(user: &AuthUser) -> Select<media_annotation::Entity> {
	media_annotation::Entity::find()
		.filter(media_annotation::Column::UserId.eq(user.id.clone()))
		.order_by_asc(media_annotation::Column::CreatedAt)
		.order_by_asc(media_annotation::Column::Id)
}

/// `GET /api/Annotation/all-for-series?seriesId`. Kavita answers `200` with an
/// empty list for an unknown or inaccessible series.
async fn all_for_series(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<SeriesIdQuery>,
) -> APIResult<Json<Vec<AnnotationDto>>> {
	let user = auth.user();
	let Some(input) =
		find_series_input(ctx.as_ref(), &user, query.series_id.unwrap_or_default())
			.await?
	else {
		return Ok(Json(Vec::new()));
	};
	let media_ids = input
		.media
		.iter()
		.map(|media| media.media.id.clone())
		.collect::<Vec<_>>();
	Ok(Json(
		load_annotations(ctx.as_ref(), &user, Some(&media_ids)).await?,
	))
}

/// `GET /api/Annotation/all?chapterId`: the annotations of one chapter.
async fn all_for_chapter(
	Extension(ctx): Extension<Arc<dyn KavitaBackend>>,
	Extension(auth): Extension<AuthContext>,
	Query(query): Query<ChapterIdQuery>,
) -> APIResult<Json<Vec<AnnotationDto>>> {
	let user = auth.user();
	let Some((input, index)) = super::query::find_media(
		ctx.as_ref(),
		&user,
		query.chapter_id.unwrap_or_default(),
	)
	.await?
	else {
		return Ok(Json(Vec::new()));
	};
	let media_ids = vec![input.media[index].media.id.clone()];
	Ok(Json(
		load_annotations(ctx.as_ref(), &user, Some(&media_ids)).await?,
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::{
		auth_user, db, library_of_type, request, series_with_files, TestBackend,
	};
	use ::tests::fake_data;
	use axum::http::StatusCode;
	use models::shared::enums::LibraryType as StumpLibraryType;
	use models::shared::readium::{ReadiumLocation, ReadiumLocator, ReadiumText};
	use sea_orm::ActiveValue::Set;

	fn locator(fragment: &str, highlight: &str) -> ReadiumLocator {
		ReadiumLocator {
			chapter_title: "Chapter One".to_owned(),
			href: "OEBPS/ch1.xhtml".to_owned(),
			title: None,
			locations: Some(ReadiumLocation {
				fragments: Some(vec![fragment.to_owned()]),
				progression: None,
				position: Some(7),
				total_progression: None,
				css_selector: None,
				partial_cfi: None,
			}),
			text: Some(ReadiumText {
				before: Some("before ".to_owned()),
				highlight: Some(highlight.to_owned()),
				after: Some(" after".to_owned()),
			}),
			..Default::default()
		}
	}

	/// A read-only projection of `media_annotations`: the highlight becomes
	/// `selectedText`, the note becomes the three `comment*` fields, and the
	/// locator fragment becomes `xPath`.
	#[tokio::test]
	async fn annotations_project_media_annotations_per_series_and_chapter() {
		let conn = db().await;
		let user_row = fake_data::User::new("annotator").insert(&conn).await;
		let user = auth_user(&user_row);
		let library = library_of_type(&conn, StumpLibraryType::Comic).await;
		let (series_row, files) = series_with_files(
			&conn,
			&library.id,
			"Zeta",
			&[("v01", "epub", 20), ("v02", "epub", 30)],
		)
		.await;
		models::entity::media_annotation::ActiveModel {
			locator: Set(locator("//p[1]", "the highlighted passage")),
			annotation_text: Set(Some("my note".to_owned())),
			media_id: Set(files[0].id.clone()),
			user_id: Set(user_row.id.clone()),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();
		models::entity::media_annotation::ActiveModel {
			locator: Set(locator("//p[9]", "second")),
			annotation_text: Set(None),
			media_id: Set(files[1].id.clone()),
			user_id: Set(user_row.id.clone()),
			..Default::default()
		}
		.insert(&conn)
		.await
		.unwrap();

		let backend = std::sync::Arc::new(TestBackend::new(conn));
		let series_id =
			KavitaIds::resolve(backend.conn(), IdKind::Series, &series_row.id)
				.await
				.unwrap();
		let first_chapter =
			KavitaIds::resolve(backend.conn(), IdKind::Media, &files[0].id)
				.await
				.unwrap();

		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/Annotation/all-for-series?seriesId={series_id}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let items = body.as_array().unwrap();
		assert_eq!(items.len(), 2);
		let first = &items[0];
		assert_eq!(first["xPath"], "//p[1]");
		assert_eq!(first["selectedText"], "the highlighted passage");
		assert_eq!(first["comment"], "my note");
		assert_eq!(first["commentPlainText"], "my note");
		assert_eq!(first["chapterTitle"], "Chapter One");
		assert_eq!(first["context"], "before the highlighted passage after");
		assert_eq!(first["pageNumber"], 7);
		assert_eq!(first["seriesId"], series_id);
		assert_eq!(first["chapterId"], first["volumeId"]);
		assert_eq!(first["ownerUsername"], "annotator");
		assert_eq!(first["highlightCount"], 1);
		assert_eq!(first["containsSpoiler"], false);
		assert!(first["likes"].as_array().unwrap().is_empty());
		// A highlight with no note carries no comment.
		assert!(items[1]["comment"].is_null());

		// The chapter form narrows to one file.
		let (status, body) = request(
			backend.clone(),
			&user,
			"GET",
			&format!("/api/annotation/all?chapterId={first_chapter}"),
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body.as_array().unwrap().len(), 1);
		assert_eq!(body[0]["chapterId"], first_chapter);

		// Another user sees none of them, and an unknown series is empty.
		let other = fake_data::User::new("stranger")
			.insert(backend.conn())
			.await;
		let (_, body) = request(
			backend.clone(),
			&auth_user(&other),
			"GET",
			&format!("/api/Annotation/all-for-series?seriesId={series_id}"),
			None,
		)
		.await;
		assert!(body.as_array().unwrap().is_empty());
		let (status, body) = request(
			backend,
			&user,
			"GET",
			"/api/Annotation/all-for-series?seriesId=999999",
			None,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert!(body.as_array().unwrap().is_empty());
	}
}
