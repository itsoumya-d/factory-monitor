//! Nightly fine-tune hot-swap orchestration (docs/09 §5), gated behind
//! the `fine_tune` feature. The trainer itself runs off-box (Lite mode:
//! frozen backbone + head/LoRA on the GPU edge); this module owns
//! dataset sampling and the report contract from docs/09 §5.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::error::VlmError;

pub const DATASET_BUDGET_IMAGES: usize = 400;
/// hard : tie negatives sampling ratio (docs/09 §5).
pub const HARD_TO_TIE_RATIO: usize = 3;
pub const FINETUNE_EPOCHS: u32 = 6;

/// First stage of the nightly sequence: sample the COCO dataset down to
/// the budget while keeping every annotation whose image survived.
pub fn sample_coco(coco: &Value, budget: usize) -> Result<Value, VlmError> {
    let images = coco
        .get("images")
        .and_then(Value::as_array)
        .ok_or(VlmError::InvalidPayload("coco missing images".into()))?;
    let annotations = coco
        .get("annotations")
        .and_then(Value::as_array)
        .ok_or(VlmError::InvalidPayload("coco missing annotations".into()))?;
    if images.is_empty() {
        return Err(VlmError::InvalidPayload("coco has no images".into()));
    }
    let take = budget.min(images.len());
    // Deterministic even thinning across the day's hard/tie negatives.
    let stride = images.len() as f64 / take.max(1) as f64;
    let mut keep_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut kept_images = Vec::with_capacity(take);
    for i in 0..take {
        let img = images[(((i as f64) * stride).floor() as usize).min(images.len() - 1)].clone();
        if let Some(id) = img.get("id").and_then(Value::as_i64) {
            keep_ids.insert(id);
        }
        kept_images.push(img);
    }
    let kept_annotations: Vec<Value> = annotations
        .iter()
        .filter(|a| {
            a.get("image_id")
                .and_then(Value::as_i64)
                .is_some_and(|id| keep_ids.contains(&id))
        })
        .cloned()
        .collect();
    Ok(json!({
        "images": kept_images,
        "annotations": kept_annotations,
        "categories": coco.get("categories").cloned().unwrap_or(json!([])),
    }))
}

/// Where the fine-tuned weights landed; promotion itself is
/// `inference-engine::try_hot_swap` (docs/07 §1) — shadow-eval + self-test
/// with auto-rollback are enforced there.
#[derive(Debug, Clone, PartialEq)]
pub struct NightlyReport {
    pub dataset_images: usize,
    pub dataset_annotations: usize,
    pub promoted: bool,
    pub notes: String,
}

#[async_trait]
pub trait FineTuneTrainer: Send + Sync {
    /// Train head/LoRA epochs over the dataset; returns the new weights
    /// path (or object) for shadow evaluation (docs/09 §5).
    async fn finetune(&self, dataset: &Value, epochs: u32) -> Result<String, VlmError>;
}

/// Compose the report after shadow-eval decided promotion.
pub fn build_report(dataset: &Value, promoted: bool, notes: &str) -> NightlyReport {
    NightlyReport {
        dataset_images: dataset
            .get("images")
            .and_then(Value::as_array)
            .map_or(0, |a| a.len()),
        dataset_annotations: dataset
            .get("annotations")
            .and_then(Value::as_array)
            .map_or(0, |a| a.len()),
        promoted,
        notes: notes.to_string(),
    }
}
