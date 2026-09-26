use axum::{middleware, Router};

use std::{
	collections::HashMap,
	future::Future,
	sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock},
};

use async_trait::async_trait;
use chrono::Utc;
use metadata_integrations::{
	CrossProviderCandidate, CrossProviderScorer, ExternalMetadata,
	ExternalSeriesMetadata, MatchCandidate, MetadataProvider, MetadataProviderError,
	SearchQuery, AUTO_MATCH_THRESHOLD,
};
use models::{
	entity::{
		library, library_config, metadata_provider_config, series, series_metadata,
	},
	shared::enums::{
		LibraryType, MetadataProvider as MetadataProviderEnum, UserPermission,
	},
};
use sea_orm::{
	ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel,
	QueryFilter, QuerySelect, TransactionTrait,
};
use serde_json::{json, Value};
use stump_auth::AuthContext;
use stump_core::{
	filesystem::metadata::{
		apply_series_match, reset_library_metadata, reset_series_metadata,
		ProviderClientCache,
	},
	utils::encryption::{encrypt_string, fetch_encryption_key},
};
use stump_komf::{
	KomfBackend, KomfError, KomfEvent, KomfEventStream, KomfIdentifyRequest, KomfJob,
	KomfJobPage, KomfMediaServerLibrary, KomfMetadataJobResponse, KomfResult,
	KomfSearchResult, KomfSeriesSearchRequest,
};
use tokio::sync::{broadcast, OnceCell, Semaphore};
use uuid::Uuid;

use crate::config::state::AppState;

pub(crate) fn mount(app_state: AppState) -> Router<AppState> {
	let backend: Arc<dyn KomfBackend> =
		Arc::new(KomfBackendAdapter::new(app_state.clone()));
	stump_komf::router::<AppState>(backend).layer(middleware::from_fn_with_state(
		app_state,
		crate::middleware::auth::auth_middleware,
	))
}

const JOB_COMPLETED_EVENT: &str = "__KomfJobCompleted";
const PROVIDER_SERIES_EVENT: &str = "ProviderSeriesEvent";
const PROVIDER_COMPLETED_EVENT: &str = "ProviderCompletedEvent";
const PROVIDER_ERROR_EVENT: &str = "ProviderErrorEvent";
const POST_PROCESSING_START_EVENT: &str = "PostProcessingStartEvent";
const PROCESSING_ERROR_EVENT: &str = "ProcessingErrorEvent";

#[derive(Clone)]
pub(crate) struct KomfBackendAdapter {
	state: AppState,
	provider_cache: Arc<OnceCell<Arc<ProviderClientCache>>>,
	jobs: JobStore,
	http: reqwest::Client,
}

impl KomfBackendAdapter {
	pub(crate) fn new(state: AppState) -> Self {
		Self {
			state,
			provider_cache: Arc::new(OnceCell::new()),
			jobs: JobStore::default(),
			http: reqwest::Client::builder()
				.redirect(reqwest::redirect::Policy::none())
				.build()
				.expect("default reqwest client configuration is valid"),
		}
	}

	fn conn(&self) -> &sea_orm::DatabaseConnection {
		self.state.conn.as_ref()
	}

	async fn provider_cache(&self) -> KomfResult<Arc<ProviderClientCache>> {
		self.provider_cache
			.get_or_try_init(|| async {
				let key = fetch_encryption_key(self.conn())
					.await
					.map_err(|error| KomfError::Internal(error.to_string()))?;
				Ok::<_, KomfError>(Arc::new(ProviderClientCache::new(key)))
			})
			.await
			.cloned()
	}

	async fn provider_client(
		&self,
		config: &metadata_provider_config::Model,
	) -> KomfResult<Arc<dyn MetadataProvider + Send + Sync>> {
		self.provider_cache()
			.await?
			.get_or_create(config)
			.await
			.map_err(|error| KomfError::Internal(error.to_string()))
	}

	async fn enabled_provider_configs(
		&self,
		library_type: Option<&LibraryType>,
	) -> KomfResult<Vec<metadata_provider_config::Model>> {
		let mut configs = metadata_provider_config::Entity::find()
			.filter(metadata_provider_config::Column::Enabled.eq(true))
			.all(self.conn())
			.await
			.map_err(db_error)?;
		configs.retain(|config| {
			is_komf_provider(config.provider_type)
				&& library_type
					.map(|kind| kind.has_provider_overlap(&config.provider_type))
					.unwrap_or(true)
		});
		Ok(configs)
	}

	async fn visible_library(
		&self,
		auth: &AuthContext,
		library_id: &str,
	) -> KomfResult<library::Model> {
		library::Entity::find_for_user(auth.scope())
			.filter(library::Column::Id.eq(library_id))
			.one(self.conn())
			.await
			.map_err(db_error)?
			.ok_or_else(|| KomfError::NotFound(format!("Library {library_id} not found")))
	}

	async fn library_type(&self, library_id: &str) -> KomfResult<LibraryType> {
		library_config::Entity::find()
			.filter(library_config::Column::LibraryId.eq(library_id))
			.one(self.conn())
			.await
			.map_err(db_error)?
			.map(|config| config.library_type)
			.ok_or_else(|| KomfError::NotFound("Library configuration not found".into()))
	}

	async fn visible_series(
		&self,
		auth: &AuthContext,
		series_id: &str,
	) -> KomfResult<series::ModelWithMetadata> {
		series::ModelWithMetadata::find_by_id_for_user(
			series_id.to_string(),
			auth.scope(),
		)
		.into_model::<series::ModelWithMetadata>()
		.one(self.conn())
		.await
		.map_err(db_error)?
		.ok_or_else(|| KomfError::NotFound(format!("Series {series_id} not found")))
	}

	async fn provider_config(
		&self,
		provider_type: MetadataProviderEnum,
	) -> KomfResult<Option<metadata_provider_config::Model>> {
		metadata_provider_config::Entity::find()
			.filter(metadata_provider_config::Column::ProviderType.eq(provider_type))
			.one(self.conn())
			.await
			.map_err(db_error)
	}

	async fn query_candidates(
		&self,
		configs: Vec<metadata_provider_config::Model>,
		query: &SearchQuery,
		job: Option<&JobHandle>,
	) -> KomfResult<CandidateSearch> {
		let mut result = CandidateSearch::default();
		for config in configs {
			result.attempted += 1;
			let provider_name = komf_provider_name(config.provider_type)
				.expect("enabled Komf provider configs were filtered");
			if let Some(job) = job {
				job.emit(PROVIDER_SERIES_EVENT, json!({ "provider": provider_name }));
			}
			let provider = match self.provider_client(&config).await {
				Ok(provider) => provider,
				Err(error) => {
					if let Some(job) = job {
						job.emit(
							PROVIDER_ERROR_EVENT,
							json!({ "provider": provider_name, "message": error.to_string() }),
						);
					}
					tracing::warn!(provider = provider_name, %error, "Komf metadata provider could not be initialized");
					continue;
				},
			};
			match provider.search_series(query).await {
				Ok(outcome) => {
					result.successful += 1;
					result.candidates.extend(outcome.candidates);
					if let Some(job) = job {
						job.emit(
							PROVIDER_COMPLETED_EVENT,
							json!({ "provider": provider_name }),
						);
					}
				},
				Err(error) => {
					if let Some(job) = job {
						job.emit(
							PROVIDER_ERROR_EVENT,
							json!({ "provider": provider_name, "message": error.to_string() }),
						);
					}
					tracing::warn!(provider = provider_name, %error, "Komf metadata provider search failed");
				},
			}
		}
		Ok(result)
	}

