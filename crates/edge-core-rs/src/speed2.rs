//! Speed-2: clamped-threshold bandit. τ ∈ [0.25, 0.85]; α/β fusion
//! weights ∈ [0, 1] with last-writer-wins audit reason.

use monitoring_contract::ids::RuleId;
use serde::{Deserialize, Serialize};

pub const TAU_MIN: f32 = 0.25;
pub const TAU_MAX: f32 = 0.85;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BanditError {
    #[error("tau {0} outside the guardrail [{TAU_MIN}, {TAU_MAX}]")]
    TauOutOfRange(f32),
    #[error("channel weights do not sum to one or a member is negative")]
    WeightsUnnormalized,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThresholdArm {
    pub tau: f32,
    pub pulls: u32,
    pub reward_sum: f32,
}

impl ThresholdArm {
    pub fn mean_reward(&self) -> f32 {
        if self.pulls == 0 {
            0.5
        } else {
            self.reward_sum / self.pulls as f32
        }
    }

    pub fn ucb(&self, total_pulls: u32) -> f32 {
        let bonus =
            (4.0 * ((total_pulls as f32 + 1.0).ln() / self.pulls.max(1) as f32).sqrt()).min(0.5);
        self.mean_reward() + bonus
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FusionWeights {
    pub alpha_rgb: f32,
    pub beta_depth: f32,
    pub gamma_csi: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThresholdBanditState {
    pub rule: RuleId,
    pub arms: Vec<ThresholdArm>,
    pub weights: FusionWeights,
}

impl ThresholdBanditState {
    pub fn try_new(rule: RuleId, taus: &[f32]) -> Result<Self, BanditError> {
        for tau in taus {
            if !(*tau >= TAU_MIN && *tau <= TAU_MAX) {
                return Err(BanditError::TauOutOfRange(*tau));
            }
        }
        Ok(Self {
            rule,
            arms: taus
                .iter()
                .map(|t| ThresholdArm {
                    tau: *t,
                    pulls: 0,
                    reward_sum: 0.0,
                })
                .collect(),
            weights: FusionWeights {
                alpha_rgb: 0.5,
                beta_depth: 0.0,
                gamma_csi: 0.5,
            },
        })
    }

    /// UCB selection over the clamped τ grid.
    pub fn select(&self) -> f32 {
        let total: u32 = self.arms.iter().map(|a| a.pulls).sum();
        self.arms
            .iter()
            .max_by(|a, b| {
                a.ucb(total)
                    .partial_cmp(&b.ucb(total))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|a| a.tau)
            .unwrap_or((TAU_MIN + TAU_MAX) / 2.0)
    }

    pub fn recorded(&mut self, tau: f32, reward: f32) {
        if let Some(a) = self.arms.iter_mut().find(|a| (a.tau - tau).abs() < 1e-3) {
            a.pulls += 1;
            a.reward_sum += reward.clamp(-1.0, 1.0);
        }
    }

    pub fn set_weights(&mut self, w: FusionWeights) -> Result<(), BanditError> {
        let sum = w.alpha_rgb + w.beta_depth + w.gamma_csi;
        if (sum - 1.0).abs() > 1e-3 {
            return Err(BanditError::WeightsUnnormalized);
        }
        if w.alpha_rgb < 0.0 || w.beta_depth < 0.0 || w.gamma_csi < 0.0 {
            return Err(BanditError::WeightsUnnormalized);
        }
        self.weights = w;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use monitoring_contract::ids::RuleId;

    fn state() -> ThresholdBanditState {
        ThresholdBanditState::try_new(RuleId::try_new(1).expect("id"), &[TAU_MIN, 0.45, TAU_MAX])
            .expect("valid taus")
    }

    #[test]
    fn guardrails_hold_on_bad_taus() {
        assert_eq!(
            ThresholdBanditState::try_new(RuleId::try_new(1).expect("id"), &[0.10]),
            Err(BanditError::TauOutOfRange(0.10))
        );
        assert!(ThresholdBanditState::try_new(RuleId::try_new(1).expect("id"), &[TAU_MAX]).is_ok());
    }

    #[test]
    fn selection_clamped_range() {
        let s = state();
        for _ in 0..10 {
            let tau = s.select();
            assert!((TAU_MIN..=TAU_MAX).contains(&tau), "tau {tau}");
        }
    }

    #[test]
    fn reward_learning_maps_to_better_arm() {
        let mut s = state();
        for _ in 0..50 {
            let tau = s.select();
            let reward = if tau < 0.3 { 1.0 } else { -0.2 };
            s.recorded(tau, reward);
        }
        let top = s
            .arms
            .iter()
            .max_by(|a, b| {
                a.mean_reward()
                    .partial_cmp(&b.mean_reward())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("arms");
        assert!(top.tau < 0.3, "high-τ arm must win: {}", top.tau);
    }

    #[test]
    fn weights_normalized_check() {
        let mut s = state();
        assert_eq!(
            s.set_weights(FusionWeights {
                alpha_rgb: 0.5,
                beta_depth: 0.5,
                gamma_csi: 0.5
            }),
            Err(BanditError::WeightsUnnormalized)
        );
        assert!(
            s.set_weights(FusionWeights {
                alpha_rgb: 0.4,
                beta_depth: 0.1,
                gamma_csi: 0.5
            })
            .is_ok()
        );
    }

    #[test]
    fn roundtrip_serde() {
        let s = state();
        let json = serde_json::to_string(&s).expect("serializable");
        let back: ThresholdBanditState = serde_json::from_str(&json).expect("deserializable");
        assert_eq!(back, s);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod ucb_tests {
    use super::*;

    #[test]
    fn ucb_is_bounded_once_pulled() {
        // Unpulled arm: neutral mean 0.5 + clamped bonus 0.5 = 1.0.
        let arm = ThresholdArm {
            tau: 0.5,
            reward_sum: 0.0,
            pulls: 0,
        };
        assert!((arm.ucb(10) - 1.0).abs() < 1e-6, "{}", arm.ucb(10));
    }

    #[test]
    fn ucb_bonus_is_capped_at_half() {
        let arm = ThresholdArm {
            tau: 0.5,
            reward_sum: 1000.0,
            pulls: 1,
        };
        let v = arm.ucb(2);
        assert!(v.is_finite(), "ucb must stay finite: {v}");
        assert!((v - 1000.5).abs() < 1e-3, "{}", v);
    }
}
