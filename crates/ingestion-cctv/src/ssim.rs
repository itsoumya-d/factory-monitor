//! Golden-baseline SSIM diffing on a 3x3 block grid (docs/04 §4).
//!
//! Morning layout diff: SSIM(today, baseline) per block; blocks whose score
//! falls below `1 - delta_thresh` (default 0.12) are the ChangeMap that feeds
//! the VLM morning diffing prompt together with block bounding boxes.

use serde::{Deserialize, Serialize};

pub const BLOCK_GRID: usize = 3;
pub const CHANGED_DELTA: f32 = 0.12;

const C1: f64 = (0.01 * 255.0) * (0.01 * 255.0);
const C2: f64 = (0.03 * 255.0) * (0.03 * 255.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRef {
    /// Row index in [0..3].
    pub row: u8,
    /// Column index in [0..3].
    pub col: u8,
}

/// SSIM over one rectangular block, computed over the full block as one
/// window (deterministic; no moving window) — adequate for a 3x3 ChangeMap.
pub fn block_ssim(a: &[u8], b: &[u8], width: u32, height: u32, block: BlockRef) -> Option<f32> {
    let w = usize::try_from(width).ok()?;
    let h = usize::try_from(height).ok()?;
    if a.len() != w * h || b.len() != a.len() {
        return None;
    }
    let x0 = w * usize::from(block.col) / BLOCK_GRID;
    let x1 = w * (usize::from(block.col) + 1) / BLOCK_GRID;
    let y0 = h * usize::from(block.row) / BLOCK_GRID;
    let y1 = h * (usize::from(block.row) + 1) / BLOCK_GRID;
    if x0 >= x1 || y0 >= y1 {
        return None;
    }

    let mut n = 0i64;
    let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0i64, 0i64, 0i64, 0i64, 0i64);
    for y in y0..y1 {
        for x in x0..x1 {
            let idx = y * w + x;
            let (va, vb) = (i64::from(a[idx]), i64::from(b[idx]));
            n += 1;
            sa += va;
            sb += vb;
            saa += va * va;
            sbb += vb * vb;
            sab += va * vb;
        }
    }
    if n == 0 {
        return None;
    }
    let nf = f64::from(n as i32);
    let (mu_a, mu_b) = (f64::from(sa as i32) / nf, f64::from(sb as i32) / nf);
    let var_a = f64::from(saa as i32) / nf - mu_a * mu_a;
    let var_b = f64::from(sbb as i32) / nf - mu_b * mu_b;
    let cov_ab = f64::from(sab as i32) / nf - mu_a * mu_b;

    let numerator = (2.0 * mu_a * mu_b + C1) * (2.0 * cov_ab + C2);
    let denominator = (mu_a * mu_a + mu_b * mu_b + C1) * (var_a + var_b + C2);
    if denominator <= 0.0 {
        return None;
    }
    Some((numerator / denominator).clamp(0.0, 1.0) as f32)
}

/// The morning ChangeMap: blocks where `SSIM(cur, baseline) < 1 - delta`.
pub fn flagged_blocks(
    current: &[u8],
    baseline: &[u8],
    width: u32,
    height: u32,
    delta: f32,
) -> Vec<BlockRef> {
    let mut out = Vec::new();
    for row in 0..BLOCK_GRID {
        for col in 0..BLOCK_GRID {
            if let Some(score) = block_ssim(
                current,
                baseline,
                width,
                height,
                BlockRef {
                    row: row as u8,
                    col: col as u8,
                },
            ) {
                if score < (1.0 - delta).clamp(0.0, 1.0) {
                    out.push(BlockRef {
                        row: row as u8,
                        col: col as u8,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn gray(width: u32, height: u32, fill: u8) -> Vec<u8> {
        vec![fill; (width * height) as usize]
    }

    #[test]
    fn identical_planes_score_one_every_block() {
        let plane = gray(90, 90, 128);
        for row in 0..3u8 {
            for col in 0..3u8 {
                let s = block_ssim(&plane, &plane, 90, 90, BlockRef { row, col }).unwrap();
                assert!((s - 1.0).abs() <= 1e-4);
            }
        }
    }

    #[test]
    fn structural_change_flags_blocks() {
        let mut base = gray(90, 90, 100);
        let mut cur = base.clone();
        for y in 60..90 {
            for x in 60..90 {
                base[y * 90 + x] = 255;
                cur[y * 90 + x] = 0;
            }
        }
        let flags = flagged_blocks(&cur, &base, 90, 90, CHANGED_DELTA);
        assert!(
            flags.iter().any(|b| b.row == 2 && b.col == 2),
            "changed corner block must flag, got {:?}",
            flags
        );
    }

    #[test]
    fn uniform_shift_keeps_block_scores_high() {
        let base = gray(90, 90, 100);
        let cur = gray(90, 90, 110);
        assert!(flagged_blocks(&cur, &base, 90, 90, CHANGED_DELTA).is_empty());
    }

    #[test]
    fn mismatched_sizes_score_none() {
        // Different total window lengths must reject.
        assert!(
            block_ssim(
                &gray(64, 64, 0),
                &gray(256, 128, 0)[..(64 * 64 + 3)],
                64,
                64,
                BlockRef { row: 0, col: 0 }
            )
            .is_none()
        );
        let full = gray(64, 64, 5);
        assert!(block_ssim(&full, &gray(64, 64, 5), 64, 64, BlockRef { row: 1, col: 2 }).is_some());
    }

    #[test]
    fn zero_variance_blocks_still_agree() {
        let base = gray(69, 72, 200);
        let flags = flagged_blocks(&base, &base, 69, 72, CHANGED_DELTA);
        assert!(flags.is_empty());
    }
}
