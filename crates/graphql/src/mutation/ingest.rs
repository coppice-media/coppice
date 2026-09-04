use std::collections::BTreeMap;

use async_graphql::{Context, Error, Object, Result, ID};
use metadata_integrations::MergeStrategy;
use models::{
	entity::{
		ingest_drop_item, ingest_metadata_application, ingest_plugin_setting,
		media_metadata,
	},
	shared::enums::UserPermission,
};
use sea_orm::{
	ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
	TransactionTrait,
};
use serde_json::{json, Value};
use stump_core::ingest::{
	contract::{FieldPick, SettingValues},
	providers::apply::{
		apply_to_media, resolve_picks_for_context, validate_picks, ResolvedFields,
	},
};

use crate::{
	data::CoreContext,
	guard::{OptionalFeature, OptionalFeatureGuard, PermissionGuard},
	input::ingest::{
		ApplyIngestMetadataInput, BulkApplyIngestMetadataInput,
		EnqueueIngestAnalysisInput, IngestMetadataFieldSelectionInput,
		SetIngestProviderSettingsInput, SetIngestQualityCheckSettingsInput,
		StageIngestUploadsInput,
	},
	object::ingest::{
		IngestAnalysisJob, IngestBulkApplyFailure, IngestBulkApplyPayload,
		IngestDropFolder, IngestDropItem, IngestProviderDescriptor,
		IngestProviderSettings, IngestQualityCheckDescriptor, IngestQualityCheckSettings,
		StageIngestUploadsPayload,
	},
};

fn core_error<E: std::fmt::Display>(error: E) -> Error {
	Error::new(error.to_string())
}

fn convert_selections(
	selections: Vec<IngestMetadataFieldSelectionInput>,
) -> Result<Vec<FieldPick>> {
	let mut picks = Vec::with_capacity(selections.len());
	for selection in selections {
		picks.push(selection.into_field_pick().map_err(Error::new)?);
	}
	validate_picks(&picks).map_err(core_error)?;
	Ok(picks)
}

fn settings_from_json(value: Option<async_graphql::Json<Value>>) -> Option<Value> {
	value.map(|json| json.0)
}

fn settings_values(value: Option<Value>) -> SettingValues {
	match value {
		Some(Value::Object(values)) => values.into_iter().collect(),
		_ => BTreeMap::new(),
	}
}

struct PluginSettingsUpdate<'a> {
	core: &'a CoreContext,
	plugin_id: &'a str,
	kind: &'a str,
	user_id: &'a str,
	default_enabled: bool,
	default_opted_in: bool,
	enabled: Option<bool>,
	opted_in: Option<bool>,
	settings: Option<Value>,
}

async fn save_plugin_settings(
	update: PluginSettingsUpdate<'_>,
) -> Result<ingest_plugin_setting::Model> {
	let existing = update
		.core
		.ingest()
		.store
		.plugin_settings(update.plugin_id, update.kind, None, Some(update.user_id))
		.await
		.map_err(core_error)?;
	let mut active = if let Some(model) = existing {
		model.into_active_model()
	} else {
		ingest_plugin_setting::ActiveModel {
			plugin_id: Set(update.plugin_id.to_owned()),
			kind: Set(update.kind.to_owned()),
			library_id: Set(None),
			user_id: Set(Some(update.user_id.to_owned())),
			enabled: Set(update.default_enabled),
			opted_in: Set(update.default_opted_in),
			..Default::default()
		}
	};
	if let Some(enabled) = update.enabled {
		active.enabled = Set(enabled);
	}
	if let Some(opted_in) = update.opted_in {
		active.opted_in = Set(opted_in);
	}
	if let Some(settings) = update.settings {
		active.values = Set(Some(settings));
	}
	update
		.core
		.ingest()
		.store
		.set_plugin_settings(active)
		.await
		.map_err(core_error)
}

#[derive(Default)]
pub struct IngestMutation;

