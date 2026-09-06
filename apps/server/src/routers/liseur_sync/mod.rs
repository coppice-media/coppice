use crate::config::state::AppState;
use axum::{Extension, Router};
use stump_auth::AuthContext;
use stump_liseur_sync::{
	AnnotationInput, AnnotationRecord, AnnotationResult, AttachmentRecord,
	AttachmentUpload, AttachmentUploadResult, CatalogBook, CatalogBookSeries,
	CatalogBooksPage, CatalogCover, CatalogDownload, CatalogFoldersPage,
	CatalogResolveResult, ChangesPage, DeleteAnnotationResult, HeadsPage,
	LiseurSyncBackend, LiseurSyncError, LiseurToken, LoginResult, OpInput, OpRecord,
	OpResult, ResolveRequest, ResolveResult, SessionInput, TokenCreateResult,
};

mod attachments;
mod storage;
mod touch;

#[derive(Clone)]
pub(crate) struct Backend {
	ctx: AppState,
}

impl Backend {
	fn new(ctx: AppState) -> Self {
		Self { ctx }
	}
}

/// Mount the feature-gated native liseur-sync router.  The protocol crate
/// owns routing and bearer middleware; this adapter only supplies the Stump
/// database backend through an extension.  The device summary route shares
/// the same bearer credentials and is therefore mounted here.
pub(crate) fn mount(ctx: AppState) -> Router<AppState> {
	stump_liseur_sync::routes::<AppState, Backend>()
		.merge(touch::router())
		.merge(attachments::router(ctx.clone()))
		.layer(Extension(Backend::new(ctx)))
}

#[async_trait::async_trait]
impl LiseurSyncBackend for Backend {
	async fn login(
		&self,
		username: &str,
		password: &str,
	) -> Result<LoginResult, LiseurSyncError> {
		storage::login(&self.ctx, username, password).await
	}

	async fn authenticate(&self, token: &str) -> Result<LiseurToken, LiseurSyncError> {
		storage::authenticate(&self.ctx, token).await
	}
	async fn mint_token(
		&self,
		user_id: &str,
		name: &str,
		scopes: Vec<String>,
		expires_in_seconds: Option<i64>,
	) -> Result<TokenCreateResult, LiseurSyncError> {
		storage::mint_token(&self.ctx, user_id, name, scopes, expires_in_seconds).await
	}

	async fn revoke_token(
		&self,
		user_id: &str,
		token_id: &str,
	) -> Result<(), LiseurSyncError> {
		storage::revoke_token(&self.ctx, user_id, token_id).await
	}
	async fn folders(
		&self,
		auth: &AuthContext,
		after: Option<String>,
		limit: usize,
	) -> Result<CatalogFoldersPage, LiseurSyncError> {
		storage::folders(&self.ctx, auth, after, limit).await
	}

	async fn folder_books(
		&self,
		auth: &AuthContext,
		folder_id: &str,
		cursor: Option<String>,
		limit: usize,
	) -> Result<CatalogBooksPage, LiseurSyncError> {
		storage::folder_books(&self.ctx, auth, folder_id, cursor, limit).await
	}

	async fn folder_search(
		&self,
		auth: &AuthContext,
		folder_id: &str,
		query: &str,
	) -> Result<Vec<CatalogBook>, LiseurSyncError> {
		storage::folder_search(&self.ctx, auth, folder_id, query).await
	}

	async fn book(
		&self,
		auth: &AuthContext,
		book_id: &str,
	) -> Result<CatalogBook, LiseurSyncError> {
		storage::book(&self.ctx, auth, book_id).await
	}

	async fn book_download(
		&self,
		auth: &AuthContext,
		book_id: &str,
	) -> Result<CatalogDownload, LiseurSyncError> {
		storage::book_download(&self.ctx, auth, book_id).await
	}

	async fn book_cover(
		&self,
		auth: &AuthContext,
		book_id: &str,
	) -> Result<CatalogCover, LiseurSyncError> {
		storage::book_cover(&self.ctx, auth, book_id).await
	}

	async fn book_series(
		&self,
		auth: &AuthContext,
		book_id: &str,
		scope: Option<String>,
	) -> Result<CatalogBookSeries, LiseurSyncError> {
		storage::book_series(&self.ctx, auth, book_id, scope).await
	}

	async fn resolve_catalog_book(
		&self,
		auth: &AuthContext,
		book_id: &str,
		confirmed: bool,
	) -> Result<CatalogResolveResult, LiseurSyncError> {
		storage::resolve_catalog_book(&self.ctx, auth, book_id, confirmed).await
	}

	async fn resolve_work(
		&self,
		user_id: &str,
		request: ResolveRequest,
	) -> Result<ResolveResult, LiseurSyncError> {
		storage::resolve_work(&self.ctx, user_id, request).await
	}

	async fn append_ops(
		&self,
		user_id: &str,
		device_id: &str,
		ops: Vec<OpInput>,
	) -> Result<Vec<OpResult>, LiseurSyncError> {
		storage::append_ops(&self.ctx, user_id, device_id, ops).await
	}

	async fn changes(
		&self,
		user_id: &str,
		since: i64,
		limit: usize,
	) -> Result<ChangesPage, LiseurSyncError> {
		storage::changes(&self.ctx, user_id, since, limit).await
	}

	async fn heads(&self, user_id: &str) -> Result<HeadsPage, LiseurSyncError> {
		storage::heads(&self.ctx, user_id).await
	}

	async fn positions(
		&self,
		user_id: &str,
		work_id: &str,
		limit: usize,
	) -> Result<Vec<OpRecord>, LiseurSyncError> {
		storage::positions(&self.ctx, user_id, work_id, limit).await
	}

	async fn append_sessions(
		&self,
		user_id: &str,
		device_id: &str,
		sessions: Vec<SessionInput>,
	) -> Result<usize, LiseurSyncError> {
		storage::append_sessions(&self.ctx, user_id, device_id, sessions).await
	}

	async fn append_annotations(
		&self,
		user_id: &str,
		device_id: &str,
		annotations: Vec<AnnotationInput>,
	) -> Result<Vec<AnnotationResult>, LiseurSyncError> {
		storage::append_annotations(&self.ctx, user_id, device_id, annotations).await
	}

	async fn annotation_changes(
		&self,
		user_id: &str,
		since: i64,
		limit: usize,
	) -> Result<(Vec<AnnotationRecord>, i64, bool), LiseurSyncError> {
		storage::annotation_changes(&self.ctx, user_id, since, limit).await
	}

	async fn work_annotations(
		&self,
		user_id: &str,
		work_id: &str,
	) -> Result<Vec<AnnotationRecord>, LiseurSyncError> {
		storage::work_annotations(&self.ctx, user_id, work_id).await
	}

	async fn delete_annotation(
		&self,
		user_id: &str,
		id: &str,
		rev: i64,
	) -> Result<DeleteAnnotationResult, LiseurSyncError> {
		storage::delete_annotation(&self.ctx, user_id, id, rev).await
	}

	fn attachment_max_bytes(&self) -> usize {
		self.ctx.config.protocols.attachment_max_bytes
	}

	async fn put_attachment(
		&self,
		user_id: &str,
		annotation_id: &str,
		upload: AttachmentUpload,
	) -> Result<AttachmentUploadResult, LiseurSyncError> {
		attachments::put(&self.ctx, user_id, annotation_id, upload).await
	}

	async fn attachments(
		&self,
		user_id: &str,
		annotation_id: &str,
	) -> Result<Vec<AttachmentRecord>, LiseurSyncError> {
		attachments::list(&self.ctx, user_id, annotation_id).await
	}
}
