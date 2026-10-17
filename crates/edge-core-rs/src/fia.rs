//! FIA: fine-grained industrial activity — occlusion recovery FSM over
//! dual-stream (branch 1×3×3 | 3×1×1 | 1×1×1 | 5×1×1 kernels) semantics.

use thiserror::Error;

/// Fully allowed person-state labels (docs compliance whitelist clone —
/// the SINGLE source is compliance-guard; this guard is a safety net).
const ALLOWED_STATES: &[&str] = &[
    "standing",
    "bent_over",
    "sitting",
    "walking",
    "lying_down",
    "hands_raised",
    "hands_up",
    "fatigue",
    "staggering",
    "fall",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FiaError {
    #[error("person state {0} is not in the compliance whitelist")]
    StateNotWhitelisted(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcclusionPhase {
    FullyVisible,
    PartiallyOccluded,
    TransientInvisible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiaVerdict {
    Healthy,
    Recovering,
    Lost,
}

/// At 30 fps, a sub-second occlusion (≤ 30 frames) must recover; longer
/// becomes Lost and relies on fresh detector tracks.
pub const TRANSIENT_FRAMES: u32 = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct OcclusionFsm {
    pub phase: OcclusionPhase,
    pub frames_missing: u32,
}

impl OcclusionFsm {
    pub fn new() -> Self {
        Self {
            phase: OcclusionPhase::FullyVisible,
            frames_missing: 0,
        }
    }

    /// One detector frame outcome: Some(confidence in [0,1]) when the
    /// detector saw a track, None when fully occluded.
    pub fn update(&mut self, seen_confidence: Option<f32>) -> FiaVerdict {
        match seen_confidence {
            Some(conf) => {
                self.frames_missing = 0;
                self.phase = if conf > 0.55 {
                    OcclusionPhase::FullyVisible
                } else {
                    OcclusionPhase::PartiallyOccluded
                };
                FiaVerdict::Healthy
            }
            None => {
                self.frames_missing += 1;
                self.phase = if self.frames_missing <= TRANSIENT_FRAMES {
                    OcclusionPhase::TransientInvisible
                } else {
                    self.phase = OcclusionPhase::TransientInvisible;
                    return FiaVerdict::Lost;
                };
                FiaVerdict::Recovering
            }
        }
    }
}

impl Default for OcclusionFsm {
    fn default() -> Self {
        Self::new()
    }
}

/// Dual-stream feature split: four branch poolings over a tiny volume of
/// (frames × height × width)beits, matching the FSM's branch kernels.
pub type Stream = Vec<Vec<Vec<f32>>>; // [t][h][w]

/// Per-frame spatial mean (1×3×3 branch).
pub fn pool_1x3x3(stream: &Stream) -> Vec<f32> {
    stream
        .iter()
        .map(|frame| {
            let n: usize = frame.iter().map(Vec::len).sum();
            frame.iter().flatten().copied().sum::<f32>() / n.max(1) as f32
        })
        .collect()
}

/// Per-frame temporal mean over a ±1 window (3×1×1 branch).
pub fn pool_3x1x1(stream: &Stream) -> Vec<f32> {
    let frames = stream.len();
    (0..frames)
        .map(|t| {
            let start = t.saturating_sub(1);
            let end = (t + 2).min(frames);
            let mean: f32 = stream
                .iter()
                .skip(start)
                .take(end - start)
                .map(|f| {
                    f.iter().flatten().copied().sum::<f32>()
                        / f.iter().flatten().count().max(1) as f32
                })
                .sum();
            mean / (end - start).max(1) as f32
        })
        .collect()
}

/// Identity branch (1×1×1): frame-local leading cell.
pub fn pool_1x1x1(stream: &Stream) -> Vec<f32> {
    stream
        .iter()
        .map(|f| f.first().and_then(|h| h.first().copied()).unwrap_or(0.0))
        .collect()
}

/// Per-frame temporal mean over a ±2 window (5×1×1 branch).
pub fn pool_5x1x1(stream: &Stream) -> Vec<f32> {
    let frames = stream.len();
    (0..frames)
        .map(|t| {
            let start = t.saturating_sub(2);
            let end = (t + 3).min(frames);
            let mean: f32 = stream
                .iter()
                .skip(start)
                .take(end - start)
                .map(|f| {
                    f.iter().flatten().copied().sum::<f32>()
                        / f.iter().flatten().count().max(1) as f32
                })
                .sum();
            mean / (end - start).max(1) as f32
        })
        .collect()
}

/// NTA gate: only whitelisted person states may ever enter inference
/// code as a label.
pub fn validate_state(state: &str) -> Result<(), FiaError> {
    if ALLOWED_STATES.contains(&state) {
        Ok(())
    } else {
        Err(FiaError::StateNotWhitelisted(state.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn cube(frames: usize) -> Stream {
        (0..frames)
            .map(|t| {
                (0..3)
                    .map(|h| {
                        (0..4)
                            .map(|w| (t * 12 + h * 4 + w) as f32 / 100.0)
                            .collect()
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn occlusion_fsm_recovers_through_transient() {
        let mut fsm = OcclusionFsm::new();
        for _ in 0..15 {
            assert_eq!(fsm.update(None), FiaVerdict::Recovering);
            assert_eq!(fsm.phase, OcclusionPhase::TransientInvisible);
        }
        assert_eq!(fsm.update(Some(0.9)), FiaVerdict::Healthy);
        assert_eq!(fsm.phase, OcclusionPhase::FullyVisible);
    }

    #[test]
    fn occlusion_fsm_hits_lost_after_budget() {
        let mut fsm = OcclusionFsm::new();
        for _ in 0..TRANSIENT_FRAMES {
            assert_eq!(fsm.update(None), FiaVerdict::Recovering);
        }
        assert_eq!(fsm.update(None), FiaVerdict::Lost);
    }

    #[test]
    fn low_confidence_reads_as_partial() {
        let mut fsm = OcclusionFsm::new();
        let _ = fsm.update(Some(0.42));
        assert_eq!(fsm.phase, OcclusionPhase::PartiallyOccluded);
    }

    #[test]
    fn four_branch_pools_keep_shape() {
        let s = cube(5);
        assert_eq!(pool_1x3x3(&s).len(), 5);
        assert_eq!(pool_3x1x1(&s).len(), 5);
        assert_eq!(pool_1x1x1(&s).len(), 5);
        assert_eq!(pool_5x1x1(&s).len(), 5);
    }

    #[test]
    fn spatial_mean_matches_mean() {
        let s = cube(2);
        let all: f32 = (0..12).map(|i| i as f32 / 100.0).sum();
        let want = all / 12.0;
        let got = pool_1x3x3(&s)[0];
        assert!((got - want).abs() < 1e-6, "want {want}, got {got}");
    }

    #[test]
    fn identity_branch_reads_leading_cell() {
        let s = cube(3);
        let got = pool_1x1x1(&s);
        assert_eq!(got, vec![0.0, 0.12, 0.24]);
    }

    #[test]
    fn nta_whitelist_gate() {
        assert!(validate_state("bent_over").is_ok());
        assert!(validate_state("joyful").is_err());
    }
}
