//! Speed-3: nightly retrain; candidate is promoted only when no
//! historical case regresses, and shadow gates promotion for a grace
//! period before flipping official.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenCase {
    pub case_id: String,
    /// The deployed model's graded outcome for the case.
    pub current_score: f32,
    /// The candidate's graded outcome (None until graded).
    pub candidate_score: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PromotionError {
    #[error("case {0} regressed: {1} → {2}")]
    Regression(String, f32, f32),
    #[error("case {0} not yet graded for the candidate")]
    Unggraded(String),
    #[error("promotion requires shadow validation window passed")]
    ShadowStillRequired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionGate {
    pub candidate_version: String,
    /// The candidate stays in shadow-served traffic until cleared.
    pub shadow_mode: bool,
    pub cases: BTreeMap<String, GoldenCase>,
}

impl RegressionGate {
    pub fn new(candidate_version: String, shadow_mode: bool) -> Self {
        Self {
            candidate_version,
            shadow_mode,
            cases: BTreeMap::new(),
        }
    }

    pub fn add_case(&mut self, case: GoldenCase) {
        self.cases.entry(case.case_id.clone()).or_insert(case);
    }

    /// Semantics for the nightly gate: shadow exit → graded → promote.
    pub fn promote(&self) -> Result<(), PromotionError> {
        if self.shadow_mode {
            return Err(PromotionError::ShadowStillRequired);
        }
        for (id, c) in &self.cases {
            let cand = c
                .candidate_score
                .ok_or_else(|| PromotionError::Unggraded(id.clone()))?;
            if cand < c.current_score {
                return Err(PromotionError::Regression(
                    id.clone(),
                    c.current_score,
                    cand,
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn gate(shadow_mode: bool) -> RegressionGate {
        let mut g = RegressionGate::new("candidate-v2".into(), shadow_mode);
        g.add_case(GoldenCase {
            case_id: "c1".into(),
            current_score: 0.8,
            candidate_score: Some(0.85),
        });
        g.add_case(GoldenCase {
            case_id: "c2".into(),
            current_score: 0.9,
            candidate_score: Some(0.9),
        });
        g
    }

    #[test]
    fn shadow_blocks_promotion() {
        assert_eq!(
            gate(true).promote(),
            Err(PromotionError::ShadowStillRequired)
        );
    }

    #[test]
    fn regressions_block_promotion() {
        let mut g = gate(false);
        g.cases.insert(
            "c1".into(),
            GoldenCase {
                case_id: "c1".into(),
                current_score: 0.8,
                candidate_score: Some(0.5),
            },
        );
        assert_eq!(
            g.promote(),
            Err(PromotionError::Regression("c1".to_string(), 0.8, 0.5))
        );
    }

    #[test]
    fn no_regression_promotes() {
        assert!(gate(false).promote().is_ok());
    }

    #[test]
    fn ungraded_case_blocks() {
        let mut g = gate(false);
        g.cases.insert(
            "c3".into(),
            GoldenCase {
                case_id: "c3".into(),
                current_score: 0.7,
                candidate_score: None,
            },
        );
        assert_eq!(
            g.promote(),
            Err(PromotionError::Unggraded("c3".to_string()))
        );
    }
}
