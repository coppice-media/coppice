//! Source health: probe a catalog source's base URL (HEAD, falling back to
//! GET) and its theme's latest-updates path with a 10 s timeout; persist
//! status, latency, redirect target, and consecutive failures. A source is
//! `DEAD` after [`DEAD_AFTER_FAILURES`] consecutive failed runs.

use std::{
	collections::HashMap,
	sync::Arc,
	time::{Duration, Instant},
};

use chrono::Utc;
use futures::{stream, StreamExt};
use models::entity::{provider_source, source_health};
use sea_orm::{
	sea_query::OnConflict, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait,
	QueryFilter,
};

use crate::{
	catalog::{CatalogSnapshot, SourceTheme},
	event::ProviderEvent,
	http::RequestHeaders,
};

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEAD_AFTER_FAILURES: i32 = 3;
/// `source_health.error` for a host behind a Cloudflare managed challenge.
pub const CHALLENGE_ERROR: &str = "cloudflare challenge";
/// Bytes of landing-page markup inspected for theme detection.
const THEME_SNIFF_LIMIT: usize = 512 * 1024;
/// Parallel base URLs probed by one run.
pub const DEFAULT_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
	Unknown,
	Ok,
	Degraded,
	Dead,
}

impl HealthStatus {
	pub fn as_str(self) -> &'static str {
		match self {
			HealthStatus::Unknown => "UNKNOWN",
			HealthStatus::Ok => "OK",
			HealthStatus::Degraded => "DEGRADED",
			HealthStatus::Dead => "DEAD",
		}
	}

	pub fn parse(value: &str) -> Self {
		match value {
			"OK" => HealthStatus::Ok,
			"DEGRADED" => HealthStatus::Degraded,
			"DEAD" => HealthStatus::Dead,
			_ => HealthStatus::Unknown,
		}
	}
}

/// Outcome of probing one base URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HealthProbe {
	pub http_status: Option<u16>,
	pub latency_ms: Option<u32>,
	pub redirect_url: Option<String>,
	pub theme: Option<SourceTheme>,
	pub latest_path_ok: Option<bool>,
	/// The base URL answered with a Cloudflare managed challenge: the host is
	/// up and gated, not broken (`crate::http::challenge_host`).
	pub challenged: bool,
	pub error: Option<String>,
}

impl HealthProbe {
	/// The base URL answered successfully.
	pub fn reachable(&self) -> bool {
		self.http_status
			.is_some_and(|status| (200..400).contains(&status))
	}

	/// Fold this probe into the previous failure count: `(status, failures)`.
	/// `dead_after` is the number of consecutive failures that marks a source
	/// dead (`provider_health_dead_after`).
	///
	/// A Cloudflare challenge never counts as a failure: the host answered, it
	/// simply refuses clients that cannot run the challenge script, so it is
	/// reported `DEGRADED` for as long as it stays gated and the count is
	/// cleared like any other answered probe. Escalating one to `DEAD` would
	/// hide a live source from the catalog for a reason an operator can fix
	/// with a cookie.
	pub fn next_state(
		&self,
		previous_failures: i32,
		dead_after: i32,
	) -> (HealthStatus, i32) {
		if self.challenged {
			return (HealthStatus::Degraded, 0);
		}
		if !self.reachable() {
			let failures = previous_failures.saturating_add(1);
			let status = if failures >= dead_after.max(1) {
				HealthStatus::Dead
			} else {
				HealthStatus::Degraded
			};
			return (status, failures);
		}
		match self.latest_path_ok {
			Some(false) => (HealthStatus::Degraded, 0),
			_ => (HealthStatus::Ok, 0),
		}
	}
}

#[derive(Clone)]
pub struct HealthChecker {
	client: reqwest::Client,
}

impl HealthChecker {
	pub fn new() -> Result<Self, reqwest::Error> {
		Ok(Self {
			client: crate::http::build_client(None, PROBE_TIMEOUT)?,
		})
	}

	pub fn with_client(client: reqwest::Client) -> Self {
		Self { client }
	}

	/// Probe `base_url` as any anonymous client sees it. `known_theme` skips
	/// markup sniffing when the theme was detected by an earlier run.
	pub async fn probe(
		&self,
		base_url: &str,
		known_theme: Option<SourceTheme>,
	) -> HealthProbe {
		self.probe_with(base_url, known_theme, &RequestHeaders::default())
			.await
	}

