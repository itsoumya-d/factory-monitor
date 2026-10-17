//! Typed rule set: figures of rules every vertical template can express,
//! including the CSI rules (docs/16 §2).

use crate::geometry::{Direction, ZoneId};
use crate::ids::{ClassId, ContractError, RuleId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmitPolicy {
    /// Suppress repeat emits within this window.
    pub debounce_ms: u64,
    pub only_on_transition: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaOp {
    Increase,
    Decrease,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsiRule {
    pub rule_id: RuleId,
    pub zone: ZoneId,
    pub kind: CsiRuleKind,
    pub emit: EmitPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CsiRuleKind {
    /// Motion presence above a multipath-variance threshold.
    Threshold { variance_min: f32, duration_ms: u64 },
    /// Trend anomaly on Doppler velocity spectrum.
    DopplerAnomaly { z_threshold: f32, window_ms: u64 },
    /// Occupancy estimate from BVP-compatible amplitude volume.
    Occupancy { max_persons: u8 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum MonitoringRule {
    /// Zone occupancy enters with class present.
    ZoneEnter {
        rule_id: RuleId,
        zone: ZoneId,
        class: ClassId,
        emit: EmitPolicy,
    },
    ZoneExit {
        rule_id: RuleId,
        zone: ZoneId,
        class: ClassId,
        emit: EmitPolicy,
    },
    Dwell {
        rule_id: RuleId,
        zone: ZoneId,
        class: ClassId,
        min_ms: u64,
        emit: EmitPolicy,
    },
    LineCross {
        rule_id: RuleId,
        zone: ZoneId,
        line_ref: String,
        class: ClassId,
        direction: Direction,
        emit: EmitPolicy,
    },
    CountDelta {
        rule_id: RuleId,
        zone: ZoneId,
        class: ClassId,
        op: DeltaOp,
        threshold: i32,
        window_ms: u64,
        emit: EmitPolicy,
    },
    Csi(CsiRule),
}

impl MonitoringRule {
    pub fn rule_id(&self) -> RuleId {
        match self {
            Self::ZoneEnter { rule_id, .. }
            | Self::ZoneExit { rule_id, .. }
            | Self::Dwell { rule_id, .. }
            | Self::LineCross { rule_id, .. }
            | Self::CountDelta { rule_id, .. }
            | Self::Csi(CsiRule { rule_id, .. }) => *rule_id,
        }
    }

    pub fn zone(&self) -> &ZoneId {
        match self {
            Self::ZoneEnter { zone, .. }
            | Self::ZoneExit { zone, .. }
            | Self::Dwell { zone, .. }
            | Self::LineCross { zone, .. }
            | Self::CountDelta { zone, .. }
            | Self::Csi(CsiRule { zone, .. }) => zone,
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        match self {
            Self::Dwell { min_ms, emit, .. } => {
                if *min_ms == 0 {
                    return Err(ContractError::Other("dwell min_ms must be positive"));
                }
                if emit.debounce_ms == 0 {
                    return Err(ContractError::Other("emit debounce_ms must be positive"));
                }
            }
            Self::CountDelta {
                threshold,
                window_ms,
                emit,
                ..
            } => {
                if *threshold == 0 {
                    return Err(ContractError::Other(
                        "count delta threshold must be non-zero",
                    ));
                }
                if *window_ms == 0 {
                    return Err(ContractError::Other("count delta window must be positive"));
                }
                if emit.debounce_ms == 0 {
                    return Err(ContractError::Other("emit debounce_ms must be positive"));
                }
            }
            Self::Csi(CsiRule { kind, emit, .. }) => {
                if emit.debounce_ms == 0 {
                    return Err(ContractError::Other("emit debounce_ms must be positive"));
                }
                match kind {
                    CsiRuleKind::Threshold {
                        variance_min,
                        duration_ms,
                    } => {
                        if !variance_min.is_finite() || *variance_min <= 0.0 || *duration_ms == 0 {
                            return Err(ContractError::Other("csi threshold rule out of range"));
                        }
                    }
                    CsiRuleKind::DopplerAnomaly {
                        z_threshold,
                        window_ms,
                    } => {
                        if !z_threshold.is_finite() || *z_threshold < 1.0 || *window_ms == 0 {
                            return Err(ContractError::Other("csi doppler rule out of range"));
                        }
                    }
                    CsiRuleKind::Occupancy { max_persons } => {
                        if *max_persons == 0 {
                            return Err(ContractError::Other(
                                "csi occupancy must allow at least one person",
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn zone() -> ZoneId {
        ZoneId("line1".into())
    }

    fn dwell() -> MonitoringRule {
        MonitoringRule::Dwell {
            rule_id: RuleId::try_new(1).expect("id"),
            zone: zone(),
            class: ClassId::try_new(7).expect("id"),
            min_ms: 500,
            emit: EmitPolicy {
                debounce_ms: 1_000,
                only_on_transition: true,
            },
        }
    }

    #[test]
    fn valid_dwell_rule_passes() {
        assert!(dwell().validate().is_ok());
    }

    #[test]
    fn dwell_and_emit_must_be_positive() {
        let mut zero_dwell = dwell();
        match &mut zero_dwell {
            MonitoringRule::Dwell { min_ms, .. } => *min_ms = 0,
            _ => unreachable!("dwelling rule"),
        }
        assert!(zero_dwell.validate().is_err());

        let mut zero_debounce = dwell();
        match &mut zero_debounce {
            MonitoringRule::Dwell { emit, .. } => emit.debounce_ms = 0,
            _ => unreachable!("dwelling rule"),
        }
        assert!(zero_debounce.validate().is_err());
    }

    #[test]
    fn csi_rules_range_checked() {
        let ok = MonitoringRule::Csi(CsiRule {
            rule_id: RuleId::try_new(2).expect("id"),
            zone: zone(),
            kind: CsiRuleKind::Threshold {
                variance_min: 0.5,
                duration_ms: 200,
            },
            emit: EmitPolicy {
                debounce_ms: 500,
                only_on_transition: false,
            },
        });
        assert!(ok.validate().is_ok());

        let bad_z = MonitoringRule::Csi(CsiRule {
            rule_id: RuleId::try_new(3).expect("id"),
            zone: zone(),
            kind: CsiRuleKind::DopplerAnomaly {
                z_threshold: 0.5,
                window_ms: 100,
            },
            emit: EmitPolicy {
                debounce_ms: 500,
                only_on_transition: false,
            },
        });
        assert!(bad_z.validate().is_err());
    }

    #[test]
    fn rule_id_extractor_works() {
        assert_eq!(dwell().rule_id().raw(), 1);
        assert_eq!(dwell().zone().0, "line1");
    }
}
