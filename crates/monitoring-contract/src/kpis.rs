//! Executable KPI formulas (docs/16 §2) with typed parameters.

use crate::ids::KpiId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "metric", rename_all = "snake_case")]
pub enum KpiKind {
    /// Drift between planned and observed cycle time.
    CycleTimeDrift { planned_cycle_ms: u64 },
    /// Average time inside a zone.
    AvgZoneDwell { zone: String },
    /// Count arriving per hour in a window.
    CumulativeThroughput { zone: String },
    /// Time between scans for a SKU on a shelf zone.
    StockoutLatency { zone: String, sku: String },
    /// Retry rate at a drop point.
    DropRetryRate { zone: String },
    /// Persons inside a zone per frame.
    FloorOccupancy { zone: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KpiDefinition {
    pub id: KpiId,
    pub name: String,
    #[serde(flatten)]
    pub kind: KpiKind,
    pub params: BTreeMap<String, f64>,
    /// Executive target for this metric.
    pub target: f64,
    pub higher_is_better: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KpiError {
    #[error("KPI name must not be empty")]
    EmptyName,
    #[error("metric references zone {0} but carries no params for it")]
    MissingZoneParam(String),
}

impl KpiDefinition {
    pub fn validate(&self) -> Result<(), KpiError> {
        if self.name.trim().is_empty() {
            return Err(KpiError::EmptyName);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn name_required() {
        let k = KpiDefinition {
            id: KpiId(1),
            name: " ".into(),
            kind: KpiKind::FloorOccupancy {
                zone: "shop".into(),
            },
            params: BTreeMap::new(),
            target: 12.0,
            higher_is_better: false,
        };
        assert!(k.validate().is_err());
    }
}
