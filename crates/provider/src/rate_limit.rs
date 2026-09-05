//! Per-source request limiter built on `governor`'s GCRA quota.

use std::{num::NonZeroU32, sync::Arc, time::Duration};

use governor::{
	clock::DefaultClock,
	state::{InMemoryState, NotKeyed},
	Quota,
};

/// A shareable requests-per-interval limiter. Cloning shares the same quota.
#[derive(Clone)]
pub struct RateLimiter {
	inner: Arc<governor::RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
	requests: u32,
	per: Duration,
}

impl std::fmt::Debug for RateLimiter {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RateLimiter")
			.field("requests", &self.requests)
			.field("per", &self.per)
			.finish()
	}
}

impl RateLimiter {
	/// Allow `requests` within every `per` window (burst equals `requests`).
	///
	/// # Panics
	/// Panics when `requests` is zero or `per` is zero, which would make every
	/// request wait forever.
	pub fn new(requests: u32, per: Duration) -> Self {
		let requests_nz =
			NonZeroU32::new(requests).expect("rate limiter requires at least one request");
		assert!(!per.is_zero(), "rate limiter window must be non-zero");
		let replenish = per / requests;
		let quota = Quota::with_period(replenish.max(Duration::from_nanos(1)))
			.expect("non-zero replenish interval")
			.allow_burst(requests_nz);
		Self {
			inner: Arc::new(governor::RateLimiter::direct(quota)),
			requests,
			per,
		}
	}

	/// `requests` per second.
	pub fn per_second(requests: u32) -> Self {
		Self::new(requests, Duration::from_secs(1))
	}

	/// A limiter that never waits; for tests and mock sources.
	pub fn unlimited() -> Self {
		Self::new(u32::MAX, Duration::from_secs(1))
	}

	/// Wait until a request may proceed.
	pub async fn until_ready(&self) {
		self.inner.until_ready().await;
	}

	/// Non-blocking check that consumes a permit when available.
	pub fn try_acquire(&self) -> bool {
		self.inner.check().is_ok()
	}

	pub fn requests(&self) -> u32 {
		self.requests
	}

	pub fn per(&self) -> Duration {
		self.per
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn burst_is_bounded_by_requests() {
		let limiter = RateLimiter::new(2, Duration::from_secs(60));
		assert!(limiter.try_acquire());
		assert!(limiter.try_acquire());
		assert!(!limiter.try_acquire());
	}

	#[tokio::test]
	async fn until_ready_waits_for_replenish() {
		let limiter = RateLimiter::new(1, Duration::from_millis(40));
		limiter.until_ready().await;
		let started = std::time::Instant::now();
		limiter.until_ready().await;
		assert!(started.elapsed() >= Duration::from_millis(30));
	}

	#[test]
	#[should_panic(expected = "at least one request")]
	fn zero_requests_panics() {
		let _ = RateLimiter::new(0, Duration::from_secs(1));
	}
}
