use std::{collections::HashMap, sync::Arc};

use metadata_integrations::{
	MediaType, MetadataProvider as IntegrationMetadataProvider, MetadataProviderError,
};
use models::{
	entity::{ingest_plugin_setting, metadata_provider_config, server_config},
	shared::enums::MetadataProvider,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, SelectColumns};
use tokio::sync::OnceCell;

use crate::{
	config::StumpConfig,
	filesystem::metadata::ProviderClientCache,
	ingest::contract::{
		BookSnapshot, IngestMediaKind, IngestMetadataProvider, MetadataCandidate,
		ProviderCapability, ProviderError, ProviderIdentity, SearchHit, SearchQuery,
		SettingDefinition, SettingValues,
	},
};

use super::facade::search_all;
use super::{EmbeddedProvider, IntegrationProvider};

/// Discovery information shown to the editor.  `available` describes whether
/// this build contains an executable implementation; credential/configuration
/// failures are intentionally reported by the settings query and skipped by
/// scheduling rather than treated as book failures.
#[derive(Debug, Clone)]
pub struct ProviderDescriptor {
	pub id: String,
	pub name: String,
	pub version: String,
	pub available: bool,
	pub configured: bool,
	pub capabilities: Vec<ProviderCapability>,
	pub supported_media_kinds: Vec<IngestMediaKind>,
	/// String media names are retained for the GraphQL discovery surface.
	pub supported_media_types: Vec<String>,
	pub enabled_default: bool,
	pub settings: Vec<SettingDefinition>,
}

struct IntegrationSpec {
	id: &'static str,
	name: &'static str,
	provider_type: MetadataProvider,
	media_types: &'static [MediaType],
}

const COMIC_VINE_MEDIA_TYPES: &[MediaType] = &[MediaType::Comic];
const HARDCOVER_MEDIA_TYPES: &[MediaType] = &[MediaType::Book];
/// Registry of the two currently registered remote integrations.
const INTEGRATIONS: &[IntegrationSpec] = &[
	IntegrationSpec {
		id: "comic_vine",
		name: "ComicVine",
		provider_type: MetadataProvider::ComicVine,
		media_types: COMIC_VINE_MEDIA_TYPES,
	},
	IntegrationSpec {
		id: "hardcover",
		name: "Hardcover",
		provider_type: MetadataProvider::Hardcover,
		media_types: HARDCOVER_MEDIA_TYPES,
	},
];

/// Registry of built-in providers and adapters for configured integration
/// providers.  Remote clients are initialized lazily after the existing
/// encrypted provider configuration has been read.
pub struct ProviderRegistry {
	_config: Arc<StumpConfig>,
	conn: Arc<sea_orm::DatabaseConnection>,
	embedded: Arc<EmbeddedProvider>,
	cache: Arc<OnceCell<Arc<ProviderClientCache>>>,
}

impl ProviderRegistry {
	pub fn new(config: Arc<StumpConfig>, conn: Arc<sea_orm::DatabaseConnection>) -> Self {
		Self {
			_config: config,
			conn,
			embedded: Arc::new(EmbeddedProvider::new()),
			cache: Arc::new(OnceCell::new()),
		}
	}

