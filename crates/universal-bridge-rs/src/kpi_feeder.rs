//! KPI feeder (docs/17 D4): maps contract KPIs (monitoring_contract)
//! onto the plant-floor buses — every KPI gets a deterministic OPC UA
//! tag inside the platform namespace and a Modbus register encoding in
//! the station's word order. CI checks the plan so a tag typo never
//! storms the production line.

use crate::fieldbus::{FieldValue, WordOrder};
use crate::opcua_plan::STATION_TAG_SLUG;
use monitoring_contract::kpis::KpiKind;

/// Tag suffix for a KPI: `shado.monitor.<variant>_<zone?>`.
pub fn kpi_tag(kind: &KpiKind) -> String {
    let slug = match kind {
        KpiKind::CycleTimeDrift { .. } => "cycle_drift".to_string(),
        KpiKind::AvgZoneDwell { zone } => format!("dwell_{zone}"),
        KpiKind::CumulativeThroughput { zone } => format!("throughput_{zone}"),
        KpiKind::StockoutLatency { zone, sku } => format!("stockout_{zone}_{sku}"),
        KpiKind::DropRetryRate { zone } => format!("drop_retry_{zone}"),
        KpiKind::FloorOccupancy { zone } => format!("occupancy_{zone}"),
    };
    format!("{STATION_TAG_SLUG}{slug}")
}

/// KPI value → Modbus holding-register words in the station's word
/// order (f64 payload, signed semantics preserved by the codec).
pub fn kpi_registers(
    kind: &KpiKind,
    value: f64,
    order: WordOrder,
) -> Result<Vec<u16>, CodecReject> {
    if !value.is_finite() {
        // A NaN/inf KPI must never ride the bus.
        return Err(CodecReject::NonFinite);
    }
    let _ = kind; // tag carries the identity; registers carry the value
    FieldValue::F64(value)
        .registers(order)
        .map_err(|_| CodecReject::EncodeFailed)
}

/// Errors surfaced to the caller instead of a poisoned bus write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecReject {
    #[error("kpi value is not finite")]
    NonFinite,
    #[error("fieldbus encode failed")]
    EncodeFailed,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use monitoring_contract::kpis::KpiKind;

    #[test]
    fn every_kpi_maps_to_a_platform_tag() {
        let kinds = [
            KpiKind::CycleTimeDrift {
                planned_cycle_ms: 3000,
            },
            KpiKind::AvgZoneDwell {
                zone: "bay_1".into(),
            },
            KpiKind::CumulativeThroughput {
                zone: "bay_2".into(),
            },
            KpiKind::StockoutLatency {
                zone: "shelf_a".into(),
                sku: "SKU42".into(),
            },
            KpiKind::DropRetryRate {
                zone: "drop_3".into(),
            },
            KpiKind::FloorOccupancy {
                zone: "shop".into(),
            },
        ];
        for kpi in &kinds {
            let tag = kpi_tag(kpi);
            assert!(tag.starts_with(STATION_TAG_SLUG), "{tag}");
            assert!(
                !tag.ends_with('.'),
                "trailing separator leaks a blank namespace: {tag}"
            );
        }
    }

    #[test]
    fn tags_are_distinct_per_kind_and_zone() {
        let a = kpi_tag(&KpiKind::FloorOccupancy {
            zone: "shop".into(),
        });
        let b = kpi_tag(&KpiKind::FloorOccupancy {
            zone: "depot".into(),
        });
        let c = kpi_tag(&KpiKind::DropRetryRate {
            zone: "shop".into(),
        });
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn kpi_value_writes_the_right_word_count_for_both_buses() {
        let kpi = KpiKind::FloorOccupancy {
            zone: "shop".into(),
        };
        // abcd: full 8-byte f64 = 4 words
        assert_eq!(
            kpi_registers(&kpi, 12.0, WordOrder::Abcd).expect("regs"),
            vec![0x4028, 0x0000, 0x0000, 0x0000]
        );
        // dcba: little-endian byte memory of 12.0f64 = [.. 0x28, 0x40];
        // the final BE word reads byte pair (0x28, 0x40) = 0x2840.
        assert_eq!(
            kpi_registers(&kpi, 12.0, WordOrder::Dcba).expect("regs"),
            vec![0x0000, 0x0000, 0x0000, 0x2840]
        );
    }

    #[test]
    fn non_finite_kpi_is_rejected_before_the_bus() {
        let kpi = KpiKind::AvgZoneDwell {
            zone: "bay_1".into(),
        };
        assert_eq!(
            kpi_registers(&kpi, f64::NAN, WordOrder::Abcd),
            Err(CodecReject::NonFinite)
        );
    }
}