#[Object]
impl IngestMutation {
	#[graphql(
		guard = "OptionalFeatureGuard::new(OptionalFeature::Upload).and(PermissionGuard::new(&[UserPermission::UploadFile, UserPermission::ManageLibrary]))"
	)]
	async fn stage_ingest_uploads(
		&self,
		ctx: &Context<'_>,
		input: StageIngestUploadsInput,
	) -> Result<StageIngestUploadsPayload> {
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let core = ctx.data::<CoreContext>()?;
		let services = core.ingest();
		let StageIngestUploadsInput {
			library_id,
			files,
			idempotency_key,
			start_analysis,
		} = input;
		let library_id = library_id.to_string();
		let file_count = files.len();
		let mut items = Vec::with_capacity(file_count);
		let mut deduplicated = 0_i32;
		for (index, upload_input) in files.into_iter().enumerate() {
			let value = upload_input.file.value(ctx)?;
			let filename = value.filename.clone();
			let reader = tokio::fs::File::from_std(value.content);
			let idempotency_key = idempotency_key.as_deref().map(|key| {
				if file_count <= 1 {
					key.to_owned()
				} else {
					format!("{key}:{index}")
				}
			});
			let staged = services
				.store
				.stage_upload(
					&library_id,
					Some(&auth.user.id),
					upload_input.relative_path.as_deref(),
					&filename,
					reader,
					idempotency_key.as_deref(),
				)
				.await
				.map_err(core_error)?;
			if staged.deduplicated {
				deduplicated += 1;
			}
			items.push(staged.item);
		}
		if start_analysis && !items.is_empty() {
			services
				.coordinator
				.enqueue(items.iter().map(|item| item.id.clone()).collect(), false)
				.await
				.map_err(core_error)?;
		}
		Ok(StageIngestUploadsPayload {
			items: items.into_iter().map(IngestDropItem::from).collect(),
			deduplicated,
		})
	}

	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::UploadFile, UserPermission::ManageLibrary])"
	)]
	async fn scan_ingest_drop_folder(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
	) -> Result<IngestDropFolder> {
		let core = ctx.data::<CoreContext>()?;
		let library_id_string = library_id.to_string();
		core.ingest()
			.store
			.scan_drop_folder(&library_id_string)
			.await
			.map_err(core_error)?;
		let info = core
			.ingest()
			.store
			.drop_folder(&library_id_string)
			.await
			.map_err(core_error)?;
		Ok(IngestDropFolder {
			library_id,
			display_path: info.path_display,
			enabled: true,
			pending_count: info.pending_files.min(i32::MAX as u32) as i32,
			last_discovered_at: None,
		})
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageJobs)")]
	async fn enqueue_ingest_analysis(
		&self,
		ctx: &Context<'_>,
		input: EnqueueIngestAnalysisInput,
	) -> Result<Vec<IngestAnalysisJob>> {
		let ids = input
			.drop_item_ids
			.into_iter()
			.map(|id| id.to_string())
			.collect();
		ctx.data::<CoreContext>()?
			.ingest()
			.coordinator
			.enqueue(ids, input.force)
			.await
			.map_err(core_error)
			.map(|jobs| jobs.into_iter().map(IngestAnalysisJob::from).collect())
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageJobs)")]
	async fn requeue_ingest_analysis(
		&self,
		ctx: &Context<'_>,
		drop_item_id: ID,
		#[graphql(default = false)] force: bool,
	) -> Result<IngestAnalysisJob> {
		ctx.data::<CoreContext>()?
			.ingest()
			.coordinator
			.requeue(drop_item_id.as_ref(), force)
			.await
			.map(IngestAnalysisJob::from)
			.map_err(core_error)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageJobs)")]
	async fn pause_ingest_analysis(
		&self,
		ctx: &Context<'_>,
		job_id: ID,
	) -> Result<IngestAnalysisJob> {
		ctx.data::<CoreContext>()?
			.ingest()
			.coordinator
			.pause(job_id.as_ref())
			.await
			.map(IngestAnalysisJob::from)
			.map_err(core_error)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageJobs)")]
	async fn resume_ingest_analysis(
		&self,
		ctx: &Context<'_>,
		job_id: ID,
	) -> Result<IngestAnalysisJob> {
		ctx.data::<CoreContext>()?
			.ingest()
			.coordinator
			.resume(job_id.as_ref())
			.await
			.map(IngestAnalysisJob::from)
			.map_err(core_error)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageJobs)")]
	async fn retry_ingest_analysis(
		&self,
		ctx: &Context<'_>,
		job_id: ID,
	) -> Result<IngestAnalysisJob> {
		ctx.data::<CoreContext>()?
			.ingest()
			.coordinator
			.retry(job_id.as_ref())
			.await
			.map(IngestAnalysisJob::from)
			.map_err(core_error)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageJobs)")]
	async fn cancel_ingest_analysis(
		&self,
		ctx: &Context<'_>,
		job_id: ID,
	) -> Result<IngestAnalysisJob> {
		ctx.data::<CoreContext>()?
			.ingest()
			.coordinator
			.cancel(job_id.as_ref())
			.await
			.map(IngestAnalysisJob::from)
			.map_err(core_error)
	}

	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::UploadFile, UserPermission::ManageLibrary])"
	)]
	async fn discard_ingest_item(
		&self,
		ctx: &Context<'_>,
		drop_item_id: ID,
		_reason: Option<String>,
	) -> Result<IngestDropItem> {
		ctx.data::<CoreContext>()?
			.ingest()
			.store
			.discard(drop_item_id.as_ref())
			.await
			.map(IngestDropItem::from)
			.map_err(core_error)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn apply_ingest_metadata(
		&self,
		ctx: &Context<'_>,
		input: ApplyIngestMetadataInput,
	) -> Result<IngestDropItem> {
		let picks = convert_selections(input.selections)?;
		let strategy = input.strategy.unwrap_or_default();
		let core = ctx.data::<CoreContext>()?;
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let item_id = input.drop_item_id.to_string();
		let item = core
			.ingest()
			.store
			.item(&item_id)
			.await
			.map_err(core_error)?
			.ok_or_else(|| Error::new("Ingest item not found"))?;
		let candidates = core
			.ingest()
			.store
			.candidates(&item_id)
			.await
			.map_err(core_error)?;
		let existing = if let Some(media_id) = &item.media_id {
			media_metadata::Entity::find()
				.filter(media_metadata::Column::MediaId.eq(media_id))
				.one(core.conn.as_ref())
				.await?
		} else {
			None
		};
		let resolved = resolve_picks_for_context(
			&picks,
			&candidates,
			existing.as_ref(),
			&[],
			strategy,
			Some(&item.id),
			Some(&item.source_sha256),
		)
		.map_err(core_error)?;
		if let Some(media_id) = &item.media_id {
			apply_to_media(core.conn.as_ref(), media_id, resolved, &auth.user.id)
				.await
				.map_err(core_error)?;
		} else {
			persist_pending_fields(
				core,
				&item,
				&picks,
				resolved,
				strategy,
				&auth.user.id,
			)
			.await?;
		}
		core.ingest()
			.store
			.item(&item_id)
			.await
			.map_err(core_error)?
			.ok_or_else(|| Error::new("Ingest item disappeared after metadata apply"))
			.map(IngestDropItem::from)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn bulk_apply_ingest_metadata(
		&self,
		ctx: &Context<'_>,
		input: BulkApplyIngestMetadataInput,
	) -> Result<IngestBulkApplyPayload> {
		let selections = input.selections;
		let strategy = input.strategy;
		let mut applied = Vec::new();
		let mut failures = Vec::new();
		for id in input.drop_item_ids {
			let drop_item_id = id.to_string();
			match self
				.apply_ingest_metadata(
					ctx,
					ApplyIngestMetadataInput {
						drop_item_id: id,
						selections: selections.clone(),
						strategy,
					},
				)
				.await
			{
				Ok(item) => applied.push(item),
				Err(error) => failures.push(IngestBulkApplyFailure {
					drop_item_id: drop_item_id.into(),
					message: error.message,
				}),
			}
		}
		Ok(IngestBulkApplyPayload { applied, failures })
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::EditMetadata)")]
	async fn approve_ingest_item(
		&self,
		ctx: &Context<'_>,
		drop_item_id: ID,
		_strategy: Option<MergeStrategy>,
	) -> Result<IngestDropItem> {
		let core = ctx.data::<CoreContext>()?;
		let id = drop_item_id.to_string();
		let item = core
			.ingest()
			.store
			.item(&id)
			.await
			.map_err(core_error)?
			.ok_or_else(|| Error::new("Ingest item not found"))?;
		let (_, committed) = core
			.ingest()
			.store
			.approve(&id, Vec::new(), item.revision)
			.await
			.map_err(core_error)?;
		Ok(IngestDropItem::from(committed))
	}

	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::UploadFile, UserPermission::ManageLibrary])"
	)]
	async fn reject_ingest_item(
		&self,
		ctx: &Context<'_>,
		drop_item_id: ID,
		reason: Option<String>,
	) -> Result<IngestDropItem> {
		ctx.data::<CoreContext>()?
			.ingest()
			.store
			.reject(drop_item_id.as_ref(), reason.as_deref())
			.await
			.map(IngestDropItem::from)
			.map_err(core_error)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataProviderManage)")]
	async fn set_ingest_provider_settings(
		&self,
		ctx: &Context<'_>,
		input: SetIngestProviderSettingsInput,
	) -> Result<IngestProviderSettings> {
		let core = ctx.data::<CoreContext>()?;
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let provider_id = input.provider_id;
		let descriptor = core
			.ingest()
			.providers
			.catalog()
			.await
			.into_iter()
			.find(|descriptor| descriptor.id == provider_id)
			.ok_or_else(|| Error::new("Ingest provider not found"))?;
		let model = save_plugin_settings(PluginSettingsUpdate {
			core,
			plugin_id: &provider_id,
			kind: "PROVIDER",
			user_id: &auth.user.id,
			default_enabled: descriptor.enabled_default,
			default_opted_in: false,
			enabled: input.enabled,
			opted_in: input.opted_in,
			settings: settings_from_json(input.settings),
		})
		.await?;
		Ok(IngestProviderSettings::from_parts(
			IngestProviderDescriptor::from(descriptor),
			Some(model),
		))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataProviderManage)")]
	async fn verify_ingest_provider(
		&self,
		ctx: &Context<'_>,
		provider_id: String,
		settings: Option<async_graphql::Json<Value>>,
	) -> Result<metadata_integrations::ProviderCredentialVerification> {
		let values = settings_values(settings_from_json(settings));
		ctx.data::<CoreContext>()?
			.ingest()
			.providers
			.verify(&provider_id, &values)
			.await
			.map(|_| metadata_integrations::ProviderCredentialVerification {
				response_status: 200,
				is_valid: true,
				error: None,
			})
			.map_err(core_error)
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataProviderManage)")]
	async fn set_ingest_quality_check_settings(
		&self,
		ctx: &Context<'_>,
		input: SetIngestQualityCheckSettingsInput,
	) -> Result<IngestQualityCheckSettings> {
		let core = ctx.data::<CoreContext>()?;
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let check_id = input.check_id;
		let descriptor = core
			.ingest()
			.quality
			.catalog()
			.into_iter()
			.find(|descriptor| descriptor.id == check_id)
			.ok_or_else(|| Error::new("Ingest quality check not found"))?;
		let model = save_plugin_settings(PluginSettingsUpdate {
			core,
			plugin_id: &check_id,
			kind: "CHECK",
			user_id: &auth.user.id,
			default_enabled: true,
			default_opted_in: false,
			enabled: input.enabled,
			opted_in: None,
			settings: settings_from_json(input.settings),
		})
		.await?;
		Ok(IngestQualityCheckSettings::from_parts(
			IngestQualityCheckDescriptor::from(descriptor),
			Some(model),
		))
	}
}