	/// Return deterministic provider discovery information.  `available` is a
	/// build-time property, while `configured` is derived from the existing
	/// encrypted metadata-provider rows.
	pub async fn catalog(&self) -> Vec<ProviderDescriptor> {
		let configs = self.provider_configs().await;
		let unknown_ids = self.unknown_plugin_ids().await;
		let configured = |provider_type: MetadataProvider| {
			configs.iter().any(|config| {
				config.provider_type == provider_type
					&& config.encrypted_api_token.is_some()
			})
		};
		let mut descriptors = vec![ProviderDescriptor {
			id: self.embedded.id().to_string(),
			name: self.embedded.name().to_string(),
			version: self.embedded.version().to_string(),
			available: true,
			configured: true,
			capabilities: self.embedded.capabilities().to_vec(),
			supported_media_kinds: self.embedded.supported_media_kinds().to_vec(),
			supported_media_types: vec![
				"COMIC".to_string(),
				"BOOK".to_string(),
				"MANGA".to_string(),
				"LIGHT_NOVEL".to_string(),
				"MANHWA".to_string(),
				"WEB_NOVEL".to_string(),
				"WEBTOON".to_string(),
			],
			enabled_default: true,
			settings: self.embedded.settings().to_vec(),
		}];
		for spec in INTEGRATIONS {
			let kinds = spec
				.media_types
				.iter()
				.flat_map(|media_type| match media_type {
					MediaType::Comic => {
						vec![
							IngestMediaKind::ComicArchive,
							IngestMediaKind::ComicRarArchive,
						]
					},
					MediaType::Book
					| MediaType::Manga
					| MediaType::LightNovel
					| MediaType::Manhwa
					| MediaType::WebNovel
					| MediaType::Webtoon => vec![IngestMediaKind::Epub, IngestMediaKind::Pdf],
				})
				.collect::<Vec<_>>();
			descriptors.push(ProviderDescriptor {
				id: spec.id.to_string(),
				name: spec.name.to_string(),
				version: super::facade::INTEGRATION_PROVIDER_VERSION.to_string(),
				available: true,
				configured: configured(spec.provider_type),
				capabilities: vec![
					ProviderCapability::Identify,
					ProviderCapability::Lookup,
					ProviderCapability::Search,
				],
				supported_media_kinds: kinds,
				supported_media_types: spec
					.media_types
					.iter()
					.map(|media_type| format!("{media_type:?}").to_uppercase())
					.collect(),
				enabled_default: false,
				settings: Vec::new(),
			});
		}
		let llm = super::llm::LlmProvider::new();
		let llm_configured = self.has_plugin_setting(super::llm::LLM_PROVIDER_ID).await;
		// `available` stays false until a settings row exists: without a
		// base_url/model the provider cannot execute anything.
		descriptors.push(ProviderDescriptor {
			id: super::llm::LLM_PROVIDER_ID.to_string(),
			name: llm.name().to_string(),
			version: super::llm::LLM_PROVIDER_VERSION.to_string(),
			available: llm_configured,
			configured: llm_configured,
			capabilities: llm.capabilities().to_vec(),
			supported_media_kinds: llm.supported_media_kinds().to_vec(),
			supported_media_types: vec![
				"COMIC".to_string(),
				"BOOK".to_string(),
				"MANGA".to_string(),
				"LIGHT_NOVEL".to_string(),
				"MANHWA".to_string(),
				"WEB_NOVEL".to_string(),
				"WEBTOON".to_string(),
			],
			enabled_default: false,
			settings: llm.settings().to_vec(),
		});
		for id in unknown_ids {
			descriptors.push(ProviderDescriptor {
				name: id.clone(),
				id,
				version: "unknown".to_string(),
				available: false,
				configured: true,
				capabilities: Vec::new(),
				supported_media_kinds: Vec::new(),
				supported_media_types: Vec::new(),
				enabled_default: false,
				settings: Vec::new(),
			});
		}
		descriptors.sort_by(|left, right| left.id.cmp(&right.id));
		descriptors
	}

	/// Identify and expand the best match from every enabled/configured
	/// provider.  One provider's failures are isolated and never fail the
	/// snapshot as a whole.
	pub async fn identify_and_lookup(
		&self,
		book: &BookSnapshot,
		library_id: &str,
		user_id: Option<&str>,
	) -> Vec<MetadataCandidate> {
		self.identify_and_lookup_selected(book, library_id, user_id, None)
			.await
	}

