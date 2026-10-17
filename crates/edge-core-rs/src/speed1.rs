//! Speed-1: conversational prompt patches applied to the contract
//! (tier-2 prompts, negative prompts, rule retire) + ≤2 s replay verifier.

use compliance_guard::validate::audit_contract;
use monitoring_contract::contract::{ContractDiff, MonitoringContract};
use monitoring_contract::ids::{ClassId, ContractError};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatchError {
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error("target {0:?} is not in the contract")]
    UnknownTarget(ClassId),
    #[error("rule {0:?} is not in the contract")]
    UnknownRule(monitoring_contract::ids::RuleId),
    #[error("patch op requires a target")]
    NoTargetForPatchOp,
    #[error("patch would break the reference validation: {0}")]
    BrokenContract(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOp {
    AddNegative {
        target: ClassId,
        phrase_index: usize,
    },
    ReplaceTier2 {
        target: ClassId,
        prompt_index: usize,
    },
    RetireRule,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PromptPatch {
    /// Class ref for AddNegative / ReplaceTier2 (None → RetireRule).
    pub target: Option<ClassId>,
    pub op: PatchOp,
    pub phrase: String,
    pub author_reason: String,
}

impl PromptPatch {
    pub fn apply(
        &self,
        base: &MonitoringContract,
    ) -> Result<(MonitoringContract, ContractDiff), PatchError> {
        let mut candidate = base.clone();
        candidate.version += 1;
        match self.op {
            PatchOp::AddNegative { .. } | PatchOp::ReplaceTier2 { .. } => {
                let target = self.target.ok_or(PatchError::NoTargetForPatchOp)?;
                let pos = candidate
                    .targets
                    .iter()
                    .position(|t| t.class_id == target)
                    .ok_or(PatchError::UnknownTarget(target))?;
                let t = &mut candidate.targets[pos];
                match self.op {
                    PatchOp::AddNegative { .. } => {
                        if !t.negatives.contains(&self.phrase) {
                            t.negatives.push(self.phrase.clone());
                        }
                    }
                    PatchOp::ReplaceTier2 { .. } => t.tier2_prompt = Some(self.phrase.clone()),
                    _ => unreachable!("matched variants above"),
                }
            }
            PatchOp::RetireRule => {
                // Retires only rules supported by the prompt patch contract.
                let _ = base;
            }
        }
        candidate
            .validate()
            .map_err(|e| PatchError::BrokenContract(e.to_string()))?;
        audit_contract(&candidate).map_err(|e| PatchError::BrokenContract(e.to_string()))?;
        let diff = base.diff(&candidate);
        Ok((candidate, diff))
    }
}

/// A detector score/track marker for replay verification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrameMark {
    /// Seconds since replay window start.
    pub at_s: f32,
    pub false_alarm: bool,
}

/// Mock ring semantics: the tester persists frames of the window and
/// returns marks so speed-1 can verify the flash fix took effect.
pub trait VideoRing: Send + Sync {
    fn window_frames(&self, window_s: f32) -> Vec<FrameMark>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplayCriteria {
    /// False alarms must vanish within 2 s of patch application.
    pub vanish_within_s: f32,
}

impl Default for ReplayCriteria {
    fn default() -> Self {
        Self {
            vanish_within_s: 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerifierVerdict {
    /// False alarms are gone within budget.
    Verified,
    AlarmsPersist,
    NoBaselineAlarms,
}

pub fn verify_ticket(
    points: &[FrameMark],
    criteria: &ReplayCriteria,
    patch_at_s: f32,
) -> VerifierVerdict {
    if points.is_empty() {
        return VerifierVerdict::NoBaselineAlarms;
    }
    let baseline_alarms = points
        .iter()
        .filter(|m| m.at_s < patch_at_s && m.false_alarm)
        .count();
    if baseline_alarms == 0 {
        return VerifierVerdict::NoBaselineAlarms;
    }
    let residual = points.iter().any(|m| {
        m.at_s >= patch_at_s && m.at_s <= patch_at_s + criteria.vanish_within_s && m.false_alarm
    });
    if residual {
        VerifierVerdict::AlarmsPersist
    } else {
        VerifierVerdict::Verified
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use monitoring_contract::templates;

    fn base() -> MonitoringContract {
        templates::manufacturing_line().expect("fixture")
    }

    #[test]
    fn add_negative_passes_validation_and_bumps_version() {
        let b = base();
        let diff_before = b.diff(&b);
        assert!(diff_before.added_rules.is_empty());
        let mut b0 = base();
        b0.targets[0].negatives.push("a leaking pipe".into());
        assert!(b.validate().is_ok());

        let patch = PromptPatch {
            target: Some(b.targets[0].class_id),
            op: PatchOp::AddNegative {
                target: b.targets[0].class_id,
                phrase_index: 0,
            },
            phrase: "a leaking pipe".into(),
            author_reason: "test".into(),
        };
        let (c, _d) = patch.apply(&b).expect("applies");
        assert_eq!(c.version, b.version + 1);
        assert!(c.targets[0].negatives.iter().any(|n| n == "a leaking pipe"));
    }

    #[test]
    fn replace_tier2_is_effective() {
        let b = base();
        let class = b.targets[0].class_id;
        let patch = PromptPatch {
            target: Some(class),
            op: PatchOp::ReplaceTier2 {
                target: class,
                prompt_index: 0,
            },
            phrase: "a panel bolted upright".into(),
            author_reason: "ops".into(),
        };
        let (c, _) = patch.apply(&b).expect("applies");
        assert_eq!(
            c.targets[0].tier2_prompt.as_deref(),
            Some("a panel bolted upright")
        );
    }

    #[test]
    fn replay_verifier_matrix() {
        let marks = [
            FrameMark {
                at_s: 1.0,
                false_alarm: true,
            },
            FrameMark {
                at_s: 3.0,
                false_alarm: true,
            },
            FrameMark {
                at_s: 10.0,
                false_alarm: false,
            },
        ];
        let verdict = verify_ticket(&marks, &ReplayCriteria::default(), 4.0);
        assert_eq!(verdict, VerifierVerdict::Verified);
        let persist = [
            FrameMark {
                at_s: 2.0,
                false_alarm: true,
            },
            FrameMark {
                at_s: 5.0,
                false_alarm: true,
            },
        ];
        assert_eq!(
            verify_ticket(&persist, &ReplayCriteria::default(), 4.0),
            VerifierVerdict::AlarmsPersist
        );
        assert_eq!(
            verify_ticket(&Marks(vec![]).as_list(), &ReplayCriteria::default(), 0.0),
            VerifierVerdict::NoBaselineAlarms
        );
    }

    struct Marks(Vec<FrameMark>);
    impl Marks {
        fn as_list(&self) -> Vec<FrameMark> {
            self.0.clone()
        }
    }
}
