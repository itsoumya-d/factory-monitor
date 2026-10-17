//! Morning layout diffing (docs/09 §2). Trigger: `Standby → Calibrating`.
//! Daily SSIM blocks are compared by the VLM; `suggested_roi_fix` answers
//! are auto-applied ONLY when they intersect the known floorplan, otherwise
//! the flow falls back to human confirmation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::VlmError;
use crate::provider::VlmRequest;
use crate::schema;

pub const TASK_LAYOUT_DIFF: &str = "factory_layout_diff";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    Moved,
    Removed,
    Added,
    Occluded,
    Irrelevant,
}

impl DiffKind {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "moved" => Self::Moved,
            "removed" => Self::Removed,
            "added" => Self::Added,
            "occluded" => Self::Occluded,
            "irrelevant" => Self::Irrelevant,
            _ => return None,
        })
    }
}

/// One corridor block that changed between yesterday and today's SSIM
/// pass (docs/04 §4 golden-baseline plumbing feeds this).
#[derive(Debug, Clone, Serialize)]
pub struct MorningBlock {
    pub block_id: String,
    pub ssim_delta: f32,
    pub bbox_2d: [f32; 4],
    pub crop_b64: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoiFix {
    pub zone_id: String,
    /// Two endpoints: `[[x1, y1], [x2, y2]]`.
    pub new_line: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutChange {
    pub kind: DiffKind,
    pub object: String,
    pub zone_id: Option<String>,
    pub blocking_counting_line: bool,
    pub suggested_roi_fix: Option<RoiFix>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewEquipment {
    pub object: String,
    pub bbox_2d: Vec<f32>,
    pub proposed_zone_id: String,
}

/// Current floorplan: counting/parking zones with world-pixel bounds
/// `[x0, y0, x1, y1]`.
#[derive(Debug, Clone, Default)]
pub struct Floorplan {
    zone_bounds: BTreeMap<String, [f32; 4]>,
}

impl Floorplan {
    pub fn new(zone_bounds: BTreeMap<String, [f32; 4]>) -> Self {
        Self { zone_bounds }
    }

    pub fn zone_known(&self, zone: &str) -> bool {
        self.zone_bounds.contains_key(zone)
    }

    /// A suggested counting line intersects the known floorplan when the
    /// zone exists and every endpoint lands inside its bounds (1 % margin).
    pub fn line_intersects(&self, zone: &str, line: &[[f32; 2]]) -> bool {
        let Some(b) = self.zone_bounds.get(zone) else {
            return false;
        };
        if line.len() != 2 {
            return false;
        }
        let mx = (b[2] - b[0]).abs() * 0.01;
        let my = (b[3] - b[1]).abs() * 0.01;
        line.iter().all(|p| {
            p[0] >= b[0] - mx && p[0] <= b[2] + mx && p[1] >= b[1] - my && p[1] <= b[3] + my
        })
    }
}

/// Webhook `FactoryLayoutChanged` payload (docs/10 §5 re-emits this).
#[derive(Debug, Clone, Serialize)]
pub struct LayoutChangePlan {
    pub auto_applied: Vec<RoiFix>,
    pub needs_human: Vec<RoiFix>,
    pub changes: Vec<LayoutChange>,
    pub walkway_blocked: bool,
    pub new_equipment: Vec<NewEquipment>,
}

/// Build the strict-JSON morning prompt of docs/09 §2.
pub fn build_morning_request(
    blocks: &[MorningBlock],
    yesterday_b64: &str,
    zones: &[String],
) -> VlmRequest {
    let today_blocks: Vec<Value> = blocks
        .iter()
        .map(|b| {
            json!({
                "block_id": b.block_id,
                "ssim_delta": b.ssim_delta,
                "bbox_2d": b.bbox_2d,
                "crop_b64": b.crop_b64,
            })
        })
        .collect();
    VlmRequest::new(
        TASK_LAYOUT_DIFF,
        json!({
            "task": TASK_LAYOUT_DIFF,
            "today_blocks": today_blocks,
            "yesterday_snapshot_b64": yesterday_b64,
            "zones": zones,
        }),
    )
}

pub fn expected_schema() -> schema::JsonSchemaSpec {
    schema::layout_diff_schema()
}

fn as_str_or_empty(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or_default().to_string()
}

fn as_bool_or_false(v: Option<&Value>) -> bool {
    v.and_then(Value::as_bool).unwrap_or(false)
}

fn parse_fix(v: &Value) -> Option<RoiFix> {
    let zone = v.get("zone_id")?.as_str()?.to_string();
    let line = v.get("new_line")?.as_array()?;
    let pts: Vec<[f32; 2]> = line
        .iter()
        .map(|p| -> Option<[f32; 2]> {
            let pair = p.as_array()?;
            if pair.len() != 2 {
                return None;
            }
            Some([pair[0].as_f64()? as f32, pair[1].as_f64()? as f32])
        })
        .collect::<Option<Vec<_>>>()?;
    if pts.len() != 2 {
        return None;
    }
    Some(RoiFix {
        zone_id: zone,
        new_line: pts,
    })
}

/// Parse + policy-apply the VLM's `expected` object into a plan. The
/// orchestrator schema-validates upstream; this fn still fails closed on
/// unknown diff kinds (docs/09 §2 parse step).
pub fn parse_layout_response(
    value: &Value,
    floorplan: &Floorplan,
) -> Result<LayoutChangePlan, VlmError> {
    let obj = value
        .as_object()
        .ok_or(VlmError::Schema("layout answer not an object".into()))?;
    let mut plan = LayoutChangePlan {
        auto_applied: Vec::new(),
        needs_human: Vec::new(),
        changes: Vec::new(),
        walkway_blocked: as_bool_or_false(obj.get("walkway_blocked")),
        new_equipment: Vec::new(),
    };

    for item in obj
        .get("detect_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind_raw = item.get("kind").and_then(Value::as_str).unwrap_or_default();
        let Some(kind) = DiffKind::parse(kind_raw) else {
            return Err(VlmError::Schema(format!("unknown diff kind {kind_raw:?}")));
        };
        plan.changes.push(LayoutChange {
            kind,
            object: as_str_or_empty(item.get("object")),
            zone_id: item
                .get("zone_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            blocking_counting_line: as_bool_or_false(item.get("blocking_counting_line")),
            suggested_roi_fix: item.get("suggested_roi_fix").and_then(parse_fix),
        });
    }

    for item in obj
        .get("new_equipment_needs_roi")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        plan.new_equipment.push(NewEquipment {
            object: as_str_or_empty(item.get("object")),
            proposed_zone_id: as_str_or_empty(item.get("proposed_zone_id")),
            bbox_2d: item
                .get("bbox_2d")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_f64)
                        .map(|f| f as f32)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }

    // Action 2 of docs/09 §2: auto-apply iff it intersects the floorplan.
    for change in &plan.changes {
        if let Some(fix) = &change.suggested_roi_fix {
            if floorplan.line_intersects(&fix.zone_id, &fix.new_line) {
                plan.auto_applied.push(fix.clone());
            } else {
                plan.needs_human.push(fix.clone());
            }
        }
    }
    Ok(plan)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn floorplan() -> Floorplan {
        let mut zones = BTreeMap::new();
        zones.insert("line_1".to_string(), [0.0, 0.0, 300.0, 200.0]);
        zones.insert("line_2".to_string(), [300.0, 0.0, 600.0, 200.0]);
        Floorplan::new(zones)
    }

    fn response() -> Value {
        serde_json::json!({
            "detect_changes": [
                {"kind": "moved", "object": "conveyor_belt", "zone_id": "line_1",
                 "blocking_counting_line": true,
                 "suggested_roi_fix": {"zone_id": "line_2", "new_line": [[320.0, 100.0], [580.0, 100.0]]}},
                {"kind": "removed", "object": "pallet", "zone_id": "line_2",
                 "blocking_counting_line": false,
                 "suggested_roi_fix": {"zone_id": "missing_zone", "new_line": [[1.0, 1.0], [2.0, 2.0]]}}
            ],
            "walkway_blocked": true,
            "new_equipment_needs_roi": [
                {"object": "compressor", "bbox_2d": [10.0, 20.0, 30.0, 40.0], "proposed_zone_id": "line_3"}
            ]
        })
    }

    #[test]
    fn request_carries_blocks_zones_and_baseline() {
        let blocks = vec![MorningBlock {
            block_id: "r2c3".into(),
            ssim_delta: -0.31,
            bbox_2d: [10.0, 20.0, 30.0, 40.0],
            crop_b64: "AAAA".into(),
        }];
        let zones = vec!["line 1".to_string(), "staging A".to_string()];
        let req = build_morning_request(&blocks, "BASE64", &zones);
        assert_eq!(req.task, TASK_LAYOUT_DIFF);
        assert_eq!(
            req.body.get("task").and_then(Value::as_str),
            Some("factory_layout_diff")
        );
        assert_eq!(
            req.body
                .get("yesterday_snapshot_b64")
                .and_then(Value::as_str),
            Some("BASE64")
        );
        let blocks_arr = req
            .body
            .get("today_blocks")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("blocks"));
        assert_eq!(blocks_arr.len(), 1);
        assert_eq!(
            blocks_arr[0].get("block_id").and_then(Value::as_str),
            Some("r2c3")
        );
    }

    #[test]
    fn roi_fixes_routed_by_floorplan_intersection() {
        let plan =
            parse_layout_response(&response(), &floorplan()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.changes.len(), 2);
        assert!(plan.walkway_blocked);
        assert!(
            matches!(plan.changes[0].kind, DiffKind::Moved)
                && matches!(plan.changes[1].kind, DiffKind::Removed)
        );
        assert_eq!(plan.auto_applied.len(), 1);
        assert_eq!(plan.auto_applied[0].zone_id, "line_2");
        assert_eq!(plan.needs_human.len(), 1);
        assert_eq!(plan.needs_human[0].zone_id, "missing_zone");
        assert_eq!(plan.new_equipment.len(), 1);
        assert_eq!(plan.new_equipment[0].proposed_zone_id, "line_3");
    }

    #[test]
    fn unknown_diff_kind_fails_closed() {
        let mut bad = response();
        bad["detect_changes"][0]["kind"] = "teleported".into();
        assert!(parse_layout_response(&bad, &floorplan()).is_err());
    }

    #[test]
    fn line_intersects_respects_bounds_and_margin() {
        let fp = floorplan();
        assert!(fp.zone_known("line_1"));
        // Inside bounds.
        assert!(fp.line_intersects("line_1", &[[10.0, 10.0], [290.0, 190.0]]));
        // 1% margin: 603 is exactly 300-wide + 3px margin.
        assert!(fp.line_intersects("line_2", &[[310.0, 10.0], [603.0, 190.0]]));
        // Past the margin fails.
        assert!(!fp.line_intersects("line_2", &[[310.0, 10.0], [605.0, 190.0]]));
        // Far outside fails.
        assert!(!fp.line_intersects("line_2", &[[100.0, 10.0], [900.0, 190.0]]));
        assert!(!fp.line_intersects("ghost_zone", &[[1.0, 1.0], [2.0, 2.0]]));
    }
}