	/// [`Self::identify_and_lookup`] restricted to an allowlist of provider
	/// ids; `None` selects every enabled provider.  Used by library-wide
	/// match jobs so a batch only contacts the providers the caller picked.
	pub async fn identify_and_lookup_selected(
		&self,
		book: &BookSnapshot,
		library_id: &str,
		user_id: Option<&str>,
		selected: Option<&[String]>,
	) -> Vec<MetadataCandidate> {
		let _ = (library_id, user_id);
		let selected_provider = |provider_id: &str| {
			selected.is_none_or(|ids| ids.iter().any(|id| id == provider_id))
		};
		let mut candidates = Vec::new();
		let settings = SettingValues::new();

		if selected_provider(self.embedded.id())
			&& self
				.embedded
				.supported_media_kinds()
				.contains(&book.media_kind)
		{
			if let Ok(identities) = self.embedded.identify(book, &settings).await {
				if let Some(identity) = best_identity(identities) {
					match self.embedded.lookup(book, &identity, &settings).await {
						Ok(mut found) => candidates.append(&mut found),
						Err(error) => tracing::warn!(
							provider = self.embedded.id(),
							?error,
							"Embedded provider lookup failed"
						),
					}
				}
			}
		}

		let configs: HashMap<_, _> = self
			.provider_configs()
			.await
			.into_iter()
			.map(|config| (config.provider_type, config))
			.collect();

		for spec in INTEGRATIONS {
			if !supported_by_snapshot(spec.media_types, book.media_kind) {
				continue;
			}
			if !selected_provider(spec.id) {
				continue;
			}
			let Some(config) = configs.get(&spec.provider_type) else {
				continue;
			};
			if !config.enabled {
				tracing::debug!(
					provider = spec.id,
					"Skipping disabled metadata provider"
				);
				continue;
			}
			if config.encrypted_api_token.is_none() {
				tracing::debug!(
					provider = spec.id,
					"Skipping provider without credentials"
				);
				continue;
			}
			let client = match self.client_for(config).await {
				Ok(client) => client,
				Err(error) => {
					tracing::warn!(
						provider = spec.id,
						?error,
						"Skipping unavailable metadata provider"
					);
					continue;
				},
			};
			let provider = IntegrationProvider::new(client);
			let identities = match provider.identify(book, &settings).await {
				Ok(identities) => identities,
				Err(error) => {
					tracing::warn!(
						provider = spec.id,
						?error,
						"Provider identification failed"
					);
					continue;
				},
			};
			let Some(identity) = best_identity(identities) else {
				continue;
			};
			match provider.lookup(book, &identity, &settings).await {
				Ok(mut found) => candidates.append(&mut found),
				Err(error) => tracing::warn!(
					provider = spec.id,
					?error,
					"Provider metadata lookup failed"
				),
			}
		}
		candidates
	}

	/// Free-text search across the enabled/configured integration providers,
	/// optionally restricted to `providers`.  Failures are isolated; hits are
	/// merged and sorted by score descending (see [`search_all`]).
	pub async fn search(
		&self,
		query: &SearchQuery,
		providers: Option<&[String]>,
	) -> Vec<SearchHit> {
		let mut adapters: Vec<Arc<dyn IngestMetadataProvider>> = Vec::new();
		let configs: HashMap<_, _> = self
			.provider_configs()
			.await
			.into_iter()
			.map(|config| (config.provider_type, config))
			.collect();

		for spec in INTEGRATIONS {
			if let Some(filter) = providers {
				if !filter.iter().any(|id| id == spec.id) {
					continue;
				}
			}
			if query
				.media_kind
				.is_some_and(|kind| !supported_by_snapshot(spec.media_types, kind))
			{
				continue;
			}
			let Some(config) = configs.get(&spec.provider_type) else {
				continue;
			};
			if !config.enabled {
				tracing::debug!(
					provider = spec.id,
					"Skipping disabled metadata provider"
				);
				continue;
			}
			if config.encrypted_api_token.is_none() {
				tracing::debug!(
					provider = spec.id,
					"Skipping provider without credentials"
				);
				continue;
			}
			let client = match self.client_for(config).await {
				Ok(client) => client,
				Err(error) => {
					tracing::warn!(
						provider = spec.id,
						?error,
						"Skipping unavailable metadata provider"
					);
					continue;
				},
			};
			adapters.push(Arc::new(IntegrationProvider::new(client)));
		}

		let enabled = adapters
			.iter()
			.map(|provider| provider.id().to_string())
			.collect::<Vec<_>>();
		search_all(&adapters, query, &enabled).await
	}

	/// Expand one identity (e.g. a picked search hit) into a full field-level
	/// candidate for the given snapshot.  Explicit and user-driven: unlike the
	/// scheduled identify flow this does not gate on the provider's enabled
	/// flag, only on it being configured.
	pub async fn lookup(
		&self,
		book: &BookSnapshot,
		identity: &ProviderIdentity,
	) -> Result<MetadataCandidate, ProviderError> {
		if identity.provider_id == self.embedded.id() {
			let mut found = self
				.embedded
				.lookup(book, identity, &SettingValues::new())
				.await?;
			return found.pop().ok_or_else(|| ProviderError::Request {
				provider_id: self.embedded.id().to_string(),
				message: "provider returned no candidates".to_string(),
			});
		}
		let spec = INTEGRATIONS
			.iter()
			.find(|spec| spec.id == identity.provider_id)
			.ok_or_else(|| ProviderError::NotConfigured {
				provider_id: identity.provider_id.clone(),
				message: "unknown provider".to_string(),
			})?;
		let config = metadata_provider_config::Entity::find()
			.filter(metadata_provider_config::Column::ProviderType.eq(spec.provider_type))
			.one(self.conn.as_ref())
			.await
			.map_err(|error| ProviderError::Request {
				provider_id: identity.provider_id.clone(),
				message: error.to_string(),
			})?
			.ok_or_else(|| ProviderError::NotConfigured {
				provider_id: identity.provider_id.clone(),
				message: "no metadata provider configuration exists".to_string(),
			})?;
		if config.encrypted_api_token.is_none() {
			return Err(ProviderError::NotConfigured {
				provider_id: identity.provider_id.clone(),
				message: "API credentials are missing".to_string(),
			});
		}
		let client = self.client_for(&config).await?;
		let mut found = IntegrationProvider::new(client)
			.lookup(book, identity, &SettingValues::new())
			.await?;
		found.pop().ok_or_else(|| ProviderError::Request {
			provider_id: identity.provider_id.clone(),
			message: "provider returned no candidates".to_string(),
		})
	}

