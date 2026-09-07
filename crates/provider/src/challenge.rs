//! Automatic Cloudflare clearance: queue a browser, apply what it earned.
//!
//! A managed challenge is the one source failure a server cannot fix by
//! itself. It is not an outage and not an authorisation problem — the host is
//! up and will answer any request carrying a `cf_clearance` cookie, and that
//! cookie only exists if something ran the challenge script. Servers do not
//! have browsers. So the fix is a `challenge_solve` job (`stump_worker`) on a
//! machine that does, and this module is the two halves of that round trip:
//! deciding when to ask, and what to do with the answer.
//!
//! The rules, in one place:
//!
//! * **Ask** when a health probe finds an enabled instance still gated *after*
//!   the headers it already carries were tried (`HealthRunSummary::challenged`),
//!   and no solve for that instance is already in flight.
//! * **Adopt** before asking. A solve that was parked in `needs_worker` and
//!   later drained — because the operator finally started the browser box —
//!   completes with nobody waiting on it. Its result is still sitting in
//!   `worker_jobs.result`, so the next ask picks it up instead of paying for a
//!   second browser session six hours later.
//! * **Apply** by merging `Cookie` and `User-Agent` into the instance's own
//!   [`RequestHeaders`](crate::RequestHeaders) — the same store the operator's
//!   `setProviderSourceHeaders` writes, masked by the same rule — rebuilding
//!   the source, and re-probing it. The probe is the verdict: a clearance that
//!   does not actually get through leaves the row where it was.
//! * **Never** run it locally. There is no fallback and there will not be one;
//!   `needs_worker` is the honest answer and the console says so.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use models::entity::provider_source;
use sea_orm::EntityTrait;
use stump_worker::{
	challenge_solve_requires, ChallengeSolveInput, ChallengeSolveOutput, JobOutcome,
	WorkerJobStatus, CHALLENGE_SOLVE,
};

use crate::{
	health::{self, ChallengedInstance, HealthStatus},
	host::{ProviderError, ProviderHost},
	http::RequestHeaders,
};

/// How long a clearance that carries no expiry of its own is trusted.
///
/// Cloudflare issues `cf_clearance` with a lifetime the site operator sets;
/// when the browser reports a session cookie instead, seven days is the
/// horizon Cloudflare's own documented maximum implies. Getting it wrong is
/// cheap in both directions: too short queues a solve a browser answers in
/// seconds, too long is corrected by the next challenged probe.
pub const DEFAULT_CLEARANCE_TTL: TimeDelta = TimeDelta::days(7);

/// How long the task awaiting a solve stays blocked on the worker.
///
/// The worker's own budget is 90 seconds of page-watching; this is that plus
/// room for Chrome to start, so a solve that a human is slow to click still
/// lands rather than being abandoned one second before it succeeds.
pub const SOLVE_TIMEOUT: Duration = Duration::from_secs(300);

/// How many terminal `challenge_solve` rows are searched for a result to
/// adopt. Enabled gated sources are single digits, so the newest solve for any
/// one of them is always well inside this; a miss only costs one more solve.
const ADOPT_SCAN: u64 = 100;

/// What asking for a solve did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChallengeSolveState {
	/// A solve that had already finished was applied without queueing
	/// anything, and `challenged` is what the re-probe then saw.
	Adopted { challenged: bool },
	/// Queued, and a browser worker is connected to take it.
	Queued,
	/// A solve for this instance was already in flight; nothing was queued.
	AlreadyQueued,
	/// Queued and parked: nothing advertises `browser`. The row stays in
	/// `needs_worker` and is offered the moment a browser worker connects.
	NeedsWorker,
}

impl ChallengeSolveState {
	/// What an operator is told.
	#[must_use]
	pub fn message(&self) -> &'static str {
		match self {
			Self::Adopted { challenged: false } => {
				"Applied a clearance an earlier solve had already earned"
			},
			Self::Adopted { challenged: true } => {
				"An earlier clearance was applied but is still refused; solving again"
			},
			Self::Queued => "A browser worker is solving the challenge",
			Self::AlreadyQueued => "A browser worker is already solving this challenge",
			Self::NeedsWorker => {
				"No browser worker is connected. Start `stump-worker --chrome …` and the queued solve runs"
			},
		}
	}
}