	/// Probe `base_url` carrying one source instance's configured request
	/// headers.
	///
	/// This is never the *first* probe of a host: the unauthenticated answer
	/// is what a catalog row reports, and this one answers the different
	/// question an operator has about a source they configured — whether the
	/// clearance it carries still gets through
	/// ([`probe_target`] runs it only after an unauthenticated probe came
	/// back challenged).
	pub async fn probe_with(
		&self,
		base_url: &str,
		known_theme: Option<SourceTheme>,
		headers: &RequestHeaders,
	) -> HealthProbe {
		let mut probe = HealthProbe::default();
		let started = Instant::now();
		let head = headers.apply(self.client.head(base_url)).send().await;
		let response = match head {
			Ok(response) if !matches!(response.status().as_u16(), 405 | 501 | 403) => {
				Ok(response)
			},
			_ => headers.apply(self.client.get(base_url)).send().await,
		};
		let response = match response {
			Ok(response) => response,
			Err(error) => {
				probe.error = Some(error.to_string());
				return probe;
			},
		};
		probe.latency_ms =
			Some(started.elapsed().as_millis().min(u32::MAX as u128) as u32);
		probe.http_status = Some(response.status().as_u16());
		probe.redirect_url = redirect_target(base_url, response.url().as_str());
		if let Some(host) = crate::http::challenge_host(
			response.status().as_u16(),
			response.headers(),
			response.url().as_str(),
		) {
			// Reported, never worked around: the probe deliberately carries no
			// clearance cookie, so an operator sees that the source needs one
			// instead of a green row that only the configured instance can
			// reach.
			tracing::debug!(host, "Source is behind a Cloudflare challenge");
			probe.challenged = true;
			probe.error = Some(CHALLENGE_ERROR.to_string());
			return probe;
		}
		if !probe.reachable() {
			probe.error = Some(format!("HTTP {}", response.status().as_u16()));
			return probe;
		}

		probe.theme = known_theme;
		if probe.theme.is_none() {
			let html = match headers.apply(self.client.get(base_url)).send().await {
				Ok(response) => response.bytes().await.ok().map(|bytes| {
					let end = bytes.len().min(THEME_SNIFF_LIMIT);
					String::from_utf8_lossy(&bytes[..end]).into_owned()
				}),
				Err(_) => None,
			};
			probe.theme = html.as_deref().and_then(SourceTheme::detect);
		}

		if let Some(theme) = probe.theme {
			let latest =
				format!("{}{}", base_url.trim_end_matches('/'), theme.latest_path());
			probe.latest_path_ok =
				Some(match headers.apply(self.client.get(&latest)).send().await {
					Ok(response) => response.status().is_success(),
					Err(_) => false,
				});
		}
		probe
	}
}

/// `Some(final_url)` when the request landed somewhere other than `base_url`
/// (ignoring a trailing slash).
fn redirect_target(base_url: &str, final_url: &str) -> Option<String> {
	let same = base_url.trim_end_matches('/') == final_url.trim_end_matches('/');
	(!same).then(|| final_url.to_string())
}

/// One source whose persisted status differed from what this run observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatusChange {
	/// `source_health.source_id`, the Keiyoushi catalog source id.
	pub source_id: String,
	pub name: String,
	pub status: HealthStatus,
}

impl SourceStatusChange {
	/// The host-facing event for this transition.
	pub fn as_event(&self) -> ProviderEvent {
		ProviderEvent::SourceHealthChanged {
			source_id: self.source_id.clone(),
			name: self.name.clone(),
			status: self.status,
		}
	}
}

/// An enabled source instance whose host is still gated after the request
/// headers the instance already carries were tried. This is the input of an
/// automatic `challenge_solve`: the one thing that can clear it is a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengedInstance {
	/// `provider_sources.id`.
	pub instance_id: String,
	/// The host the challenge sits on, after any redirect.
	pub host: String,
	/// The URL a browser should be pointed at to earn the clearance.
	pub url: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HealthRunSummary {
	pub probed_urls: usize,
	pub updated_sources: usize,
	pub ok: usize,
	pub degraded: usize,
	pub dead: usize,
	/// Sources whose status *changed* in this run. A source observed for the
	/// first time is not a change: a cold run over a full catalog writes
	/// every row but announces nothing.
	pub changed: Vec<SourceStatusChange>,
	/// Enabled instances still behind a challenge after their configured
	/// headers were tried. Empty unless a probe came back challenged, so a
	/// healthy run allocates nothing.
	pub challenged: Vec<ChallengedInstance>,
}

