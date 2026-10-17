//! Runway guardrails — the single source of truth for safe numeric ranges.
//!
//! Defined exactly once (docs/06 §3) and consumed by the RL update path, the
//! manual override form, and every wire decoder. A bad human report can never
//! blind the detector because every construction path funnels through
//! [`checked_f32`] / [`clamp_f32`] below.

use core::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use crate::error::CommonTypesError;

/// Detector confidence gate `τ_conf` (docs/08 §2).
pub const CONF_SAFE_RANGE: RangeInclusive<f32> = 0.25..=0.85;
/// ByteTrack IoU association gate `τ_iou`.
pub const IOU_SAFE_RANGE: RangeInclusive<f32> = 0.30..=0.70;
/// Fusion trust ratio `α_vision` (and by construction `β_wifi = 1 - α`).
pub const FUSION_WEIGHT_RANGE: RangeInclusive<f32> = 0.05..=0.95;
/// ByteTrack zombie-forgetting buffer in units of @30 fps frames (0.5..3 s).
pub const TRACK_BUFFER_RANGE: RangeInclusive<u16> = 15..=90;
/// λ of the false-positive term in the reward function.
pub const FP_WEIGHT_RANGE: RangeInclusive<f32> = 0.5..=3.0;

/// Maximum `τ_conf` movement the watchdog tolerates in a single 30-min epoch
/// (docs/08 §6: ±0.15); larger proposals are rejected and last-safe is kept.
pub const CONF_STEP_MAX: f32 = 0.15;

/// Deterministic cold-start action (docs/08 §8).
pub const CONF_DEFAULT: f32 = 0.45;
pub const IOU_DEFAULT: f32 = 0.45;
pub const VISION_WEIGHT_DEFAULT: f32 = 0.70;
pub const TRACK_BUFFER_DEFAULT: u16 = 45;

/// Validate a finite `f32` against a safe range; `NaN` is always rejected.
pub fn checked_f32(
    field: &'static str,
    value: f32,
    range: RangeInclusive<f32>,
) -> Result<f32, CommonTypesError> {
    if value.is_finite() && range.contains(&value) {
        Ok(value)
    } else {
        Err(CommonTypesError::out_of_range(
            field,
            format_finite(value),
            *range.start(),
            *range.end(),
        ))
    }
}

/// Validate a finite `u16` against a safe range.
pub fn checked_u16(
    field: &'static str,
    value: u16,
    range: RangeInclusive<u16>,
) -> Result<u16, CommonTypesError> {
    if range.contains(&value) {
        Ok(value)
    } else {
        Err(CommonTypesError::out_of_range(
            field,
            value.to_string(),
            f32::from(*range.start()),
            f32::from(*range.end()),
        ))
    }
}

/// Clamp any `f32` (including `NaN`/`±∞`) into a safe range. A `NaN` maps to
/// the conservative (stricter) end of the range so decoding junk never grows
/// detection freedom.
pub fn clamp_f32(range: RangeInclusive<f32>, value: f32) -> f32 {
    if value.is_nan() {
        *range.start()
    } else {
        value.clamp(*range.start(), *range.end())
    }
}

fn format_finite(value: f32) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else {
        format!("{value}")
    }
}

/// Numeric thresholds actually applied by the detector for one event
/// (persisted per `PackageCountEvent` for auditability).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DetectorThresholds {
    pub conf_threshold: f32,
    pub iou_threshold: f32,
    pub track_buffer_frames: u16,
}

impl DetectorThresholds {
    pub fn try_new(
        conf_threshold: f32,
        iou_threshold: f32,
        track_buffer_frames: u16,
    ) -> Result<Self, CommonTypesError> {
        Ok(Self {
            conf_threshold: checked_f32("conf_threshold", conf_threshold, CONF_SAFE_RANGE)?,
            iou_threshold: checked_f32("iou_threshold", iou_threshold, IOU_SAFE_RANGE)?,
            track_buffer_frames: checked_u16(
                "track_buffer_frames",
                track_buffer_frames,
                TRACK_BUFFER_RANGE,
            )?,
        })
    }

    pub fn snapshot(&self) -> Self {
        *self
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    prop_compose! {
        fn any_f32()(
            bits in any::<u32>()
        ) -> f32 {
            f32::from_bits(bits)
        }
    }

    proptest! {
        #[test]
        fn clamp_conf_always_lands_inside_safe_range(raw in any_f32()) {
            let v = clamp_f32(CONF_SAFE_RANGE, raw);
            prop_assert!(CONF_SAFE_RANGE.contains(&v));
        }

        #[test]
        fn clamp_iou_always_lands_inside_safe_range(raw in any_f32()) {
            let v = clamp_f32(IOU_SAFE_RANGE, raw);
            prop_assert!(IOU_SAFE_RANGE.contains(&v));
        }

        #[test]
        fn clamp_blend_always_lands_inside_safe_range(raw in any_f32()) {
            let v = clamp_f32(FUSION_WEIGHT_RANGE, raw);
            prop_assert!(FUSION_WEIGHT_RANGE.contains(&v));
        }

        #[test]
        fn nan_maps_to_stricter_end(raw in any::<f32>().prop_filter("finite only", |v| v.is_finite())) {
            prop_assert_eq!(clamp_f32(CONF_SAFE_RANGE, f32::NAN), *CONF_SAFE_RANGE.start());
            prop_assert_eq!(
                checked_f32("x", raw, CONF_SAFE_RANGE).is_ok(),
                CONF_SAFE_RANGE.contains(&raw)
            );
        }
    }

    #[test]
    fn checked_f32_rejects_out_of_range_and_nan() {
        assert!(checked_f32("conf", 0.45, CONF_SAFE_RANGE).is_ok());
        assert!(checked_f32("conf", 0.90, CONF_SAFE_RANGE).is_err());
        assert!(checked_f32("conf", 0.10, CONF_SAFE_RANGE).is_err());
        assert!(checked_f32("conf", f32::NAN, CONF_SAFE_RANGE).is_err());
        assert!(checked_f32("conf", f32::INFINITY, CONF_SAFE_RANGE).is_err());
    }

    #[test]
    fn checked_u16_rejects_out_of_range() {
        assert!(checked_u16("buffer", 45, TRACK_BUFFER_RANGE).is_ok());
        assert!(checked_u16("buffer", 10, TRACK_BUFFER_RANGE).is_err());
        assert!(checked_u16("buffer", 200, TRACK_BUFFER_RANGE).is_err());
    }

    #[test]
    fn detector_thresholds_try_new_validates() {
        assert!(DetectorThresholds::try_new(0.45, 0.45, 45).is_ok());
        assert!(DetectorThresholds::try_new(0.10, 0.45, 45).is_err());
        assert!(DetectorThresholds::try_new(0.45, 0.90, 45).is_err());
        assert!(DetectorThresholds::try_new(0.45, 0.45, 5).is_err());
    }

    #[test]
    fn thresholds_serde_roundtrip() {
        let t = DetectorThresholds::try_new(0.45, 0.45, 45).unwrap();
        let json = serde_json::to_string(&t).unwrap();
        let back: DetectorThresholds = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }
}
