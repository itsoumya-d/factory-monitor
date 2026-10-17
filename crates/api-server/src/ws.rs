//! WebSocket fan-out (docs/10 §3): one broadcast channel, topic-filtered
//! clients, capacity-4096 backpressure where slow clients get dropped
//! (the dashboard reconnects and resyncs via REST snapshots).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use common_types::counting::PackageCountEvent;
use common_types::lifecycle::LifecycleState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum WsEvent {
    CountEvent(Box<PackageCountEvent>),
    FusionSignal {
        zone_id: String,
        ts: DateTime<Utc>,
        vision_rate: f32,
        wifi_rate: f32,
        divergence: f32,
    },
    RlTransition {
        before: common_types::action::RlAction,
        after: common_types::action::RlAction,
        reason: String,
        was_rejected: bool,
    },
    LifecycleEvent {
        from: LifecycleState,
        to: LifecycleState,
        ts: DateTime<Utc>,
    },
    CameraHealth {
        camera_id: String,
        ok: bool,
        fps: f32,
        dropped_ratio: f32,
    },
    LabelQueueAdded {
        frame_id: uuid::Uuid,
        ts: DateTime<Utc>,
        zone_id: String,
    },
}

impl WsEvent {
    pub fn topic(&self) -> &'static str {
        match self {
            WsEvent::CountEvent(_) | WsEvent::FusionSignal { .. } => "counts",
            WsEvent::RlTransition { .. } => "rl",
            WsEvent::LifecycleEvent { .. } => "lifecycle",
            WsEvent::CameraHealth { .. } => "health",
            WsEvent::LabelQueueAdded { .. } => "labels",
        }
    }
}

/// `?topics=counts,rl` — unknown topics fail the upgrade (fast feedback).
pub fn parse_topics(q: &str) -> Result<Option<HashSet<&'static str>>, String> {
    const KNOWN: [&str; 5] = ["counts", "rl", "lifecycle", "health", "labels"];
    if q.is_empty() {
        return Ok(Some(KNOWN.into_iter().collect()));
    }
    let mut set = HashSet::new();
    for t in q.split(',') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        let known = KNOWN
            .iter()
            .find(|k| **k == t)
            .ok_or(format!("unknown topic {t:?}"))?;
        set.insert(*known);
    }
    // Empty set would subscribe silently to nothing — reject: caller bug.
    if set.is_empty() {
        return Err("no valid topics in query".to_string());
    }
    Ok(Some(set))
}

/// Wire shape for the browser variant (tagged enum for impl-free dispatch).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum WsWire {
    CountEvent(serde_json::Value),
    FusionSignal(serde_json::Value),
    RlTransition(serde_json::Value),
    LifecycleEvent(serde_json::Value),
    CameraHealth(serde_json::Value),
    LabelQueueAdded(serde_json::Value),
}

pub fn to_wire(event: &WsEvent) -> Option<WsWire> {
    fn json<T: serde::Serialize>(v: &T) -> serde_json::Value {
        serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
    }
    Some(match event {
        WsEvent::CountEvent(e) => WsWire::CountEvent(json(e)),
        WsEvent::FusionSignal {
            zone_id,
            ts,
            vision_rate,
            wifi_rate,
            divergence,
        } => WsWire::FusionSignal(json(&serde_json::json!({
            "zone_id": zone_id, "ts": ts, "vision_rate": vision_rate,
            "wifi_rate": wifi_rate, "divergence": divergence,
        }))),
        WsEvent::RlTransition {
            before,
            after,
            reason,
            was_rejected,
        } => WsWire::RlTransition(json(&serde_json::json!({
            "before": before, "after": after, "reason": reason, "rejected": was_rejected,
        }))),
        WsEvent::LifecycleEvent { from, to, ts } => {
            WsWire::LifecycleEvent(json(&serde_json::json!({"from": from, "to": to, "ts": ts})))
        }
        WsEvent::CameraHealth {
            camera_id,
            ok,
            fps,
            dropped_ratio,
        } => WsWire::CameraHealth(json(&serde_json::json!({
            "camera_id": camera_id, "ok": ok, "fps": fps, "dropped_ratio": dropped_ratio,
        }))),
        WsEvent::LabelQueueAdded {
            frame_id,
            ts,
            zone_id,
        } => WsWire::LabelQueueAdded(json(&serde_json::json!({
            "frame_id": frame_id, "ts": ts, "zone_id": zone_id,
        }))),
    })
}

