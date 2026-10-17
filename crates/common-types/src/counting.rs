//! `PackageCountEvent` — the single source of counting truth (docs/06 §2).
//!
//! Every produced note carries the thresholds actually used, the fusion blend,
//! uncertainty flags, model version, and lazy frame references, so any count
//! is auditable end-to-end and `count_delta` may be negative on
//! track-kill-reconcile (invariant #5 of docs/06 §7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bounds::DetectorThresholds;
use crate::error::CommonTypesError;
use crate::fusion::ModalityBlend;
use crate::ids::{FrameRef, ModelVersion, TrackId, ZoneId};

/// Per-zone day-total carried alongside the delta (`count_events`.
/// `cumulative_after` in docs/11 §1). Non-monotone by design when
/// track-kill reconcile subtracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CumulativeCount {
    pub cumulative_after: i64,
}

bitflags::bitflags! {
    /// Uncertainty bit mask serializing exactly as the `SMALLINT`
    /// `uncertainty_mask` of `count_events` (docs/11 §1).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct UncertaintyFlags: u16 {
        /// Count was interpolated through an occlusion by the Wi-Fi path.
        const OCCLUDED_ESTIMATE = 1 << 0;
        /// Vision and Wi-Fi disagreed beyond tolerance for this window.
        const MODALITY_DIVERGENCE = 1 << 1;
        /// Edge clock was degraded when this event was produced.
        const CLOCK_DEGRADED = 1 << 2;
        /// ByteTrack association strength dropped below robust levels.
        const LOW_TRACK_STRENGTH = 1 << 3;
    }
}

impl UncertaintyFlags {
    pub const fn to_db_mask(&self) -> i16 {
        self.bits() as i16
    }

    pub const fn from_db_mask(mask: i16) -> Self {
        Self::from_bits_truncate(mask as u16)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageCountEvent {
    pub event_id: Uuid,
    pub zone_id: ZoneId,
    pub ts_from: DateTime<Utc>,
    pub ts_to: DateTime<Utc>,
    pub count_delta: i64,
    #[serde(flatten)]
    pub cumulative: CumulativeCount,
    pub confidence: f32,
    pub uncertainty: UncertaintyFlags,
    pub modality_blend: ModalityBlend,
    pub thresholds_used: DetectorThresholds,
    pub track_ids: Vec<TrackId>,
    pub frame_refs: Vec<FrameRef>,
    pub model_version: ModelVersion,
}

impl PackageCountEvent {
    /// Build + validate in one step; the batcher in `inference-engine` is the
    /// only intended producer.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        ts_from: DateTime<Utc>,
        ts_to: DateTime<Utc>,
        zone_id: ZoneId,
        count_delta: i64,
        cumulative_after: i64,
        confidence: f32,
        uncertainty: UncertaintyFlags,
        modality_blend: ModalityBlend,
        thresholds_used: DetectorThresholds,
        track_ids: Vec<TrackId>,
        frame_refs: Vec<FrameRef>,
        model_version: ModelVersion,
    ) -> Result<Self, CommonTypesError> {
        let event = Self {
            event_id: Uuid::now_v7(),
            zone_id,
            ts_from,
            ts_to,
            count_delta,
            cumulative: CumulativeCount { cumulative_after },
            confidence,
            uncertainty,
            modality_blend,
            thresholds_used,
            track_ids,
            frame_refs,
            model_version,
        };
        event.validate()?;
        Ok(event)
    }

    /// Five-point contract check applied at construction time (and available
    /// for paranoia after deserialization).
    pub fn validate(&self) -> Result<(), CommonTypesError> {
        if !(self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence)) {
            let value = if self.confidence.is_nan() {
                "NaN".to_string()
            } else {
                format!("{}", self.confidence)
            };
            return Err(CommonTypesError::out_of_range(
                "confidence",
                value,
                0.0,
                1.0,
            ));
        }
        if self.ts_to < self.ts_from {
            return Err(CommonTypesError::ReversedTimeWindow {
                ts_from: self.ts_from,
                ts_to: self.ts_to,
            });
        }
        Ok(())
    }

    /// Convenience for the audit UI: flag checks made human-observable.
    pub fn is_occluded_estimate(&self) -> bool {
        self.uncertainty
            .contains(UncertaintyFlags::OCCLUDED_ESTIMATE)
    }

