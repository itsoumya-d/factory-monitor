//! Auto-labeling for the nightly fine-tune (docs/09 §4): package pending
//! items into COCO, then run the human-audit gate — auto-accept only when
//! the VLM was confident, the frame's uncertainty came from a known
//! root-cause family, and we are inside the 10-day rollout guard.

use serde_json::{Value, json};

pub const AUTO_ACCEPT_MIN_CONFIDENCE: f32 = 0.95;
pub const ROLLOUT_GUARD_DAYS: u32 = 10;

/// One pending pen-labeled candidate produced by EOD root-cause runs or
/// the ring buffer's Amber promotions.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelItem {
    pub file_name: String,
    pub height: u32,
    pub width: u32,
    pub bbox: [f64; 4],
    pub category: String,
    pub confidence: f32,
    /// Was this frame's uncertainty attributed to a root cause the VLM
    /// already knows (docs/09 §4 acceptance rule b)?
    pub from_known_root: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelDecision {
    /// (a) conf ≥ 0.95, (b) known root, (c) ≤ 10-day rollout guard.
    AutoAccept,
    HoldReview,
}

pub fn acceptance_gate(
    confidence: f32,
    from_known_root: bool,
    days_since_rollout: u32,
) -> LabelDecision {
    if confidence >= AUTO_ACCEPT_MIN_CONFIDENCE
        && from_known_root
        && days_since_rollout <= ROLLOUT_GUARD_DAYS
    {
        LabelDecision::AutoAccept
    } else {
        LabelDecision::HoldReview
    }
}

/// COCO export shape from docs/09 §4.
#[derive(Debug, Clone, PartialEq)]
pub struct CocoDataset {
    pub images: Vec<Value>,
    pub annotations: Vec<Value>,
    /// Owned 1-indexed taxonomy; index i is category_id i+1.
    pub categories: Vec<String>,
}

/// Package `items` into the docs/09 §4 COCO skeleton. Categories not in
/// `known_categories` are skipped (fail-closed taxonomy).
pub fn package_coco(items: &[LabelItem], known_categories: &[String]) -> CocoDataset {
    let mut images = Vec::new();
    let mut annotations = Vec::new();
    let mut image_id = 1u64;
    let mut ann_id = 1u64;
    for item in items {
        let Some(category_id) = known_categories
            .iter()
            .position(|c| c.as_str() == item.category)
            .map(|i| (i + 1) as u64)
        else {
            continue;
        };
        images.push(json!({
            "id": image_id,
            "file_name": item.file_name,
            "height": item.height,
            "width": item.width,
        }));
        annotations.push(json!({
            "id": ann_id,
            "image_id": image_id,
            "category_id": category_id,
            "bbox": item.bbox,
            "area": item.bbox[2] * item.bbox[3],
            "iscrowd": 0,
        }));
        image_id += 1;
        ann_id += 1;
    }
    CocoDataset {
        images,
        annotations,
        categories: known_categories.to_vec(),
    }
}

impl CocoDataset {
    pub fn to_json(&self) -> Value {
        json!({
            "images": self.images,
            "annotations": self.annotations,
            "categories": self.categories,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn item(category: &str, confidence: f32, from_known_root: bool) -> LabelItem {
        LabelItem {
            file_name: "cam_a_372.jpg".into(),
            height: 720,
            width: 1280,
            bbox: [10.0, 20.0, 30.0, 40.0],
            category: category.into(),
            confidence,
            from_known_root,
        }
    }

    #[test]
    fn gate_boundaries_0_95_and_10_days() {
        assert_eq!(
            acceptance_gate(0.95, true, 10),
            LabelDecision::AutoAccept,
            "rule edges are inclusive"
        );
        assert_eq!(acceptance_gate(0.94, true, 10), LabelDecision::HoldReview);
        assert_eq!(acceptance_gate(0.99, false, 1), LabelDecision::HoldReview);
        assert_eq!(acceptance_gate(0.99, true, 11), LabelDecision::HoldReview);
    }

    #[test]
    fn coco_packaging_skips_unknown_categories_and_ids_sequences() {
        let items = vec![
            item("pallet", 0.97, true),
            item("ufo", 0.99, true),
            item("package", 0.96, false),
        ];
        let coco = package_coco(&items, &["package".into(), "pallet".into()]);
        assert_eq!(coco.images.len(), 2, "ufo not in taxonomy");
        assert_eq!(coco.annotations.len(), 2);
        // category_id is 1-based over the taxonomy list.
        let first = &coco.annotations[0];
        assert_eq!(first.get("category_id").and_then(Value::as_i64), Some(2));
        assert_eq!(first.get("iscrowd").and_then(Value::as_i64), Some(0));
        assert_eq!(first.get("area").and_then(Value::as_f64), Some(1200.0));
        assert_eq!(coco.images[0].get("id").and_then(Value::as_i64), Some(1));
        assert_eq!(coco.images[1].get("id").and_then(Value::as_i64), Some(2));
        let json = coco.to_json();
        let cats = json
            .get("categories")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("categories"))
            .len();
        assert_eq!(cats, 2);
    }
}
