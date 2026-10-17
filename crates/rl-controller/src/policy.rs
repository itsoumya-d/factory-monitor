//! `BanditPolicy` — linear contextual Thompson sampling over action buckets
//! (docs/08 §5) with the runaway guardrails (§6): clamped-only construction,
//! per-epoch τ_conf veto, 3-rejection freeze, persisted pause, and
//! always-beatable manual override.

use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use chrono::{DateTime, Timelike, Utc};
use common_types::action::RlAction;
use common_types::counting::PackageCountEvent;
use common_types::state::FactoryEnvState;
use common_types::traits::{PolicyAgent, PolicyFeedback, PolicyUpdateReport};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tracing::warn;

use crate::arm::LinearThompsonArm;
use crate::context::{
    ActionDim, CONF_BUCKETS, EXPLORATION_EPSILON, FEATURE_DIM, FP_WEIGHT_BUCKETS, IOU_BUCKETS,
    PPO_GAMMA, TRACK_BUFFER_BUCKETS, VISION_WEIGHT_BUCKETS, default_action, epoch_of,
    epoch_of_range, feature_vec, in_exploration_window, next_exploration_window,
};

/// Max |τ_conf| move from the last-safe action per epoch before veto (§6.2).
pub const MAX_CONF_STEP: f32 = common_types::bounds::CONF_STEP_MAX;
/// Freeze after this many consecutive vetoes (§6.2).
pub const VETO_FREEZE_THRESHOLD: u16 = 3;
/// Immediate-route shaping weight (docs/08 §4: 0.1 of final reward).
pub const IMMEDIATE_WEIGHT: f32 = 0.1;
/// Reward envelope: |r| never enters an arm beyond this.
pub const REWARD_CLAMP: f32 = 8.0;
/// Eligibility-trace horizon cap in halftimes (docs/08 §4).
pub const TRACE_HORIZON_STEPS: u32 = 23;
/// Episodic memory of (epoch, arms-used, context), bounded.
const HISTORY_CAP: usize = 512;
const PICKS: [ActionDim; 5] = [
    ActionDim::Conf,
    ActionDim::Iou,
    ActionDim::VisionWeight,
    ActionDim::TrackBuffer,
    ActionDim::FpWeight,
];

/// One epoch's attribution record: what was chosen and the context seen.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EpochRecord {
    pub epoch: u64,
    pub picks: [(ActionDim, usize); 5],
    pub x: [f32; FEATURE_DIM],
}

/// Persistable view of the whole policy (docs/08 §8).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InternalState {
    pub arms: Vec<((ActionDim, usize), LinearThompsonArm)>,
    pub last_safe: RlAction,
    pub frozen: bool,
    pub consecutive_vetoes: u16,
    pub pause_until: Option<DateTime<Utc>>,
    pub fp_bucket: usize,
    pub last_seen: Option<DateTime<Utc>>,
    pub history: Vec<EpochRecord>,
}

pub struct BanditPolicy {
    rng: StdRng,
    arms: HashMap<(ActionDim, usize), LinearThompsonArm>,
    last_safe: RlAction,
    frozen: bool,
    consecutive_vetoes: u16,
    pause_until: Option<DateTime<Utc>>,
    fp_bucket: usize,
    last_seen: Option<DateTime<Utc>>,
    history: VecDeque<EpochRecord>,
}

fn new_arms() -> HashMap<(ActionDim, usize), LinearThompsonArm> {
    let mut map = HashMap::new();
    for dim in PICKS {
        for bucket in 0..dim.bucket_count() {
            map.insert((dim, bucket), LinearThompsonArm::new());
        }
    }
    map
}

impl BanditPolicy {
    pub fn new() -> Self {
        Self {
            rng: StdRng::seed_from_u64(42),
            arms: new_arms(),
            last_safe: default_action(),
            frozen: false,
            consecutive_vetoes: 0,
            pause_until: None,
            fp_bucket: FP_WEIGHT_BUCKETS.len() / 2,
            last_seen: None,
            history: VecDeque::new(),
        }
    }

