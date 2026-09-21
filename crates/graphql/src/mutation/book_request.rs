use std::{path::Path, time::Duration};

use async_graphql::{Context, Object, Result, ID};
use chrono::Utc;
use models::entity::{
	book_request, book_request_approval, book_request_gateway_setting, book_request_grab,
	book_request_release, book_request_search,
};
use sea_orm::{
	ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
	Set,
};
use serde_json::{json, Value};
use stump_auth::AuthContext;
use stump_core::{
	request_gateway::{GatewayClient, GatewayRelease},
	utils::encryption::encrypt_string,
};
use uuid::Uuid;

use crate::{
	data::CoreContext,
	input::book_request::{
		BookRequestGatewayInput, CreateBookRequestInput, ExternalWorkReferenceInput,
	},
	object::book_request::{BookRequest, BookRequestGatewaySettings, BookRequestGrab},
};
const DEFAULT_SCORE_FLOOR: i32 = 80;
const DEFAULT_VERIFICATION_THRESHOLD: i32 = 70;
const DEFAULT_MAX_RETRIES: i32 = 3;
const GATEWAY_TIMEOUT: Duration = Duration::from_secs(15);

fn manage(auth: &AuthContext) -> bool {
	auth.user.is_server_owner
		|| auth
			.user
			.has_permission(models::shared::enums::UserPermission::ManageServer)
		|| auth
			.user
			.has_permission(models::shared::enums::UserPermission::ManageLibrary)
}

fn can_view(auth: &AuthContext, request: &book_request::Model) -> bool {
	manage(auth) || auth.user.id == request.requester_id
}

async fn find_request(core: &CoreContext, id: &str) -> Result<book_request::Model> {
	book_request::Entity::find_by_id(id)
		.one(core.conn.as_ref())
		.await?
		.ok_or_else(|| async_graphql::Error::new("book request not found"))
}

async fn find_gateway(core: &CoreContext) -> Result<book_request_gateway_setting::Model> {
	book_request_gateway_setting::Entity::find_by_id("default")
		.one(core.conn.as_ref())
		.await?
		.ok_or_else(|| async_graphql::Error::new("request gateway is not configured"))
}

async fn gateway(
	core: &CoreContext,
) -> Result<(book_request_gateway_setting::Model, GatewayClient)> {
	let settings = find_gateway(core).await?;
	if !settings.enabled {
		return Err(async_graphql::Error::new("request gateway is disabled"));
	}
	let key = core
		.get_encryption_key()
		.await
		.map_err(|error| async_graphql::Error::new(error.to_string()))?;
	let token =
		stump_core::utils::encryption::decrypt_string(&settings.encrypted_token, &key)
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
	let client = GatewayClient::new(&settings.endpoint, token, GATEWAY_TIMEOUT)
		.map_err(|error| async_graphql::Error::new(error.to_string()))?;
	Ok((settings, client))
}

fn require_target(input: &CreateBookRequestInput) -> Result<()> {
	let count = input.media_id.is_some() as u8
		+ input.work_id.is_some() as u8
		+ input.external.is_some() as u8;
	if count != 1 {
		return Err(async_graphql::Error::new(
			"exactly one of mediaId, workId, or external is required",
		));
	}
	Ok(())
}

