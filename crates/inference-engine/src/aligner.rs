//! Cross-modal aligner (docs/07 §5): pair per-zone `VisionTick`s with the
//! temporally-nearest `WifiTick`. Pairing tolerance is ±100 ms on healthy
//! clocks and ±300 ms when either side reports a degraded clock.

use std::collections::{HashMap, VecDeque};

use common_types::edge_stamp::EdgeStamp;
use common_types::fusion::{AlignedTick, VisionTick, WifiTick};
use common_types::ids::ZoneId;

pub const HEALTHY_TOLERANCE_MS: i64 = 100;
pub const DEGRADED_TOLERANCE_MS: i64 = 300;

/// Vision-side buffered pool depth: two ticks ≈ 200 ms of crossings.
const MAX_BUFFERED_VISION: usize = 2;

#[derive(Debug, Clone)]
pub struct VisionPacket {
    pub zone_id: ZoneId,
    pub ts: EdgeStamp,
    pub tick: VisionTick,
}

#[derive(Debug, Clone)]
struct BufferedVision {
    ts: EdgeStamp,
    tick: VisionTick,
}

#[derive(Debug, Clone)]
struct CachedWifi {
    ts: EdgeStamp,
    tick: WifiTick,
}

pub fn tick_distance_ms(a: &EdgeStamp, b: &EdgeStamp) -> i64 {
    (a.utc - b.utc).num_milliseconds().abs()
}

/// Tolerance of a candidate pair: ±100 ms only when *both* stamps (and the
/// Wi-Fi clock_ok bit) are healthy; ±300 ms when either side is degraded.
fn pair_tol_ms(
    vision_ts: &EdgeStamp,
    vision_clock_ok: bool,
    wifi_ts: &EdgeStamp,
    wifi_clock_ok: bool,
) -> i64 {
    if vision_ts.clock_ok && vision_clock_ok && wifi_ts.clock_ok && wifi_clock_ok {
        HEALTHY_TOLERANCE_MS
    } else {
        DEGRADED_TOLERANCE_MS
    }
}

/// Index of the buffered vision tick closest to the incoming Wi-Fi stamp,
/// pairing only inside that candidate's own tolerance.
fn select_closest(
    queue: &VecDeque<BufferedVision>,
    wifi_ts: &EdgeStamp,
    wifi_clock_ok: bool,
) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (i, buffered) in queue.iter().enumerate() {
        let tol = pair_tol_ms(&buffered.ts, true, wifi_ts, wifi_clock_ok);
        let delta = tick_distance_ms(wifi_ts, &buffered.ts);
        if delta <= tol && best.is_none_or(|(_, best_delta)| delta < best_delta) {
            best = Some((i, delta));
        }
    }
    best.map(|(i, _)| i)
}

/// Align vision and CSI ticks per zone. A new vision tick pairs against the
/// cached Wi-Fi tick when inside tolerance; otherwise it buffers (max 2 per
/// zone). A new Wi-Fi tick pairs against the closest buffered vision tick;
/// otherwise it caches (latest wins). Both sides are consumed exactly once
/// by a successful pairing — crossings are never dropped inside the window.
pub struct CrossModalAligner {
    vision: HashMap<ZoneId, VecDeque<BufferedVision>>,
    wifi: HashMap<ZoneId, CachedWifi>,
}

impl CrossModalAligner {
    pub fn new() -> Self {
        Self {
            vision: HashMap::new(),
            wifi: HashMap::new(),
        }
    }

    /// A new vision tick arrives: pair it with the freshly cached Wi-Fi tick
    /// (if any), else buffer it (cap 2, oldest evicted). The cached Wi-Fi
    /// tick survives failed pairings — it is consumed only on success.
    pub fn push_vision(&mut self, packet: VisionPacket) -> Option<AlignedTick> {
        let VisionPacket { zone_id, ts, tick } = packet;
        if let Some(cached) = self.wifi.get(&zone_id) {
            let CachedWifi {
                ts: wifi_ts,
                tick: wifi_tick,
            } = cached;
            let tol = pair_tol_ms(&ts, true, wifi_ts, wifi_tick.clock_ok);
            if tick_distance_ms(&ts, wifi_ts) <= tol {
                let CachedWifi {
                    tick: wifi_tick,
                    ts: _,
                } = self.wifi.remove(&zone_id)?;
                if let Some(queue) = self.vision.get_mut(&zone_id) {
                    if queue.is_empty() {
                        self.vision.remove(&zone_id);
                    }
                }
                return Some(AlignedTick {
                    ts,
                    zone_id,
                    vision: tick,
                    wifi: wifi_tick,
                });
            }
        }
        let queue = self.vision.entry(zone_id).or_default();
        queue.push_back(BufferedVision { ts, tick });
        while queue.len() > MAX_BUFFERED_VISION {
            queue.pop_front();
        }
        None
    }

