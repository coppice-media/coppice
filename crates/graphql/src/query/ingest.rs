use crate::{
	data::CoreContext,
	guard::PermissionGuard,
	object::ingest::{
		IngestAnalysisJob, IngestDropFolder, IngestDropItem, IngestMediaKind,
		IngestMetadataCandidate, IngestProviderDescriptor, IngestProviderSettings,
		IngestQualityCheckDescriptor, IngestQualityCheckSettings, IngestQualityReport,
		IngestReworkItem, IngestReworkReason, IngestSearchHit,
		PaginatedIngestAnalysisJobResponse, PaginatedIngestDropItemResponse,
		PaginatedIngestReworkResponse,
	},
	pagination::{
		CursorPaginationInfo, OffsetPaginationInfo, Pagination, PaginationInfo,
	},
};
use async_graphql::{Context, Object, Result, ID};
use models::shared::enums::{JobStatus, UserPermission};
use sea_orm::EntityTrait;
use stump_ingest::contract::SearchQuery;
use stump_ingest::store::Pagination as StorePagination;
use stump_ingest::policy;

use crate::object::metadata_policy::MetadataPolicy;

#[derive(Default)]
pub struct IngestQuery;

enum PageShape {
	Cursor {
		after: Option<String>,
		limit: u64,
	},
	Offset {
		input: crate::pagination::OffsetPagination,
	},
}

fn store_page(pagination: Pagination) -> Result<(StorePagination, PageShape)> {
	match pagination.resolve() {
		Pagination::Offset(input) => {
			let page_size = input.page_size.unwrap_or(20);
			Ok((
				StorePagination {
					page: input.page,
					page_size,
				},
				PageShape::Offset { input },
			))
		},
		Pagination::None(_) => Ok((
			StorePagination {
				page: 1,
				page_size: 1_000_000,
			},
			PageShape::Offset {
				input: crate::pagination::OffsetPagination {
					page: 1,
					page_size: None,
					zero_based: Some(false),
				},
			},
		)),
		Pagination::Cursor(input) => Ok((
			StorePagination {
				page: 1,
				page_size: input.limit,
			},
			PageShape::Cursor {
				after: input.after,
				limit: input.limit,
			},
		)),
	}
}

fn page_info(shape: PageShape, total: u64, ids: &[String]) -> PaginationInfo {
	match shape {
		PageShape::Offset { input } if input.page_size.is_none() => {
			OffsetPaginationInfo::unpaged(total).into()
		},
		PageShape::Offset { input } => OffsetPaginationInfo::new(input, total).into(),
		PageShape::Cursor { after, limit } => {
			let current_cursor = after.or_else(|| ids.first().cloned());
			let next_cursor = (ids.len() as u64 == limit)
				.then(|| ids.last().cloned())
				.flatten();
			CursorPaginationInfo {
				current_cursor,
				next_cursor,
				limit,
			}
			.into()
		},
	}
}

fn map_store_error<E: std::fmt::Display>(error: E) -> async_graphql::Error {
	async_graphql::Error::new(error.to_string())
}

async fn rework_reasons(
	core: &CoreContext,
	item: &models::entity::ingest_drop_item::Model,
) -> Result<Vec<IngestReworkReason>> {
	let Some(report_id) = &item.quality_report_id else {
		return Ok(Vec::new());
	};
	let Some(report) =
		models::entity::ingest_quality_report::Entity::find_by_id(report_id)
			.one(core.conn.as_ref())
			.await?
	else {
		return Ok(Vec::new());
	};
	let checks = IngestQualityReport::checks_from_json(&report.checks);
	Ok(checks
		.into_iter()
		.filter(|check| {
			!matches!(
				check.outcome.status,
				stump_ingest::contract::QualityStatus::Pass
					| stump_ingest::contract::QualityStatus::NotApplicable
			)
		})
		.map(|check| IngestReworkReason {
			check_id: check.outcome.check_id,
			status: check.outcome.status.into(),
			message: check.outcome.label,
		})
		.collect())
}