/// One base URL to probe, with every catalog source that shares it. This is
/// the unit of work of a health run (and of the core health job's tasks).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthTarget {
	/// Base URL without its trailing slash.
	pub base_url: String,
	/// Catalog sources served by `base_url`.
	pub sources: Vec<crate::catalog::CatalogSource>,
	/// Theme detected by an earlier run; skips the markup sniff.
	pub known_theme: Option<SourceTheme>,
}

/// Group the catalog's sources by base URL, newest known theme attached, so
/// each host is probed exactly once. Enabled instance base URLs listed in
/// `priority` come first.
pub async fn plan<C: ConnectionTrait>(
	conn: &C,
	snapshot: &CatalogSnapshot,
	priority: &[String],
) -> Result<Vec<HealthTarget>, sea_orm::DbErr> {
	let themes: HashMap<String, SourceTheme> = source_health::Entity::find()
		.all(conn)
		.await?
		.into_iter()
		.filter_map(|row| {
			let theme = row.theme.as_deref().and_then(SourceTheme::parse)?;
			Some((row.source_id, theme))
		})
		.collect();

	let mut by_url: HashMap<String, Vec<crate::catalog::CatalogSource>> = HashMap::new();
	for entry in &snapshot.entries {
		for source in &entry.sources {
			if source.base_url.is_empty() {
				continue;
			}
			by_url
				.entry(source.base_url.trim_end_matches('/').to_string())
				.or_default()
				.push(source.clone());
		}
	}

	let priority: Vec<&str> = priority
		.iter()
		.map(|url| url.trim_end_matches('/'))
		.collect();
	let mut targets: Vec<HealthTarget> = by_url
		.into_iter()
		.map(|(base_url, sources)| {
			let known_theme = sources
				.iter()
				.find_map(|source| themes.get(&source.id).copied());
			HealthTarget {
				base_url,
				sources,
				known_theme,
			}
		})
		.collect();
	targets.sort_by(|a, b| a.base_url.cmp(&b.base_url));
	targets.sort_by_key(|target| !priority.contains(&target.base_url.as_str()));
	Ok(targets)
}

/// The health target for one base URL: every catalog source that shares it,
/// and the theme an earlier run detected.
///
/// `sources` is empty when the catalog does not know the host — a source
/// enabled straight from a definition repository, or a snapshot that has not
/// been fetched. [`probe_target`] then probes and reports as usual and simply
/// writes no rows, which is the honest outcome: there is no catalog source
/// for the row to be about.
pub async fn target_for<C: ConnectionTrait>(
	conn: &C,
	snapshot: &CatalogSnapshot,
	base_url: &str,
) -> Result<HealthTarget, sea_orm::DbErr> {
	let base_url = base_url.trim_end_matches('/').to_string();
	let sources: Vec<crate::catalog::CatalogSource> = snapshot
		.entries
		.iter()
		.flat_map(|entry| entry.sources.iter())
		.filter(|source| source.base_url.trim_end_matches('/') == base_url)
		.cloned()
		.collect();
	let known_theme = if sources.is_empty() {
		None
	} else {
		source_health::Entity::find()
			.filter(
				source_health::Column::SourceId
					.is_in(sources.iter().map(|source| source.id.clone())),
			)
			.all(conn)
			.await?
			.into_iter()
			.find_map(|row| row.theme.as_deref().and_then(SourceTheme::parse))
	};
	Ok(HealthTarget {
		base_url,
		sources,
		known_theme,
	})
}