	fn reference_candidate(
		title: String,
		metadata: Option<&series_metadata::Model>,
	) -> CrossProviderCandidate {
		let authors = metadata
			.and_then(|metadata| metadata.writers.clone())
			.map(|writer| vec![writer]);
		let candidate = MatchCandidate {
			provider: "coppice-local".to_string(),
			external_id: "local-reference".to_string(),
			metadata: ExternalMetadata::Series(ExternalSeriesMetadata {
				provider: "coppice-local".to_string(),
				external_id: "local-reference".to_string(),
				title,
				alternative_titles: alternative_titles(
					metadata.and_then(|metadata| metadata.alternate_titles.as_deref()),
				),
				year: metadata.and_then(|metadata| metadata.year),
				authors,
				publisher: metadata.and_then(|metadata| metadata.publisher.clone()),
				..Default::default()
			}),
			confidence: 0.0,
			confidence_factors: Vec::new(),
		};
		CrossProviderCandidate::new(candidate)
	}

	fn mapped_search_results(
		&self,
		reference: &CrossProviderCandidate,
		candidates: Vec<MatchCandidate>,
	) -> Vec<KomfSearchResult> {
		let cross_candidates: Vec<_> = candidates
			.into_iter()
			.filter(|candidate| {
				matches!(candidate.metadata, ExternalMetadata::Series(_))
					&& provider_kind(&candidate.provider).is_some()
			})
			.map(CrossProviderCandidate::new)
			.collect();
		let scorer = CrossProviderScorer;
		let grouped = scorer.merge_same_work_candidates(cross_candidates.clone());
		let urls: HashMap<_, _> = grouped
			.iter()
			.flat_map(|work| work.external_references.iter())
			.map(|reference| {
				(
					(reference.provider.clone(), reference.external_id.clone()),
					reference.url.clone(),
				)
			})
			.collect();
		let ranked = scorer.rank_candidates(reference, &cross_candidates);
		ranked
			.into_iter()
			.filter_map(|scored| {
				let candidate = &scored.candidate.candidate;
				let ExternalMetadata::Series(metadata) = &candidate.metadata else {
					return None;
				};
				let provider = komf_provider_name(provider_kind(&candidate.provider)?)?;
				let url = urls
					.get(&(candidate.provider.clone(), candidate.external_id.clone()))?
					.clone()?;
				Some(KomfSearchResult {
					url,
					image_url: metadata.cover_url.clone(),
					title: metadata.title.clone(),
					provider: provider.to_string(),
					result_id: candidate.external_id.clone(),
				})
			})
			.collect()
	}

	async fn apply_candidate(
		&self,
		series_id: &str,
		candidate: &MatchCandidate,
	) -> KomfResult<()> {
		apply_series_match(
			self.conn(),
			series_id,
			candidate,
			metadata_integrations::MergeStrategy::FillGaps,
			Vec::new(),
			Vec::new(),
		)
		.await
		.map_err(|error| KomfError::Internal(error.to_string()))
	}

	async fn start_auto_match(
		&self,
		series_id: String,
		owner_id: String,
	) -> KomfResult<KomfMetadataJobResponse> {
		let adapter = self.clone();
		self.jobs
			.start(owner_id, series_id.clone(), move |job| async move {
				adapter.auto_match_series(&series_id, &job).await
			})
	}

	async fn auto_match_series(
		&self,
		series_id: &str,
		job: &JobHandle,
	) -> KomfResult<String> {
		let model = series::ModelWithMetadata::find_by_id(series_id.to_string())
			.into_model::<series::ModelWithMetadata>()
			.one(self.conn())
			.await
			.map_err(db_error)?
			.ok_or_else(|| {
				KomfError::NotFound(format!("Series {series_id} not found"))
			})?;
		let library_id = model
			.series
			.library_id
			.as_deref()
			.ok_or_else(|| KomfError::NotFound("Series library not found".into()))?;
		let library_type = self.library_type(library_id).await?;
		let title = model
			.metadata
			.as_ref()
			.and_then(|metadata| metadata.title.clone())
			.unwrap_or_else(|| model.series.name.clone());
		let reference = Self::reference_candidate(title.clone(), model.metadata.as_ref());
		let configs = self.enabled_provider_configs(Some(&library_type)).await?;
		let query = search_query(&title, model.metadata.as_ref());
		let search = self.query_candidates(configs, &query, Some(job)).await?;
		if search.attempted > 0 && search.successful == 0 {
			return Err(KomfError::Internal(
				"All enabled metadata providers failed".into(),
			));
		}
		let candidates: Vec<_> = search
			.candidates
			.into_iter()
			.filter(|candidate| matches!(candidate.metadata, ExternalMetadata::Series(_)))
			.map(CrossProviderCandidate::new)
			.collect();
		let scorer = CrossProviderScorer;
		let Some(best) = scorer
			.rank_candidates(&reference, &candidates)
			.into_iter()
			.find(|scored| scored.score.confidence >= AUTO_MATCH_THRESHOLD)
		else {
			return Ok("No candidate met the auto-match threshold".into());
		};
		let mut candidate = best.candidate.candidate.clone();
		candidate.confidence = best.score.confidence;
		candidate.confidence_factors = best.score.confidence_factors;
		job.emit(POST_PROCESSING_START_EVENT, json!({}));
		self.apply_candidate(series_id, &candidate).await?;
		Ok(format!("Matched with {}", candidate.provider))
	}

	async fn cover_for_provider(
		&self,
		provider_type: MetadataProviderEnum,
		provider_series_id: &str,
	) -> KomfResult<Option<(String, axum::body::Bytes)>> {
		let Some(config) = self.provider_config(provider_type).await? else {
			return Ok(None);
		};
		if !config.enabled {
			return Ok(None);
		}
		let client = self.provider_client(&config).await?;
		let metadata = client
			.fetch_series_metadata(provider_series_id)
			.await
			.map_err(provider_error)?;
		let Some(cover_url) = metadata.cover_url else {
			return Ok(None);
		};
		if !is_allowed_cover_url(provider_type, &cover_url) {
			return Ok(None);
		}
		let response = self
			.http
			.get(&cover_url)
			.send()
			.await
			.map_err(|error| KomfError::Internal(error.to_string()))?;
		if !response.status().is_success() {
			return Ok(None);
		}
		let content_type = response
			.headers()
			.get(reqwest::header::CONTENT_TYPE)
			.and_then(|value| value.to_str().ok())
			.filter(|value| value.starts_with("image/"))
			.unwrap_or("image/jpeg")
			.to_string();
		let bytes = response
			.bytes()
			.await
			.map_err(|error| KomfError::Internal(error.to_string()))?;
		Ok(Some((content_type, bytes)))
	}

	async fn reset_series_data(&self, series_id: &str) -> KomfResult<()> {
		reset_series_metadata(self.conn(), series_id)
			.await
			.map_err(|error| KomfError::Internal(error.to_string()))
	}

	async fn reset_library_data(
		&self,
		auth: &AuthContext,
		library_id: &str,
	) -> KomfResult<()> {
		let series_ids = series::Entity::find_for_user(auth.scope())
			.filter(series::Column::LibraryId.eq(library_id))
			.select_only()
			.column(series::Column::Id)
			.into_tuple::<String>()
			.all(self.conn())
			.await
			.map_err(db_error)?;
		reset_library_metadata(self.conn(), &series_ids)
			.await
			.map_err(|error| KomfError::Internal(error.to_string()))
	}

