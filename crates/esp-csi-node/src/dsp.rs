//! Core-only radix-2 FFT (no_std-safe, iterative, no external DSP dep).

#[cfg(not(test))]
use crate::{Vec, vec};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    pub fn magnitude(&self) -> f32 {
        libm::sqrtf(self.re * self.re + self.im * self.im)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DspError {
    #[error("fft length {0} exceeds the {1}-sample node budget")]
    TooLong(usize, usize),
}

pub const MAX_FFT_SAMPLES: usize = 128;

/// Forward FFT for one power-of-two length ≤ 128; input is not consumed.
pub fn fft(data: &[f32]) -> Result<Vec<Complex>, DspError> {
    if data.is_empty() {
        return Ok(vec![]);
    }
    if data.len() > MAX_FFT_SAMPLES {
        return Err(DspError::TooLong(data.len(), MAX_FFT_SAMPLES));
    }
    let n = data.len();
    if !n.is_power_of_two() {
        let next = n.next_power_of_two().min(MAX_FFT_SAMPLES);
        let mut padded = data.to_vec();
        padded.resize(next, 0.0);
        return fft_pow2(&padded);
    }
    fft_pow2(data)
}

fn fft_pow2(data: &[f32]) -> Result<Vec<Complex>, DspError> {
    let n = data.len();
    let mut buf: Vec<Complex> = data.iter().map(|v| Complex::new(*v, 0.0)).collect();
    if n <= 1 {
        return Ok(buf);
    }

    // Bit-reversal permutation.
    let bits = n.ilog2();
    for i in 0..n {
        let j = (i as u32).reverse_bits() >> (u32::BITS - bits);
        if (i as u32) < j {
            buf.swap(i as usize, j as usize);
        }
    }

    let mut len: usize = 2;
    while len <= n {
        let half = len / 2;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let angle = -2.0 * core::f32::consts::PI * (k as f32) / (len as f32);
                let twiddle = Complex::new(libm::cosf(angle), libm::sinf(angle));
                let a = buf[start + k];
                let b = buf[start + k + half];
                let t = Complex::new(
                    twiddle.re * b.re - twiddle.im * b.im,
                    twiddle.re * b.im + twiddle.im * b.re,
                );
                buf[start + k] = Complex::new(a.re + t.re, a.im + t.im);
                buf[start + k + half] = Complex::new(a.re - t.re, a.im - t.im);
            }
        }
        len *= 2;
    }
    Ok(buf)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn dft(input: &[f32], out_len: usize) -> Vec<Complex> {
        let mut out = Vec::new();
        for k in 0..out_len {
            let mut re = 0.0;
            let mut im = 0.0;
            for (t, v) in input.iter().enumerate() {
                let angle = -2.0 * core::f32::consts::PI * k as f32 * t as f32 / input.len() as f32;
                re += v * libm::cosf(angle);
                im += v * libm::sinf(angle);
            }
            out.push(Complex::new(re, im));
        }
        out
    }

    #[test]
    fn fft_matches_naive_dft() {
        let signal: Vec<f32> = (0..8)
            .map(|t| (libm::cosf(2.0 * core::f32::consts::PI * t as f32 / 8.0)))
            .collect();
        let fast = fft(&signal).expect("8 is a power of two");
        let naive = dft(&signal, 8);
        for (f, m) in fast.iter().zip(naive.iter()) {
            assert!((f.re - m.re).abs() < 1e-3, "re: {} vs {}", f.re, m.re);
            assert!((f.im - m.im).abs() < 1e-3, "im: {} vs {}", f.im, m.im);
        }
    }

    #[test]
    fn non_power_of_two_pads_to_next() {
        let signal: Vec<f32> = (0..12)
            .map(|t| (libm::cosf(2.0 * core::f32::consts::PI * t as f32 / 6.0)))
            .collect();
        // The FFT zero-pads to 16 internally, so the reference transform
        // must be the padded series' own 16-point DFT.
        let padded: Vec<f32> = {
            let mut p = signal.clone();
            p.resize(16, 0.0);
            p
        };
        let fast = fft(&signal).expect("paddable length");
        let naive = dft(&padded, 16);
        for (f, m) in fast.iter().zip(naive.iter()) {
            assert!(
                (f.re - m.re).abs() < 1e-2,
                "re mismatch: {} vs {}",
                f.re,
                m.re
            );
            assert!(
                (f.im - m.im).abs() < 1e-2,
                "im mismatch: {} vs {}",
                f.im,
                m.im
            );
        }
    }

    #[test]
    fn budget_guard_prefers_error() {
        let oversized = vec![0.0f32; 129];
        assert_eq!(
            fft(&oversized),
            Err(DspError::TooLong(129, MAX_FFT_SAMPLES))
        );
    }
}
