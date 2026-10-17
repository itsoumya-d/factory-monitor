//! Root schema and referral validation for the whole contract.

use crate::geometry::{ContractZone, GeometryError};
use crate::ids::{ClassId, ContractError, KpiId, RuleId};
use crate::kpis::{KpiDefinition, KpiError};
use crate::ontology::{DomainOntology, VocabularyError};
use crate::rules::{CsiRule, MonitoringRule};
use crate::targets::OpenVocabTarget;
use crate::triggers::Trigger;
use crate::webhooks::{WebhookBinding, WebhookError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitoringContract {
    pub contract_id: crate::ids::ContractId,
    /// Monotonic contract revision — bumped by every effective edit.
    pub version: u32,
    pub name: String,
    pub domain: DomainOntology,
    pub targets: Vec<OpenVocabTarget>,
    pub zones: Vec<ContractZone>,
    pub rules: Vec<MonitoringRule>,
    pub triggers: Vec<Trigger>,
    pub webhooks: Vec<WebhookBinding>,
    pub kpis: Vec<KpiDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContractDiff {
    pub added_rules: Vec<RuleId>,
    pub removed_rules: Vec<RuleId>,
    pub changed_rules: Vec<RuleId>,
}

impl MonitoringContract {
    /// Referral validation; every rule, trigger and KPI must point to
    /// elements that exist in this contract.
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        self.domain
            .validate()
            .map_err(ContractValidationError::Vocabulary)?;

        let mut class_ids = BTreeSet::new();
        for t in &self.targets {
            t.validate().map_err(ContractValidationError::Targets)?;
            if !class_ids.insert(t.class_id) {
                return Err(ContractValidationError::DuplicateClassId(t.class_id));
            }
        }
        if self.targets.is_empty() {
            return Err(ContractValidationError::NoTargets);
        }

        let mut zone_ids = BTreeSet::new();
        for z in &self.zones {
            z.validate().map_err(ContractValidationError::Geometry)?;
            if !zone_ids.insert(z.zone.0.clone()) {
                return Err(ContractValidationError::DuplicateZone(z.zone.0.clone()));
            }
        }
        if self.zones.is_empty() {
            return Err(ContractValidationError::NoZones);
        }

        let mut rule_ids = BTreeSet::new();
        for r in &self.rules {
            r.validate().map_err(ContractValidationError::Rule)?;
            if !rule_ids.insert(r.rule_id()) {
                return Err(ContractValidationError::DuplicateRuleId(r.rule_id()));
            }
            if !zone_ids.contains(&r.zone().0) {
                return Err(ContractValidationError::UnknownZone(r.zone().0.clone()));
            }
            match r {
                MonitoringRule::ZoneEnter { class, .. }
                | MonitoringRule::ZoneExit { class, .. }
                | MonitoringRule::Dwell { class, .. }
                | MonitoringRule::LineCross { class, .. }
                | MonitoringRule::CountDelta { class, .. } => {
                    if !class_ids.contains(class) {
                        return Err(ContractValidationError::UnknownClass(*class));
                    }
                }
                MonitoringRule::Csi(CsiRule { .. }) => {} // sensor-space rule, no class target
            }
            if let MonitoringRule::LineCross { line_ref, .. } = r {
                let known = self
                    .zones
                    .iter()
                    .find(|z| z.zone.0 == r.zone().0)
                    .map(|z| z.guard_lines.iter().any(|l| &l.line_id == line_ref))
                    .unwrap_or(false);
                if !known {
                    return Err(ContractValidationError::UnknownLineRef(line_ref.clone()));
                }
            }
        }
        for t in &self.triggers {
            t.validate().map_err(ContractValidationError::Trigger)?;
            if !rule_ids.contains(&t.on_rule) {
                return Err(ContractValidationError::UnknownRuleRef(t.on_rule));
            }
        }

        for w in &self.webhooks {
            w.validate().map_err(ContractValidationError::Webhook)?;
        }
        if self.webhooks.is_empty() {
            return Err(ContractValidationError::NoWebhook);
        }

        let mut kpi_ids = BTreeSet::new();
        for k in &self.kpis {
            k.validate().map_err(ContractValidationError::Kpi)?;
            if !kpi_ids.insert(k.id) {
                return Err(ContractValidationError::DuplicateKpi(k.id));
            }
        }
        if self.kpis.is_empty() {
            return Err(ContractValidationError::NoKpi);
        }

        Ok(())
    }

    /// Speed-1 patch diffing: base contract → candidate contract.
    pub fn diff(&self, candidate: &Self) -> ContractDiff {
        let mut diff = ContractDiff::default();
        for r in &self.rules {
            let candidate_rule = candidate.rules.iter().find(|c| c.rule_id() == r.rule_id());
            match candidate_rule {
                None => diff.removed_rules.push(r.rule_id()),
                Some(c) if *c != *r => diff.changed_rules.push(r.rule_id()),
                Some(_) => {}
            }
        }
        for c in &candidate.rules {
            if !self.rules.iter().any(|r| r.rule_id() == c.rule_id()) {
                diff.added_rules.push(c.rule_id());
            }
        }
        diff
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ContractValidationError {
    #[error("vocabulary: {0}")]
    Vocabulary(#[from] VocabularyError),
    #[error("targets: {0}")]
    Targets(ContractError),
    #[error("geometry: {0}")]
    Geometry(#[from] GeometryError),
    #[error("rule: {0}")]
    Rule(#[from] ContractError),
    #[error("trigger: {0}")]
    Trigger(ContractError),
    #[error("webhook: {0}")]
    Webhook(#[from] WebhookError),
    #[error("kpi: {0}")]
    Kpi(#[from] KpiError),
    #[error("no targets defined")]
    NoTargets,
    #[error("class id {0:?} duplicated in targets")]
    DuplicateClassId(ClassId),
    #[error("no zones defined")]
    NoZones,
    #[error("zone {0} duplicated")]
    DuplicateZone(String),
    #[error("rule id {0:?} duplicated")]
    DuplicateRuleId(RuleId),
    #[error("rule references unknown zone {0}")]
    UnknownZone(String),
    #[error("rule references unknown class {0:?}")]
    UnknownClass(ClassId),
    #[error("line-cross rule references unknown line {0}")]
    UnknownLineRef(String),
    #[error("trigger references unknown rule {0:?}")]
    UnknownRuleRef(RuleId),
    #[error("no webhook binding")]
    NoWebhook,
    #[error("no KPI defined")]
    NoKpi,
    #[error("kpi id {0:?} duplicated")]
    DuplicateKpi(KpiId),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::templates;

    #[test]
    fn every_template_validates() {
        for built in [
            templates::manufacturing_line(),
            templates::qsr_drivethru(),
            templates::retail_warehouse(),
            templates::healthcare_nosocomial(),
        ] {
            let contract = built.expect("template fixture must be valid");
            assert!(contract.validate().is_ok(), "template must be valid");
        }
    }

    #[test]
    fn diff_detects_changed_rule() {
        let base = templates::manufacturing_line().expect("template fixture");
        let mut candidate = base.clone();
        let MonitoringRule::Dwell { min_ms, emit, .. } = &mut candidate.rules[0] else {
            unreachable!("first rule is dwell in template");
        };
        *min_ms += 1;
        emit.debounce_ms += 1;
        let d = base.diff(&candidate);
        assert_eq!(d.added_rules.len(), 0);
        assert_eq!(d.removed_rules.len(), 0);
        assert_eq!(d.changed_rules.len(), 1);
    }
}