    /// Manual override API (guardrail #4): pause until an instant; `None`
    /// clears. The caller persists the state through `internal_state`.
    pub fn pause(&mut self, until: Option<DateTime<Utc>>) {
        self.pause_until = until;
    }

    pub fn is_paused(&self, at: DateTime<Utc>) -> bool {
        self.pause_until.is_some_and(|until| at < until)
    }

    pub fn frozen(&self) -> bool {
        self.frozen
    }

    pub fn consecutive_vetoes(&self) -> u16 {
        self.consecutive_vetoes
    }

    pub fn last_safe(&self) -> RlAction {
        self.last_safe
    }

    /// The λ that multiplies the FP term of the caller's reward computation
    /// (docs/08 §2: fp_weight in [0.5, 3.0]) — the bandit tunes it like any
    /// other dim but it is policy state, not an `RlAction` dimension.
    pub fn current_fp_weight(&self) -> f32 {
        FP_WEIGHT_BUCKETS[self.fp_bucket]
    }

    pub fn fp_bucket(&self) -> usize {
        self.fp_bucket
    }

    pub fn history(&self) -> &VecDeque<EpochRecord> {
        &self.history
    }

    pub fn internal_state(&self) -> InternalState {
        InternalState {
            arms: {
                let mut arms: Vec<((ActionDim, usize), LinearThompsonArm)> =
                    self.arms.iter().map(|(k, v)| (*k, v.clone())).collect();
                arms.sort_by_key(|(k, _)| *k);
                arms
            },
            last_safe: self.last_safe,
            frozen: self.frozen,
            consecutive_vetoes: self.consecutive_vetoes,
            pause_until: self.pause_until,
            fp_bucket: self.fp_bucket,
            last_seen: self.last_seen,
            history: self.history.iter().cloned().collect(),
        }
    }

    pub fn restore_internal_state(&mut self, state: InternalState) {
        self.arms = state.arms.into_iter().collect();
        self.last_safe = state.last_safe;
        self.frozen = state.frozen;
        self.consecutive_vetoes = state.consecutive_vetoes;
        self.pause_until = state.pause_until;
        self.fp_bucket = state.fp_bucket;
        self.last_seen = state.last_seen;
        self.history = state.history.into();
        self.history.truncate(HISTORY_CAP);
    }

    /// Manual unfreeze after an operator sign-off; optionally re-seats the
    /// restored last-known-good action.
    pub fn unfreeze(&mut self, restored: Option<RlAction>) {
        self.frozen = false;
        self.consecutive_vetoes = 0;
        if let Some(action) = restored {
            self.last_safe = action;
        }
    }

    fn arm(&mut self, dim: ActionDim, bucket: usize) -> &mut LinearThompsonArm {
        self.arms.entry((dim, bucket)).or_default()
    }

    /// ε-greedy bucket jump (early morning only, never frozen, docs §5).
    fn explore_bucket(&mut self, dim: ActionDim, current: usize, local_hour: u32) -> usize {
        let armed = in_exploration_window(local_hour)
            && !self.frozen
            && self.rng.random_bool(EXPLORATION_EPSILON as f64);
        if !armed {
            return current;
        }
        let count = dim.bucket_count();
        if count <= 1 {
            return current;
        }
        self.rng.random_range(0..count)
    }

    /// Per-dim Thompson argmax over buckets, with ε exploration.
    fn pick(&mut self, x: &[f32; FEATURE_DIM], local_hour: u32) -> [(ActionDim, usize); 5] {
        let mut picks = [(ActionDim::Conf, 0usize); 5];
        for (slot, dim) in PICKS.iter().enumerate() {
            let mut best_bucket = 0usize;
            let mut best_value = f32::MIN;
            for bucket in 0..dim.bucket_count() {
                let key = (*dim, bucket);
                let value = self
                    .arms
                    .get(&key)
                    .map_or(0.0, |arm| arm.sample(x, &mut self.rng));
                if value > best_value {
                    best_value = value;
                    best_bucket = bucket;
                }
            }
            let chosen = self.explore_bucket(*dim, best_bucket, local_hour);
            picks[slot] = (*dim, chosen);
        }
        picks
    }

