//! Vertical template seeds: minimum viable, valid, ready-to-clone
//! contracts per domain (docs/16 §2).

use crate::contract::MonitoringContract;
use crate::geometry::{ContractZone, Direction, DwellPolicy, GeoPoint, GuardLine, Polygon, ZoneId};
use crate::ids::{ClassId, ContractError, ContractId, KpiId, RuleId};
use crate::kpis::{KpiDefinition, KpiKind};
use crate::ontology::{DomainKind, DomainOntology, WorkStage};
use crate::rules::{DeltaOp, EmitPolicy, MonitoringRule};
use crate::targets::OpenVocabTarget;
use crate::triggers::Trigger;
use crate::webhooks::{RetryPolicy, WebhookBinding, WebhookChannel, WebhookEvent};

fn target(id: u16, name: &str, tier1: &str) -> Result<OpenVocabTarget, ContractError> {
    Ok(OpenVocabTarget {
        class_id: ClassId::try_new(id)?,
        display_name: name.into(),
        tier1_prompt: tier1.into(),
        tier2_prompt: Some(format!("{tier1}, high detail")),
        negatives: vec![],
        min_score: 0.55,
    })
}

fn zone(id: &str, cams: &[&str]) -> ContractZone {
    ContractZone {
        zone: ZoneId(id.into()),
        cameras: cams.iter().map(|c| c.to_string()).collect(),
        container: Polygon(vec![
            GeoPoint { x: 0.1, y: 0.1 },
            GeoPoint { x: 0.9, y: 0.1 },
            GeoPoint { x: 0.9, y: 0.9 },
            GeoPoint { x: 0.1, y: 0.9 },
        ]),
        guard_lines: vec![GuardLine {
            line_id: "mid".into(),
            a: GeoPoint { x: 0.5, y: 0.2 },
            b: GeoPoint { x: 0.5, y: 0.8 },
        }],
        dwell_policy: Some(DwellPolicy {
            warn_after_s: 60,
            alert_after_s: 120,
        }),
    }
}

fn dwell_rule(
    id: u16,
    zone_id: &str,
    class: u16,
    min_ms: u64,
) -> Result<MonitoringRule, ContractError> {
    Ok(MonitoringRule::Dwell {
        rule_id: RuleId::try_new(id)?,
        zone: ZoneId(zone_id.into()),
        class: ClassId::try_new(class)?,
        min_ms,
        emit: EmitPolicy {
            debounce_ms: 5_000,
            only_on_transition: true,
        },
    })
}

fn stage(id: &str) -> WorkStage {
    WorkStage {
        stage_id: id.into(),
        name: id.into(),
        observable: true,
    }
}

fn trigger(id: u16) -> Result<Trigger, ContractError> {
    Ok(Trigger {
        on_rule: RuleId::try_new(id)?,
        stable_for_ms: 2_000,
        cooldown_ms: 15_000,
    })
}

fn webhook() -> WebhookBinding {
    WebhookBinding {
        channel: WebhookChannel::Http,
        url: "https://hooks.example.invalid/shado".into(),
        secret_ref: Some("vault://hooks/vertical".into()),
        events: vec![
            WebhookEvent::Dwell,
            WebhookEvent::ZoneEntry,
            WebhookEvent::ZoneExit,
        ],
        retry: RetryPolicy {
            max_attempts: 3,
            backoff_ms: 5_000,
        },
    }
}

fn kpi(
    id: u16,
    name: &str,
    kind: KpiKind,
    target_value: f64,
    higher_is_better: bool,
) -> KpiDefinition {
    KpiDefinition {
        id: KpiId(id),
        name: name.into(),
        kind,
        params: Default::default(),
        target: target_value,
        higher_is_better,
    }
}

