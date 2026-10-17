//! Errors surfaced by the inference-engine crate.

use common_types::{CameraId, ZoneId};

#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    #[error(
        "preproc: frame {camera} buffer mismatch: need {expected} bytes for {width}x{height}, got {actual}"
    )]
    InvalidFrameBuffer {
        camera: CameraId,
        expected: usize,
        actual: usize,
        width: u32,
        height: u32,
    },
    #[error(
        "preproc: frame {camera} geometry too large for the letterbox budget ({width}x{height})"
    )]
    FrameTooLarge {
        camera: CameraId,
        width: u32,
        height: u32,
    },
    #[error("preproc: zero-size clip on frame {camera}")]
    EmptyFrame { camera: CameraId },
    #[error("preproc: target must be at least 1x1 pixels")]
    InvalidTarget,
    #[error("tracker: assignment failure: {0}")]
    Assignment(String),
    #[error("fusion: zone {zone} has no throughput model registered")]
    MissingZoneModel { zone: ZoneId },
    #[error("vision detector: {0}")]
    Detector(String),
}
