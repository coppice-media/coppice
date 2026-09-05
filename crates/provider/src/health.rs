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
use models::entity::source_health;
use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait};

use crate::catalog::{CatalogSnapshot, SourceTheme};

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEAD_AFTER_FAILURES: i32 = 3;
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
	pub error: Option<String>,
}

impl HealthProbe {
	/// The base URL answered successfully.
	pub fn reachable(&self) -> bool {
		self.http_status
			.is_some_and(|status| (200..400).contains(&status))
	}

	/// Fold this probe into the previous failure count: `(status, failures)`.
	pub fn next_state(&self, previous_failures: i32) -> (HealthStatus, i32) {
		if !self.reachable() {
			let failures = previous_failures.saturating_add(1);
			let status = if failures >= DEAD_AFTER_FAILURES {
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

	/// Probe `base_url`. `known_theme` skips markup sniffing when the theme
	/// was detected by an earlier run.
	pub async fn probe(
		&self,
		base_url: &str,
		known_theme: Option<SourceTheme>,
	) -> HealthProbe {
		let mut probe = HealthProbe::default();
		let started = Instant::now();
		let head = self.client.head(base_url).send().await;
		let response = match head {
			Ok(response) if !matches!(response.status().as_u16(), 405 | 501 | 403) => {
				Ok(response)
			},
			_ => self.client.get(base_url).send().await,
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
		if !probe.reachable() {
			probe.error = Some(format!("HTTP {}", response.status().as_u16()));
			return probe;
		}

		probe.theme = known_theme;
		if probe.theme.is_none() {
			let html = match self.client.get(base_url).send().await {
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
			probe.latest_path_ok = Some(match self.client.get(&latest).send().await {
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HealthRunSummary {
	pub probed_urls: usize,
	pub updated_sources: usize,
	pub ok: usize,
	pub degraded: usize,
	pub dead: usize,
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
) -> Result<HealthRunSummary, sea_orm::DbErr> {
	let existing: HashMap<String, source_health::Model> = source_health::Entity::find()
		.all(conn)
		.await?
		.into_iter()
		.map(|row| (row.source_id.clone(), row))
		.collect();

	// base_url -> catalog sources sharing it
	let mut by_url: HashMap<
		String,
		Vec<(
			&crate::catalog::CatalogEntry,
			&crate::catalog::CatalogSource,
		)>,
	> = HashMap::new();
	for entry in &snapshot.entries {
		for source in &entry.sources {
			if source.base_url.is_empty() {
				continue;
			}
			by_url
				.entry(source.base_url.trim_end_matches('/').to_string())
				.or_default()
				.push((entry, source));
		}
	}

	let mut urls: Vec<String> = by_url.keys().cloned().collect();
	urls.sort();
	let priority: Vec<String> = priority
		.iter()
		.map(|url| url.trim_end_matches('/').to_string())
		.collect();
	urls.sort_by_key(|url| !priority.contains(url));

	let checker = Arc::new(checker.clone());
	let known_themes: HashMap<String, SourceTheme> = by_url
		.iter()
		.filter_map(|(url, sources)| {
			sources.iter().find_map(|(_, source)| {
				existing
					.get(&source.id)
					.and_then(|row| row.theme.as_deref())
					.and_then(SourceTheme::parse)
					.map(|theme| (url.clone(), theme))
			})
		})
		.collect();

	let probes: Vec<(String, HealthProbe)> = stream::iter(urls.into_iter())
		.map(|url| {
			let checker = checker.clone();
			let theme = known_themes.get(&url).copied();
			async move {
				let probe = checker.probe(&url, theme).await;
				(url, probe)
			}
		})
		.buffer_unordered(concurrency.max(1))
		.collect()
		.await;

	let mut summary = HealthRunSummary {
		probed_urls: probes.len(),
		..Default::default()
	};
	let checked_at = Utc::now();
	for (url, probe) in probes {
		let Some(sources) = by_url.get(&url) else {
			continue;
		};
		for (_, source) in sources {
			let previous_failures = existing
				.get(&source.id)
				.map(|row| row.consecutive_failures)
				.unwrap_or_default();
			let (status, failures) = probe.next_state(previous_failures);
			match status {
				HealthStatus::Ok => summary.ok += 1,
				HealthStatus::Degraded => summary.degraded += 1,
				HealthStatus::Dead => summary.dead += 1,
				HealthStatus::Unknown => {},
			}
			let model = source_health::ActiveModel {
				source_id: Set(source.id.clone()),
				name: Set(source.name.clone()),
				lang: Set(source.lang.clone()),
				base_url: Set(source.base_url.clone()),
				theme: Set(probe.theme.map(|theme| theme.as_str().to_string()).or_else(
					|| existing.get(&source.id).and_then(|row| row.theme.clone()),
				)),
				status: Set(status.as_str().to_string()),
				http_status: Set(probe.http_status.map(i32::from)),
				latency_ms: Set(probe
					.latency_ms
					.map(|ms| ms.min(i32::MAX as u32) as i32)),
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
	fn failures_escalate_to_dead_after_three_runs() {
		let failed = HealthProbe {
			http_status: Some(503),
			..Default::default()
		};
		assert_eq!(failed.next_state(0), (HealthStatus::Degraded, 1));
		assert_eq!(failed.next_state(1), (HealthStatus::Degraded, 2));
		assert_eq!(failed.next_state(2), (HealthStatus::Dead, 3));
		let unreachable = HealthProbe::default();
		assert_eq!(unreachable.next_state(5), (HealthStatus::Dead, 6));

		let healthy = HealthProbe {
			http_status: Some(200),
			latest_path_ok: Some(true),
			..Default::default()
		};
		assert_eq!(healthy.next_state(2), (HealthStatus::Ok, 0));
		let latest_broken = HealthProbe {
			http_status: Some(200),
			latest_path_ok: Some(false),
			..Default::default()
		};
		assert_eq!(latest_broken.next_state(0), (HealthStatus::Degraded, 0));
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
			let summary = check_catalog(&conn, &checker, &snapshot, &[], 4)
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
		let dead = source_health::Entity::find()
			.filter(source_health::Column::SourceId.eq("3"))
			.one(&conn)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(dead.consecutive_failures, 3);
		assert_eq!(dead.status, "DEAD");
		assert_eq!(dead.http_status, Some(500));
	}
}
