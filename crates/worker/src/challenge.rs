//! The `challenge_solve` runner: earn a Cloudflare clearance in a real,
//! visible browser.
//!
//! Everything about this runner is the opposite of the usual scraper advice,
//! on purpose. It launches Chrome **headed**, against a **persistent profile**,
//! with the automation flags Chrome would normally announce itself with turned
//! off — because the thing on the other side is a bot detector, and a headless
//! browser with `--enable-automation` and a throwaway profile is the exact
//! shape it is looking for. A managed challenge that a real browser clears in
//! two seconds can take a headless one forever, or never.
//!
//! The second reason to be headed is the interactive one: some challenges end
//! in a Turnstile checkbox that only a human can tick. The window is on the
//! operator's screen for the whole 90-second budget, so if Cloudflare asks for
//! a click, the operator gives it one and the job succeeds. There is no
//! fallback for that and there cannot be: a server has no browser and no
//! human, which is why this is a worker job at all.
//!
//! The profile is the worker's own (`<data dir>/chrome`), never the operator's
//! real one: a persistent profile is what makes the *second* solve instant
//! (Cloudflare remembers the browser), and pointing this at a live Chrome
//! profile would both fail — Chrome refuses a `--user-data-dir` another
//! process holds — and put the operator's cookies in reach of a source.
//!
//! Behind the default-off `browser` feature: this is the one capability whose
//! dependency is heavyweight, and an operator who only wants Opus should not
//! compile a CDP stack to get it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::network::Cookie;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::client::{Assignment, JobRunner, Progress};
use crate::kind::{
	ChallengeSolveInput, ChallengeSolveOutput, BROWSER, CHALLENGE_SOLVE, CLEARANCE_COOKIE,
};

/// How long a solve may take before the job fails.
///
/// Long enough that a human who is not watching the screen can notice the
/// window, read the checkbox and click it. An unattended solve of a plain
/// managed challenge finishes in two or three seconds and never comes near
/// this. Overridable with `--solve-budget`.
pub const SOLVE_BUDGET: Duration = Duration::from_secs(120);

/// How long the interstitial has to persist before the log says out loud that
/// somebody has to click it. A challenge that clears itself is done well
/// inside this.
const CLICK_PROMPT_AFTER: Duration = Duration::from_secs(5);

/// How often the page is asked whether it is still the interstitial.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The Chrome flags this runner launches with, replacing chromiumoxide's
/// puppeteer-derived defaults.
///
/// The defaults are dropped rather than extended because two of them are
/// exactly what has to go: `--enable-automation` (which sets
/// `navigator.webdriver`, shows the "controlled by automated test software"
/// infobar, and is read by every bot detector) and the background-networking
/// and sync suppressions that make the browser look nothing like a browser.
/// What is left is what a desktop Chrome launched by a person also has,
/// minus the first-run interruptions nobody is there to dismiss.
const CHROME_ARGS: [&str; 6] = [
	"--no-first-run",
	"--no-default-browser-check",
	"--disable-session-crashed-bubble",
	// Its own window rather than a tab folded into whatever Chrome the
	// profile last had open: a solve nobody can see is a solve nobody clicks.
	"--new-window",
	// A Linux desktop Chrome would otherwise try to unlock a keyring, which
	// on a worker box is either absent or a modal dialog nobody answers.
	"--password-store=basic",
	"--use-mock-keychain",
];

/// What the page reported the last time it was asked.
#[derive(Debug, Clone, Deserialize)]
struct PageState {
	title: String,
	ready: String,
	/// The interstitial marker that matched, for the log line.
	marker: Option<String>,
	challenge: bool,
}

