//! The single reward-shaping contract (docs/08 §4 + docs/10 §1): every
//! feed of a human correction into any learner (rl-controller policy,
//! edge-core bandit, api-server persistence) goes through
//! `correction_reward`.
//!
//! Semantics: reward penalizes the MAGNITUDE of the counting error
//! (docs/01's "counted 1,485, actually 1,420" must be a penalty, not a
//! bonus). Directional signals (over- vs under-count) reach the policy
//! through `failure_tags` and the err-momentum features of
//! `FactoryEnvState`, so the scalar stays sign-consistent negative for
//! every wrong count.

/// Denominator docs/10 §1 (matches the api-server's undercount shaping,
/// which the hermetic suite pins at −1.5 for a 150-under).
pub const REWARD_SCALE: f32 = 100.0;

/// Immediate-route shaping weight (docs/08 §4: 0.1 of final reward).
pub const IMMEDIATE_WEIGHT: f32 = 0.1;

pub fn correction_reward(reported: i64, actual: i64) -> f32 {
    -((actual - reported).unsigned_abs() as f32) / REWARD_SCALE
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_documented_example() {
        // docs/01: "Line 4 counted 1,485, actually 1,420" is a penalty.
        assert!((correction_reward(1485, 1420) + 0.65).abs() < 1e-6);
    }

    #[test]
    fn perfect_count_scores_zero() {
        assert_eq!(correction_reward(1200, 1200), 0.0);
    }

    #[test]
    fn under_and_over_count_penalize_equal_magnitudes() {
        // Undercount stays at the hermetic-pinned shape: −1.5.
        assert!((correction_reward(100, 250) + 1.5).abs() < 1e-6);
        // Overcount is a penalty, not the legacy bonus.
        assert!((correction_reward(250, 100) + 1.5).abs() < 1e-6);
        assert!((correction_reward(950, 1000) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn rewards_scale_linearly_and_never_go_positive() {
        for &delta in &[1_i64, 7, 64, 150, 9999] {
            let r = correction_reward(0, delta);
            assert!(r < 0.0);
            assert!((r + (delta as f32) / REWARD_SCALE).abs() < 1e-4);
        }
    }
}
