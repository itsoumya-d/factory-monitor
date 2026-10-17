//! Fusion-layer contracts: the normalized trust pair and the aligned input
//! tick consumed by `SensorFusionEngine` (docs/07 §5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bounds::{FUSION_WEIGHT_RANGE, checked_f32, clamp_f32};
use crate::edge_stamp::EdgeStamp;
use crate::error::CommonTypesError;
use crate::ids::ZoneId;

/// Vision-vs-Wi-Fi trust split with the `α + β = 1.0` invariant enforced by
/// construction (invariant #1 of docs/06 §7).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawBlendWire")]
pub struct ModalityBlend {
    vision: f32,
    wifi: f32,
}

impl ModalityBlend {
    /// Accept an already-normalized, in-range pair (hard validation).
    pub fn new(vision: f32, wifi: f32) -> Result<Self, CommonTypesError> {
        let vision = checked_f32("modality_blend.vision", vision, FUSION_WEIGHT_RANGE)?;
        let wifi = checked_f32("modality_blend.wifi", wifi, FUSION_WEIGHT_RANGE)?;
        if (vision + wifi - 1.0).abs() <= 1.0e-3 {
            Ok(Self { vision, wifi })
        } else {
            Err(CommonTypesError::BlendNotNormalized { vision, wifi })
        }
    }

    /// Project a raw trust scalar into a normalized pair (soft validation):
    /// `vision` is clamped into range, `wifi` becomes the complement.
    pub fn normalize(vision: f32) -> Self {
        let vision = clamp_f32(FUSION_WEIGHT_RANGE, vision);
        Self {
            vision,
            wifi: 1.0 - vision,
        }
    }

    pub const fn vision(&self) -> f32 {
        self.vision
    }

    pub const fn wifi(&self) -> f32 {
        self.wifi
    }

    pub const fn as_tuple(&self) -> (f32, f32) {
        (self.vision, self.wifi)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RawBlendWire {
    pub vision: f32,
    pub wifi: f32,
}

impl TryFrom<RawBlendWire> for ModalityBlend {
    type Error = CommonTypesError;

    fn try_from(wire: RawBlendWire) -> Result<Self, Self::Error> {
        ModalityBlend::new(wire.vision, wire.wifi)
    }
}

impl From<ModalityBlend> for RawBlendWire {
    fn from(blend: ModalityBlend) -> Self {
        Self {
            vision: blend.vision(),
            wifi: blend.wifi(),
        }
    }
}

/// One aligned 100 ms slice handed to the fusion engine (docs/07 §5) by the
/// `CrossModalAligner`: the vision-side batch of line crossings and the
/// Wi-Fi-side feature window for the same zone and instant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignedTick {
    pub ts: EdgeStamp,
    pub zone_id: ZoneId,
    pub vision: VisionTick,
    pub wifi: WifiTick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisionTick {
    pub ts_from: DateTime<Utc>,
    pub ts_to: DateTime<Utc>,
    /// Net line crossings batched inside this tick (+1 per accepted direction).
    pub crossings: i64,
    pub mean_box_confidence: Option<f32>,
    pub occlusion_ratio: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WifiTick {
    pub presence_prob: f32,
    pub doppler_energy: f32,
    pub motion_variance: f32,
    pub packet_loss_ratio: f32,
    pub mover_count_estimate: Option<f64>,
    pub clock_ok: bool,
}

impl WifiTick {
    /// Packet loss above 20% within the window makes CSI testimony unreliable
    /// (healthy LAN target is < 5%, docs/05 §6).
    pub const PACKET_LOSS_DEGRADED: f32 = 0.20;

    pub fn is_csi_degraded(&self) -> bool {
        !self.clock_ok || self.packet_loss_ratio > Self::PACKET_LOSS_DEGRADED
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn normalize_sum_is_exactly_one(vision in -100.0f32..=100.0) {
            let blend = ModalityBlend::normalize(vision);
            prop_assert!((blend.vision() + blend.wifi() - 1.0).abs() <= 1.0e-3);
            prop_assert!(FUSION_WEIGHT_RANGE.contains(&blend.vision()));
            prop_assert!(FUSION_WEIGHT_RANGE.contains(&blend.wifi()));
        }

        #[test]
        fn new_rejects_unnormalized_pairs(vision in -100.0f32..=100.0, wifi in -100.0f32..=100.0) {
            let res = ModalityBlend::new(vision, wifi);
            if (vision + wifi - 1.0).abs() <= 1.0e-3
                && FUSION_WEIGHT_RANGE.contains(&vision)
                && FUSION_WEIGHT_RANGE.contains(&wifi)
            {
                prop_assert!(res.is_ok());
            } else {
                prop_assert!(res.is_err());
            }
        }
    }

    #[test]
    fn normalize_half_is_balanced() {
        let blend = ModalityBlend::normalize(0.5);
        assert_eq!(blend.as_tuple(), (0.5, 0.5));
    }

    #[test]
    fn normalize_clamps_extremes() {
        assert_eq!(ModalityBlend::normalize(5.0).vision(), 0.95);
        assert_eq!(ModalityBlend::normalize(-5.0).vision(), 0.05);
        assert_eq!(ModalityBlend::normalize(f32::NAN).vision(), 0.05);
    }

    #[test]
    fn decode_rejects_junk_weights() {
        let junk = r#"{"vision":0.9,"wifi":0.9}"#;
        assert!(serde_json::from_str::<ModalityBlend>(junk).is_err());
        let valid = r#"{"vision":0.7,"wifi":0.3}"#;
        let decode = serde_json::from_str::<ModalityBlend>(valid).unwrap();
        assert_eq!(decode.as_tuple(), (0.7, 0.3));
    }

    #[test]
    fn csi_degraded_flags_obey_packet_loss_budget() {
        let healthy = WifiTick {
            presence_prob: 1.0,
            doppler_energy: 0.0,
            motion_variance: 0.0,
            packet_loss_ratio: 0.05,
            mover_count_estimate: None,
            clock_ok: true,
        };
        assert!(!healthy.is_csi_degraded());
        let lossy = WifiTick {
            packet_loss_ratio: 0.25,
            ..healthy
        };
        assert!(lossy.is_csi_degraded());
        let clockless = WifiTick {
            packet_loss_ratio: 0.0,
            clock_ok: false,
            ..healthy
        };
        assert!(clockless.is_csi_degraded());
    }
}
