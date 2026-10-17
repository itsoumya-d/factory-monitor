//! Cross-modal fusion token seam. The real weights live behind the
//! `tensorrt` feature (ort); the default seam provides a deterministic
//! attention pooling used by tests and by the shadow lane.

use compliance_guard::annex3::RgbFrame;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FusionError {
    #[error("rgb token wrong dim: want {want}, got {got}")]
    RgbDim { want: usize, got: usize },
    #[error("depth token wrong dim: want {want}, got {got}")]
    DepthDim { want: usize, got: usize },
    #[error("csi token wrong dim: want {want}, got {got}")]
    CsiDim { want: usize, got: usize },
    #[error("channel weights must stay normalized after clipping")]
    WeightDrift,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FusionOut {
    /// Confidence the fused decision carries into the FSM.
    pub track_confidence: f32,
    /// Softmax-normalized channel weights [rgb, depth, csi].
    pub channel_weights: [f32; 3],
}

/// The loop contract — identical for the mock and the ort backend.
pub trait FusionRuntime {
    fn cross_modal_attention(
        &self,
        rgb: &[f32],
        depth: &[f32],
        csi: &[f32],
    ) -> Result<FusionOut, FusionError>;
}

pub const TOKEN_DIM: usize = 3;

/// Deriving-style softmax over projected channel means; weights fuse the
/// three channels with a deterministic A matrix (no randomness).
pub struct DeterministicFusion {
    d_model: usize,
}

impl DeterministicFusion {
    pub fn new(d_model: usize) -> Self {
        Self {
            d_model: d_model.max(TOKEN_DIM),
        }
    }

    fn softmax(vars: [f32; 3]) -> [f32; 3] {
        let max = vars.iter().copied().fold(f32::MIN, f32::max);
        let exps: Vec<f32> = vars.iter().map(|v| (v - max).exp()).collect();
        let sum: f32 = exps.iter().sum();
        [exps[0] / sum, exps[1] / sum, exps[2] / sum]
    }
}

impl FusionRuntime for DeterministicFusion {
    fn cross_modal_attention(
        &self,
        rgb: &[f32],
        depth: &[f32],
        csi: &[f32],
    ) -> Result<FusionOut, FusionError> {
        let mean = |v: &[f32]| -> Result<f32, usize> {
            if v.len() < TOKEN_DIM {
                Err(v.len())
            } else {
                Ok(v.iter().take(self.d_model).sum::<f32>() / self.d_model.min(v.len()) as f32)
            }
        };
        let rgb_mean = mean(rgb).map_err(|got| FusionError::RgbDim {
            want: TOKEN_DIM,
            got,
        })?;
        let depth_mean = mean(depth).map_err(|got| FusionError::DepthDim {
            want: TOKEN_DIM,
            got,
        })?;
        let csi_mean = mean(csi).map_err(|got| FusionError::CsiDim {
            want: TOKEN_DIM,
            got,
        })?;
        let weights = Self::softmax([rgb_mean, depth_mean, csi_mean]);
        let drift = (weights.iter().sum::<f32>() - 1.0).abs();
        if drift > 1e-4 {
            return Err(FusionError::WeightDrift);
        }
        Ok(FusionOut {
            track_confidence: weights.iter().sum(),
            channel_weights: weights,
        })
    }
}

/// RGB frames keep their RAM-volatility marker through the fusion seam
/// (docs/04 invariant: raw RGB never persists).
pub struct RgbFrameToken {
    pub frame: RgbFrame,
    pub embedding: Vec<f32>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn softmax_is_normalized_and_stable() {
        let w = DeterministicFusion::softmax([0.0, 0.0, 0.0]);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        for v in &w {
            assert!(*v > 0.0 && *v < 1.0);
        }
    }

    #[test]
    fn fusion_weights_sum_to_one() {
        let f = DeterministicFusion::new(3);
        let out = f
            .cross_modal_attention(&[1.0; 3], &[2.0; 3], &[3.0; 3])
            .expect("dims fine");
        assert!((out.track_confidence - 1.0).abs() < 1e-5);
        assert!((out.channel_weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn undersized_tokens_rejected() {
        let f = DeterministicFusion::new(3);
        assert!(
            f.cross_modal_attention(&[1.0], &[1.0; 3], &[1.0; 3])
                .is_err()
        );
        assert!(f.cross_modal_attention(&[1.0; 3], &[], &[1.0; 3]).is_err());
        assert!(f.cross_modal_attention(&[1.0; 3], &[1.0; 3], &[]).is_err());
    }

    #[test]
    fn csi_channel_dominates_when_spir_alized() {
        let f = DeterministicFusion::new(3);
        let out = f
            .cross_modal_attention(&[0.0; 3], &[0.0; 3], &[10.0; 3])
            .expect("dims fine");
        let strongest = out
            .channel_weights
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal));
        assert_eq!(strongest.expect("weights").0, 2);
    }
}