async fn persist_pending_fields(
	core: &CoreContext,
	item: &ingest_drop_item::Model,
	picks: &[FieldPick],
	resolved: ResolvedFields,
	strategy: MergeStrategy,
	actor: &str,
) -> Result<()> {
	let mut pending = item.pending_fields.clone().unwrap_or_else(|| json!({}));
	let Value::Object(object) = &mut pending else {
		return Err(Error::new("pending fields are not a JSON object"));
	};
	if let Value::Object(values) = serde_json::to_value(&resolved.values)? {
		object.extend(values);
	}
	for field in resolved.cleared {
		if let Ok(Value::String(name)) = serde_json::to_value(field) {
			object.remove(&name);
		}
	}
	let transaction = core.conn.begin().await?;
	let mut active: ingest_drop_item::ActiveModel = item.clone().into_active_model();
	active.pending_fields = Set(Some(pending));
	active.revision = Set(item.revision.saturating_add(1));
	active.update(&transaction).await?;
	let strategy = serde_json::to_value(strategy)
		.ok()
		.and_then(|value| value.as_str().map(ToOwned::to_owned))
		.unwrap_or_else(|| format!("{strategy:?}").to_uppercase());
	let audit = ingest_metadata_application::ActiveModel {
		drop_item_id: Set(Some(item.id.clone())),
		media_id: Set(item.media_id.clone()),
		expected_revision: Set(item.revision),
		strategy: Set(strategy),
		picks: Set(serde_json::to_value(picks)?),
		actor: Set(actor.to_owned()),
		applied_fields: Set(serde_json::to_value(resolved.values)?),
		failed_fields: Set(json!([])),
		..Default::default()
	};
	audit.insert(&transaction).await?;
	transaction.commit().await?;
	Ok(())
}
