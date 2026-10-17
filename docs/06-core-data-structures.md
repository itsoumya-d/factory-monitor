# 06 — Core Data Structures & Traits (`common-types` crate)

This crate is the **contract layer**: no I/O, no inference — only pure types + traits. Every other crate depends on it. It is the first file set written and the last allowed to change without a workspace-wide re-review.

## 1. `FactoryEnvState` — the RL state vector

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryEnvState {
    pub ts: EdgeStamp,
    /// Per-camera luminance proxy in [0..255] of BT.709 luma mean.
    pub lux_proxy: Vec<f32>,
    /// 0..1 blur/variance-of-laplassian degradation score per camera.
    pub blur_score: Vec<f32>,
    /// 0..1 sanitized residual CSI variability (environmental noise floor).
    pub csi_variance: f32,
    /// Sub-second occlusion: fraction of zones where vision is behind an occluder.
    pub occlusion_ratio: f32,
    /// Current detector confidence threshold τ_conf (state reveals to agent).
    pub conf_threshold: f32,
    pub iou_threshold: f32,
    pub vision_weight: f32,
    pub wifi_weight: f32,
    /// Rolling 30-min auto-detected scene complexity score (0..1) from VLM/box entropy.
    pub scene_complexity: f32,
    pub lifecycle: LifecycleState,
    pub zones: Vec<ZoneTopology>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleState { Standby, Calibrating, Counting, Correcting, Degraded }
```

## 2. `PackageCountEvent` — the single source of counting truth

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageCountEvent {
    pub event_id: Uuid,          // v7, time-sortable
    pub zone_id: ZoneId,         // "line_2"
    pub ts_from: EdgeStamp,
    pub ts_to: EdgeStamp,
    pub count_delta: i64,        // +1 per crossing (can be -1 on BoundaryBanish)
    #[serde(flatten)] pub cumulative: CumulativeCount, // per zone day total
    pub confidence: f32,         // ≤1
    pub uncertainty: UncertaintyFlags, // bitflags: OCCLUDED_ESTIMATE | MODALITY_DIVERGENCE | CLOCK_DEGRADED | LOW_TRACK_STRENGTH
    pub modality_blend: ModalityBlend, // vision vs wifi weights actually used
    pub thresholds_used: DetectorThresholds,
    pub track_ids: Vec<TrackId>, // ByteTrack ids that caused this crossing
    pub frame_refs: Vec<FrameRef>, // ring buffer handles, resolved lazily
    pub model_version: ModelVersion,
}

pub struct ModalityBlend { pub vision: f32, pub wifi: f32 } // norm must sum to 1.0
```

## 3. `DetectorThresholds` + guardrail types

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DetectorThresholds {
    pub conf_threshold: f32,   // hard clamp [0.25, 0.85]
    pub iou_threshold: f32,    // hard clamp [0.30, 0.70]
    pub track_buffer_frames: u16, // ByteTrack forgetting time, clamp [15, 90] @30tps = 0.5..3 s
}

pub const CONF_SAFE_RANGE: RangeInclusive<f32> = 0.25_f32..=0.85;
pub const IOU_SAFE_RANGE: RangeInclusive<f32>  = 0.30_f32..=0.70;
pub const FUSION_WEIGHT_RANGE: RangeInclusive<f32> = 0.05_f32..=0.95;
```

> **Rule:** these ranges are defined ONE place (`common-types::bounds`) and consumed by both the RL update and the dashboard's manual override form.

## 4. `HumanAuditFeedback` — the EOD correction payload

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanAuditFeedback {
    pub correction_id: Uuid,
    pub zone_id: ZoneId,
    pub reported_count: i64,   // what the system said
    pub actual_count: i64,     // what the operator says is true
    pub ts_from: DateTime<Utc>,// ISO; dashboards send "14:00-16:00"
    pub ts_to: DateTime<Utc>,
    pub failure_tags: BitFlags<FailureTag>, // MISSED_COUNT | DOUBLE_COUNT | PALLET_AS_BOX | OTHER
    pub notes: Option<String>,
    pub operator_id: OperatorId,
    pub wants_retrain: bool,   // explicit opt-in for heavyweight VLM/nightly loop
}
```

## 5. `RlAction` / traits

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RlAction {
    pub conf_threshold: f32,
    pub iou_threshold: f32,
    pub vision_weight: f32,
    pub wifi_weight: f32,      // = 1 - vision_weight, kept as death-guard
    pub track_buffer: u16,
}

#[async_trait]  // deliberate: policy may hit local Qdrant for state-retrieval
pub trait PolicyAgent: Send {
    fn name(&self) -> &str;
    async fn select_action(&mut self, state: &FactoryEnvState) -> RlAction;
    /// Called on every PackageCountEvent (small, immediate signal) and on
    /// HumanAuditFeedback (big, deferred signal). Implementations own
    /// internal reward attribution (bandit vs PPO schedule).
    async fn update_policy(&mut self, feedback: PolicyFeedback) -> Result<PolicyUpdateReport, anyhow::Error>;
    async fn snapshot_weights(&self) -> Vec<u8>;      // for hot-reload & rollback
    async fn restore_weights(&mut self, blob: &[u8]) -> Result<(), anyhow::Error>;
}

pub enum PolicyFeedback {
    EventSignal { event: PackageCountEvent, immediate: f32 },   // fast intra-day loop
    HumanAudit { feedback: HumanAuditFeedback, computed_reward: f32 },
}
```

## 6. Sensor traits

```rust
#[async_trait]
pub trait VisionDetector: Send {
    fn version(&self) -> ModelVersion;
    async fn detect(&mut self, frame: &FrameView, thresholds: DetectorThresholds)
        -> DetectOutput;   // boxes+scores+class, all in GPU-resident layout when possible
    async fn warm(&mut self) -> Result<(), DetectError>;    // morning pre-warm
}

#[async_trait]
pub trait CsiSource: Send { /* … as in docs/05 §2 */ }

pub trait SensorFusionEngine: Send {
    /// Given aligned per-tick slices of frame-detections and CSI features,
    /// emit fused events. Implemented in inference-engine.
    fn fuse_tick(&mut self, tick: AlignedTick) -> Vec<PackageCountEvent>;
}
```

## 7. Invariants (workspace-wide, checked in unit tests and `#[warn-]` tests in CI)

1. `ModalityBlend{ vision + wifi } == 1.0 ± 1e-3` (normalization is infallible by construction).
2. Every emitted `PackageCountEvent` persists **unexported until DB ack** (api-server is the sole persisting writer).
3. `EdgeStamp::clock_ok == false` ⇒ `uncertainty.clocks := CLOCK_DEGRADED` MUST be set.
4. `RlAction` values are ALWAYS inside `bounds::SAFE_*` — enforced by `RlAction::clamped()`, constructors private.
5. `PackageCountEvent::count_delta` may be negative (track-kill reconcile) → DB reader must not assume monotonicity; enforce per-zone day-total is recomputed from accumulated events, not stored as a single running int.
