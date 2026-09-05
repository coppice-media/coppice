use async_graphql::{Context, ID, Json, Object, Result};
use stump_api_types::settings::{SettingDefinition, SettingValues};
use stump_core::annotation_sync::{encrypt_sink_values, upsert_sink_config};
use stump_jobs::JobPayload;

use crate::{
	data::CoreContext,
	object::annotation::AnnotationSyncStatus,
	query::annotation::build_status,
};

#[derive(Default)]
pub struct AnnotationMutation;

#[Object]
impl AnnotationMutation {
	/// Configures one annotation export sink for the current user: replaces
	/// the settings map (secret values are encrypted at rest) and optionally
	/// toggles the sink.
	async fn set_annotation_sink_settings(
		&self,
		ctx: &Context<'_>,
		sink_id: String,
		settings: Option<Json<serde_json::Value>>,
		enabled: Option<bool>,
	) -> Result<AnnotationSyncStatus> {
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let descriptor = stump_annotation_sync::registry::catalog()
			.into_iter()
			.find(|sink| sink.id == sink_id)
			.ok_or_else(|| async_graphql::Error::new("Annotation sink not found"))?;

		let values: SettingValues = match &settings {
			Some(Json(value)) => value
				.as_object()
				.map(|map| map.iter().map(|(key, value)| (key.clone(), value.clone())).collect())
				.unwrap_or_default(),
			None => SettingValues::default(),
		};

		// Secrets are only encrypted (and only require the key) when actually
		// present in the submitted values.
		let has_secret = values.iter().any(|(key, value)| {
			value.as_str().is_some()
				&& descriptor
					.settings
					.iter()
					.any(|definition| definition.key == key.as_str() && definition.secret)
		});
		let stored = if has_secret {
			let encryption_key = core
				.get_encryption_key()
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))?;
			encrypt_sink_values(&descriptor.settings, &values, &encryption_key)
				.map_err(|error| async_graphql::Error::new(error.to_string()))?
		} else {
			values.clone()
		};

		upsert_sink_config(
			core.conn.as_ref(),
			&auth.user.id,
			&sink_id,
			Some(serde_json::to_value(stored).map_err(|error| {
				async_graphql::Error::new(format!("Failed to serialize sink settings: {error}"))
			})?),
			enabled.unwrap_or(true),
		)
		.await
		.map_err(|error| async_graphql::Error::new(error.to_string()))?;

		build_status(core, &auth.user.id).await
	}

	/// Enqueues an immediate annotation export for a user (self by default;
	/// other users require the server owner), bypassing the debounce.
	async fn run_annotation_sync(
		&self,
		ctx: &Context<'_>,
		user_id: Option<ID>,
	) -> Result<AnnotationSyncStatus> {
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;

		let user_id = match user_id {
			Some(id) => {
				if id.as_str() != auth.user.id && !auth.user.is_server_owner {
					return Err("You may only run your own annotation sync".into());
				}
				id.to_string()
			},
			None => auth.user.id.clone(),
		};

		core.enqueue(stump_core::job::stump_job::StumpJob::AnnotationSync {
			user_id: user_id.clone(),
		})
		.await
		.map_err(|error| async_graphql::Error::new(error.to_string()))?;

		build_status(core, &user_id).await
	}
}

/// Re-exported for tests: validates a sink id against the catalog.
#[allow(dead_code)]
fn sink_definitions(sink_id: &str) -> Option<Vec<SettingDefinition>> {
	stump_annotation_sync::registry::catalog()
		.into_iter()
		.find(|sink| sink.id == sink_id)
		.map(|descriptor| descriptor.settings)
}
