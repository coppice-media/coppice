//! Inbound rate limiting.
//!
//! One outer layer classifies every request into one of three classes and
//! keys the check on the calling identity, without touching the database:
//!
//! - **AUTH** — credential-verifying requests: native login/register/refresh,
//!   the OIDC callback, device pairing, liseur `/v1/login` + `/v1/tokens`, the
//!   Kobo device handshake, and the *first hit* of any Basic, `X-API-Key`, or
//!   path-embedded API key credential (Komga, OPDS, Kobo, KOReader). Keyed by
//!   client IP, since the credential is exactly what is not yet trusted.
//! - **WRITE** — mutating requests: every `POST`/`PUT`/`PATCH`/`DELETE` except
//!   the known query-over-POST endpoints, plus Kobo library sync and GraphQL
//!   mutations (sniffed from the JSON body). Keyed by credential, else IP.
//! - **READ/STREAM** — everything else, including page and file streaming.
//!   Unlimited rate, but at most `stream_concurrency` requests in flight per
//!   key; a permit is held until the response body has been fully sent.
//!
//! A credential counts as an authentication attempt until the server has
//! answered it with anything other than `401`; it counts again after a `401`
//! or after sixty seconds without a request (mirroring the Basic cache TTL in
//! `auth.rs`, after which bcrypt runs again). Credentials are only ever stored
//! as SHA-256 fingerprints.
//!
//! Rejections are `429` with `Retry-After` and the standard `APIError` JSON
//! body. Per-class allowed/limited counters feed `/api/v2/health`.

use std::{
	collections::HashMap,
	net::{IpAddr, Ipv4Addr},
	pin::Pin,
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc, Mutex,
	},
	task::{Context, Poll},
	time::{Duration, Instant},
};

use axum::{
	body::{Body, Bytes},
	extract::{Request, State},
	http::{header, HeaderMap, HeaderValue, Method, StatusCode},
	middleware::Next,
	response::{IntoResponse, Response},
};
use governor::{
	clock::{Clock, DefaultClock},
	state::keyed::DefaultKeyedStateStore,
	Quota, RateLimiter as GovernorLimiter,
};
use http_body::{Frame, SizeHint};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
	config::{rate_limit::RateLimitConfig, session::SESSION_NAME},
	errors::APIError,
	middleware::{auth::KOMGA_API_KEY_HEADER, host::extract_client_ip},
};

/// How long a verified credential stays exempt from the AUTH class without
/// traffic. Matches the Basic-auth cache TTL in `auth.rs`.
const VERIFIED_TTL: Duration = Duration::from_secs(60);
const VERIFIED_CAPACITY: usize = 1024;
/// How long a READ/STREAM request waits for an in-flight slot before `429`.
const STREAM_WAIT: Duration = Duration::from_secs(10);
/// Largest GraphQL JSON body inspected for an operation type; larger bodies
/// (and non-JSON multipart uploads) are treated as mutations.
const GRAPHQL_SNIFF_LIMIT: usize = 1024 * 1024;
/// Keyed governor state is pruned every this many checks.
const RETAIN_EVERY: u64 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
	Auth,
	Write,
	Stream,
}

impl Class {
	fn name(self) -> &'static str {
		match self {
			Class::Auth => "auth",
			Class::Write => "write",
			Class::Stream => "stream",
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
	Credential([u8; 32]),
	Ip(IpAddr),
}

type Keyed = GovernorLimiter<Key, DefaultKeyedStateStore<Key>, DefaultClock>;

#[derive(Default)]
struct Counters {
	allowed: AtomicU64,
	limited: AtomicU64,
}

pub struct RateLimiter {
	config: RateLimitConfig,
	trust_proxy_headers: bool,
	clock: DefaultClock,
	auth: Keyed,
	write: Keyed,
	streams: Mutex<HashMap<Key, Arc<Semaphore>>>,
	verified: Mutex<HashMap<[u8; 32], Instant>>,
	counters: [Counters; 3],
	checks: AtomicU64,
}

impl RateLimiter {
	pub fn new(config: RateLimitConfig, trust_proxy_headers: bool) -> Self {
		let clock = DefaultClock::default();
		let keyed = |per_min, burst| {
			GovernorLimiter::new(
				Quota::per_minute(per_min).allow_burst(burst),
				DefaultKeyedStateStore::default(),
				&clock,
			)
		};
		let auth = keyed(config.auth_per_min, config.auth_burst);
		let write = keyed(config.write_per_min, config.write_burst);
		Self {
			config,
			trust_proxy_headers,
			clock,
			auth,
			write,
			streams: Mutex::new(HashMap::new()),
			verified: Mutex::new(HashMap::new()),
			counters: Default::default(),
			checks: AtomicU64::new(0),
		}
	}

