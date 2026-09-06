use async_graphql::{Context, Object, Result, ID};
use chrono::Utc;
use models::{
	entity::known_duplicate_page,
	shared::enums::{DuplicatePageAction, UserPermission},
};
use sea_orm::{prelude::*, sea_query::OnConflict, Set};
use stump_ingest::quality::duplicate_pages_across_books::parse_dhash;

use crate::{
	data::CoreContext, guard::PermissionGuard,
	object::duplicate_page::KnownDuplicatePage,
	query::duplicate_page::assert_library_access,
};

#[derive(Default)]
pub struct DuplicatePageMutation;

#[Object]
impl DuplicatePageMutation {
	/// Record a decision about a recurring page hash. `SKIP` hides every page
	/// within the duplicate tolerance of `dhash` from all page-serving routes
	/// of the library's books (files are never modified); `KEEP` marks it as
	/// reviewed so it stops appearing as a candidate. Marking again replaces
	/// the previous decision.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn mark_duplicate_page(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
		#[graphql(
			desc = "16 hexadecimal digits, as reported by candidates and quality evidence"
		)]
		dhash: String,
		action: DuplicatePageAction,
	) -> Result<KnownDuplicatePage> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		assert_library_access(conn, user, &library_id).await?;
		let dhash = parse_dhash(&dhash).ok_or("Invalid dhash")?;

		let model = known_duplicate_page::ActiveModel {
			library_id: Set(library_id.to_string()),
			dhash: Set(dhash),
			action: Set(action),
			created_by: Set(Some(user.id.clone())),
			created_at: Set(Utc::now().into()),
		};
		let model = known_duplicate_page::Entity::insert(model)
			.on_conflict(
				OnConflict::columns([
					known_duplicate_page::Column::LibraryId,
					known_duplicate_page::Column::Dhash,
				])
				.update_columns([
					known_duplicate_page::Column::Action,
					known_duplicate_page::Column::CreatedBy,
					known_duplicate_page::Column::CreatedAt,
				])
				.to_owned(),
			)
			.exec_with_returning(conn)
			.await?;

		core.visible_pages_cache().invalidate_all();
		Ok(KnownDuplicatePage::from(model))
	}

	/// Forget a decision so the hash is reported as a candidate again and, for
	/// a former `SKIP`, its pages become visible again.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageLibrary)")]
	async fn unmark_duplicate_page(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
		dhash: String,
	) -> Result<bool> {
		let stump_auth::AuthContext { user, .. } =
			ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let conn = core.conn.as_ref();
		assert_library_access(conn, user, &library_id).await?;
		let dhash = parse_dhash(&dhash).ok_or("Invalid dhash")?;

		let result = known_duplicate_page::Entity::delete_many()
			.filter(known_duplicate_page::Column::LibraryId.eq(library_id.to_string()))
			.filter(known_duplicate_page::Column::Dhash.eq(dhash))
			.exec(conn)
			.await?;
		let removed = result.rows_affected > 0;
		if removed {
			core.visible_pages_cache().invalidate_all();
		}
		Ok(removed)
	}
}
