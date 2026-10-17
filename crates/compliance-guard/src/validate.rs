//! Contract-level compliance engine: wires Art 5 prong 1 over all
//! open-vocab prompts and person-state labels.

use monitoring_contract::contract::MonitoringContract;

use crate::article5::Article5Error;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ContractComplianceError {
    #[error("target {0}: {1}")]
    Target(usize, #[source] Article5Error),
}

/// Scan every target prompt and negative prompt in the contract;
/// a single forbidden word rejects the whole contract.
pub fn audit_contract(contract: &MonitoringContract) -> Result<(), ContractComplianceError> {
    for (i, t) in contract.targets.iter().enumerate() {
        crate::article5::scan_prompt(&t.tier1_prompt)
            .map_err(|e| ContractComplianceError::Target(i, e))?;
        let tier2_ok = t
            .tier2_prompt
            .as_deref()
            .map(|p| {
                crate::article5::scan_prompt(p).map_err(|e| ContractComplianceError::Target(i, e))
            })
            .unwrap_or(Ok(()));
        tier2_ok?;
        for n in &t.negatives {
            crate::article5::scan_prompt(n).map_err(|e| ContractComplianceError::Target(i, e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{ContractComplianceError, audit_contract};
    use monitoring_contract::templates;

    #[test]
    fn all_vertical_templates_pass_compliance() {
        for built in [
            templates::manufacturing_line(),
            templates::qsr_drivethru(),
            templates::retail_warehouse(),
            templates::healthcare_nosocomial(),
        ] {
            let c = built.expect("template fixture must be valid");
            assert!(audit_contract(&c).is_ok());
        }
    }

    #[test]
    fn emotionword_in_prompt_rejects_contract() {
        let mut c = templates::qsr_drivethru().expect("template fixture");
        c.targets[0].tier1_prompt = "a smiling car at the drive-through".into();
        let got = audit_contract(&c);
        assert!(
            matches!(got, Err(ContractComplianceError::Target(_, _))),
            "single {got:?}"
        );
    }
}
