# 07 — Inference Engine Crate (`inference-engine`)

## 1. Session Management

```rust
pub struct OrtSessionPool {
    vision: Lru<Vec<DetectorEndpoint>>,   // 1 GPU + optional CPU fallback
    wifi:   WifiInferEndpoint,
}

pub struct DetectorEndpoint {
    session: ort::Session,               // pinned to a dedicated thread
    exec_provider: EpKind,               // TensorRT | Cuda | Cpu
    input_layout: TensorLayout,          // [1,3,640,640] fp16
    output_layout: ModelOutLayout,       // raw NMS-free (RF-DETR/YOLO26) or NMS-required
    model_version: ModelVersion,
}
```

- Model swap at inquiry (zero-downtime): `OrtSessionPool::try_hot_swap(new_onnx_bytes, version)` → acquires slot with refcount, does a self-test inference on the golden-check frame, and only then flips the endpoint pointer. On self-test failure → rollback with a logged `ModelRollback` event.

## 2. Pre-/Post-processing (vision)

| Step | Implementation | Notes |
|---|---|---|
| Resize + letterbox | hand-rolled on `ndarray` from `FrameView` (no opencv dependency in hot path) | ASPECT-preserving to 640² using per-model `preproc`, vectorized with `ndarray` AVX |
| Color | BGR→RGB in place | NV12→BGRx already done by gst nvvidconv |
| Normalize | x = (x - mean) * invstd, per channel fp16 | constants per model family |
| Decode boxes | all in `ndarray`: parse heads, apply sigmoid/temperature, filter < τ_conf | RF-DETR / YOLO26 are NMS-FREE (deterministic latency); if a NMS-required model is loaded (legacy), run the simplified IoU-NMS with `lapjv` shortcut on ≤50 boxes |
| **Clamp by RL** | read detector thresholds **from the current action bus** (no blocking); per-frame clone of `RlAction` → thresholds object | docs/08 §5 |

## 3. ByteTrack (custom Rust implementation)

Two-stage association per frame:

```
Detections (score DESC) ──┬──► stage 1: IoU(track_last, det) ≥ 0.5 → matched (Hungarian via lapjv)
                          │        unmatched_high: enter stage 2
                          ├──► stage 2: IoU(track_kf_predict, det) ≥ 0.35 for RE-CONFIRMED tracks
                          │
                          └──► unmatched high-score dets → NEW TRACKS (pending × 2 frames verified)
unmatched low-score dets  ───► stage 3: rescue association ≥ 0.4 with DEAUTHERITIZED zombies
unmatched tracks         ───► keep alive `track_buffer` frames (RL-adjustable 15..90) → then zombie-kill
```

- **Kalman filter:** 8-dim state `[x, y, w, h, vx, vy, vw, vh]`, `nalgebra` SMatrix; `update` per frame; `predict` per frame.
- Re-ID features: **none** (real-time constraint, occlusion handled by fusion instead — a deliberate trade documented here).
- Class confidence smoothing: track's score = EMA of detection scores (α = 0.6).

## 4. Virtual Line-Crossing Counter

```rust
pub struct ZoneROIs {
    pub zone_id: ZoneId,
    pub camera_id: CameraId,
    ///.Counting lines are open polygons; counting fires on segment intersection of
    /// track centroid trajectory between frame k-1 and frame k.
    pub lines: Vec<CountLine>,   // each has direction: A→B == shipped, B→A == discarded
    pub zones: Vec<MaskPolygon>, // for presence context
}
```

Rays: for each track with ≥ 2-frame history and class == package/bar-box (classes:
`Consts::countable`), compute segment; test `seg_intersect(prev_centroid, centroid, line.a,
line.b)`. Crossing → emit `LineCrossed{track, line_id, dir}` → the counter increments `count_delta` of a `PackageCountEvent` batcher (batch 200 ms).

## 5. Modality Fusion Engine

```rust
pub struct ModalityFusionEngine { /* last events per zone, weights from action bus */ }

impl SensorFusionEngine for ModalityFusionEngine {
    fn fuse_tick(&mut self, tick: AlignedTick) -> Vec<PackageCountEvent> {
        // vision path: sum of batched line-crossings within tick (± 100ms)
        // wifi path: cups per zone: presence gate + motion-cadence delta → expected package throughput from a per-zone Wi-Fi throughput model (regressed offline)
        // 1) Exact agreement (|Δvision - Δwifi| ≤ 1): clean count; conf = wα·conf_v + wβ·conf_w
        // 2) Mild divergence (2..=3): take vision count (primary sensor), flag MODALITY_DIVERGENCE, conf *= 0.65
        // 3) Strong divergence (>3) or vision==0 ∧ wifi says ≥1 while occlusion_ratio > 0.5:
        //    use Wi-Fi propagated estimate (occluded interpolation), flag OCCLUDED_ESTIMATE
    }
}
```

- **Key subtlety:** during occlusion the Wi-Fi path is a *rate* estimator (packages/sec), so fusion integrates that rate over the occluded time span and stamps the day's total with a soft reconciation at the next unoccluded frame (drift reconciliation ≤ ±2 boxes, then HUD shows the two values).
- **Clock degraded:** if `clock_ok == false`, fusion marks `CLOCK_DEGRADED` and expands tolerance band to ±300 ms.

## 6. Hard-Negative Ring Buffer (memory)

- Store `(timestamp, camera_id, low-res frame bytes, CSI window bytes, track state, divergenceerture)`,
- Budget: **1 GB** RAM ring, ~200 MB NVMe spill, FIFO by day-part.
- Entries get **promotion states**: `Amber → Uploaded → AutoLabeled → LabeledOK → TrainedEpoch{v}`. The nightly pipeline (docs/09 §6) walks this pipeline.
- Writes are `try_send` only; if the ring is full and the frame has `uncertainty ∉ {OCCLUDED_ESTIMATE}`, it stays full → prioritize: divergence > occlusion > plain low-conf.

## 7. Bench budgets (debug gates, docs/14 §7)

| Operation | Budget p95 |
|---|---|
| preprocess (letterbox + fp16) | ≤ 6 ms |
| ort detect (TensorRT cached engine) | ≤ 18 ms |
| ByteTrack update (300 dets) | ≤ 6 ms |
| crossing math (≤ 12 lines) | ≤ 2 ms |
| fusion tick | ≤ 1 ms |
| **e2e frame→event (per frame, on 4060-class GPU)** | **≤ 40 ms** (≈ 2 cameras @ 30fps steady-state) |
