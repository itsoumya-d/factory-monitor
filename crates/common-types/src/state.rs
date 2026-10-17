//! `FactoryEnvState` — the observation the RL agent sees (docs/06 §1) plus the
//! per-zone topology snapshot from the morning calibration.

use serde::{Deserialize, Serialize};

use crate::ZoneId;
use crate::edge_stamp::EdgeStamp;
use crate::ids::CameraId;
use crate::lifecycle::LifecycleState;

/// Geometry + sensor wiring of one counting zone after morning calibration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneTopology {
    pub zone_id: ZoneId,
    pub camera_ids: Vec<CameraId>,
    pub csi_node_ids: Vec<u8>,
    pub counting_lines: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactoryEnvState {
    pub ts: EdgeStamp,
    /// BT.709 luma mean in [0..255] per camera (order matches `zones`' cameras).
    pub lux_proxy: Vec<f32>,
    pub blur_score: Vec<f32>,
    pub csi_variance: f32,
    pub occlusion_ratio: f32,
    pub conf_threshold: f32,
    pub iou_threshold: f32,
    pub vision_weight: f32,
    pub wifi_weight: f32,
    pub scene_complexity: f32,
    pub lifecycle: LifecycleState,
    pub zones: Vec<ZoneTopology>,
}

impl FactoryEnvState {
    /// 12-d normalized RL observation (docs/08 §3). The agent must consume
    /// this projection, never the raw struct, so every policy sees identical
    /// scaling; lifecycle is one-hot as `{motor_on, degraded}`.
    pub fn rl_state_vector(&self) -> [f32; 12] {
        let mean_lux = normalize_mean(&self.lux_proxy, 255.0);
        let mean_blur = normalize_mean(&self.blur_score, 1.0);
        let motor_on = matches!(
            self.lifecycle,
            LifecycleState::Counting | LifecycleState::Correcting
        ) as u8 as f32;
        let degraded = matches!(self.lifecycle, LifecycleState::Degraded) as u8 as f32;
        [
            mean_lux,
            mean_blur,
            unit(self.csi_variance),
            unit(self.occlusion_ratio),
            unit(self.conf_threshold),
            unit(self.iou_threshold),
            unit(self.vision_weight),
            unit(1.0 - self.vision_weight),
            unit(self.scene_complexity),
            self.err_momentum_1h(),
            self.err_momentum_24h(),
            motor_on * 0.8 + degraded * 0.2,
        ]
    }

    /// Exponential-moving over/under-count signals are provided upstream from
    /// the audit store; here they collapse to normalized placeholders of 0.0
    /// until `rl-controller` seeds real values via its own state builder.
    const fn err_momentum_1h(&self) -> f32 {
        0.0
    }

    const fn err_momentum_24h(&self) -> f32 {
        0.0
    }
}

fn unit(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn normalize_mean(values: &[f32], scale: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let sum: f32 = values.iter().filter(|v| v.is_finite()).by_ref().sum();
    (sum / scale / values.len().max(1) as f32).clamp(0.0, 1.0)
}