pub fn manufacturing_line() -> Result<MonitoringContract, ContractError> {
    Ok(MonitoringContract {
        contract_id: ContractId::now_v7(),
        version: 1,
        name: "assembly-line-template".into(),
        domain: DomainOntology {
            kind: DomainKind::Manufacturing,
            work_stages: vec![
                stage("sourcing"),
                stage("assembly"),
                stage("inspection"),
                stage("handling"),
            ],
        },
        targets: vec![
            target(1, "panel", "a metal panel")?,
            target(2, "worker", "a factory worker")?,
        ],
        zones: vec![zone("line1", &["cam-line-a"])],
        rules: vec![
            dwell_rule(1, "line1", 2, 60_000)?,
            MonitoringRule::CountDelta {
                rule_id: RuleId::try_new(2)?,
                zone: ZoneId("line1".into()),
                class: ClassId::try_new(1)?,
                op: DeltaOp::Decrease,
                threshold: -1,
                window_ms: 10_000,
                emit: EmitPolicy {
                    debounce_ms: 2_000,
                    only_on_transition: false,
                },
            },
            MonitoringRule::LineCross {
                rule_id: RuleId::try_new(3)?,
                zone: ZoneId("line1".into()),
                line_ref: "mid".into(),
                class: ClassId::try_new(2)?,
                direction: Direction::Ab,
                emit: EmitPolicy {
                    debounce_ms: 1_000,
                    only_on_transition: true,
                },
            },
        ],
        triggers: vec![trigger(1)?],
        webhooks: vec![webhook()],
        kpis: vec![kpi(
            1,
            "cycle drift",
            KpiKind::CycleTimeDrift {
                planned_cycle_ms: 90_000,
            },
            0.10,
            false,
        )],
    })
}

pub fn qsr_drivethru() -> Result<MonitoringContract, ContractError> {
    Ok(MonitoringContract {
        contract_id: ContractId::now_v7(),
        version: 1,
        name: "qsr-drivethru-template".into(),
        domain: DomainOntology {
            kind: DomainKind::Qsr,
            work_stages: vec![stage("order"), stage("payment"), stage("handoff")],
        },
        targets: vec![
            target(1, "vehicle", "a car at a drive-through")?,
            target(2, "food_bag", "a bag of food")?,
        ],
        zones: vec![zone("lane", &["cam-lane"])],
        rules: vec![dwell_rule(1, "lane", 1, 120_000)?],
        triggers: vec![trigger(1)?],
        webhooks: vec![webhook()],
        kpis: vec![kpi(
            1,
            "lane dwell",
            KpiKind::AvgZoneDwell {
                zone: "lane".into(),
            },
            180.0,
            false,
        )],
    })
}

pub fn retail_warehouse() -> Result<MonitoringContract, ContractError> {
    Ok(MonitoringContract {
        contract_id: ContractId::now_v7(),
        version: 1,
        name: "retail-warehouse-template".into(),
        domain: DomainOntology {
            kind: DomainKind::RetailWarehouse,
            work_stages: vec![stage("restock"), stage("audit")],
        },
        targets: vec![
            target(1, "shelf", "a store shelf with products")?,
            target(2, "cart", "a shopping cart")?,
        ],
        zones: vec![zone("aisle3", &["cam-aisle3"])],
        rules: vec![dwell_rule(1, "aisle3", 2, 30_000)?],
        triggers: vec![trigger(1)?],
        webhooks: vec![webhook()],
        kpis: vec![kpi(
            1,
            "stockout",
            KpiKind::StockoutLatency {
                zone: "aisle3".into(),
                sku: "A1".into(),
            },
            300.0,
            false,
        )],
    })
}

pub fn healthcare_nosocomial() -> Result<MonitoringContract, ContractError> {
    Ok(MonitoringContract {
        contract_id: ContractId::now_v7(),
        version: 1,
        name: "healthcare-fall-risk-template".into(),
        domain: DomainOntology {
            kind: DomainKind::Healthcare,
            work_stages: vec![stage("monitoring"), stage("response")],
        },
        targets: vec![
            target(1, "patient", "a patient standing")?,
            target(2, "staff", "a nurse in scrubs")?,
        ],
        zones: vec![zone("ward4", &["cam-ward4-a", "cam-ward4-b"])],
        rules: vec![dwell_rule(1, "ward4", 1, 10_000)?],
        triggers: vec![trigger(1)?],
        webhooks: vec![webhook()],
        kpis: vec![kpi(
            1,
            "ward occupancy",
            KpiKind::FloorOccupancy {
                zone: "ward4".into(),
            },
            8.0,
            false,
        )],
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn all_templates_reference_own_zones_and_lines() {
        for built in [
            manufacturing_line(),
            qsr_drivethru(),
            retail_warehouse(),
            healthcare_nosocomial(),
        ] {
            let c = built.expect("template fixture must be valid");
            let zone_ids: Vec<String> = c.zones.iter().map(|z| z.zone.0.clone()).collect();
            for r in &c.rules {
                assert!(
                    zone_ids.contains(&r.zone().0),
                    "rule must target an existing zone"
                );
            }
            assert!(c.validate().is_ok());
        }
    }

    #[test]
    fn manufacturing_has_three_rule_variants() {
        let c = manufacturing_line().expect("fixture");
        assert_eq!(c.rules.len(), 3);
    }
}
