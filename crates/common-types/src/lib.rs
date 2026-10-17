//! Shado contract layer.
//!
//! Pure types + traits only: no I/O, no inference, no tokio. Every crate in
//! the workspace depends on this one; it is the authoritative definition of
//! the motion envelope (bounds), counting events, environment state, RL
//! action space, and human audit feedback (see docs/06-core-data-structures.md).
//!
//! Wire-position of safety: every clampable numeric contract is constructed
//! through a validating or clamping constructor; serde deserialization routes
//! through the same validation (`try_from` wire structs), so no unchecked
//! value can enter the system by decoding JSON.

pub mod action;
pub mod bounds;
pub mod counting;
pub mod edge_stamp;
pub mod error;
pub mod feedback;
pub mod frame;
pub mod fusion;
pub mod ids;
pub mod lifecycle;
pub mod state;
pub mod traits;

pub use action::{RawActionWire, RlAction};
pub use bounds::{
    CONF_DEFAULT, CONF_SAFE_RANGE, CONF_STEP_MAX, DetectorThresholds, FP_WEIGHT_RANGE,
    FUSION_WEIGHT_RANGE, IOU_DEFAULT, IOU_SAFE_RANGE, TRACK_BUFFER_DEFAULT, TRACK_BUFFER_RANGE,
    VISION_WEIGHT_DEFAULT, checked_f32, checked_u16, clamp_f32,
};
pub use counting::{CumulativeCount, PackageCountEvent, UncertaintyFlags};
pub use edge_stamp::EdgeStamp;
pub use error::CommonTypesError;
pub use feedback::{FailureTag, HumanAuditFeedback};
pub use frame::{FrameView, PixelFormat};
pub use fusion::{AlignedTick, ModalityBlend, RawBlendWire, VisionTick, WifiTick};
pub use ids::{CameraId, FrameRef, ModelVersion, OperatorId, TrackId, ZoneId};
pub use lifecycle::LifecycleState;
pub use state::{FactoryEnvState, ZoneTopology};
pub use traits::{
    DetectError, DetectOutput, Detection, PolicyAgent, PolicyFeedback, PolicyUpdateReport,
    SensorFusionEngine, VisionDetector,
};
