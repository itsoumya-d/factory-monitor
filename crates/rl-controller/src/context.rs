//! Action-dimension bucket grid + context features (docs/08 §3/§5). The
//! bandit never selects raw floats — it picks a bucket per action dim, then
//! routes through `RlAction::clamped` (the ONLY constructor, guardrail #1).

use chrono::{DateTime, Timelike, Utc};
use common_types::action::RlAction;
use common_types::state::FactoryEnvState;

/// Reward-eligible 30-min epochs (docs/08 §4).
pub const EPOCH_MINUTES: i64 = 30;

/// Exploration window (docs/08 §5): ε-greedy early morning only; NEVER
/// during peak throughput hours 13:00–17:00.
pub const EXPLORATION_START_HOUR: u32 = 5;
pub const EXPLORATION_END_HOUR: u32 = 13;
pub const EXPLORATION_EPSILON: f32 = 0.05;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ActionDim {
    Conf,
    Iou,
    VisionWeight,
    TrackBuffer,
    FpWeight,
}

pub const ACTION_DIMS: [ActionDim; 5] = [
    ActionDim::Conf,
    ActionDim::Iou,
    ActionDim::VisionWeight,
    ActionDim::TrackBuffer,
    ActionDim::FpWeight,
];

/// Discrete action buckets, all within the guardrail envelopes.
pub const CONF_BUCKETS: [f32; 5] = [0.30, 0.45, 0.60, 0.75, 0.85];
pub const IOU_BUCKETS: [f32; 4] = [0.35, 0.45, 0.55, 0.70];
pub const VISION_WEIGHT_BUCKETS: [f32; 4] = [0.10, 0.30, 0.70, 0.90];
pub const TRACK_BUFFER_BUCKETS: [u16; 3] = [20, 45, 90];
pub const FP_WEIGHT_BUCKETS: [f32; 4] = [0.5, 1.0, 2.0, 3.0];
pub const PPO_GAMMA: f32 = 0.92;

/// Bias + 12-d normalized state (docs/08 §3).
pub const FEATURE_DIM: usize = 13;

impl ActionDim {
    pub fn bucket_count(self) -> usize {
        match self {
            Self::Conf => CONF_BUCKETS.len(),
            Self::Iou => IOU_BUCKETS.len(),
            Self::VisionWeight => VISION_WEIGHT_BUCKETS.len(),
            Self::TrackBuffer => TRACK_BUFFER_BUCKETS.len(),
            Self::FpWeight => FP_WEIGHT_BUCKETS.len(),
        }
    }