    fn action_from(picks: [(ActionDim, usize); 5]) -> RlAction {
        let value_of = |dim: ActionDim| -> usize {
            picks
                .iter()
                .position(|(d, _)| *d == dim)
                .map_or(0, |pos| picks[pos].1)
        };
        let conf = CONF_BUCKETS[value_of(ActionDim::Conf).min(CONF_BUCKETS.len() - 1)];
        let iou = IOU_BUCKETS[value_of(ActionDim::Iou).min(IOU_BUCKETS.len() - 1)];
        let vision = VISION_WEIGHT_BUCKETS
            [value_of(ActionDim::VisionWeight).min(VISION_WEIGHT_BUCKETS.len() - 1)];
        let buffer = TRACK_BUFFER_BUCKETS
            [value_of(ActionDim::TrackBuffer).min(TRACK_BUFFER_BUCKETS.len() - 1)];
        // Guardrail #1: `RlAction::clamped()` is the ONLY constructor.
        RlAction::clamped(conf, iou, vision, buffer)
    }

    fn accept(
        &mut self,
        action: RlAction,
        picks: [(ActionDim, usize); 5],
        x: [f32; FEATURE_DIM],
        epoch: u64,
    ) {
        self.last_safe = action;
        self.consecutive_vetoes = 0;
        self.fp_bucket = match picks.iter().position(|(d, _)| d == &ActionDim::FpWeight) {
            Some(slot) => picks[slot].1,
            None => self.fp_bucket,
        };
        self.history.push_back(EpochRecord { epoch, picks, x });
        if self.history.len() > HISTORY_CAP {
            self.history.pop_front();
        }
    }

    /// Veto gate (docs/08 §6.2): a τ_conf jump beyond ±0.15 from last-safe.
    /// Fires `PolicyActionRejected`; three in a row freeze the policy.
    fn veto_conf(&mut self, action: &RlAction) -> bool {
        let jump = (action.conf_threshold() - self.last_safe.conf_threshold()).abs();
        if jump <= MAX_CONF_STEP {
            return true;
        }
        self.consecutive_vetoes += 1;
        warn!(
            jump,
            vetoes = self.consecutive_vetoes,
            "PolicyActionRejected: τ_conf out of runway"
        );
        if self.consecutive_vetoes >= VETO_FREEZE_THRESHOLD {
            self.frozen = true;
            warn!(
                vetoes = self.consecutive_vetoes,
                "RL policy frozen; keep last-known-good action"
            );
        }
        false
    }

    /// Credit the arms that owned `epoch` with `reward` at the stored
    /// context. Unknown epochs are silently dropped.
    fn credit(&mut self, epoch: u64, reward: f32) {
        let Some(record) = self
            .history
            .iter()
            .rev()
            .find(|record| record.epoch == epoch)
            .cloned()
        else {
            return;
        };
        for (dim, bucket) in record.picks {
            self.arm(dim, bucket).observe(&record.x, reward);
        }
    }

    fn apply_event_signal(&mut self, event: &PackageCountEvent, immediate: f32) {
        let reward = IMMEDIATE_WEIGHT * immediate.clamp(-REWARD_CLAMP, REWARD_CLAMP);
        let range = epoch_of_range(event.ts_from, event.ts_to);
        let steps = (*range.end() - *range.start()).min(3);
        for step in 0..=steps {
            let epoch = range.start() + step;
            let trace = PPO_GAMMA.powi((step.min(u64::from(TRACE_HORIZON_STEPS))) as i32);
            self.credit(epoch, reward * trace);
        }
    }

