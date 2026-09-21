pub mod config;
pub mod error;
mod handoff;
mod http;
mod http_util;
pub mod mam;
mod qbit;
mod store;

use config::GatewayConfig;
use error::GatewayResult;
use mam::MamClient;
use qbit::QbitClient;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
	pub(crate) config: Arc<GatewayConfig>,
	pub(crate) mam: Arc<MamClient>,
	pub(crate) qbit: Arc<QbitClient>,
	pub(crate) store: Arc<Mutex<store::ResultStore>>,
}

impl AppState {
	pub fn new(config: GatewayConfig) -> GatewayResult<Self> {
		let config = Arc::new(config);
		Ok(Self {
			mam: Arc::new(MamClient::new(config.clone())?),
			qbit: Arc::new(QbitClient::new(config.clone())?),
			config,
			store: Arc::new(Mutex::new(store::ResultStore::new())),
		})
	}
}

pub fn router(state: AppState) -> axum::Router {
	http::router(state)
}