/// Shared recommendation -> request handoff. It intentionally creates only a
/// pending ledger row; accepting a recommendation never grants or downloads.
pub async fn create_request_from_recommendation(
	core: &CoreContext,
	requester_id: &str,
	media_id: Option<String>,
	work_id: Option<String>,
	external: Option<ExternalWorkReferenceInput>,
	title: Option<String>,
	authors: Option<String>,
	cover_url: Option<String>,
	destination_shelf_id: Option<String>,
	destination_device_id: Option<String>,
) -> Result<book_request::Model> {
	let input = CreateBookRequestInput {
		media_id: media_id.map(ID::from),
		work_id: work_id.map(ID::from),
		external,
		title,
		authors,
		cover_url,
		destination_shelf_id: destination_shelf_id.map(ID::from),
		destination_device_id: destination_device_id.map(ID::from),
		automation_enabled: false,
	};
	require_target(&input)?;
	let external = input.external.as_ref();
	let title = input
		.title
		.clone()
		.or_else(|| external.map(|value| value.title.clone()))
		.map(|value| value.trim().to_owned())
		.filter(|value| !value.is_empty())
		.ok_or_else(|| async_graphql::Error::new("request title is required"))?;
	let authors = input
		.authors
		.clone()
		.or_else(|| external.and_then(|value| value.authors.clone()));
	let cover_url = input
		.cover_url
		.clone()
		.or_else(|| external.and_then(|value| value.cover_url.clone()));
	let settings = book_request_gateway_setting::Entity::find_by_id("default")
		.one(core.conn.as_ref())
		.await?;
	let (scoring_floor, verification_threshold, max_retries, approval_policy) = settings
		.map(|value| {
			(
				value.scoring_floor,
				value.verification_threshold,
				value.max_retries,
				if value.require_approval {
					"REQUIRED"
				} else {
					"OPTIONAL"
				},
			)
		})
		.unwrap_or((
			DEFAULT_SCORE_FLOOR,
			DEFAULT_VERIFICATION_THRESHOLD,
			DEFAULT_MAX_RETRIES,
			"REQUIRED",
		));
	let now = Utc::now().fixed_offset();
	let model = book_request::ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		requester_id: Set(requester_id.to_owned()),
		internal_media_id: Set(input.media_id.map(|value| value.to_string())),
		internal_work_id: Set(input.work_id.map(|value| value.to_string())),
		source_provider: Set(external.map(|value| value.source_provider.clone())),
		remote_id: Set(external.map(|value| value.remote_id.clone())),
		external_key: Set(external.and_then(|value| value.external_key.clone())),
		title: Set(title),
		authors: Set(authors),
		cover_url: Set(cover_url),
		destination_shelf_id: Set(input
			.destination_shelf_id
			.map(|value| value.to_string())),
		destination_device_id: Set(input
			.destination_device_id
			.map(|value| value.to_string())),
		status: Set("PENDING".into()),
		approval_policy: Set(approval_policy.into()),
		automation_enabled: Set(false),
		scoring_floor: Set(scoring_floor.clamp(0, 100)),
		verification_threshold: Set(verification_threshold.clamp(0, 100)),
		max_retries: Set(max_retries.clamp(0, 10)),
		retries: Set(0),
		approved_by: Set(None),
		rejected_by: Set(None),
		failure_code: Set(None),
		failure_message: Set(None),
		created_at: Set(now),
		updated_at: Set(now),
		approved_at: Set(None),
		completed_at: Set(None),
	};
	Ok(model.insert(core.conn.as_ref()).await?)
}

fn token_words(value: &str) -> Vec<String> {
	value
		.to_ascii_lowercase()
		.split(|character: char| !character.is_ascii_alphanumeric())
		.filter(|word| word.len() > 1)
		.map(str::to_owned)
		.collect()
}

fn overlap(left: &str, right: &str) -> bool {
	let right_words = token_words(right);
	token_words(left)
		.iter()
		.any(|word| right_words.iter().any(|candidate| candidate == word))
}

fn score_release(
	request: &book_request::Model,
	release: &GatewayRelease,
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
	} else if overlap(&request.title, &release.title) {
		15
	} else {
		0
	};
	let author_score = match (&request.authors, &release.authors) {
		(Some(expected), Some(actual)) if expected.eq_ignore_ascii_case(actual) => 20,
		(Some(expected), Some(actual)) if overlap(expected, actual) => 14,
		(Some(_), Some(_)) => 0,
		_ => 0,
	};
	// The target's optional fields are not persisted separately; a present,
	// normalized fact earns a conservative verification point rather than an
	// invented match. This keeps explanations honest for external references.
	let format_score = i32::from(release.format.is_some()) * 10;
	let language_score = i32::from(release.language.is_some()) * 10;
	let edition_score = i32::from(release.edition.is_some()) * 10;
	let quality_score = i32::from(release.quality.is_some()) * 10;
	let size_score = i32::from(release.size_bytes.is_some()) * 5;
	let seeders_score = i32::from(release.seeders.unwrap_or_default() > 0) * 5;
	let score = title_score
		+ author_score
		+ format_score
		+ language_score
		+ edition_score
		+ quality_score
		+ size_score
		+ seeders_score;
	(
		score,
		json!({
			"title": title_score,
			"author": author_score,
			"format": format_score,
			"language": language_score,
			"edition": edition_score,
			"quality": quality_score,
			"size": size_score,
			"seeders": seeders_score,
		}),
	)
}