	async fn get_config_json(&self) -> KomfResult<Value> {
		let configs = metadata_provider_config::Entity::find()
			.all(self.conn())
			.await
			.map_err(db_error)?;
		let enabled = |provider| {
			configs
				.iter()
				.find(|config| config.provider_type == provider)
				.is_some_and(|config| config.enabled)
		};
		Ok(komf_config(enabled))
	}

	async fn save_native_provider_config(
		&self,
		provider: MetadataProviderEnum,
		enabled: Option<bool>,
		token_change: Option<Option<String>>,
	) -> KomfResult<()> {
		let existing = self.provider_config(provider).await?;
		let old_enabled = existing.as_ref().is_some_and(|config| config.enabled);
		let old_token = existing
			.as_ref()
			.and_then(|config| config.encrypted_api_token.clone());
		let token_changed = token_change.is_some();
		let next_enabled = enabled.unwrap_or(old_enabled);
		let next_token = token_change.unwrap_or(old_token.clone());
		if metadata_integrations::requires_api_token(&provider.to_string())
			&& next_enabled
			&& next_token.is_none()
		{
			return Err(KomfError::Unprocessable(format!(
				"{} requires an API token before it can be enabled",
				provider
			)));
		}
		if existing.is_none() && !next_enabled && next_token.is_none() {
			return Ok(());
		}
		let tx = self.conn().begin().await.map_err(db_error)?;
		match existing {
			Some(model) => {
				let mut active = model.into_active_model();
				if enabled.is_some() {
					active.enabled = Set(next_enabled);
				}
				if token_changed {
					active.encrypted_api_token = Set(next_token);
				}
				active.update(&tx).await.map_err(db_error)?;
			},
			None => {
				metadata_provider_config::ActiveModel {
					provider_type: Set(provider),
					enabled: Set(next_enabled),
					encrypted_api_token: Set(next_token),
					..Default::default()
				}
				.insert(&tx)
				.await
				.map_err(db_error)?;
			},
		}
		tx.commit().await.map_err(db_error)?;
		if let Some(cache) = self.provider_cache.get() {
			cache.clear().await;
		}
		Ok(())
	}

	async fn persist_config_patch(&self, patch: Value) -> KomfResult<()> {
		let root = patch.as_object().ok_or_else(|| {
			KomfError::BadRequest("Configuration patch must be an object".into())
		})?;
		for (key, value) in root {
			if key != "metadataProviders" && !value.is_null() {
				return Err(KomfError::Unprocessable(format!(
					"Komf configuration field `{key}` has no native Coppice setting"
				)));
			}
		}
		let Some(metadata_patch) = root.get("metadataProviders") else {
			return Ok(());
		};
		if metadata_patch.is_null() {
			return Err(KomfError::Unprocessable(
				"Clearing the Komf provider configuration is not supported".into(),
			));
		}
		let metadata_patch = metadata_patch.as_object().ok_or_else(|| {
			KomfError::BadRequest("metadataProviders must be an object".into())
		})?;
		for (key, value) in metadata_patch {
			if !matches!(
				key.as_str(),
				"malClientId" | "defaultProviders" | "libraryProviders"
			) && !value.is_null()
			{
				return Err(KomfError::Unprocessable(format!(
					"Komf provider setting `{key}` has no native Coppice equivalent"
				)));
			}
		}
		if metadata_patch.get("libraryProviders").is_some_and(|value| {
			!value.is_null() && value.as_object().is_none_or(|map| !map.is_empty())
		}) {
			return Err(KomfError::Unprocessable(
				"Per-library Komf provider overrides are not supported".into(),
			));
		}

		let token_change = match metadata_patch.get("malClientId") {
			None => None,
			Some(Value::Null) => Some(None),
			Some(Value::String(token)) if token.trim().is_empty() => Some(None),
			Some(Value::String(token)) => {
				let key = fetch_encryption_key(self.conn())
					.await
					.map_err(|error| KomfError::Internal(error.to_string()))?;
				Some(Some(
					encrypt_string(token, &key)
						.map_err(|error| KomfError::Internal(error.to_string()))?,
				))
			},
			Some(_) => {
				return Err(KomfError::BadRequest(
					"malClientId must be a string or null".into(),
				));
			},
		};

		let mut enabled_changes = Vec::new();
		if let Some(default_providers) = metadata_patch.get("defaultProviders") {
			if default_providers.is_null() {
				for provider in native_komf_providers() {
					enabled_changes.push((provider, false));
				}
			} else {
				let default_providers =
					default_providers.as_object().ok_or_else(|| {
						KomfError::BadRequest("defaultProviders must be an object".into())
					})?;
				for (key, value) in default_providers {
					let Some(provider) = config_provider(key) else {
						if !value.is_null() {
							return Err(KomfError::Unprocessable(format!(
							"Komf provider `{key}` is not backed by a native Coppice provider"
						)));
						}
						continue;
					};
					if value.is_null() {
						enabled_changes.push((provider, false));
						continue;
					}
					let provider_patch = value.as_object().ok_or_else(|| {
						KomfError::BadRequest(format!(
							"{key} provider patch must be an object"
						))
					})?;
					for (field, field_value) in provider_patch {
						if field != "enabled" && !field_value.is_null() {
							return Err(KomfError::Unprocessable(format!(
								"Komf provider setting `{key}.{field}` has no native Coppice equivalent"
							)));
						}
					}
					if let Some(enabled) = provider_patch.get("enabled") {
						if let Some(enabled) = enabled.as_bool() {
							enabled_changes.push((provider, enabled));
						} else if !enabled.is_null() {
							return Err(KomfError::BadRequest(format!(
								"{key}.enabled must be a boolean"
							)));
						}
					}
				}
			}
		}
		let mut unique_changes = Vec::new();
		for (provider, enabled) in enabled_changes {
			if let Some(existing) = unique_changes
				.iter_mut()
				.find(|(current, _)| *current == provider)
			{
				existing.1 = enabled;
			} else {
				unique_changes.push((provider, enabled));
			}
		}
		let has_token_change = token_change.is_some();
		if !has_token_change
			&& unique_changes.iter().any(|(provider, enabled)| {
				*provider == MetadataProviderEnum::Mal && *enabled
			}) && self
			.provider_config(MetadataProviderEnum::Mal)
			.await?
			.is_none_or(|config| config.encrypted_api_token.is_none())
		{
			return Err(KomfError::Unprocessable(
				"MAL requires an API token before it can be enabled".into(),
			));
		}
		if let Some(token_change) = token_change {
			let enabled = unique_changes
				.iter()
				.find(|(provider, _)| *provider == MetadataProviderEnum::Mal)
				.map(|(_, enabled)| *enabled);
			self.save_native_provider_config(
				MetadataProviderEnum::Mal,
				enabled,
				Some(token_change),
			)
			.await?;
		}
		for (provider, enabled) in unique_changes {
			if provider == MetadataProviderEnum::Mal && has_token_change {
				continue;
			}
			self.save_native_provider_config(provider, Some(enabled), None)
				.await?;
		}
		Ok(())
	}
}