	pub fn enabled(&self) -> bool {
		self.config.enabled
	}

	/// The `rate_limit` member of the `/api/v2/health` payload.
	pub fn health_status(&self) -> serde_json::Value {
		if !self.config.enabled {
			return json!({ "status": "disabled" });
		}
		let counts = |class: Class| {
			let counters = &self.counters[class as usize];
			(
				counters.allowed.load(Ordering::Relaxed),
				counters.limited.load(Ordering::Relaxed),
			)
		};
		let (auth_allowed, auth_limited) = counts(Class::Auth);
		let (write_allowed, write_limited) = counts(Class::Write);
		let (stream_allowed, stream_limited) = counts(Class::Stream);
		json!({
			"status": "enabled",
			"auth": {
				"allowed": auth_allowed,
				"limited": auth_limited,
				"per_minute": self.config.auth_per_min.get(),
				"burst": self.config.auth_burst.get(),
			},
			"write": {
				"allowed": write_allowed,
				"limited": write_limited,
				"per_minute": self.config.write_per_min.get(),
				"burst": self.config.write_burst.get(),
			},
			"stream": {
				"allowed": stream_allowed,
				"limited": stream_limited,
				"concurrency": self.config.stream_concurrency.get(),
			},
		})
	}

	async fn handle(&self, req: Request, next: Next) -> Response {
		let identity = self.identity(&req);
		let (req, class) = match self.classify(req).await {
			Ok(classified) => classified,
			Err(rejection) => return rejection,
		};
		let class = match identity.verifiable {
			Some(fingerprint) if !self.is_verified(&fingerprint) => Class::Auth,
			_ => class,
		};

		let response = match class {
			Class::Auth => match self.check(&self.auth, &Key::Ip(identity.ip)) {
				Ok(()) => self.allowed(class, next.run(req).await),
				Err(retry_after) => self.limited(class, &identity, retry_after),
			},
			Class::Write => match self.check(&self.write, &identity.key) {
				Ok(()) => self.allowed(class, next.run(req).await),
				Err(retry_after) => self.limited(class, &identity, retry_after),
			},
			Class::Stream => match self.acquire_stream(&identity.key).await {
				Some(permit) => {
					let (parts, body) = next.run(req).await.into_parts();
					self.allowed(
						class,
						Response::from_parts(
							parts,
							Body::new(PermitBody {
								inner: body,
								_permit: permit,
							}),
						),
					)
				},
				None => self.limited(class, &identity, Duration::from_secs(1)),
			},
		};

		if let Some(fingerprint) = identity.verifiable {
			self.record_verification(fingerprint, response.status());
		}
		response
	}

	fn allowed(&self, class: Class, response: Response) -> Response {
		self.counters[class as usize]
			.allowed
			.fetch_add(1, Ordering::Relaxed);
		response
	}

	fn limited(&self, class: Class, identity: &Identity, retry_after: Duration) -> Response {
		self.counters[class as usize]
			.limited
			.fetch_add(1, Ordering::Relaxed);
		tracing::warn!(
			class = class.name(),
			ip = %identity.ip,
			retry_after_secs = retry_after.as_secs(),
			"Request rate limited"
		);
		too_many_requests(retry_after)
	}

	fn check(&self, limiter: &Keyed, key: &Key) -> Result<(), Duration> {
		if self.checks.fetch_add(1, Ordering::Relaxed) % RETAIN_EVERY == 0 {
			self.auth.retain_recent();
			self.write.retain_recent();
		}
		limiter
			.check_key(key)
			.map_err(|not_until| not_until.wait_time_from(self.clock.now()))
	}

	async fn acquire_stream(&self, key: &Key) -> Option<OwnedSemaphorePermit> {
		let semaphore = {
			let mut streams = self
				.streams
				.lock()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			if streams.len() >= VERIFIED_CAPACITY {
				// Only the map and in-flight permits hold a semaphore; drop idle keys.
				streams.retain(|_, semaphore| Arc::strong_count(semaphore) > 1);
			}
			streams
				.entry(key.clone())
				.or_insert_with(|| {
					Arc::new(Semaphore::new(self.config.stream_concurrency.get()))
				})
				.clone()
		};
		tokio::time::timeout(STREAM_WAIT, semaphore.acquire_owned())
			.await
			.ok()?
			.ok()
	}

