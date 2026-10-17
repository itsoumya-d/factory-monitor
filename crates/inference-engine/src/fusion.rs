//! Modality fusion engine (docs/07 §5): the counted-truth decision rules —
//! vision is primary, Wi-Fi blends into confidence, divergence bands gate
//! flags, and the Wi-Fi rate integrator bridges occlusions with a soft
//! reconciliation at the next unoccluded frame.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::debug;

use common_types::action::RlAction;
use common_types::counting::{PackageCountEvent, UncertaintyFlags};
use common_types::fusion::{AlignedTick, ModalityBlend};
use common_types::ids::{ModelVersion, ZoneId};
use common_types::traits::SensorFusionEngine;

use crate::error::InferenceError;

/// Per-zone Wi-Fi throughput model (docs/07 §5: "regressed offline"). Counts
/// emerge from `rate × movers × occluded seconds`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WifiRateModel {
    pub boxes_per_mover_second: f32,
}

impl WifiRateModel {
    /// Conservative calibration seed (three boxes/min/mover).
    pub const DEFAULT: Self = Self {
        boxes_per_mover_second: 0.05,
    };

    pub fn try_new(boxes_per_mover_second: f32) -> Result<Self, InferenceError> {
        if boxes_per_mover_second.is_finite() && boxes_per_mover_second > 0.0 {
            Ok(Self {
                boxes_per_mover_second,
            })
        } else {
            Err(InferenceError::Detector(
                "throughput model rate must be finite and positive".to_string(),
            ))
        }
    }
}

/// Below this presence probability the CSI path claims nobody is there.
pub const PRESENCE_GATE: f32 = 0.5;
/// `mean_box_confidence` under this marks `LOW_TRACK_STRENGTH`.
pub const LOW_STRENGTH_GATE: f32 = 0.5;
/// Divergence band boundaries (docs/07 §5).
pub const EXACT_AGREEMENT_TOL: f32 = 1.0;
pub const STRONG_DIVERGENCE: f32 = 3.0;

struct OccludedCarry {
    start_ts: DateTime<Utc>,
    accumulated: f32,
    enter_strong: bool,
}

/// Vision tick duration in seconds for the rate integration.
fn tick_duration_secs(tick: &AlignedTick) -> f32 {
    (tick.vision.ts_to - tick.vision.ts_from)
        .num_milliseconds()
        .max(0) as f32
        / 1000.0
}

/// The multi-modal decision engine. The action bus feeds thresholds; every
/// emitted event stamps the thresholds and the blend actually used.
pub struct ModalityFusionEngine {
    action: RlAction,
    models: HashMap<ZoneId, WifiRateModel>,
    cumulative: HashMap<ZoneId, i64>,
    occluded_carry: HashMap<ZoneId, OccludedCarry>,
    model_version: ModelVersion,
}

impl ModalityFusionEngine {
    pub fn new(model_version: ModelVersion) -> Self {
        Self {
            action: RlAction::defaults(),
            models: HashMap::new(),
            cumulative: HashMap::new(),
            occluded_carry: HashMap::new(),
            model_version,
        }
    }

    /// Wire the current RL action: thresholds for stamped events and the
    /// normalized blend for confidence weighting (docs/08 — never blocking;
    /// the app layer calls this on every action-bus refresh).
    pub fn set_action(&mut self, action: RlAction) {
        self.action = action;
    }

    pub fn action(&self) -> RlAction {
        self.action
    }

    pub fn blend(&self) -> ModalityBlend {
        ModalityBlend::normalize(self.action.vision_weight())
    }

    pub fn set_zone_model(&mut self, zone_id: ZoneId, model: WifiRateModel) {
        self.models.insert(zone_id, model);
    }

    pub fn zone_model(&self, zone_id: &ZoneId) -> Option<WifiRateModel> {
        self.models.get(zone_id).copied()
    }

    pub fn cumulative(&self, zone_id: &ZoneId) -> i64 {
        self.cumulative.get(zone_id).copied().unwrap_or(0)
    }

    /// Seed/repair a zone day-total (audit reconciliation path).
    pub fn seed_cumulative(&mut self, zone_id: &ZoneId, total: i64) {
        self.cumulative.insert(zone_id.clone(), total);
    }