/// The expression polled against the page.
///
/// Returns a JSON *string* rather than an object: `Runtime.evaluate` only
/// returns structured values by value when asked to, and a string round-trips
/// through every path unchanged.
const PAGE_STATE_JS: &str = r#"
(function () {
	var markers = [
		'#challenge-running',
		'#challenge-form',
		'#challenge-stage',
		'#cf-challenge-running',
		'.cf-browser-verification',
		'[id^="cf-chl"]',
		'[class^="cf-chl"]'
	];
	var found = null;
	for (var i = 0; i < markers.length; i++) {
		try {
			if (document.querySelector(markers[i])) { found = markers[i]; break; }
		} catch (e) { /* a selector Blink dislikes is not a marker */ }
	}
	var title = document.title || '';
	var interstitial = /just a moment|checking your browser|attention required/i;
	return JSON.stringify({
		title: title,
		ready: document.readyState,
		marker: found,
		challenge: found !== null || interstitial.test(title)
	});
})()
"#;

/// Runs `challenge_solve` jobs by driving a headed Chrome.
pub struct ChallengeRunner {
	chrome: PathBuf,
	profile: PathBuf,
	/// `Google Chrome 152.0.7977.82`, as the binary reports itself. Advertised
	/// so the console's Workers page names the browser the clearance will be
	/// issued to.
	version: String,
	budget: Duration,
}

impl ChallengeRunner {
	/// Check that `chrome` runs and prepare the profile directory.
	///
	/// Refusing to start when the binary does not answer is the same rule the
	/// transcode runner follows: a worker that advertises `browser` and then
	/// fails every job keeps the server routing to it instead of parking the
	/// job in `needs_worker`, which is at least a state the console explains.
	pub fn probe(chrome: &Path, profile: &Path) -> Result<Self, String> {
		let output = std::process::Command::new(chrome)
			.arg("--version")
			.output()
			.map_err(|error| format!("{} did not run: {error}", chrome.display()))?;
		if !output.status.success() {
			return Err(format!(
				"{} --version exited with {}",
				chrome.display(),
				output.status
			));
		}
		let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
		if version.is_empty() {
			return Err(format!("{} reported no version", chrome.display()));
		}
		std::fs::create_dir_all(profile).map_err(|error| {
			format!(
				"Cannot create the browser profile at {}: {error}",
				profile.display()
			)
		})?;
		Ok(Self {
			chrome: chrome.to_path_buf(),
			profile: profile.to_path_buf(),
			version,
			budget: SOLVE_BUDGET,
		})
	}

	/// Override the solve budget (`--solve-budget`). A shorter one is what a
	/// test wants; an unattended deployment wants every second of [`SOLVE_BUDGET`].
	#[must_use]
	pub fn with_budget(mut self, budget: Duration) -> Self {
		self.budget = budget;
		self
	}

	pub fn version(&self) -> &str {
		&self.version
	}

	pub fn profile(&self) -> &Path {
		&self.profile
	}

	/// Drive one URL through its challenge and report the jar.
	async fn solve(
		&self,
		input: &ChallengeSolveInput,
		progress: &Progress,
	) -> Result<ChallengeSolveOutput, String> {
		let config = BrowserConfig::builder()
			.chrome_executable(&self.chrome)
			.user_data_dir(&self.profile)
			.with_head()
			// No `Emulation.setDeviceMetricsOverride`: an emulated viewport is
			// both a fingerprint and a lie about a window a human is looking
			// at. `hide` also drops `navigator.webdriver`.
			.hide()
			.window_size(1100, 900)
			.disable_default_args()
			.args(CHROME_ARGS)
			.launch_timeout(Duration::from_secs(30))
			.build()?;

		let (mut browser, mut handler) = Browser::launch(config)
			.await
			.map_err(|error| format!("Chrome did not start: {error}"))?;
		// The handler pumps the CDP socket; without a task driving it no
		// command ever completes.
		let pump = tokio::spawn(async move { while handler.next().await.is_some() {} });

		let outcome = self.drive(&browser, input, progress).await;

		// Closing is best-effort on both counts: the job's value is the jar it
		// already read, and a browser that refuses to shut down cleanly is a
		// log line, not a failed solve.
		if let Err(error) = browser.close().await {
			tracing::warn!(%error, "Chrome did not close cleanly");
		}
		let _ = browser.wait().await;
		pump.abort();
		outcome
	}

