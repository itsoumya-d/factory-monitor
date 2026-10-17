//! Doppler velocity spectrum (BVP): stationary-subcarrier phase deltas
//! across frames map to motion speed via v = f_doppler·λ/2.

use crate::dsp::{DspError, fft};

#[cfg(not(test))]
use crate::{Vec, vec};

/// Doppler velocity bins from a per-subcarrier time series of phase (or
/// amplitude) samples. `packet_rate_hz` is the CSI packet cadence,
/// `wavelength_m` is the carrier (≈ 0.0523 m at 2.4 GHz).
pub fn velocity_spectrum(
    series: &[f32],
    packet_rate_hz: f32,
    wavelength_m: f32,
) -> Result<Vec<(f32, f32)>, DspError> {
    if series.len() < 16 {
        return Ok(vec![]);
    }
    let spectrum = fft(series)?;
    let n = series.len();
    let mut bins = Vec::with_capacity(n / 2);
    for (bin, c) in spectrum.iter().enumerate().take(n / 2) {
        let freq = bin as f32 * packet_rate_hz / n as f32;
        let speed = freq * wavelength_m / 2.0;
        bins.push((speed, c.magnitude()));
    }
    Ok(bins)
}

/// Dominant nonzero velocity from a binned spectrum (skips the DC bin).
pub fn dominant_velocity(bins: &[(f32, f32)]) -> f32 {
    let mut best = (0.0_f32, 0.0_f32);
    for (speed, mag) in bins {
        if *speed <= 0.0 {
            continue;
        }
        if *mag > best.1 {
            best = (*speed, *mag);
        }
    }
    best.0
}

/// Return the magnitude of the FFT bin nearest `velocity`, for
/// operator reading of a live control (mocked speed probe).
pub fn magnitude_near(bins: &[(f32, f32)], velocity: f32) -> f32 {
    bins.iter()
        .map(|(s, m)| (speed_delta(*s, velocity), *m))
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal))
        .map(|(_, m)| m)
        .unwrap_or(0.0)
}

fn speed_delta(a: f32, b: f32) -> f32 {
    (a - b).abs()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn stationary_series_peaks_at_dc() {
        let series = vec![1.0_f32; 32];
        let bins = velocity_spectrum(&series, 100.0, 0.0523).expect("32 samples");
        let (speed, mag) = bins.first().copied().expect("bins nonempty");
        assert!((speed).abs() < 1e-9);
        assert!(mag > 0.0);
        // Every other bin ≈ 0 (constant signal).
        for (s, m) in bins.iter().skip(1) {
            assert!(*m < 1e-3, "bin {s} leaked energy: {m}");
        }
    }

    #[test]
    fn dominant_velocity_skips_dc() {
        let mut bins = vec![(0.0, 100.0), (1.0, 3.0)];
        assert!((dominant_velocity(&bins) - 1.0).abs() < 1e-7);
        // All-zero magnitudes return 0.0 without any search bugs.
        bins = vec![(0.0, 5.0)];
        assert!(dominant_velocity(&bins) == 0.0);
    }

    #[test]
    fn short_series_is_empty_spectrum() {
        let got = velocity_spectrum(&[1.0_f32; 4], 100.0, 0.0523).expect("returns cleanly");
        assert!(got.is_empty());
    }

    #[test]
    fn magnitude_near_finds_closest_bin() {
        let bins = vec![(0.0, 100.0), (1.0, 3.0)];
        assert!((magnitude_near(&bins, 1.05) - 3.0).abs() < 1e-6);
        let empty: Vec<(f32, f32)> = vec![];
        assert!(magnitude_near(&empty, 1.0) == 0.0);
    }
}
