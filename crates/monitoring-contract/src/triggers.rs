//! Event schedule: when rules are allowed to evaluate and notify.

use crate::ids::{ContractError, RuleId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    pub on_rule: RuleId,
    /// Rule must hold stable for this long before firing (debounce).
    pub stable_for_ms: u64,
    /// After a fire, re-arm only after this long.
    pub cooldown_ms: u64,
}

impl Trigger {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.stable_for_ms == 0 || self.cooldown_ms == 0 {
            return Err(ContractError::Other("trigger debounces must be positive"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn trigger_basics() {
        let ok = Trigger {
            on_rule: RuleId::try_new(1).unwrap(),
            stable_for_ms: 2_000,
            cooldown_ms: 10_000,
        };
        assert!(ok.validate().is_ok());
        let bad = Trigger {
            on_rule: ok.on_rule,
            stable_for_ms: 0,
            cooldown_ms: 10_000,
        };
        assert!(bad.validate().is_err());
    }
}