	async fn drive(
		&self,
		browser: &Browser,
		input: &ChallengeSolveInput,
		progress: &Progress,
	) -> Result<ChallengeSolveOutput, String> {
		let page = browser
			.new_page(input.url.as_str())
			.await
			.map_err(|error| format!("Could not open {}: {error}", input.url))?;
		// The whole design assumes a human can see this window; behind a
		// terminal it is a 120-second wait for a click nobody knew to make.
		if let Err(error) = page.activate().await {
			tracing::warn!(%error, "Could not bring the browser window to the front");
		}

		let started = Instant::now();
		let mut last = None;
		let mut prompted = false;
		let cleared = loop {
			let state = page_state(&page).await?;
			let elapsed = started.elapsed();
			if !state.challenge && state.ready != "loading" {
				break true;
			}
			if elapsed >= self.budget {
				break false;
			}
			if last.as_ref() != Some(&state.title) {
				tracing::info!(
					title = %state.title,
					marker = ?state.marker,
					"Cloudflare challenge in progress"
				);
				last = Some(state.title.clone());
			}
			// A challenge that clears itself does so in two or three seconds.
			// One still up after five is waiting for a person, and saying so
			// once — loudly — is the difference between a solve and a
			// timeout.
			if !prompted && elapsed >= CLICK_PROMPT_AFTER {
				tracing::warn!(
					host = %input.host,
					seconds = self.budget.as_secs(),
					"The challenge is waiting for a click: tick the checkbox in the Chrome window on screen"
				);
				prompted = true;
			}
			progress.report(
				elapsed.as_secs_f64() / self.budget.as_secs_f64(),
				if prompted {
					format!("Waiting for a click on {}", input.host)
				} else {
					format!("Solving the challenge on {}", input.host)
				},
			);
			tokio::time::sleep(POLL_INTERVAL).await;
		};

		let cookies = page
			.get_cookies()
			.await
			.map_err(|error| format!("Could not read cookies: {error}"))?;
		let user_agent: String = page
			.evaluate_expression("navigator.userAgent")
			.await
			.map_err(|error| format!("Could not read the user agent: {error}"))?
			.into_value()
			.map_err(|error| format!("The user agent was not a string: {error}"))?;
		let _ = page.close().await;

		let output = collect(&cookies, input, user_agent);
		if !output.has_clearance() {
			// The distinction matters to an operator: a budget that ran out
			// means nobody clicked, while a cleared page with no clearance
			// means this host does not gate with one and the source's problem
			// is something else.
			return Err(if cleared {
				format!(
					"{} stopped showing the challenge but issued no `{CLEARANCE_COOKIE}` cookie",
					input.host
				)
			} else {
				format!(
					"The challenge on {} was still up after {}s",
					input.host,
					self.budget.as_secs()
				)
			});
		}
		Ok(output)
	}
}

/// Ask the page what it is showing.
async fn page_state(page: &chromiumoxide::Page) -> Result<PageState, String> {
	let raw: String = page
		.evaluate_expression(PAGE_STATE_JS)
		.await
		.map_err(|error| format!("Could not inspect the page: {error}"))?
		.into_value()
		.map_err(|error| format!("The page state was not a string: {error}"))?;
	serde_json::from_str(&raw)
		.map_err(|error| format!("The page state was not the expected JSON: {error}"))
}

/// Cookies Cloudflare uses to track the *progress* of a challenge rather than
/// its outcome: `cf_chl_rc_ni` counts non-interactive retries, `cf_chl_*` and
/// `_cf_chl_*` carry challenge state.
const CHALLENGE_STATE_PREFIXES: [&str; 2] = ["cf_chl", "_cf_chl"];

