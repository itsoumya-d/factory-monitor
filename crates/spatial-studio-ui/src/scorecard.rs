//! EU AI Act readiness scorecard computed from concrete platform facts.

use monitoring_contract::contract::MonitoringContract;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    EmotionLockApplied,
    AggregationStationScoped,
    DpBudgetConfigured,
    DiagnosableSelfAudit,
    TpmProvenanceChain,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check: CheckId,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scorecard {
    pub results: Vec<CheckResult>,
}

impl Scorecard {
    pub fn done_checks(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }

    pub fn pct(self) -> u8 {
        if self.results.is_empty() {
            0
        } else {
            (self.done_checks() * 100 / self.results.len()) as u8
        }
    }
}

pub fn audit(
    contract: &MonitoringContract,
    dp_budget: Option<u32>,
    self_audit_passed: bool,
    tpm_ok: bool,
) -> Scorecard {
    let emotion_ok = compliance_guard::validate::audit_contract(contract).is_ok();
    let aggregation_ok = contract
        .zones
        .iter()
        .all(|z| z.cameras.iter().all(|c| !c.trim().is_empty()));
    Scorecard {
        results: vec![
            CheckResult {
                check: CheckId::EmotionLockApplied,
                passed: emotion_ok,
            },
            CheckResult {
                check: CheckId::AggregationStationScoped,
                passed: aggregation_ok,
            },
            CheckResult {
                check: CheckId::DpBudgetConfigured,
                passed: dp_budget.is_some(),
            },
            CheckResult {
                check: CheckId::DiagnosableSelfAudit,
                passed: self_audit_passed,
            },
            CheckResult {
                check: CheckId::TpmProvenanceChain,
                passed: tpm_ok,
            },
        ],
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use monitoring_contract::templates;

    #[test]
    fn full_green_scorecard_is_100() {
        let contract = templates::qsr_drivethru().expect("fixture");
        let sc = audit(&contract, Some(200), true, true);
        assert_eq!(sc.done_checks(), 5);
        assert_eq!(sc.pct(), 100);
    }

    #[test]
    fn missing_dp_budget_is_partial() {
        let contract = templates::qsr_drivethru().expect("fixture");
        let sc = audit(&contract, None, true, true);
        assert_eq!(sc.done_checks(), 4);
        assert_eq!(sc.pct(), 80);
    }
}