#[Object]
impl IngestQuery {
	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::UploadFile, UserPermission::ManageLibrary])"
	)]
	async fn ingest_drop_folder(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
	) -> Result<Option<IngestDropFolder>> {
		let core = ctx.data::<CoreContext>()?;
		let services = core.ingest();
		let library_id = library_id.to_string();
		let info = services
			.store
			.drop_folder(&library_id)
			.await
			.map_err(map_store_error)?;
		Ok(Some(IngestDropFolder {
			library_id: library_id.into(),
			display_path: info.path_display,
			enabled: true,
			pending_count: info.pending_files.min(i32::MAX as u32) as i32,
			last_discovered_at: None,
		}))
	}

	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::ReadJobs, UserPermission::MetadataFetchRecordRead])"
	)]
	async fn ingest_drop_items(
		&self,
		ctx: &Context<'_>,
		library_id: Option<ID>,
		status: Option<crate::object::ingest::IngestDropItemStatus>,
		#[graphql(default, validator(custom = "crate::pagination::PaginationValidator"))]
		pagination: Pagination,
	) -> Result<PaginatedIngestDropItemResponse> {
		let core = ctx.data::<CoreContext>()?;
		let services = core.ingest();
		let library_id = library_id.map(|id| id.to_string());
		let (page, shape) = store_page(pagination)?;
		let status = status.map(Into::into);
		let (models, total) = services
			.store
			.list_items(library_id.as_deref(), status, page)
			.await
			.map_err(map_store_error)?;
		let ids = models
			.iter()
			.map(|item| item.id.clone())
			.collect::<Vec<_>>();
		Ok(PaginatedIngestDropItemResponse {
			nodes: models.into_iter().map(IngestDropItem::from).collect(),
			page_info: page_info(shape, total, &ids),
		})
	}

	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::ReadJobs, UserPermission::MetadataFetchRecordRead])"
	)]
	async fn ingest_item(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<Option<IngestDropItem>> {
		let core = ctx.data::<CoreContext>()?;
		let item = core
			.ingest()
			.store
			.item(id.as_ref())
			.await
			.map_err(map_store_error)?;
		Ok(item.map(IngestDropItem::from))
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ReadJobs)")]
	async fn ingest_analysis_queue(
		&self,
		ctx: &Context<'_>,
		status: Option<JobStatus>,
		#[graphql(default, validator(custom = "crate::pagination::PaginationValidator"))]
		pagination: Pagination,
	) -> Result<PaginatedIngestAnalysisJobResponse> {
		let core = ctx.data::<CoreContext>()?;
		let (page, shape) = store_page(pagination)?;
		let (models, total) = core
			.ingest()
			.coordinator
			.queue(status, page)
			.await
			.map_err(map_store_error)?;
		let ids = models.iter().map(|job| job.id.clone()).collect::<Vec<_>>();
		Ok(PaginatedIngestAnalysisJobResponse {
			nodes: models.into_iter().map(IngestAnalysisJob::from).collect(),
			page_info: page_info(shape, total, &ids),
		})
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ReadJobs)")]
	async fn ingest_analysis_job(
		&self,
		ctx: &Context<'_>,
		id: ID,
	) -> Result<Option<IngestAnalysisJob>> {
		let core = ctx.data::<CoreContext>()?;
		Ok(core
			.ingest()
			.coordinator
			.job(id.as_ref())
			.await
			.map_err(map_store_error)?
			.map(IngestAnalysisJob::from))
	}

	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::ReadJobs, UserPermission::MetadataFetchRecordRead])"
	)]
	async fn ingest_rework_items(
		&self,
		ctx: &Context<'_>,
		min_score: Option<f64>,
		#[graphql(default, validator(custom = "crate::pagination::PaginationValidator"))]
		pagination: Pagination,
	) -> Result<PaginatedIngestReworkResponse> {
		let min_score = min_score.unwrap_or(100.0);
		if !min_score.is_finite() || !(0.0..=100.0).contains(&min_score) {
			return Err("minScore must be between 0 and 100".into());
		}
		let core = ctx.data::<CoreContext>()?;
		let (page, shape) = store_page(pagination)?;
		let (models, total) = core
			.ingest()
			.store
			.rework_items(min_score.ceil() as u8, page)
			.await
			.map_err(map_store_error)?;
		let mut nodes = Vec::with_capacity(models.len());
		let mut ids = Vec::with_capacity(models.len());
		for model in models {
			ids.push(model.id.clone());
			nodes.push(IngestReworkItem {
				reasons: rework_reasons(core, &model).await?,
				item: IngestDropItem::from(model),
			});
		}
		Ok(PaginatedIngestReworkResponse {
			nodes,
			page_info: page_info(shape, total, &ids),
		})
	}

	#[graphql(
		guard = "PermissionGuard::new(&[UserPermission::ReadJobs, UserPermission::MetadataFetchRecordRead])"
	)]
	async fn ingest_bulk_items(
		&self,
		ctx: &Context<'_>,
		ids: Vec<ID>,
	) -> Result<Vec<IngestDropItem>> {
		let ids = ids.into_iter().map(|id| id.to_string()).collect::<Vec<_>>();
		Ok(ctx
			.data::<CoreContext>()?
			.ingest()
			.store
			.bulk_items(&ids)
			.await
			.map_err(map_store_error)?
			.into_iter()
			.map(IngestDropItem::from)
			.collect())
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataProviderRead)")]
	async fn ingest_provider_catalog(
		&self,
		ctx: &Context<'_>,
		#[graphql(default = false)] include_disabled: bool,
	) -> Result<Vec<IngestProviderDescriptor>> {
		let _ = include_disabled;
		let catalog = ctx
			.data::<CoreContext>()?
			.ingest()
			.providers
			.catalog()
			.await;
		Ok(catalog
			.into_iter()
			.map(IngestProviderDescriptor::from)
			.collect())
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataProviderRead)")]
	async fn ingest_provider_settings(
		&self,
		ctx: &Context<'_>,
		provider_id: String,
	) -> Result<Option<IngestProviderSettings>> {
		let core = ctx.data::<CoreContext>()?;
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let services = core.ingest();
		let Some(descriptor) = services
			.providers
			.catalog()
			.await
			.into_iter()
			.find(|descriptor| descriptor.id == provider_id)
		else {
			return Ok(None);
		};
		let model = services
			.store
			.plugin_settings(&provider_id, "PROVIDER", None, Some(&auth.user.id))
			.await
			.map_err(map_store_error)?;
		Ok(Some(IngestProviderSettings::from_parts(
			IngestProviderDescriptor::from(descriptor),
			model,
		)))
	}
	#[graphql(guard = "PermissionGuard::one(UserPermission::ManageJobs)")]
	async fn ingest_provider_search(
		&self,
		ctx: &Context<'_>,
		query: String,
		media_kind: Option<IngestMediaKind>,
		providers: Option<Vec<String>>,
		#[graphql(default = 10)] limit: Option<i32>,
	) -> Result<Vec<IngestSearchHit>> {
		let text = query.trim();
		if text.is_empty() {
			return Err("query must not be empty".into());
		}
		let limit = limit.unwrap_or(10).clamp(1, 50) as u8;
		let core = ctx.data::<CoreContext>()?;
		let hits = core
			.ingest()
			.providers
			.search(
				&SearchQuery {
					text: text.to_string(),
					media_kind: media_kind.map(Into::into),
					limit,
				},
				providers.as_deref(),
			)
			.await;
		Ok(hits.into_iter().map(IngestSearchHit::from).collect())
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ReadJobs)")]
	async fn ingest_quality_check_catalog(
		&self,
		ctx: &Context<'_>,
		#[graphql(default = false)] include_disabled: bool,
	) -> Result<Vec<IngestQualityCheckDescriptor>> {
		let _ = include_disabled;
		let catalog = ctx.data::<CoreContext>()?.ingest().quality.catalog();
		Ok(catalog
			.into_iter()
			.map(IngestQualityCheckDescriptor::from)
			.collect())
	}

	#[graphql(guard = "PermissionGuard::one(UserPermission::ReadJobs)")]
	async fn ingest_quality_check_settings(
		&self,
		ctx: &Context<'_>,
		check_id: String,
	) -> Result<Option<IngestQualityCheckSettings>> {
		let core = ctx.data::<CoreContext>()?;
		let auth = ctx.data::<stump_auth::AuthContext>()?;
		let services = core.ingest();
		let Some(descriptor) = services
			.quality
			.catalog()
			.into_iter()
			.find(|descriptor| descriptor.id == check_id)
		else {
			return Ok(None);
		};
		let model = services
			.store
			.plugin_settings(&check_id, "CHECK", None, Some(&auth.user.id))
			.await
			.map_err(map_store_error)?;
		Ok(Some(IngestQualityCheckSettings::from_parts(
			IngestQualityCheckDescriptor::from(descriptor),
			model,
		)))
	}

	/// Latest quality report stored against a library media row (library-wide
	/// rework). Batch a table column with GraphQL aliases: one field per
	/// media id in a single document.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ReadJobs)")]
	async fn ingest_media_quality_report(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
	) -> Result<Option<IngestQualityReport>> {
		ctx.data::<CoreContext>()?
			.ingest()
			.store
			.report_for_media(media_id.as_ref())
			.await
			.map(|report| report.map(IngestQualityReport::from))
			.map_err(map_store_error)
	}

	/// Metadata candidates stored against a library media row by a
	/// `matchLibraryMedia` run.
	#[graphql(guard = "PermissionGuard::one(UserPermission::ReadJobs)")]
	async fn ingest_media_metadata_candidates(
		&self,
		ctx: &Context<'_>,
		media_id: ID,
	) -> Result<Vec<IngestMetadataCandidate>> {
		ctx.data::<CoreContext>()?
			.ingest()
			.store
			.candidates_for_media(media_id.as_ref())
			.await
			.map(|candidates| {
				candidates
					.into_iter()
					.map(IngestMetadataCandidate::from)
					.collect()
			})
			.map_err(map_store_error)
	}

	/// The effective per-field metadata policy for one library: the server
	/// default with the library's stored override applied, one row per field
	/// the policy vocabulary covers.
	#[graphql(guard = "PermissionGuard::one(UserPermission::MetadataProviderRead)")]
	async fn metadata_policy(
		&self,
		ctx: &Context<'_>,
		library_id: ID,
	) -> Result<MetadataPolicy> {
		let core = ctx.data::<CoreContext>()?;
		policy::effective_policy(core.conn.as_ref(), library_id.as_ref())
			.await
			.map(MetadataPolicy::from)
			.map_err(map_store_error)
	}
}