impl ProviderHost {
	/// Ask for a clearance for one gated instance.
	///
	/// Returns as soon as routing is decided; the solve itself runs on a
	/// worker and is applied by a task this spawns. An operator's "Solve now"
	/// therefore answers immediately with *what will happen* rather than
	/// holding a request open for the minutes a human may take to tick a
	/// checkbox.
	pub async fn solve_challenge(
		self: &Arc<Self>,
		instance_id: &str,
		dead_after: i32,
	) -> Result<ChallengeSolveState, ProviderError> {
		let row = self.enabled_instance(instance_id).await?;
		let jobs = self.worker_jobs()?;

		if let Some(output) = self.adoptable_result(instance_id).await? {
			let challenged = self
				.apply_clearance(&row, &output, dead_after)
				.await?
				.challenged
				.iter()
				.any(|instance| instance.instance_id == instance_id);
			if !challenged {
				return Ok(ChallengeSolveState::Adopted { challenged });
			}
			tracing::info!(
				instance = instance_id,
				"The stored clearance is refused; queueing a fresh solve"
			);
		}

		if self.claim_solve(instance_id, &jobs).await? {
			return Ok(ChallengeSolveState::AlreadyQueued);
		}

		let url = row.base_url.trim_end_matches('/').to_string();
		let input = ChallengeSolveInput {
			source_instance_id: instance_id.to_string(),
			host: crate::http::host_of(&url),
			url,
		};
		let requires = challenge_solve_requires();
		// Asked before the job exists, because `enqueue_and_wait` reports a
		// parked job and a finished one through the same return value, and
		// the caller has to be answered now.
		let has_worker = jobs.hub().capable_worker(&requires).await.is_some();

		let host = self.clone();
		let payload = serde_json::to_value(&input).unwrap_or_default();
		tokio::spawn(async move {
			let outcome = jobs
				.enqueue_and_wait(
					CHALLENGE_SOLVE,
					payload,
					requires,
					stump_worker::INTERACTIVE_PRIORITY,
					SOLVE_TIMEOUT,
				)
				.await;
			host.finish_solve(&input, outcome, dead_after).await;
			host.release_solve(&input.source_instance_id);
		});

		Ok(if has_worker {
			ChallengeSolveState::Queued
		} else {
			ChallengeSolveState::NeedsWorker
		})
	}

	/// Take the right to solve for `instance_id`, or report that somebody
	/// already has it.
	///
	/// Two guards, because they cover different windows. The in-memory set
	/// covers *this* process between deciding to solve and the row existing —
	/// the enqueue happens on a spawned task, so a double-clicked button would
	/// otherwise queue twice. The `worker_jobs` scan covers everything the set
	/// cannot: a job parked in `needs_worker` since before a restart is still
	/// going to run, and must not be queued again.
	async fn claim_solve(
		&self,
		instance_id: &str,
		jobs: &stump_worker::WorkerJobs,
	) -> Result<bool, ProviderError> {
		if !self
			.solving
			.lock()
			.expect("solve registry poisoned")
			.insert(instance_id.to_string())
		{
			return Ok(true);
		}
		let active = jobs
			.active_of_kind(CHALLENGE_SOLVE)
			.await
			.map_err(worker_error)?
			.into_iter()
			.any(|job| input_instance(&job.input) == Some(instance_id));
		if active {
			self.release_solve(instance_id);
		}
		Ok(active)
	}

	fn release_solve(&self, instance_id: &str) {
		self.solving
			.lock()
			.expect("solve registry poisoned")
			.remove(instance_id);
	}