	fn is_verified(&self, fingerprint: &[u8; 32]) -> bool {
		self.verified
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.get(fingerprint)
			.is_some_and(|verified_at| verified_at.elapsed() < VERIFIED_TTL)
	}

	fn record_verification(&self, fingerprint: [u8; 32], status: StatusCode) {
		let mut verified = self
			.verified
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		if status == StatusCode::UNAUTHORIZED {
			verified.remove(&fingerprint);
			return;
		}
		if verified.len() >= VERIFIED_CAPACITY && !verified.contains_key(&fingerprint) {
			verified.retain(|_, verified_at| verified_at.elapsed() < VERIFIED_TTL);
			if verified.len() >= VERIFIED_CAPACITY {
				if let Some(oldest) = verified
					.iter()
					.min_by_key(|(_, verified_at)| **verified_at)
					.map(|(key, _)| *key)
				{
					verified.remove(&oldest);
				}
			}
		}
		verified.insert(fingerprint, Instant::now());
	}

	fn identity(&self, req: &Request) -> Identity {
		let headers = req.headers();
		let ip = extract_client_ip(headers, req.extensions(), self.trust_proxy_headers)
			.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
		let (credential, verifiable) = credential(headers, req.uri().path());
		Identity {
			ip,
			key: credential.map(Key::Credential).unwrap_or(Key::Ip(ip)),
			verifiable: credential.filter(|_| verifiable),
		}
	}

	/// Resolves the path class, reading the body only for GraphQL documents.
	async fn classify(&self, req: Request) -> Result<(Request, Class), Response> {
		match path_class(req.method(), req.uri().path()) {
			PathClass::Fixed(class) => Ok((req, class)),
			#[cfg(feature = "graphql")]
			PathClass::GraphQl => graphql_class(req).await,
		}
	}
}

struct Identity {
	ip: IpAddr,
	/// Credential fingerprint when one is presented, else the client IP.
	key: Key,
	/// Basic / `X-API-Key` / path-key fingerprint subject to the first-hit rule.
	verifiable: Option<[u8; 32]>,
}

/// Returns the presented credential fingerprint and whether it is verified per
/// request by the auth middleware (Basic, `X-API-Key`, path key) rather than a
/// cheap bearer/session lookup.
fn credential(headers: &HeaderMap, path: &str) -> (Option<[u8; 32]>, bool) {
	if let Some(authorization) = headers.get(header::AUTHORIZATION) {
		let is_basic = authorization
			.as_bytes()
			.get(..6)
			.is_some_and(|scheme| scheme.eq_ignore_ascii_case(b"basic "));
		return (Some(fingerprint(authorization.as_bytes())), is_basic);
	}
	if let Some(api_key) = headers.get(KOMGA_API_KEY_HEADER) {
		return (Some(fingerprint(api_key.as_bytes())), true);
	}
	if let Some(api_key) = path_api_key(path) {
		return (Some(fingerprint(api_key.as_bytes())), true);
	}
	if let Some(session) = cookie(headers, SESSION_NAME) {
		return (Some(fingerprint(session.as_bytes())), false);
	}
	#[cfg(feature = "komga")]
	if let Some(remember_me) = crate::middleware::auth::komga_remember_me_cookie(headers)
	{
		return (Some(fingerprint(remember_me.as_bytes())), false);
	}
	(None, false)
}

fn fingerprint(secret: &[u8]) -> [u8; 32] {
	Sha256::digest(secret).into()
}

/// The `{api_key}` segment of `/opds/{api_key}/v1.2/...`, `/kobo/{api_key}/...`,
/// and `/koreader/{api_key}/...`.
fn path_api_key(path: &str) -> Option<&str> {
	let rest = ["/opds/", "/kobo/", "/koreader/"]
		.iter()
		.find_map(|prefix| path.strip_prefix(prefix))?;
	match rest.split('/').next()? {
		"" | "v1.2" | "v2.0" => None,
		segment => Some(segment),
	}
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
	headers
		.get_all(header::COOKIE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(';'))
		.map(str::trim)
		.find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

enum PathClass {
	Fixed(Class),
	#[cfg(feature = "graphql")]
	GraphQl,
}

fn path_class(method: &Method, path: &str) -> PathClass {
	if let Some(rest) = path.strip_prefix("/api/v2/auth/") {
		// `me`, `viewer`, `oidc/config`, and `oidc/authorize` are plain reads;
		// the callback and every POST verify credentials.
		if rest == "oidc/callback" || *method != Method::GET {
			return PathClass::Fixed(Class::Auth);
		}
	}
	if *method == Method::POST
		&& matches!(path, "/api/v2/devices/pair/start" | "/v1/login" | "/v1/tokens")
	{
		return PathClass::Fixed(Class::Auth);
	}
	if let Some(rest) = path.strip_prefix("/kobo/") {
		let after_key = rest.split_once('/').map(|(_, rest)| rest).unwrap_or("");
		if *method == Method::POST && after_key == "v1/auth/device" {
			return PathClass::Fixed(Class::Auth);
		}
		if *method == Method::GET && after_key == "v1/library/sync" {
			return PathClass::Fixed(Class::Write);
		}
	}
	#[cfg(feature = "graphql")]
	if *method == Method::POST && path.trim_end_matches('/') == "/api/graphql" {
		return PathClass::GraphQl;
	}

	let mutating = matches!(
		*method,
		Method::POST | Method::PUT | Method::PATCH | Method::DELETE
	);
	if !mutating || is_query_over_post(path) {
		PathClass::Fixed(Class::Stream)
	} else {
		PathClass::Fixed(Class::Write)
	}
}

/// POST endpoints that only read: the version probe, Komga `*/list` catalog
/// queries, and liseur `*/resolve` lookups.
fn is_query_over_post(path: &str) -> bool {
	path == "/api/v2/version"
		|| ((path.starts_with("/api/v1/") || path.starts_with("/komga/api/v1/"))
			&& path.ends_with("/list"))
		|| (path.starts_with("/v1/") && path.ends_with("/resolve"))
}

#[cfg(feature = "graphql")]
async fn graphql_class(req: Request) -> Result<(Request, Class), Response> {
	let is_json = req
		.headers()
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| {
			value
				.trim_start()
				.get(..16)
				.is_some_and(|mime| mime.eq_ignore_ascii_case("application/json"))
		});
	let declared_length = req
		.headers()
		.get(header::CONTENT_LENGTH)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<usize>().ok());
	if !is_json || declared_length.is_none_or(|length| length > GRAPHQL_SNIFF_LIMIT) {
		return Ok((req, Class::Write));
	}

