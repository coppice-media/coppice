use std::{net::SocketAddr, sync::Arc};

use axum::{extract::connect_info::Connected, serve::IncomingStream, Extension, Router};
use stump_core::{
	config::{bootstrap_config_dir, logging::init_tracing},
	StumpCore,
};
use tokio::net::TcpListener;
use tower_http::{
	compression::{
		predicate::{DefaultPredicate, NotForContentType, Predicate},
		CompressionLayer,
	},
	trace::TraceLayer,
};

use crate::{
	config::{
		cors, oidc::OidcProvider, rate_limit::RateLimitConfig, session::get_session_layer,
	},
	errors::{EntryError, ServerError, ServerResult},
	middleware::rate_limit::{rate_limit_middleware, RateLimiter},
	routers,
	utils::shutdown_signal_with_cleanup,
};
use stump_core::config::StumpConfig;

pub async fn run_http_server(config: StumpConfig) -> ServerResult<()> {
	let core = StumpCore::new(config.clone()).await;

	// Server-only resources are started lazily or only when their configuration
	// requires them; the context retains the shared lifecycle handles.

	// Cancel any islanded jobs from a previous run. This is a direct database
	// operation and deliberately does not create the lazy job runtime.
	if config.jobs.enable_background_jobs {
		core.cancel_islanded_jobs()
			.await
			.map_err(|e| ServerError::ServerStartError(e.to_string()))?;
	}

	// Initialize the server configuration. If it already exists, nothing will happen.
	core.init_server_config()
		.await
		.map_err(|e| ServerError::ServerStartError(e.to_string()))?;

	// Initialize the encryption key, if it doesn't exist
	core.init_encryption()
		.await
		.map_err(|e| ServerError::ServerStartError(e.to_string()))?;

	core.init_jwt_secrets()
		.await
		.map_err(|e| ServerError::ServerStartError(e.to_string()))?;

	core.init_journal_mode()
		.await
		.map_err(|e| ServerError::ServerStartError(e.to_string()))?;

	// The scheduler and watcher are optional server-owned startup handles. The
	// context can start either one later when a request changes configuration.
	let _scheduler = if config.jobs.enable_background_jobs {
		core.init_scheduler()
			.await
			.map_err(|e| ServerError::ServerStartError(e.to_string()))?
	} else {
		None
	};

	if config.jobs.enable_background_jobs {
		core.init_library_watcher()
			.await
			.map_err(|e| ServerError::ServerStartError(e.to_string()))?;
	}
	let oidc_provider: Option<Arc<OidcProvider>> = {
		if let Some(oidc_config) = config.oidc.as_ref().filter(|c| c.is_configured()) {
			let state = OidcProvider::new(oidc_config).await.map_err(|e| {
				tracing::error!(?e, "OIDC client initialization failed");
				ServerError::ServerStartError(format!("OIDC client init failed: {e:?}"))
			})?;
			tracing::info!("OIDC client initialized successfully");
			Some(Arc::new(state))
		} else {
			None
		}
	};

	let server_ctx = core.get_context();
	let app_state = server_ctx.arced();
	// The provider host (Mode B virtual libraries) starts only when
	// `providers.enable_providers` is on; it registers the `provider://`
	// media resolver for every protocol.
	#[cfg(feature = "providers")]
	stump_core::providers::init(&server_ctx)
		.await
		.map_err(|e| ServerError::ServerStartError(e.to_string()))?;
	let cors_layer = cors::get_cors_layer(config.clone());
	let rate_limiter = Arc::new(RateLimiter::new(
		RateLimitConfig::from_env().map_err(ServerError::ServerStartError)?,
		config.server.trust_proxy_headers,
	));

	println!("{}", core.get_shadow_text());

	let app_router = routers::mount(app_state.clone()).await;

	// we have to exclude downloadable content types from compression, 1 bc a lot are already compressed
	// but also because compression seems to strip the content-length header which breaks download progress
	// tracking on the mobile app
	let compression_predicate = DefaultPredicate::new()
		.and(NotForContentType::const_new("application/epub"))
		.and(NotForContentType::const_new("application/zip"))
		.and(NotForContentType::const_new("application/pdf"))
		.and(NotForContentType::const_new("application/octet-stream"))
		.and(NotForContentType::const_new("application/x-rar"))
		.and(NotForContentType::const_new("application/vnd.rar"))
		.and(NotForContentType::const_new(
			"application/vnd.comicbook+zip",
		))
		.and(NotForContentType::const_new("application/x-cbr"))
		.and(NotForContentType::const_new("application/x-cbz"));

	// The rate limiter sits outside the session layer so rejected requests
	// never load a session; it is also exposed to `/api/v2/health`.
	let app = Router::new()
		.merge(app_router)
		.with_state(app_state.clone())
		.layer(get_session_layer(app_state.clone()))
		.layer(axum::middleware::from_fn_with_state(
			rate_limiter.clone(),
			rate_limit_middleware,
		));

	// Komga clients (Komelia) keep separate HTTP clients that share one cookie
	// jar; a session-clearing `Set-Cookie` on a 401 from any of them (for
	// example an SSE reconnect carrying a dead cookie) wipes the live session
	// of the others. Komga itself never clears cookies on 401, so strip the
	// clears that both Stump and tower-sessions attach, on Komga paths only.
	// This must sit outside the session layer to see tower-sessions' header.
	#[cfg(feature = "komga")]
	let app = app.layer(axum::middleware::from_fn(
		crate::middleware::auth::strip_komga_unauthorized_cookie_clears,
	));

	let app = app
		.layer(cors_layer)
		.layer(CompressionLayer::new().compress_when(compression_predicate))
		// The default span only carries method/uri at TRACE; record them on the
		// span so every 4xx/5xx `on_failure` line names the request.
		.layer(TraceLayer::new_for_http().make_span_with(
			tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::DEBUG),
		))
		.layer(Extension(oidc_provider))
		.layer(Extension(rate_limiter));

	// TODO: Refactor to use https://docs.rs/async-shutdown/latest/async_shutdown/
	let cleanup = {
		let server_ctx = server_ctx.clone();
		let background_jobs_enabled = config.jobs.enable_background_jobs;
		move || async move {
			println!("Initializing graceful shutdown...");
			if background_jobs_enabled {
				// These methods are no-ops when the corresponding lazy resource
				// was never created, including resources started after boot.
				let _ = server_ctx.stop_library_watcher().await;
				server_ctx.stop_scheduler().await;
				server_ctx.stop_job_runtime().await;
			}
		}
	};

	let ip: std::net::IpAddr =
		config
			.server
			.ip
			.parse()
			.map_err(|e: std::net::AddrParseError| {
				ServerError::ServerStartError(e.to_string())
			})?;
	let addr = SocketAddr::from((ip, config.server.port));
	let listener = tokio::net::TcpListener::bind(&addr)
		.await
		.map_err(|e| ServerError::ServerStartError(e.to_string()))?;

	tracing::info!("⚡️ Stump HTTP server starting on http://{}", addr);

	// TODO: Experiment with higher concurrency, YEARS ago at this point (before enforcing WAL even)
	// I experienced multi-writer issues but perhaps with SeaORM + WAL we can have parallel scans.
	let http = axum::serve(
		listener,
		app.into_make_service_with_connect_info::<StumpRequestInfo>(),
	)
	.with_graceful_shutdown(shutdown_signal_with_cleanup(Some(cleanup)));

	let _ = http.await;

	Ok(())
}

#[allow(dead_code)]
pub async fn bootstrap_http_server_config() -> Result<StumpConfig, EntryError> {
	// Get STUMP_CONFIG_DIR to bootstrap startup
	let config_dir = bootstrap_config_dir();

	let config = StumpCore::init_config(config_dir)
		.map_err(|e| EntryError::InvalidConfig(e.to_string()))?;

	// Note: init_tracing after loading the environment so the correct verbosity
	// level is used for logging.
	init_tracing(&config);

	if config.server.verbosity >= 3 {
		tracing::trace!(?config, "App config");
	}

	Ok(config)
}

#[derive(Clone, Debug)]
pub struct StumpRequestInfo {
	pub ip_addr: std::net::IpAddr,
}

impl Connected<IncomingStream<'_, TcpListener>> for StumpRequestInfo {
	fn connect_info(target: IncomingStream<'_, TcpListener>) -> Self {
		StumpRequestInfo {
			ip_addr: target.remote_addr().ip(),
		}
	}
}
