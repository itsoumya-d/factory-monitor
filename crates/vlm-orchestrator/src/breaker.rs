//! Circuit breaker + call metrics (docs/09 §1): a 429/5xx burst opens the
//! circuit for ≥ 5 min; reuse is only allowed after the probe window.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::error::VlmError;

pub struct CircuitBreaker {
    failure_threshold: u32,
    open_for: Duration,
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(3, Duration::from_secs(5 * 60))
    }
}

impl CircuitBreaker {
    pub const fn new(failure_threshold: u32, open_for: Duration) -> Self {
        Self {
            failure_threshold,
            open_for,
            consecutive_failures: 0,
            open_until: None,
        }
    }

    pub fn is_open(&self, now: Instant) -> bool {
        self.open_until.is_some_and(|until| now < until)
    }

    /// Gate every outbound attempt through here.
    pub fn check(&self, now: Instant) -> Result<(), VlmError> {
        if self.is_open(now) {
            let left = self
                .open_until
                .map_or(Duration::ZERO, |u| u.saturating_duration_since(now));
            return Err(VlmError::CircuitOpen {
                retry_in_secs: left.as_secs().max(1),
            });
        }
        Ok(())
    }

    pub fn record_failure(&mut self, now: Instant) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= self.failure_threshold {
            self.open_until = Some(now + self.open_for);
            tracing::warn!(
                failures = self.consecutive_failures,
                open_secs = self.open_for.as_secs(),
                "vlm circuit opened after failure burst"
            );
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.open_until = None;
    }
}

/// `vlm_calls_total{provider,result}` + `vlm_latency_seconds`-style
/// counters, kept as atomics (Prometheus exposition lands in Phase 6).
#[derive(Default)]
pub struct VlmMetrics {
    calls_total: AtomicU64,
    failures_total: AtomicU64,
    retries_total: AtomicU64,
    latency_ms_sum: AtomicU64,
    latency_calls: AtomicU64,
}

pub struct MetricsSnapshot {
    pub calls_total: u64,
    pub failures_total: u64,
    pub retries_total: u64,
    pub latency_ms_sum: u64,
    pub latency_calls: u64,
    pub latency_ms_avg: u64,
}

impl VlmMetrics {
    pub fn record_call(&self) {
        self.calls_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_failure(&self) {
        self.failures_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_retry(&self) {
        self.retries_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_latency_ms(&self, ms: u64) {
        self.latency_ms_sum.fetch_add(ms, Ordering::Relaxed);
        self.latency_calls.fetch_add(1, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> MetricsSnapshot {
        let calls = self.calls_total.load(Ordering::Relaxed);
        let lat_n = self.latency_calls.load(Ordering::Relaxed);
        let lat_sum = self.latency_ms_sum.load(Ordering::Relaxed);
        MetricsSnapshot {
            calls_total: calls,
            failures_total: self.failures_total.load(Ordering::Relaxed),
            retries_total: self.retries_total.load(Ordering::Relaxed),
            latency_ms_sum: lat_sum,
            latency_calls: lat_n,
            latency_ms_avg: if lat_n == 0 {
                0
            } else {
                lat_sum / lat_n.max(1)
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn three_consecutive_failures_open_the_circuit() {
        let mut b = CircuitBreaker::default();
        let t0 = Instant::now();
        b.record_failure(t0);
        assert!(!b.is_open(t0), "one failure is not a burst");
        b.record_failure(t0);
        assert!(!b.is_open(t0));
        b.record_failure(t0);
        assert!(b.is_open(t0), "third consecutive failure opens");
        let err = b.check(t0).unwrap_err();
        assert!(matches!(err, VlmError::CircuitOpen { retry_in_secs: 300 }));
    }

    #[test]
    fn success_resets_the_failure_streak() {
        let mut b = CircuitBreaker::default();
        let t0 = Instant::now();
        b.record_failure(t0);
        b.record_failure(t0);
        b.record_success();
        b.record_failure(t0);
        b.record_failure(t0);
        assert!(!b.is_open(t0), "streak reset keeps the circuit closed");
    }

    #[test]
    fn circuit_closes_after_the_open_window() {
        let mut b = CircuitBreaker::new(1, Duration::from_secs(300));
        let t0 = Instant::now();
        b.record_failure(t0);
        assert!(b.is_open(t0));
        assert!(
            !b.is_open(t0 + Duration::from_secs(301)),
            "probe window elapsed"
        );
        b.record_success();
        assert!(!b.is_open(t0 + Duration::from_secs(302)));
    }
}