	/// Queue a solve for every instance a health run found still gated.
	///
	/// Failures are logged rather than propagated: a health run's job is to
	/// record what it saw, and an unreachable worker queue is not a reason to
	/// fail the probe that discovered the problem.
	pub async fn solve_challenges(
		self: &Arc<Self>,
		challenged: &[ChallengedInstance],
		dead_after: i32,
	) {
		for instance in challenged {
			match self
				.solve_challenge(&instance.instance_id, dead_after)
				.await
			{
				Ok(state) => tracing::info!(
					instance = %instance.instance_id,
					host = %instance.host,
					?state,
					"{}",
					state.message()
				),
				Err(error) => tracing::warn!(
					instance = %instance.instance_id,
					%error,
					"Could not queue a challenge solve"
				),
			}
		}
	}

	/// Merge a solved clearance into an instance's request headers, rebuild
	/// it, and re-probe.
	///
	/// The merge keeps whatever else the operator configured (a `Referer`, an
	/// `Accept-Language`) and replaces only the two headers a clearance is:
	/// the cookie, and the user agent it was issued to. Replacing the whole
	/// map — which is what the mutation does — would silently drop the rest.
	pub async fn apply_clearance(
		&self,
		row: &provider_source::Model,
		output: &ChallengeSolveOutput,
		dead_after: i32,
	) -> Result<health::HealthRunSummary, ProviderError> {
		let merged = merge_clearance(
			&RequestHeaders::parse(row.request_headers.as_deref()),
			output,
		)?;
		self.set_source_headers(&row.id, merged).await?;
		tracing::info!(
			instance = %row.id,
			cookies = output.cookies.len(),
			expires_at = ?clearance_expiry(output),
			"Stored a browser-earned Cloudflare clearance"
		);
		self.reprobe_instance(row, dead_after).await
	}

	/// Re-probe one instance's host with the headers it now carries and
	/// refresh every `source_health` row for that host.
	///
	/// Runs the same [`health::probe_target`] a health run does, so a row
	/// written here is indistinguishable from one written by the job — there
	/// is no second definition of what a source's health means.
	pub async fn reprobe_instance(
		&self,
		row: &provider_source::Model,
		dead_after: i32,
	) -> Result<health::HealthRunSummary, ProviderError> {
		let snapshot = self.catalog().snapshot().await?;
		let target = health::target_for(self.conn(), &snapshot, &row.base_url).await?;
		let summary =
			health::probe_target(self.conn(), self.health_checker(), &target, dead_after)
				.await?;
		for change in &summary.changed {
			self.emit(change.as_event());
		}
		Ok(summary)
	}

	/// Apply a finished solve, or explain why nothing happened.
	async fn finish_solve(
		&self,
		input: &ChallengeSolveInput,
		outcome: Result<JobOutcome, stump_worker::WorkerError>,
		dead_after: i32,
	) {
		let result = match outcome {
			Ok(JobOutcome::Done { result, .. }) => result,
			Ok(JobOutcome::NeedsWorker) => {
				// Not an error and not lost: the row is parked, a connecting
				// browser worker is offered it, and whatever it produces is
				// adopted by the next ask.
				tracing::info!(
					host = %input.host,
					"No browser worker is connected; the solve is parked"
				);
				return;
			},
			Ok(JobOutcome::Failed { error }) => {
				tracing::warn!(host = %input.host, %error, "A challenge solve failed");
				return;
			},
			Err(error) => {
				tracing::warn!(host = %input.host, %error, "A challenge solve was not run");
				return;
			},
		};
		let output: ChallengeSolveOutput = match serde_json::from_value(result) {
			Ok(output) => output,
			Err(error) => {
				tracing::warn!(%error, "A challenge solve reported an unusable result");
				return;
			},
		};
		let row = match self.enabled_instance(&input.source_instance_id).await {
			Ok(row) => row,
			Err(error) => {
				// Disabled while the browser was working: the headers would
				// be stored and never used, so say so and drop it.
				tracing::info!(%error, "Discarding a clearance for a source that is gone");
				return;
			},
		};
		if let Err(error) = self.apply_clearance(&row, &output, dead_after).await {
			tracing::error!(%error, instance = %row.id, "Could not apply a clearance");
		}
	}