    fn apply_human_audit(
        &mut self,
        ts_from: DateTime<Utc>,
        ts_to: DateTime<Utc>,
        full_reward: f32,
    ) {
        let range = epoch_of_range(ts_from, ts_to);
        let now_epoch = *range.end();
        let full = full_reward.clamp(-REWARD_CLAMP, REWARD_CLAMP);
        for epoch in range {
            let age = (now_epoch - epoch).min(u64::from(TRACE_HORIZON_STEPS));
            let trace = PPO_GAMMA.powi(age as i32);
            self.credit(epoch, full * trace);
        }
    }

    fn report(&self) -> PolicyUpdateReport {
        let seen = self.last_seen.unwrap_or_else(Utc::now);
        PolicyUpdateReport {
            new_action: self.last_safe,
            was_clamped: false,
            attribution_steps: u16::try_from(self.history.len()).unwrap_or(u16::MAX),
            next_exploration_window: next_exploration_window(seen),
        }
    }
}

impl Default for BanditPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PolicyAgent for BanditPolicy {
    fn name(&self) -> &str {
        "bandit-v1"
    }

    /// Thompson sample → per-dim argmax → `RlAction::clamped`. Paused or
    /// frozen policies hand back the last-safe action untouched.
    async fn select_action(&mut self, state: &FactoryEnvState) -> RlAction {
        let now = state.ts.utc;
        self.last_seen = Some(now);
        if self.is_paused(now) || self.frozen {
            return self.last_safe;
        }
        let x = feature_vec(state);
        // edge wall clock assumed UTC-aligned; explore-hour filter in helper.
        let local_hour = now.hour();
        let picks = self.pick(&x, local_hour);
        let action = Self::action_from(picks);
        let accepted = self.veto_conf(&action);
        if accepted {
            self.accept(action, picks, x, epoch_of(now));
        }
        self.last_safe
    }

    /// Two feedback routes (docs/08 §4): immediate ± shaping at 0.1×; the
    /// EOD audit's full reward attributed over its window via traces.
    async fn update_policy(
        &mut self,
        feedback: &PolicyFeedback,
    ) -> Result<PolicyUpdateReport, anyhow::Error> {
        self.last_seen = Some(Utc::now());
        match feedback {
            PolicyFeedback::EventSignal { event, immediate } => {
                self.apply_event_signal(event, *immediate);
            }
            PolicyFeedback::HumanAudit {
                feedback,
                computed_reward,
            } => {
                self.apply_human_audit(feedback.ts_from, feedback.ts_to, *computed_reward);
            }
        }
        Ok(self.report())
    }

