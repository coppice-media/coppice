//! KOReader progress-sync (kosync) wire contract under `/koreader/{api_key}`.
//!
//! Three routes (`users/auth`, `PUT syncs/progress`, `GET syncs/progress/{document}`),
//! two DTOs, and the [`KoreaderBackend`] trait. Authentication (Stump API key
//! in the path, `AccessKoreaderSync`), persistence, and the partial-MD5
//! `koreader_hash` live in the host.
//!
//! See `crates/koreader/README.md` for the kosync pin (`009367df`), the Liseur
//! `31f8182d` client (device-verified), decisions, and verification.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
	extract::Path,
	response::Response,
	routing::{get, put},
	Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use stump_auth::AuthContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutProgressInput {
	pub document: String,
	pub progress: String,
	pub percentage: f32,
	pub device: String,
	pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutProgressResponse {
	pub document: String,
	pub timestamp: u64,
}

/// Backend operations needed by the KOReader synchronization surface.
///
/// Authentication and permission middleware remain in the server. The adapter owns all persistence
/// and reading-progress updates while this crate owns route extraction and protocol DTOs.
#[async_trait]
pub trait KoreaderBackend: Send + Sync + 'static {
	type Error: axum::response::IntoResponse + Send + 'static;

	async fn check_authorized(&self) -> Result<Response, Self::Error>;
	async fn get_progress(
		&self,
		auth: AuthContext,
		document: String,
	) -> Result<Response, Self::Error>;
	async fn put_progress(
		&self,
		auth: AuthContext,
		input: PutProgressInput,
	) -> Result<Response, Self::Error>;
}

#[derive(Debug, Deserialize)]
struct APIKeyDocumentPath {
	#[allow(dead_code)]
	api_key: String,
	document: String,
}

#[derive(Debug, Deserialize)]
struct APIKeyPath {
	#[allow(dead_code)]
	api_key: String,
}

pub fn router<S, B>(backend: B) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	B: KoreaderBackend,
{
	let backend = Arc::new(backend);
	Router::new()
		.nest(
			"/koreader",
			Router::new().nest(
				"/{api_key}",
				Router::new()
					.route("/users/auth", get(check_authorized::<B>))
					.route("/syncs/progress", put(put_progress::<B>))
					.route("/syncs/progress/{document}", get(get_progress::<B>)),
			),
		)
		.layer(Extension(backend))
}

async fn check_authorized<B: KoreaderBackend>(
	Extension(backend): Extension<Arc<B>>,
) -> Result<Response, B::Error> {
	backend.check_authorized().await
}

async fn get_progress<B: KoreaderBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(APIKeyDocumentPath { document, .. }): Path<APIKeyDocumentPath>,
	Extension(auth): Extension<AuthContext>,
) -> Result<Response, B::Error> {
	backend.get_progress(auth, document).await
}

async fn put_progress<B: KoreaderBackend>(
	Extension(backend): Extension<Arc<B>>,
	Path(APIKeyPath { .. }): Path<APIKeyPath>,
	Extension(auth): Extension<AuthContext>,
	Json(input): Json<PutProgressInput>,
) -> Result<Response, B::Error> {
	backend.put_progress(auth, input).await
}

pub mod routes {
	pub use super::{router, KoreaderBackend, PutProgressInput, PutProgressResponse};
}
