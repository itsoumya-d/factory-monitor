//! Decoder-watchdog reconnect arithmetic (docs/04 §3).
//!
//! Backoff `2^n x 1 s` capped at 60 s; deterministic (no jitter) so recovery
//! timing is reproducible in the burn-in rig (docs/14 §4).

use std::time::Duration;

pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Attempt numbering is 1-based; `delay(1) == 1 s`.
#[must_use]
pub fn backoff_delay(attempt: u32) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let exp = attempt.saturating_sub(1).min(31);
    let secs = 1u64 << exp;
    Duration::from_secs(secs.min(MAX_BACKOFF.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubling_until_cap() {
        assert_eq!(backoff_delay(0), Duration::ZERO);
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        assert_eq!(backoff_delay(3), Duration::from_secs(4));
        assert_eq!(backoff_delay(5), Duration::from_secs(16));
        assert_eq!(backoff_delay(6), Duration::from_secs(32));
        assert_eq!(backoff_delay(7), MAX_BACKOFF);
        assert_eq!(backoff_delay(13), MAX_BACKOFF);
        assert_eq!(backoff_delay(u32::MAX), MAX_BACKOFF);
    }
}