    pub fn bucket_value(self, bucket: usize) -> BucketValue {
        match self {
            Self::Conf => BucketValue::f(CONF_BUCKETS[bucket]),
            Self::Iou => BucketValue::f(IOU_BUCKETS[bucket]),
            Self::VisionWeight => BucketValue::f(VISION_WEIGHT_BUCKETS[bucket]),
            Self::TrackBuffer => BucketValue::u(TRACK_BUFFER_BUCKETS[bucket]),
            Self::FpWeight => BucketValue::f(FP_WEIGHT_BUCKETS[bucket]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BucketValue {
    Float(f32),
    Frames(u16),
}

impl BucketValue {
    fn f(v: f32) -> Self {
        Self::Float(v)
    }
    fn u(v: u16) -> Self {
        Self::Frames(v)
    }
}

/// Total arms across all dims (19): 5 + 4 + 4 + 3 + 4 — each is a linear
/// contextual Thompson model over the shared 13-dim state features.
pub const TOTAL_ARMS: usize = 5 + 4 + 4 + 3 + 4;

/// One-hotted arm choice per dim (FpWeight's arm is private policy state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmChoice {
    pub dim: ActionDim,
    pub bucket: usize,
}

/// Feature vector for the linear models (docs/08 §3): bias + the 12-d
/// normalized state projection. Feature construction is the ONLY way arms
/// see state — raw structs never reach the bandit.
pub fn feature_vec(state: &FactoryEnvState) -> [f32; 13] {
    let mut x = [0.0f32; 13];
    x[0] = 1.0;
    x[1..13].copy_from_slice(&state.rl_state_vector());
    x
}

/// Discrete 30-min epoch index, stable regardless of local calendar; epoch 0
/// is 1970-01-01T00:00Z.
pub fn epoch_of(ts: DateTime<Utc>) -> u64 {
    (ts.timestamp() / (EPOCH_MINUTES * 60)) as u64
}

pub fn epoch_of_range(
    ts_from: DateTime<Utc>,
    ts_to: DateTime<Utc>,
) -> std::ops::RangeInclusive<u64> {
    epoch_of(ts_from)..=epoch_of(ts_to.max(ts_from))
}

/// ε should be armed only inside the early-morning window on the EDGE's
/// local wall clock (the admin's business hours, docs/08 §5).
pub fn in_exploration_window(local_hour: u32) -> bool {
    (EXPLORATION_START_HOUR..EXPLORATION_END_HOUR).contains(&local_hour)
}

/// Next exploration window after `ts` (for the PolicyUpdateReport).
pub fn next_exploration_window(ts: DateTime<Utc>) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let hour = ts.hour();
    if in_exploration_window(hour) {
        let end = ts
            .date_naive()
            .and_hms_opt(EXPLORATION_END_HOUR, 0, 0)?
            .and_local_timezone(Utc)
            .single()?;
        return Some((ts, end));
    }
    let start_day = if hour >= EXPLORATION_END_HOUR {
        ts.date_naive().succ_opt()?
    } else {
        ts.date_naive()
    };
    let start = start_day
        .and_hms_opt(EXPLORATION_START_HOUR, 0, 0)?
        .and_local_timezone(Utc)
        .single()?;
    let end = start_day
        .and_hms_opt(EXPLORATION_END_HOUR, 0, 0)?
        .and_local_timezone(Utc)
        .single()?;
    Some((start, end))
}

/// The cold-start never-fail default (docs/08 §8) — used when no snapshot
/// exists at all, and what the watchdog restores on freeze.
pub fn default_action() -> RlAction {
    RlAction::clamped(
        common_types::bounds::CONF_DEFAULT,
        common_types::bounds::IOU_DEFAULT,
        common_types::bounds::VISION_WEIGHT_DEFAULT,
        common_types::bounds::TRACK_BUFFER_DEFAULT,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn buckets_sit_inside_guardrail_envelopes() {
        assert!(
            CONF_BUCKETS
                .iter()
                .all(|b| common_types::bounds::CONF_SAFE_RANGE.contains(b))
        );
        assert!(
            IOU_BUCKETS
                .iter()
                .all(|b| common_types::bounds::IOU_SAFE_RANGE.contains(b))
        );
        assert!(
            VISION_WEIGHT_BUCKETS
                .iter()
                .all(|b| common_types::bounds::FUSION_WEIGHT_RANGE.contains(b))
        );
        assert!(
            TRACK_BUFFER_BUCKETS
                .iter()
                .all(|b| common_types::bounds::TRACK_BUFFER_RANGE.contains(b))
        );
        assert!(
            FP_WEIGHT_BUCKETS
                .iter()
                .all(|b| common_types::bounds::FP_WEIGHT_RANGE.contains(b))
        );
    }

    #[test]
    fn default_action_is_documented_cold_start() {
        let action = default_action();
        assert_eq!(action.conf_threshold(), 0.45);
        assert_eq!(action.iou_threshold(), 0.45);
        assert!((action.vision_weight() - 0.70).abs() < 1e-6);
        assert_eq!(action.track_buffer_frames(), 45);
    }

    #[test]
    fn epochs_are_stable_30min_slices() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(epoch_of(t0), epoch_of(t0 + chrono::Duration::minutes(29)));
        assert_eq!(
            epoch_of(t0) + 1,
            epoch_of(t0 + chrono::Duration::minutes(30))
        );
        let range = epoch_of_range(t0, t0 + chrono::Duration::minutes(90));
        assert_eq!(range.start(), &epoch_of(t0));
        assert_eq!(range.end(), &(epoch_of(t0) + 3));
    }

    #[test]
    fn exploration_window_bans_peak_hours() {
        assert!(in_exploration_window(5));
        assert!(in_exploration_window(12));
        assert!(!in_exploration_window(13), "peak throughput forbidden");
        assert!(!in_exploration_window(17));
        assert!(!in_exploration_window(3));
    }

    #[test]
    fn next_exploration_window_resolves() {
        let day = |y: i32, m: u32, d: u32, h: u32, min: u32| {
            Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
        };
        // Inside window: ends today at 13:00.
        let (start, end) = next_exploration_window(day(2026, 4, 7, 6, 30)).unwrap();
        assert_eq!(start, day(2026, 4, 7, 6, 30));
        assert_eq!(end, day(2026, 4, 7, 13, 0));
        // After the window: starts tomorrow 05:00.
        let (start, end) = next_exploration_window(day(2026, 4, 7, 14, 0)).unwrap();
        assert_eq!(start, day(2026, 4, 8, 5, 0));
        assert_eq!(end, day(2026, 4, 8, 13, 0));
        // Before the window: starts today.
        let (start, _) = next_exploration_window(day(2026, 4, 7, 3, 0)).unwrap();
        assert_eq!(start, day(2026, 4, 7, 5, 0));
    }

    #[test]
    fn features_are_bias_plus_state_vector() {
        let state = FactoryEnvState {
            ts: common_types::edge_stamp::EdgeStamp {
                utc: Utc::now(),
                stream_pts: 0,
                ptp_offset_ns: 0,
                clock_ok: true,
            },
            lux_proxy: vec![128.0],
            blur_score: vec![0.0],
            csi_variance: 0.5,
            occlusion_ratio: 0.0,
            conf_threshold: 0.45,
            iou_threshold: 0.45,
            vision_weight: 0.7,
            wifi_weight: 0.3,
            scene_complexity: 0.2,
            lifecycle: common_types::lifecycle::LifecycleState::Counting,
            zones: vec![],
        };
        let x = feature_vec(&state);
        assert_eq!(x.len(), FEATURE_DIM);
        assert_eq!(x[0], 1.0);
        let v = state.rl_state_vector();
        assert!((x[1] - v[0]).abs() < 1e-6);
        assert!((x[12] - v[11]).abs() < 1e-6, "lifecycle one-hot last");
    }

    #[test]
    fn bucket_counts_match_the_bucket_tables() {
        assert_eq!(ActionDim::Conf.bucket_count(), CONF_BUCKETS.len());
        assert_eq!(ActionDim::Iou.bucket_count(), IOU_BUCKETS.len());
        assert_eq!(
            ActionDim::VisionWeight.bucket_count(),
            VISION_WEIGHT_BUCKETS.len()
        );
        assert_eq!(
            ActionDim::TrackBuffer.bucket_count(),
            TRACK_BUFFER_BUCKETS.len()
        );
        assert_eq!(ActionDim::FpWeight.bucket_count(), FP_WEIGHT_BUCKETS.len());
    }

    #[test]
    fn bucket_value_maps_the_exact_table_entry() {
        // Float dims: value variant carries the table constant verbatim.
        for (b, v) in CONF_BUCKETS.iter().enumerate() {
            assert_eq!(ActionDim::Conf.bucket_value(b), BucketValue::Float(*v));
        }
        for (b, v) in IOU_BUCKETS.iter().enumerate() {
            assert_eq!(ActionDim::Iou.bucket_value(b), BucketValue::Float(*v));
        }
        for (b, v) in VISION_WEIGHT_BUCKETS.iter().enumerate() {
            assert_eq!(
                ActionDim::VisionWeight.bucket_value(b),
                BucketValue::Float(*v)
            );
        }
        for (b, v) in FP_WEIGHT_BUCKETS.iter().enumerate() {
            assert_eq!(ActionDim::FpWeight.bucket_value(b), BucketValue::Float(*v));
        }
        // The only frame-valued dim: TrackBuffer returns Frames, not Float.
        for (b, v) in TRACK_BUFFER_BUCKETS.iter().enumerate() {
            assert_eq!(
                ActionDim::TrackBuffer.bucket_value(b),
                BucketValue::Frames(*v)
            );
        }
    }
}