    /// The tick's Wi-Fi estimate (expected boxes inside the window). Zero
    /// when CSI is degraded, presence sits under the gate, or no model is
    /// registered for the zone (counts then trust vision alone, flagged).
    fn wifi_estimate(&self, tick: &AlignedTick) -> f32 {
        if tick.wifi.is_csi_degraded() || tick.wifi.presence_prob < PRESENCE_GATE {
            return 0.0;
        }
        let Some(model) = self.zone_model(&tick.zone_id) else {
            return 0.0;
        };
        let movers = match tick.wifi.mover_count_estimate {
            Some(m) if m.is_finite() => m.max(1.0) as f32,
            _ => 1.0,
        };
        model.boxes_per_mover_second * movers * tick_duration_secs(tick)
    }

    fn confidence_wifi(tick: &AlignedTick) -> f32 {
        let p = tick.wifi.presence_prob.clamp(0.0, 1.0);
        let loss = tick.wifi.packet_loss_ratio.clamp(0.0, 1.0);
        p * (1.0 - loss)
    }

    fn base_flags(&self, tick: &AlignedTick) -> UncertaintyFlags {
        let mut flags = UncertaintyFlags::empty();
        if !tick.ts.clock_ok || !tick.wifi.clock_ok {
            flags |= UncertaintyFlags::CLOCK_DEGRADED;
        }
        if let Some(c) = tick.vision.mean_box_confidence {
            if c.is_finite() && c < LOW_STRENGTH_GATE {
                flags |= UncertaintyFlags::LOW_TRACK_STRENGTH;
            }
        }
        flags
    }

    fn build_event(
        &self,
        tick: &AlignedTick,
        ts_from: DateTime<Utc>,
        delta: i64,
        base_scale: f32,
        flags: UncertaintyFlags,
    ) -> Result<PackageCountEvent, InferenceError> {
        let blend = self.blend();
        let conf_v = tick
            .vision
            .mean_box_confidence
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let conf_w = Self::confidence_wifi(tick);
        let confidence =
            (blend.vision() * conf_v + blend.wifi() * conf_w).clamp(0.0, 1.0) * base_scale.max(0.0);
        let confidence = confidence.clamp(0.0, 1.0);
        let delta_total = self.cumulative(&tick.zone_id) + delta;
        PackageCountEvent::try_new(
            ts_from,
            tick.vision.ts_to,
            tick.zone_id.clone(),
            delta,
            delta_total,
            confidence,
            flags,
            blend,
            self.action.as_detector_thresholds(),
            Vec::new(),
            Vec::new(),
            self.model_version.clone(),
        )
        .map_err(|e| InferenceError::Detector(format!("count event rejected: {e}")))
    }

    fn finish(&mut self, event: PackageCountEvent) -> Vec<PackageCountEvent> {
        let zone = event.zone_id.clone();
        let new_total = self.cumulative(&zone) + event.count_delta;
        self.cumulative.insert(zone, new_total);
        let mut stamped = event;
        stamped.cumulative.cumulative_after = new_total;
        vec![stamped]
    }

    fn emit(
        &mut self,
        tick: &AlignedTick,
        delta: i64,
        base_scale: f32,
        flags: UncertaintyFlags,
    ) -> Vec<PackageCountEvent> {
        self.emit_ts(tick, tick.vision.ts_from, delta, base_scale, flags)
    }

    fn emit_ts(
        &mut self,
        tick: &AlignedTick,
        ts_from: DateTime<Utc>,
        delta: i64,
        base_scale: f32,
        flags: UncertaintyFlags,
    ) -> Vec<PackageCountEvent> {
        match self.build_event(tick, ts_from, delta, base_scale, flags) {
            Ok(event) => self.finish(event),
            Err(e) => {
                debug!(error = %e, "fusion event rejected; dropped tick");
                Vec::new()
            }
        }
    }
}

