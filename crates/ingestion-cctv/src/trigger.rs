//! Lights-on / lights-off detection (docs/04 §2).
//!
//! Per-camera hysteresis camera-side trigger plus a `>= 2/3` quorum combine —
//! never rely on a single camera. The dwell/hysteresis pair prevents
//! flicker-flapping (parameters from docs/04: on=25 with 10 s sustain,
//! off=12 with 5-minute dwell).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use common_types::{CameraId, EdgeStamp};
use serde::{Deserialize, Serialize};

pub const LUMA_ON_THRESHOLD: f32 = 25.0;
pub const LUMA_OFF_THRESHOLD: f32 = 12.0;
pub const ON_SUSTAIN: std::time::Duration = std::time::Duration::from_secs(10);
pub const OFF_DWELL: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossDirection {
    On,
    Off,
}

/// Per-camera hysteresis state machine. Feed sampled `lux` values in
/// timestamp order of the ingest loop; returns the crossed direction exactly
/// once at the edge.
#[derive(Debug, Clone, PartialEq)]
pub struct LuminanceTrigger {
    on_threshold: f32,
    off_threshold: f32,
    on_sustain_ms: u64,
    off_dwell_ms: u64,
    lights_on: bool,
    above_since: Option<DateTime<Utc>>,
    below_since: Option<DateTime<Utc>>,
}

impl LuminanceTrigger {
    /// Production defaults (docs/04 §2).
    pub const fn defaults() -> Self {
        Self {
            on_threshold: LUMA_ON_THRESHOLD,
            off_threshold: LUMA_OFF_THRESHOLD,
            on_sustain_ms: ON_SUSTAIN.as_millis() as u64,
            off_dwell_ms: OFF_DWELL.as_millis() as u64,
            lights_on: false,
            above_since: None,
            below_since: None,
        }
    }

    /// Test-tunable variant used by fixtures where dwells are shortened.
    pub const fn custom(
        on_threshold: f32,
        off_threshold: f32,
        on_sustain_ms: u64,
        off_dwell_ms: u64,
    ) -> Self {
        Self {
            on_threshold,
            off_threshold,
            on_sustain_ms,
            off_dwell_ms,
            lights_on: false,
            above_since: None,
            below_since: None,
        }
    }

    #[must_use]
    pub fn feed(&mut self, ts: EdgeStamp, lux: f32) -> Option<CrossDirection> {
        if !lux.is_finite() {
            return None;
        }
        if !self.lights_on {
            if lux >= self.on_threshold {
                let since = *self.above_since.get_or_insert(ts.utc);
                if ms_between(since, ts.utc) >= self.on_sustain_ms {
                    self.lights_on = true;
                    self.below_since = None;
                    return Some(CrossDirection::On);
                }
            } else {
                self.above_since = None;
            }
        } else if lux <= self.off_threshold {
            let since = *self.below_since.get_or_insert(ts.utc);
            if ms_between(since, ts.utc) >= self.off_dwell_ms {
                self.lights_on = false;
                self.above_since = None;
                return Some(CrossDirection::Off);
            }
        } else {
            self.below_since = None;
        }
        None
    }

    pub const fn is_lights_on(&self) -> bool {
        self.lights_on
    }
}

#[inline]
fn ms_between(from: DateTime<Utc>, to: DateTime<Utc>) -> u64 {
    (to - from).num_milliseconds().max(0) as u64
}

/// Combine per-camera crossings with the `>= 2/3` rule (docs/04 §2). Each
/// camera contributes its latest crossing; a system edge fires exactly once
/// per direction, clearing the matching pending votes so repeated votes do
/// not re-fire.
#[derive(Debug, Clone, PartialEq)]
pub struct LuminanceQuorum {
    lights_on: bool,
    pending: HashMap<CameraId, CrossDirection>,
    required: u32,
}

impl LuminanceQuorum {
    pub fn new(total_cameras: u32) -> Self {
        let required = (total_cameras * 2).div_ceil(3);
        Self {
            lights_on: false,
            pending: HashMap::new(),
            required: required.max(1),
        }
    }

    /// Fixed required-vote variant (used by ingestor `quorum_required`).
    pub fn with_total_and_required(total_cameras: u32, required: u32) -> Self {
        Self {
            lights_on: false,
            pending: HashMap::new(),
            required: required.max(1).min(total_cameras.max(1)),
        }
    }

    pub const fn required(&self) -> u32 {
        self.required
    }

    #[must_use]
    pub fn feed(&mut self, camera: CameraId, direction: CrossDirection) -> Option<CrossDirection> {
        self.pending.insert(camera, direction);
        let count =
            |dir: CrossDirection| self.pending.values().filter(|d| **d == dir).count() as u32;
        if !self.lights_on && count(CrossDirection::On) >= self.required {
            self.pending.clear();
            self.lights_on = true;
            return Some(CrossDirection::On);
        }
        if self.lights_on && count(CrossDirection::Off) >= self.required {
            self.pending.clear();
            self.lights_on = false;
            return Some(CrossDirection::Off);
        }
        None
    }