    pub fn is_clock_degraded(&self) -> bool {
        self.uncertainty.contains(UncertaintyFlags::CLOCK_DEGRADED)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::action::RlAction;
    use crate::ids::CameraId;
    use chrono::TimeZone;

    struct EventFixture {
        ts_from: DateTime<Utc>,
        ts_to: DateTime<Utc>,
        zone_id: ZoneId,
        count_delta: i64,
        cumulative_after: i64,
        confidence: f32,
        uncertainty: UncertaintyFlags,
        modality_blend: ModalityBlend,
        thresholds_used: DetectorThresholds,
        track_ids: Vec<TrackId>,
        frame_refs: Vec<FrameRef>,
        model_version: ModelVersion,
    }

    fn fixture(ts_from: DateTime<Utc>, ts_to: DateTime<Utc>) -> EventFixture {
        EventFixture {
            ts_from,
            ts_to,
            zone_id: ZoneId::try_new("line_2").unwrap(),
            count_delta: 2,
            cumulative_after: 1485,
            confidence: 0.92,
            uncertainty: UncertaintyFlags::empty(),
            modality_blend: ModalityBlend::normalize(0.7),
            thresholds_used: RlAction::defaults().as_detector_thresholds(),
            track_ids: vec![TrackId::new(7)],
            frame_refs: vec![],
            model_version: ModelVersion::try_new("rfdetr-base-v3").unwrap(),
        }
    }

    fn assemble(parts: EventFixture) -> Result<PackageCountEvent, CommonTypesError> {
        PackageCountEvent::try_new(
            parts.ts_from,
            parts.ts_to,
            parts.zone_id,
            parts.count_delta,
            parts.cumulative_after,
            parts.confidence,
            parts.uncertainty,
            parts.modality_blend,
            parts.thresholds_used,
            parts.track_ids,
            parts.frame_refs,
            parts.model_version,
        )
    }

    fn build(
        ts_from: DateTime<Utc>,
        ts_to: DateTime<Utc>,
    ) -> Result<PackageCountEvent, CommonTypesError> {
        assemble(fixture(ts_from, ts_to))
    }

    fn ts(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 17, hour, 0, 0).unwrap()
    }

    #[test]
    fn happy_path_event_with_v7_id() {
        let event = build(ts(14), ts(14) + chrono::Duration::milliseconds(100)).unwrap();
        assert_eq!(event.count_delta, 2);
        assert_eq!(event.cumulative.cumulative_after, 1485);
        assert_eq!(event.event_id.to_string().len(), 36);
    }

    #[test]
    fn confidence_out_of_band_rejected() {
        let mut parts = fixture(ts(14), ts(14));
        parts.confidence = 1.2;
        assert!(assemble(parts).is_err());

        let mut nan_parts = fixture(ts(14), ts(14));
        nan_parts.confidence = f32::NAN;
        assert!(assemble(nan_parts).is_err());
    }

    #[test]
    fn reversed_window_rejected() {
        assert!(build(ts(14) + chrono::Duration::seconds(5), ts(14)).is_err());
    }

    #[test]
    fn negative_reconcile_delta_allowed_but_total_still_audited() {
        let event = build(ts(14), ts(14)).unwrap();
        let reconcile = PackageCountEvent::try_new(
            event.ts_from,
            event.ts_to,
            event.zone_id.clone(),
            -1,
            event.cumulative.cumulative_after - 1,
            0.9,
            event.uncertainty,
            event.modality_blend,
            event.thresholds_used,
            event.track_ids.clone(),
            event.frame_refs.clone(),
            event.model_version.clone(),
        )
        .unwrap();
        assert_eq!(reconcile.count_delta, -1);
    }

    #[test]
    fn uncertainty_flags_roundtrip_db_mask() {
        let event = build(ts(14), ts(14)).unwrap();
        let masked = event.uncertainty.to_db_mask();
        let restored = UncertaintyFlags::from_db_mask(masked);
        assert_eq!(restored, event.uncertainty);
    }

    #[test]
    fn serde_roundtrip_stability() {
        let mut parts = fixture(ts(14), ts(14) + chrono::Duration::milliseconds(100));
        parts.count_delta = 3;
        parts.cumulative_after = 1488;
        parts.confidence = 0.87;
        parts.uncertainty = UncertaintyFlags::OCCLUDED_ESTIMATE | UncertaintyFlags::CLOCK_DEGRADED;
        parts.modality_blend = ModalityBlend::normalize(0.6);
        parts.track_ids = vec![TrackId::new(1), TrackId::new(2)];
        parts.frame_refs = vec![FrameRef::new(CameraId::try_new("line2_cam1").unwrap(), 42)];
        let flagged = assemble(parts).unwrap();

        let json = serde_json::to_string(&flagged).unwrap();
        let back: PackageCountEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, flagged);
        assert!(back.is_occluded_estimate());
        assert!(back.is_clock_degraded());
    }
}
