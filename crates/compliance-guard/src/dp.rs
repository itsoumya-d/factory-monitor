//! ε-DP for released aggregates: clipped record gradients + Gaussian
//! noise; a budget ledger refuses over-spend.

/// L2-clamp a gradient to `radius`.
pub fn clip_l2(gradient: &[f32], radius: f32) -> Vec<f32> {
    let norm: f32 = gradient.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= radius || norm == 0.0 {
        gradient.to_vec()
    } else {
        let scale = radius / norm;
        gradient.iter().map(|x| x * scale).collect()
    }
}

/// Small xorshift PRNG so determinism is testable.
struct XorShift(u64);
impl XorShift {
    fn next_f64(&mut self) -> f64 {
        let x = self.0;
        self.0 ^= x << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 1_000_000) as f64 / 1_000_000.0
    }
}

impl Default for XorShift {
    fn default() -> Self {
        Self(0x9E377_9B97F4A7C15)
    }
}

/// Sum of clipped gradients with Gaussian noise scaled by σ =
/// C·√(2·ln(1.25/δ))/ε (the classical Gaussian-mechanism bound).
pub fn noisy_sum(
    gradients: &[Vec<f32>],
    clip_radius: f32,
    epsilon: f32,
    delta: f32,
    seed: u64,
) -> Vec<f32> {
    let dim = gradients.first().map(Vec::len).unwrap_or(0);
    let ln_term = (2.0 * (1.25 / f64::from(delta.max(1e-12))).ln().abs()).sqrt();
    let sigma = (f64::from(clip_radius) * ln_term / f64::from(epsilon.max(1e-6))) as f32;
    let mut rng = XorShift(seed.max(1));
    let mut out = vec![0.0_f32; dim];
    for g in gradients {
        let clipped = clip_l2(g, clip_radius);
        for (o, v) in out.iter_mut().zip(clipped).take(dim) {
            *o += v;
        }
    }
    for v in &mut out {
        *v += (rng.next_f64() as f32 - 0.5) * 2.0 * sigma;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BudgetError {
    #[error("epsilon budget exhausted ({spent}/{total})")]
    Exhausted { spent: u32, total: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLedger {
    pub total: u32,
    pub spent: u32,
}

impl BudgetLedger {
    pub fn new(total: u32) -> Self {
        Self { total, spent: 0 }
    }

    pub fn spend(&mut self, epsilon_millie: u32) -> Result<(), BudgetError> {
        let after = self.spent + epsilon_millie;
        if after > self.total {
            return Err(BudgetError::Exhausted {
                spent: self.spent,
                total: self.total,
            });
        }
        self.spent = after;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn clipping_bounds_radius() {
        let long = vec![10.0, 20.0];
        let clipped = clip_l2(&long, 5.0);
        let norm: f32 = clipped.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 5.0).abs() < 0.01);
    }

    #[test]
    fn noise_scales_with_inverse_epsilon() {
        let data = vec![vec![1.0, 2.0]; 50];
        let sum: Vec<f32> = vec![50.0, 100.0];

        let high = noisy_sum(&data, 10.0, 100.0, 0.01, 7);
        let low = noisy_sum(&data, 10.0, 1.0, 0.01, 8);

        let high_noise: f32 = high.iter().zip(&sum).map(|(&r, &s)| (r - s).abs()).sum();
        let low_noise: f32 = low.iter().zip(&sum).map(|(&r, &s)| (r - s).abs()).sum();
        assert!(
            low_noise >= high_noise,
            "smaller epsilon must add more noise"
        );
    }

    #[test]
    fn budget_refuses_overspend() {
        let mut b = BudgetLedger::new(200);
        assert!(b.spend(100).is_ok());
        assert!(b.spend(100).is_ok());
        assert_eq!(
            b.spend(1),
            Err(BudgetError::Exhausted {
                spent: 200,
                total: 200
            })
        );
    }
}