	let (parts, body) = req.into_parts();
	let bytes = axum::body::to_bytes(body, GRAPHQL_SNIFF_LIMIT)
		.await
		.map_err(|error| APIError::BadRequest(error.to_string()).into_response())?;
	let class = if graphql_is_mutation(&bytes) {
		Class::Write
	} else {
		Class::Stream
	};
	Ok((Request::from_parts(parts, Body::from(bytes)), class))
}

/// Whether the selected operation of a GraphQL JSON request is a mutation.
/// Unparseable documents are left to the GraphQL handler to reject.
#[cfg(feature = "graphql")]
fn graphql_is_mutation(body: &[u8]) -> bool {
	use async_graphql::parser::types::{DocumentOperations, OperationType};

	#[derive(serde::Deserialize)]
	struct GraphQlRequest<'a> {
		#[serde(borrow)]
		query: std::borrow::Cow<'a, str>,
		#[serde(rename = "operationName")]
		operation_name: Option<String>,
	}

	let Ok(request) = serde_json::from_slice::<GraphQlRequest>(body) else {
		return false;
	};
	let Ok(document) = async_graphql::parser::parse_query(&request.query) else {
		return false;
	};
	match (&document.operations, request.operation_name.as_deref()) {
		(DocumentOperations::Single(operation), _) => {
			operation.node.ty == OperationType::Mutation
		},
		(DocumentOperations::Multiple(operations), Some(name)) => operations
			.get(name)
			.is_some_and(|operation| operation.node.ty == OperationType::Mutation),
		(DocumentOperations::Multiple(operations), None) => operations
			.values()
			.any(|operation| operation.node.ty == OperationType::Mutation),
	}
}

fn too_many_requests(retry_after: Duration) -> Response {
	let mut response = APIError::TooManyRequests.into_response();
	let seconds = retry_after.as_secs_f64().ceil().max(1.0) as u64;
	response
		.headers_mut()
		.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
	response
}