/// Probe one target and upsert a `source_health` row for every catalog
/// source sharing its base URL. A source is marked dead once it has
/// `dead_after` consecutive failed runs; one reachable run resets the count.
///
/// A host that answers the anonymous probe with a Cloudflare challenge is
/// probed a *second* time, once per enabled instance that carries request
/// headers, and the first attempt that gets through is what the row records.
/// The two probes answer two different questions and both are worth asking:
/// the anonymous one is what any client sees (and is the only one a source
/// nobody configured gets), the authenticated one is whether this server can
/// reach it — and without the second the row of a source whose clearance
/// works would sit at `DEGRADED` forever, with no way to tell it from one
/// that is genuinely gated.
///
/// Every enabled instance the challenge survives is reported in
/// [`HealthRunSummary::challenged`], which is what the caller feeds to an
/// automatic solve.
pub async fn probe_target<C: ConnectionTrait>(
	conn: &C,
	checker: &HealthChecker,
	target: &HealthTarget,
	dead_after: i32,
) -> Result<HealthRunSummary, sea_orm::DbErr> {
	let mut probe = checker.probe(&target.base_url, target.known_theme).await;
	let mut challenged = Vec::new();
	if probe.challenged {
		probe =
			clear_with_configured_headers(conn, checker, target, probe, &mut challenged)
				.await?;
	}
	let source_ids: Vec<String> = target
		.sources
		.iter()
		.map(|source| source.id.clone())
		.collect();
	let existing: HashMap<String, source_health::Model> = source_health::Entity::find()
		.filter(source_health::Column::SourceId.is_in(source_ids))
		.all(conn)
		.await?
		.into_iter()
		.map(|row| (row.source_id.clone(), row))
		.collect();

	let mut summary = HealthRunSummary {
		probed_urls: 1,
		challenged,
		..Default::default()
	};
	let checked_at = Utc::now();
	for source in &target.sources {
		let previous = existing.get(&source.id);
		let previous_failures = previous
			.map(|row| row.consecutive_failures)
			.unwrap_or_default();
		let (status, failures) = probe.next_state(previous_failures, dead_after);
		match status {
			HealthStatus::Ok => summary.ok += 1,
			HealthStatus::Degraded => summary.degraded += 1,
			HealthStatus::Dead => summary.dead += 1,
			HealthStatus::Unknown => {},
		}
		// A first observation writes the row but is not a transition: one
		// cold run over the whole catalog would otherwise announce every
		// source at once.
		if previous.is_some_and(|row| HealthStatus::parse(&row.status) != status) {
			summary.changed.push(SourceStatusChange {
				source_id: source.id.clone(),
				name: source.name.clone(),
				status,
			});
		}
		let model = source_health::ActiveModel {
			source_id: Set(source.id.clone()),
			name: Set(source.name.clone()),
			lang: Set(source.lang.clone()),
			base_url: Set(source.base_url.clone()),
			theme: Set(probe
				.theme
				.map(|theme| theme.as_str().to_string())
				.or_else(|| existing.get(&source.id).and_then(|row| row.theme.clone()))),
			status: Set(status.as_str().to_string()),
			http_status: Set(probe.http_status.map(i32::from)),
			latency_ms: Set(probe.latency_ms.map(|ms| ms.min(i32::MAX as u32) as i32)),
			redirect_url: Set(probe.redirect_url.clone()),
			latest_path_ok: Set(probe.latest_path_ok),
			consecutive_failures: Set(failures),
			error: Set(probe.error.clone()),
			checked_at: Set(Some(checked_at.into())),
		};
		source_health::Entity::insert(model)
			.on_conflict(
				OnConflict::column(source_health::Column::SourceId)
					.update_columns([
						source_health::Column::Name,
						source_health::Column::Lang,
						source_health::Column::BaseUrl,
						source_health::Column::Theme,
						source_health::Column::Status,
						source_health::Column::HttpStatus,
						source_health::Column::LatencyMs,
						source_health::Column::RedirectUrl,
						source_health::Column::LatestPathOk,
						source_health::Column::ConsecutiveFailures,
						source_health::Column::Error,
						source_health::Column::CheckedAt,
					])
					.to_owned(),
			)
			.exec(conn)
			.await?;
		summary.updated_sources += 1;
	}
	Ok(summary)
}