#[async_trait]
impl KomfBackend for KomfBackendAdapter {
	async fn providers(
		&self,
		auth: &AuthContext,
		library_id: Option<&str>,
	) -> KomfResult<Vec<String>> {
		let library_type = if let Some(library_id) = library_id {
			self.visible_library(auth, library_id).await?;
			Some(self.library_type(library_id).await?)
		} else {
			None
		};
		Ok(self
			.enabled_provider_configs(library_type.as_ref())
			.await?
			.into_iter()
			.filter_map(|config| {
				komf_provider_name(config.provider_type).map(str::to_string)
			})
			.collect())
	}

	async fn search_series(
		&self,
		auth: &AuthContext,
		request: KomfSeriesSearchRequest,
	) -> KomfResult<Vec<KomfSearchResult>> {
		let local_series = if let Some(series_id) = request.series_id.as_deref() {
			Some(self.visible_series(auth, series_id).await?)
		} else {
			None
		};
		let library_id = request.library_id.as_deref().or_else(|| {
			local_series
				.as_ref()
				.and_then(|model| model.series.library_id.as_deref())
		});
		if let Some(library_id) = library_id {
			self.visible_library(auth, library_id).await?;
		}
		if let (Some(requested), Some(series)) =
			(request.library_id.as_deref(), local_series.as_ref())
		{
			if series.series.library_id.as_deref() != Some(requested) {
				return Err(KomfError::BadRequest(
					"seriesId does not belong to libraryId".into(),
				));
			}
		}
		let library_type = match library_id {
			Some(library_id) => Some(self.library_type(library_id).await?),
			None => None,
		};
		let metadata = local_series
			.as_ref()
			.and_then(|model| model.metadata.as_ref());
		let local_title = metadata
			.and_then(|metadata| metadata.title.clone())
			.or_else(|| local_series.as_ref().map(|model| model.series.name.clone()));
		let titles = search_titles(request.name.as_deref(), local_title);
		let query = search_query(&titles.query, metadata);
		let reference = Self::reference_candidate(titles.reference, metadata);
		let configs = self.enabled_provider_configs(library_type.as_ref()).await?;
		let search = self.query_candidates(configs, &query, None).await?;
		if search.attempted > 0 && search.successful == 0 {
			return Err(KomfError::Internal(
				"All enabled metadata providers failed".into(),
			));
		}
		Ok(self.mapped_search_results(&reference, search.candidates))
	}

	async fn series_cover(
		&self,
		auth: &AuthContext,
		library_id: &str,
		provider: &str,
		provider_series_id: &str,
	) -> KomfResult<Option<(String, axum::body::Bytes)>> {
		self.visible_library(auth, library_id).await?;
		let Some(provider) = provider_kind(provider) else {
			return Ok(None);
		};
		let library_type = self.library_type(library_id).await?;
		let configs = self.enabled_provider_configs(Some(&library_type)).await?;
		if !configs
			.iter()
			.any(|config| config.provider_type == provider)
		{
			return Ok(None);
		}
		self.cover_for_provider(provider, provider_series_id).await
	}

	async fn identify_series(
		&self,
		auth: &AuthContext,
		request: KomfIdentifyRequest,
	) -> KomfResult<KomfMetadataJobResponse> {
		require(auth, UserPermission::EditMetadata)?;
		let model = self.visible_series(auth, &request.series_id).await?;
		let Some(library_id) = model.series.library_id.clone() else {
			return Err(KomfError::NotFound("Series library not found".into()));
		};
		if request
			.library_id
			.as_deref()
			.is_some_and(|requested| requested != library_id)
		{
			return Err(KomfError::BadRequest(
				"seriesId does not belong to libraryId".into(),
			));
		}
		let provider_type = provider_kind(&request.provider).ok_or_else(|| {
			KomfError::BadRequest("Unsupported metadata provider".into())
		})?;
		let config = self
			.provider_config(provider_type)
			.await?
			.filter(|config| config.enabled)
			.ok_or_else(|| {
				KomfError::BadRequest("Metadata provider is not enabled".into())
			})?;
		let adapter = self.clone();
		let series_id = request.series_id.clone();
		let external_id = request.provider_series_id;
		let provider_name = komf_provider_name(provider_type)
			.expect("provider_kind returns supported providers")
			.to_string();
		self.jobs
			.start(auth.id(), series_id.clone(), move |job| async move {
				job.emit(PROVIDER_SERIES_EVENT, json!({ "provider": provider_name }));
				let provider = adapter.provider_client(&config).await?;
				let metadata = provider
					.fetch_series_metadata(&external_id)
					.await
					.map_err(provider_error)?;
				job.emit(
					PROVIDER_COMPLETED_EVENT,
					json!({ "provider": provider_name }),
				);
				job.emit(POST_PROCESSING_START_EVENT, json!({}));
				let candidate = MatchCandidate {
					provider: provider.id().to_string(),
					external_id: external_id.clone(),
					metadata: ExternalMetadata::Series(metadata),
					confidence: 1.0,
					confidence_factors: vec![metadata_integrations::ConfidenceFactor {
						factor: "explicit_selection".into(),
						weight: 1.0,
						matched: true,
					}],
				};
				adapter.apply_candidate(&series_id, &candidate).await?;
				Ok(format!("Applied {provider_name} metadata"))
			})
	}

	async fn match_series(
		&self,
		auth: &AuthContext,
		library_id: &str,
		series_id: &str,
	) -> KomfResult<KomfMetadataJobResponse> {
		require(auth, UserPermission::EditMetadata)?;
		let series = self.visible_series(auth, series_id).await?;
		if series.series.library_id.as_deref() != Some(library_id) {
			return Err(KomfError::NotFound(format!(
				"Series {series_id} not found in library {library_id}"
			)));
		}
		self.start_auto_match(series_id.to_string(), auth.id())
			.await
	}

	async fn match_library(
		&self,
		auth: &AuthContext,
		library_id: &str,
	) -> KomfResult<()> {
		require(auth, UserPermission::EditMetadata)?;
		self.visible_library(auth, library_id).await?;
		let series_ids = series::Entity::find_for_user(auth.scope())
			.filter(series::Column::LibraryId.eq(library_id))
			.select_only()
			.column(series::Column::Id)
			.into_tuple::<String>()
			.all(self.conn())
			.await
			.map_err(db_error)?;
		for series_id in series_ids {
			self.start_auto_match(series_id, auth.id()).await?;
		}
		Ok(())
	}

	async fn reset_series(
		&self,
		auth: &AuthContext,
		library_id: &str,
		series_id: &str,
		remove_comic_info: bool,
	) -> KomfResult<()> {
		require(auth, UserPermission::EditMetadata)?;
		let series = self.visible_series(auth, series_id).await?;
		if series.series.library_id.as_deref() != Some(library_id) {
			return Err(KomfError::NotFound(format!(
				"Series {series_id} not found in library {library_id}"
			)));
		}
		if remove_comic_info {
			return Err(KomfError::Unprocessable(
				"Removing embedded ComicInfo is not supported; native metadata was not reset".into(),
			));
		}
		self.reset_series_data(series_id).await
	}

	async fn reset_library(
		&self,
		auth: &AuthContext,
		library_id: &str,
		remove_comic_info: bool,
	) -> KomfResult<()> {
		require(auth, UserPermission::EditMetadata)?;
		self.visible_library(auth, library_id).await?;
		if remove_comic_info {
			return Err(KomfError::Unprocessable(
				"Removing embedded ComicInfo is not supported; native metadata was not reset".into(),
			));
		}
		self.reset_library_data(auth, library_id).await
	}