	/// The newest finished solve for this instance whose clearance the source
	/// is not already carrying and which has not expired, if there is one.
	async fn adoptable_result(
		&self,
		instance_id: &str,
	) -> Result<Option<ChallengeSolveOutput>, ProviderError> {
		let row = self.enabled_instance(instance_id).await?;
		let carried = RequestHeaders::parse(row.request_headers.as_deref());
		let jobs = self.worker_jobs()?;
		let finished = jobs
			.list_of_kind(CHALLENGE_SOLVE, Some(WorkerJobStatus::Done), ADOPT_SCAN)
			.await
			.map_err(worker_error)?;
		let now = Utc::now();
		for job in finished {
			if input_instance(&job.input) != Some(instance_id) {
				continue;
			}
			let Some(result) = job.result else { continue };
			let Ok(output) = serde_json::from_value::<ChallengeSolveOutput>(result)
			else {
				continue;
			};
			if clearance_expiry(&output) <= now {
				// Older solves are older still; nothing here is worth having.
				return Ok(None);
			}
			return Ok((!carries(&carried, &output)).then_some(output));
		}
		Ok(None)
	}

	async fn enabled_instance(
		&self,
		instance_id: &str,
	) -> Result<provider_source::Model, ProviderError> {
		let row = provider_source::Entity::find_by_id(instance_id)
			.one(self.conn())
			.await?
			.filter(|row| row.enabled)
			.ok_or_else(|| ProviderError::UnknownSource(instance_id.to_string()))?;
		Ok(row)
	}
}

/// When the clearance in `output` stops being worth replaying.
#[must_use]
pub fn clearance_expiry(output: &ChallengeSolveOutput) -> DateTime<Utc> {
	output
		.expires_at
		.and_then(|seconds| DateTime::from_timestamp(seconds, 0))
		.unwrap_or_else(|| Utc::now() + DEFAULT_CLEARANCE_TTL)
}

/// The instance a `challenge_solve` job's stored input names.
fn input_instance(input: &serde_json::Value) -> Option<&str> {
	input.get("source_instance_id")?.as_str()
}

/// `Cookie` and `User-Agent` from the solve, everything else from what the
/// operator already configured.
fn merge_clearance(
	existing: &RequestHeaders,
	output: &ChallengeSolveOutput,
) -> Result<RequestHeaders, ProviderError> {
	existing
		.merged_with([
			("Cookie".to_string(), output.cookie_header()),
			("User-Agent".to_string(), output.user_agent.clone()),
		])
		.map_err(|error| ProviderError::Other(error.to_string()))
}

/// Whether the instance already sends exactly this clearance, in which case
/// applying it again would be a rebuild and a probe for nothing.
///
/// Asked as "would applying it change anything", so the comparison never
/// reads a stored credential back out.
fn carries(existing: &RequestHeaders, output: &ChallengeSolveOutput) -> bool {
	merge_clearance(existing, output).is_ok_and(|merged| merged == *existing)
}

fn worker_error(error: stump_worker::WorkerError) -> ProviderError {
	ProviderError::Other(error.to_string())
}