/// Fold the browser's cookie list into the job's output, keeping only the
/// cookies the requested host would actually send *and* that mean something
/// to a later request.
///
/// Challenge-state cookies are dropped. They describe a challenge that is over
/// — replaying "this client has retried eight times" to the edge is at best
/// noise on every future request and at worst an invitation to re-challenge —
/// and they would sit in the operator's stored `Cookie` header forever.
fn collect(
	cookies: &[Cookie],
	input: &ChallengeSolveInput,
	user_agent: String,
) -> ChallengeSolveOutput {
	let mut output = ChallengeSolveOutput {
		user_agent,
		..ChallengeSolveOutput::default()
	};
	for cookie in cookies {
		if !domain_matches(&input.host, &cookie.domain) {
			continue;
		}
		if cookie.name != CLEARANCE_COOKIE
			&& CHALLENGE_STATE_PREFIXES
				.iter()
				.any(|prefix| cookie.name.starts_with(prefix))
		{
			continue;
		}
		if cookie.name == CLEARANCE_COOKIE {
			output.expires_at = cookie_expiry(cookie.expires);
		}
		output
			.cookies
			.insert(cookie.name.clone(), cookie.value.clone());
	}
	output
}

/// Whether a cookie set for `domain` is sent to `host`. Cookie domains are
/// written both with and without the leading dot, and a host cookie for
/// `x.example` is also sent to `www.x.example`.
fn domain_matches(host: &str, domain: &str) -> bool {
	let domain = domain.trim_start_matches('.');
	if domain.is_empty() {
		return false;
	}
	host.eq_ignore_ascii_case(domain)
		|| host
			.to_ascii_lowercase()
			.ends_with(&format!(".{}", domain.to_ascii_lowercase()))
}

/// CDP reports a session cookie's expiry as `-1`, and occasionally as `0`.
/// Neither is a moment in time, and the server has its own horizon for a
/// clearance that does not carry one.
fn cookie_expiry(expires: f64) -> Option<i64> {
	(expires.is_finite() && expires > 0.0).then(|| expires as i64)
}

#[async_trait::async_trait]
impl JobRunner for ChallengeRunner {
	fn capabilities(&self) -> Value {
		serde_json::json!({
			BROWSER: {
				"chrome": self.version,
				"headless": false,
			}
		})
	}

	async fn run(
		&self,
		assignment: Assignment,
		progress: Progress,
	) -> Result<Value, String> {
		if assignment.kind != CHALLENGE_SOLVE {
			return Err(format!("`{}` is not a browser job", assignment.kind));
		}
		let input: ChallengeSolveInput = serde_json::from_value(assignment.input)
			.map_err(|error| format!("Unusable challenge_solve input: {error}"))?;
		tracing::info!(
			host = %input.host,
			url = %input.url,
			instance = %input.source_instance_id,
			"Opening a browser window to solve a Cloudflare challenge"
		);
		progress.report(0.0, format!("Opening {}", input.url));
		let output = self.solve(&input, &progress).await?;
		tracing::info!(
			host = %input.host,
			cookies = output.cookies.len(),
			expires_at = ?output.expires_at,
			"Earned a Cloudflare clearance"
		);
		serde_json::to_value(output)
			.map_err(|error| format!("Could not encode the solve result: {error}"))
	}
}