async fn mark_request_failed(
	core: &CoreContext,
	id: &str,
	code: &str,
	message: &str,
) -> Result<book_request::Model> {
	let request = find_request(core, id).await?;
	let mut active = request.into_active_model();
	active.status = Set("FAILED".into());
	active.failure_code = Set(Some(code.to_owned()));
	active.failure_message = Set(Some(message.chars().take(512).collect()));
	Ok(active.update(core.conn.as_ref()).await?)
}

async fn load_visible_request(
	core: &CoreContext,
	auth: &AuthContext,
	id: &str,
) -> Result<book_request::Model> {
	let request = find_request(core, id).await?;
	if !can_view(auth, &request) {
		return Err(async_graphql::Error::new(
			"not authorized to view this request",
		));
	}
	Ok(request)
}

#[derive(Default)]
pub struct BookRequestMutation;

#[Object]
impl BookRequestMutation {
	async fn create_book_request(
		&self,
		ctx: &Context<'_>,
		input: CreateBookRequestInput,
	) -> Result<BookRequest> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		require_target(&input)?;
		let automation_enabled = input.automation_enabled;
		let model = create_request_from_recommendation(
			core,
			&auth.user.id,
			input.media_id.map(|value| value.to_string()),
			input.work_id.map(|value| value.to_string()),
			input.external,
			input.title,
			input.authors,
			input.cover_url,
			input.destination_shelf_id.map(|value| value.to_string()),
			input.destination_device_id.map(|value| value.to_string()),
		)
		.await?;
		let mut saved = model;
		if automation_enabled {
			let mut active = saved.clone().into_active_model();
			active.automation_enabled = Set(true);
			saved = active.update(core.conn.as_ref()).await?;
			if saved.approval_policy == "OPTIONAL" {
				core.enqueue(
					stump_core::job::stump_job::StumpJob::book_request_automation(
						saved.id.clone(),
					),
				)
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))?;
			}
		}
		Ok(saved.into())
	}

	async fn approve_book_request(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
		reason: Option<String>,
	) -> Result<BookRequest> {
		let auth = ctx.data::<AuthContext>()?;
		if !manage(auth) {
			return Err(async_graphql::Error::new(
				"book request approval requires operator access",
			));
		}
		let core = ctx.data::<CoreContext>()?;
		let request = find_request(core, request_id.as_ref()).await?;
		if !matches!(request.status.as_str(), "PENDING" | "AWAITING_APPROVAL") {
			return Err(async_graphql::Error::new(
				"request is not awaiting approval",
			));
		}
		let now = Utc::now().fixed_offset();
		book_request_approval::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			request_id: Set(request.id.clone()),
			approver_id: Set(auth.user.id.clone()),
			decision: Set("APPROVED".into()),
			reason: Set(reason),
			created_at: Set(now),
		}
		.insert(core.conn.as_ref())
		.await?;
		let mut active = request.into_active_model();
		active.status = Set("APPROVED".into());
		active.approved_by = Set(Some(auth.user.id.clone()));
		active.approved_at = Set(Some(now));
		active.rejected_by = Set(None);
		let saved = active.update(core.conn.as_ref()).await?;
		if saved.automation_enabled {
			core.enqueue(
				stump_core::job::stump_job::StumpJob::book_request_automation(
					saved.id.clone(),
				),
			)
			.await
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		}
		Ok(saved.into())
	}

	async fn reject_book_request(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
		reason: Option<String>,
	) -> Result<BookRequest> {
		let auth = ctx.data::<AuthContext>()?;
		if !manage(auth) {
			return Err(async_graphql::Error::new(
				"book request rejection requires operator access",
			));
		}
		let core = ctx.data::<CoreContext>()?;
		let request = find_request(core, request_id.as_ref()).await?;
		if matches!(request.status.as_str(), "REJECTED" | "COMPLETED" | "FAILED") {
			return Err(async_graphql::Error::new("request state is terminal"));
		}
		let now = Utc::now().fixed_offset();
		book_request_approval::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			request_id: Set(request.id.clone()),
			approver_id: Set(auth.user.id.clone()),
			decision: Set("REJECTED".into()),
			reason: Set(reason.clone()),
			created_at: Set(now),
		}
		.insert(core.conn.as_ref())
		.await?;
		let mut active = request.into_active_model();
		active.status = Set("REJECTED".into());
		active.rejected_by = Set(Some(auth.user.id.clone()));
		active.failure_message = Set(reason);
		Ok(active.update(core.conn.as_ref()).await?.into())
	}

	async fn search_book_request(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<BookRequest> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_visible_request(core, auth, request_id.as_ref()).await?;
		if matches!(request.status.as_str(), "REJECTED" | "COMPLETED") {
			return Err(async_graphql::Error::new("request state is terminal"));
		}
		let (_settings, client) = gateway(core).await?;
		let query = match &request.authors {
			Some(authors) if !authors.trim().is_empty() => {
				format!("{} {}", request.title, authors)
			},
			_ => request.title.clone(),
		};
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
		.insert(core.conn.as_ref())
		.await?;
		let mut searching = request.clone().into_active_model();
		searching.status = Set("SEARCHING".into());
		searching.update(core.conn.as_ref()).await?;
		let releases = match client.search(&query).await {
			Ok(releases) => releases,
			Err(error) => {
				let message = error.to_string();
				let mut active = book_request_search::Entity::find_by_id(&search_id)
					.one(core.conn.as_ref())
					.await?
					.ok_or_else(|| async_graphql::Error::new("search row disappeared"))?
					.into_active_model();
				active.status = Set("FAILED".into());
				active.error_code = Set(Some("GATEWAY_SEARCH_FAILED".into()));
				active.error_message = Set(Some(message.clone()));
				active.finished_at = Set(Some(Utc::now().fixed_offset()));
				active.update(core.conn.as_ref()).await?;
				return mark_request_failed(
					core,
					&request.id,
					"GATEWAY_SEARCH_FAILED",
					&message,
				)
				.await
				.map(Into::into);
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
			.insert(core.conn.as_ref())
			.await?;
		}
		let mut search = book_request_search::Entity::find_by_id(&search_id)
			.one(core.conn.as_ref())
			.await?
			.ok_or_else(|| async_graphql::Error::new("search row disappeared"))?
			.into_active_model();
		search.status = Set("COMPLETED".into());
		search.result_count = Set(scored.len() as i32);
		search.finished_at = Set(Some(Utc::now().fixed_offset()));
		search.update(core.conn.as_ref()).await?;
		let mut request = find_request(core, &request.id).await?.into_active_model();
		let approved = request.approved_by.clone().unwrap().is_some();
		request.status = Set(
			if request.approval_policy.clone().unwrap() == "REQUIRED" && !approved {
				"AWAITING_APPROVAL".into()
			} else {
				"APPROVED".into()
			},
		);
		Ok(request.update(core.conn.as_ref()).await?.into())
	}

	async fn select_book_request_release(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
		release_id: ID,
	) -> Result<BookRequest> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_visible_request(core, auth, request_id.as_ref()).await?;
		let release = book_request_release::Entity::find_by_id(release_id.as_ref())
			.filter(book_request_release::Column::RequestId.eq(request.id.clone()))
			.one(core.conn.as_ref())
			.await?
			.ok_or_else(|| async_graphql::Error::new("release not found for request"))?;
		if release.score < request.scoring_floor {
			return Err(async_graphql::Error::new(format!(
				"release score {} is below floor {}",
				release.score, request.scoring_floor
			)));
		}
		book_request_release::Entity::update_many()
			.filter(book_request_release::Column::RequestId.eq(request.id.clone()))
			.set(book_request_release::ActiveModel {
				selected: Set(false),
				..Default::default()
			})
			.exec(core.conn.as_ref())
			.await?;
		let mut selected = release.into_active_model();
		selected.selected = Set(true);
		selected.update(core.conn.as_ref()).await?;
		let mut request = request.into_active_model();
		if matches!(
			request.status.clone().unwrap().as_str(),
			"PENDING" | "SEARCHING" | "AWAITING_APPROVAL" | "NEEDS_SELECTION"
		) {
			let approved = request.approved_by.clone().unwrap().is_some();
			if request.approval_policy.clone().unwrap() == "OPTIONAL" || approved {
				request.status = Set("APPROVED".into());
			} else {
				request.status = Set("AWAITING_APPROVAL".into());
			}
		}
		Ok(request.update(core.conn.as_ref()).await?.into())
	}

	async fn grab_book_request(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<BookRequestGrab> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_visible_request(core, auth, request_id.as_ref()).await?;
		if !matches!(request.status.as_str(), "APPROVED") {
			return Err(async_graphql::Error::new(
				"request must be approved before grabbing",
			));
		}
		let release = book_request_release::Entity::find()
			.filter(book_request_release::Column::RequestId.eq(request.id.clone()))
			.filter(book_request_release::Column::Selected.eq(true))
			.order_by_asc(book_request_release::Column::Rank)
			.one(core.conn.as_ref())
			.await?
			.ok_or_else(|| {
				async_graphql::Error::new("select a release before grabbing")
			})?;
		let (settings, client) = gateway(core).await?;
		let response = client
			.grab(&release.remote_id, &request.id)
			.await
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		let now = Utc::now().fixed_offset();
		let grab = book_request_grab::ActiveModel {
			id: Set(Uuid::new_v4().to_string()),
			request_id: Set(request.id.clone()),
			release_id: Set(release.id),
			opaque_id: Set(response.opaque_id),
			status: Set(if response.status.is_empty() {
				"SUBMITTED".into()
			} else {
				response.status
			}),
			attempts: Set(1),
			max_attempts: Set(settings.max_retries.clamp(0, 10)),
			failure_code: Set(None),
			failure_message: Set(None),
			started_at: Set(Some(now)),
			finished_at: Set(None),
			last_polled_at: Set(None),
			next_poll_at: Set(Some(now)),
			created_at: Set(now),
			updated_at: Set(now),
		};
		let grab = grab.insert(core.conn.as_ref()).await?;
		let mut active = request.into_active_model();
		active.status = Set("GRABBED".into());
		active.update(core.conn.as_ref()).await?;
		core.start_scheduler_with_maintenance()
			.await
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		Ok(grab.into())
	}

	async fn poll_book_request_grab(
		&self,
		ctx: &Context<'_>,
		grab_id: ID,
	) -> Result<BookRequestGrab> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let grab = book_request_grab::Entity::find_by_id(grab_id.as_ref())
			.one(core.conn.as_ref())
			.await?
			.ok_or_else(|| async_graphql::Error::new("grab not found"))?;
		load_visible_request(core, auth, &grab.request_id).await?;
		let mut active = grab.into_active_model();
		active.next_poll_at = Set(Some(Utc::now().fixed_offset()));
		let saved = active.update(core.conn.as_ref()).await?;
		core.start_scheduler_with_maintenance()
			.await
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		Ok(saved.into())
	}

	async fn retry_book_request(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
	) -> Result<BookRequest> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let request = load_visible_request(core, auth, request_id.as_ref()).await?;
		if request.retries >= request.max_retries {
			return Err(async_graphql::Error::new("request retry limit reached"));
		}
		let automation_enabled = request.automation_enabled;
		let mut active = request.into_active_model();
		active.retries = Set(active.retries.clone().unwrap() + 1);
		active.status = Set("APPROVED".into());
		active.failure_code = Set(None);
		active.failure_message = Set(None);
		let saved = active.update(core.conn.as_ref()).await?;
		if automation_enabled {
			core.enqueue(
				stump_core::job::stump_job::StumpJob::book_request_automation(
					saved.id.clone(),
				),
			)
			.await
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		}
		Ok(saved.into())
	}
	async fn update_book_request_gateway(
		&self,
		ctx: &Context<'_>,
		input: BookRequestGatewayInput,
	) -> Result<BookRequestGatewaySettings> {
		let auth = ctx.data::<AuthContext>()?;
		if !manage(auth)
			|| (!auth.user.is_server_owner
				&& !auth
					.user
					.has_permission(models::shared::enums::UserPermission::ManageServer))
		{
			return Err(async_graphql::Error::new(
				"gateway settings require server administrator access",
			));
		}
		if !(0..=100).contains(&input.scoring_floor)
			|| !(0..=100).contains(&input.verification_threshold)
			|| !(0..=10).contains(&input.max_retries)
		{
			return Err(async_graphql::Error::new(
				"gateway scoring thresholds or retry count are out of bounds",
			));
		}
		if input
			.handoff_root
			.as_deref()
			.is_some_and(|root| !Path::new(root).is_absolute())
		{
			return Err(async_graphql::Error::new(
				"handoff root must be an absolute path",
			));
		}
		let core = ctx.data::<CoreContext>()?;
		let old = book_request_gateway_setting::Entity::find_by_id("default")
			.one(core.conn.as_ref())
			.await?;
		let token = if input.token == "********" || input.token.trim().is_empty() {
			let old = old.as_ref().ok_or_else(|| {
				async_graphql::Error::new("token is required on first configuration")
			})?;
			let key = core
				.get_encryption_key()
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))?;
			stump_core::utils::encryption::decrypt_string(&old.encrypted_token, &key)
				.map_err(|error| async_graphql::Error::new(error.to_string()))?
		} else {
			input.token.clone()
		};
		GatewayClient::new(&input.endpoint, token.clone(), GATEWAY_TIMEOUT)
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		let key = core
			.get_encryption_key()
			.await
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		let encrypted_token = encrypt_string(&token, &key)
			.map_err(|error| async_graphql::Error::new(error.to_string()))?;
		let now = Utc::now().fixed_offset();
		let model = book_request_gateway_setting::ActiveModel {
			id: Set("default".into()),
			endpoint: Set(input.endpoint.clone()),
			encrypted_token: Set(encrypted_token.clone()),
			enabled: Set(input.enabled),
			require_approval: Set(input.require_approval),
			automation_enabled: Set(input.automation_enabled),
			scoring_floor: Set(input.scoring_floor),
			verification_threshold: Set(input.verification_threshold),
			max_retries: Set(input.max_retries),
			handoff_root: Set(input.handoff_root.clone()),
			updated_by: Set(Some(auth.user.id.clone())),
			updated_at: Set(now),
		};
		let saved = match model.insert(core.conn.as_ref()).await {
			Ok(saved) => saved,
			Err(_) => {
				let existing =
					book_request_gateway_setting::Entity::find_by_id("default")
						.one(core.conn.as_ref())
						.await?
						.ok_or_else(|| {
							async_graphql::Error::new(
								"gateway settings could not be saved",
							)
						})?;
				let mut active = existing.into_active_model();
				active.endpoint = Set(input.endpoint);
				active.encrypted_token = Set(encrypted_token);
				active.enabled = Set(input.enabled);
				active.require_approval = Set(input.require_approval);
				active.automation_enabled = Set(input.automation_enabled);
				active.scoring_floor = Set(input.scoring_floor);
				active.verification_threshold = Set(input.verification_threshold);
				active.max_retries = Set(input.max_retries);
				active.handoff_root = Set(input.handoff_root);
				active.updated_by = Set(Some(auth.user.id.clone()));
				active.updated_at = Set(now);
				active.update(core.conn.as_ref()).await?
			},
		};
		Ok(saved.into())
	}
}