    pub const fn is_lights_on(&self) -> bool {
        self.lights_on
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::TimeZone;

    fn ts(seconds: i64) -> EdgeStamp {
        let utc = Utc.with_ymd_and_hms(2026, 10, 17, 6, 0, 0).unwrap()
            + chrono::Duration::seconds(seconds);
        EdgeStamp::new(utc, 0, 0, true)
    }

    #[test]
    fn on_requires_sustained_above() {
        let mut trig = LuminanceTrigger::custom(25.0, 12.0, 10_000, 300_000);
        assert_eq!(trig.feed(ts(0), 30.0_f32), None);
        assert_eq!(trig.feed(ts(5), 40.0_f32), None);
        assert!(!trig.is_lights_on());
        assert_eq!(trig.feed(ts(10), 30.0_f32), Some(CrossDirection::On));
        assert!(trig.is_lights_on());
    }

    #[test]
    fn dipped_below_on_resets_sustain() {
        let mut trig = LuminanceTrigger::custom(25.0, 12.0, 10_000, 300_000);
        assert_eq!(trig.feed(ts(0), 30.0_f32), None);
        assert_eq!(trig.feed(ts(5), 10.0_f32), None);
        assert_eq!(trig.feed(ts(20), 30.0_f32), None);
        assert!(!trig.is_lights_on(), "dip must restart the 10 s sustain");
        assert_eq!(trig.feed(ts(31), 30.0_f32), Some(CrossDirection::On));
    }

    #[test]
    fn off_requires_dwell() {
        let mut trig = LuminanceTrigger::custom(25.0, 12.0, 10_000, 300_000);
        assert_eq!(trig.feed(ts(0), 30.0_f32), None);
        assert_eq!(trig.feed(ts(10), 30.0_f32), Some(CrossDirection::On));
        assert_eq!(trig.feed(ts(300), 8.0_f32), None);
        assert_eq!(trig.feed(ts(600), 8.0_f32), Some(CrossDirection::Off));
        assert!(!trig.is_lights_on());
    }

    #[test]
    fn flicker_above_off_resets_dwell_start() {
        let mut trig = LuminanceTrigger::custom(25.0, 12.0, 10_000, 300_000);
        assert_eq!(trig.feed(ts(0), 30.0_f32), None);
        assert_eq!(trig.feed(ts(10), 30.0_f32), Some(CrossDirection::On));
        assert_eq!(trig.feed(ts(100), 8.0_f32), None);
        assert_eq!(trig.feed(ts(120), 50.0_f32), None);
        assert_eq!(
            trig.feed(ts(400), 8.0_f32),
            None,
            "off dwell restarted at 120s"
        );
        assert_eq!(trig.feed(ts(499), 8.0_f32), None, "99s of the 300s dwell");
        assert_eq!(trig.feed(ts(801), 8.0_f32), Some(CrossDirection::Off));
    }

    #[test]
    fn nan_samples_are_ignored() {
        let mut trig = LuminanceTrigger::custom(25.0, 12.0, 10_000, 300_000);
        assert_eq!(trig.feed(ts(0), f32::NAN), None);
        assert_eq!(trig.feed(ts(15), 30.0_f32), None);
        assert_eq!(trig.feed(ts(25), 30.0_f32), Some(CrossDirection::On));
    }

    #[test]
    fn quorum_requires_two_thirds() {
        let mut quorum = LuminanceQuorum::new(3);
        assert_eq!(quorum.required(), 2);
        let cam = CameraId::try_new("cam").unwrap();
        let cam2 = CameraId::try_new("cam2").unwrap();
        let cam3 = CameraId::try_new("cam3").unwrap();
        assert_eq!(quorum.feed(cam, CrossDirection::On), None);
        assert_eq!(
            quorum.feed(cam2.clone(), CrossDirection::On),
            Some(CrossDirection::On)
        );
        assert_eq!(
            quorum.feed(cam3.clone(), CrossDirection::On),
            None,
            "already lit"
        );
        assert_eq!(
            quorum.feed(cam3, CrossDirection::Off),
            None,
            "one off vote after light remains quorum-lit"
        );
        assert_eq!(
            quorum.feed(cam2, CrossDirection::Off),
            Some(CrossDirection::Off)
        );
    }

    #[test]
    fn quorum_never_floats_below_one() {
        let mut quorum = LuminanceQuorum::new(1);
        assert_eq!(quorum.required(), 1);
        let cam = CameraId::try_new("cam").unwrap();
        assert_eq!(
            quorum.feed(cam, CrossDirection::On),
            Some(CrossDirection::On)
        );
    }
}
