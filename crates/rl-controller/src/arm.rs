//! Linear contextual Thompson arm (docs/08 §5): ridge-regularized mean over
//! the shared context features with a decaying posterior spread. Selection
//! samples the mean; the driver routes everything through `RlAction::clamped`.

use nalgebra::{DMatrix, DVector};

use crate::context::FEATURE_DIM;

/// Ridge regularization of `XᵀX + λI` (float noise/lag scale).
pub const RIDGE: f32 = 1.0;
/// Initial posterior spread before the arm has observations ±1.0 reward.
pub const POSTERIOR_SPREAD_0: f32 = 1.0;
/// Spread shrink per observation: ±σ0/√(1+n).
pub const SPREAD_DECAY: f32 = 0.5;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearThompsonArm {
    /// Ridge precision matrix A = λI + Σ x xᵀ (d×d).
    precision: DMatrix<f32>,
    /// Correlation vector f = Σ r·x.
    correlation: DVector<f32>,
    observations: u64,
    sum_sq_residual: f32,
}

impl LinearThompsonArm {
    pub fn new() -> Self {
        Self {
            precision: DMatrix::identity(FEATURE_DIM, FEATURE_DIM) * RIDGE,
            correlation: DVector::zeros(FEATURE_DIM),
            observations: 0,
            sum_sq_residual: 0.0,
        }
    }

    pub fn observations(&self) -> u64 {
        self.observations
    }

    /// Posterior spread ±σ0 / √(1 + n).
    pub fn spread(&self) -> f32 {
        POSTERIOR_SPREAD_0 / (1.0 + (self.observations as f32).sqrt() * SPREAD_DECAY * 2.0)
    }

    /// Posterior mean of reward for context `x`: w = A⁻¹ f, r̂ = xᵀw.
    pub fn mean(&self, x: &[f32; FEATURE_DIM]) -> f32 {
        let w = self.weights();
        let xv = DVector::from_column_slice(x);
        let r = xv.transpose() * &w;
        r[(0, 0)]
    }

    pub fn weights(&self) -> DVector<f32> {
        let f = &self.correlation;
        // Solve A w = f  (fall back to ridge-only identity when singular).
        match self.precision.clone().try_inverse() {
            Some(inv) => inv * f,
            None => DVector::zeros(FEATURE_DIM),
        }
    }

    /// Sample a reward from N(mean(x), spread²); clamped into the reward
    /// envelope ±8 so a saturated spike cannot dominate an argmax.
    pub fn sample(&self, x: &[f32; FEATURE_DIM], rng: &mut impl rand::Rng) -> f32 {
        let mean = self.mean(x);
        let mean = if mean.is_finite() { mean } else { 0.0 };
        let sigma = self.spread().max(1.0e-3);
        match rand_distr::Normal::new(mean, sigma) {
            Ok(normal) => {
                use rand_distr::Distribution;
                let draw: f32 = normal.sample(rng);
                draw.clamp(-8.0, 8.0)
            }
            Err(err) => {
                tracing::debug!(error = %err, "linear arm sampler degraded to mean");
                mean
            }
        }
    }

    /// Observe (x, r): ridge update + residual bookkeeping.
    pub fn observe(&mut self, x: &[f32; FEATURE_DIM], r: f32) {
        let xv = DVector::from_column_slice(x);
        self.precision += xv.clone() * xv.transpose();
        self.correlation += xv * r;
        let predicted = self.mean(x);
        self.sum_sq_residual += (r - predicted).powi(2);
        self.observations += 1;
    }
}

impl Default for LinearThompsonArm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn bias_only() -> [f32; FEATURE_DIM] {
        let mut x = [0.0; FEATURE_DIM];
        x[0] = 1.0;
        x
    }

    #[test]
    fn observe_pulls_mean_toward_constant_reward() {
        let mut arm = LinearThompsonArm::new();
        let x = bias_only();
        for _ in 0..40 {
            arm.observe(&x, 2.0);
        }
        let mean = arm.mean(&x);
        assert!((mean - 2.0).abs() < 0.05, "mean {mean} should approach 2.0");
        assert_eq!(arm.observations(), 40);
    }

    #[test]
    fn spread_decays_with_observations() {
        let mut arm = LinearThompsonArm::new();
        let x = bias_only();
        let before = arm.spread();
        for _ in 0..25 {
            arm.observe(&x, 1.0);
        }
        assert!(
            arm.spread() < before,
            "spread must shrink as data accumulates"
        );
    }

    #[test]
    fn sample_is_finite_even_with_corrupt_correlation() {
        let mut arm = LinearThompsonArm::new();
        arm.correlation[0] = f32::NAN;
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let s = arm.sample(&bias_only(), &mut rng);
        assert!(s.is_finite(), "non-finite posterior must fall back to 0.0");
    }
}
