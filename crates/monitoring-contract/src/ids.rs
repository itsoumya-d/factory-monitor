//! Stable identifiers for contracts and their elements.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContractId(Uuid);

impl ContractId {
    pub fn now_v7() -> Self {
        Self(Uuid::now_v7())
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for ContractId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClassId(u16);

impl ClassId {
    pub fn try_new(raw: u16) -> Result<Self, ContractError> {
        if raw > 1023 {
            return Err(ContractError::ClassIdOutOfRange(raw));
        }
        Ok(Self(raw))
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RuleId(u16);

impl RuleId {
    pub fn try_new(raw: u16) -> Result<Self, ContractError> {
        if raw == 0 {
            return Err(ContractError::RuleIdZero);
        }
        Ok(Self(raw))
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KpiId(pub u16);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContractError {
    ClassIdOutOfRange(u16),
    RuleIdZero,
    #[serde(skip_deserializing, skip_serializing)]
    Other(&'static str),
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClassIdOutOfRange(v) => write!(
                f,
                "class id {v} exceeds open-vocab vocabulary budget (1023)"
            ),
            Self::RuleIdZero => write!(f, "rule id 0 is reserved"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ContractError {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn class_id_budget_is_1024_ids() {
        assert!(ClassId::try_new(0).is_ok());
        assert!(ClassId::try_new(1023).is_ok());
        assert!(ClassId::try_new(1024).is_err());
    }

    #[test]
    fn rule_id_zero_reserved() {
        assert!(RuleId::try_new(1).is_ok());
        assert!(RuleId::try_new(0).is_err());
    }

    #[test]
    fn contract_id_sortable() {
        let a = ContractId::now_v7();
        let b = ContractId::now_v7();
        assert!(a != b);
    }
}
