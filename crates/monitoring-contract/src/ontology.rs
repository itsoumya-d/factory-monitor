//! Domain vocabulary: which kind of site, and which observable work stages.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainKind {
    Manufacturing,
    Qsr,
    RetailWarehouse,
    Healthcare,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkStage {
    /// Stable verb-noun identifier, e.g. "assemble_panel".
    pub stage_id: String,
    /// Human label shown in UI.
    pub name: String,
    /// Rules referencing a stage may only use ids listed here.
    pub observable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainOntology {
    pub kind: DomainKind,
    pub work_stages: Vec<WorkStage>,
}

impl DomainOntology {
    pub fn validate(&self) -> Result<(), VocabularyError> {
        if self.work_stages.is_empty() {
            return Err(VocabularyError::NoWorkStages);
        }
        let mut seen = Vec::new();
        for stage in &self.work_stages {
            if stage.stage_id.is_empty() {
                return Err(VocabularyError::EmptyStageId);
            }
            if seen.contains(&stage.stage_id) {
                return Err(VocabularyError::DuplicateStageId(stage.stage_id.clone()));
            }
            seen.push(stage.stage_id.clone());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VocabularyError {
    #[error("domain must declare at least one work stage")]
    NoWorkStages,
    #[error("work stage id must not be empty")]
    EmptyStageId,
    #[error("duplicate stage id: {0}")]
    DuplicateStageId(String),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn stage(id: &str) -> WorkStage {
        WorkStage {
            stage_id: id.to_string(),
            name: id.to_string(),
            observable: true,
        }
    }

    #[test]
    fn rejects_empty_and_duplicate_stages() {
        let bad = DomainOntology {
            kind: DomainKind::Qsr,
            work_stages: vec![],
        };
        assert_eq!(bad.validate(), Err(VocabularyError::NoWorkStages));

        let dup = DomainOntology {
            kind: DomainKind::Qsr,
            work_stages: vec![stage("a"), stage("a")],
        };
        assert!(dup.validate().is_err());
    }

    #[test]
    fn valid_ontology_passes() {
        let ok = DomainOntology {
            kind: DomainKind::Manufacturing,
            work_stages: vec![stage("assembly"), stage("inspection")],
        };
        assert!(ok.validate().is_ok());
    }
}
