//! Durable request automation invoked by the core job runtime.
//!
//! This module deliberately lives in `stump_core`, not GraphQL: the job
//! dispatcher must be able to resume the request lifecycle without creating a
//! transport/schema dependency cycle. Every invocation is bounded to one
//! search and at most one grab; a later invocation polls/retries the opaque
//! grab state.

use std::time::Duration;

use chrono::Utc;
use models::entity::{
	book_request, book_request_gateway_setting, book_request_grab, book_request_release,
	book_request_search,
};
use sea_orm::{
	ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
	Set,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{request_gateway::GatewayClient, CoreError, CoreResult, Ctx};

const REQUEST_GATEWAY_TIMEOUT: Duration = Duration::from_secs(15);

/// Process one approved, automation-enabled request.
///
/// No result is grabbed unless there is exactly one top-ranked release whose
/// total score reaches the request floor and whose verification components
/// reach the request threshold. Ties and empty/weak result sets transition to
/// `NEEDS_SELECTION` and await an explicit user selection.
pub async fn process_book_request(ctx: &Ctx, request_id: &str) -> CoreResult<()> {
	let conn = ctx.conn.as_ref();
	let request = book_request::Entity::find_by_id(request_id)
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("book request not found".into()))?;
	if !request.automation_enabled
		|| (request.approval_policy == "REQUIRED" && request.approved_by.is_none())
		|| !matches!(request.status.as_str(), "APPROVED" | "NEEDS_SELECTION")
	{
		return Ok(());
	}
	let settings = book_request_gateway_setting::Entity::find_by_id("default")
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("request gateway is not configured".into()))?;
	if !settings.enabled || !settings.automation_enabled {
		return Ok(());
	}
	let key = ctx.get_encryption_key().await?;
	let token =
		crate::utils::encryption::decrypt_string(&settings.encrypted_token, &key)?;
	let client = GatewayClient::new(&settings.endpoint, token, REQUEST_GATEWAY_TIMEOUT)
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	let query = request
		.authors
		.as_deref()
		.filter(|authors| !authors.trim().is_empty())
		.map(|authors| format!("{} {authors}", request.title))
		.unwrap_or_else(|| request.title.clone());
	let now = Utc::now().fixed_offset();
	let search_id = Uuid::new_v4().to_string();
	book_request_search::ActiveModel {
		id: Set(search_id.clone()),
		request_id: Set(request.id.clone()),
		query: Set(query.clone()),
		provider: Set("mam".into()),
		status: Set("SEARCHING".into()),
		result_count: Set(0),
		error_code: Set(None),
		error_message: Set(None),
		started_at: Set(Some(now)),
		finished_at: Set(None),
		created_at: Set(now),
	}
	.insert(conn)
	.await?;
	let mut request_active = request.clone().into_active_model();
	request_active.status = Set("SEARCHING".into());
	request_active.update(conn).await?;
	let releases = match client.search(&query).await {
		Ok(releases) => releases,
		Err(error) => {
			let message = error.to_string();
			let mut search = book_request_search::Entity::find_by_id(&search_id)
				.one(conn)
				.await?
				.ok_or_else(|| CoreError::NotFound("request search disappeared".into()))?
				.into_active_model();
			search.status = Set("FAILED".into());
			search.error_code = Set(Some("GATEWAY_SEARCH_FAILED".into()));
			search.error_message = Set(Some(message.clone()));
			search.finished_at = Set(Some(Utc::now().fixed_offset()));
			search.update(conn).await?;
			let mut request = request.into_active_model();
			request.status = Set("FAILED".into());
			request.failure_code = Set(Some("GATEWAY_SEARCH_FAILED".into()));
			request.failure_message = Set(Some(message));
			request.update(conn).await?;
			return Ok(());
		},
	};
	let mut scored = releases
		.into_iter()
		.map(|release| {
			let (score, components) = score_release(&request, &release);
			(score, release, components)
		})
		.collect::<Vec<_>>();
	scored.sort_by(|left, right| {
		right
			.0
			.cmp(&left.0)
			.then_with(|| left.1.remote_id.cmp(&right.1.remote_id))
	});
	for (rank, (score, release, components)) in scored.iter().enumerate() {
		book_request_release::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			search_id: Set(search_id.clone()),
			request_id: Set(request.id.clone()),
			source_provider: Set("mam".into()),
			remote_id: Set(release.remote_id.clone()),
			external_key: Set(release.external_key.clone()),
			title: Set(release.title.clone()),
			authors: Set(release.authors.clone()),
			format: Set(release.format.clone()),
			language: Set(release.language.clone()),
			edition: Set(release.edition.clone()),
			quality: Set(release.quality.clone()),
			size_bytes: Set(release.size_bytes),
			seeders: Set(release.seeders),
			preview_name: Set(release.preview_name.clone()),
			preview_mime: Set(release.preview_mime.clone()),
			preview_bytes: Set(release.preview_bytes),
			score: Set(*score),
			score_components: Set(components.clone()),
			rank: Set(rank as i32 + 1),
			selected: Set(false),
			created_at: Set(now),
		}
		.insert(conn)
		.await?;
	}
	let mut search = book_request_search::Entity::find_by_id(&search_id)
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("request search disappeared".into()))?
		.into_active_model();
	search.status = Set("COMPLETED".into());
	search.result_count = Set(scored.len() as i32);
	search.finished_at = Set(Some(Utc::now().fixed_offset()));
	search.update(conn).await?;
	let verification = scored
		.first()
		.map(|(_, _, components)| verification_score(components))
		.unwrap_or_default();
	let unique = scored
		.first()
		.is_some_and(|top| scored.get(1).is_none_or(|next| top.0 > next.0));
	let qualifies = scored
		.first()
		.is_some_and(|(score, _, _)| *score >= request.scoring_floor)
		&& verification >= request.verification_threshold
		&& unique;
	if !qualifies {
		let failure_message = format!(
			"top release did not uniquely meet score floor {} and verification threshold {}",
			request.scoring_floor, request.verification_threshold
		);
		let mut request = request.into_active_model();
		request.status = Set("NEEDS_SELECTION".into());
		request.failure_code = Set(Some(
			if scored.is_empty() {
				"NO_RELEASES"
			} else {
				"NO_UNIQUE_QUALIFYING_RELEASE"
			}
			.into(),
		));
		request.failure_message = Set(Some(failure_message));
		request.update(conn).await?;
		return Ok(());
	}
	let (_, release, _) = scored.first().expect("qualifying top release exists");
	let release = book_request_release::Entity::find()
		.filter(book_request_release::Column::RequestId.eq(request.id.clone()))
		.filter(book_request_release::Column::RemoteId.eq(release.remote_id.clone()))
		.order_by_desc(book_request_release::Column::CreatedAt)
		.one(conn)
		.await?
		.ok_or_else(|| CoreError::NotFound("automated release row disappeared".into()))?;
	book_request_release::Entity::update_many()
		.filter(book_request_release::Column::RequestId.eq(request.id.clone()))
		.set(book_request_release::ActiveModel {
			selected: Set(false),
			..Default::default()
		})
		.exec(conn)
		.await?;
	let mut selected = release.clone().into_active_model();
	selected.selected = Set(true);
	selected.update(conn).await?;
	let response = client
		.grab(&release.remote_id, &request.id)
		.await
		.map_err(|error| CoreError::BadRequest(error.to_string()))?;
	book_request_grab::ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		request_id: Set(request.id.clone()),
		release_id: Set(release.id),
		opaque_id: Set(response.opaque_id),
		status: Set(response.status),
		attempts: Set(1),
		max_attempts: Set(settings.max_retries.clamp(0, 10)),
		failure_code: Set(None),
		failure_message: Set(None),
		started_at: Set(Some(Utc::now().fixed_offset())),
		finished_at: Set(None),
		last_polled_at: Set(None),
		next_poll_at: Set(Some(now)),
		created_at: Set(now),
		updated_at: Set(now),
	}
	.insert(conn)
	.await?;
	let mut request = request.into_active_model();
	request.status = Set("GRABBED".into());
	request.failure_code = Set(None);
	request.failure_message = Set(None);
	request.update(conn).await?;
	Ok(())
}

