//! Shared trait surface: the RL policy agent, the vision detector plug-in,
//! and the sensor-fusion engine (docs/06 §5, docs/08 §7).
//!
//! `PolicyAgent`/`VisionDetector` are object-safe `async_trait` traits so the
//! api-server can interpose mocks (docs/14 §2: sources + models are faked in
//! CI) without monomorphizing implementations across wiring code.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::action::RlAction;
use crate::bounds::DetectorThresholds;
use crate::counting::PackageCountEvent;
use crate::feedback::HumanAuditFeedback;
use crate::frame::FrameView;
use crate::fusion::AlignedTick;
use crate::ids::ModelVersion;
use crate::state::FactoryEnvState;

#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("detector session is not warmed for a morning boot")]
    NotWarmed,
    #[error("detection backend failure: {0}")]
    Backend(String),
}

/// One per-object detection result, pre-tracking layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub bbox_xyxy: [f32; 4],
    pub score: f32,
    pub class_id: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DetectOutput {
    pub detections: Vec<Detection>,
    /// Wall time of the inference call, for per-frame budget accounting.
    pub elapsed_ms: f32,
}

/// Tier-1 edge detector abstraction (`ort`/mock switchable, docs/03 §2).
#[async_trait]
pub trait VisionDetector: Send {
    fn version(&self) -> ModelVersion;

    async fn detect(
        &mut self,
        frame: &FrameView,
        thresholds: DetectorThresholds,
    ) -> Result<DetectOutput, DetectError>;

    /// Pre-warm for the morning boot (warms kernels, caches TensorRT plans).
    async fn warm(&mut self) -> Result<(), DetectError>;
}

/// The two routes that can drive a policy update (docs/06 §5): the small
/// intra-day signal per counted event, and the large deferred EOD audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyFeedback {
    EventSignal {
        event: PackageCountEvent,
        immediate: f32,
    },
    HumanAudit {
        feedback: HumanAuditFeedback,
        computed_reward: f32,
    },
}

/// The RL loop interface, deliberately tiny (docs/08): only thresholds and
/// fusion weights are ever adjusted; the agent never touches counts, models,
/// or writes data downstream.
#[async_trait]
pub trait PolicyAgent: Send {
    fn name(&self) -> &str;
    async fn select_action(&mut self, state: &FactoryEnvState) -> RlAction;
    async fn update_policy(
        &mut self,
        feedback: &PolicyFeedback,
    ) -> Result<PolicyUpdateReport, anyhow::Error>;
    async fn snapshot_weights(&self) -> Vec<u8>;
    async fn restore_weights(&mut self, blob: &[u8]) -> Result<(), anyhow::Error>;
}

/// Report surfaced on the dashboard + `rl_audit` row after each update
/// (docs/08 §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyUpdateReport {
    pub new_action: RlAction,
    pub was_clamped: bool,
    pub attribution_steps: u16,
    pub next_exploration_window: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

/// The multi-modal decision engine (docs/07 §5).
pub trait SensorFusionEngine: Send {
    fn fuse_tick(&mut self, tick: AlignedTick) -> Vec<PackageCountEvent>;
}
