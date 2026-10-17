//! Open-vocabulary prompt targets (docs/16 §2): tier-1 / tier-2 prompts,
//! negative prompts, and the CLIP-score gate.

use crate::ids::{ClassId, ContractError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenVocabTarget {
    pub class_id: ClassId,
    pub display_name: String,
    /// Tier-1: short, high-recall text embedding prompt.
    pub tier1_prompt: String,
    /// Tier-2: heavy prompt for re-scoring uncertain detections.
    pub tier2_prompt: Option<String>,
    pub negatives: Vec<String>,
    /// CLIP score lower-bound; protocol clamps to [0.30, 0.95].
    pub min_score: f32,
}

impl OpenVocabTarget {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.display_name.trim().is_empty() {
            return Err(ContractError::Other("display name must not be empty"));
        }
        if self.tier1_prompt.trim().is_empty() {
            return Err(ContractError::Other("tier-1 prompt must not be empty"));
        }
        if self.tier1_prompt.chars().count() > 128 {
            return Err(ContractError::Other("tier-1 prompt exceeds 128 chars"));
        }
        if !(0.30..=0.95).contains(&self.min_score) {
            return Err(ContractError::Other("min_score outside [0.30, 0.95]"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn target(min_score: f32, tier1: &str) -> OpenVocabTarget {
        OpenVocabTarget {
            class_id: ClassId::try_new(1).unwrap(),
            display_name: "panel".to_string(),
            tier1_prompt: tier1.to_string(),
            tier2_prompt: None,
            negatives: vec![],
            min_score,
        }
    }

    #[test]
    fn score_gate_protocol_range() {
        assert!(target(0.5, "a metal panel").validate().is_ok());
        assert!(target(0.29, "x").validate().is_err());
        assert!(target(0.96, "x").validate().is_err());
    }

    #[test]
    fn empty_prompt_denied() {
        assert!(target(0.5, "   ").validate().is_err());
    }
}
