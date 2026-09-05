//! HTTP plumbing shared by sources, the catalog fetcher, and the health checker.

use std::time::Duration;

use crate::rate_limit::RateLimiter;

/// User agent sent to every remote source, catalog, and health probe.
pub const USER_AGENT: &str = concat!(
	"Stump/",
	env!("CARGO_PKG_VERSION"),
	" (+https://github.com/stumpapp/stump)"
);

/// Default per-request timeout for source API calls and page downloads.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Build a reqwest client with the Stump user agent.
///
/// `user_agent` overrides the default for sources that must impersonate a
/// specific client (MangaDex only requires a descriptive one).
pub fn build_client(
	user_agent: Option<&str>,
	timeout: Duration,
) -> Result<reqwest::Client, reqwest::Error> {
	reqwest::Client::builder()
		.user_agent(user_agent.unwrap_or(USER_AGENT))
		.timeout(timeout)
		.build()
}

/// A rate-limited HTTP client owned by one source instance.
#[derive(Clone, Debug)]
pub struct SourceHttp {
	client: reqwest::Client,
	limiter: RateLimiter,
}

impl SourceHttp {
	pub fn new(client: reqwest::Client, limiter: RateLimiter) -> Self {
		Self { client, limiter }
	}

	/// Client with the Stump user agent, default timeout, and the given limiter.
	pub fn with_limiter(limiter: RateLimiter) -> Result<Self, reqwest::Error> {
		Ok(Self::new(build_client(None, DEFAULT_TIMEOUT)?, limiter))
	}

	pub fn client(&self) -> &reqwest::Client {
		&self.client
	}

	pub fn limiter(&self) -> &RateLimiter {
		&self.limiter
	}
}
