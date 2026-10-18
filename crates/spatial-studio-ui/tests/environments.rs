//! The "any CCTV + Wi-Fi environment" acceptance suite (docs/17 §1):
//! every shipped vertical template must validate, pass the EU AI Act
//! article-5 scan, score 100% on the compliance scorecard, and feed its
//! native KPIs through the bridge (OPC UA tags + Modbus registers).

#![allow(clippy::unwrap_used)]

use compliance_guard::validate::audit_contract;
use monitoring_contract::contract::MonitoringContract;
use monitoring_contract::kpis::KpiKind;
use monitoring_contract::templates::{
    healthcare_nosocomial, manufacturing_line, qsr_drivethru, retail_warehouse,
};
use spatial_studio_ui::scorecard::{CheckId, Scorecard, audit};
use universal_bridge_rs::fieldbus::WordOrder;
use universal_bridge_rs::kpi_feeder::{kpi_registers, kpi_tag};

///validate + article-5 + station-scoped aggregation per template.
fn contract_facts(c: &MonitoringContract) -> (bool, bool, Scorecard, usize, usize) {
    let valid = c.validate().is_ok();
    let emotion_ok = audit_contract(c).is_ok();
    let aggregation_ok = c
        .zones
        .iter()
        .all(|z| z.cameras.iter().all(|cam| !cam.trim().is_empty()));
    let sc = audit(c, Some(200), true, true);
    (
        valid,
        emotion_ok && aggregation_ok,
        sc,
        c.zones.len(),
        c.targets.len(),
    )
}

fn assert_template_green(c: &MonitoringContract, vertical: &str) {
    let (valid, invariants, sc, zones, targets) = contract_facts(c);
    assert!(valid, "{vertical}: contract geometry/prompts validation");
    assert!(invariants, "{vertical}: emotion lock + station aggregation");
    for r in &sc.results {
        assert!(
            r.passed,
            "{vertical}: check {r:?} failed — vertical not deployable"
        );
    }
    assert_eq!(
        sc.done_checks(),
        5,
        "{vertical}: all five AI Act checks must be green"
    );
    assert_eq!(sc.pct(), 100, "{vertical}: scorecard percent");
    assert!(zones > 0, "{vertical}: zones present");
    assert!(targets > 0, "{vertical}: targets present");
}

#[test]
fn factory_vertical_deploys() {
    let c = manufacturing_line().unwrap();
    assert_template_green(&c, "factory");
    // Its native KPI rides the bridge to OPC UA + Modbus.
    let k = KpiKind::CycleTimeDrift {
        planned_cycle_ms: 1500,
    };
    let tag = kpi_tag(&k);
    assert_eq!(tag, "ns=5;s=shado.monitor.cycle_drift");
    let regs = kpi_registers(&k, 12.0, WordOrder::Abcd).unwrap();
    assert_eq!(regs[0], 0x4028, "12.0 f64 BE words");
}

#[test]
fn cafe_drivethru_vertical_deploys() {
    let c = qsr_drivethru().unwrap();
    assert_template_green(&c, "qsr");
    let z0 = c.zones[0].zone.0.clone();
    let kind = KpiKind::DropRetryRate { zone: z0.clone() };
    let tag = kpi_tag(&kind);
    assert_eq!(tag, format!("ns=5;s=shado.monitor.drop_retry_{z0}"));
    kpi_registers(&kind, 0.35, WordOrder::Cdab).unwrap();
}

#[test]
fn retail_warehouse_vertical_deploys() {
    let c = retail_warehouse().unwrap();
    assert_template_green(&c, "retail");
    let z0 = c.zones[0].zone.0.clone();
    let kind = KpiKind::StockoutLatency {
        zone: z0.clone(),
        sku: "SKU-1".into(),
    };
    let tag = kpi_tag(&kind);
    assert_eq!(tag, format!("ns=5;s=shado.monitor.stockout_{z0}_SKU-1"));
    kpi_registers(&kind, 90.0, WordOrder::Abcd).unwrap();
}

#[test]
fn healthcare_ward_vertical_deploys() {
    let c = healthcare_nosocomial().unwrap();
    assert_template_green(&c, "healthcare");
    let z0 = c.zones[0].zone.0.clone();
    let kind = KpiKind::FloorOccupancy { zone: z0.clone() };
    let tag = kpi_tag(&kind);
    assert_eq!(tag, format!("ns=5;s=shado.monitor.occupancy_{z0}"));
    kpi_registers(&kind, 2.0, WordOrder::Dcba).unwrap();
}

/// Non-finite KPIs must never ride the bus, in ANY vertical.
#[test]
fn nan_kpi_is_rejected_on_the_bridge() {
    for kind in [
        KpiKind::CycleTimeDrift {
            planned_cycle_ms: 1,
        },
        KpiKind::AvgZoneDwell { zone: "z".into() },
    ] {
        let e = kpi_registers(&kind, f64::NAN, WordOrder::Abcd).unwrap_err();
        assert!(matches!(
            e,
            universal_bridge_rs::kpi_feeder::CodecReject::NonFinite
        ));
    }
}

/// Cross-vertical smoke: the checks enumerated are exactly the five
/// documented ones, for every vertical.
#[test]
fn scorecard_covers_exactly_five_documented_checks() {
    let expected = [
        CheckId::EmotionLockApplied,
        CheckId::AggregationStationScoped,
        CheckId::DpBudgetConfigured,
        CheckId::DiagnosableSelfAudit,
        CheckId::TpmProvenanceChain,
    ];
    for contract in [
        manufacturing_line(),
        qsr_drivethru(),
        retail_warehouse(),
        healthcare_nosocomial(),
    ] {
        let c = contract.unwrap();
        let sc = audit(&c, Some(200), true, true);
        let ids: Vec<CheckId> = sc.results.iter().map(|r| r.check).collect();
        assert_eq!(ids, expected.to_vec());
    }
}