	async fn config(&self, _auth: &AuthContext) -> KomfResult<Value> {
		self.get_config_json().await
	}

	async fn update_config(&self, auth: &AuthContext, patch: Value) -> KomfResult<()> {
		require(auth, UserPermission::MetadataProviderManage)?;
		self.persist_config_patch(patch).await
	}

	async fn jobs(
		&self,
		auth: &AuthContext,
		status: Option<&str>,
		page: i64,
		page_size: i64,
	) -> KomfResult<KomfJobPage> {
		self.jobs.page(&auth.id(), status, page, page_size)
	}

	async fn job(&self, auth: &AuthContext, job_id: &str) -> KomfResult<Option<KomfJob>> {
		self.jobs.get(&auth.id(), job_id)
	}

	async fn delete_all_jobs(&self, auth: &AuthContext) -> KomfResult<()> {
		require(auth, UserPermission::EditMetadata)?;
		self.jobs.delete_owner(&auth.id())
	}

	fn job_events(&self, auth: &AuthContext, job_id: &str) -> Option<KomfEventStream> {
		self.jobs.events(&auth.id(), job_id)
	}

	async fn libraries(
		&self,
		auth: &AuthContext,
	) -> KomfResult<Vec<KomfMediaServerLibrary>> {
		Ok(library::Entity::find_for_user(auth.scope())
			.all(self.conn())
			.await
			.map_err(db_error)?
			.into_iter()
			.map(|library| KomfMediaServerLibrary {
				id: library.id,
				name: library.name,
				roots: vec![library.path],
			})
			.collect())
	}
}

#[derive(Default)]
struct CandidateSearch {
	candidates: Vec<MatchCandidate>,
	attempted: usize,
	successful: usize,
}

/// Retention and scheduling limits for the process-local Komf job store.
///
/// Komelia polls `/api/jobs` and replays `/api/jobs/{id}/events`; finished jobs
/// stay listable until they age out or the finished-job cap evicts the oldest.
/// Running jobs are never evicted. Provider work runs through a shared
/// semaphore so a whole-library match is processed at a bounded pace instead
/// of spawning one provider search per series at once.
#[derive(Clone, Copy)]
struct JobLimits {
	max_finished_jobs: usize,
	finished_job_max_age: chrono::Duration,
	max_event_history: usize,
	max_concurrent_jobs: usize,
}

impl Default for JobLimits {
	fn default() -> Self {
		Self {
			max_finished_jobs: 1000,
			finished_job_max_age: chrono::Duration::hours(24),
			max_event_history: 128,
			max_concurrent_jobs: 3,
		}
	}
}

#[derive(Clone)]
struct JobStore {
	limits: JobLimits,
	records: Arc<StdRwLock<HashMap<Uuid, Arc<JobRecord>>>>,
	slots: Arc<Semaphore>,
}

impl Default for JobStore {
	fn default() -> Self {
		Self::with_limits(JobLimits::default())
	}
}

struct JobRecord {
	owner_id: String,
	job: StdMutex<KomfJob>,
	event_tx: broadcast::Sender<KomfEvent>,
	event_history: StdMutex<Vec<KomfEvent>>,
	max_event_history: usize,
}

#[derive(Clone)]
struct JobHandle(Arc<JobRecord>);

impl JobHandle {
	fn emit(&self, name: &str, data: Value) {
		self.0.emit(KomfEvent {
			name: name.to_string(),
			data: Some(data),
		});
	}
}

impl JobRecord {
	fn emit(&self, event: KomfEvent) {
		if let Ok(mut history) = self.event_history.lock() {
			if history.len() >= self.max_event_history {
				let overflow = history.len() + 1 - self.max_event_history;
				history.drain(..overflow);
			}
			history.push(event.clone());
			let _ = self.event_tx.send(event);
		}
	}

	fn update(&self, apply: impl FnOnce(&mut KomfJob)) {
		let mut job = self.job.lock().unwrap_or_else(|error| error.into_inner());
		apply(&mut job);
	}

	fn snapshot(&self) -> KomfJob {
		self.job
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.clone()
	}

	fn finished_at(&self) -> Option<chrono::DateTime<Utc>> {
		self.job
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.finished_at
	}

	fn stream(self: &Arc<Self>) -> KomfEventStream {
		let history_guard = self
			.event_history
			.lock()
			.unwrap_or_else(|error| error.into_inner());
		let mut receiver = self.event_tx.subscribe();
		let history = history_guard.clone();
		drop(history_guard);
		Box::pin(async_stream::stream! {
			for event in history {
				let finished = event.name == JOB_COMPLETED_EVENT;
				yield event;
				if finished {
					return;
				}
			}
			loop {
				match receiver.recv().await {
					Ok(event) => {
					let finished = event.name == JOB_COMPLETED_EVENT;
					yield event;
					if finished {
						return;
					}
				},
				Err(broadcast::error::RecvError::Closed) => return,
				Err(broadcast::error::RecvError::Lagged(_)) => continue,
			}
			}
		})
	}
}

impl JobStore {
	fn with_limits(limits: JobLimits) -> Self {
		Self {
			limits,
			records: Arc::default(),
			slots: Arc::new(Semaphore::new(limits.max_concurrent_jobs.max(1))),
		}
	}

	fn start<F, Fut>(
		&self,
		owner_id: String,
		series_id: String,
		work: F,
	) -> KomfResult<KomfMetadataJobResponse>
	where
		F: FnOnce(JobHandle) -> Fut + Send + 'static,
		Fut: Future<Output = KomfResult<String>> + Send + 'static,
	{
		let id = Uuid::new_v4();
		let (event_tx, _) = broadcast::channel(32);
		let record = Arc::new(JobRecord {
			owner_id,
			job: StdMutex::new(KomfJob {
				series_id,
				id: id.to_string(),
				status: "RUNNING".into(),
				message: "Queued".into(),
				started_at: Utc::now(),
				finished_at: None,
			}),
			event_tx,
			event_history: StdMutex::new(Vec::new()),
			max_event_history: self.limits.max_event_history.max(1),
		});
		{
			let mut records = self.records.write().map_err(|_| {
				KomfError::Internal("Komf job store lock poisoned".into())
			})?;
			prune_finished(&mut records, &self.limits, Utc::now());
			records.insert(id, record.clone());
		}
		let slots = self.slots.clone();
		tokio::spawn(async move {
			let result = match slots.acquire().await {
				Ok(_permit) => {
					record.update(|job| job.message = "Processing".into());
					work(JobHandle(record.clone())).await
				},
				Err(_) => Err(KomfError::Internal("Komf job scheduler closed".into())),
			};
			record.update(|job| {
				match &result {
					Ok(message) => {
						job.status = "COMPLETED".into();
						job.message = message.clone();
					},
					Err(error) => {
						job.status = "FAILED".into();
						job.message = error.to_string();
					},
				}
				job.finished_at = Some(Utc::now());
			});
			if let Err(error) = result {
				record.emit(KomfEvent {
					name: PROCESSING_ERROR_EVENT.into(),
					data: Some(json!({ "message": error.to_string() })),
				});
			}
			record.emit(KomfEvent {
				name: JOB_COMPLETED_EVENT.into(),
				data: None,
			});
		});
		Ok(KomfMetadataJobResponse {
			job_id: id.to_string(),
		})
	}