	pub async fn verify(
		&self,
		provider_id: &str,
		_settings: &SettingValues,
	) -> Result<(), ProviderError> {
		if provider_id == self.embedded.id() {
			return Ok(());
		}
		let spec = INTEGRATIONS
			.iter()
			.find(|spec| spec.id == provider_id)
			.ok_or_else(|| ProviderError::NotConfigured {
				provider_id: provider_id.to_string(),
				message: "unknown provider".to_string(),
			})?;
		let config = metadata_provider_config::Entity::find()
			.filter(metadata_provider_config::Column::ProviderType.eq(spec.provider_type))
			.one(self.conn.as_ref())
			.await
			.map_err(|error| ProviderError::Request {
				provider_id: provider_id.to_string(),
				message: error.to_string(),
			})?
			.ok_or_else(|| ProviderError::NotConfigured {
				provider_id: provider_id.to_string(),
				message: "no metadata provider configuration exists".to_string(),
			})?;
		if config.encrypted_api_token.is_none() {
			return Err(ProviderError::NotConfigured {
				provider_id: provider_id.to_string(),
				message: "API credentials are missing".to_string(),
			});
		}
		let client = self.client_for(&config).await?;
		let verification = client
			.verify_credentials()
			.await
			.map_err(|error| map_cache_provider_error(provider_id, error))?;
		if verification.is_valid {
			Ok(())
		} else {
			Err(ProviderError::Request {
				provider_id: provider_id.to_string(),
				message: verification.error.unwrap_or_else(|| {
					format!(
						"credential verification failed (HTTP {})",
						verification.response_status
					)
				}),
			})
		}
	}
	async fn provider_configs(&self) -> Vec<metadata_provider_config::Model> {
		match metadata_provider_config::Entity::find()
			.all(self.conn.as_ref())
			.await
		{
			Ok(configs) => configs,
			Err(error) => {
				tracing::warn!(?error, "Unable to load metadata provider configuration");
				Vec::new()
			},
		}
	}

	async fn unknown_plugin_ids(&self) -> Vec<String> {
		let known: std::collections::HashSet<&str> = std::iter::once(self.embedded.id())
			.chain(INTEGRATIONS.iter().map(|spec| spec.id))
			.collect();
		match ingest_plugin_setting::Entity::find()
			.filter(ingest_plugin_setting::Column::Kind.eq("PROVIDER"))
			.all(self.conn.as_ref())
			.await
		{
			Ok(settings) => settings
				.into_iter()
				.map(|setting| setting.plugin_id)
				.filter(|id| !known.contains(id.as_str()))
				.collect::<std::collections::BTreeSet<_>>()
				.into_iter()
				.collect(),
			Err(error) => {
				tracing::warn!(
					?error,
					"Unable to load persisted ingest provider settings"
				);
				Vec::new()
			},
		}
	}

	/// Whether a persisted `PROVIDER`-kind ingest plugin setting row exists
	/// for `plugin_id`.  Used to derive the llm provider's configured state.
	async fn has_plugin_setting(&self, plugin_id: &str) -> bool {
		match ingest_plugin_setting::Entity::find()
			.filter(ingest_plugin_setting::Column::Kind.eq("PROVIDER"))
			.filter(ingest_plugin_setting::Column::PluginId.eq(plugin_id))
			.one(self.conn.as_ref())
			.await
		{
			Ok(Some(_)) => true,
			Ok(None) => false,
			Err(error) => {
				tracing::warn!(
					?error,
					"Unable to load persisted ingest provider settings"
				);
				false
			},
		}
	}

