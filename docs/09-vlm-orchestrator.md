# 09 — VLM Orchestrator Crate (`vlm-orchestrator`)

The ONLY component allowed to call out to the cloud AI (OpenAI-compatible / Gemini / Claude via endpoint abstraction). Three functions: **morning diffing**, **EOD root-cause localization**, **auto-labeling for nightly fine-tune**.

## 1. Provider abstraction

```rust
#[async_trait]
pub trait VlmProvider: Send + Sync {
    /// Structured-chat with JSON-schema constrained output.
    async fn chat_json(&self, req: VlmRequest<'_>, schema: &JsonSchemaSpec)
        -> Result<serde_json::Value, VlmError>;
    fn name(&self) -> &str;
}

pub struct OpenAiCompatProvider { /* base_url, key, model, timeout 90s, retry 3× exp */ }
pub struct GeminiProvider { /* vertex/gemini URL-scheme */ }
```

- Retries: 3× exponential (1s/4s/16s) with jitter; circuit-breaker open ≥ 5 min on 429/5xx bursts (metrics: `vlm_calls_total{provider,result}`, `vlm_latency_seconds`).
- Never embed secrets in DB; key from env/secret store only (docs/13 §7).

## 2. Morning Layout Diffing (Phase 1 of lifecycle)

**Trigger:** `LifecycleState::Standby → Calibrating` (docs/02 §7).

Prompt design (strictly returns valid JSON):

```json
{
  "task": "factory_layout_diff",
  "today_blocks": [ {"block_id": "r2c3", "ssim_delta": -0.31, "bbox_2d": [..], "crop_b64": "<jpeg>"} ],
  "yesterday_snapshot_b64": "<jpeg>",
  "zones": [ "line 1", "line 2", "staging A" ],
  "expected": {
    "detect_changes": [ { "kind": "moved | removed | added | occluded | irrelevant",
                          "object": "conveyor_belt | pallet | forklift | station | person_blockage | package_stack",
                          "zone_id": null,
                          "blocking_counting_line": true,
                          "suggested_roi_fix": { "zone_id": "line_2", "new_line": [[x1,y1],[x2,y2]] } } ],
    "walkway_blocked": false,
    "new_equipment_needs_roi": [ { "object": "...", "bbox_2d": [..], "proposed_zone_id": "..." } ]
  }
}
```

**Actions parsed from response:**

1. Merge `moved`/`removed` diffs into a `LayoutChangePlan`
2. If valid ROI fix is returned AND it intersects the known floorplan, auto-apply the counting line. Otherwise fall back to "human confirm" flow.
3. Fire webhook `FactoryLayoutChanged` to ERP/MES/Slack (docs/10 §5).
4. Persist new `GoldenBaseline` for the affected cameras (docs/04 §4).

## 3. EOD Root-Cause Localization

Operator submits `HumanAuditFeedback` (e.g. reported 1485 vs actual 1420, window 14:00–16:00 zone `line_4`).

**Root-cause pipeline (Rust order-of-operations):**

1. Query `count_events` in [ts_from, ts_to] zone, sort by (`uncertainty_score`, `modality_divergence`) desc.
2. Diagnose clusters: group events at 1-min granularity; pick top 3 candidate windows (the "culprit episodes").
3. For each window, sample ≤ 12 ambiguous frames (ring-buffer promoted `Amber`), plus detected box snapshots; encode a batch prompt:
    ```json
    { "task": "root_cause_localization",
      "counts": { "predicted": 1485, "actual": 1420 },
      "known_tags": [ "PALLET_AS_BOX" ],
      "windows": [ { "ts": "...", "frame_b64": "...", "pre_boxes": [..] } ] }
    ```
4. VLM returns {"culprit_window": ts-range, "root_cause": "pallets misclassified as boxes", "corrected_boxes": [COCO], "notes": "..."}.
5. Rust applies: (a) adjust `zone_failure_profile` counters used next morning; (b) pass `corrected_boxes` to a Human review queue.

## 4. Auto-Labeling for Nightly Fine-Tune

- Pending items (`LabeledOK = false`) are packaged into **COCO**:
  ```
  { "images": [...file_name, height, width...],
    "annotations": [...bbox, category_id, iscrowd...], "categories": ["package", "pallet"] }
  ```
- Human audit gate (setting): labels auto-accepted when (a) VLM confidence ≥ 0.95, (b) frame `uncertainty` originated from VLM-known root, (c) ≤ 10-day rollout guard → otherwise held for review in dashboard `Review` tab.
- Qdrant upserts: embedding(text root_cause + visual) → `hard_negatives` collection, metadata {zone, ts, model_version_so_far}.

## 5. Nightly Fine-Tune Hot-Swap *(gated `fine_tune` feature)*

Sequence (orchestrated by `api-server` cron at 02:00, implemented here as calls, docs/12 shows the UI):

```rust
async fn nightly_fine_tune(state: &AppState) -> Result<NightlyReport> {
    let dataset = sample_coco(state, budget: 400 imgs, balance hard:tie negatives 3:1).await?;
    // Local fine-tune only in Lite mode (freeze backbone; head/LoRA on GPU edge):
    let new_weights = trainer::head_lora_finetune(&dataset, epochs: 6).await?;
    // Shadow eval then promote happens in inference-engine::try_hot_swap (docs/07 §1)
    shadow_eval_and_promote(new_weights, golden_checkset).await
}
```

- **Fail-safe:** hot-swap with self-test; auto-rollback on failure; always keep previous `.onnx` (2 generations) on disk + in `model_registry` table.

## 6. Rate & Cost Guardrails

- Daily spend cap (config `vlm.daily_usd_cap`, default $15): token/output counter resets 00:00 factory time; when exceeded soft mode keeps morning-diffing and EOD only, and downshifts auto-labeling to a queue that waits for budget.
- Payload cap: only JPEG 640² or ≤ 512 KB per image; never send raw CSI binaries (feature vectors only, 4 KB).