	fn page(
		&self,
		owner_id: &str,
		status: Option<&str>,
		page: i64,
		page_size: i64,
	) -> KomfResult<KomfJobPage> {
		if page < 0 || page_size <= 0 {
			return Err(KomfError::BadRequest("Invalid jobs page".into()));
		}
		let mut jobs = {
			let mut records = self.records.write().map_err(|_| {
				KomfError::Internal("Komf job store lock poisoned".into())
			})?;
			prune_finished(&mut records, &self.limits, Utc::now());
			records
				.values()
				.filter(|record| record.owner_id == owner_id)
				.map(|record| record.snapshot())
				.filter(|job| status.is_none_or(|status| job.status == status))
				.collect::<Vec<_>>()
		};
		jobs.sort_by(|left, right| right.started_at.cmp(&left.started_at));
		let count = jobs.len() as i64;
		let offset = if page == 0 {
			0
		} else {
			(page - 1).saturating_mul(page_size)
		};
		let offset = usize::try_from(offset).unwrap_or(usize::MAX);
		let limit = usize::try_from(page_size).unwrap_or(usize::MAX);
		let content = jobs.into_iter().skip(offset).take(limit).collect();
		Ok(KomfJobPage {
			content,
			total_pages: (count / page_size) as i32,
			current_page: page as i32,
		})
	}

	fn get(&self, owner_id: &str, job_id: &str) -> KomfResult<Option<KomfJob>> {
		let id = Uuid::parse_str(job_id)
			.map_err(|error| KomfError::BadRequest(error.to_string()))?;
		Ok(self
			.records
			.read()
			.map_err(|_| KomfError::Internal("Komf job store lock poisoned".into()))?
			.get(&id)
			.filter(|record| record.owner_id == owner_id)
			.map(|record| record.snapshot()))
	}

	fn events(&self, owner_id: &str, job_id: &str) -> Option<KomfEventStream> {
		let id = Uuid::parse_str(job_id).ok()?;
		let record = self.records.read().ok()?.get(&id)?.clone();
		if record.owner_id != owner_id {
			return None;
		}
		Some(record.stream())
	}

	fn delete_owner(&self, owner_id: &str) -> KomfResult<()> {
		self.records
			.write()
			.map_err(|_| KomfError::Internal("Komf job store lock poisoned".into()))?
			.retain(|_, record| record.owner_id != owner_id);
		Ok(())
	}
}

/// Drops finished jobs older than the retention window, then evicts the
/// oldest-finished jobs beyond the count cap. Running jobs are never dropped.
fn prune_finished(
	records: &mut HashMap<Uuid, Arc<JobRecord>>,
	limits: &JobLimits,
	now: chrono::DateTime<Utc>,
) {
	let cutoff = now - limits.finished_job_max_age;
	let mut finished = Vec::new();
	records.retain(|id, record| match record.finished_at() {
		Some(finished_at) if finished_at < cutoff => false,
		Some(finished_at) => {
			finished.push((finished_at, *id));
			true
		},
		None => true,
	});
	if finished.len() > limits.max_finished_jobs {
		finished.sort_unstable();
		for (_, id) in &finished[..finished.len() - limits.max_finished_jobs] {
			records.remove(id);
		}
	}
}

fn require(auth: &AuthContext, permission: UserPermission) -> KomfResult<()> {
	auth.enforce_permissions(&[permission])
		.map_err(|error| KomfError::Forbidden(error.to_string()))
}

fn db_error(error: sea_orm::DbErr) -> KomfError {
	KomfError::Internal(error.to_string())
}

fn provider_error(error: MetadataProviderError) -> KomfError {
	KomfError::Internal(error.to_string())
}

fn is_komf_provider(provider: MetadataProviderEnum) -> bool {
	matches!(
		provider,
		MetadataProviderEnum::AniList
			| MetadataProviderEnum::Mal
			| MetadataProviderEnum::MangaDex
			| MetadataProviderEnum::MangaUpdates
			| MetadataProviderEnum::Metron
	)
}

fn native_komf_providers() -> [MetadataProviderEnum; 4] {
	[
		MetadataProviderEnum::AniList,
		MetadataProviderEnum::Mal,
		MetadataProviderEnum::MangaDex,
		MetadataProviderEnum::MangaUpdates,
	]
}

fn provider_kind(provider: &str) -> Option<MetadataProviderEnum> {
	match provider.trim().to_ascii_uppercase().as_str() {
		"ANILIST" => Some(MetadataProviderEnum::AniList),
		"MAL" => Some(MetadataProviderEnum::Mal),
		"MANGADEX" => Some(MetadataProviderEnum::MangaDex),
		"MANGA_UPDATES" | "MANGAUPDATES" => Some(MetadataProviderEnum::MangaUpdates),
		"METRON" => Some(MetadataProviderEnum::Metron),
		_ => None,
	}
}

fn config_provider(key: &str) -> Option<MetadataProviderEnum> {
	match key {
		"aniList" => Some(MetadataProviderEnum::AniList),
		"mal" => Some(MetadataProviderEnum::Mal),
		"mangaDex" => Some(MetadataProviderEnum::MangaDex),
		"mangaUpdates" => Some(MetadataProviderEnum::MangaUpdates),
		_ => None,
	}
}

fn komf_provider_name(provider: MetadataProviderEnum) -> Option<&'static str> {
	match provider {
		MetadataProviderEnum::AniList => Some("ANILIST"),
		MetadataProviderEnum::Mal => Some("MAL"),
		MetadataProviderEnum::MangaDex => Some("MANGADEX"),
		MetadataProviderEnum::MangaUpdates => Some("MANGA_UPDATES"),
		MetadataProviderEnum::Metron => Some("METRON"),
		_ => None,
	}
}

fn alternative_titles(raw: Option<&str>) -> Vec<String> {
	raw.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
		.and_then(|value| value.as_array().cloned())
		.unwrap_or_default()
		.into_iter()
		.filter_map(|value| {
			value
				.as_str()
				.or_else(|| value.get("title").and_then(Value::as_str))
				.map(str::to_string)
		})
		.collect()
}

struct SearchTitles {
	/// Local title used as the ranking reference for cross-provider scoring.
	reference: String,
	/// Title sent to providers; a blank or missing `name` falls back to the
	/// local series title because Komelia omits `name` when it passes `seriesId`.
	query: String,
}

fn search_titles(
	requested_name: Option<&str>,
	local_title: Option<String>,
) -> SearchTitles {
	let requested = requested_name
		.map(str::trim)
		.filter(|name| !name.is_empty())
		.map(str::to_string);
	let reference = local_title
		.or_else(|| requested.clone())
		.unwrap_or_default();
	let query = requested.unwrap_or_else(|| reference.clone());
	SearchTitles { reference, query }
}

fn search_query(title: &str, metadata: Option<&series_metadata::Model>) -> SearchQuery {
	SearchQuery {
		title: title.to_string(),
		author: metadata.and_then(|metadata| metadata.writers.clone()),
		year: metadata.and_then(|metadata| metadata.year),
		limit: Some(20),
		..Default::default()
	}
}

