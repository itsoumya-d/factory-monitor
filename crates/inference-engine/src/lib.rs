//! Phase-3 brain crate: vision preprocessing, ByteTrack tracking, counting
//! zones/line-crossing math, 200 ms vision tick batching, cross-modal
//! alignment, modality fusion, and the hard-negative ring buffer (docs/07).

pub mod aligner;
pub mod batcher;
pub mod error;
pub mod fusion;
pub mod mock;
pub mod preproc;
pub mod ring;
pub mod tracker;
pub mod zones;

pub use aligner::{
    CrossModalAligner, DEGRADED_TOLERANCE_MS, HEALTHY_TOLERANCE_MS, VisionPacket, tick_distance_ms,
};
pub use batcher::{DEFAULT_WINDOW_MS, VisionTickBatcher};
pub use error::InferenceError;
pub use fusion::{
    EXACT_AGREEMENT_TOL, LOW_STRENGTH_GATE, ModalityFusionEngine, PRESENCE_GATE, STRONG_DIVERGENCE,
    WifiRateModel,
};
pub use mock::ScriptedVisionDetector;
pub use preproc::{DEFAULT_TARGET, LetterboxPlan, ModelFamily, NormalizedTensor, preprocess};
pub use ring::{HardNegativeRecord, HardNegativeRing, NegativePriority, PromotionState};
pub use tracker::{ByteTracker, TrackParams, TrackState, TrackedBox, iou_xyxy};
pub use zones::{
    CountLine, CountableClass, CrossDir, LineCrossCounter, LineCrossOutcome, MaskPolygon,
    TrackMotion, ZoneRois, segments_intersect,
};