    /// A new Wi-Fi tick arrives: pair it with the closest buffered vision
    /// tick (if any), else cache it (latest wins).
    pub fn push_wifi(
        &mut self,
        zone_id: &ZoneId,
        ts: EdgeStamp,
        tick: WifiTick,
    ) -> Option<AlignedTick> {
        let maybe_idx = self
            .vision
            .get_mut(zone_id)
            .and_then(|queue| select_closest(queue, &ts, tick.clock_ok));
        let Some(idx) = maybe_idx else {
            self.wifi.insert(zone_id.clone(), CachedWifi { ts, tick });
            return None;
        };
        let pinned = self.vision.get_mut(zone_id).and_then(|q| q.remove(idx))?;
        if self
            .vision
            .get(zone_id)
            .is_some_and(|queue| queue.is_empty())
        {
            self.vision.remove(zone_id);
        }
        Some(AlignedTick {
            ts: pinned.ts,
            zone_id: zone_id.clone(),
            vision: pinned.tick,
            wifi: tick,
        })
    }

    pub fn buffered_vision(&self, zone_id: &ZoneId) -> usize {
        self.vision.get(zone_id).map_or(0, VecDeque::len)
    }

    pub fn pending_zones(&self) -> Vec<ZoneId> {
        let mut keys: Vec<ZoneId> = self
            .vision
            .iter()
            .filter(|(_, q)| !q.is_empty())
            .map(|(z, _)| z.clone())
            .collect();
        keys.sort();
        keys
    }
}