#[cfg(test)]
mod tests {

	/// The provider search surface is consumed by the editor's generated
	/// GraphQL client; these tests pin the exact operation signatures.
	#[test]
	fn ingest_provider_search_sdl_is_stable() {
		let sdl = crate::schema::build_schema_bare().sdl();
		assert!(
			sdl.contains(
				"ingestProviderSearch(query: String!, mediaKind: IngestMediaKind, \
				 providers: [String!], limit: Int = 10): [IngestSearchHit!]!"
			),
			"ingestProviderSearch signature drifted:\n{sdl}"
		);
	}

	#[test]
	fn lookup_ingest_candidate_sdl_is_stable() {
		let sdl = crate::schema::build_schema_bare().sdl();
		assert!(
			sdl.contains(
				"lookupIngestCandidate(dropItemId: ID!, providerId: String!, \
				 externalId: String!): IngestMetadataCandidate!"
			),
			"lookupIngestCandidate signature drifted:\n{sdl}"
		);
	}

	/// The editor's Policy tab generates its client from these three
	/// signatures.
	#[test]
	fn metadata_policy_sdl_is_stable() {
		let sdl = crate::schema::build_schema_bare().sdl();
		for signature in [
			"metadataPolicy(libraryId: ID!): MetadataPolicy!",
			"setMetadataPolicy(libraryId: ID!, input: MetadataPolicyInput!): \
			 MetadataPolicy!",
			"applyBestIngestMetadata(dropItemId: ID, mediaId: ID): \
			 IngestApplyBestPayload!",
		] {
			assert!(
				sdl.contains(signature),
				"{signature} drifted:\n{sdl}"
			);
		}
	}
}
