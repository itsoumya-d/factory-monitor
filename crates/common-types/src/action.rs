//! The clamped RL action space (docs/06 §5, docs/08 §2, §6).
//!
//! `RlAction` has no public fields: the ONLY constructors are `clamped()`
//! (soft-projects any input into the safe ranges) and `try_new()` (accepts
//! exactly-valid values). Deserialization routes through `RawActionWire` +
//! `try_from`, so decoded JSON can never carry an out-of-runway action into
//! the system (invariant #4 of docs/06 §7).

use serde::{Deserialize, Serialize};

use crate::bounds::{
    CONF_SAFE_RANGE, CONF_STEP_MAX, DetectorThresholds, FUSION_WEIGHT_RANGE, IOU_SAFE_RANGE,
    TRACK_BUFFER_RANGE, checked_f32, checked_u16, clamp_f32,
};
use crate::error::CommonTypesError;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawActionWire")]
pub struct RlAction {
    conf_threshold: f32,
    iou_threshold: f32,
    vision_weight: f32,
    track_buffer_frames: u16,
}

impl RlAction {
    /// Soft-project a raw policy output into the guardrail envelope.
    pub fn clamped(conf: f32, iou: f32, vision_weight: f32, track_buffer: u16) -> Self {
        let vision_weight = clamp_f32(FUSION_WEIGHT_RANGE, vision_weight);
        Self {
            conf_threshold: clamp_f32(CONF_SAFE_RANGE, conf),
            iou_threshold: clamp_f32(IOU_SAFE_RANGE, iou),
            vision_weight,
            track_buffer_frames: track_buffer
                .clamp(*TRACK_BUFFER_RANGE.start(), *TRACK_BUFFER_RANGE.end()),
        }
    }

    /// Accept a value set that must already be in-range; NaN/out-of-range are
    /// hard errors (used by config defaults and manual overrides).
    pub fn try_new(
        conf_threshold: f32,
        iou_threshold: f32,
        vision_weight: f32,
        track_buffer_frames: u16,
    ) -> Result<Self, CommonTypesError> {
        Ok(Self {
            conf_threshold: checked_f32("conf_threshold", conf_threshold, CONF_SAFE_RANGE)?,
            iou_threshold: checked_f32("iou_threshold", iou_threshold, IOU_SAFE_RANGE)?,
            vision_weight: checked_f32("vision_weight", vision_weight, FUSION_WEIGHT_RANGE)?,
            track_buffer_frames: checked_u16(
                "track_buffer_frames",
                track_buffer_frames,
                TRACK_BUFFER_RANGE,
            )?,
        })
    }

    /// Deterministic cold-start values (docs/08 §8).
    pub fn defaults() -> Self {
        Self::clamped(0.45, 0.45, 0.70, 45)
    }

    pub const fn conf_threshold(&self) -> f32 {
        self.conf_threshold
    }

    pub const fn iou_threshold(&self) -> f32 {
        self.iou_threshold
    }

    pub const fn vision_weight(&self) -> f32 {
        self.vision_weight
    }

    /// `β_wifi = 1 - α_vision`, enforced by construction as the death-guard.
    pub const fn wifi_weight(&self) -> f32 {
        1.0 - self.vision_weight
    }

    pub const fn track_buffer_frames(&self) -> u16 {
        self.track_buffer_frames
    }

    /// Watchdog check (docs/08 §6): would this action's `τ_conf` move farther
    /// than one epoch allowance relative to `prior`?
    pub fn exceeds_conf_step(&self, prior: &Self) -> bool {
        (self.conf_threshold - prior.conf_threshold).abs() > CONF_STEP_MAX
    }

    pub fn as_detector_thresholds(&self) -> DetectorThresholds {
        DetectorThresholds {
            conf_threshold: self.conf_threshold,
            iou_threshold: self.iou_threshold,
            track_buffer_frames: self.track_buffer_frames,
        }
    }
}

impl Default for RlAction {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Mirror of `RlAction` with public fields used ONLY as the serde wire form;
/// the `try_from` bridge keeps decode-time validation mandatory.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RawActionWire {
    pub conf_threshold: f32,
    pub iou_threshold: f32,
    pub vision_weight: f32,
    pub track_buffer_frames: u16,
}

impl TryFrom<RawActionWire> for RlAction {
    type Error = CommonTypesError;

    fn try_from(wire: RawActionWire) -> Result<Self, Self::Error> {
        RlAction::try_new(
            wire.conf_threshold,
            wire.iou_threshold,
            wire.vision_weight,
            wire.track_buffer_frames,
        )
    }
}

impl From<RlAction> for RawActionWire {
    fn from(action: RlAction) -> Self {
        Self {
            conf_threshold: action.conf_threshold(),
            iou_threshold: action.iou_threshold(),
            vision_weight: action.vision_weight(),
            track_buffer_frames: action.track_buffer_frames(),
        }
    }
}