impl SensorFusionEngine for ModalityFusionEngine {
    fn fuse_tick(&mut self, tick: AlignedTick) -> Vec<PackageCountEvent> {
        let base_flags = self.base_flags(&tick);
        let delta_v = tick.vision.crossings;
        let secs = tick_duration_secs(&tick);
        let delta_w = self.wifi_estimate(&tick);
        let divergence = (delta_v as f32 - delta_w).abs();
        // Degraded CSI does not enter the carry lane: `wifi_estimate`
        // already zeroes it and the vision path flows plain (vision-only).
        let relying_on_wifi = tick.vision.occlusion_ratio > 0.5;
        let inst = if secs > 0.0 { delta_w.max(0.0) } else { 0.0 };

        debug!(zone = %tick.zone_id, delta_v, est = delta_w, "fuse_tick");

        // Rule 3 (docs/07 §5): occluded vision or degraded CSI allow the
        // Wi-Fi rate integrator to carry; strong divergence on a clear tick
        // counts vision immediately with the divergence flag.
        if let Some(carry) = self.occluded_carry.get_mut(&tick.zone_id) {
            if relying_on_wifi {
                carry.accumulated += inst;
                debug!(zone = %tick.zone_id, collected = carry.accumulated, "carry grows");
                return Vec::new();
            }
            let collected = carry.accumulated;
            let start_ts = carry.start_ts;
            let enter_strong = carry.enter_strong;
            let mut flags = base_flags | UncertaintyFlags::OCCLUDED_ESTIMATE;
            if enter_strong {
                flags |= UncertaintyFlags::MODALITY_DIVERGENCE;
            }
            self.occluded_carry.remove(&tick.zone_id);
            let rounded = collected.round() as i64;
            debug!(zone = %tick.zone_id, collected, rounded, "occlusion carry closed");
            if rounded != 0 {
                return self.emit_ts(&tick, start_ts, rounded, 1.0, flags);
            }
            if delta_v != 0 {
                // Zero-integrated estimate over a counted span: keep the
                // vision count instead of silently dropping it.
                return self.emit(
                    &tick,
                    delta_v,
                    0.65,
                    flags | UncertaintyFlags::MODALITY_DIVERGENCE,
                );
            }
            return Vec::new();
        }
        if divergence > STRONG_DIVERGENCE || relying_on_wifi {
            if relying_on_wifi {
                self.occluded_carry.insert(
                    tick.zone_id.clone(),
                    OccludedCarry {
                        start_ts: tick.vision.ts_from,
                        accumulated: inst,
                        enter_strong: divergence > STRONG_DIVERGENCE,
                    },
                );
                debug!(zone = %tick.zone_id, est = inst, "occlusion carry opened");
                return Vec::new();
            }
            if delta_v != 0 {
                return self.emit(
                    &tick,
                    delta_v,
                    0.65,
                    base_flags | UncertaintyFlags::MODALITY_DIVERGENCE,
                );
            }
            return Vec::new();
        }

        // Rules 1 & 2: exact agreement counts cleanly, mild divergence
        // counts vision at reduced confidence with the divergence flag;
        // zero-vision ticks stay silent either way.
        if delta_v == 0 {
            return Vec::new();
        }
        if divergence <= EXACT_AGREEMENT_TOL {
            return self.emit(&tick, delta_v, 1.0, base_flags);
        }
        self.emit(
            &tick,
            delta_v,
            0.65,
            base_flags | UncertaintyFlags::MODALITY_DIVERGENCE,
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::Duration;
    use common_types::edge_stamp::EdgeStamp;
    use common_types::fusion::VisionTick;
    use common_types::fusion::WifiTick;
    use common_types::ids::{ModelVersion, ZoneId};

    const ZONE: &str = "line_1";
    /// Deterministic epoch base; every test tick spans [BASE, BASE+100 ms].
    const BASE: DateTime<Utc> = DateTime::<Utc>::UNIX_EPOCH;
    const RATE: f32 = 30.0; // 30 boxes/mover/s → est = rate × movers × 0.1 s

    fn model_version() -> ModelVersion {
        ModelVersion::try_new("shado-fusion-test-v1").unwrap()
    }

    fn stamp(base: DateTime<Utc>, offset_ms: i64) -> EdgeStamp {
        EdgeStamp {
            utc: base + Duration::milliseconds(offset_ms),
            stream_pts: offset_ms as u64,
            ptp_offset_ns: 0,
            clock_ok: true,
        }
    }

    fn vision_tick(base: DateTime<Utc>, crossings: i64, occluded: bool) -> VisionTick {
        VisionTick {
            ts_from: base,
            ts_to: base + Duration::milliseconds(100),
            crossings,
            mean_box_confidence: Some(0.8),
            occlusion_ratio: i32::from(occluded) as f32 * 0.9,
        }
    }

    fn wifi_tick() -> WifiTick {
        WifiTick {
            presence_prob: 0.9,
            doppler_energy: 1.0,
            motion_variance: 0.5,
            packet_loss_ratio: 0.02,
            mover_count_estimate: Some(1.0),
            clock_ok: true,
        }
    }

    fn zone() -> ZoneId {
        ZoneId::try_new(ZONE).unwrap()
    }

    fn engine() -> ModalityFusionEngine {
        let mut engine = ModalityFusionEngine::new(model_version());
        engine.set_zone_model(zone(), WifiRateModel::try_new(RATE).unwrap());
        engine
    }

    fn tick(base: DateTime<Utc>, crossings: i64, occluded: bool) -> AlignedTick {
        tick_custom(base, crossings, occluded, wifi_tick())
    }

    fn tick_custom(
        base: DateTime<Utc>,
        crossings: i64,
        occluded: bool,
        wifi: WifiTick,
    ) -> AlignedTick {
        AlignedTick {
            ts: stamp(base, 0),
            zone_id: zone(),
            vision: vision_tick(base, crossings, occluded),
            wifi,
        }
    }

    #[test]
    fn exact_agreement_blends_confidence() {
        // vision 3, wifi est 3.0 (30 × 1 × 0.1 s) → rule 1.
        let mut engine = engine();
        let events = engine.fuse_tick(tick(BASE, 3, false));
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.count_delta, 3);
        assert_eq!(event.cumulative.cumulative_after, 3);
        assert_eq!(event.uncertainty, UncertaintyFlags::empty());
        // blend 0.7 vision / 0.3 wifi: 0.7×0.8 + 0.3×(0.9×0.98)
        let expected = 0.7 * 0.8 + 0.3 * (0.9 * 0.98);
        assert!(
            (event.confidence - expected).abs() < 1e-4,
            "confidence {event:?}"
        );
        assert_eq!(event.model_version, model_version());
        assert_eq!(event.ts_from, BASE);
        assert_eq!(event.ts_to, BASE + Duration::milliseconds(100));
    }

    #[test]
    fn mild_divergence_damps_confidence_and_flags() {
        // vision 1, wifi est 2.4 (24 × 1 × 0.1) → divergence 1.4 ≤ 3: rule 2.
        let mut engine = engine();
        engine.set_zone_model(zone(), WifiRateModel::try_new(24.0).unwrap());
        let events = engine.fuse_tick(tick(BASE, 1, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].count_delta, 1);
        assert!(
            events[0]
                .uncertainty
                .contains(UncertaintyFlags::MODALITY_DIVERGENCE)
        );
        assert!(
            (events[0].confidence - 0.65_f32 * (0.7 * 0.8 + 0.3 * 0.882)).abs() < 1e-3,
            "rule 2 damps by ×0.65"
        );
    }

    #[test]
    fn zero_agreement_emits_nothing() {
        let mut engine = engine();
        // est 0 (movers 0 counted as 1 → 30 × 0.1 = 3 ≠ 0)... use blockers:
        let mut wifi = wifi_tick();
        wifi.presence_prob = 0.1; // below presence gate → est 0
        let events = engine.fuse_tick(tick_custom(BASE, 0, false, wifi));
        assert!(events.is_empty(), "rule 1 with Δv = 0 stays silent");
        assert_eq!(engine.cumulative(&zone()), 0);
    }

    #[test]
    fn strong_divergence_not_occluded_counts_vision() {
        // vision 1, est 10 (100 × 1 × 0.1): div 9 > 3, not occluded → count
        // vision immediately with divergence flag (no carry).
        let mut engine = engine();
        engine.set_zone_model(zone(), WifiRateModel::try_new(100.0).unwrap());
        let events = engine.fuse_tick(tick(BASE, 1, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].count_delta, 1);
        assert!(
            events[0]
                .uncertainty
                .contains(UncertaintyFlags::MODALITY_DIVERGENCE)
        );
        assert!(
            !events[0]
                .uncertainty
                .contains(UncertaintyFlags::OCCLUDED_ESTIMATE),
            "not occluded strong divergence: no carry"
        );
        assert!(engine.occluded_carry.is_empty());
    }

    #[test]
    fn occluded_carry_accumulates_and_reconciles() {
        let mut engine = engine();
        let t0 = BASE;
        // Open: occluded, cue est 3.0 (30 × 1 × 0.1) vs vision 0 → div 3.0 ≤ 3,
        // but occlusion > 0.5 with wifi est ≥ 1 opens the carry anyway.
        let events = engine.fuse_tick(tick(t0, 0, true));
        assert!(events.is_empty(), "carry open stays silent");
        assert_eq!(engine.occluded_carry.len(), 1);
        // Mid: still occluded, another 3.0 integrates into the carry.
        let events = engine.fuse_tick(tick(t0 + Duration::milliseconds(200), 0, true));
        assert!(events.is_empty());
        // Close: unoccluded tick → soft reconciliation, rounded 6.
        let events = engine.fuse_tick(tick(t0 + Duration::milliseconds(400), 0, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].count_delta, 6);
        assert!(
            events[0]
                .uncertainty
                .contains(UncertaintyFlags::OCCLUDED_ESTIMATE)
        );
        assert!(engine.occluded_carry.is_empty());
        assert_eq!(engine.cumulative(&zone()), 6);
    }

    #[test]
    fn strong_occluded_carry_flags_divergence_on_close() {
        let mut engine = engine();
        engine.set_zone_model(zone(), WifiRateModel::try_new(50.0).unwrap());
        // Open strong: est 5.0 vs vision 0 → div 5 > 3 while occluded.
        assert!(engine.fuse_tick(tick(BASE, 0, true)).is_empty());
        let events = engine.fuse_tick(tick(BASE + Duration::milliseconds(200), 0, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].count_delta, 5);
        assert!(
            events[0]
                .uncertainty
                .contains(UncertaintyFlags::OCCLUDED_ESTIMATE)
        );
        assert!(
            events[0]
                .uncertainty
                .contains(UncertaintyFlags::MODALITY_DIVERGENCE)
        );
    }

    #[test]
    fn zero_accumulation_falls_back_to_vision_count() {
        let mut engine = engine();
        // Weak rate: est ≈ 0.005 → carry opens on occlusion with acc ~0.
        engine.set_zone_model(zone(), WifiRateModel::try_new(0.0001).unwrap());
        assert!(engine.fuse_tick(tick(BASE, 0, true)).is_empty());
        let events = engine.fuse_tick(tick(BASE + Duration::milliseconds(200), 2, false));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].count_delta, 2, "vision count kept for audit");
        assert!(
            events[0]
                .uncertainty
                .contains(UncertaintyFlags::MODALITY_DIVERGENCE)
        );
    }

    #[test]
    fn degraded_csi_counts_vision_without_divergence_in_mild_band() {
        let mut engine = engine();
        let mut wifi = wifi_tick();
        wifi.presence_prob = 0.2; // below gate → est 0, vision-only
        let events = engine.fuse_tick(tick_custom(BASE, 1, false, wifi));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].count_delta, 1);
        assert!(
            events[0].uncertainty.is_empty(),
            "csi-degraded mild band: plain emission"
        );
    }

    #[test]
    fn low_strength_and_clock_flags_stamped() {
        let mut engine = engine();
        let mut vision = vision_tick(BASE, 1, false);
        vision.mean_box_confidence = Some(0.3); // < 0.5 → LOW_TRACK_STRENGTH
        let mut wifi = wifi_tick();
        wifi.presence_prob = 0.1;
        let mut ts = stamp(BASE, 0);
        ts.clock_ok = false;
        let events = engine.fuse_tick(AlignedTick {
            ts,
            zone_id: zone(),
            vision,
            wifi,
        });
        assert_eq!(events.len(), 1);
        let flags = events[0].uncertainty;
        assert!(flags.contains(UncertaintyFlags::LOW_TRACK_STRENGTH));
        assert!(flags.contains(UncertaintyFlags::CLOCK_DEGRADED));
    }

    #[test]
    fn cumulative_tracks_per_zone() {
        let mut engine = engine();
        let zone_a = ZoneId::try_new("zone_a").unwrap();
        let zone_b = ZoneId::try_new("zone_b").unwrap();
        engine.set_zone_model(zone_a.clone(), WifiRateModel::try_new(RATE).unwrap());
        engine.set_zone_model(zone_b.clone(), WifiRateModel::try_new(RATE).unwrap());
        for zone in [zone_a.clone(), zone_b.clone()] {
            for _ in 0..2 {
                let events = engine.fuse_tick(AlignedTick {
                    ts: stamp(BASE, 0),
                    zone_id: zone.clone(),
                    vision: vision_tick(BASE, 2, false),
                    wifi: wifi_tick(),
                });
                assert_eq!(events.len(), 1);
            }
        }
        assert_eq!(engine.cumulative(&zone_a), 4);
        assert_eq!(engine.cumulative(&zone_b), 4);
        assert_eq!(engine.cumulative(&ZoneId::try_new("zone_c").unwrap()), 0);
    }

    #[test]
    fn seed_cumulative_rebases() {
        let mut engine = engine();
        assert!(engine.fuse_tick(tick(BASE, 3, false)).len() == 1);
        engine.seed_cumulative(&zone(), 900);
        let events = engine.fuse_tick(tick(BASE + Duration::milliseconds(200), 2, false));
        assert_eq!(events[0].cumulative.cumulative_after, 902);
    }

    #[test]
    fn wifi_rate_model_rejects_bad_rates() {
        assert!(WifiRateModel::try_new(0.0).is_err());
        assert!(WifiRateModel::try_new(f32::NAN).is_err());
        assert!(WifiRateModel::try_new(-1.0).is_err());
        assert!(WifiRateModel::try_new(WifiRateModel::DEFAULT.boxes_per_mover_second).is_ok());
    }

    #[test]
    fn set_action_changes_blend() {
        let mut engine = engine();
        let action = RlAction::try_new(0.5, 0.5, 0.6, 45).unwrap();
        engine.set_action(action);
        assert!((engine.blend().vision() - 0.6).abs() < 1e-6);
        assert!((engine.blend().wifi() - 0.4).abs() < 1e-6);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod rl_cannot_blind {
    use super::*;
    use common_types::bounds::{CONF_SAFE_RANGE, FUSION_WEIGHT_RANGE};

    /// Deterministic LCG — nightly soak without external rand deps.
    struct Lcg(u64);
    impl Lcg {
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
    }

    /// Every raw policy output that can exist keeps the blend inside the
    /// guardrails: vision never below 0.05, all weights normalized.
    #[test]
    fn clamped_actions_keep_the_blend_in_guardrails() {
        let mut rng = Lcg(0xDA1A);
        for _ in 0..200 {
            let raw_c = 10.0 * (rng.next_u32() as f32 / u32::MAX as f32) - 5.0;
            let raw_w = 10.0 * (rng.next_u32() as f32 / u32::MAX as f32) - 5.0;
            let action = RlAction::clamped(raw_c, 0.5, raw_w, 45);
            let wf = action.vision_weight();
            let _ = action.conf_threshold();
            assert!(
                FUSION_WEIGHT_RANGE.contains(&wf),
                "weight {wf} outside {FUSION_WEIGHT_RANGE:?} from raw {raw_w}"
            );
            assert!(
                CONF_SAFE_RANGE.contains(&action.conf_threshold()),
                "conf outside guardrail from raw {raw_c}"
            );
            let blended = ModalityBlend::normalize(wf);
            let sum = blended.vision() + blended.wifi();
            assert!((sum - 1.0).abs() < 1e-5, "normalized into unity: {sum}");
            assert!(blended.vision() >= *FUSION_WEIGHT_RANGE.start() - 1e-6);
        }
    }

    #[test]
    fn extreme_feedback_rewinds_to_bounded_reward() {
        for delta in [-8000_i64, -150, 150, 8000] {
            let reward = rl_controller::reward::correction_reward(0, delta);
            assert!(reward.is_finite(), "{delta} → {reward}");
            assert!(
                reward <= 0.0,
                "every error is a penalty: {delta} → {reward}"
            );
        }
    }
}