fn score_release(
	request: &book_request::Model,
	release: &crate::request_gateway::GatewayRelease,
) -> (i32, Value) {
	let title_score = if request.title.eq_ignore_ascii_case(&release.title) {
		30
	} else if release
		.title
		.to_ascii_lowercase()
		.contains(&request.title.to_ascii_lowercase())
		|| request
			.title
			.to_ascii_lowercase()
			.contains(&release.title.to_ascii_lowercase())
	{
		24
	} else if words_overlap(&request.title, &release.title) {
		15
	} else {
		0
	};
	let author_score = match (&request.authors, &release.authors) {
		(Some(expected), Some(actual)) if expected.eq_ignore_ascii_case(actual) => 20,
		(Some(expected), Some(actual)) if words_overlap(expected, actual) => 14,
		_ => 0,
	};
	let format_score = i32::from(release.format.is_some()) * 10;
	let language_score = i32::from(release.language.is_some()) * 10;
	let edition_score = i32::from(release.edition.is_some()) * 10;
	let quality_score = i32::from(release.quality.is_some()) * 10;
	let size_score = i32::from(release.size_bytes.is_some()) * 5;
	let seeders_score = i32::from(release.seeders.unwrap_or_default() > 0) * 5;
	(
		title_score
			+ author_score
			+ format_score
			+ language_score
			+ edition_score
			+ quality_score
			+ size_score
			+ seeders_score,
		json!({"title": title_score, "author": author_score, "format": format_score, "language": language_score, "edition": edition_score, "quality": quality_score, "size": size_score, "seeders": seeders_score}),
	)
}

fn words_overlap(left: &str, right: &str) -> bool {
	let right = right.to_ascii_lowercase();
	left.to_ascii_lowercase()
		.split(|character: char| !character.is_ascii_alphanumeric())
		.filter(|word| word.len() > 1)
		.any(|word| {
			right
				.split(|character: char| !character.is_ascii_alphanumeric())
				.any(|candidate| candidate == word)
		})
}

fn verification_score(components: &Value) -> i32 {
	[
		"title", "author", "format", "language", "edition", "quality",
	]
	.into_iter()
	.filter_map(|key| components.get(key).and_then(Value::as_i64))
	.map(|value| value as i32)
	.sum()
}
