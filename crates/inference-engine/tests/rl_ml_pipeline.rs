//! End-to-end acceptance suite: the RL controller steers the ML stack
//! (docs/08). Every test drives the full chain — policy sample →
//! guardrail-clamped `RlAction` → `ModalityFusionEngine.set_action` (blend)
//! → `ByteTracker::reconfigure` (tracker params) — so no policy sample can
//! ever break the vision side.

#![allow(clippy::unwrap_used)]

use chrono::{Duration, Utc};
use common_types::ids::ModelVersion;
use common_types::lifecycle::LifecycleState;
use common_types::state::FactoryEnvState;
use common_types::traits::Detection;
use common_types::traits::PolicyAgent;
use inference_engine::tracker::{ByteTracker, TrackParams};
use inference_engine::{fusion::ModalityFusionEngine, tracker::TrackParams as TP};
use rl_controller::policy::{BanditPolicy, InternalState};

/// A valid site state; the sweep varies every observation dim.
fn state(ts: chrono::DateTime<Utc>, lux: f32, csi: f32, complexity: f32) -> FactoryEnvState {
    FactoryEnvState {
        ts: EdgeStamp {
            utc: ts,
            stream_pts: 0,
            ptp_offset_ns: 0,
            clock_ok: true,
        },
        lux_proxy: vec![lux],
        blur_score: vec![0.05],
        csi_variance: csi,
        occlusion_ratio: 0.1,
        conf_threshold: 0.60,
        iou_threshold: 0.55,
        vision_weight: 0.5,
        wifi_weight: 0.5,
        scene_complexity: complexity,
        lifecycle: LifecycleState::Counting,
        zones: vec![],
    }
}

use common_types::edge_stamp::EdgeStamp;

use common_types::action::RlAction;
use common_types::bounds::{
    CONF_SAFE_RANGE, FUSION_WEIGHT_RANGE, IOU_SAFE_RANGE, TRACK_BUFFER_RANGE,
};

fn in_guardrails(a: RlAction) {
    assert!(
        CONF_SAFE_RANGE.contains(&a.conf_threshold()),
        "conf {} out of runway {CONF_SAFE_RANGE:?}",
        a.conf_threshold()
    );
    assert!(
        IOU_SAFE_RANGE.contains(&a.iou_threshold()),
        "iou {} out of runway {IOU_SAFE_RANGE:?}",
        a.iou_threshold()
    );
    assert!(
        FUSION_WEIGHT_RANGE.contains(&a.vision_weight()),
        "vision weight {} out of runway {FUSION_WEIGHT_RANGE:?}",
        a.vision_weight()
    );
    assert!(
        TRACK_BUFFER_RANGE.contains(&a.track_buffer_frames()),
        "track buffer {} out of runway {TRACK_BUFFER_RANGE:?}",
        a.track_buffer_frames()
    );
}

/// 64 environment sweeps → policy → fusion + tracker. Whatever the bandit
/// samples, every consumer stays inside its guardrail.
#[tokio::test]
async fn policy_actions_fuse_and_track_across_a_site_sweep() {
    let mut policy = BanditPolicy::new();
    for i in 0..64u32 {
        let t = Utc::now() + Duration::seconds(i as i64);
        let st = state(
            t,
            10.0 + i as f32 * 2.0,
            (i % 101) as f32 / 100.0,
            (i % 17) as f32 / 17.0,
        );
        let action = policy.select_action(&st).await;
        in_guardrails(action);

        // ML consumer 1: the fusion engine carries the action verbatim.
        let mut fusion = ModalityFusionEngine::new(ModelVersion::try_new("eng:sweep").unwrap());
        fusion.set_action(action);
        assert_eq!(fusion.action(), action, "action bus round-trip");
        let blend = fusion.blend();
        assert!(
            (blend.vision() + blend.wifi() - 1.0).abs() < 1e-6,
            "blend must stay normalized"
        );
        assert!(FUSION_WEIGHT_RANGE.contains(&blend.vision()));

        // ML consumer 2: the tracker reconfigures straight from the action
        // and keeps tracking a live constant-velocity object.
        let mut tr = ByteTracker::new(TrackParams::docs_default());
        tr.reconfigure(TP::from_thresholds(
            action.iou_threshold(),
            action.track_buffer_frames(),
        ));
        let x = 10.0 + 10.0 * i as f32;
        let out = tr
            .update(&[Detection {
                bbox_xyxy: [x, 100.0, x + 100.0, 200.0],
                score: 0.9,
                class_id: 0,
            }])
            .unwrap();
        assert_eq!(out.len(), 1, "live track survives each RL sample");
    }
}

/// Paused policies hand back the last-safe action untouched (guardrail #4:
/// an operator can always silence the loop).
#[tokio::test]
async fn paused_policy_serves_last_safe_without_sampling() {
    let mut policy = BanditPolicy::new();
    let safe = policy.last_safe();
    assert_eq!(safe, RlAction::defaults());
    policy.pause(Some(Utc::now() + Duration::hours(1)));
    let action = policy
        .select_action(&state(Utc::now(), 40.0, 0.4, 0.1))
        .await;
    assert_eq!(action, safe, "paused policy must not sample arms");
    policy.pause(None);
    let action = policy
        .select_action(&state(Utc::now(), 60.0, 0.6, 0.1))
        .await;
    in_guardrails(action);
}

/// Frozen-state restore: a snapshot with `frozen = true` makes the policy
/// serve last-safe until an operator signs off via `unfreeze`.
#[tokio::test]
async fn frozen_state_serves_last_safe_until_unfreeze() {
    let mut policy = BanditPolicy::new();
    let mut snapshot: InternalState = policy.internal_state();
    snapshot.frozen = true;
    policy.restore_internal_state(snapshot);
    let action = policy
        .select_action(&state(Utc::now(), 120.0, 0.9, 0.6))
        .await;
    assert_eq!(action, policy.last_safe(), "frozen policy is inert");
    assert!(policy.frozen());

    let restored = RlAction::clamped(0.55, 0.55, 0.8, 45);
    policy.unfreeze(Some(restored));
    assert!(!policy.frozen());
    let action = policy
        .select_action(&state(Utc::now(), 130.0, 0.5, 0.2))
        .await;
    in_guardrails(action);
}
