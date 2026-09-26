use async_graphql::{Context, Object, Result, ID};
use chrono::Utc;
use models::entity::{book_request, book_request_approval};
use sea_orm::{
	sea_query::Expr, ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait,
	IntoActiveModel, QueryFilter, Set,
};
use stump_auth::AuthContext;
use uuid::Uuid;

use crate::{
	data::CoreContext,
	input::book_request::{
		CreateBookRequestInput, ExternalWorkReferenceInput, RequestFormat,
	},
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

/// A narrator preference is free text from a picker; blank means "no
/// preference", which is stored as `NULL` rather than an empty string.
fn normalize_narrator(value: Option<String>) -> Result<Option<String>> {
	let Some(value) = value else {
		return Ok(None);
	};
	let trimmed = value.trim();
	if trimmed.chars().count() > 200 {
		return Err(async_graphql::Error::new(
			"preferred narrator must be at most 200 characters",
		));
	}
	Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

/// Request states with nothing left for a narrator preference to bias.
const NARRATOR_FROZEN_STATUSES: [&str; 2] = ["COMPLETED", "REJECTED"];

/// The requester may change their own request; operators may change any.
/// Once a request is completed or rejected the preference has nothing left
/// to bias, so it is frozen with the rest of the row.
pub(crate) async fn set_preferred_narrator(
	core: &CoreContext,
	auth: &AuthContext,
	request_id: &str,
	narrator: Option<String>,
) -> Result<book_request::Model> {
	let request = find_request(core, request_id).await?;
	if request.requester_id != auth.user.id && !manage(auth) {
		return Err(async_graphql::Error::new(
			"not authorized to change this request",
		));
	}
	if NARRATOR_FROZEN_STATUSES.contains(&request.status.as_str()) {
		return Err(async_graphql::Error::new("request state is terminal"));
	}
	let narrator = normalize_narrator(narrator)?;
	update_preferred_narrator(core.conn.as_ref(), request_id, narrator).await?;
	find_request(core, request_id).await
}

/// Writes only `preferred_narrator` (and `updated_at`), and only while the
/// stored status is still open: a completion or rejection that landed after
/// the authorization read above wins, and none of that row's other columns
/// are ever rewritten from the stale model.
async fn update_preferred_narrator(
	conn: &impl ConnectionTrait,
	request_id: &str,
	narrator: Option<String>,
) -> Result<()> {
	let written = book_request::Entity::update_many()
		.col_expr(
			book_request::Column::PreferredNarrator,
			Expr::value(narrator),
		)
		.col_expr(
			book_request::Column::UpdatedAt,
			Expr::value(Utc::now().fixed_offset()),
		)
		.filter(book_request::Column::Id.eq(request_id))
		.filter(book_request::Column::Status.is_not_in(NARRATOR_FROZEN_STATUSES))
		.exec(conn)
		.await?;
	if written.rows_affected == 0 {
		return Err(async_graphql::Error::new("request state is terminal"));
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
	format: RequestFormat,
	isbn: Option<String>,
	preferred_narrator: Option<String>,
) -> Result<book_request::Model> {
	let input = CreateBookRequestInput {
		media_id: media_id.map(ID::from),
		work_id: work_id.map(ID::from),
		external,
		title,
		authors,
		format,
		isbn,
		cover_url,
		destination_shelf_id: destination_shelf_id.map(ID::from),
		destination_device_id: destination_device_id.map(ID::from),
		preferred_narrator,
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
	let preferred_narrator = normalize_narrator(input.preferred_narrator)?;
	let now = Utc::now().fixed_offset();
	let model = book_request::ActiveModel {
		id: Set(Uuid::new_v4().to_string()),
		requester_id: Set(requester_id.to_owned()),
		internal_media_id: Set(input.media_id.map(|value| value.to_string())),
		internal_work_id: Set(input.work_id.map(|value| value.to_string())),
		source_provider: Set(external.map(|value| value.source_provider.clone())),
		remote_id: Set(external.map(|value| value.remote_id.clone())),
		external_key: Set(external.and_then(|value| value.external_key.clone())),
		format: Set(input.format.as_str().to_owned()),
		isbn: Set(input.isbn),
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
		preferred_narrator: Set(preferred_narrator),
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
			input.format,
			input.isbn,
			input.preferred_narrator,
		)
		.await?
		.into())
	}

	/// Set or clear (`narrator: null`) the reader a request would rather
	/// have. Requester or operator; refused once the request is completed or
	/// rejected.
	async fn set_book_request_preferred_narrator(
		&self,
		ctx: &Context<'_>,
		request_id: ID,
		narrator: Option<String>,
	) -> Result<BookRequest> {
		let auth = ctx.data::<AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		Ok(
			set_preferred_narrator(core, auth, request_id.as_ref(), narrator)
				.await?
				.into(),
		)
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

#[cfg(test)]
mod tests {
	use sea_orm::{ConnectionTrait, Database, DbBackend, EntityTrait, Statement};

	use super::*;

	async fn request_core() -> CoreContext {
		let db = Database::connect("sqlite::memory:").await.unwrap();
		db.execute(Statement::from_string(
			DbBackend::Sqlite,
			"CREATE TABLE book_requests (
				id TEXT PRIMARY KEY NOT NULL,
				requester_id TEXT NOT NULL,
				internal_media_id TEXT,
				internal_work_id TEXT,
				source_provider TEXT,
				remote_id TEXT,
				external_key TEXT,
				format TEXT NOT NULL DEFAULT 'ANY',
				isbn TEXT,
				title TEXT NOT NULL,
				authors TEXT,
				cover_url TEXT,
				destination_shelf_id TEXT,
				destination_device_id TEXT,
				status TEXT NOT NULL,
				approval_policy TEXT NOT NULL,
				approved_by TEXT,
				rejected_by TEXT,
				failure_code TEXT,
				failure_message TEXT,
				created_at TEXT NOT NULL,
				updated_at TEXT NOT NULL,
				approved_at TEXT,
				completed_at TEXT,
				preferred_narrator TEXT
			)"
			.to_owned(),
		))
		.await
		.unwrap();
		std::sync::Arc::new(stump_core::Ctx::for_testing(db))
	}

	async fn create_audiobook_request(
		core: &CoreContext,
		requester_id: &str,
		preferred_narrator: Option<&str>,
	) -> book_request::Model {
		let external = ExternalWorkReferenceInput {
			source_provider: "hardcover".to_owned(),
			remote_id: "book-42".to_owned(),
			external_key: None,
			title: "A requested book".to_owned(),
			authors: Some("A Writer".to_owned()),
			cover_url: None,
		};
		create_request_from_recommendation(
			core,
			requester_id,
			None,
			None,
			Some(external),
			None,
			None,
			None,
			None,
			None,
			RequestFormat::Audiobook,
			Some("9780306406157".to_owned()),
			preferred_narrator.map(str::to_owned),
		)
		.await
		.unwrap()
	}

	fn auth_for(user_id: &str, server_owner: bool) -> AuthContext {
		AuthContext {
			user: models::entity::user::AuthUser {
				id: user_id.to_owned(),
				is_server_owner: server_owner,
				..crate::tests::common::get_default_user()
			},
			api_key: None,
			device_id: None,
		}
	}

	#[tokio::test]
	async fn book_request_persists_the_selected_format_isbn_and_narrator() {
		let core = request_core().await;
		let created =
			create_audiobook_request(&core, "requester", Some("  Ray Porter ")).await;
		let stored = book_request::Entity::find_by_id(created.id.clone())
			.one(core.conn.as_ref())
			.await
			.unwrap()
			.unwrap();

		assert_eq!(stored.format, "AUDIOBOOK");
		assert_eq!(stored.isbn.as_deref(), Some("9780306406157"));
		assert_eq!(stored.preferred_narrator.as_deref(), Some("Ray Porter"));

		let blank = create_audiobook_request(&core, "requester", Some("   ")).await;
		assert_eq!(blank.preferred_narrator, None, "blank is no preference");
	}

	#[tokio::test]
	async fn preferred_narrator_is_set_by_requester_or_operator_until_terminal() {
		let core = request_core().await;
		let created = create_audiobook_request(&core, "requester", None).await;
		let requester = auth_for("requester", false);
		let stranger = auth_for("someone-else", false);
		let operator = auth_for("operator", true);

		let error = set_preferred_narrator(
			&core,
			&stranger,
			&created.id,
			Some("Ray Porter".to_owned()),
		)
		.await
		.expect_err("another member cannot steer someone else's request");
		assert_eq!(error.message, "not authorized to change this request");

		let updated = set_preferred_narrator(
			&core,
			&requester,
			&created.id,
			Some("Ray Porter".to_owned()),
		)
		.await
		.unwrap();
		assert_eq!(updated.preferred_narrator.as_deref(), Some("Ray Porter"));

		let updated = set_preferred_narrator(
			&core,
			&operator,
			&created.id,
			Some("Kate Reading".to_owned()),
		)
		.await
		.unwrap();
		assert_eq!(updated.preferred_narrator.as_deref(), Some("Kate Reading"));

		let cleared = set_preferred_narrator(&core, &requester, &created.id, None)
			.await
			.unwrap();
		assert_eq!(cleared.preferred_narrator, None, "null clears");

		let mut active = cleared.into_active_model();
		active.status = Set("COMPLETED".to_owned());
		active.update(core.conn.as_ref()).await.unwrap();
		let error = set_preferred_narrator(
			&core,
			&operator,
			&created.id,
			Some("Ray Porter".to_owned()),
		)
		.await
		.expect_err("a completed request is frozen");
		assert_eq!(error.message, "request state is terminal");

		assert!(set_preferred_narrator(&core, &operator, "missing", None)
			.await
			.is_err());
	}

	/// The write itself is the gate: a rejection that lands between the
	/// authorization read and the update wins, and a successful write leaves
	/// every other column — including a concurrent approval — exactly as
	/// stored rather than restoring the stale model.
	#[tokio::test]
	async fn narrator_write_is_gated_on_the_stored_status_and_touches_nothing_else() {
		let core = request_core().await;
		let created = create_audiobook_request(&core, "requester", None).await;
		let approved_at = Utc::now().fixed_offset();
		book_request::Entity::update_many()
			.col_expr(book_request::Column::Status, Expr::value("APPROVED"))
			.col_expr(
				book_request::Column::ApprovedBy,
				Expr::value(Some("operator")),
			)
			.col_expr(
				book_request::Column::ApprovedAt,
				Expr::value(Some(approved_at)),
			)
			.filter(book_request::Column::Id.eq(&created.id))
			.exec(core.conn.as_ref())
			.await
			.unwrap();

		update_preferred_narrator(
			core.conn.as_ref(),
			&created.id,
			Some("Ray Porter".to_owned()),
		)
		.await
		.unwrap();
		let stored = find_request(&core, &created.id).await.unwrap();
		assert_eq!(stored.preferred_narrator.as_deref(), Some("Ray Porter"));
		assert_eq!(stored.status, "APPROVED", "the approval is kept");
		assert_eq!(stored.approved_by.as_deref(), Some("operator"));
		assert_eq!(
			stored.approved_at.map(|at| at.timestamp()),
			Some(approved_at.timestamp())
		);

		book_request::Entity::update_many()
			.col_expr(book_request::Column::Status, Expr::value("REJECTED"))
			.col_expr(
				book_request::Column::FailureMessage,
				Expr::value(Some("out of scope")),
			)
			.filter(book_request::Column::Id.eq(&created.id))
			.exec(core.conn.as_ref())
			.await
			.unwrap();
		let before = find_request(&core, &created.id).await.unwrap();
		let error = update_preferred_narrator(
			core.conn.as_ref(),
			&created.id,
			Some("Kate Reading".to_owned()),
		)
		.await
		.expect_err("a rejection that landed after the read wins");
		assert_eq!(error.message, "request state is terminal");
		let after = find_request(&core, &created.id).await.unwrap();
		assert_eq!(after, before, "nothing is written to a terminal row");
		assert_eq!(after.preferred_narrator.as_deref(), Some("Ray Porter"));
		assert_eq!(after.failure_message.as_deref(), Some("out of scope"));
	}
}