/// The verdict of a re-probe, for a caller that only wants the status.
#[must_use]
pub fn probe_status(summary: &health::HealthRunSummary) -> HealthStatus {
	if summary.dead > 0 {
		HealthStatus::Dead
	} else if summary.degraded > 0 {
		HealthStatus::Degraded
	} else if summary.ok > 0 {
		HealthStatus::Ok
	} else {
		HealthStatus::Unknown
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn output(cookie: &str, ua: &str, expires: Option<i64>) -> ChallengeSolveOutput {
		let mut cookies = std::collections::BTreeMap::new();
		cookies.insert("cf_clearance".to_string(), cookie.to_string());
		cookies.insert("__cf_bm".to_string(), "bm".to_string());
		ChallengeSolveOutput {
			cookies,
			user_agent: ua.to_string(),
			expires_at: expires,
		}
	}

	/// The clearance replaces the two headers it *is* and leaves the rest of
	/// the operator's configuration alone. Dropping a configured `Referer`
	/// because a cookie arrived would break the source the solve was meant to
	/// fix.
	#[test]
	fn merging_replaces_the_clearance_and_keeps_everything_else() {
		let existing = RequestHeaders::from_pairs([
			(
				"Referer".to_string(),
				"https://readcomicsonline.ru/".to_string(),
			),
			("Cookie".to_string(), "cf_clearance=stale".to_string()),
			("User-Agent".to_string(), "Old/1.0".to_string()),
		])
		.expect("valid");

		let merged =
			merge_clearance(&existing, &output("fresh", "Mozilla/5.0 (X11)", None))
				.expect("valid");

		// `to_json` is the stored form and the only place values are visible
		// at all, which makes it the right assertion: this is exactly the row
		// the next build reads.
		assert_eq!(
			merged.to_json().as_deref(),
			Some(
				r#"{"cookie":"__cf_bm=bm; cf_clearance=fresh","referer":"https://readcomicsonline.ru/","user-agent":"Mozilla/5.0 (X11)"}"#
			)
		);
	}

	/// A source already sending exactly this clearance is not rebuilt and not
	/// re-probed: adoption is for results nobody applied, not for every pass.
	#[test]
	fn an_already_carried_clearance_is_not_adopted_again() {
		let solved = output("fresh", "Mozilla/5.0 (X11)", None);
		let applied =
			merge_clearance(&RequestHeaders::default(), &solved).expect("valid");
		assert!(carries(&applied, &solved));

		let newer = output("newer", "Mozilla/5.0 (X11)", None);
		assert!(!carries(&applied, &newer));

		// Same cookie, different browser: the cookie is bound to the agent it
		// was issued to, so this is a different clearance.
		let other_agent = output("fresh", "Mozilla/5.0 (Windows)", None);
		assert!(!carries(&applied, &other_agent));
	}

	/// A cookie with its own expiry is trusted exactly that long; one without
	/// gets the server's horizon, because a session cookie has no moment in
	/// time to compare against.
	#[test]
	fn expiry_comes_from_the_cookie_and_falls_back_to_the_default_ttl() {
		let stamped = output("x", "UA", Some(1_800_000_000));
		assert_eq!(
			clearance_expiry(&stamped),
			DateTime::from_timestamp(1_800_000_000, 0).unwrap()
		);

		let session = output("x", "UA", None);
		let horizon = clearance_expiry(&session);
		let expected = Utc::now() + DEFAULT_CLEARANCE_TTL;
		assert!((horizon - expected).num_seconds().abs() < 5, "{horizon}");

		// An expiry in the past is what makes the next challenged probe queue
		// a fresh solve instead of re-applying a dead cookie.
		let stale = output("x", "UA", Some(1_000_000_000));
		assert!(clearance_expiry(&stale) < Utc::now());
	}

	#[test]
	fn the_instance_is_read_out_of_the_stored_job_input() {
		let input = serde_json::json!({
			"source_instance_id": "en.readcomicsonline",
			"host": "readcomicsonline.ru",
			"url": "https://readcomicsonline.ru"
		});
		assert_eq!(input_instance(&input), Some("en.readcomicsonline"));
		assert_eq!(input_instance(&serde_json::json!({})), None);
	}

	/// The whole server half of a solve, against a host that behaves like a
	/// gated one: everybody gets the interstitial, a request carrying the
	/// clearance gets the page.
	///
	/// Four things have to happen and each is a way this has broken: the
	/// headers are *stored* (the row is the only durable copy), the instance
	/// is *rebuilt* (a built source holds a copy of the row, so an un-rebuilt
	/// one keeps sending the old cookie), the re-probe *carries* them, and the
	/// health row flips.
	#[tokio::test]
	async fn a_solved_clearance_is_stored_rebuilt_and_reprobed() {
		use crate::{
			catalog::INDEX_FILE_NAME,
			host::{ProviderHostConfig, SourceFactory},
			mock_http::{CannedResponse, MockServer},
		};
		use models::entity::source_health;
		use sea_orm::EntityTrait;
		use std::sync::Mutex as StdMutex;

		/// What the factory saw the last time it built the instance.
		static BUILT: StdMutex<Vec<Option<String>>> = StdMutex::new(Vec::new());

		let server = MockServer::spawn(vec![(
			"/",
			CannedResponse::status(403).with_header("cf-mitigated", "challenge"),
		)])
		.await;
		server.set_route_when_header(
			"/",
			"cookie",
			"cf_clearance=fresh",
			CannedResponse::html("<html><body>comics</body></html>"),
		);

		let conn = Arc::new(::tests::db::test_database().await);
		let dir = tempfile::tempdir().expect("tempdir");
		// An on-disk catalog index, so the re-probe writes the same
		// `source_health` row a health run would rather than a special one.
		std::fs::write(
			dir.path().join(INDEX_FILE_NAME),
			serde_json::to_vec(&serde_json::json!([{
				"name": "Gated",
				"pkg": "eu.kanade.tachiyomi.extension.en.gated",
				"lang": "en",
				"version": "1.0.0",
				"nsfw": 0,
				"sources": [{
					"id": "1001",
					"name": "Gated",
					"lang": "en",
					"baseUrl": server.base_url(),
				}],
			}]))
			.expect("index"),
		)
		.expect("write index");

		let host = ProviderHost::open(
			conn.clone(),
			vec![SourceFactory {
				implementation: "gated",
				name: "Gated",
				catalog_pkg: "eu.kanade.tachiyomi.extension.en.gated",
				base_url: "http://unused.test",
				build: |row| {
					BUILT
						.lock()
						.expect("built")
						.push(row.request_headers.clone());
					Ok(crate::mock::MockSource::with_id(&row.id))
				},
			}],
			Vec::new(),
			ProviderHostConfig {
				cache_dir: dir.path().to_path_buf(),
				cache_max_bytes: u64::MAX,
				// Never reachable: the index on disk is the only source of
				// catalog truth in this test.
				catalog_url: Some("http://127.0.0.1:9/".to_string()),
				definitions_url: Some("http://127.0.0.1:9/".to_string()),
				virtual_series_ttl: Duration::from_secs(300),
			},
		)
		.await
		.expect("host");
		let row = host
			.enable_catalog_source("1001", None)
			.await
			.expect("enable");
		assert_eq!(row.base_url, server.base_url());

		// Before: the host is gated, and the instance is named as the one
		// thing a browser has to be asked about.
		let snapshot = host.catalog().snapshot().await.expect("snapshot");
		let target = health::target_for(conn.as_ref(), &snapshot, &row.base_url)
			.await
			.expect("target");
		let before =
			health::probe_target(conn.as_ref(), host.health_checker(), &target, 3)
				.await
				.expect("probe");
		assert_eq!(probe_status(&before), HealthStatus::Degraded);
		assert_eq!(
			before
				.challenged
				.iter()
				.map(|instance| instance.instance_id.as_str())
				.collect::<Vec<_>>(),
			vec![row.id.as_str()]
		);

		let solved = output("fresh", "Mozilla/5.0 (X11; Linux x86_64)", None);
		let after = host.apply_clearance(&row, &solved, 3).await.expect("apply");

		// Stored, as the row the next boot reads.
		let stored = provider_source::Entity::find_by_id(row.id.as_str())
			.one(conn.as_ref())
			.await
			.expect("query")
			.expect("row");
		assert_eq!(
			stored.request_headers.as_deref(),
			Some(
				r#"{"cookie":"__cf_bm=bm; cf_clearance=fresh","user-agent":"Mozilla/5.0 (X11; Linux x86_64)"}"#
			)
		);
		// Rebuilt, and rebuilt *with* those headers.
		assert_eq!(
			BUILT.lock().expect("built").last(),
			Some(&stored.request_headers)
		);
		// Re-probed, carrying them: the gated host only answered because the
		// request had the clearance on it.
		let authenticated = server
			.requests()
			.into_iter()
			.filter(|request| {
				request.headers.iter().any(|(name, value)| {
					name == "cookie" && value.contains("cf_clearance=fresh")
				})
			})
			.collect::<Vec<_>>();
		assert!(!authenticated.is_empty(), "the re-probe sent no clearance");
		assert!(
			authenticated.iter().all(|request| request
				.headers
				.iter()
				.any(|(name, value)| name == "user-agent"
					&& value == "Mozilla/5.0 (X11; Linux x86_64)")),
			"the clearance was replayed without the agent it was issued to"
		);
		// And the row flipped, with nothing left behind it.
		assert_eq!(probe_status(&after), HealthStatus::Ok);
		assert!(after.challenged.is_empty());
		let health_row = source_health::Entity::find_by_id("1001")
			.one(conn.as_ref())
			.await
			.expect("query")
			.expect("health row");
		assert_eq!(health_row.status, "OK");
		assert_eq!(health_row.error, None);
	}

	/// A solve is asked for once. The second ask, before the first job even
	/// has a row, is refused — a double-clicked "Solve now" must not open two
	/// browser windows — and with nobody advertising `browser` the first one
	/// parks rather than failing.
	#[tokio::test]
	async fn a_second_ask_is_refused_while_one_is_in_flight() {
		use crate::{
			host::{ProviderHostConfig, SourceFactory},
			mock_http::{CannedResponse, MockServer},
		};

		let server = MockServer::spawn(vec![(
			"/",
			CannedResponse::status(403).with_header("cf-mitigated", "challenge"),
		)])
		.await;
		let conn = Arc::new(::tests::db::test_database().await);
		let dir = tempfile::tempdir().expect("tempdir");
		let host = ProviderHost::open(
			conn.clone(),
			vec![SourceFactory {
				implementation: "gated",
				name: "Gated",
				catalog_pkg: "eu.kanade.tachiyomi.extension.en.gated",
				base_url: "http://unused.test",
				build: |row| Ok(crate::mock::MockSource::with_id(&row.id)),
			}],
			Vec::new(),
			ProviderHostConfig {
				cache_dir: dir.path().to_path_buf(),
				cache_max_bytes: u64::MAX,
				catalog_url: Some("http://127.0.0.1:9/".to_string()),
				definitions_url: Some("http://127.0.0.1:9/".to_string()),
				virtual_series_ttl: Duration::from_secs(300),
			},
		)
		.await
		.expect("host");
		host.enable_implementation("gated", "en", None)
			.await
			.expect("enable");
		// The factory's base URL is a placeholder; point the row at the mock.
		let mut active: provider_source::ActiveModel =
			host.enabled_instance("gated-en").await.expect("row").into();
		active.base_url = sea_orm::ActiveValue::Set(server.base_url().to_string());
		sea_orm::ActiveModelTrait::update(active, conn.as_ref())
			.await
			.expect("point at the mock");

		// No queue installed at all is a verdict, not an outage.
		assert!(matches!(
			host.solve_challenge("gated-en", 3).await,
			Err(ProviderError::NoWorkerQueue)
		));

		host.set_worker_jobs(Arc::new(stump_worker::WorkerJobs::new(
			conn.clone(),
			dir.path().join("worker-output"),
		)));

		assert_eq!(
			host.solve_challenge("gated-en", 3).await.expect("solve"),
			ChallengeSolveState::NeedsWorker
		);
		assert_eq!(
			host.solve_challenge("gated-en", 3).await.expect("solve"),
			ChallengeSolveState::AlreadyQueued
		);

		// A disabled source is not something to open a browser for.
		host.disable_source("gated-en").await.expect("disable");
		assert!(matches!(
			host.solve_challenge("gated-en", 3).await,
			Err(ProviderError::UnknownSource(id)) if id == "gated-en"
		));
	}
}