/// Re-probe a gated host with each enabled instance's configured request
/// headers, and record the instances the challenge survived.
///
/// Every instance is tried on its own: they share a host, so in practice they
/// share one clearance, but each carries its own header map and "can *this*
/// instance reach the host" is the question a solve is queued from. The first
/// probe that gets through is what the shared `source_health` row records; an
/// instance with no configured headers has nothing to try and is reported
/// challenged without a request, which is exactly the state an automatic
/// solve exists to leave.
async fn clear_with_configured_headers<C: ConnectionTrait>(
	conn: &C,
	checker: &HealthChecker,
	target: &HealthTarget,
	anonymous: HealthProbe,
	challenged: &mut Vec<ChallengedInstance>,
) -> Result<HealthProbe, sea_orm::DbErr> {
	let url = anonymous
		.redirect_url
		.clone()
		.unwrap_or_else(|| target.base_url.clone());
	let host = crate::http::host_of(&url);
	// Enabled instances are a handful, and matching a base URL means
	// comparing it without its trailing slash, which is not a SQL predicate
	// worth writing.
	let instances = provider_source::Entity::find()
		.filter(provider_source::Column::Enabled.eq(true))
		.all(conn)
		.await?;

	let mut cleared = None;
	for instance in instances {
		if instance.base_url.trim_end_matches('/') != target.base_url {
			continue;
		}
		let headers = RequestHeaders::parse(instance.request_headers.as_deref());
		if !headers.is_empty() {
			let probe = checker
				.probe_with(&target.base_url, target.known_theme, &headers)
				.await;
			if !probe.challenged {
				tracing::debug!(
					instance = %instance.id,
					host,
					"Configured request headers cleared the Cloudflare challenge"
				);
				cleared = cleared.or(Some(probe));
				continue;
			}
		}
		challenged.push(ChallengedInstance {
			instance_id: instance.id,
			host: host.clone(),
			url: url.clone(),
		});
	}
	Ok(cleared.unwrap_or(anonymous))
}

impl HealthRunSummary {
	/// Fold another run's (or target's) counters and transitions into this one.
	pub fn merge(&mut self, other: HealthRunSummary) {
		self.probed_urls += other.probed_urls;
		self.updated_sources += other.updated_sources;
		self.ok += other.ok;
		self.degraded += other.degraded;
		self.dead += other.dead;
		self.changed.extend(other.changed);
		self.challenged.extend(other.challenged);
	}
}

