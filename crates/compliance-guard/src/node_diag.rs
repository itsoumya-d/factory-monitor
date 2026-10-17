//! DiagnosableNode trait + the five anti-shortcut controls every
//! perception node must survive (docs/15, docs/16 §4).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutControl {
    /// Constants collapse the output distribution to the training prior.
    OutputPriorBias,
    /// A change of input must visibly change outputs.
    ConstantInputFault,
    /// Shuffling subcarriers must destroy accuracy, else the input is decorative.
    SubcarrierShuffle,
    /// Nearest-neighbor memorized answers hiding real generalization.
    NearestNeighborLeak,
    /// Temporal-only models must fail when time order is destroyed.
    TemporalOnlyShortcut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlVerdict {
    pub control: ShortcutControl,
    pub verdict: Verdict,
    /// Free-form evidence, kept small for the audit trail.
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorSelfAuditReport {
    pub node_id: String,
    pub verdicts: Vec<ControlVerdict>,
}

impl SensorSelfAuditReport {
    pub fn all_control_verdicts(&self) -> Result<&ControlVerdict, ShortcutControl> {
        let failed = self.verdicts.iter().find(|v| v.verdict == Verdict::Fail);
        match failed {
            Some(v) => Err(v.control),
            None => self
                .verdicts
                .first()
                .ok_or(ShortcutControl::OutputPriorBias),
        }
    }
}

/// The contract every perceivable node (CSI chip, camera host) declares.
pub trait DiagnosableNode {
    /// Stable node identity.
    fn node_id(&self) -> String;
    /// Run the anti-shortcut battery and report layer-verdicts.
    fn self_audit(&self) -> SensorSelfAuditReport;
}

impl ShortcutControl {
    /// Exhaustive list in run order.
    pub fn all_variants() -> Vec<Self> {
        vec![
            Self::OutputPriorBias,
            Self::ConstantInputFault,
            Self::SubcarrierShuffle,
            Self::NearestNeighborLeak,
            Self::TemporalOnlyShortcut,
        ]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_report(fail_control: Option<ShortcutControl>) -> SensorSelfAuditReport {
        let verdicts = ShortcutControl::all_variants();
        let cast = verdicts
            .into_iter()
            .map(|c| ControlVerdict {
                control: c,
                verdict: if Some(c) == fail_control {
                    Verdict::Fail
                } else {
                    Verdict::Pass
                },
                detail: "test".into(),
            })
            .collect();
        SensorSelfAuditReport {
            node_id: "node-1".into(),
            verdicts: cast,
        }
    }

    #[test]
    fn reports_fail_on_first_failed_control() {
        let ok = sample_report(None);
        assert!(ok.all_control_verdicts().is_ok());

        let bad = sample_report(Some(ShortcutControl::SubcarrierShuffle));
        assert_eq!(
            bad.all_control_verdicts(),
            Err(ShortcutControl::SubcarrierShuffle)
        );
    }
}