	async fn client_for(
		&self,
		config: &metadata_provider_config::Model,
	) -> Result<Arc<dyn IntegrationMetadataProvider + Send + Sync>, ProviderError> {
		let cache = self
			.cache
			.get_or_try_init(|| async {
				let encryption_key = server_config::Entity::find()
					.select_only()
					.select_column(server_config::Column::EncryptionKey)
					.into_model::<server_config::EncryptionKeySelect>()
					.one(self.conn.as_ref())
					.await
					.map_err(|error| error.to_string())?
					.and_then(|record| record.encryption_key)
					.ok_or_else(|| {
						"server encryption key is not configured".to_string()
					})?;
				Ok::<_, String>(Arc::new(ProviderClientCache::new(encryption_key)))
			})
			.await
			.map_err(|error| map_cache_error(config.provider_type, error))?;
		cache
			.get_or_create(config)
			.await
			.map_err(|error| map_cache_error(config.provider_type, error))
	}
}

fn best_identity(
	mut identities: Vec<crate::ingest::contract::ProviderIdentity>,
) -> Option<crate::ingest::contract::ProviderIdentity> {
	identities.retain(|identity| identity.confidence > 0.5);
	identities.sort_by(|left, right| {
		right
			.confidence
			.partial_cmp(&left.confidence)
			.unwrap_or(std::cmp::Ordering::Equal)
			.then_with(|| left.external_id.cmp(&right.external_id))
			.then_with(|| left.display.cmp(&right.display))
	});
	identities.into_iter().next()
}

fn supported_by_snapshot(media_types: &[MediaType], kind: IngestMediaKind) -> bool {
	media_types.iter().any(|media_type| match media_type {
		MediaType::Comic => matches!(
			kind,
			IngestMediaKind::ComicArchive | IngestMediaKind::ComicRarArchive
		),
		MediaType::Book
		| MediaType::Manga
		| MediaType::LightNovel
		| MediaType::Manhwa
		| MediaType::WebNovel
		| MediaType::Webtoon => matches!(kind, IngestMediaKind::Epub | IngestMediaKind::Pdf),
	})
}

fn provider_id(provider: MetadataProvider) -> &'static str {
	match provider {
		MetadataProvider::ComicVine => "comic_vine",
		MetadataProvider::Hardcover => "hardcover",
	}
}

fn map_cache_error<E: std::fmt::Display>(
	provider_type: MetadataProvider,
	error: E,
) -> ProviderError {
	let provider_id = provider_id(provider_type).to_string();
	ProviderError::NotConfigured {
		provider_id,
		message: error.to_string(),
	}
}

fn map_cache_provider_error(
	provider_id: &str,
	error: MetadataProviderError,
) -> ProviderError {
	if error.is_rate_limited() {
		ProviderError::RateLimited {
			provider_id: provider_id.to_string(),
		}
	} else {
		ProviderError::Request {
			provider_id: provider_id.to_string(),
			message: error.to_string(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn catalog_is_deterministically_sorted_and_has_embedded_provider() {
		let config = Arc::new(StumpConfig::debug());
		let conn = Arc::new(
			sea_orm::MockDatabase::new(sea_orm::DatabaseBackend::Sqlite)
				.append_query_results([Vec::<metadata_provider_config::Model>::new()])
				.append_query_results([Vec::<ingest_plugin_setting::Model>::new()])
				.append_query_results([Vec::<ingest_plugin_setting::Model>::new()])
				.into_connection(),
		);
		let registry = ProviderRegistry::new(config, conn);
		let catalog = registry.catalog().await;
		let ids: Vec<_> = catalog
			.iter()
			.map(|descriptor| descriptor.id.as_str())
			.collect();
		assert_eq!(
			ids,
			vec!["builtin:embedded", "comic_vine", "hardcover", "llm"]
		);
		assert!(catalog[0].enabled_default);
		// The llm provider is dormant until an ingest_plugin_setting row
		// exists for it.
		let llm = catalog.last().unwrap();
		assert_eq!(llm.id, "llm");
		assert!(!llm.enabled_default);
		assert!(!llm.available);
		assert!(!llm.configured);
	}

	#[test]
	fn best_identity_requires_confidence_above_half() {
		let identity = crate::ingest::contract::ProviderIdentity {
			provider_id: "provider".to_string(),
			external_id: "id".to_string(),
			display: "Book".to_string(),
			confidence: 0.5,
			factors: serde_json::Value::Null,
		};
		assert!(best_identity(vec![identity]).is_none());
	}
}
