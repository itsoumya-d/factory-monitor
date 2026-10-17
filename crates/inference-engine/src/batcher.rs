//! Vision tick batching (docs/07 §5): group per-zone line-crossing sums into
//! 200 ms `VisionTick`s for the cross-modal aligner.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::debug;

use common_types::edge_stamp::EdgeStamp;
use common_types::fusion::VisionTick;
use common_types::ids::ZoneId;

/// Default batching horizon (docs/07 §4: "batch 200 ms").
pub const DEFAULT_WINDOW_MS: i64 = 200;

#[derive(Debug, Clone)]
struct OpenBatch {
    ts_from: DateTime<Utc>,
    ts_to: DateTime<Utc>,
    crossings_net: i64,
    conf_sum: f64,
    conf_n: u32,
    occlusion_max: f32,
}

impl OpenBatch {
    fn new(ts: EdgeStamp, delta: i64, mean_conf: Option<f32>, occlusion: f32) -> Self {
        let (ts_from, ts_to) = (ts.utc, ts.utc);
        let mut batch = Self {
            ts_from,
            ts_to,
            crossings_net: 0,
            conf_sum: 0.0,
            conf_n: 0,
            occlusion_max: 0.0,
        };
        batch.absorb(delta, ts.utc, mean_conf, occlusion);
        batch
    }

    fn absorb(&mut self, delta: i64, ts: DateTime<Utc>, mean_conf: Option<f32>, occlusion: f32) {
        self.ts_to = self.ts_to.max(ts);
        self.crossings_net += delta;
        if let Some(c) = mean_conf.filter(|c| c.is_finite()) {
            self.conf_sum += f64::from(c);
            self.conf_n += 1;
        }
        if occlusion.is_finite() {
            self.occlusion_max = self.occlusion_max.max(occlusion.clamp(0.0, 1.0));
        }
    }

    fn into_tick(self) -> VisionTick {
        let mean_conf = if self.conf_n > 0 {
            Some((self.conf_sum / f64::from(self.conf_n)) as f32)
        } else {
            None
        };
        VisionTick {
            ts_from: self.ts_from,
            ts_to: self.ts_to,
            crossings: self.crossings_net,
            mean_box_confidence: mean_conf,
            occlusion_ratio: self.occlusion_max,
        }
    }
}

/// Groups `push`-ed deltas per zone and closes an open batch when the
/// horizon is exceeded or an explicit flush arrives.
pub struct VisionTickBatcher {
    window_ms: i64,
    batches: HashMap<ZoneId, OpenBatch>,
}

impl VisionTickBatcher {
    pub fn new(window_ms: i64) -> Self {
        Self {
            window_ms: window_ms.max(1),
            batches: HashMap::new(),
        }
    }

    pub fn with_default_window() -> Self {
        Self::new(DEFAULT_WINDOW_MS)
    }

