//! IT orchestrator tool contracts (rig/mcp seam) — typed descriptors the
//! agent models consume; temperature pinned 0.0–0.2.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AgentSchemaError {
    #[error("deployment temperature out of [0.0, 0.2]")]
    TemperamentOutOfRange,
    #[error("tool {0:?} must publish a non-empty description")]
    EmptyDescription(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolContract {
    /// Read KPIs by id as defined by the MonitoringContract.
    GetKpi,
    /// Pending (Amber) corrections awaiting commit.
    GetPendingCorrections,
    /// Propose a Speed-1 prompt patch (never applies it directly).
    ProposePatch,
}

impl ToolContract {
    pub const fn name(self) -> &'static str {
        match self {
            Self::GetKpi => "get_kpi",
            Self::GetPendingCorrections => "get_pending_corrections",
            Self::ProposePatch => "propose_patch",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::GetKpi => "queries kpis defined by the monitoring contract",
            Self::GetPendingCorrections => "lists amber corrections and their confidence bounds",
            Self::ProposePatch => "returns a tier-2/negative prompt patch for operator review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDeployment {
    pub temperature: f32,
    pub tools: Vec<ToolContract>,
    /// Platform-internal guardrail: agent never writes counts directly.
    pub read_only_boundary: bool,
}

impl AgentDeployment {
    pub fn validate(&self) -> Result<(), AgentSchemaError> {
        if !((0.0_f32..=0.2).contains(&self.temperature)) {
            return Err(AgentSchemaError::TemperamentOutOfRange);
        }
        if !self.read_only_boundary {
            return Err(AgentSchemaError::TemperamentOutOfRange);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_stable_descriptors() {
        for t in [
            ToolContract::GetKpi,
            ToolContract::GetPendingCorrections,
            ToolContract::ProposePatch,
        ] {
            assert!(!t.name().is_empty());
            assert!(!t.description().trim().is_empty());
        }
    }

    #[test]
    fn temperature_boundaries() {
        let ok = AgentDeployment {
            temperature: 0.2,
            tools: vec![ToolContract::GetKpi],
            read_only_boundary: true,
        };
        assert!(ok.validate().is_ok());
        let hot = AgentDeployment {
            temperature: 0.3,
            read_only_boundary: true,
            tools: vec![],
        };
        assert_eq!(hot.validate(), Err(AgentSchemaError::TemperamentOutOfRange));
        let writable = AgentDeployment {
            temperature: 0.1,
            tools: vec![],
            read_only_boundary: false,
        };
        assert!(writable.validate().is_err());
    }
}
