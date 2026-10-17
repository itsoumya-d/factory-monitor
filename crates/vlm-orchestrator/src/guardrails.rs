//! Rate & cost guardrails (docs/09 §6): daily USD cap with 00:00 reset,
//! soft-mode routing, exponential retry backoff 1s/4s/16s.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};

use crate::error::VlmError;

/// Cost estimate from wire sizes. Rates roughly mirror hosted-VLM pricing
/// (~$3 / 1 MB of image-bearing prompt input is the observed band); they
/// only need to be honest enough to trip a spend cap.
pub const USD_PER_INPUT_MB: f64 = 3.0;
pub const USD_PER_OUTPUT_MB: f64 = 6.0;

pub const DEFAULT_DAILY_USD_CAP: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    MorningDiff,
    Eod,
    AutoLabel,
}

impl OpKind {
    /// Soft mode keeps morning diffing and EOD running; auto-labeling is
    /// queued for next-boot budget (docs/09 §6).
    pub const fn allowed_in_soft_mode(self) -> bool {
        !matches!(self, OpKind::AutoLabel)
    }
}

pub struct DailyBudget {
    cap_usd: f64,
    used_usd: f64,
    day: Option<NaiveDate>,
}

impl DailyBudget {
    pub fn new(cap_usd: f64) -> Self {
        Self {
            cap_usd,
            used_usd: 0.0,
            day: None,
        }
    }

    /// USD estimate for a call with the given wire sizes.
    pub fn estimate_cost_usd(input_bytes: usize, output_bytes: usize) -> f64 {
        bytes_to_usd(input_bytes, USD_PER_INPUT_MB) + bytes_to_usd(output_bytes, USD_PER_OUTPUT_MB)
    }

    /// Charge a completed call. A new UTC factory-day resets the counter
    /// first (00:00 boundary, docs/09 §6).
    pub fn charge(&mut self, cost_usd: f64, now: DateTime<Utc>) {
        let today = now.naive_utc().date();
        if self.day != Some(today) {
            self.reset(today);
        }
        self.used_usd += cost_usd;
    }

    /// `true` when the cap was reached before `charge` on this day; calls
    /// are checked against this BEFORE dialing out.
    pub fn is_over_cap(&self, now: DateTime<Utc>) -> bool {
        self.day_for_check(now) >= self.cap_usd
    }

    pub fn used_usd(&self) -> f64 {
        self.used_usd
    }

    pub fn cap_usd(&self) -> f64 {
        self.cap_usd
    }

    fn day_for_check(&self, now: DateTime<Utc>) -> f64 {
        let today = now.naive_utc().date();
        if self.day != Some(today) {
            0.0
        } else {
            self.used_usd
        }
    }

    fn reset(&mut self, today: NaiveDate) {
        self.day = Some(today);
        self.used_usd = 0.0;
    }
}

fn bytes_to_usd(bytes: usize, usd_per_mb: f64) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0)) * usd_per_mb
}

/// 1 ÷ 4 ÷ 16 seconds exponential backoff (docs/09 §1). Attempts are
/// 0-indexed; anything beyond the third attempt stops growing.
pub fn retry_backoff(attempt: u32) -> Duration {
    let secs = 1u64 << (attempt * 2).min(4);
    Duration::from_secs(secs)
}

/// Pre-flight gate shared by the orchestrator's three operations.
pub fn preflight(op: OpKind, budget: &DailyBudget, now: DateTime<Utc>) -> Result<(), VlmError> {
    let over = budget.is_over_cap(now);
    if over && !op.allowed_in_soft_mode() {
        return Err(VlmError::BudgetExhausted);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn at_ymd(y: i32, m: u32, d: u32, hh: u32) -> DateTime<Utc> {
        match chrono::TimeZone::with_ymd_and_hms(&Utc, y, m, d, hh, 0, 0) {
            chrono::LocalResult::Single(ts) => ts,
            _ => Utc::now(),
        }
    }

    #[test]
    fn soft_mode_blocks_only_auto_label() {
        assert!(OpKind::MorningDiff.allowed_in_soft_mode());
        assert!(OpKind::Eod.allowed_in_soft_mode());
        assert!(!OpKind::AutoLabel.allowed_in_soft_mode());
    }

    #[test]
    fn budget_resets_at_day_boundary_and_blocks_when_over() {
        let mut budget = DailyBudget::new(1.0);
        let day1 = at_ymd(2026, 4, 7, 9);
        budget.charge(0.9, day1);
        assert!(!budget.is_over_cap(day1), "0.9 < cap 1.0");
        budget.charge(0.2, day1);
        assert!(budget.is_over_cap(day1));

        let day2 = at_ymd(2026, 4, 8, 0);
        assert!(!budget.is_over_cap(day2), "fresh day, zero usage");
        budget.charge(0.1, day2);
        assert!(budget.used_usd() < 1.0, "counter restarted");
    }

    #[test]
    fn preflight_puts_auto_label_to_queue_when_over_cap() {
        let mut budget = DailyBudget::new(0.10);
        let now = at_ymd(2026, 4, 7, 14);
        budget.charge(1.0, now);
        assert!(
            matches!(
                preflight(OpKind::AutoLabel, &budget, now).unwrap_err(),
                VlmError::BudgetExhausted
            ),
            "auto-label waits for budget"
        );
        preflight(OpKind::MorningDiff, &budget, now)
            .expect("morning diffing keeps the lights on in soft mode");
        preflight(OpKind::Eod, &budget, now).expect("EOD keeps running in soft mode");
    }

    #[test]
    fn backoff_sequence_is_1_4_16() {
        assert_eq!(retry_backoff(0), Duration::from_secs(1));
        assert_eq!(retry_backoff(1), Duration::from_secs(4));
        assert_eq!(retry_backoff(2), Duration::from_secs(16));
        assert_eq!(retry_backoff(9), Duration::from_secs(16), "saturating");
    }

    #[test]
    fn cost_estimate_is_monotone_in_both_sizes() {
        let a = DailyBudget::estimate_cost_usd(1024, 1024);
        let b = DailyBudget::estimate_cost_usd(1024, 4096);
        let c = DailyBudget::estimate_cost_usd(4096, 1024);
        assert!(b > a && c > a);
        assert!(a > 0.0);
    }
}