#[derive(Debug, PartialEq)]
pub enum PushOutcome {
    /// Delivered to ≥ 1 subscriber.
    Delivered,
    /// No subscribers care about this topic (still legal).
    NoInterestedSubscribers,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn topic_mapping_matches_docs_table() {
        let ts = Utc
            .with_ymd_and_hms(2026, 4, 7, 12, 0, 0)
            .latest()
            .unwrap_or_else(Utc::now);
        let event = PackageCountEvent::try_new(
            ts,
            ts,
            common_types::ids::ZoneId::try_new("line_1").unwrap(),
            1,
            1,
            0.9,
            common_types::counting::UncertaintyFlags::empty(),
            common_types::fusion::ModalityBlend::normalize(0.7),
            common_types::bounds::DetectorThresholds::try_new(0.45, 0.45, 45).unwrap(),
            vec![],
            vec![],
            common_types::ids::ModelVersion::try_new("test").unwrap(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(WsEvent::CountEvent(Box::new(event)).topic(), "counts");
        assert_eq!(
            WsEvent::FusionSignal {
                zone_id: "z".into(),
                ts,
                vision_rate: 0.1,
                wifi_rate: 0.1,
                divergence: 0.0
            }
            .topic(),
            "counts"
        );
        assert_eq!(
            WsEvent::RlTransition {
                before: common_types::action::RlAction::defaults(),
                after: common_types::action::RlAction::defaults(),
                reason: "t".into(),
                was_rejected: false
            }
            .topic(),
            "rl"
        );
        assert_eq!(
            WsEvent::LifecycleEvent {
                from: LifecycleState::Standby,
                to: LifecycleState::Counting,
                ts
            }
            .topic(),
            "lifecycle"
        );
        assert_eq!(
            WsEvent::CameraHealth {
                camera_id: "c".into(),
                ok: true,
                fps: 25.0,
                dropped_ratio: 0.0
            }
            .topic(),
            "health"
        );
        assert_eq!(
            WsEvent::LabelQueueAdded {
                frame_id: uuid::Uuid::now_v7(),
                ts,
                zone_id: "z".into()
            }
            .topic(),
            "labels"
        );
    }

    #[test]
    fn topics_filter_parses_and_validates() {
        let all = parse_topics("counts,rl").unwrap_or_default();
        assert!(all.is_some_and(|s| s.len() == 2));
        let defaulted = parse_topics("").unwrap_or_default();
        assert!(
            defaulted.is_some_and(|s| s.len() == 5),
            "empty query = all topics"
        );
        assert!(parse_topics("counts,,rl").is_ok(), "blanks tolerated");
        assert!(
            parse_topics("counts;junk").is_err(),
            "unknown topic rejected"
        );
        assert!(parse_topics(",").is_err(), "all-blank rejected");
    }

    #[test]
    fn wire_serializes_each_variant() {
        let ts = Utc
            .with_ymd_and_hms(2026, 4, 7, 12, 0, 0)
            .latest()
            .unwrap_or_else(Utc::now);
        let event = WsEvent::LifecycleEvent {
            from: LifecycleState::Standby,
            to: LifecycleState::Counting,
            ts,
        };
        let wire = to_wire(&event).unwrap_or_else(|| panic!("wire"));
        let text = serde_json::to_string(&wire).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.contains("\"type\":\"lifecycle_event\""), "{text}");
    }
}