fn is_allowed_cover_url(provider: MetadataProviderEnum, url: &str) -> bool {
	let Ok(url) = reqwest::Url::parse(url) else {
		return false;
	};
	if url.scheme() != "https" {
		return false;
	}
	let Some(host) = url.host_str() else {
		return false;
	};
	match provider {
		MetadataProviderEnum::AniList => host == "anilist.co" || host == "s4.anilist.co",
		MetadataProviderEnum::Mal => {
			host == "myanimelist.net" || host == "cdn.myanimelist.net"
		},
		MetadataProviderEnum::MangaDex => {
			host == "mangadex.org" || host == "uploads.mangadex.org"
		},
		MetadataProviderEnum::MangaUpdates => {
			host == "mangaupdates.com"
				|| host == "www.mangaupdates.com"
				|| host == "cdn.mangaupdates.com"
		},
		MetadataProviderEnum::Metron => {
			host == "metron.cloud" || host == "www.metron.cloud"
		},
		_ => false,
	}
}

fn komf_config(enabled: impl Fn(MetadataProviderEnum) -> bool) -> Value {
	let providers = json!({
		"mangaBaka": provider_config(false, false, "API"),
		"bookWalker": provider_config(false, true, "MANGA"),
		"mangaDex": {
			"priority": 10,
			"enabled": enabled(MetadataProviderEnum::MangaDex),
			"seriesMetadata": series_fields(),
			"bookMetadata": book_fields(),
			"nameMatchingMode": null,
			"mediaType": "MANGA",
			"authorRoles": ["WRITER"],
			"artistRoles": artist_roles(),
			"coverLanguages": ["en", "ja"],
			"links": []
		},
		"mangaUpdates": provider_config(enabled(MetadataProviderEnum::MangaUpdates), true, "MANGA"),
		"aniList": {
			"priority": 10,
			"enabled": enabled(MetadataProviderEnum::AniList),
			"seriesMetadata": series_fields(),
			"nameMatchingMode": null,
			"mediaType": "MANGA",
			"authorRoles": ["WRITER"],
			"artistRoles": artist_roles(),
			"tagsScoreThreshold": 60,
			"tagsSizeLimit": 15
		},
		"mal": provider_config(enabled(MetadataProviderEnum::Mal), true, "MANGA"),
		"comicVine": provider_config(false, true, "MANGA"),
		"nautiljon": provider_config(false, true, "MANGA"),
		"yenPress": provider_config(false, true, "MANGA"),
		"kodansha": provider_config(false, true, "MANGA"),
		"viz": provider_config(false, true, "MANGA"),
		"bangumi": provider_config(false, true, "MANGA"),
		"hentag": provider_config(false, true, "MANGA"),
		"webtoons": provider_config(false, true, "MANGA")
	});
	json!({
		"komga": {
			"baseUri": "",
			"komgaUser": "",
			"eventListener": event_listener(),
			"metadataUpdate": metadata_update("MANGA")
		},
		"kavita": {
			"baseUri": "",
			"eventListener": event_listener(),
			"metadataUpdate": metadata_update("MANGA")
		},
		"notifications": {
			"apprise": {"urls": null, "seriesCover": false},
			"discord": {"webhooks": null, "seriesCover": false}
		},
		"metadataProviders": {
			"malClientId": null,
			"comicVineClientId": null,
			"comicVineSearchLimit": null,
			"comicVineIssueName": null,
			"comicVineIdFormat": null,
			"nameMatchingMode": "CLOSEST_MATCH",
			"defaultProviders": providers,
			"libraryProviders": {},
			"mangaBakaDatabase": null,
			"bookWalkerDownloadDate": null
		}
	})
}

fn provider_config(enabled: bool, has_books: bool, extra: &str) -> Value {
	let mut config = json!({
		"priority": 10,
		"enabled": enabled,
		"seriesMetadata": series_fields(),
		"nameMatchingMode": null,
		"mediaType": if extra == "API" { "MANGA" } else { extra },
		"authorRoles": ["WRITER"],
		"artistRoles": artist_roles()
	});
	if has_books {
		config["bookMetadata"] = book_fields();
	} else {
		config["mode"] = json!(extra);
	}
	config
}

fn series_fields() -> Value {
	json!({
		"status": true,
		"title": true,
		"summary": true,
		"publisher": true,
		"readingDirection": true,
		"ageRating": true,
		"language": true,
		"genres": true,
		"tags": true,
		"totalBookCount": true,
		"authors": true,
		"releaseDate": true,
		"thumbnail": true,
		"links": true,
		"books": true,
		"useOriginalPublisher": false,
		"originalPublisherTagName": "",
		"englishPublisherTagName": "",
		"frenchPublisherTagName": ""
	})
}

fn book_fields() -> Value {
	json!({
		"title": true,
		"summary": true,
		"number": true,
		"numberSort": true,
		"releaseDate": true,
		"authors": true,
		"tags": true,
		"isbn": true,
		"links": true,
		"thumbnail": true
	})
}

fn artist_roles() -> Value {
	json!(["PENCILLER", "INKER", "COLORIST", "LETTERER", "COVER"])
}

fn event_listener() -> Value {
	json!({
		"enabled": false,
		"metadataLibraryFilter": [],
		"metadataSeriesExcludeFilter": [],
		"notificationsLibraryFilter": []
	})
}

fn metadata_update(library_type: &str) -> Value {
	json!({
		"default": {
			"libraryType": library_type,
			"aggregate": false,
			"mergeTags": false,
			"mergeGenres": false,
			"bookCovers": false,
			"seriesCovers": false,
			"overrideExistingCovers": false,
			"lockCovers": false,
			"updateModes": [],
			"postProcessing": {
				"seriesTitle": false,
				"seriesTitleLanguage": null,
				"alternativeSeriesTitles": false,
				"alternativeSeriesTitleLanguages": [],
				"orderBooks": false,
				"readingDirectionValue": null,
				"languageValue": null,
				"fallbackToAltTitle": false,
				"scoreTagName": null,
				"originalPublisherTagName": null,
				"publisherTagNames": []
			}
		},
		"library": {}
	})
}
#[cfg(test)]
mod tests {
	use std::{
		sync::{
			atomic::{AtomicUsize, Ordering},
			Arc,
		},
		time::Duration,
	};

