//! Kobo `tags` write-back: device shelves onto canonical containers.
//!
//! Every operation goes through `stump_core::collections` so a shelf created
//! on the device and a collection created natively are the same row. The
//! device's name is recorded as `source_device` provenance (last writer
//! wins on `name`), mirroring how `library_sync` records sightings.
//!
//! Wire contract pinned to Calibre-Web
//! `a97826402f1b39c45b7ea8d906efddc9f1750934`:
//! - creation with a name that already exists for the user reuses the shelf,
//! - unknown shelf ids are 404, unknown books are silently ignored,
//! - rename/delete/item routes require the shelf owner.

use models::entity::{device, device_credential};
use models::shared::enums::DeviceCredentialKind;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use stump_auth::AuthContext;
use stump_core::collections;
use stump_kobo::routes::{TagCreateRequest, TagItemsRequest};

use crate::{config::state::AppState, errors::APIResult};

pub(crate) async fn create_tag(
	ctx: AppState,
	auth: AuthContext,
	request: TagCreateRequest,
) -> APIResult<String> {
	let user = auth.user();
	let device = device_name(&ctx, auth.api_key.as_deref()).await;
	let model =
		collections::create_device_shelf(&ctx, user, request.id, request.name, device).await?;
	Ok(model.id)
}

pub(crate) async fn rename_tag(
	ctx: AppState,
	auth: AuthContext,
	tag_id: String,
	name: String,
) -> APIResult<()> {
	let user = auth.user();
	let device = device_name(&ctx, auth.api_key.as_deref()).await;
	collections::rename_shelf(&ctx, user, &tag_id, name, device).await?;
	Ok(())
}

pub(crate) async fn delete_tag(ctx: AppState, auth: AuthContext, tag_id: String) -> APIResult<()> {
	collections::delete_shelf(&ctx, auth.user(), &tag_id).await?;
	Ok(())
}

pub(crate) async fn add_tag_items(
	ctx: AppState,
	auth: AuthContext,
	tag_id: String,
	revision_ids: Vec<String>,
) -> APIResult<()> {
	let user = auth.user();
	let device = device_name(&ctx, auth.api_key.as_deref()).await;
	// Unknown books are silently ignored (Calibre-Web behavior), so filter to
	// the books this user can actually see before touching the container.
	let visible = visible_books(&ctx, user, &revision_ids).await?;
	collections::add_shelf_items(&ctx, user, &tag_id, visible, device).await?;
	Ok(())
}

pub(crate) async fn remove_tag_items(
	ctx: AppState,
	auth: AuthContext,
	tag_id: String,
	revision_ids: Vec<String>,
) -> APIResult<()> {
	let user = auth.user();
	let device = device_name(&ctx, auth.api_key.as_deref()).await;
	collections::remove_shelf_items(&ctx, user, &tag_id, revision_ids, device).await?;
	Ok(())
}

async fn visible_books(
	ctx: &AppState,
	user: &models::entity::user::AuthUser,
	book_ids: &[String],
) -> APIResult<Vec<String>> {
	if book_ids.is_empty() {
		return Ok(Vec::new());
	}
	let visible = models::entity::media::Entity::find_for_user(user)
		.filter(models::entity::media::Column::Id.is_in(book_ids.to_vec()))
		.filter(models::entity::media::Column::DeletedAt.is_null())
		.select_only()
		.column(models::entity::media::Column::Id)
		.into_tuple::<String>()
		.all(ctx.conn.as_ref())
		.await?;
	Ok(visible)
}

/// Resolve the device name behind the request's API key for provenance.
/// Best-effort: an unknown key yields `None` and the write proceeds.
async fn device_name(ctx: &AppState, api_key: Option<&str>) -> Option<String> {
	use prefixed_api_key::PrefixedApiKey;

	let api_key = api_key?;
	let pak = PrefixedApiKey::from_string(api_key).ok()?;
	let credential = device_credential::Entity::find()
		.filter(device_credential::Column::CredentialKind.eq(DeviceCredentialKind::ApiKey))
		.filter(device_credential::Column::CredentialRef.eq(pak.short_token().to_owned()))
		.one(ctx.conn.as_ref())
		.await
		.ok()
		.flatten()?;
	let device = device::Entity::find_by_id(credential.device_id)
		.one(ctx.conn.as_ref())
		.await
		.ok()
		.flatten()?;
	Some(device.name)
}
