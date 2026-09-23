use async_graphql::{Context, Object, Result, ID};
use chrono::Utc;
use models::entity::{book_request, book_request_approval};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use stump_auth::AuthContext;
use uuid::Uuid;

use crate::{
	data::CoreContext,
	input::book_request::{CreateBookRequestInput, ExternalWorkReferenceInput},
	object::book_request::BookRequest,
};

fn manage(auth: &AuthContext) -> bool {
	auth.user.is_server_owner
		|| auth
			.user
			.has_permission(models::shared::enums::UserPermission::ManageServer)
		|| auth
			.user
			.has_permission(models::shared::enums::UserPermission::ManageLibrary)
}

async fn find_request(core: &CoreContext, id: &str) -> Result<book_request::Model> {
	book_request::Entity::find_by_id(id)
		.one(core.conn.as_ref())
		.await?
		.ok_or_else(|| async_graphql::Error::new("book request not found"))
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
/// pending ledger row; accepting a recommendation never grants or acquires a book.
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
		approval_policy: Set("REQUIRED".into()),
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
		Ok(create_request_from_recommendation(
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
		.await?
		.into())
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
		Ok(active.update(core.conn.as_ref()).await?.into())
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
}