/// Response body that releases its READ/STREAM permit only once fully sent
/// (or dropped), while preserving the inner body's size hint so
/// `Content-Length` is unchanged.
struct PermitBody {
	inner: Body,
	_permit: OwnedSemaphorePermit,
}

impl http_body::Body for PermitBody {
	type Data = Bytes;
	type Error = axum::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		Pin::new(&mut self.get_mut().inner).poll_frame(cx)
	}

	fn is_end_stream(&self) -> bool {
		self.inner.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		self.inner.size_hint()
	}
}

pub async fn rate_limit_middleware(
	State(limiter): State<Arc<RateLimiter>>,
	req: Request,
	next: Next,
) -> Response {
	if !limiter.config.enabled {
		return next.run(req).await;
	}
	limiter.handle(req, next).await
}

#[cfg(test)]
mod tests {
	use std::net::Ipv4Addr;

	use axum::{
		body::to_bytes,
		http::Request,
		middleware,
		routing::{any, get, patch, post},
		Router,
	};
	use tokio::sync::Notify;
	use tower::Service;

	use super::*;
	use crate::http_server::StumpRequestInfo;

	fn limiter(config: RateLimitConfig) -> Arc<RateLimiter> {
		Arc::new(RateLimiter::new(config, false))
	}

	fn app(limiter: Arc<RateLimiter>) -> Router {
		Router::new()
			.route("/api/v2/auth/login", post(|| async { StatusCode::OK }))
			.route("/api/v2/auth/me", get(|| async { StatusCode::OK }))
			.route(
				"/api/v1/books/{id}/read-progress",
				patch(|| async { StatusCode::OK }),
			)
			.route("/api/v1/books/list", post(|| async { StatusCode::OK }))
			.route("/opds/v1.2/catalog", get(|| async { StatusCode::OK }))
			.route(
				"/opds/v1.2/rejected",
				get(|| async { StatusCode::UNAUTHORIZED }),
			)
			.route("/api/graphql", post(|| async { StatusCode::OK }))
			.route("/api/v1/books/{id}/pages/{page}", get(|| async { "page" }))
			.route("/v1/login", post(|| async { StatusCode::OK }))
			.route("/kobo/{key}/v1/library/sync", get(|| async { StatusCode::OK }))
			.layer(middleware::from_fn_with_state(limiter, rate_limit_middleware))
	}

	fn request(method: Method, path: &str, ip: IpAddr) -> Request<Body> {
		let mut req = Request::builder()
			.method(method)
			.uri(path)
			.body(Body::empty())
			.unwrap();
		req.extensions_mut().insert(StumpRequestInfo { ip_addr: ip });
		req
	}

	fn ip(last: u8) -> IpAddr {
		IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
	}

	async fn call(app: &Router, req: Request<Body>) -> Response {
		// `Router` is always ready, so `call` without `poll_ready` is sound.
		app.clone().call(req).await.unwrap()
	}

	async fn status(app: &Router, req: Request<Body>) -> StatusCode {
		call(app, req).await.status()
	}