impl Default for CrossModalAligner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};
    use common_types::fusion::WifiTick;

    fn stamp_at(base: DateTime<Utc>, offset_ms: i64) -> EdgeStamp {
        EdgeStamp {
            utc: base + Duration::milliseconds(offset_ms),
            stream_pts: offset_ms as u64,
            ptp_offset_ns: 0,
            clock_ok: true,
        }
    }

    fn vision_tick(base: DateTime<Utc>) -> VisionTick {
        VisionTick {
            ts_from: base,
            ts_to: base + Duration::milliseconds(100),
            crossings: 1,
            mean_box_confidence: Some(0.8),
            occlusion_ratio: 0.0,
        }
    }

    fn wifi_tick() -> WifiTick {
        WifiTick {
            presence_prob: 0.9,
            doppler_energy: 1.0,
            motion_variance: 0.5,
            packet_loss_ratio: 0.02,
            mover_count_estimate: Some(2.0),
            clock_ok: true,
        }
    }

    fn zone(id: &str) -> ZoneId {
        ZoneId::try_new(id).unwrap_or_else(|e| panic!("test zone id: {e}"))
    }

    fn packet(zone_id: &ZoneId, ts: EdgeStamp) -> VisionPacket {
        VisionPacket {
            zone_id: zone_id.clone(),
            ts,
            tick: vision_tick(ts.utc),
        }
    }

    #[test]
    fn pairs_when_vision_pushed_first() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-pair");
        aligner.push_vision(packet(&z, stamp_at(base, 0)));
        assert_eq!(aligner.buffered_vision(&z), 1);
        let tick = aligner
            .push_wifi(&z, stamp_at(base, 30), wifi_tick())
            .expect("matches buffered vision");
        assert_eq!(tick.vision.crossings, 1);
        assert_eq!(tick.zone_id, z);
        assert_eq!(aligner.buffered_vision(&z), 0);
        assert!(aligner.pending_zones().is_empty());
    }

    #[test]
    fn pairs_when_wifi_pushed_first() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-wifi-first");
        aligner.push_wifi(&z, stamp_at(base, 0), wifi_tick());
        let tick = aligner
            .push_vision(packet(&z, stamp_at(base, 50)))
            .expect("matches cached wifi");
        assert_eq!(tick.vision.crossings, 1);
        assert!(!tick.wifi.is_csi_degraded());
        assert_eq!(aligner.buffered_vision(&z), 0);
        assert!(!aligner.pending_zones().contains(&z));
    }

    #[test]
    fn healthy_clocks_await_exactly_100ms() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-healthy");
        aligner.push_wifi(&z, stamp_at(base, 0), wifi_tick());
        let paired = aligner
            .push_vision(packet(&z, stamp_at(base, 100)))
            .is_some();
        assert!(paired, "boundary ±100 ms must pair on healthy clocks");
    }

    #[test]
    fn healthy_clocks_reject_far_vision_and_keep_wifi() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-far");
        aligner.push_wifi(&z, stamp_at(base, 0), wifi_tick());
        // 150 ms away with healthy clocks: outside the pulse window, buffered.
        let buffered = aligner
            .push_vision(packet(&z, stamp_at(base, 150)))
            .is_none();
        assert!(buffered);
        // Wi-Fi must stay cached (latest-wins until consumed by a pairing).
        let paired = aligner
            .push_vision(packet(&z, stamp_at(base, 10)))
            .expect("second vision pulse pairs the still-cached wifi");
        assert_eq!(paired.vision.crossings, 1);
    }

    #[test]
    fn degraded_clocks_widen_tolerance() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-degraded");
        let mut stamp = stamp_at(base, 0);
        stamp.clock_ok = false;
        aligner.push_wifi(&z, stamp, wifi_tick());
        let paired = aligner
            .push_vision(packet(&z, stamp_at(base, 250)))
            .expect("degraded tolerance 300 ms covers 250 ms");
        assert_eq!(
            paired.ts.utc,
            stamp_at(base, 250).utc,
            "aligned tick stamps the vision side"
        );
    }

    #[test]
    fn zones_do_not_cross_pair() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        aligner.push_wifi(&zone("z-a"), stamp_at(base, 0), wifi_tick());
        let no_pair = aligner
            .push_vision(packet(&zone("z-b"), stamp_at(base, 0)))
            .is_none();
        assert!(no_pair, "vision of zone-b must not pair wifi of zone-a");
        assert!(aligner.pending_zones().contains(&zone("z-b")));
        assert_eq!(aligner.buffered_vision(&zone("z-a")), 0);
    }

    #[test]
    fn latest_wifi_wins() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-latest");
        let mut first = wifi_tick();
        first.presence_prob = 0.1;
        let mut second = wifi_tick();
        second.presence_prob = 0.9;
        aligner.push_wifi(&z, stamp_at(base, 0), first);
        // No vision to pair with: the newer tick must replace the cached one.
        aligner.push_wifi(&z, stamp_at(base, 200), second);
        let tick = aligner
            .push_vision(packet(&z, stamp_at(base, 200)))
            .expect("pairs replaced (latest) wifi");
        assert!((tick.wifi.presence_prob - 0.9).abs() < 1e-6);
        assert_eq!(tick.vision.crossings, 1);
    }

    #[test]
    fn vision_buffer_evicts_oldest_beyond_cap() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-cap-two");
        for i in 0..5 {
            aligner.push_vision(packet(&z, stamp_at(base, 200 * i)));
        }
        assert_eq!(aligner.buffered_vision(&z), MAX_BUFFERED_VISION);
        assert!(!aligner.pending_zones().is_empty());
    }

    #[test]
    fn closest_buffered_vision_wins() {
        let mut aligner = CrossModalAligner::default();
        let base = Utc::now();
        let z = zone("z-closest");
        aligner.push_vision(packet(&z, stamp_at(base, 0)));
        aligner.push_vision(packet(&z, stamp_at(base, 90)));
        let tick = aligner
            .push_wifi(&z, stamp_at(base, 95), wifi_tick())
            .expect("closest buffered vision (90 ms) wins");
        assert_eq!(tick.ts.stream_pts, 90);
    }
}