/// The runner as a `JobRunner`, for the binary's composite.
impl From<ChallengeRunner> for Arc<dyn JobRunner> {
	fn from(runner: ChallengeRunner) -> Self {
		Arc::new(runner)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cookie(name: &str, value: &str, domain: &str, expires: f64) -> Cookie {
		Cookie {
			name: name.to_string(),
			value: value.to_string(),
			domain: domain.to_string(),
			path: "/".to_string(),
			expires,
			size: 0,
			http_only: true,
			secure: true,
			session: expires <= 0.0,
			same_site: None,
			priority:
				chromiumoxide::cdp::browser_protocol::network::CookiePriority::Medium,
			source_scheme:
				chromiumoxide::cdp::browser_protocol::network::CookieSourceScheme::Secure,
			source_port: 443,
			partition_key: None,
			partition_key_opaque: None,
		}
	}

	fn input() -> ChallengeSolveInput {
		ChallengeSolveInput {
			source_instance_id: "en.readcomicsonline".to_string(),
			host: "readcomicsonline.ru".to_string(),
			url: "https://readcomicsonline.ru/".to_string(),
		}
	}

	/// The jar the server stores is only the requested host's, and the expiry
	/// it tracks is the clearance's own — not some other cookie's.
	#[test]
	fn only_the_hosts_cookies_are_collected_with_the_clearance_expiry() {
		let cookies = [
			cookie(
				CLEARANCE_COOKIE,
				"abc",
				".readcomicsonline.ru",
				1_800_000_000.0,
			),
			cookie("__cf_bm", "bm", "readcomicsonline.ru", 1_700_000_000.0),
			cookie("tracker", "nope", ".doubleclick.net", 1_900_000_000.0),
		];
		let output = collect(&cookies, &input(), "Mozilla/5.0".to_string());

		assert_eq!(output.cookies.len(), 2);
		assert_eq!(output.cookies.get(CLEARANCE_COOKIE).unwrap(), "abc");
		assert!(!output.cookies.contains_key("tracker"));
		assert_eq!(output.expires_at, Some(1_800_000_000));
		assert!(output.has_clearance());
		// Sorted, so the header two solves of the same jar produce is the same
		// string.
		assert_eq!(output.cookie_header(), "__cf_bm=bm; cf_clearance=abc");
	}

	/// Cookies that describe a finished challenge's *progress* are dropped.
	/// A live solve of readcomicsonline.ru handed back `cf_chl_rc_ni=8`
	/// alongside the clearance; storing that in an operator's `Cookie` header
	/// would replay "I have retried eight times" on every future request.
	#[test]
	fn challenge_state_cookies_are_not_part_of_a_clearance() {
		let cookies = [
			cookie(CLEARANCE_COOKIE, "abc", "readcomicsonline.ru", -1.0),
			cookie("cf_chl_rc_ni", "8", "readcomicsonline.ru", -1.0),
			cookie("_cf_chl_opt", "x", "readcomicsonline.ru", -1.0),
			cookie("__cf_bm", "bm", "readcomicsonline.ru", -1.0),
		];
		let output = collect(&cookies, &input(), "Mozilla/5.0".to_string());
		assert_eq!(output.cookie_header(), "__cf_bm=bm; cf_clearance=abc");
	}

	/// A session clearance has no moment in time to report; the server, not
	/// the worker, decides how long to trust one.
	#[test]
	fn a_session_clearance_reports_no_expiry() {
		let cookies = [cookie(CLEARANCE_COOKIE, "abc", "readcomicsonline.ru", -1.0)];
		let output = collect(&cookies, &input(), "Mozilla/5.0".to_string());
		assert_eq!(output.expires_at, None);
		assert!(output.has_clearance());
	}

	#[test]
	fn cookie_domains_match_with_or_without_the_leading_dot() {
		assert!(domain_matches(
			"readcomicsonline.ru",
			".readcomicsonline.ru"
		));
		assert!(domain_matches("readcomicsonline.ru", "readcomicsonline.ru"));
		assert!(domain_matches(
			"www.readcomicsonline.ru",
			".readcomicsonline.ru"
		));
		// A suffix is not a domain match: `evilreadcomicsonline.ru` is a
		// different site.
		assert!(!domain_matches(
			"evilreadcomicsonline.ru",
			"readcomicsonline.ru"
		));
		assert!(!domain_matches("readcomicsonline.ru", "other.ru"));
		assert!(!domain_matches("readcomicsonline.ru", ""));
	}

	/// The interstitial is recognised by title *or* by markup: Cloudflare
	/// changes one without the other, and a solve that stopped a second early
	/// hands back a cookie that is refused on the next request.
	#[test]
	fn the_page_state_shape_is_what_the_poll_reads() {
		let state: PageState = serde_json::from_str(
			r##"{"title":"Just a moment...","ready":"interactive","marker":"#challenge-running","challenge":true}"##,
		)
		.expect("page state");
		assert!(state.challenge);
		assert_eq!(state.marker.as_deref(), Some("#challenge-running"));

		let cleared: PageState = serde_json::from_str(
			r#"{"title":"Read Comics Online","ready":"complete","marker":null,"challenge":false}"#,
		)
		.expect("page state");
		assert!(!cleared.challenge);
		assert!(cleared.marker.is_none());
	}
}