	#[tokio::test]
	async fn twenty_one_rapid_logins_yield_429_and_other_ip_is_unaffected() {
		let limiter = limiter(RateLimitConfig::default());
		let app = app(limiter.clone());

		for attempt in 1..=20 {
			assert_eq!(
				status(&app, request(Method::POST, "/api/v2/auth/login", ip(1))).await,
				StatusCode::OK,
				"attempt {attempt} must be within the burst"
			);
		}
		let response =
			call(&app, request(Method::POST, "/api/v2/auth/login", ip(1))).await;
		assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
		let retry_after = response
			.headers()
			.get(header::RETRY_AFTER)
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.parse::<u64>().ok())
			.expect("Retry-After header");
		assert!((1..=6).contains(&retry_after), "retry after {retry_after}s");
		assert_eq!(
			response.headers().get(header::CONTENT_TYPE).unwrap(),
			"application/json"
		);
		let body: serde_json::Value =
			serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap())
				.unwrap();
		assert_eq!(body, json!({ "status": 429, "message": "Too many requests" }));

		assert_eq!(
			status(&app, request(Method::POST, "/api/v2/auth/login", ip(2))).await,
			StatusCode::OK,
			"a second IP has its own bucket"
		);

		let health = limiter.health_status();
		assert_eq!(health["status"], "enabled");
		assert_eq!(health["auth"]["allowed"], 21);
		assert_eq!(health["auth"]["limited"], 1);
		assert_eq!(health["auth"]["per_minute"], 10);
		assert_eq!(health["auth"]["burst"], 20);
		assert_eq!(health["write"]["limited"], 0);
		assert_eq!(health["stream"]["concurrency"], 32);
	}

	#[tokio::test]
	async fn auth_bucket_keys_on_ip_even_when_credentials_rotate() {
		let app = app(limiter(RateLimitConfig::default()));
		let basic = |n: u32| {
			let mut req = request(Method::GET, "/opds/v1.2/rejected", ip(1));
			req.headers_mut().insert(
				header::AUTHORIZATION,
				HeaderValue::from_str(&format!("Basic guess-{n}")).unwrap(),
			);
			req
		};
		for _ in 0..20 {
			assert_eq!(status(&app, basic(rand::random())).await, StatusCode::UNAUTHORIZED);
		}
		assert_eq!(
			status(&app, basic(rand::random())).await,
			StatusCode::TOO_MANY_REQUESTS
		);
	}

	#[tokio::test]
	async fn verified_credential_stops_counting_until_rejected() {
		let limiter = limiter(RateLimitConfig::default());
		let app = app(limiter.clone());
		let with_basic = |path: &str| {
			let mut req = request(Method::GET, path, ip(1));
			req.headers_mut()
				.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic ok"));
			req
		};

		// First hit counts, the 200 verifies the credential, later hits are reads.
		for _ in 0..40 {
			assert_eq!(
				status(&app, with_basic("/opds/v1.2/catalog")).await,
				StatusCode::OK
			);
		}
		let health = limiter.health_status();
		assert_eq!(health["auth"]["allowed"], 1);
		assert_eq!(health["stream"]["allowed"], 39);

		// The first rejection is still a read (the credential was verified);
		// the 401 un-verifies it and every later attempt counts again.
		for _ in 0..20 {
			assert_eq!(
				status(&app, with_basic("/opds/v1.2/rejected")).await,
				StatusCode::UNAUTHORIZED
			);
		}
		assert_eq!(
			status(&app, with_basic("/opds/v1.2/rejected")).await,
			StatusCode::TOO_MANY_REQUESTS
		);
	}

	#[tokio::test]
	async fn write_class_limits_per_credential() {
		let limiter = limiter(RateLimitConfig::default());
		let app = app(limiter.clone());
		let progress = |token: &'static str| {
			let mut req = request(Method::PATCH, "/api/v1/books/1/read-progress", ip(1));
			req.headers_mut()
				.insert(header::AUTHORIZATION, HeaderValue::from_static(token));
			req
		};
		for _ in 0..60 {
			assert_eq!(status(&app, progress("Bearer one")).await, StatusCode::OK);
		}
		assert_eq!(
			status(&app, progress("Bearer one")).await,
			StatusCode::TOO_MANY_REQUESTS
		);
		assert_eq!(
			status(&app, progress("Bearer two")).await,
			StatusCode::OK,
			"another bearer token has its own bucket"
		);
		// Query-over-POST stays a read.
		for _ in 0..70 {
			let mut req = request(Method::POST, "/api/v1/books/list", ip(1));
			req.headers_mut()
				.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer one"));
			assert_eq!(status(&app, req).await, StatusCode::OK);
		}
		let health = limiter.health_status();
		assert_eq!(health["write"]["allowed"], 61);
		assert_eq!(health["write"]["limited"], 1);
		assert_eq!(health["stream"]["allowed"], 70);
	}

	#[tokio::test]
	async fn kobo_sync_get_and_liseur_login_are_classified() {
		let limiter = limiter(RateLimitConfig::default());
		let app = app(limiter.clone());
		assert_eq!(
			status(&app, request(Method::GET, "/kobo/stump_key/v1/library/sync", ip(1)))
				.await,
			StatusCode::OK
		);
		// The path key's first hit is AUTH; once verified the sync is a WRITE.
		assert_eq!(
			status(&app, request(Method::GET, "/kobo/stump_key/v1/library/sync", ip(1)))
				.await,
			StatusCode::OK
		);
		assert_eq!(status(&app, request(Method::POST, "/v1/login", ip(1))).await, StatusCode::OK);
		let health = limiter.health_status();
		assert_eq!(health["auth"]["allowed"], 2);
		assert_eq!(health["write"]["allowed"], 1);
	}

	#[cfg(feature = "graphql")]
	#[tokio::test]
	async fn graphql_mutations_are_writes_and_queries_are_reads() {
		let limiter = limiter(RateLimitConfig::default());
		let app = app(limiter.clone());
		let gql = |document: &str| {
			let mut req = request(Method::POST, "/api/graphql", ip(1));
			let body = serde_json::to_vec(&json!({ "query": document })).unwrap();
			req.headers_mut().insert(
				header::CONTENT_TYPE,
				HeaderValue::from_static("application/json"),
			);
			req.headers_mut()
				.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
			*req.body_mut() = Body::from(body);
			req
		};
		for _ in 0..60 {
			assert_eq!(
				status(&app, gql("mutation { updateProgress(id: 1) { id } }")).await,
				StatusCode::OK
			);
		}
		assert_eq!(
			status(&app, gql("mutation { updateProgress(id: 1) { id } }")).await,
			StatusCode::TOO_MANY_REQUESTS
		);
		for _ in 0..70 {
			assert_eq!(status(&app, gql("{ me { id } }")).await, StatusCode::OK);
		}
		let health = limiter.health_status();
		assert_eq!(health["write"]["allowed"], 60);
		assert_eq!(health["stream"]["allowed"], 70);

		assert!(graphql_is_mutation(
			br#"{"query":"query A { a } mutation B { b }","operationName":"B"}"#
		));
		assert!(!graphql_is_mutation(
			br#"{"query":"query A { a } mutation B { b }","operationName":"A"}"#
		));
		assert!(!graphql_is_mutation(b"not json"));
	}

	#[tokio::test(start_paused = true)]
	async fn stream_class_caps_in_flight_requests_per_key() {
		let mut config = RateLimitConfig::default();
		config.stream_concurrency = std::num::NonZeroUsize::new(2).unwrap();
		let limiter = limiter(config);
		let release = Arc::new(Notify::new());
		let app = {
			let release = release.clone();
			Router::new()
				.route(
					"/api/v1/books/{id}/pages/{page}",
					any(move || {
						let release = release.clone();
						async move {
							release.notified().await;
							"page"
						}
					}),
				)
				.layer(middleware::from_fn_with_state(
					limiter.clone(),
					rate_limit_middleware,
				))
		};
		let page = |ip_last: u8| request(Method::GET, "/api/v1/books/1/pages/1", ip(ip_last));

		let in_flight: Vec<_> = (0..2)
			.map(|_| {
				let app = app.clone();
				tokio::spawn(async move { call(&app, page(1)).await })
			})
			.collect();
		tokio::task::yield_now().await;

		// Third request for the same key waits, then gives up with 429.
		let response = call(&app, page(1)).await;
		assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
		assert_eq!(response.headers()[header::RETRY_AFTER], "1");

		// A different key is unaffected.
		release.notify_one();
		let other = tokio::spawn({
			let app = app.clone();
			async move { call(&app, page(2)).await }
		});
		release.notify_one();
		release.notify_one();
		let responses: Vec<Response> = futures_util::future::join_all(in_flight)
			.await
			.into_iter()
			.map(Result::unwrap)
			.collect();
		assert_eq!(other.await.unwrap().status(), StatusCode::OK);

		// Permits are held until the bodies are consumed.
		assert_eq!(
			call(&app, page(1)).await.status(),
			StatusCode::TOO_MANY_REQUESTS
		);
		for response in responses {
			assert_eq!(response.status(), StatusCode::OK);
			assert_eq!(to_bytes(response.into_body(), 64).await.unwrap(), "page");
		}
		release.notify_one();
		assert_eq!(
			call(&app, page(1)).await.status(),
			StatusCode::OK
		);

		let health = limiter.health_status();
		assert_eq!(health["stream"]["allowed"], 4);
		assert_eq!(health["stream"]["limited"], 2);
	}

	#[tokio::test]
	async fn disabled_limiter_passes_everything_through() {
		let mut config = RateLimitConfig::default();
		config.enabled = false;
		let limiter = limiter(config);
		let app = app(limiter.clone());
		for _ in 0..30 {
			assert_eq!(
				status(&app, request(Method::POST, "/api/v2/auth/login", ip(1))).await,
				StatusCode::OK
			);
		}
		assert_eq!(limiter.health_status(), json!({ "status": "disabled" }));
	}

	#[test]
	fn path_classes() {
		let fixed = |method: Method, path: &str| match path_class(&method, path) {
			PathClass::Fixed(class) => class,
			#[cfg(feature = "graphql")]
			PathClass::GraphQl => panic!("graphql needs the body"),
		};
		assert_eq!(fixed(Method::POST, "/api/v2/auth/login"), Class::Auth);
		assert_eq!(fixed(Method::POST, "/api/v2/auth/register"), Class::Auth);
		assert_eq!(fixed(Method::GET, "/api/v2/auth/oidc/callback"), Class::Auth);
		assert_eq!(fixed(Method::GET, "/api/v2/auth/me"), Class::Stream);
		assert_eq!(fixed(Method::POST, "/api/v2/devices/pair/start"), Class::Auth);
		assert_eq!(fixed(Method::POST, "/v1/tokens"), Class::Auth);
		assert_eq!(fixed(Method::POST, "/kobo/k/v1/auth/device"), Class::Auth);
		assert_eq!(fixed(Method::GET, "/kobo/k/v1/library/sync"), Class::Write);
		assert_eq!(fixed(Method::PUT, "/kobo/k/v1/library/b/state"), Class::Write);
		assert_eq!(fixed(Method::PUT, "/koreader/k/syncs/progress"), Class::Write);
		assert_eq!(fixed(Method::GET, "/koreader/k/syncs/progress/d"), Class::Stream);
		assert_eq!(fixed(Method::POST, "/v1/ops"), Class::Write);
		assert_eq!(fixed(Method::POST, "/v1/annotations"), Class::Write);
		assert_eq!(fixed(Method::POST, "/v1/works/resolve"), Class::Stream);
		assert_eq!(fixed(Method::POST, "/api/v1/series/list"), Class::Stream);
		assert_eq!(fixed(Method::POST, "/komga/api/v1/books/list"), Class::Stream);
		assert_eq!(fixed(Method::POST, "/api/v2/version"), Class::Stream);
		assert_eq!(fixed(Method::PATCH, "/api/v1/books/1/read-progress"), Class::Write);
		assert_eq!(fixed(Method::PUT, "/opds/v2.0/books/1/progression"), Class::Write);
		assert_eq!(fixed(Method::GET, "/api/v1/books/1/pages/1"), Class::Stream);
		assert_eq!(fixed(Method::GET, "/opds/v1.2/books/1/file/x.cbz"), Class::Stream);
		assert_eq!(fixed(Method::HEAD, "/login"), Class::Stream);
	}

	#[test]
	fn credential_selection() {
		let mut headers = HeaderMap::new();
		assert_eq!(credential(&headers, "/api/v2/media"), (None, false));
		assert_eq!(
			credential(&headers, "/opds/stump_abc/v1.2/catalog"),
			(Some(fingerprint(b"stump_abc")), true)
		);
		assert_eq!(credential(&headers, "/opds/v1.2/catalog"), (None, false));
		assert_eq!(
			credential(&headers, "/koreader/stump_k/users/auth"),
			(Some(fingerprint(b"stump_k")), true)
		);

		headers.insert(
			header::COOKIE,
			HeaderValue::from_static("theme=dark; stump_session=abc123"),
		);
		assert_eq!(
			credential(&headers, "/api/v2/media"),
			(Some(fingerprint(b"abc123")), false)
		);
		headers.insert(
			KOMGA_API_KEY_HEADER,
			HeaderValue::from_static("stump_key"),
		);
		assert_eq!(
			credential(&headers, "/api/v1/books"),
			(Some(fingerprint(b"stump_key")), true)
		);
		headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer jwt"));
		assert_eq!(
			credential(&headers, "/api/v1/books"),
			(Some(fingerprint(b"Bearer jwt")), false)
		);
		headers.insert(header::AUTHORIZATION, HeaderValue::from_static("BASIC dXNlcjpwYXNz"));
		assert_eq!(
			credential(&headers, "/api/v1/books"),
			(Some(fingerprint(b"BASIC dXNlcjpwYXNz")), true)
		);
	}

	#[test]
	fn retry_after_rounds_up_to_whole_seconds() {
		let response = too_many_requests(Duration::from_millis(1500));
		assert_eq!(response.headers()[header::RETRY_AFTER], "2");
		let response = too_many_requests(Duration::ZERO);
		assert_eq!(response.headers()[header::RETRY_AFTER], "1");
	}
}
