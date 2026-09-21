use mam_gateway::{config::GatewayConfig, router, AppState};
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.compact()
		.init();

	let config = GatewayConfig::from_env()?;
	let bind_addr = config.bind_addr();
	let state = AppState::new(config)?;
	let listener = TcpListener::bind(bind_addr).await?;
	info!(%bind_addr, "mam gateway listening");
	axum::serve(listener, router(state)).await?;
	Ok(())
}
