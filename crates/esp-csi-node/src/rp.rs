//! Recurrence plot coarse-graining (8×8, 4-bit recurrence codes),
//! the amplitude signature fed to anomaly comparison.

pub const RP_DIM: usize = 8;
#[cfg(not(test))]
use crate::Vec;

/// Resample `window` to RP_DIM samples, standardize, and record
/// per-pair recurrence as a 4-bit code in [[0, 15]].
pub fn recurrence_plot(window: &[f32]) -> [[u8; RP_DIM]; RP_DIM] {
    let step = if window.len() >= RP_DIM {
        window.len() / RP_DIM
    } else {
        1
    };
    let sampled: Vec<f32> = (0..RP_DIM)
        .map(|i| {
            let idx = (i * step).min(window.len().saturating_sub(1));
            window.get(idx).copied().unwrap_or(0.0)
        })
        .collect();

    let mean: f32 = sampled.iter().sum::<f32>() / sampled.len().max(1) as f32;
    let variance: f32 = sampled
        .iter()
        .map(|v| (*v - mean) * (*v - mean))
        .sum::<f32>()
        / sampled.len().max(1) as f32;
    let std = libm::sqrtf(variance.max(1e-12));

    let mut grid = [[0u8; RP_DIM]; RP_DIM];
    for i in 0..RP_DIM {
        grid[i][i] = 15;
        for j in (i + 1)..RP_DIM {
            let zi = (sampled[i] - mean) / std;
            let zj = (sampled[j] - mean) / std;
            // distance over z-space: closer → higher recurrence.
            let dist = (zi - zj).abs();
            let rec = ((2.0 - dist).max(0.0) / 2.0 * 15.0).round() as u8;
            grid[i][j] = rec.min(15);
            grid[j][i] = rec.min(15);
        }
    }
    grid
}

/// L2 distance between two recurrence plots (coarse anomaly score).
pub fn rp_distance(a: &[[u8; RP_DIM]; RP_DIM], b: &[[u8; RP_DIM]; RP_DIM]) -> u32 {
    let mut acc: u32 = 0;
    for (row_a, row_b) in a.iter().zip(b.iter()) {
        for (va, vb) in row_a.iter().zip(row_b.iter()) {
            let d = i32::from(*va) - i32::from(*vb);
            acc += u32::try_from(d.abs()).unwrap_or(0);
        }
    }
    acc
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn constant_window_is_full_diagonal_and_uniform_edges() {
        let grid = recurrence_plot(&[3.0; 64]);
        assert_eq!(grid[0][0], 15);
        // Constant window → all pairs identical z (std = tiny) → all recurrence max.
        for row in &grid {
            for cell in row {
                assert_eq!(*cell, 15, "identical samples must be full-recurrence");
            }
        }
    }

    #[test]
    fn distinct_steps_redden_less() {
        let mut window = Vec::new();
        for t in 0..64 {
            window.push(if t < 32 { 0.0 } else { 10.0 });
        }
        let grid = recurrence_plot(&window);
        assert_eq!(grid[0][0], 15);
        assert!(
            rp_distance(&grid, &recurrence_plot(&[0.0; 64])) > 0,
            "mixed window is not flat"
        );
    }

    #[test]
    fn distance_metric_sums_abs_deltas() {
        let base = recurrence_plot(&[0.0; 64]);
        let mut noise = base;
        noise[1][2] = 1;
        assert_eq!(rp_distance(&base, &noise), 14); // single (1,2) cell delta
    }
}