	use futures_util::StreamExt;
	use metadata_integrations::{
		ExternalMetadata, ExternalSeriesMetadata, MatchCandidate, MetadataField,
	};
	use migrations::{Migrator, MigratorTrait};
	use models::entity::series_metadata;
	use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};
	use serde_json::json;
	use stump_core::config::StumpConfig;
	use tests::fake_data;
	use uuid::Uuid;

	use super::{search_titles, JobLimits, JobStore, KomfBackendAdapter};

	const OWNER: &str = "owner";

	fn limits() -> JobLimits {
		JobLimits {
			max_finished_jobs: 100,
			finished_job_max_age: chrono::Duration::hours(1),
			max_event_history: 64,
			max_concurrent_jobs: 2,
		}
	}

	async fn wait_until_finished(store: &JobStore, job_ids: &[String]) {
		for job_id in job_ids {
			for _ in 0..500 {
				let job = store
					.get(OWNER, job_id)
					.expect("job lookup")
					.expect("job exists while waiting");
				if job.finished_at.is_some() {
					break;
				}
				tokio::time::sleep(Duration::from_millis(5)).await;
			}
		}
	}

	#[test]
	fn search_uses_local_series_title_when_name_is_missing_or_blank() {
		let titles = search_titles(None, Some("Local title".into()));
		assert_eq!(titles.query, "Local title");
		assert_eq!(titles.reference, "Local title");

		let titles = search_titles(Some("   "), Some("Local title".into()));
		assert_eq!(titles.query, "Local title");

		let titles = search_titles(Some("Typed name"), Some("Local title".into()));
		assert_eq!(titles.query, "Typed name");
		assert_eq!(
			titles.reference, "Local title",
			"ranking still uses the local series as reference"
		);

		let titles = search_titles(Some(" Typed name "), None);
		assert_eq!(titles.query, "Typed name");
		assert_eq!(titles.reference, "Typed name");
	}

	#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
	async fn job_store_caps_concurrent_provider_work() {
		let store = JobStore::with_limits(limits());
		let active = Arc::new(AtomicUsize::new(0));
		let peak = Arc::new(AtomicUsize::new(0));
		let mut job_ids = Vec::new();
		for index in 0..6 {
			let active = active.clone();
			let peak = peak.clone();
			let response = store
				.start(
					OWNER.into(),
					format!("series-{index}"),
					move |_job| async move {
						let now = active.fetch_add(1, Ordering::SeqCst) + 1;
						peak.fetch_max(now, Ordering::SeqCst);
						tokio::time::sleep(Duration::from_millis(40)).await;
						active.fetch_sub(1, Ordering::SeqCst);
						Ok("done".into())
					},
				)
				.expect("job starts");
			job_ids.push(response.job_id);
		}
		wait_until_finished(&store, &job_ids).await;
		assert!(
			peak.load(Ordering::SeqCst) <= 2,
			"peak concurrency {} exceeded the limit",
			peak.load(Ordering::SeqCst)
		);
		let page = store.page(OWNER, Some("COMPLETED"), 0, 1000).expect("page");
		assert_eq!(page.content.len(), 6, "every queued job still completes");
	}

	#[tokio::test]
	async fn job_store_evicts_oldest_finished_jobs_beyond_count_cap() {
		let store = JobStore::with_limits(JobLimits {
			max_finished_jobs: 2,
			..limits()
		});
		let mut job_ids = Vec::new();
		for index in 0..4 {
			let response = store
				.start(OWNER.into(), format!("series-{index}"), |_job| async {
					Ok("done".into())
				})
				.expect("job starts");
			job_ids.push(response.job_id);
			wait_until_finished(&store, &job_ids[index..]).await;
			// Keep `finished_at` strictly increasing so eviction order is exact.
			tokio::time::sleep(Duration::from_millis(2)).await;
		}
		let running = store
			.start(OWNER.into(), "series-running".into(), |_job| async {
				tokio::time::sleep(Duration::from_secs(30)).await;
				Ok("done".into())
			})
			.expect("job starts");

		let page = store.page(OWNER, None, 0, 1000).expect("page");
		let listed: Vec<_> = page.content.iter().map(|job| job.id.as_str()).collect();
		assert_eq!(
			page.content.len(),
			3,
			"two finished jobs plus the running one"
		);
		assert!(listed.contains(&running.job_id.as_str()));
		assert!(listed.contains(&job_ids[3].as_str()));
		assert!(listed.contains(&job_ids[2].as_str()));
		assert_eq!(store.get(OWNER, &job_ids[0]).expect("lookup"), None);
		assert_eq!(store.get(OWNER, &job_ids[1]).expect("lookup"), None);
		assert!(
			store.events(OWNER, &job_ids[0]).is_none(),
			"event history of evicted jobs is released"
		);
	}

	#[tokio::test]
	async fn job_store_expires_finished_jobs_by_age() {
		let store = JobStore::with_limits(limits());
		let response = store
			.start(OWNER.into(), "series".into(), |_job| async {
				Ok("done".into())
			})
			.expect("job starts");
		wait_until_finished(&store, std::slice::from_ref(&response.job_id)).await;
		assert_eq!(
			store.page(OWNER, None, 0, 10).expect("page").content.len(),
			1
		);

		let id = Uuid::parse_str(&response.job_id).expect("job id");
		let record = store.records.read().expect("records")[&id].clone();
		record.update(|job| {
			job.finished_at = Some(chrono::Utc::now() - chrono::Duration::hours(2));
		});

		assert!(store
			.page(OWNER, None, 0, 10)
			.expect("page")
			.content
			.is_empty());
		assert_eq!(store.get(OWNER, &response.job_id).expect("lookup"), None);
	}

	#[tokio::test]
	async fn job_event_history_is_bounded_and_keeps_completion_marker() {
		let store = JobStore::with_limits(JobLimits {
			max_event_history: 3,
			..limits()
		});
		let response = store
			.start(OWNER.into(), "series".into(), |job| async move {
				for index in 0..5 {
					job.emit("ProviderSeriesEvent", json!({ "index": index }));
				}
				Ok("done".into())
			})
			.expect("job starts");
		wait_until_finished(&store, std::slice::from_ref(&response.job_id)).await;

		let events: Vec<_> = store
			.events(OWNER, &response.job_id)
			.expect("event stream")
			.collect()
			.await;
		let names: Vec<_> = events.iter().map(|event| event.name.as_str()).collect();
		assert_eq!(
			names,
			[
				"ProviderSeriesEvent",
				"ProviderSeriesEvent",
				super::JOB_COMPLETED_EVENT
			]
		);
		assert_eq!(events[0].data, Some(json!({ "index": 3 })));
		assert_eq!(events[1].data, Some(json!({ "index": 4 })));
	}

	#[tokio::test]
	async fn identify_metadata_uses_native_storage_and_respects_series_locks() {
		let db = Database::connect("sqlite::memory:")
			.await
			.expect("connect in-memory sqlite");
		Migrator::up(&db, None)
			.await
			.expect("migrate Komf metadata test database");
		let library = fake_data::Library {
			id: Some("identify-lock-library".into()),
			..Default::default()
		}
		.insert(&db)
		.await;
		let series = fake_data::Series {
			id: Some("identify-lock-series".into()),
			name: Some("Local title".into()),
			library_id: Some(library.id),
			..Default::default()
		}
		.insert(&db)
		.await;
		series_metadata::ActiveModel {
			series_id: Set(series.id.clone()),
			title: Set(None),
			summary: Set(None),
			locked_fields: Set(Some(json!([MetadataField::Title]))),
			..Default::default()
		}
		.insert(&db)
		.await
		.expect("insert locked native metadata");

		let ctx = Arc::new(stump_core::Ctx::for_testing_with_config(
			db,
			StumpConfig::debug(),
		));
		let adapter = KomfBackendAdapter::new(ctx);
		let candidate = MatchCandidate {
			provider: "MAL".into(),
			external_id: "42".into(),
			metadata: ExternalMetadata::Series(ExternalSeriesMetadata {
				provider: "MAL".into(),
				external_id: "42".into(),
				title: "Provider title".into(),
				summary: Some("Provider summary".into()),
				..Default::default()
			}),
			confidence: 1.0,
			confidence_factors: Vec::new(),
		};
		adapter
			.apply_candidate(&series.id, &candidate)
			.await
			.expect("apply selected provider metadata");

		let stored = series_metadata::Entity::find_by_id(series.id)
			.one(adapter.conn())
			.await
			.expect("query native series metadata")
			.expect("native metadata row remains");
		assert_eq!(stored.title, None, "locked title must remain empty");
		assert_eq!(stored.summary.as_deref(), Some("Provider summary"));
		assert_eq!(stored.metadata_source.as_deref(), Some("MAL"));
		assert_eq!(stored.metadata_external_id.as_deref(), Some("42"));
	}
}