/// Probe every distinct base URL in the catalog and upsert one
/// `source_health` row per catalog source. Enabled instance base URLs listed
/// in `priority` are probed first.
pub async fn check_catalog<C: ConnectionTrait>(
	conn: &C,
	checker: &HealthChecker,
	snapshot: &CatalogSnapshot,
	priority: &[String],
	concurrency: usize,
	dead_after: i32,
) -> Result<HealthRunSummary, sea_orm::DbErr> {
	let targets = plan(conn, snapshot, priority).await?;
	// Probes run concurrently; the upserts they hand back are applied on
	// this task, so one SQLite writer is never contended by the fan-out.
	let checker = Arc::new(checker.clone());
	let results: Vec<Result<HealthRunSummary, sea_orm::DbErr>> =
		stream::iter(targets.iter())
			.map(|target| {
				let checker = checker.clone();
				async move { probe_target(conn, checker.as_ref(), target, dead_after).await }
			})
			.buffer_unordered(concurrency.max(1))
			.collect()
			.await;

	let mut summary = HealthRunSummary::default();
	for result in results {
		summary.merge(result?);
	}
	Ok(summary)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		catalog::{CatalogEntry, CatalogSource},
		mock_http::{CannedResponse, MockServer},
	};
	use sea_orm::{ColumnTrait, QueryFilter};

	fn source(id: &str, name: &str, base_url: &str) -> CatalogEntry {
		CatalogEntry {
			name: name.to_string(),
			pkg: format!("eu.kanade.tachiyomi.extension.en.{}", name.to_lowercase()),
			apk_url: None,
			icon_url: None,
			lang: "en".to_string(),
			version: "1.0.0".to_string(),
			nsfw: false,
			sources: vec![CatalogSource {
				id: id.to_string(),
				name: name.to_string(),
				lang: "en".to_string(),
				base_url: base_url.to_string(),
			}],
		}
	}

	#[test]
	fn failures_escalate_to_dead_after_the_configured_count() {
		let failed = HealthProbe {
			http_status: Some(503),
			..Default::default()
		};
		let d = DEAD_AFTER_FAILURES;
		assert_eq!(failed.next_state(0, d), (HealthStatus::Degraded, 1));
		assert_eq!(failed.next_state(1, d), (HealthStatus::Degraded, 2));
		assert_eq!(failed.next_state(2, d), (HealthStatus::Dead, 3));
		let unreachable = HealthProbe::default();
		assert_eq!(unreachable.next_state(5, d), (HealthStatus::Dead, 6));
		// A configured threshold replaces the default in both directions.
		assert_eq!(failed.next_state(0, 1), (HealthStatus::Dead, 1));
		assert_eq!(failed.next_state(3, 5), (HealthStatus::Degraded, 4));
		// A nonsensical threshold still marks the first failure dead rather
		// than never escalating.
		assert_eq!(failed.next_state(0, 0), (HealthStatus::Dead, 1));

		let healthy = HealthProbe {
			http_status: Some(200),
			latest_path_ok: Some(true),
			..Default::default()
		};
		// Recovery resets the count, so a dead source needs `dead_after`
		// fresh failures to die again.
		assert_eq!(healthy.next_state(9, d), (HealthStatus::Ok, 0));
		let latest_broken = HealthProbe {
			http_status: Some(200),
			latest_path_ok: Some(false),
			..Default::default()
		};
		assert_eq!(latest_broken.next_state(0, d), (HealthStatus::Degraded, 0));
	}

	#[tokio::test]
	async fn probe_detects_theme_and_checks_latest_path() {
		let server = MockServer::spawn(vec![
			(
				"/",
				CannedResponse::html(
					"<html><link href='/wp-content/themes/madara/x.css'></html>",
				),
			),
			(
				"/manga/?m_orderby=latest",
				CannedResponse::html("<html>latest</html>"),
			),
		])
		.await;
		let checker = HealthChecker::with_client(reqwest::Client::new());
		let probe = checker.probe(server.base_url(), None).await;
		assert_eq!(probe.http_status, Some(200));
		assert_eq!(probe.theme, Some(SourceTheme::Madara));
		assert_eq!(probe.latest_path_ok, Some(true));
		assert!(probe.redirect_url.is_none());
		assert!(probe.latency_ms.is_some());
		assert!(probe.error.is_none());

		// A known theme skips the markup sniff: only HEAD + latest path.
		let before = server.requests().len();
		let probe = checker
			.probe(server.base_url(), Some(SourceTheme::Madara))
			.await;
		assert_eq!(probe.theme, Some(SourceTheme::Madara));
		assert_eq!(server.requests().len() - before, 2);
	}

	#[tokio::test]
	async fn probe_reports_failures_and_redirects() {
		let server = MockServer::spawn(vec![("/", CannedResponse::status(503))]).await;
		let checker = HealthChecker::with_client(reqwest::Client::new());
		let probe = checker.probe(server.base_url(), None).await;
		assert_eq!(probe.http_status, Some(503));
		assert!(!probe.reachable());
		assert_eq!(probe.error.as_deref(), Some("HTTP 503"));

		let target =
			MockServer::spawn(vec![("/", CannedResponse::html("<html/>"))]).await;
		let redirecting = MockServer::spawn(vec![(
			"/",
			CannedResponse::status(302).with_header("Location", target.base_url()),
		)])
		.await;
		let probe = checker.probe(redirecting.base_url(), None).await;
		assert_eq!(probe.http_status, Some(200));
		assert_eq!(
			probe
				.redirect_url
				.as_deref()
				.map(|u| u.trim_end_matches('/')),
			Some(target.base_url())
		);

		let closed = MockServer::spawn(vec![]).await;
		let dead_url = closed.base_url().to_string();
		drop(closed);
		let probe = checker.probe("http://127.0.0.1:1", None).await;
		assert!(probe.http_status.is_none());
		assert!(probe.error.is_some());
		let _ = dead_url;
	}

	#[tokio::test]
	async fn check_catalog_upserts_rows_and_tracks_consecutive_failures() {
		let conn = ::tests::db::test_database().await;
		let healthy =
			MockServer::spawn(vec![("/", CannedResponse::html("<html/>"))]).await;
		let broken = MockServer::spawn(vec![("/", CannedResponse::status(500))]).await;
		let snapshot = CatalogSnapshot {
			fetched_at: Utc::now(),
			entries: vec![
				source("1", "Alpha", healthy.base_url()),
				source("2", "AlphaMirror", healthy.base_url()),
				source("3", "Broken", broken.base_url()),
			],
		};
		let checker = HealthChecker::with_client(reqwest::Client::new());

		for run in 1..=3 {
			let summary =
				check_catalog(&conn, &checker, &snapshot, &[], 4, DEAD_AFTER_FAILURES)
					.await
					.unwrap();
			assert_eq!(summary.probed_urls, 2, "run {run}");
			assert_eq!(summary.updated_sources, 3, "run {run}");
		}
		// The shared base URL is probed once per run, not once per source.
		assert_eq!(healthy.request_count("/"), 3 * 2);

		let rows = source_health::Entity::find().all(&conn).await.unwrap();
		assert_eq!(rows.len(), 3);
		let alpha = rows.iter().find(|row| row.source_id == "1").unwrap();
		assert_eq!(alpha.status, "OK");
		assert_eq!(alpha.http_status, Some(200));
		assert_eq!(alpha.consecutive_failures, 0);
		assert!(alpha.checked_at.is_some());
		async fn dead_row(conn: &sea_orm::DatabaseConnection) -> source_health::Model {
			source_health::Entity::find()
				.filter(source_health::Column::SourceId.eq("3"))
				.one(conn)
				.await
				.unwrap()
				.unwrap()
		}
		let dead = dead_row(&conn).await;
		assert_eq!(dead.consecutive_failures, 3);
		assert_eq!(dead.status, "DEAD");
		assert_eq!(dead.http_status, Some(500));

		// Recovery: one reachable run clears the count and the DEAD mark.
		broken.set_route("/", CannedResponse::html("<html/>"));
		check_catalog(&conn, &checker, &snapshot, &[], 4, DEAD_AFTER_FAILURES)
			.await
			.unwrap();
		let recovered = dead_row(&conn).await;
		assert_eq!(recovered.status, "OK");
		assert_eq!(recovered.consecutive_failures, 0);
		assert_eq!(recovered.http_status, Some(200));
		assert!(recovered.error.is_none());

		// And a lower configured threshold kills it on the next failure.
		broken.set_route("/", CannedResponse::status(500));
		let summary = check_catalog(&conn, &checker, &snapshot, &[], 4, 1)
			.await
			.unwrap();
		assert_eq!(summary.dead, 1);
		let dead_again = dead_row(&conn).await;
		assert_eq!(dead_again.status, "DEAD");
		assert_eq!(dead_again.consecutive_failures, 1);
	}

	/// `readcomicsonline.ru` since 2026-06: every HTML path answers `403`
	/// with `cf-mitigated: challenge`. The site is up, so it must be reported
	/// as gated for as long as it stays gated — never buried as `DEAD`, which
	/// would hide it from the catalog for a reason a cookie fixes.
	#[tokio::test]
	async fn a_challenged_source_stays_degraded_and_never_dies() {
		let conn = ::tests::db::test_database().await;
		let challenged = MockServer::spawn(vec![(
			"/",
			CannedResponse::status(403).with_header("cf-mitigated", "challenge"),
		)])
		.await;
		let snapshot = CatalogSnapshot {
			fetched_at: Utc::now(),
			entries: vec![source("1", "Gated", challenged.base_url())],
		};
		let checker = HealthChecker::with_client(reqwest::Client::new());

		// One run past `dead_after` is where a plain failure would flip to
		// DEAD; a challenge must not.
		for run in 1..=DEAD_AFTER_FAILURES + 1 {
			let summary =
				check_catalog(&conn, &checker, &snapshot, &[], 4, DEAD_AFTER_FAILURES)
					.await
					.unwrap();
			assert_eq!(summary.degraded, 1, "run {run}");
			assert_eq!(summary.dead, 0, "run {run}");
			let row = source_health::Entity::find()
				.filter(source_health::Column::SourceId.eq("1"))
				.one(&conn)
				.await
				.unwrap()
				.unwrap();
			assert_eq!(row.status, "DEGRADED", "run {run}");
			assert_eq!(row.consecutive_failures, 0, "run {run}");
			assert_eq!(row.http_status, Some(403), "run {run}");
			assert_eq!(row.error.as_deref(), Some(CHALLENGE_ERROR), "run {run}");
			// The challenge is reported before theme sniffing, so no
			// latest-path claim is invented from an interstitial.
			assert_eq!(row.latest_path_ok, None, "run {run}");
		}

		let probe = checker.probe(challenged.base_url(), None).await;
		assert!(probe.challenged);
		assert!(!probe.reachable());
		// And the same host answering a plain 403 does escalate.
		challenged.set_route("/", CannedResponse::status(403));
		let plain = checker.probe(challenged.base_url(), None).await;
		assert!(!plain.challenged);
		assert_eq!(plain.error.as_deref(), Some("HTTP 403"));
		assert_eq!(
			plain.next_state(2, DEAD_AFTER_FAILURES),
			(HealthStatus::Dead, 3)
		);
	}
}