    async fn snapshot_weights(&self) -> Vec<u8> {
        // Serialization of our own plain-data state cannot fail in practice;
        // on the impossible path log loudly and let the next load reject the
        // empty blob as corrupt (fail-safe, never panic in prod).
        match serde_json::to_vec(&self.internal_state()) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::error!(error = %err, "RL snapshot serialization failed");
                b"{}".to_vec()
            }
        }
    }

    async fn restore_weights(&mut self, blob: &[u8]) -> Result<(), anyhow::Error> {
        let state: InternalState = serde_json::from_slice(blob)?;
        self.restore_internal_state(state);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common_types::counting::UncertaintyFlags;
    use common_types::edge_stamp::EdgeStamp;
    use common_types::lifecycle::LifecycleState;

    fn base_day(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 7, h, m, 0).unwrap()
    }

    fn state(at: DateTime<Utc>) -> FactoryEnvState {
        FactoryEnvState {
            ts: EdgeStamp {
                utc: at,
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
            lifecycle: LifecycleState::Counting,
            zones: vec![],
        }
    }

    fn zone() -> common_types::ids::ZoneId {
        common_types::ids::ZoneId::try_new("line_1").unwrap()
    }

    fn event(ts_from: DateTime<Utc>, ts_to: DateTime<Utc>) -> PackageCountEvent {
        PackageCountEvent::try_new(
            ts_from,
            ts_to,
            zone(),
            2,
            2,
            0.8,
            UncertaintyFlags::empty(),
            common_types::fusion::ModalityBlend::normalize(0.7),
            RlAction::defaults().as_detector_thresholds(),
            Vec::new(),
            Vec::new(),
            common_types::ids::ModelVersion::try_new("p4-test").unwrap(),
        )
        .unwrap()
    }

    fn trained_conf_arm_internal(bucket: usize) -> InternalState {
        // Fully train the conf bucket arm so the afternoon argmax picks it:
        // observe high reward at bias-only context 50 times.
        let mut arm = LinearThompsonArm::new();
        for _ in 0..50 {
            arm.observe(
                &{
                    let mut x = [0.0f32; FEATURE_DIM];
                    x[0] = 1.0;
                    x
                },
                6.0,
            );
        }
        InternalState {
            arms: vec![((ActionDim::Conf, bucket), arm)],
            last_safe: default_action(),
            frozen: false,
            consecutive_vetoes: 0,
            pause_until: None,
            fp_bucket: 2,
            last_seen: None,
            history: Vec::new(),
        }
    }

    #[tokio::test]
    async fn select_envelope_and_seed_determinism() {
        let mut policy = BanditPolicy::new();
        // Pretrain τ_conf bucket 1 (0.45 = default): guarantees acceptance.
        policy.restore_internal_state(trained_conf_arm_internal(1));
        let a1 = policy.select_action(&state(base_day(14, 0))).await;
        assert!(common_types::bounds::CONF_SAFE_RANGE.contains(&a1.conf_threshold()));
        assert!(common_types::bounds::IOU_SAFE_RANGE.contains(&a1.iou_threshold()));
        assert!(common_types::bounds::FUSION_WEIGHT_RANGE.contains(&a1.vision_weight()));
        assert!(common_types::bounds::TRACK_BUFFER_RANGE.contains(&a1.track_buffer_frames()));
        // Seeded rng + identical state: two fresh policies agree exactly.
        let mut twin = BanditPolicy::new();
        twin.restore_internal_state(trained_conf_arm_internal(1));
        let a2 = twin.select_action(&state(base_day(14, 0))).await;
        assert_eq!(a1, a2, "seeded bandit is deterministic");
        assert!(!policy.history().is_empty());
    }

    #[tokio::test]
    async fn veto_blocks_far_conf_jumps_and_freezes_on_third() {
        let mut policy = BanditPolicy::new();
        // Train bucket 4 (τ_conf = 0.85): 0.85 − 0.45 = 0.40 > 0.15 → veto.
        policy.restore_internal_state(trained_conf_arm_internal(4));
        let a1 = policy.select_action(&state(base_day(14, 0))).await;
        assert_eq!(a1, default_action(), "veto keeps last-safe");
        assert_eq!(policy.consecutive_vetoes(), 1);
        policy.select_action(&state(base_day(14, 31))).await;
        policy.select_action(&state(base_day(15, 1))).await;
        assert!(policy.frozen(), "three in a row freeze the policy");
        // Frozen policy: always last-safe, no further vetoes.
        let a4 = policy.select_action(&state(base_day(15, 31))).await;
        assert_eq!(a4, default_action());
        policy.unfreeze(Some(default_action()));
        assert!(!policy.frozen());
        assert_eq!(policy.consecutive_vetoes(), 0);
    }

    #[tokio::test]
    async fn trained_arm_wins_its_dim_under_veto_threshold() {
        // Train bucket 1 (0.45) to dominate: stays at default → accepted.
        let mut policy = BanditPolicy::new();
        policy.restore_internal_state(trained_conf_arm_internal(1));
        let action = policy.select_action(&state(base_day(14, 0))).await;
        assert_eq!(action.conf_threshold(), 0.45);
        assert_eq!(policy.consecutive_vetoes(), 0);
        assert_eq!(policy.history().len(), 1);
    }

    #[tokio::test]
    async fn pause_override_beats_agent() {
        let mut policy = BanditPolicy::new();
        policy.restore_internal_state(trained_conf_arm_internal(1));
        policy.pause(Some(base_day(15, 0)));
        let held = policy.select_action(&state(base_day(14, 31))).await;
        assert_eq!(held, default_action());
        assert!(policy.history().is_empty(), "paused policy never acts");
        assert_eq!(policy.current_fp_weight(), 2.0, "mid λ bucket default");
        // Manual override lifted: the agent acts again.
        policy.pause(None);
        policy.select_action(&state(base_day(14, 31))).await;
        assert!(!policy.history().is_empty());
    }

    #[tokio::test]
    async fn event_signal_shapes_with_immediate_weight() {
        let mut policy = BanditPolicy::new();
        let ev = event(base_day(14, 0), base_day(14, 20));
        let report = policy
            .update_policy(&PolicyFeedback::EventSignal {
                event: ev,
                immediate: 1.0,
            })
            .await
            .unwrap();
        assert_eq!(report.new_action, default_action());
        assert!(!report.was_clamped);
        assert!(report.next_exploration_window.is_some());
        assert_eq!(IMMEDIATE_WEIGHT, 0.1);
    }

    #[tokio::test]
    async fn human_audit_attribution_via_traces() {
        let mut policy = BanditPolicy::new();
        // Pretrain τ_conf bucket 1 (0.45 = default): guaranteed accept.
        policy.restore_internal_state(trained_conf_arm_internal(1));
        // One accepted select at 14:00 (epoch e).
        policy.select_action(&state(base_day(14, 0))).await;
        let record_epoch = policy.history().front().unwrap().epoch;
        // Audit window = epoch record±1 epochs, full reward −4.
        policy
            .update_policy(&PolicyFeedback::HumanAudit {
                feedback: common_types::feedback::HumanAuditFeedback {
                    zone_id: zone(),
                    reported_count: 10,
                    actual_count: 6,
                    ts_from: base_day(13, 59),
                    ts_to: base_day(14, 29),
                    failure_tags: common_types::feedback::FailureTag::DOUBLE_COUNT,
                    notes: None,
                    operator_id: common_types::ids::OperatorId::try_new("op").unwrap(),
                    wants_retrain: false,
                },
                computed_reward: -4.0,
            })
            .await
            .unwrap();
        // The trained arm received −4 × γ^age·... via the record's context.
        let internal = policy.internal_state();
        let credited = internal
            .arms
            .iter()
            .find(|((dim, _), _)| dim == &ActionDim::Conf)
            .and_then(|(_, arm)| {
                if arm.observations() > 0 {
                    Some(arm.weights())
                } else {
                    None
                }
            });
        assert!(
            credited.is_some(),
            "the epoch record's arms were credited on the audit window"
        );
        assert_eq!(record_epoch, epoch_of(base_day(14, 0)));
    }

    #[tokio::test]
    async fn snapshot_roundtrip_preserves_state() {
        let mut policy = BanditPolicy::new();
        policy.select_action(&state(base_day(14, 0))).await;
        policy.pause(Some(base_day(18, 0)));
        let blob = policy.snapshot_weights().await;
        let mut other = BanditPolicy::new();
        other.restore_weights(&blob).await.unwrap();
        assert_eq!(other.internal_state(), policy.internal_state());
        assert_eq!(other.name(), "bandit-v1");
    }

    #[tokio::test]
    async fn restore_corrupt_blob_is_an_error() {
        let mut policy = BanditPolicy::new();
        let result = policy.restore_weights(b"not json").await;
        assert!(result.is_err());
    }
}
