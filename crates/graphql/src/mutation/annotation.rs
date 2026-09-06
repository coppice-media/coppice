use async_graphql::{Context, Json, Object, Result, ID};
use stump_api_types::settings::SettingValues;
use stump_core::{
	annotation_sync::{encrypt_sink_values, upsert_sink_config},
	job::stump_job::StumpJob,
};

use crate::{
	data::CoreContext,
	object::annotation::AnnotationSyncStatus,
	query::annotation::{build_status, resolve_target_user},
};

#[derive(Default)]
pub struct AnnotationMutation;

#[Object]
impl AnnotationMutation {
	/// Configures one annotation export sink for the current user: replaces
	/// the settings map (secret values are encrypted at rest) and sets
	/// whether the sink runs (enabled by default).
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

		let values: SettingValues = match settings {
			Some(Json(serde_json::Value::Object(map))) => map.into_iter().collect(),
			Some(_) => {
				return Err("Sink settings must be a JSON object".into());
			},
			None => SettingValues::default(),
		};

		for definition in descriptor.settings.iter().filter(|d| d.required) {
			let present = values
				.get(definition.key)
				.is_some_and(|value| !value.is_null() && value.as_str() != Some(""));
			if !present {
				return Err(
					format!("Missing required setting `{}`", definition.key).into()
				);
			}
		}

		// Secrets are only encrypted (and only require the key) when actually
		// present in the submitted values.
		let has_secret = values.iter().any(|(key, value)| {
			value.is_string()
				&& descriptor
					.settings
					.iter()
					.any(|definition| definition.key == key && definition.secret)
		});
		let stored = if has_secret {
			let encryption_key = core
				.get_encryption_key()
				.await
				.map_err(|error| async_graphql::Error::new(error.to_string()))?;
			encrypt_sink_values(&descriptor.settings, &values, &encryption_key)
				.map_err(|error| async_graphql::Error::new(error.to_string()))?
		} else {
			values
		};

		upsert_sink_config(
			core.conn.as_ref(),
			&auth.user.id,
			&sink_id,
			Some(serde_json::to_value(stored)?),
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
		let user_id = resolve_target_user(auth, user_id)?;

		core.enqueue(StumpJob::AnnotationSync {
			user_id: user_id.clone(),
		})
		.await
		.map_err(|error| async_graphql::Error::new(error.to_string()))?;

		build_status(core, &user_id).await
	}
}