    pub fn active_zones(&self) -> Vec<ZoneId> {
        let mut keys: Vec<ZoneId> = self.batches.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Register one frame's crossing delta for a zone. Emits the previous
    /// open batch when the current timestamp exceeds its window.
    pub fn push(
        &mut self,
        zone_id: &ZoneId,
        ts: EdgeStamp,
        delta: i64,
        mean_conf: Option<f32>,
        occlusion_ratio: f32,
    ) -> Option<VisionTick> {
        let prior = match self.batches.get_mut(zone_id) {
            Some(open) if (ts.utc - open.ts_from).num_milliseconds() >= self.window_ms => {
                let closed = open.clone();
                *open = OpenBatch::new(ts, delta, mean_conf, occlusion_ratio);
                Some(closed)
            }
            Some(open) => {
                open.absorb(delta, ts.utc, mean_conf, occlusion_ratio);
                None
            }
            None => {
                let fresh = OpenBatch::new(ts, delta, mean_conf, occlusion_ratio);
                self.batches.insert(zone_id.clone(), fresh);
                None
            }
        };
        prior.map(OpenBatch::into_tick)
    }

    pub fn flush(&mut self, zone_id: &ZoneId) -> Option<VisionTick> {
        self.batches.remove(zone_id).map(OpenBatch::into_tick)
    }

    pub fn flush_all(&mut self) -> Vec<(ZoneId, VisionTick)> {
        let mut out: Vec<(ZoneId, VisionTick)> = self
            .batches
            .drain()
            .map(|(zone, batch)| (zone, batch.into_tick()))
            .collect();
        out.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        debug!(count = out.len(), "vision batcher flushed all zones");
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::Duration;
    use proptest::prelude::*;

    fn stamp_at(ms: i64) -> EdgeStamp {
        EdgeStamp::new(
            DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(ms),
            ms as u64,
            0,
            true,
        )
    }

    fn zone(name: &str) -> ZoneId {
        ZoneId::try_new(name).unwrap()
    }

    #[test]
    fn frames_within_window_group_into_one_tick() {
        let zone = zone("line_1");
        let mut batcher = VisionTickBatcher::with_default_window();
        assert!(
            batcher
                .push(&zone, stamp_at(0), 1, Some(0.8), 0.1)
                .is_none()
        );
        assert!(
            batcher
                .push(&zone, stamp_at(100), 1, Some(0.4), 0.3)
                .is_none()
        );
        let tick = batcher.push(&zone, stamp_at(201), 1, None, 0.2).unwrap();
        assert_eq!(tick.crossings, 2);
        assert!((tick.mean_box_confidence.unwrap() - 0.6).abs() < 1e-6);
        assert!((tick.occlusion_ratio - 0.3).abs() < 1e-6);
        assert_eq!(
            tick.ts_from,
            DateTime::<Utc>::UNIX_EPOCH,
            "ts_from is the batch start"
        );
        assert_eq!(
            tick.ts_to,
            DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(100),
            "closed tick spans its absorbed frames (0..100 ms)"
        );
    }

    #[test]
    fn window_overrun_emits_and_opens_fresh() {
        let zone = zone("line_1");
        let mut batcher = VisionTickBatcher::with_default_window();
        assert!(
            batcher
                .push(&zone, stamp_at(0), 1, Some(0.7), 0.0)
                .is_none()
        );
        let old = batcher
            .push(&zone, stamp_at(250), 2, Some(0.3), 0.0)
            .unwrap();
        assert_eq!(old.crossings, 1);
        assert_eq!(old.mean_box_confidence, Some(0.7));
        let flush = batcher.flush(&zone).unwrap();
        assert_eq!(flush.crossings, 2);
        assert_eq!(
            flush.ts_from,
            DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(250)
        );
    }

    #[test]
    fn zones_are_isolated() {
        let a = zone("a");
        let b = zone("b");
        let mut batcher = VisionTickBatcher::with_default_window();
        assert!(batcher.push(&a, stamp_at(0), 1, None, 0.0).is_none());
        assert!(batcher.push(&b, stamp_at(0), 5, None, 0.0).is_none());
        let fa = batcher.flush(&a).unwrap();
        let fb = batcher.flush(&b).unwrap();
        assert_eq!(fa.crossings, 1);
        assert_eq!(fb.crossings, 5);
        assert!(batcher.flush(&a).is_none());
    }

    #[test]
    fn negative_delta_allowed_for_track_kill_reconcile() {
        let zone = zone("line_2");
        let mut batcher = VisionTickBatcher::with_default_window();
        assert!(batcher.push(&zone, stamp_at(0), -2, None, 0.0).is_none());
        let tick = batcher.flush(&zone).unwrap();
        assert_eq!(tick.crossings, -2);
    }

    #[test]
    fn mean_conf_ignores_non_finite() {
        let zone = zone("line_1");
        let mut batcher = VisionTickBatcher::with_default_window();
        assert!(
            batcher
                .push(&zone, stamp_at(0), 0, Some(f32::NAN), 0.0)
                .is_none()
        );
        assert!(
            batcher
                .push(&zone, stamp_at(50), 0, Some(0.9), 0.0)
                .is_none()
        );
        let tick = batcher.flush(&zone).unwrap();
        assert_eq!(tick.mean_box_confidence, Some(0.9));
    }

    #[test]
    fn outlier_occlusion_clamped() {
        let zone = zone("line_1");
        let mut batcher = VisionTickBatcher::with_default_window();
        assert!(batcher.push(&zone, stamp_at(0), 0, None, 7.0).is_none());
        let tick = batcher.flush(&zone).unwrap();
        assert!((tick.occlusion_ratio - 1.0).abs() < 1e-6);
    }

    #[test]
    fn flush_all_is_deterministic_and_sorted() {
        let a = zone("a");
        let b = zone("b");
        let mut batcher = VisionTickBatcher::with_default_window();
        assert!(batcher.push(&b, stamp_at(0), 1, None, 0.0).is_none());
        assert!(batcher.push(&a, stamp_at(0), 2, None, 0.0).is_none());
        let drained = batcher.flush_all();
        assert_eq!(
            drained
                .iter()
                .map(|(z, _)| z.as_str().to_string())
                .collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(drained[0].1.crossings, 2);
        assert_eq!(drained[1].1.crossings, 1);
        assert!(batcher.batches.is_empty());
    }

    proptest! {
        #[test]
        fn batches_never_shrink_crossings(deltas in proptest::collection::vec(-3i64..=3, 1..40)) {
            let zone = zone("line_1");
            let mut batcher = VisionTickBatcher::with_default_window();
            let mut total = 0i64;
            let mut emitted = 0i64;
            for (i, d) in deltas.iter().enumerate() {
                total += *d;
                if let Some(tick) = batcher.push(&zone, stamp_at((i as i64) * 30), *d, None, 0.0) {
                    emitted += tick.crossings;
                }
            }
            emitted += batcher.flush(&zone).map(|t| t.crossings).unwrap_or(0);
            prop_assert_eq!(total, emitted, "no crossing delta may be lost");
        }
    }
}
